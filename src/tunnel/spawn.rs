use std::fs;
use std::io::Write;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

use crate::config;
use crate::secure_fs;
use crate::session::{askpass, PendingSecret};
use crate::store::{ManagedHost, Tunnel, TunnelType};

/// Runtime liveness for CLI `tunnel list` / `show`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelRuntimeState {
    Running,
    Stopped,
    External,
    Unknown,
}

impl TunnelRuntimeState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Stopped => "stopped",
            Self::External => "external",
            Self::Unknown => "unknown",
        }
    }
}

/// Build the full `ssh -N …` argv for a tunnel (without spawning).
pub fn build_tunnel_argv(
    tunnel: &Tunnel,
    host: &ManagedHost,
    has_stored_secret: bool,
) -> Result<Vec<String>> {
    let mut args: Vec<String> = vec!["ssh".into(), "-N".into()];
    splice_tunnel_ssh_options(&mut args, has_stored_secret);

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
    Ok(args)
}

/// Insert non-interactive ssh options after the `ssh` argv0. Background tunnels
/// must never open `/dev/tty` for a password prompt — that writes over the TUI
/// and steals mouse/keyboard input from crossterm.
pub fn splice_tunnel_ssh_options(args: &mut Vec<String>, has_stored_secret: bool) {
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

pub fn stage_tunnel_askpass(
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

/// Ensure `data_dir/tunnels/` exists with owner-only permissions.
pub fn ensure_tunnel_pid_dir(data_dir: &Path) -> Result<PathBuf> {
    let dir = data_dir.join("tunnels");
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    secure_fs::restrict_dir(&dir);
    Ok(dir)
}

pub fn tunnel_pid_path(pid_dir: &Path, tunnel_id: i64) -> PathBuf {
    pid_dir.join(format!("{tunnel_id}.pid"))
}

pub fn read_tunnel_pid(path: &Path) -> Result<Option<u32>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let pid: u32 = raw
        .trim()
        .parse()
        .with_context(|| format!("invalid pid in {}", path.display()))?;
    Ok(Some(pid))
}

pub fn write_tunnel_pid(path: &Path, pid: u32) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        secure_fs::restrict_dir(parent);
    }
    let mut f = fs::File::create(path).with_context(|| format!("create {}", path.display()))?;
    write!(f, "{pid}")?;
    secure_fs::restrict_file(path);
    Ok(())
}

pub fn remove_tunnel_pid(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_file(path).with_context(|| format!("remove {}", path.display()))?;
    }
    Ok(())
}

pub fn is_pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

pub fn kill_pid(pid: u32) -> Result<()> {
    #[cfg(unix)]
    {
        let rc = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
        if rc != 0 {
            anyhow::bail!("failed to signal pid {pid}");
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        anyhow::bail!("tunnel stop is unsupported on this platform");
    }
}

/// True when `127.0.0.1:port` is already bound (another listener).
pub fn is_local_port_bound(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_err()
}

pub fn tunnel_runtime_state(tunnel_id: i64, local_port: u16, pid_dir: &Path) -> TunnelRuntimeState {
    let pid_path = tunnel_pid_path(pid_dir, tunnel_id);
    match read_tunnel_pid(&pid_path) {
        Ok(Some(pid)) if is_pid_alive(pid) => return TunnelRuntimeState::Running,
        Ok(Some(_)) => {
            let _ = remove_tunnel_pid(&pid_path);
        }
        Ok(None) => {}
        Err(_) => return TunnelRuntimeState::Unknown,
    }
    match TcpListener::bind(("127.0.0.1", local_port)) {
        Err(_) => TunnelRuntimeState::External,
        Ok(_) => TunnelRuntimeState::Stopped,
    }
}

/// Spawn a detached tunnel child and record its PID under `data_dir/tunnels/`.
pub fn spawn_detached_tunnel(
    tunnel: &Tunnel,
    host: &ManagedHost,
    secret: Option<&PendingSecret>,
    data_dir: &Path,
) -> Result<u32> {
    let pid_dir = ensure_tunnel_pid_dir(data_dir)?;
    let pid_path = tunnel_pid_path(&pid_dir, tunnel.id);

    if let Some(old_pid) = read_tunnel_pid(&pid_path)? {
        if is_pid_alive(old_pid) {
            anyhow::bail!("tunnel {} already running (pid {old_pid})", tunnel.id);
        }
        remove_tunnel_pid(&pid_path)?;
    }

    if is_local_port_bound(tunnel.local_port) {
        anyhow::bail!("local port {} already in use", tunnel.local_port);
    }

    let args = build_tunnel_argv(tunnel, host, secret.is_some())?;
    let mut cmd = Command::new(&args[0]);
    cmd.args(&args[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let _askpass = stage_tunnel_askpass(&mut cmd, secret)?;
    let child = cmd.spawn().context("spawn tunnel ssh")?;
    let pid = child.id();
    write_tunnel_pid(&pid_path, pid)?;
    Ok(pid)
}

/// Stop a CLI-detached tunnel via its PID file. Returns true when a live PID was signalled.
pub fn stop_detached_tunnel(data_dir: &Path, tunnel_id: i64) -> Result<bool> {
    let pid_dir = ensure_tunnel_pid_dir(data_dir)?;
    let pid_path = tunnel_pid_path(&pid_dir, tunnel_id);
    let Some(pid) = read_tunnel_pid(&pid_path)? else {
        return Ok(false);
    };
    let was_live = is_pid_alive(pid);
    if was_live {
        kill_pid(pid)?;
    }
    remove_tunnel_pid(&pid_path)?;
    Ok(was_live)
}

/// Resolved data directory for tunnel PID files.
pub fn tunnel_data_dir() -> Result<PathBuf> {
    config::data_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn pid_file_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = ensure_tunnel_pid_dir(tmp.path()).unwrap();
        let path = tunnel_pid_path(&dir, 42);
        write_tunnel_pid(&path, 12345).unwrap();
        assert_eq!(read_tunnel_pid(&path).unwrap(), Some(12345));
        remove_tunnel_pid(&path).unwrap();
        assert_eq!(read_tunnel_pid(&path).unwrap(), None);
    }
}
