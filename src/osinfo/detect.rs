//! Background OS-detection worker.
//!
//! Mirrors [`crate::ping`]'s worker shape: a single thread blocking on an mpsc
//! `Receiver<OsDetectCmd>`, self-terminating when the command `Sender` drops.
//! For each command it shells a short, non-interactive `ssh` probe that prints
//! `/etc/os-release` (falling back to `uname -s`), feeds the output through
//! [`crate::osinfo::parse_os`], and emits an [`OsDetectEvent::Detected`] only
//! when a canonical id is recognised. Failures are silent — the host's
//! `os_icon` stays empty and the probe is retried on the next connect.

use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use crate::osinfo::parse_os;
use crate::session::PendingSecret;

/// Request to probe one host for its operating system.
pub struct OsDetectCmd {
    /// Managed host id the result is written back to.
    pub host_id: i64,
    /// Base ssh argv (from `ssh_argv_for_entry`); argv\[0\] is `ssh`.
    pub argv: Vec<String>,
    /// Stored credential to hand ssh via `SSH_ASKPASS`, if any.
    pub secret: Option<PendingSecret>,
}

/// Result of a successful OS probe.
pub enum OsDetectEvent {
    /// A canonical OS id was detected for `host_id`.
    Detected { host_id: i64, os: String },
}

/// How the probe is executed. Abstracted so tests can inject canned output
/// without spawning ssh.
pub trait ProbeRunner: Send {
    /// Run the probe for `argv` and return the combined stdout on success, or
    /// `None` on any failure (spawn error, non-zero exit).
    fn run(&self, argv: &[String], secret: Option<&PendingSecret>) -> Option<String>;
}

/// Real probe: shells out to `ssh` with non-interactive options and a remote
/// command that dumps the OS release info.
pub struct SshProbeRunner;

impl ProbeRunner for SshProbeRunner {
    fn run(&self, argv: &[String], secret: Option<&PendingSecret>) -> Option<String> {
        if argv.is_empty() {
            return None;
        }

        // Splice BatchMode-friendly options right after the program name and
        // append the remote command as the final argument.
        let mut full: Vec<String> = Vec::with_capacity(argv.len() + 8);
        full.push(argv[0].clone());
        for opt in [
            "-o",
            "BatchMode=no",
            "-o",
            "ConnectTimeout=8",
            "-o",
            "StrictHostKeyChecking=accept-new",
        ] {
            full.push(opt.to_string());
        }
        full.extend(argv[1..].iter().cloned());
        full.push("cat /etc/os-release 2>/dev/null || uname -s".to_string());

        let mut cmd = Command::new(&full[0]);
        cmd.args(&full[1..]);

        // Stage the stored secret through SSH_ASKPASS, exactly like a live
        // session does (see src/session/mod.rs). Kept alive until after the
        // child exits so its owner-only temp file is removed on drop.
        let mut _askpass = None;
        if let Some(secret) = secret {
            if let Ok(exe) = std::env::current_exe() {
                if let Ok(guard) = crate::session::askpass::AskpassSecret::new(secret.value()) {
                    for (k, v) in guard.env(&exe) {
                        cmd.env(k, v);
                    }
                    _askpass = Some(guard);
                }
            }
        }

        let output = cmd.output().ok()?;
        if !output.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

/// Spawn the OS-detection worker using the real [`SshProbeRunner`].
pub fn spawn_os_detect_worker() -> (Sender<OsDetectCmd>, Receiver<OsDetectEvent>) {
    spawn_os_detect_worker_with(SshProbeRunner)
}

/// Spawn the OS-detection worker with a custom [`ProbeRunner`] (test seam).
pub fn spawn_os_detect_worker_with<R: ProbeRunner + 'static>(
    runner: R,
) -> (Sender<OsDetectCmd>, Receiver<OsDetectEvent>) {
    let (cmd_tx, cmd_rx) = mpsc::channel::<OsDetectCmd>();
    let (event_tx, event_rx) = mpsc::channel::<OsDetectEvent>();
    thread::spawn(move || {
        detect_loop(runner, cmd_rx, event_tx);
    });
    (cmd_tx, event_rx)
}

fn detect_loop<R: ProbeRunner>(
    runner: R,
    cmd_rx: Receiver<OsDetectCmd>,
    event_tx: Sender<OsDetectEvent>,
) {
    // The for-loop ends when the command Sender drops, self-terminating the
    // thread.
    for cmd in cmd_rx {
        let Some(out) = runner.run(&cmd.argv, cmd.secret.as_ref()) else {
            continue; // probe failed — stay silent
        };
        let Some(os) = parse_os(&out) else {
            continue; // unrecognised — stay silent
        };
        let event = OsDetectEvent::Detected {
            host_id: cmd.host_id,
            os: os.to_string(),
        };
        if event_tx.send(event).is_err() {
            return; // Receiver dropped, exit thread
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockProbeRunner {
        output: Option<String>,
    }

    impl ProbeRunner for MockProbeRunner {
        fn run(&self, _argv: &[String], _secret: Option<&PendingSecret>) -> Option<String> {
            self.output.clone()
        }
    }

    #[test]
    fn detects_os_from_canned_release() {
        let runner = MockProbeRunner {
            output: Some(
                "NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\nVERSION_ID=\"22.04\"\n".to_string(),
            ),
        };
        let (tx, rx) = spawn_os_detect_worker_with(runner);
        tx.send(OsDetectCmd {
            host_id: 42,
            argv: vec!["ssh".to_string(), "host".to_string()],
            secret: None,
        })
        .unwrap();
        drop(tx); // let the worker finish

        let ev = rx.recv().expect("expected a Detected event");
        match ev {
            OsDetectEvent::Detected { host_id, os } => {
                assert_eq!(host_id, 42);
                assert_eq!(os, "ubuntu");
            }
        }
        assert!(rx.recv().is_err(), "no further events after Sender drops");
    }

    #[test]
    fn probe_failure_emits_no_event() {
        let runner = MockProbeRunner { output: None };
        let (tx, rx) = spawn_os_detect_worker_with(runner);
        tx.send(OsDetectCmd {
            host_id: 7,
            argv: vec!["ssh".to_string(), "host".to_string()],
            secret: None,
        })
        .unwrap();
        drop(tx);

        assert!(rx.recv().is_err(), "failed probe must emit nothing");
    }

    #[test]
    fn unrecognised_output_emits_no_event() {
        let runner = MockProbeRunner {
            output: Some("some totally unrelated banner text\n".to_string()),
        };
        let (tx, rx) = spawn_os_detect_worker_with(runner);
        tx.send(OsDetectCmd {
            host_id: 1,
            argv: vec!["ssh".to_string(), "host".to_string()],
            secret: None,
        })
        .unwrap();
        drop(tx);

        assert!(rx.recv().is_err(), "unparsable output must emit nothing");
    }
}
