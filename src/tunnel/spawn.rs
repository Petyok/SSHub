use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

use crate::secure_fs;
use crate::session::hop_auth::HopSecret;
use crate::session::{askpass, PendingSecret};
use crate::store::{ManagedHost, ResolvedConnection, Tunnel, TunnelType};

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
///
/// `resolved` is the effective connection for `host` (host-explicit → group
/// chain → global, via `LauncherStore::resolve_connection`): tunnels honour
/// the same group defaults as every connect path. Callers that cannot resolve
/// fall back to `ResolvedConnection::from_stored`.
pub fn build_tunnel_argv(
    tunnel: &Tunnel,
    host: &ManagedHost,
    resolved: &ResolvedConnection,
    has_stored_secret: bool,
) -> Result<Vec<String>> {
    // A jump that is not valid hops would be dropped from the argv, and a
    // tunnel that skips the configured bastion is a direct connection nobody
    // asked for: refuse.
    if let Some(problem) = &resolved.jump_error {
        anyhow::bail!("{}: {problem}", host.name);
    }
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

    // Effective values, mirroring the connect path (`resolved_to_ssh_host`):
    // a group-inherited port/jump/identity must reach the tunnel's ssh too.
    let port = resolved.port;
    if port != 22 {
        args.push("-p".into());
        args.push(port.to_string());
    }
    if let Some(jump) = resolved.jump_argv.as_ref() {
        if !jump.is_empty() {
            args.push("-J".into());
            args.push(jump.clone());
        }
    }
    if resolved.forward_agent {
        args.push("-A".into());
    }
    if let Some(identity) = resolved.identity.as_ref() {
        if let Some(key) = identity.private_key.as_ref() {
            args.push("-i".into());
            args.push(key.to_string_lossy().into_owned());
        }
    }
    let target = if let Some(username) = resolved.username.as_ref() {
        format!("{}@{}", username, host.address)
    } else if let Some(identity) = resolved.identity.as_ref() {
        if let Some(u) = identity.username.as_ref() {
            format!("{}@{}", u, host.address)
        } else {
            host.address.clone()
        }
    } else {
        host.address.clone()
    };
    // Same guard as the connect path: an address stored with a leading `-` would
    // reach ssh as an option, not a host. See `ssh::safe_ssh_target`.
    args.push(crate::ssh::safe_ssh_target(target));
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
        "-o".to_string(),
        // Without it a failed bind or a refused forward leaves ssh running with
        // no forward while the tunnel reads "up", and nothing retries it.
        "ExitOnForwardFailure=yes".to_string(),
    ];
    if has_stored_secret {
        opts.push("-o".to_string());
        opts.push("StrictHostKeyChecking=accept-new".to_string());
    }
    for (i, opt) in opts.into_iter().enumerate() {
        args.insert(1 + i, opt);
    }
}

/// The staged-file guard and the env to give the ssh child.
type StagedAskpass = (askpass::AskpassSecret, Vec<(String, String)>);

/// Stage the tunnel's secrets and make ssh delete the file itself once it is
/// connected. A tunnel lives for days; secrets are only ever asked during the
/// connect, so nothing may stay on disk for its whole life (hop passwords that
/// were never asked would). `LocalCommand` runs after authentication, hops
/// included, even with `-N`. Returns the guard and the env for the ssh child.
pub fn stage_tunnel_askpass(
    args: &mut Vec<String>,
    secret: Option<&PendingSecret>,
    hops: &[HopSecret],
) -> Result<Option<StagedAskpass>> {
    if secret.is_none() && hops.is_empty() {
        return Ok(None);
    }
    let exe = askpass::helper_exe()?;
    let guard = askpass::AskpassSecret::staged(secret, hops)?;
    let env = guard.env(&exe);
    // Quoted for sh; `%` doubled because ssh expands tokens in LocalCommand.
    let path = guard
        .path()
        .to_string_lossy()
        .replace('\'', "'\\''")
        .replace('%', "%%");
    if args.first().map(String::as_str) == Some("ssh") {
        args.splice(
            1..1,
            [
                "-o".to_string(),
                "PermitLocalCommand=yes".to_string(),
                "-o".to_string(),
                format!("LocalCommand=rm -f -- '{path}'"),
            ],
        );
    }
    Ok(Some((guard, env)))
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
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .parse()
        .with_context(|| format!("invalid pid in {}", path.display()))?;
    Ok(Some(pid))
}

pub fn write_tunnel_pid(path: &Path, pid: u32) -> Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        secure_fs::restrict_dir(parent);
    }
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut f = options
        .open(path)
        .with_context(|| format!("create {}", path.display()))?;
    // The start time rides on the second line so a recycled pid can be told
    // from the process that was started (`tunnel_pid_alive`).
    match process_start(pid) {
        Some(start) => write!(f, "{pid}\n{start}")?,
        None => write!(f, "{pid}")?,
    }
    secure_fs::restrict_file(path);
    Ok(())
}

pub fn remove_tunnel_pid(path: &Path) -> Result<()> {
    // A detached tunnel's staged secret goes with its pid file (#144).
    let record = staged_secret_record(path);
    if let Ok(staged) = fs::read(&record) {
        use std::os::unix::ffi::OsStrExt;
        askpass::remove_outlived(Path::new(std::ffi::OsStr::from_bytes(&staged)));
        let _ = fs::remove_file(&record);
    }
    if path.exists() {
        fs::remove_file(path).with_context(|| format!("remove {}", path.display()))?;
    }
    Ok(())
}

/// Where a detached tunnel records the path of its staged askpass secret.
fn staged_secret_record(pid_path: &Path) -> PathBuf {
    pid_path.with_extension("askpass")
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

/// The process's start time as an opaque string, to tell it from a later
/// process that was given the same pid. Linux: field 22 of `/proc/<pid>/stat`
/// (clock ticks since boot); elsewhere `ps -o lstart=`. `None` when the
/// process is gone or the platform cannot tell.
#[cfg(unix)]
fn process_start(pid: u32) -> Option<String> {
    if let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) {
        // `pid (comm) state ...`: comm may hold spaces and parens, so count
        // from the last `)`. Field 3 (state) is index 0 there, field 22 is 19.
        let rest = stat.rsplit_once(')')?.1;
        return rest.split_whitespace().nth(19).map(String::from);
    }
    let out = Command::new("ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let start = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !start.is_empty()).then_some(start)
}

#[cfg(not(unix))]
fn process_start(_pid: u32) -> Option<String> {
    None
}

/// Whether `pid` is an `ssh` process (a tunnel is `ssh -N ...`).
#[cfg(unix)]
fn process_is_ssh(pid: u32) -> bool {
    let comm = match fs::read(format!("/proc/{pid}/cmdline")) {
        Ok(raw) => {
            String::from_utf8_lossy(raw.split(|b| *b == 0).next().unwrap_or_default()).into_owned()
        }
        Err(_) => Command::new("ps")
            .args(["-o", "comm=", "-p", &pid.to_string()])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default(),
    };
    Path::new(&comm).file_name().is_some_and(|n| n == "ssh")
}

#[cfg(not(unix))]
fn process_is_ssh(_pid: u32) -> bool {
    false
}

/// The pid file's second line: the start time recorded with the pid.
fn recorded_start(path: &Path) -> Option<String> {
    let raw = fs::read_to_string(path).ok()?;
    raw.lines().nth(1).map(|l| l.trim().to_string())
}

/// The recorded `pid` is still the tunnel's ssh: alive AND the same process
/// (its start time matches the one written beside the pid), not a stranger
/// that was handed a recycled pid — which `kill(pid, 0)` cannot tell apart and
/// a stop would then SIGTERM. A pid file without a start time (written before
/// they were recorded, or by a platform that cannot read one) falls back to
/// "is an `ssh` process".
pub fn tunnel_pid_alive(path: &Path, pid: u32) -> bool {
    if !is_pid_alive(pid) {
        return false;
    }
    match recorded_start(path) {
        Some(recorded) => process_start(pid).is_some_and(|now| now == recorded),
        None => process_is_ssh(pid),
    }
}

/// True when `127.0.0.1:port` is already bound (another listener).
pub fn is_local_port_bound(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_err()
}

pub fn tunnel_runtime_state(tunnel_id: i64, local_port: u16, pid_dir: &Path) -> TunnelRuntimeState {
    let pid_path = tunnel_pid_path(pid_dir, tunnel_id);
    match read_tunnel_pid(&pid_path) {
        Ok(Some(pid)) if tunnel_pid_alive(&pid_path, pid) => return TunnelRuntimeState::Running,
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
///
/// Known limitations (acceptable for the v1 CLI, single-user local use):
/// - The stale-PID and port-bound checks are best-effort with no advisory
///   lock, so two near-simultaneous `tunnel start <id>` invocations can race
///   between the checks and the PID write and leak one ssh child.
/// - A pid file written before start times were recorded is trusted only while
///   its pid is an `ssh` process (see [`tunnel_pid_alive`]).
pub fn spawn_detached_tunnel(
    tunnel: &Tunnel,
    host: &ManagedHost,
    resolved: &ResolvedConnection,
    secret: Option<&PendingSecret>,
    hops: &[HopSecret],
    data_dir: &Path,
) -> Result<u32> {
    let pid_dir = ensure_tunnel_pid_dir(data_dir)?;
    let pid_path = tunnel_pid_path(&pid_dir, tunnel.id);

    if let Some(old_pid) = read_tunnel_pid(&pid_path)? {
        if tunnel_pid_alive(&pid_path, old_pid) {
            anyhow::bail!("tunnel {} already running (pid {old_pid})", tunnel.id);
        }
        remove_tunnel_pid(&pid_path)?;
    }

    if is_local_port_bound(tunnel.local_port) {
        anyhow::bail!("local port {} already in use", tunnel.local_port);
    }

    let mut args = build_tunnel_argv(tunnel, host, resolved, secret.is_some() || !hops.is_empty())?;
    let staged = stage_tunnel_askpass(&mut args, secret, hops)?;
    // A synced credential lives only in this process, which is about to exit
    // while ssh stays: it cannot be handed to a detached ssh without a file.
    if staged
        .as_ref()
        .is_some_and(|(guard, _)| guard.holds_synced())
    {
        anyhow::bail!(
            "a synced credential cannot be handed to a detached tunnel: start this tunnel from the TUI or in the foreground"
        );
    }
    let mut cmd = Command::new(&args[0]);
    cmd.args(&args[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some((_, env)) = &staged {
        cmd.envs(env.iter().map(|(k, v)| (k, v)));
        cmd.env(askpass::READ_ONCE_ENV, "1");
    }
    let staged = staged.map(|(guard, _)| guard);
    let child = cmd.spawn().context("spawn tunnel ssh")?;
    let pid = child.id();
    write_tunnel_pid(&pid_path, pid)?;
    if let Some(staged) = staged {
        // ssh outlives this process and asks for the secret later (#144): leave
        // the file to it (the helper deletes it once read) and record it, so
        // stopping a tunnel that never asked removes it too.
        use std::os::unix::ffi::OsStrExt;
        let path = staged.outlive();
        if let Err(e) = fs::write(staged_secret_record(&pid_path), path.as_os_str().as_bytes()) {
            askpass::remove_outlived(&path);
            return Err(e).context("record staged tunnel secret");
        }
    }
    Ok(pid)
}

/// Stop a CLI-detached tunnel via its PID file. Returns true when a live PID was signalled.
///
/// The recorded pid is only signalled when it is still the process that was
/// started (see [`tunnel_pid_alive`]): a recycled pid belongs to a stranger.
pub fn stop_detached_tunnel(data_dir: &Path, tunnel_id: i64) -> Result<bool> {
    let pid_dir = ensure_tunnel_pid_dir(data_dir)?;
    let pid_path = tunnel_pid_path(&pid_dir, tunnel_id);
    let Some(pid) = read_tunnel_pid(&pid_path)? else {
        return Ok(false);
    };
    let was_live = tunnel_pid_alive(&pid_path, pid);
    if was_live {
        kill_pid(pid)?;
    }
    remove_tunnel_pid(&pid_path)?;
    Ok(was_live)
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
    fn a_tunnel_whose_forward_fails_exits_instead_of_reporting_up() {
        for secret in [false, true] {
            let mut args = vec!["ssh".into(), "-N".into(), "host".into()];
            splice_tunnel_ssh_options(&mut args, secret);
            assert!(args
                .windows(2)
                .any(|w| w == ["-o", "ExitOnForwardFailure=yes"]));
        }
    }

    #[test]
    fn detached_tunnel_keeps_its_staged_secret_until_stopped() {
        // #144: `tunnel start` returned, dropped the staged askpass file, and
        // the detached ssh found nothing when it asked for the password. The
        // ssh here dials a closed port, so it never asks.
        use crate::store::{LauncherStore, NewHost};
        if Command::new("ssh").arg("-V").output().is_err() {
            crate::test_support::tool_missing("ssh");
            return;
        }
        let free_port = || {
            TcpListener::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap()
                .port()
        };
        let store = LauncherStore::open_in_memory().unwrap();
        let mut new_host = NewHost::launcher("t", "127.0.0.1");
        new_host.port = Some(free_port());
        let host = store.create_host(&new_host).unwrap();
        let tunnel = Tunnel {
            id: 7,
            host_id: Some(host.id),
            tunnel_type: TunnelType::Local,
            local_port: free_port(),
            remote_host: "localhost".into(),
            remote_port: 80,
            label: None,
            auto_connect: false,
            created_at: 0,
            updated_at: 0,
        };
        let data = tempfile::tempdir().unwrap();
        let secret = PendingSecret::Password("s3cr3t".into());
        spawn_detached_tunnel(
            &tunnel,
            &host,
            &ResolvedConnection::from_stored(&host),
            Some(&secret),
            &[],
            data.path(),
        )
        .unwrap();

        let pid_path = tunnel_pid_path(&ensure_tunnel_pid_dir(data.path()).unwrap(), 7);
        let staged = PathBuf::from(fs::read_to_string(staged_secret_record(&pid_path)).unwrap());
        assert_eq!(fs::read_to_string(&staged).unwrap(), "s3cr3t\nC\tp\n");

        stop_detached_tunnel(data.path(), 7).unwrap();
        assert!(!staged.exists(), "stop must remove the staged secret");
        assert!(!staged_secret_record(&pid_path).exists());
    }

    #[test]
    fn a_staged_tunnel_deletes_its_secrets_once_ssh_is_connected() {
        // LocalCommand runs after authentication: the staged file must not
        // live as long as the tunnel. ssh hands the value to `sh -c` after
        // turning `%%` into `%`, which this does by hand.
        let mut args: Vec<String> = ["ssh", "-N", "-L", "1:h:2", "target"]
            .map(String::from)
            .into();
        let secret = PendingSecret::Password("s3cr3t".into());
        let (guard, env) = stage_tunnel_askpass(&mut args, Some(&secret), &[])
            .unwrap()
            .unwrap();
        assert!(env.iter().any(|(k, _)| k == "SSH_ASKPASS"));
        assert_eq!(
            args.last().map(String::as_str),
            Some("target"),
            "options precede the host"
        );
        assert!(args
            .windows(2)
            .any(|w| w == ["-o", "PermitLocalCommand=yes"]));
        let command = args
            .iter()
            .find_map(|a| a.strip_prefix("LocalCommand="))
            .unwrap()
            .replace("%%", "%");
        assert!(guard.path().exists());
        let status = Command::new("sh").args(["-c", &command]).status().unwrap();
        assert!(status.success());
        assert!(!guard.path().exists(), "{command}");
    }

    #[test]
    fn a_tunnel_stages_hop_references_never_their_secrets() {
        use crate::credentials::{SecretRef, StoreLocator};
        use crate::session::auth_prompt::PromptClass;
        let dir = tempfile::tempdir().unwrap();
        let store = StoreLocator::File {
            path: dir.path().join("credentials.json"),
            prefix: "profile:3:".into(),
        };
        store.open().set("host:5", "hop-s3cr3t").unwrap();
        let hop = HopSecret {
            user: Some("ops".into()),
            host: "192.0.2.11".into(),
            via: "hop-1".into(),
            key_path: None,
            class: PromptClass::Password,
            refs: vec![SecretRef::Key("host:5".into())],
            store: Some(store),
        };
        let mut args: Vec<String> = ["ssh", "-N", "-L", "1:h:2", "target"]
            .map(String::from)
            .into();
        let (guard, _env) = stage_tunnel_askpass(&mut args, None, &[hop])
            .unwrap()
            .unwrap();
        let text = fs::read_to_string(guard.path()).unwrap();
        assert!(!text.contains("hop-s3cr3t"), "{text}");
        assert!(text.contains("host:5"), "{text}");
        // Even with nothing secret staged, ssh's LocalCommand removes the file.
        let command = args
            .iter()
            .find_map(|a| a.strip_prefix("LocalCommand="))
            .unwrap()
            .replace("%%", "%");
        assert!(Command::new("sh")
            .args(["-c", &command])
            .status()
            .unwrap()
            .success());
        assert!(!guard.path().exists());
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

    #[cfg(unix)]
    #[test]
    fn a_recycled_pid_is_neither_alive_nor_killed() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = ensure_tunnel_pid_dir(tmp.path()).unwrap();
        let path = tunnel_pid_path(&dir, 9);
        // An unrelated process that happens to own the recorded pid.
        let mut other = Command::new("sleep").arg("30").spawn().unwrap();
        let pid = other.id();

        // Recorded together with its start time: the same process is alive.
        write_tunnel_pid(&path, pid).unwrap();
        assert!(tunnel_pid_alive(&path, pid));
        remove_tunnel_pid(&path).unwrap();

        // The pid was recycled: same number, different start time.
        fs::write(&path, format!("{pid}\nnot-the-start-time")).unwrap();
        assert_eq!(read_tunnel_pid(&path).unwrap(), Some(pid));
        assert!(!tunnel_pid_alive(&path, pid));
        let free = TcpListener::bind(("127.0.0.1", 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        assert!(
            matches!(
                tunnel_runtime_state(9, free, &dir),
                TunnelRuntimeState::Stopped
            ),
            "a recycled pid must not read as a running tunnel"
        );
        fs::write(&path, format!("{pid}\nnot-the-start-time")).unwrap();
        assert!(!stop_detached_tunnel(tmp.path(), 9).unwrap());
        assert!(
            other.try_wait().unwrap().is_none(),
            "the stranger was signalled"
        );

        // A legacy bare-pid file has no start time: only an ssh process counts.
        fs::write(&path, pid.to_string()).unwrap();
        assert!(!tunnel_pid_alive(&path, pid));
        let _ = other.kill();
        let _ = other.wait();
    }
    #[test]
    fn a_tunnel_through_an_invalid_stored_jump_is_refused_not_run_direct() {
        // `from_stored` is what callers fall back to when resolving fails; it
        // drops the bad jump from the argv, so the tunnel itself must refuse.
        let store = crate::store::LauncherStore::open_in_memory().unwrap();
        let mut nh = crate::store::NewHost::launcher("web", "10.0.0.1");
        nh.proxy_jump = Some("bastion,-oProxyCommand=id".into());
        let host = store.create_host(&nh).unwrap();
        let tunnel = Tunnel {
            id: 1,
            host_id: Some(host.id),
            tunnel_type: TunnelType::Local,
            local_port: 8080,
            remote_host: "localhost".into(),
            remote_port: 80,
            label: None,
            auto_connect: false,
            created_at: 0,
            updated_at: 0,
        };
        let err = build_tunnel_argv(
            &tunnel,
            &host,
            &ResolvedConnection::from_stored(&host),
            false,
        )
        .expect_err("must refuse");
        assert!(format!("{err:#}").contains("not a valid"), "{err:#}");
        // A good jump still builds, with -J.
        let mut ok = crate::store::NewHost::launcher("ok", "10.0.0.2");
        ok.proxy_jump = Some("bastion".into());
        let ok = store.create_host(&ok).unwrap();
        let argv =
            build_tunnel_argv(&tunnel, &ok, &ResolvedConnection::from_stored(&ok), false).unwrap();
        assert!(argv.windows(2).any(|w| w == ["-J", "bastion"]));
    }

    #[test]
    fn tunnel_argv_carries_group_inherited_connection_values() {
        // Oracle: hand-computed — the host sets no connection field itself;
        // group "prod" supplies port 2222, ProxyJump, agent forwarding, and
        // an identity (key + username). The tunnel argv must carry those
        // EFFECTIVE values, exactly like the connect path does.
        use crate::store::{LauncherStore, NewHost, NewHostGroup, NewIdentity};

        let store = LauncherStore::open_in_memory().unwrap();
        let iid = store
            .create_identity(&NewIdentity {
                name: "prod-ident".into(),
                username: Some("prod-user".into()),
                private_key: Some(PathBuf::from("/keys/prod_ed25519")),
                ..Default::default()
            })
            .unwrap()
            .id;
        let group = store
            .create_group(&NewHostGroup {
                name: "prod".into(),
                default_identity_id: Some(iid),
                default_port: Some(2222),
                default_proxy_jump: Some("bastion.example".into()),
                default_forward_agent: Some(true),
                ..Default::default()
            })
            .unwrap();
        let mut nh = NewHost::launcher("web", "10.0.0.1");
        nh.group_id = Some(group.id);
        nh.port = None;
        nh.forward_agent = None;
        nh.transport = None;
        let created = store.create_host(&nh).unwrap();
        let host = store.get_host(created.id).unwrap().unwrap();
        assert_eq!(host.port, None);
        assert_eq!(host.proxy_jump, None);
        assert_eq!(host.forward_agent, None);

        let resolved = store.resolve_connection(&host).unwrap();
        assert_eq!(resolved.port, 2222);
        assert_eq!(resolved.proxy_jump.as_deref(), Some("bastion.example"));
        assert!(resolved.forward_agent);

        let tunnel = Tunnel {
            id: 1,
            host_id: Some(host.id),
            tunnel_type: TunnelType::Local,
            local_port: 8080,
            remote_host: "localhost".into(),
            remote_port: 80,
            label: None,
            auto_connect: false,
            created_at: 0,
            updated_at: 0,
        };
        let argv = build_tunnel_argv(&tunnel, &host, &resolved, false).unwrap();
        let expected: Vec<String> = [
            "ssh",
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=30",
            "-o",
            "ServerAliveInterval=10",
            "-o",
            "ServerAliveCountMax=3",
            "-o",
            "TCPKeepAlive=yes",
            "-o",
            "ExitOnForwardFailure=yes",
            "-N",
            "-L",
            "8080:localhost:80",
            "-p",
            "2222",
            "-J",
            "bastion.example",
            "-A",
            "-i",
            "/keys/prod_ed25519",
            "prod-user@10.0.0.1",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        assert_eq!(argv, expected);
    }

    #[test]
    fn a_detached_tunnel_refuses_a_synced_credential_and_leaves_nothing_behind() {
        let store = crate::store::LauncherStore::open_in_memory().unwrap();
        let host = store
            .create_host(&crate::store::NewHost::launcher("web", "127.0.0.1"))
            .unwrap();
        let id = store
            .create_tunnel(&crate::store::NewTunnel {
                host_id: Some(host.id),
                tunnel_type: crate::store::TunnelType::Local,
                local_port: 18997,
                remote_host: "localhost".into(),
                remote_port: 80,
                label: None,
                auto_connect: false,
            })
            .unwrap();
        let tunnel = store.get_tunnel(id).unwrap().unwrap();
        let resolved = store.resolve_connection(&host).unwrap();
        let gate = crate::credentials::SecretGate::default();
        gate.insert("host:1", zeroize::Zeroizing::new("SYNCED-tunnel-pw".into()));
        let secret = PendingSecret::Password("SYNCED-tunnel-pw".into());
        let dir = tempfile::tempdir().unwrap();
        let err = spawn_detached_tunnel(&tunnel, &host, &resolved, Some(&secret), &[], dir.path())
            .unwrap_err();
        assert!(err.to_string().contains("detached tunnel"), "{err:#}");
        let pids = dir.path().join("tunnels");
        assert!(
            !pids.exists() || std::fs::read_dir(&pids).unwrap().next().is_none(),
            "no pid file, so no ssh was started"
        );
    }
}
