use std::collections::{HashMap, HashSet};
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::config::{tunnel_backoff_delay, tunnel_failure_attempt, TunnelReconnectConfig};
use crate::session::{askpass, PendingSecret};
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
    /// True until the child survives [`TunnelReconnectConfig::stable_secs`].
    proving: bool,
    stderr_tail: Arc<Mutex<String>>,
    _askpass: Option<askpass::AskpassSecret>,
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

    pub fn start(
        &mut self,
        tunnel: &Tunnel,
        host: Option<&ManagedHost>,
        secret: Option<&PendingSecret>,
    ) -> Result<()> {
        if self.processes.contains_key(&tunnel.id) {
            self.stop_process(tunnel.id)?;
        }

        let mut args: Vec<String> = vec!["ssh".into(), "-N".into()];
        splice_tunnel_ssh_options(&mut args, secret.is_some());

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

        let mut cmd = Command::new(&args[0]);
        cmd.args(&args[1..])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped());

        let askpass = stage_tunnel_askpass(&mut cmd, secret)?;

        let mut child = cmd.spawn()?;

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
                proving: true,
                stderr_tail,
                _askpass: askpass,
            },
        );
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

    pub fn clear_user_stopped(&mut self, tunnel_id: i64) {
        self.user_stopped.remove(&tunnel_id);
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

    pub fn check_health(
        &mut self,
        tunnels: &[Tunnel],
        cfg: &TunnelReconnectConfig,
    ) -> Vec<ReconnectEvent> {
        let mut events = Vec::new();
        let stable = Duration::from_secs(cfg.stable_secs.max(1));
        let mut exited: Vec<(i64, String, Duration)> = Vec::new();
        let mut stabilized: Vec<i64> = Vec::new();

        for (&id, proc) in self.processes.iter_mut() {
            if !matches!(proc.status, TunnelStatus::Up) {
                continue;
            }
            if proc.proving && proc.started_at.elapsed() >= stable {
                proc.proving = false;
                stabilized.push(id);
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
                    exited.push((id, err, proc.started_at.elapsed()));
                }
                Ok(None) => {}
                Err(e) => {
                    exited.push((id, format!("{e}"), proc.started_at.elapsed()));
                }
            }
        }

        for id in stabilized {
            if self.reconnect.remove(&id).is_some() {
                events.push(ReconnectEvent::Reconnected { tunnel_id: id });
            }
        }

        for (id, err, uptime) in exited {
            let _ = self.processes.remove(&id);
            let auto = tunnels
                .iter()
                .find(|t| t.id == id)
                .is_some_and(|t| t.auto_connect);
            if auto && !self.user_stopped.contains(&id) {
                events.extend(self.record_tunnel_failure(id, &err, cfg, uptime));
            } else if !auto {
                self.terminal_errors.insert(id, err);
            }
        }
        events
    }

    fn record_tunnel_failure(
        &mut self,
        tunnel_id: i64,
        err: &str,
        cfg: &TunnelReconnectConfig,
        uptime: Duration,
    ) -> Vec<ReconnectEvent> {
        if self.user_stopped.contains(&tunnel_id) {
            return Vec::new();
        }
        let current = self
            .reconnect
            .get(&tunnel_id)
            .map(|r| r.attempt)
            .unwrap_or(0);
        let next = tunnel_failure_attempt(current, uptime.as_secs(), cfg.stable_secs.max(1));
        if cfg.max_attempts > 0 && next >= cfg.max_attempts {
            self.reconnect.insert(
                tunnel_id,
                ReconnectState {
                    attempt: next,
                    next_retry: Instant::now(),
                    phase: ReconnectPhase::GaveUp,
                    last_error: err.to_string(),
                },
            );
            return vec![ReconnectEvent::GaveUp {
                tunnel_id,
                attempts: next,
                error: err.to_string(),
            }];
        }
        self.schedule_reconnect(tunnel_id, err, cfg, next);
        Vec::new()
    }

    /// Schedule reconnect after a keep-alive tunnel failed to spawn at bootstrap.
    /// Returns a gave-up event when the retry budget is exhausted.
    pub fn on_auto_start_failed(
        &mut self,
        tunnel_id: i64,
        err: &str,
        cfg: &TunnelReconnectConfig,
    ) -> Option<ReconnectEvent> {
        if self.user_stopped.contains(&tunnel_id) {
            return None;
        }
        let current = self
            .reconnect
            .get(&tunnel_id)
            .map(|r| r.attempt)
            .unwrap_or(0);
        let next = if self.reconnect.contains_key(&tunnel_id) {
            tunnel_failure_attempt(current, 0, cfg.stable_secs.max(1))
        } else {
            0
        };
        if cfg.max_attempts > 0 && next >= cfg.max_attempts {
            self.reconnect.insert(
                tunnel_id,
                ReconnectState {
                    attempt: next,
                    next_retry: Instant::now(),
                    phase: ReconnectPhase::GaveUp,
                    last_error: err.to_string(),
                },
            );
            return Some(ReconnectEvent::GaveUp {
                tunnel_id,
                attempts: next,
                error: err.to_string(),
            });
        }
        self.schedule_reconnect(tunnel_id, err, cfg, next);
        None
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
        resolve_secret: impl Fn(&ManagedHost) -> Option<PendingSecret>,
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
            if self.processes.contains_key(&tunnel_id) {
                continue;
            }
            let Some(tunnel) = tunnels.iter().find(|t| t.id == tunnel_id) else {
                self.reconnect.remove(&tunnel_id);
                continue;
            };
            if !tunnel.auto_connect || self.user_stopped.contains(&tunnel_id) {
                self.reconnect.remove(&tunnel_id);
                continue;
            }

            let stored = self
                .reconnect
                .get(&tunnel_id)
                .map(|r| r.attempt)
                .unwrap_or(0);
            let attempt = stored.saturating_add(1);
            let next_on_fail = tunnel_failure_attempt(stored, 0, cfg.stable_secs.max(1));
            let is_final_try = cfg.max_attempts > 0 && next_on_fail >= cfg.max_attempts;

            if !is_final_try {
                events.push(ReconnectEvent::Attempt { tunnel_id, attempt });
            }

            let host = tunnel.host_id.and_then(&resolve_host);
            let secret = host.as_ref().and_then(&resolve_secret);
            match self.start(tunnel, host.as_ref(), secret.as_ref()) {
                Ok(()) => {}
                Err(e) => {
                    let err = format!("{e:#}");
                    events.extend(self.record_tunnel_failure(tunnel_id, &err, cfg, Duration::ZERO));
                }
            }
        }
        events
    }

    pub fn status(&self, tunnel_id: i64) -> &str {
        if let Some(proc) = self.processes.get(&tunnel_id) {
            if matches!(proc.status, TunnelStatus::Up) {
                if proc.proving {
                    if self.is_reconnecting(tunnel_id) {
                        return "reconnecting";
                    }
                    return "starting";
                }
                return "up";
            }
            return proc.status.label();
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

    pub fn has_child(&self, tunnel_id: i64) -> bool {
        self.processes.contains_key(&tunnel_id)
    }

    pub fn is_running(&self, tunnel_id: i64) -> bool {
        self.processes
            .get(&tunnel_id)
            .is_some_and(|p| matches!(p.status, TunnelStatus::Up) && !p.proving)
    }

    pub fn active_count(&self) -> usize {
        self.processes
            .values()
            .filter(|p| matches!(p.status, TunnelStatus::Up) && !p.proving)
            .count()
    }

    pub fn uptime_secs(&self, tunnel_id: i64) -> Option<u64> {
        self.processes
            .get(&tunnel_id)
            .filter(|p| matches!(p.status, TunnelStatus::Up) && !p.proving)
            .map(|p| p.started_at.elapsed().as_secs())
    }
}

/// Insert non-interactive ssh options after the `ssh` argv0. Background tunnels
/// must never open `/dev/tty` for a password prompt — that writes over the TUI
/// and steals mouse/keyboard input from crossterm.
fn splice_tunnel_ssh_options(args: &mut Vec<String>, has_stored_secret: bool) {
    if args.first().map(String::as_str) != Some("ssh") {
        return;
    }
    let batchmode = if has_stored_secret {
        "BatchMode=no"
    } else {
        "BatchMode=yes"
    };
    let mut opts = vec![
        "-o".to_string(),
        batchmode.to_string(),
        "-o".to_string(),
        "ConnectTimeout=30".to_string(),
        "-o".to_string(),
        "ServerAliveInterval=10".to_string(),
        "-o".to_string(),
        "ServerAliveCountMax=3".to_string(),
        "-o".to_string(),
        "TCPKeepAlive=yes".to_string(),
    ];
    if has_stored_secret {
        opts.push("-o".to_string());
        opts.push("StrictHostKeyChecking=accept-new".to_string());
    }
    for (i, opt) in opts.into_iter().enumerate() {
        args.insert(1 + i, opt);
    }
}

fn stage_tunnel_askpass(
    cmd: &mut Command,
    secret: Option<&PendingSecret>,
) -> Result<Option<askpass::AskpassSecret>> {
    let Some(secret) = secret else {
        return Ok(None);
    };
    let exe = std::env::current_exe()?;
    let guard = askpass::AskpassSecret::new(secret.value())?;
    for (k, v) in guard.env(&exe) {
        cmd.env(k, v);
    }
    Ok(Some(guard))
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
    use std::time::Duration;

    #[test]
    fn splice_tunnel_ssh_options_forces_batchmode_without_secret() {
        let mut args = vec!["ssh".into(), "-N".into(), "host".into()];
        splice_tunnel_ssh_options(&mut args, false);
        assert!(args.windows(2).any(|w| w == ["-o", "BatchMode=yes"]));
        assert!(!args.iter().any(|a| a.contains("accept-new")));
    }

    #[test]
    fn splice_tunnel_ssh_options_allows_askpass_with_secret() {
        let mut args = vec!["ssh".into(), "-N".into(), "host".into()];
        splice_tunnel_ssh_options(&mut args, true);
        assert!(args.windows(2).any(|w| w == ["-o", "BatchMode=no"]));
        assert!(args
            .windows(2)
            .any(|w| w == ["-o", "StrictHostKeyChecking=accept-new"]));
    }

    #[test]
    fn splice_tunnel_ssh_options_includes_keepalive() {
        let mut args = vec!["ssh".into(), "-N".into(), "host".into()];
        splice_tunnel_ssh_options(&mut args, false);
        assert!(args
            .windows(2)
            .any(|w| w == ["-o", "ServerAliveInterval=10"]));
        assert!(args
            .windows(2)
            .any(|w| w == ["-o", "ServerAliveCountMax=3"]));
        assert!(args.windows(2).any(|w| w == ["-o", "TCPKeepAlive=yes"]));
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
    fn on_auto_start_failed_gives_up_at_max_attempts() {
        let mut mgr = TunnelManager::new();
        let cfg = TunnelReconnectConfig {
            max_attempts: 2,
            ..Default::default()
        };
        // After one recorded failure (attempt 1), the next start failure exhausts budget.
        mgr.schedule_reconnect(1, "err1", &cfg, 1);
        let ev = mgr.on_auto_start_failed(1, "err2", &cfg);
        assert!(matches!(
            ev,
            Some(ReconnectEvent::GaveUp {
                tunnel_id: 1,
                attempts: 2,
                ..
            })
        ));
        assert!(mgr.is_gave_up(1));
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
        assert!((4..=5).contains(&secs));

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
