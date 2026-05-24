use std::collections::HashMap;
use std::process::{Child, Command};
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

        let child = Command::new(&args[0])
            .args(&args[1..])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        self.processes.insert(
            tunnel.id,
            TunnelProcess {
                child,
                started_at: Instant::now(),
                status: TunnelStatus::Up,
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
            match proc.child.try_wait() {
                Ok(Some(status)) => {
                    if status.success() {
                        proc.status = TunnelStatus::Down;
                    } else {
                        proc.status = TunnelStatus::Error(format!("exited with {}", status));
                    }
                }
                Ok(None) => {
                    proc.status = TunnelStatus::Up;
                }
                Err(e) => {
                    proc.status = TunnelStatus::Error(format!("{e}"));
                }
            }
        }
        // Remove dead processes
        self.processes
            .retain(|_, p| matches!(p.status, TunnelStatus::Up));
    }

    pub fn status(&self, tunnel_id: i64) -> &str {
        self.processes
            .get(&tunnel_id)
            .map(|p| p.status.label())
            .unwrap_or("stopped")
    }

    pub fn is_running(&self, tunnel_id: i64) -> bool {
        self.processes.contains_key(&tunnel_id)
    }

    pub fn active_count(&self) -> usize {
        self.processes.len()
    }

    pub fn uptime_secs(&self, tunnel_id: i64) -> Option<u64> {
        self.processes
            .get(&tunnel_id)
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
