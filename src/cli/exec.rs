//! `sshub exec <host> -- <command>`: run one command on a saved host.
//!
//! `sshub connect` hands the terminal to ssh for a human; this is the scripted
//! counterpart. stdio passes through, the remote exit code becomes ours, and
//! nothing here ever prompts. Session logging is deliberately skipped — the
//! `script(1)` wrapper `connect` uses fights output redirection, which is the
//! whole point of exec.

use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::app::{prepare_cli_connect_argv, resolve_pending_secret, session_argv_for_entry};
use crate::cli::context::CliContext;
use crate::cli::parse::{self, OutputFormat};
use crate::session::askpass::AskpassSecret;
use crate::session_transport::SessionTransport;

/// What `timeout(1)` exits with when it kills the child; mirrored so a script
/// can treat `sshub exec --timeout` the same way it treats `timeout`.
const EXIT_TIMEOUT: i32 = 124;

/// How often the timeout path checks on the child. std has no timed wait, and
/// 50ms is noise next to the round-trip of any real remote command.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Serialize)]
struct ExecRecord<'a> {
    host: &'a str,
    command: &'a str,
    exit_code: i32,
    stdout: String,
    stderr: String,
    duration_ms: u128,
}

pub fn run(ctx: &mut CliContext, args: &[String]) -> Result<i32> {
    let (mut head, after_dashes) = split_at_dashes(args);

    let verbose = parse::take_flag(&mut head, "--verbose") || parse::take_flag(&mut head, "-v");
    let tty = parse::take_flag(&mut head, "--tty");
    let fmt = parse::parse_format(&head).map_err(anyhow::Error::msg)?;
    parse::take_opt(&mut head, "--format");
    let timeout = match parse::take_opt(&mut head, "--timeout") {
        Some(v) => Some(Duration::from_secs(v.parse().map_err(|_| {
            anyhow::anyhow!("invalid --timeout '{v}' (whole seconds)")
        })?)),
        None => None,
    };

    let pos = parse::positional(&head);
    let Some(name) = pos.first().map(|s| s.to_string()) else {
        parse::usage("exec requires a host: sshub exec <host> -- <command>");
    };
    let command = after_dashes.unwrap_or_else(|| pos[1..].join(" "));
    if command.trim().is_empty() {
        parse::usage("exec requires a command: sshub exec <host> -- <command>");
    }

    let entry = ctx.host_by_name(&name)?.clone();
    // mosh has no one-shot command mode: `mosh host -- cmd` opens an interactive
    // session anyway, which for a script means a hang, not an error.
    if matches!(entry.session_transport(), SessionTransport::Mosh) {
        anyhow::bail!(
            "host '{name}' uses the mosh transport, which cannot run a one-shot command — \
             use `sshub connect {name}`, or switch the host to the ssh transport"
        );
    }

    let host_name = entry.name().to_string();
    let (pending_secret, _) = resolve_pending_secret(&entry, ctx.password_store.as_ref());
    let argv = exec_argv(
        prepare_cli_connect_argv(
            session_argv_for_entry(&entry),
            pending_secret.is_some(),
            verbose,
        ),
        entry.ssh_host().remote_command.as_deref(),
        &command,
        tty,
    );

    let mut askpass_guard = None;
    let mut extra_env: Vec<(String, String)> = Vec::new();
    if let Some(secret) = pending_secret.as_ref() {
        if let Ok(exe) = crate::session::askpass::helper_exe() {
            if let Ok(guard) = AskpassSecret::new(secret.value()) {
                extra_env = guard.env(&exe);
                askpass_guard = Some(guard);
            }
        }
    }

    let json = matches!(fmt, OutputFormat::Json);
    let program = argv.first().context("empty exec argv")?;
    let mut cmd = Command::new(program);
    cmd.args(&argv[1..]);
    // stdin is inherited in both modes so `echo … | sshub exec db -- 'psql -f -'`
    // works; only the output side is buffered, and only for JSON.
    cmd.stdin(Stdio::inherit());
    cmd.stdout(if json {
        Stdio::piped()
    } else {
        Stdio::inherit()
    });
    cmd.stderr(if json {
        Stdio::piped()
    } else {
        Stdio::inherit()
    });
    for (k, v) in &extra_env {
        cmd.env(k, v);
    }

    // Legacy ssh_config hosts have no managed record, so fall back to the
    // resolved SshHost rather than logging username=None.
    let ssh = entry.ssh_host();
    let username = entry
        .managed()
        .and_then(|m| {
            m.username
                .clone()
                .or_else(|| m.identity.as_ref().and_then(|i| i.username.clone()))
        })
        .or(ssh.user);

    let started = Instant::now();
    let mut child = match cmd.spawn() {
        Ok(child) => {
            // The command itself is never logged: it can carry a password in an
            // argument, and the audit log is not the place to keep one.
            let _ = ctx.store.log_auth_event(
                &host_name,
                username.as_deref(),
                "exec",
                "launched",
                "cli exec",
                None,
            );
            child
        }
        Err(e) => {
            let msg = format!("spawn failed: {e:#}");
            let _ = ctx.store.log_auth_event(
                &host_name,
                username.as_deref(),
                "exec",
                "fail",
                &msg,
                None,
            );
            eprintln!("sshub: {msg}");
            return Ok(1);
        }
    };

    // Drain the pipes on their own threads: a command that outfills the pipe
    // buffer would otherwise block forever while we poll for the timeout.
    let readers = json.then(|| (drain(child.stdout.take()), drain(child.stderr.take())));
    let (status, timed_out) = wait_or_kill(&mut child, timeout)?;
    let (stdout, stderr) = match readers {
        Some((out, err)) => (
            out.join().unwrap_or_default(),
            err.join().unwrap_or_default(),
        ),
        None => (String::new(), String::new()),
    };

    let exit_code = if timed_out {
        eprintln!("sshub: exec timed out, killed the remote command");
        EXIT_TIMEOUT
    } else {
        status.code().unwrap_or(1)
    };

    if json {
        let record = ExecRecord {
            host: &host_name,
            command: &command,
            exit_code,
            stdout,
            stderr,
            duration_ms: started.elapsed().as_millis(),
        };
        println!("{}", serde_json::to_string_pretty(&record)?);
    }

    drop(askpass_guard);
    Ok(exit_code)
}

/// Split argv at the first `--`: everything before it is ours, everything after
/// is the remote command, joined with spaces exactly the way ssh joins its own
/// trailing arguments. `None` when there is no `--` at all.
pub(crate) fn split_at_dashes(args: &[String]) -> (Vec<String>, Option<String>) {
    match args.iter().position(|a| a == "--") {
        Some(i) => (args[..i].to_vec(), Some(args[i + 1..].join(" "))),
        None => (args.to_vec(), None),
    }
}

/// The part of `rest` that may carry sshub's own `--help` / `-h`. Anything after
/// `--` belongs to the remote command: `sshub exec web -- ls -h` lists a remote
/// directory, it does not print our help.
pub(crate) fn help_scan(rest: &[String]) -> &[String] {
    match rest.iter().position(|a| a == "--") {
        Some(i) => &rest[..i],
        None => rest,
    }
}

/// Turn a connect argv into an exec argv: no PTY unless asked for, and the
/// ad-hoc command appended after the target.
///
/// A host with a stored `remote_command` already carries it as a trailing
/// `-- <cmd>` (see `build_ssh_argv`); that one is dropped, because the command
/// typed on the exec line has to win.
pub(crate) fn exec_argv(
    mut argv: Vec<String>,
    stored_remote_command: Option<&str>,
    command: &str,
    tty: bool,
) -> Vec<String> {
    if let Some(stored) = stored_remote_command.filter(|s| !s.is_empty()) {
        let n = argv.len();
        if n >= 2 && argv[n - 1] == stored && argv[n - 2] == "--" {
            argv.truncate(n - 2);
        }
    }
    // `-T` keeps this batch-friendly; `-tt` forces a PTY for the commands that
    // insist on one (sudo without NOPASSWD, top).
    argv.insert(1, if tty { "-tt" } else { "-T" }.to_string());
    argv.push("--".into());
    argv.push(command.to_string());
    argv
}

/// Read a child pipe to the end on its own thread.
fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> JoinHandle<String> {
    std::thread::spawn(move || {
        let mut buf = String::new();
        if let Some(mut pipe) = pipe {
            // Lossy on purpose: a remote command may emit anything, and exec
            // still has to report its exit code.
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            buf = String::from_utf8_lossy(&bytes).into_owned();
        }
        buf
    })
}

/// Wait for `child`, killing it once `timeout` has passed. The bool is true when
/// it was killed rather than having exited on its own.
fn wait_or_kill(child: &mut Child, timeout: Option<Duration>) -> Result<(ExitStatus, bool)> {
    let Some(limit) = timeout else {
        return Ok((child.wait().context("wait exec child")?, false));
    };
    let deadline = Instant::now() + limit;
    loop {
        if let Some(status) = child.try_wait().context("poll exec child")? {
            return Ok((status, false));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            return Ok((child.wait().context("reap timed-out exec child")?, true));
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn exec_argv_appends_the_command_after_the_target() {
        let argv = exec_argv(
            s(&["ssh", "-p", "2222", "root@10.0.0.1"]),
            None,
            "uptime",
            false,
        );
        assert_eq!(
            argv,
            s(&["ssh", "-T", "-p", "2222", "root@10.0.0.1", "--", "uptime"])
        );
    }

    #[test]
    fn exec_argv_tty_flag_forces_a_pty() {
        let argv = exec_argv(s(&["ssh", "web"]), None, "sudo reboot", true);
        assert_eq!(argv, s(&["ssh", "-tt", "web", "--", "sudo reboot"]));
    }

    #[test]
    fn exec_argv_ad_hoc_command_replaces_a_stored_remote_command() {
        // What `build_ssh_argv` produces for a host with remote_command set.
        let stored = s(&["ssh", "web", "--", "tmux attach"]);
        let argv = exec_argv(stored, Some("tmux attach"), "uptime", false);
        assert_eq!(argv, s(&["ssh", "-T", "web", "--", "uptime"]));
        assert_eq!(argv.iter().filter(|a| *a == "--").count(), 1);
    }

    #[test]
    fn exec_argv_keeps_a_trailing_word_that_only_looks_like_a_stored_command() {
        // No `--` before it, so it is the target, not a stored remote command.
        let argv = exec_argv(s(&["ssh", "web"]), Some("web"), "uptime", false);
        assert_eq!(argv, s(&["ssh", "-T", "web", "--", "uptime"]));
    }

    #[test]
    fn split_at_dashes_takes_everything_after_the_separator_as_the_command() {
        let (head, cmd) = split_at_dashes(&s(&["web", "--tty", "--", "tail", "-n", "5", "log"]));
        assert_eq!(head, s(&["web", "--tty"]));
        assert_eq!(cmd.as_deref(), Some("tail -n 5 log"));
    }

    #[test]
    fn split_at_dashes_without_a_separator_leaves_the_command_to_the_positionals() {
        let (head, cmd) = split_at_dashes(&s(&["web", "uptime"]));
        assert_eq!(head, s(&["web", "uptime"]));
        assert_eq!(cmd, None);
    }

    /// Differential test against OpenSSH: the argv `exec_argv` builds has to be
    /// one real ssh parses as *this* host, with the PTY policy we asked for.
    /// `ssh -G` resolves a command line and exits without connecting, so this
    /// needs neither network nor server. `-F /dev/null` keeps the developer's
    /// own `~/.ssh/config` out of the answer.
    ///
    /// What it does not prove: `-G` never echoes a remote command given on the
    /// command line, so nothing here checks how ssh joins the words after `--`.
    /// That claim is only covered by the unit tests above.
    #[test]
    fn argv_is_understood_by_real_ssh() {
        if Command::new("ssh").arg("-V").output().is_err() {
            eprintln!("skipping: no ssh binary");
            return;
        }

        // `-T` is spelled "no" by OpenSSH < 10 and "false" by newer ones.
        for (tty, want_tty) in [(false, ["no", "false"]), (true, ["force", "force"])] {
            let argv = exec_argv(
                s(&["ssh", "-p", "2222", "deploy@10.0.0.5"]),
                None,
                "tail -n 5 /var/log/app.log",
                tty,
            );
            let out = Command::new("ssh")
                .args(["-F", "/dev/null", "-G"])
                .args(&argv[1..])
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "ssh rejected the argv we build: {argv:?}\n{}",
                String::from_utf8_lossy(&out.stderr)
            );

            let cfg = String::from_utf8_lossy(&out.stdout);
            let field = |key: &str| {
                cfg.lines()
                    .find_map(|l| l.strip_prefix(&format!("{key} ")))
                    .map(str::trim)
            };
            // The target still resolves the way we meant it to: a command
            // appended in the wrong place would land ssh on another host.
            assert_eq!(field("hostname"), Some("10.0.0.5"), "argv {argv:?}");
            assert_eq!(field("user"), Some("deploy"), "argv {argv:?}");
            assert_eq!(field("port"), Some("2222"), "argv {argv:?}");
            let requesttty = field("requesttty").unwrap_or("<missing>");
            assert!(
                want_tty.contains(&requesttty),
                "ssh read {requesttty:?} out of {argv:?}, expected one of {want_tty:?}"
            );
        }
    }

    #[test]
    fn help_scan_ignores_flags_meant_for_the_remote_command() {
        assert!(help_scan(&s(&["web", "--", "ls", "-h"]))
            .iter()
            .all(|a| a != "-h"));
        assert!(help_scan(&s(&["web", "--help"]))
            .iter()
            .any(|a| a == "--help"));
    }
}
