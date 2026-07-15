use std::collections::{HashMap, HashSet};
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::Result;

use crate::config::{tunnel_backoff_delay, TunnelReconnectConfig};
use crate::store::{ManagedHost, Tunnel, TunnelType};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconnectEvent {
    Attempt {
        tunnel_id: i64,
        attempt: u32,
    },
    Reconnected {
        tunnel_id: i64,
    },
    GaveUp {
        tunnel_id: i64,
        attempts: u32,
        error: String,
    },
}

impl ReconnectEvent {
    pub fn tunnel_id(&self) -> i64 {
        match self {
            Self::Attempt { tunnel_id, .. }
            | Self::Reconnected { tunnel_id }
            | Self::GaveUp { tunnel_id, .. } => *tunnel_id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReconnectPhase {
    Reconnecting,
    GaveUp,
}

#[derive(Debug, Clone)]
struct ReconnectState {
    attempt: u32,
    next_retry: Instant,
    phase: ReconnectPhase,
    last_error: String,
}

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
    reconnect: HashMap<i64, ReconnectState>,
    user_stopped: HashSet<i64>,
    /// Last error for tunnels without keep-alive (no live process entry).
    terminal_errors: HashMap<i64, String>,
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
            reconnect: HashMap::new(),
            user_stopped: HashSet::new(),
            terminal_errors: HashMap::new(),
        }
    }

    pub fn start(&mut self, tunnel: &Tunnel, host: Option<&ManagedHost>) -> Result<()> {
        if self.processes.contains_key(&tunnel.id) {
            self.stop_process(tunnel.id)?;
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
        self.reconnect.remove(&tunnel.id);
        self.terminal_errors.remove(&tunnel.id);

        Ok(())
    }

    /// User-initiated stop: kill the child and suppress auto-reconnect.
    pub fn stop_user(&mut self, tunnel_id: i64) -> Result<()> {
        self.user_stopped.insert(tunnel_id);
        self.reconnect.remove(&tunnel_id);
        self.terminal_errors.remove(&tunnel_id);
        self.stop_process(tunnel_id)
    }

    fn stop_process(&mut self, tunnel_id: i64) -> Result<()> {
        if let Some(mut proc) = self.processes.remove(&tunnel_id) {
            let _ = proc.child.kill();
            let _ = proc.child.wait();
        }
        Ok(())
    }

    pub fn mark_user_stopped(&mut self, tunnel_id: i64) {
        self.user_stopped.insert(tunnel_id);
        self.reconnect.remove(&tunnel_id);
    }

    pub fn resume_auto_reconnect(&mut self, tunnel_id: i64) {
        self.user_stopped.remove(&tunnel_id);
        self.reconnect.remove(&tunnel_id);
        self.terminal_errors.remove(&tunnel_id);
    }

    pub fn is_reconnecting(&self, tunnel_id: i64) -> bool {
        self.reconnect
            .get(&tunnel_id)
            .is_some_and(|r| r.phase == ReconnectPhase::Reconnecting)
    }

    pub fn is_gave_up(&self, tunnel_id: i64) -> bool {
        self.reconnect
            .get(&tunnel_id)
            .is_some_and(|r| r.phase == ReconnectPhase::GaveUp)
    }

    pub fn needs_tunnel_list(&self) -> bool {
        !self.processes.is_empty() || !self.reconnect.is_empty()
    }

    pub fn check_health(&mut self, tunnels: &[Tunnel], cfg: &TunnelReconnectConfig) {
        let mut exited: Vec<(i64, String)> = Vec::new();
        for (&id, proc) in self.processes.iter_mut() {
            if !matches!(proc.status, TunnelStatus::Up) {
                continue;
            }
            match proc.child.try_wait() {
                Ok(Some(status)) => {
                    let detail = proc
                        .stderr_tail
                        .lock()
                        .ok()
                        .and_then(|s| s.lines().last().map(|l| l.trim().to_string()))
                        .unwrap_or_default();
                    let err = if status.success() {
                        "tunnel exited".into()
                    } else if detail.is_empty() {
                        format!("exited with {status}")
                    } else {
                        detail
                    };
                    exited.push((id, err));
                }
                Ok(None) => {}
                Err(e) => {
                    exited.push((id, format!("{e}")));
                }
            }
        }

        for (id, err) in exited {
            let _ = self.processes.remove(&id);
            let auto = tunnels
                .iter()
                .find(|t| t.id == id)
                .is_some_and(|t| t.auto_connect);
            if auto && !self.user_stopped.contains(&id) {
                self.on_auto_start_failed(id, &err, cfg);
            } else if !auto {
                self.terminal_errors.insert(id, err);
            }
        }
    }

    /// Schedule reconnect after a keep-alive tunnel failed to start or exited.
    pub fn on_auto_start_failed(&mut self, tunnel_id: i64, err: &str, cfg: &TunnelReconnectConfig) {
        if self.user_stopped.contains(&tunnel_id) {
            return;
        }
        self.schedule_reconnect(tunnel_id, err, cfg, 0);
    }

    fn schedule_reconnect(
        &mut self,
        tunnel_id: i64,
        err: &str,
        cfg: &TunnelReconnectConfig,
        attempt: u32,
    ) {
        let delay = tunnel_backoff_delay(attempt.max(1), tunnel_id, cfg);
        self.reconnect.insert(
            tunnel_id,
            ReconnectState {
                attempt,
                next_retry: Instant::now() + delay,
                phase: ReconnectPhase::Reconnecting,
                last_error: err.to_string(),
            },
        );
    }

    pub fn tick_reconnect(
        &mut self,
        tunnels: &[Tunnel],
        cfg: &TunnelReconnectConfig,
        resolve_host: impl Fn(i64) -> Option<ManagedHost>,
    ) -> Vec<ReconnectEvent> {
        let now = Instant::now();
        let live_ids: std::collections::HashSet<i64> = tunnels.iter().map(|t| t.id).collect();
        self.reconnect.retain(|id, _| live_ids.contains(id));
        self.terminal_errors.retain(|id, _| live_ids.contains(id));

        let due: Vec<i64> = self
            .reconnect
            .iter()
            .filter(|(_, r)| r.phase == ReconnectPhase::Reconnecting && now >= r.next_retry)
            .map(|(&id, _)| id)
            .collect();

        let mut events = Vec::new();
        for tunnel_id in due {
            let Some(tunnel) = tunnels.iter().find(|t| t.id == tunnel_id) else {
                self.reconnect.remove(&tunnel_id);
                continue;
            };
            if !tunnel.auto_connect || self.user_stopped.contains(&tunnel_id) {
                self.reconnect.remove(&tunnel_id);
                continue;
            }

            let attempt = self
                .reconnect
                .get(&tunnel_id)
                .map(|r| r.attempt + 1)
                .unwrap_or(1);
            events.push(ReconnectEvent::Attempt { tunnel_id, attempt });

            let host = tunnel.host_id.and_then(|hid| resolve_host(hid));
            match self.start(tunnel, host.as_ref()) {
                Ok(()) => {
                    self.reconnect.remove(&tunnel_id);
                    events.push(ReconnectEvent::Reconnected { tunnel_id });
                }
                Err(e) => {
                    let err = format!("{e:#}");
                    let max = cfg.max_attempts;
                    if max > 0 && attempt >= max {
                        self.reconnect.insert(
                            tunnel_id,
                            ReconnectState {
                                attempt,
                                next_retry: now,
                                phase: ReconnectPhase::GaveUp,
                                last_error: err.clone(),
                            },
                        );
                        events.push(ReconnectEvent::GaveUp {
                            tunnel_id,
                            attempts: attempt,
                            error: err,
                        });
                    } else {
                        let delay = tunnel_backoff_delay(attempt + 1, tunnel_id, cfg);
                        self.reconnect.insert(
                            tunnel_id,
                            ReconnectState {
                                attempt,
                                next_retry: now + delay,
                                phase: ReconnectPhase::Reconnecting,
                                last_error: err,
                            },
                        );
                    }
                }
            }
        }
        events
    }

    pub fn status(&self, tunnel_id: i64) -> &str {
        if self
            .processes
            .get(&tunnel_id)
            .is_some_and(|p| matches!(p.status, TunnelStatus::Up))
        {
            return "up";
        }
        match self.reconnect.get(&tunnel_id).map(|r| r.phase) {
            Some(ReconnectPhase::Reconnecting) => return "reconnecting",
            Some(ReconnectPhase::GaveUp) => return "gave_up",
            None => {}
        }
        if self.terminal_errors.contains_key(&tunnel_id) {
            return "error";
        }
        if self.processes.contains_key(&tunnel_id) {
            return self.processes.get(&tunnel_id).unwrap().status.label();
        }
        "stopped"
    }

    pub fn error_detail(&self, tunnel_id: i64) -> Option<&str> {
        if let Some(TunnelStatus::Error(msg)) = self.processes.get(&tunnel_id).map(|p| &p.status) {
            return Some(msg.as_str());
        }
        if let Some(err) = self.terminal_errors.get(&tunnel_id) {
            return Some(err.as_str());
        }
        self.reconnect
            .get(&tunnel_id)
            .map(|r| r.last_error.as_str())
    }

    pub fn reconnect_attempt(&self, tunnel_id: i64) -> Option<u32> {
        self.reconnect.get(&tunnel_id).map(|r| r.attempt)
    }

    pub fn reconnect_countdown_secs(&self, tunnel_id: i64) -> Option<u64> {
        let r = self.reconnect.get(&tunnel_id)?;
        if r.phase != ReconnectPhase::Reconnecting {
            return None;
        }
        Some(
            r.next_retry
                .saturating_duration_since(Instant::now())
                .as_secs(),
        )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::TunnelType;
    use std::time::Duration;

    fn sample_tunnel(id: i64, auto_connect: bool) -> Tunnel {
        Tunnel {
            id,
            host_id: Some(1),
            tunnel_type: TunnelType::Local,
            local_port: 8080,
            remote_host: "localhost".into(),
            remote_port: 80,
            label: Some("test".into()),
            auto_connect,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn mark_user_stopped_clears_reconnect_state() {
        let mut mgr = TunnelManager::new();
        mgr.schedule_reconnect(1, "gone", &TunnelReconnectConfig::default(), 0);
        assert!(mgr.is_reconnecting(1));
        mgr.mark_user_stopped(1);
        assert!(!mgr.is_reconnecting(1));
    }

    #[test]
    fn resume_clears_user_stopped_and_gave_up() {
        let mut mgr = TunnelManager::new();
        mgr.mark_user_stopped(1);
        mgr.reconnect.insert(
            1,
            ReconnectState {
                attempt: 3,
                next_retry: Instant::now(),
                phase: ReconnectPhase::GaveUp,
                last_error: "fail".into(),
            },
        );
        mgr.resume_auto_reconnect(1);
        assert!(!mgr.user_stopped.contains(&1));
        assert!(!mgr.is_gave_up(1));
    }

    #[test]
    fn status_reports_reconnecting_and_gave_up() {
        let mut mgr = TunnelManager::new();
        mgr.reconnect.insert(
            2,
            ReconnectState {
                attempt: 1,
                next_retry: Instant::now() + Duration::from_secs(5),
                phase: ReconnectPhase::Reconnecting,
                last_error: "wait".into(),
            },
        );
        assert_eq!(mgr.status(2), "reconnecting");
        let secs = mgr.reconnect_countdown_secs(2).unwrap();
        assert!(secs >= 4 && secs <= 5);

        mgr.reconnect.insert(
            2,
            ReconnectState {
                attempt: 12,
                next_retry: Instant::now(),
                phase: ReconnectPhase::GaveUp,
                last_error: "done".into(),
            },
        );
        assert_eq!(mgr.status(2), "gave_up");
        assert_eq!(mgr.error_detail(2), Some("done"));
    }
}
