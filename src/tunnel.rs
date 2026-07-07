use std::collections::HashMap;
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::Result;

use crate::store::{ManagedHost, Tunnel, TunnelType};

#[derive(Debug)]
pub enum TunnelStatus {
    Up,
    Down,
    Error(String),
}

impl TunnelStatus {
    pub fn label(&self) -> &str {
        match self {
            TunnelStatus::Up => "up",
            TunnelStatus::Down => "down",
            TunnelStatus::Error(_) => "error",
        }
    }
}

struct TunnelProcess {
    child: Child,
    started_at: Instant,
    status: TunnelStatus,
    /// Live tail of the child's stderr, drained by a background thread so a
    /// chatty `ssh -N` can't fill the pipe buffer and stall the forwarding.
    stderr_tail: Arc<Mutex<String>>,
}

pub struct TunnelManager {
    processes: HashMap<i64, TunnelProcess>,
}

impl Default for TunnelManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TunnelManager {
    pub fn new() -> Self {
        Self {
            processes: HashMap::new(),
        }
    }

    pub fn start(&mut self, tunnel: &Tunnel, host: Option<&ManagedHost>) -> Result<()> {
        if self.processes.contains_key(&tunnel.id) {
            self.stop(tunnel.id)?;
        }

        let mut args: Vec<String> = vec!["ssh".into(), "-N".into()];

        let flag = match tunnel.tunnel_type {
            TunnelType::Local => "-L",
            TunnelType::Remote => "-R",
            TunnelType::Dynamic => "-D",
        };

        let spec = if tunnel.tunnel_type == TunnelType::Dynamic {
            format!("{}", tunnel.local_port)
        } else {
            format!(
                "{}:{}:{}",
                tunnel.local_port, tunnel.remote_host, tunnel.remote_port
            )
        };

        args.push(flag.into());
        args.push(spec);

        if let Some(host) = host {
            if host.port != 22 {
                args.push("-p".into());
                args.push(host.port.to_string());
            }
            if let Some(ref jump) = host.proxy_jump {
                if !jump.is_empty() {
                    args.push("-J".into());
                    args.push(jump.clone());
                }
            }
            if host.forward_agent {
                args.push("-A".into());
            }
            if let Some(ref identity) = host.identity {
                if let Some(ref key) = identity.private_key {
                    args.push("-i".into());
                    args.push(key.to_string_lossy().into_owned());
                }
            }
            let target = if let Some(ref username) = host.username {
                format!("{}@{}", username, host.address)
            } else if let Some(ref identity) = host.identity {
                if let Some(ref u) = identity.username {
                    format!("{}@{}", u, host.address)
                } else {
                    host.address.clone()
                }
            } else {
                host.address.clone()
            };
            args.push(target);
        } else {
            anyhow::bail!("No host associated with tunnel");
        }

        let mut child = Command::new(&args[0])
            .args(&args[1..])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        // Drain stderr continuously on a background thread into a bounded tail.
        // Reading it only after exit (as before) let the ~64KB pipe buffer fill
        // and block ssh's writes, freezing a long-lived tunnel.
        let stderr_tail = Arc::new(Mutex::new(String::new()));
        if let Some(err) = child.stderr.take() {
            let buf = Arc::clone(&stderr_tail);
            let _ = std::thread::Builder::new()
                .name("sshub-tunnel-stderr".into())
                .spawn(move || {
                    use std::io::{BufRead, BufReader};
                    for line in BufReader::new(err).lines().map_while(Result::ok) {
                        if let Ok(mut s) = buf.lock() {
                            s.push_str(&line);
                            s.push('\n');
                            if s.len() > 8192 {
                                let cut = s.len() - 4096;
                                s.drain(..cut);
                            }
                        }
                    }
                });
        }

        self.processes.insert(
            tunnel.id,
            TunnelProcess {
                child,
                started_at: Instant::now(),
                status: TunnelStatus::Up,
                stderr_tail,
            },
        );

        Ok(())
    }

    pub fn stop(&mut self, tunnel_id: i64) -> Result<()> {
        if let Some(mut proc) = self.processes.remove(&tunnel_id) {
            let _ = proc.child.kill();
            let _ = proc.child.wait();
        }
        Ok(())
    }

    pub fn check_health(&mut self) {
        for proc in self.processes.values_mut() {
            // Once a tunnel has terminated, keep its final status so the UI
            // can show *why* it stopped; the entry is cleared on stop()/start().
            if !matches!(proc.status, TunnelStatus::Up) {
                continue;
            }
            match proc.child.try_wait() {
                Ok(Some(status)) => {
                    // The background reader has the ssh error; grab its last line.
                    let detail = proc
                        .stderr_tail
                        .lock()
                        .ok()
                        .and_then(|s| s.lines().last().map(|l| l.trim().to_string()))
                        .unwrap_or_default();
                    proc.status = if status.success() {
                        TunnelStatus::Down
                    } else if detail.is_empty() {
                        TunnelStatus::Error(format!("exited with {}", status))
                    } else {
                        TunnelStatus::Error(detail)
                    };
                }
                Ok(None) => {}
                Err(e) => {
                    proc.status = TunnelStatus::Error(format!("{e}"));
                }
            }
        }
    }

    pub fn status(&self, tunnel_id: i64) -> &str {
        self.processes
            .get(&tunnel_id)
            .map(|p| p.status.label())
            .unwrap_or("stopped")
    }

    /// Detailed error message for a failed tunnel, if any.
    pub fn error_detail(&self, tunnel_id: i64) -> Option<&str> {
        match self.processes.get(&tunnel_id).map(|p| &p.status) {
            Some(TunnelStatus::Error(msg)) => Some(msg.as_str()),
            _ => None,
        }
    }

    pub fn is_running(&self, tunnel_id: i64) -> bool {
        self.processes
            .get(&tunnel_id)
            .is_some_and(|p| matches!(p.status, TunnelStatus::Up))
    }

    pub fn active_count(&self) -> usize {
        self.processes
            .values()
            .filter(|p| matches!(p.status, TunnelStatus::Up))
            .count()
    }

    pub fn uptime_secs(&self, tunnel_id: i64) -> Option<u64> {
        self.processes
            .get(&tunnel_id)
            .filter(|p| matches!(p.status, TunnelStatus::Up))
            .map(|p| p.started_at.elapsed().as_secs())
    }
}

impl Drop for TunnelManager {
    fn drop(&mut self) {
        for (_, mut proc) in self.processes.drain() {
            let _ = proc.child.kill();
            let _ = proc.child.wait();
        }
    }
}
