//! Out-of-band notifications through a local sendmail-compatible command.
//!
//! The command line is split into arguments without a shell: `;`, `|`, `$` and
//! backticks are ordinary text, so a configured value runs the program it names
//! and nothing else. The message goes to the command's stdin as RFC 5322 with a
//! plain-text body. The body is never logged and never put into an error, because
//! it may carry a recovery code.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};

use crate::config::NotifyConfig;

/// Upper bound for one send, so a hung MTA cannot hold the sync thread.
pub const SEND_TIMEOUT: Duration = Duration::from_secs(30);

/// Stderr kept from a failed send: enough for the MTA's reason, small enough to log.
const STDERR_LIMIT: usize = 512;

/// Send a plain-text notification through `cfg.command`.
pub fn send_notification(cfg: &NotifyConfig, subject: &str, body: &str) -> Result<()> {
    send_with_timeout(cfg, subject, body, SEND_TIMEOUT)
}

fn send_with_timeout(
    cfg: &NotifyConfig,
    subject: &str,
    body: &str,
    timeout: Duration,
) -> Result<()> {
    let command = cfg
        .command
        .as_deref()
        .context("notify.command is not set")?;
    let argv = split_command(command)?;
    let message = build_message(cfg, subject, body, SystemTime::now())?;
    run(&argv, message.as_bytes(), timeout)
}

/// Split a command line into arguments. Quotes and backslashes work as in a shell;
/// nothing is expanded and no operator has a meaning.
fn split_command(line: &str) -> Result<Vec<String>> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_arg = false;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_arg = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(ch) => current.push(ch),
                        None => bail!("notify.command has an unclosed single quote"),
                    }
                }
            }
            '"' => {
                in_arg = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(ch @ ('"' | '\\')) => current.push(ch),
                            Some(ch) => {
                                current.push('\\');
                                current.push(ch);
                            }
                            None => bail!("notify.command has an unclosed double quote"),
                        },
                        Some(ch) => current.push(ch),
                        None => bail!("notify.command has an unclosed double quote"),
                    }
                }
            }
            '\\' => {
                in_arg = true;
                match chars.next() {
                    Some(ch) => current.push(ch),
                    None => bail!("notify.command ends with a backslash"),
                }
            }
            c if c.is_whitespace() => {
                if in_arg {
                    args.push(std::mem::take(&mut current));
                    in_arg = false;
                }
            }
            c => {
                in_arg = true;
                current.push(c);
            }
        }
    }
    if in_arg {
        args.push(current);
    }
    match args.first() {
        None => bail!("notify.command is empty"),
        Some(program) if program.is_empty() => bail!("notify.command names no program"),
        Some(_) => Ok(args),
    }
}

/// RFC 5322 message: headers, a blank line, then the body with CRLF line endings.
fn build_message(cfg: &NotifyConfig, subject: &str, body: &str, now: SystemTime) -> Result<String> {
    let from = cfg.from.as_deref().context("notify.from is not set")?;
    if cfg.to.is_empty() {
        bail!("notify.to lists no recipients");
    }
    check_header("notify.from", from)?;
    check_header("subject", subject)?;
    for addr in &cfg.to {
        check_header("notify.to", addr)?;
        if addr.trim().is_empty() {
            bail!("notify.to has an empty address");
        }
    }
    let domain = from
        .rsplit_once('@')
        .map(|(_, host)| host.trim_end_matches('>'))
        .filter(|host| !host.is_empty())
        .unwrap_or("sshub.invalid");

    let mut msg = String::new();
    push_header(&mut msg, "From", from);
    push_header(&mut msg, "To", &cfg.to.join(", "));
    push_header(&mut msg, "Subject", subject);
    push_header(&mut msg, "Date", &rfc2822_date(now));
    push_header(
        &mut msg,
        "Message-ID",
        &format!("<{}@{domain}>", message_id_token()),
    );
    push_header(&mut msg, "Content-Type", "text/plain; charset=utf-8");
    msg.push_str("\r\n");
    for line in body.lines() {
        msg.push_str(line);
        msg.push_str("\r\n");
    }
    Ok(msg)
}

fn check_header(field: &str, value: &str) -> Result<()> {
    if value.contains(['\r', '\n', '\0']) {
        bail!("{field} contains a line break");
    }
    Ok(())
}

fn push_header(msg: &mut String, name: &str, value: &str) {
    msg.push_str(name);
    msg.push_str(": ");
    msg.push_str(value);
    msg.push_str("\r\n");
}

/// Unique enough for a Message-ID: time, process, and a counter for same-tick sends.
fn message_id_token() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{nanos}.{}.{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// RFC 2822 date in UTC, e.g. `Tue, 14 Nov 2023 22:13:20 +0000`.
fn rfc2822_date(t: SystemTime) -> String {
    const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    // 1970-01-01 was a Thursday; index 0 is Sunday.
    let weekday = (days + 4).rem_euclid(7) as usize;
    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} +0000",
        WEEKDAYS[weekday],
        day,
        MONTHS[(month - 1) as usize],
        year,
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Days since 1970-01-01 to (year, month, day). Howard Hinnant's algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn run(argv: &[String], message: &[u8], timeout: Duration) -> Result<()> {
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to start notify command `{}`", argv[0]))?;

    let mut stdin = child.stdin.take().context("notify command has no stdin")?;
    let input = message.to_vec();
    // A child that exits early closes the pipe; the exit status reports that.
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let mut stderr = child
        .stderr
        .take()
        .context("notify command has no stderr")?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        buf
    });

    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("notify command timed out after {}s", timeout.as_secs());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let _ = writer.join();
    let stderr = reader.join().unwrap_or_default();
    if status.success() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&stderr);
    let shown: String = text.trim().chars().take(STDERR_LIMIT).collect();
    bail!("notify command failed ({status}): {shown}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    const EPOCH_2023: u64 = 1_700_000_000;

    fn cfg(command: Option<&str>) -> NotifyConfig {
        NotifyConfig {
            command: command.map(str::to_string),
            to: vec!["ops@example.com".into()],
            from: Some("sshub <sshub@example.com>".into()),
        }
    }

    #[test]
    fn split_handles_quotes_and_backslashes() {
        assert_eq!(
            split_command("/usr/sbin/sendmail -t \"Sub dir/x\" 'a b' c\\ d \"q\\\"x\"").unwrap(),
            [
                "/usr/sbin/sendmail",
                "-t",
                "Sub dir/x",
                "a b",
                "c d",
                "q\"x"
            ]
        );
    }

    #[test]
    fn split_treats_shell_operators_as_text() {
        assert_eq!(
            split_command("sendmail -t; touch /x | cat $HOME `id`").unwrap(),
            ["sendmail", "-t;", "touch", "/x", "|", "cat", "$HOME", "`id`"]
        );
    }

    #[test]
    fn split_rejects_empty_and_unterminated_input() {
        for bad in ["", "   ", "\"\"", "sendmail \"-t", "'x", "sendmail \\"] {
            assert!(split_command(bad).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn dates_match_known_utc_instants() {
        let at = |secs: u64| rfc2822_date(UNIX_EPOCH + Duration::from_secs(secs));
        assert_eq!(at(0), "Thu, 01 Jan 1970 00:00:00 +0000");
        assert_eq!(at(EPOCH_2023), "Tue, 14 Nov 2023 22:13:20 +0000");
        assert_eq!(at(951_782_400), "Tue, 29 Feb 2000 00:00:00 +0000");
    }

    #[test]
    fn message_has_headers_then_crlf_body() {
        let now = UNIX_EPOCH + Duration::from_secs(EPOCH_2023);
        let msg = build_message(
            &cfg(Some("sendmail -t")),
            "Recovery started",
            "code: ABC\nline 2\n",
            now,
        )
        .unwrap();
        assert!(msg.starts_with("From: sshub <sshub@example.com>\r\nTo: ops@example.com\r\n"));
        assert!(msg.contains("\r\nSubject: Recovery started\r\n"));
        assert!(msg.contains("\r\nDate: Tue, 14 Nov 2023 22:13:20 +0000\r\n"));
        assert!(msg.contains("\r\nMessage-ID: <"));
        assert!(msg.contains("@example.com>\r\n"));
        assert!(msg.ends_with("\r\n\r\ncode: ABC\r\nline 2\r\n"));
    }

    #[test]
    fn message_refuses_header_injection_and_missing_fields() {
        let now = UNIX_EPOCH;
        let mut c = cfg(Some("sendmail -t"));
        assert!(build_message(&c, "hi\r\nBcc: evil@x", "b", now).is_err());
        c.to = vec!["ops@example.com\nBcc: evil@x".into()];
        assert!(build_message(&c, "hi", "b", now).is_err());
        c.to = vec![];
        assert!(build_message(&c, "hi", "b", now).is_err());
        c.to = vec!["ops@example.com".into()];
        c.from = None;
        assert!(build_message(&c, "hi", "b", now).is_err());
    }

    #[test]
    fn missing_command_is_an_error_not_a_silent_skip() {
        assert!(send_notification(&cfg(None), "s", "b").is_err());
    }

    /// Write an executable `/bin/sh` script named `name` into `dir`.
    #[cfg(unix)]
    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// A fake sendmail that records its argv (one per line) and stdin into `dir`.
    #[cfg(unix)]
    fn recording_sendmail(dir: &Path) -> PathBuf {
        let d = dir.display();
        script(
            dir,
            "sendmail",
            &format!("printf '%s\\n' \"$@\" > '{d}/argv'\ncat > '{d}/stdin'\n"),
        )
    }

    #[cfg(unix)]
    fn read_lines(path: &Path) -> Vec<String> {
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect()
    }

    #[cfg(unix)]
    #[test]
    fn sends_argv_and_rfc5322_on_stdin() {
        let dir = tempfile::tempdir().unwrap();
        let fake = recording_sendmail(dir.path());
        let command = format!("{} -t -i", fake.display());
        send_with_timeout(
            &cfg(Some(&command)),
            "Subject line",
            "body text",
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(read_lines(&dir.path().join("argv")), ["-t", "-i"]);
        let stdin = std::fs::read_to_string(dir.path().join("stdin")).unwrap();
        assert!(stdin.contains("\r\nSubject: Subject line\r\n"));
        assert!(stdin.ends_with("\r\n\r\nbody text\r\n"));
    }

    /// The injection attempt from the design: the `;` must not start a shell.
    #[cfg(unix)]
    #[test]
    fn injected_operator_is_an_argument_and_never_a_shell() {
        let dir = tempfile::tempdir().unwrap();
        let fake = recording_sendmail(dir.path());
        let pwned = dir.path().join("pwned");
        let command = format!("{} -t; touch {}", fake.display(), pwned.display());
        send_with_timeout(&cfg(Some(&command)), "s", "b", Duration::from_secs(10)).unwrap();
        assert_eq!(
            read_lines(&dir.path().join("argv")),
            ["-t;", "touch", &pwned.display().to_string()]
        );
        assert!(!pwned.exists(), "a shell ran the injected command");
    }

    #[cfg(unix)]
    #[test]
    fn a_hung_command_is_killed_at_the_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let slow = script(dir.path(), "slowmail", "sleep 10\n");
        let started = Instant::now();
        let err = send_with_timeout(
            &cfg(Some(&slow.display().to_string())),
            "s",
            "b",
            Duration::from_millis(300),
        )
        .unwrap_err();
        assert!(err.to_string().contains("timed out"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[test]
    fn non_zero_exit_reports_status_and_stderr_but_not_the_body() {
        let dir = tempfile::tempdir().unwrap();
        let refusing = script(dir.path(), "refuse", "echo 'relay refused' >&2\nexit 75\n");
        let err = send_with_timeout(
            &cfg(Some(&refusing.display().to_string())),
            "s",
            "RECOVERY-CODE-123",
            Duration::from_secs(10),
        )
        .unwrap_err();
        let text = err.to_string();
        assert!(text.contains("exit status: 75"), "{text}");
        assert!(text.contains("relay refused"), "{text}");
        assert!(!text.contains("RECOVERY-CODE-123"), "{text}");
    }
}
