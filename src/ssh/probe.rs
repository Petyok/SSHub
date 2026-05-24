//! SSH connection probe — runs `ssh -v` in the background to capture verbose logs.

use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct SshLogEntry {
    pub host_name: String,
    pub line: String,
    pub level: LogLevel,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Success,
    Error,
}

/// Spawn a background thread that probes hosts via `ssh -v` and sends log lines.
pub fn spawn_ssh_probe(
    hosts: Vec<(String, String, u16)>,
    interval: Duration,
) -> Receiver<SshLogEntry> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        probe_loop(hosts, interval, tx);
    });
    rx
}

fn probe_loop(hosts: Vec<(String, String, u16)>, interval: Duration, tx: Sender<SshLogEntry>) {
    loop {
        for (name, address, port) in &hosts {
            let lines = probe_once(name, address, *port);
            for entry in lines {
                if tx.send(entry).is_err() {
                    return;
                }
            }
        }
        thread::sleep(interval);
    }
}

fn probe_once(name: &str, address: &str, port: u16) -> Vec<SshLogEntry> {
    let output = Command::new("ssh")
        .args([
            "-v",
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=5",
            "-o",
            "StrictHostKeyChecking=no",
            "-p",
            &port.to_string(),
            address,
            "exit",
            "0",
        ])
        .output();

    let (stderr, succeeded) = match &output {
        Ok(out) => (
            String::from_utf8_lossy(&out.stderr).to_string(),
            out.status.success(),
        ),
        Err(e) => (format!("probe failed: {e}"), false),
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let mut entries: Vec<SshLogEntry> = stderr
        .lines()
        .filter(|l| !l.is_empty())
        .filter(|line| is_meaningful_line(line))
        .map(|line| {
            let level = classify_line(line);
            let clean = strip_debug_prefix(line);
            SshLogEntry {
                host_name: name.to_string(),
                line: clean,
                level,
                timestamp: now,
            }
        })
        .collect();

    // Append a summary entry for the overall probe result.
    let (summary, level) = if succeeded {
        (format!("\u{2713} {name}: connected"), LogLevel::Success)
    } else {
        (
            format!("\u{2717} {name}: connection failed"),
            LogLevel::Error,
        )
    };
    entries.push(SshLogEntry {
        host_name: name.to_string(),
        line: summary,
        level,
        timestamp: now,
    });

    entries
}

/// Returns `true` for stderr lines that carry useful information.
/// Filters out the bulk of `debug1:` noise, keeping only errors,
/// success markers, connection attempts, identity file probes, and
/// connection-status lines.
fn is_meaningful_line(line: &str) -> bool {
    let lower = line.to_lowercase();

    // Always keep error indicators.
    if lower.contains("error")
        || lower.contains("denied")
        || lower.contains("refused")
        || lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("no route")
        || lower.contains("reset by peer")
        || lower.contains("connection closed")
    {
        return true;
    }

    // Always keep success indicators.
    if lower.contains("authenticated")
        || lower.contains("session opened")
        || lower.contains("connection established")
    {
        return true;
    }

    // Keep connection-attempt lines.
    if lower.contains("connecting to") {
        return true;
    }

    // Keep identity file lines that mention a type (key being tried).
    if lower.contains("identity file") && lower.contains("type") {
        return true;
    }

    // Keep "debug1: Connection ..." status lines.
    if line.starts_with("debug1: Connection") {
        return true;
    }

    // Drop all other debug1: lines — they are noise.
    if line.starts_with("debug1:") {
        return false;
    }

    // Non-debug1 lines (e.g. plain errors from ssh) are kept.
    true
}

/// Strip the "debug1: " prefix so the log reads cleaner.
fn strip_debug_prefix(line: &str) -> String {
    if let Some(rest) = line.strip_prefix("debug1: ") {
        rest.to_string()
    } else {
        line.to_string()
    }
}

fn classify_line(line: &str) -> LogLevel {
    let l = line.to_lowercase();
    if l.contains("error")
        || l.contains("denied")
        || l.contains("refused")
        || l.contains("timeout")
        || l.contains("timed out")
        || l.contains("no route")
        || l.contains("reset by peer")
        || l.contains("connection closed")
    {
        LogLevel::Error
    } else if l.contains("authentication")
        || l.contains("authenticated")
        || l.contains("session opened")
        || l.contains("channel")
        || l.contains("connection established")
    {
        LogLevel::Success
    } else {
        LogLevel::Info
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_error_lines() {
        assert_eq!(
            classify_line("Permission denied (publickey)"),
            LogLevel::Error
        );
        assert_eq!(
            classify_line("ssh: connect to host 1.2.3.4 port 22: Connection refused"),
            LogLevel::Error
        );
        assert_eq!(
            classify_line("ssh: connect to host 1.2.3.4 port 22: Connection timed out"),
            LogLevel::Error
        );
    }

    #[test]
    fn classify_success_lines() {
        assert_eq!(
            classify_line("debug1: Authentication succeeded (publickey)"),
            LogLevel::Success
        );
        assert_eq!(classify_line("debug1: channel 0: new"), LogLevel::Success);
    }

    #[test]
    fn classify_info_lines() {
        assert_eq!(
            classify_line("debug1: Connecting to 10.0.0.1 port 22"),
            LogLevel::Info
        );
        assert_eq!(
            classify_line("debug1: SSH2_MSG_KEXINIT sent"),
            LogLevel::Info
        );
    }
}
