//! Sharing this machine's network with the host (#174): a reverse dynamic
//! forward (`ssh -R 127.0.0.1:0`) gives the host a SOCKS5 proxy on its
//! loopback, and the traffic sent through it leaves from here.

/// The `-R` spec: loopback only, port 0 so the server picks a free one.
pub(crate) const REMOTE_FORWARD: &str = "127.0.0.1:0";

/// What the proxy may open from this machine: web ports only, so the host
/// cannot reach ssh, databases or anything else behind it. One constant so a
/// test can substitute its own port.
pub(crate) const PERMIT_REMOTE_OPEN: &str = "*:80 *:443";

/// Where the sharing of a session stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetShare {
    /// Not on.
    Off,
    /// The sidecar ssh is up; it has not said what the server did.
    Starting,
    /// The server allocated this port on the host's loopback.
    Port(u16),
    /// The sidecar ended or the server refused the forward; the reason went to
    /// the host's ssh log.
    Failed,
}

/// Argv options for a proxy connection, before the destination. A shared proxy never rides a user's ControlMaster: the `-R`
/// would be forwarded to a master that has no `PermitRemoteOpen`, and the
/// proxy would reach any port.
///
/// The first destination is passed twice. OpenSSH 8.8 to 9.1 skip the first
/// argument of `PermitRemoteOpen` (bz3515, fixed in 9.2): `*:80 *:443` would
/// lose port 80 there, and a one-entry list would allow everything. The copy
/// is what they skip; any other version reads a harmless duplicate.
pub(crate) fn ssh_args(permit_remote_open: &str) -> [String; 8] {
    let first = permit_remote_open
        .split_whitespace()
        .next()
        .unwrap_or(permit_remote_open);
    [
        "-R".into(),
        REMOTE_FORWARD.into(),
        "-o".into(),
        format!("PermitRemoteOpen={first} {permit_remote_open}"),
        "-o".into(),
        "ControlMaster=no".into(),
        "-o".into(),
        "ControlPath=none".into(),
    ]
}

/// What one line of the sidecar ssh's own `-E` log says about the forward.
/// OpenSSH_10.5p1 writes, at INFO and on its own lines:
/// `Allocated port 45509 for remote forward to socks:0` and
/// `Warning: remote port forwarding failed for listen port 0`. Only these
/// exact shapes count, so a RemoteForward of the user's own is ignored. The
/// server's pre-auth banner goes to stderr, which is never parsed; its debug
/// messages reach the log only at DEBUG, which the sidecar does not use.
pub(crate) fn parse_log_line(line: &str) -> Option<NetShare> {
    if let Some(port) = line
        .strip_prefix("Allocated port ")
        .and_then(|rest| rest.strip_suffix(" for remote forward to socks:0"))
    {
        return port.parse().ok().map(NetShare::Port);
    }
    (line == "Warning: remote port forwarding failed for listen port 0").then_some(NetShare::Failed)
}

/// One line for the host's ssh log: the proxy and ready-to-paste commands.
/// Starts with `session:` so it survives the filter that drops handshake
/// diagnostics once connected.
pub(crate) fn hint(port: u16) -> String {
    let proxy = format!("socks5h://127.0.0.1:{port}");
    format!(
        "session: sharing this machine's network, SOCKS5 proxy on the host at 127.0.0.1:{port} \
         (web ports only). apt -o Acquire::http::Proxy={proxy} -o Acquire::https::Proxy={proxy} \
         install ...  |  curl -x {proxy} ..."
    )
}

/// The sidecar's argv: `ssh -N` to the same host the session reached, built
/// from the session's own argv so every way of opening one (saved host, ssh
/// config alias, ad-hoc) is covered. `None` unless that is plain `ssh`.
///
/// The session's `-t` and `-A` are dropped. A startup command, if the argv
/// ends in `-- <cmd>`, stays on the line but never runs: `-N` asks for no
/// command. Options given first win in ssh, so the ones put in front here
/// override the session's own (`StrictHostKeyChecking=ask`, `LogLevel`,
/// `ForwardAgent`). The host key was verified by the session, so it must be
/// known already: a background process never prompts.
pub(crate) fn sidecar_argv(
    session_argv: &[String],
    has_stored_secret: bool,
    permit_remote_open: &str,
) -> Option<Vec<String>> {
    if session_argv.first().map(String::as_str) != Some("ssh") {
        return None;
    }
    let mut argv: Vec<String> = vec!["ssh".into(), "-N".into()];
    argv.extend(ssh_args(permit_remote_open));
    argv.extend(
        session_argv[1..]
            .iter()
            .filter(|a| !matches!(a.as_str(), "-t" | "-tt" | "-A"))
            .cloned(),
    );
    crate::tunnel::spawn::splice_tunnel_ssh_options(&mut argv, has_stored_secret);
    argv.splice(
        1..1,
        [
            "-o",
            "StrictHostKeyChecking=yes",
            "-o",
            "LogLevel=INFO",
            // The user's own forwards in ssh_config are also bound by the
            // session: a second bind fails, and must not end the proxy.
            "-o",
            "ExitOnForwardFailure=no",
            "-o",
            "ForwardAgent=no",
            "-o",
            "ForwardX11=no",
        ]
        .map(String::from),
    );
    Some(argv)
}

/// How many log lines of the sidecar are kept for a failure reason.
const TAIL_LINES: usize = 6;

/// The second ssh connection that only carries the proxy.
///
/// It ends with the session and with sshub: dropping it (quit, tab close,
/// session end) signals the ssh and waits for it; a terminal hangup reaches it
/// as SIGHUP, because it shares sshub's process group; and on Linux it also
/// asks the kernel to send it SIGTERM when sshub dies however that happens,
/// SIGKILL included. Elsewhere only a SIGKILL of sshub (or a crash) can leave
/// it running.
///
/// Its own log goes to a private file (`-E`) that is the only thing parsed for
/// the port. stderr carries the server's pre-auth banner, which a server can
/// write freely, so it is never parsed: it only feeds the failure reason.
pub(crate) struct Sidecar {
    child: std::process::Child,
    /// Private dir holding `ssh.log`; removed with the sidecar.
    _dir: tempfile::TempDir,
    log: std::fs::File,
    /// A log line that has not ended yet.
    partial: String,
    stderr: std::sync::mpsc::Receiver<String>,
    tail: std::collections::VecDeque<String>,
    stderr_tail: std::collections::VecDeque<String>,
    stored_secret: bool,
    /// Kept so the staged secret file goes when the sidecar does.
    _askpass: Option<crate::session::askpass::AskpassSecret>,
}

fn push_tail(tail: &mut std::collections::VecDeque<String>, line: String) {
    if tail.len() == TAIL_LINES {
        tail.pop_front();
    }
    tail.push_back(line);
}

impl Sidecar {
    pub(crate) fn spawn(
        mut argv: Vec<String>,
        secret: Option<&crate::session::PendingSecret>,
        hops: &[crate::session::hop_auth::HopSecret],
    ) -> anyhow::Result<Self> {
        use anyhow::Context;
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};

        let dir = tempfile::Builder::new()
            .prefix("sshub-share-")
            .tempdir()
            .context("create a private dir for the proxy ssh log")?;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
            .context("restrict the proxy ssh log dir")?;
        let log_path = dir.path().join("ssh.log");
        std::fs::File::create(&log_path).context("create the proxy ssh log")?;

        let staged = crate::tunnel::spawn::stage_tunnel_askpass(&mut argv, secret, hops)?;
        let mut cmd = Command::new(&argv[0]);
        // `-E` first: ssh keeps the first log file it is given.
        cmd.arg("-E")
            .arg(&log_path)
            .args(&argv[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if let Some((_, env)) = &staged {
            cmd.envs(env.iter().map(|(k, v)| (k, v)));
            cmd.env(crate::session::askpass::READ_ONCE_ENV, "1");
        }
        // Deliberately not its own process group: a terminal hangup is sent to
        // sshub's foreground group, and the sidecar must get it too.
        #[cfg(target_os = "linux")]
        {
            let parent = std::process::id() as libc::pid_t;
            // SAFETY: only async-signal-safe calls (prctl, getppid, _exit)
            // between fork and exec. PDEATHSIG is tied to the thread that
            // spawns, which is sshub's UI thread, alive as long as sshub is.
            unsafe {
                cmd.pre_exec(move || {
                    if libc::prctl(
                        libc::PR_SET_PDEATHSIG,
                        libc::SIGTERM as libc::c_ulong,
                        0,
                        0,
                        0,
                    ) != 0
                    {
                        return Err(std::io::Error::last_os_error());
                    }
                    // sshub may have died before the prctl took effect.
                    if libc::getppid() != parent {
                        libc::_exit(1);
                    }
                    Ok(())
                });
            }
        }
        let mut child = cmd.spawn().context("spawn the proxy ssh")?;
        let stderr = child.stderr.take().context("proxy ssh has no stderr")?;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            use std::io::BufRead;
            for line in std::io::BufReader::new(stderr).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            _dir: dir,
            log: std::fs::File::open(&log_path).context("open the proxy ssh log")?,
            partial: String::new(),
            stderr: rx,
            tail: std::collections::VecDeque::new(),
            stderr_tail: std::collections::VecDeque::new(),
            stored_secret: secret.is_some() || !hops.is_empty(),
            _askpass: staged.map(|(guard, _)| guard),
        })
    }

    #[cfg(test)]
    pub(crate) fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Complete lines ssh wrote to its own log since the last call, and
    /// whether the process has ended.
    pub(crate) fn poll(&mut self) -> (Vec<String>, bool) {
        use std::io::Read;
        let ended = matches!(self.child.try_wait(), Ok(Some(_)) | Err(_));
        // After the exit check: whatever was written before it is read now.
        let mut fresh = String::new();
        let _ = self.log.read_to_string(&mut fresh);
        self.partial.push_str(&fresh);
        let mut lines = Vec::new();
        while let Some(at) = self.partial.find('\n') {
            let line: String = self.partial.drain(..=at).collect();
            lines.push(line.trim_end().to_string());
        }
        if ended && !self.partial.is_empty() {
            lines.push(std::mem::take(&mut self.partial));
        }
        for line in &lines {
            push_tail(&mut self.tail, line.clone());
        }
        for line in self.stderr.try_iter() {
            push_tail(&mut self.stderr_tail, line);
        }
        if ended {
            // The pipe closes with the process: take what is left, without
            // waiting on a straggler that kept it open.
            while let Ok(line) = self
                .stderr
                .recv_timeout(std::time::Duration::from_millis(100))
            {
                push_tail(&mut self.stderr_tail, line);
            }
        }
        (lines, ended)
    }

    /// Why the proxy ssh ended, for the ssh log.
    pub(crate) fn failure_reason(&self) -> String {
        let join = |t: &std::collections::VecDeque<String>| {
            t.iter().cloned().collect::<Vec<_>>().join(" | ")
        };
        let log = join(&self.tail);
        let text = if log.is_empty() {
            join(&self.stderr_tail)
        } else {
            log
        };
        if text.contains("HOST IDENTIFICATION HAS CHANGED") || text.contains("has changed") {
            return "the host key has changed".into();
        }
        if text.contains("Host key verification failed") {
            return "the host key did not verify".into();
        }
        if text.contains("Permission denied") && !self.stored_secret {
            return "needs a login without prompts: key or stored password".into();
        }
        if text.is_empty() {
            "ssh ended".into()
        } else {
            text
        }
    }

    fn kill(&mut self) {
        // A reaped pid may be reused: never signal it.
        if !matches!(self.child.try_wait(), Ok(None)) {
            return;
        }
        // ssh takes its own ProxyCommand / ProxyJump helper down when it exits.
        let pid = self.child.id() as libc::pid_t;
        unsafe { libc::kill(pid, libc::SIGTERM) };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while matches!(self.child.try_wait(), Ok(None)) {
            if std::time::Instant::now() >= deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        self.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sidecar_is_ssh_only_and_leads_with_its_own_options() {
        let session: Vec<String> = [
            "ssh",
            "-t",
            "-A",
            "-o",
            "StrictHostKeyChecking=ask",
            "-o",
            "LogLevel=DEBUG1",
            "-p",
            "2222",
            "me@host",
        ]
        .map(String::from)
        .to_vec();
        let argv = sidecar_argv(&session, false, PERMIT_REMOTE_OPEN).unwrap();
        assert_eq!(argv[0], "ssh");
        assert!(argv.contains(&"-N".to_string()));
        assert!(!argv.iter().any(|a| a == "-t" || a == "-A"));
        assert_eq!(argv.last().map(String::as_str), Some("me@host"));
        // ssh keeps the first value of an option, so ours must come first.
        let at = |opt: &str| argv.iter().position(|a| a == opt).unwrap();
        assert!(at("StrictHostKeyChecking=yes") < at("StrictHostKeyChecking=ask"));
        assert!(at("LogLevel=INFO") < at("LogLevel=DEBUG1"));
        assert!(argv.contains(&"ExitOnForwardFailure=yes".to_string()));
        assert!(argv.contains(&"BatchMode=yes".to_string()));
        assert!(sidecar_argv(&["mosh".into(), "host".into()], false, "*:80").is_none());
    }

    #[test]
    fn parses_what_openssh_writes() {
        assert_eq!(
            parse_log_line("Allocated port 45509 for remote forward to socks:0"),
            Some(NetShare::Port(45509))
        );
        assert_eq!(
            parse_log_line("Warning: remote port forwarding failed for listen port 0"),
            Some(NetShare::Failed)
        );
        // A debug line quoting the words is not the message.
        assert_eq!(
            parse_log_line("debug1: Remote: Allocated port 1 for remote forward to x:0"),
            None
        );
        assert_eq!(
            parse_log_line("Allocated port 99999 for remote forward"),
            None
        );
        // A RemoteForward of the user's own is not the proxy.
        assert_eq!(
            parse_log_line("Allocated port 5 for remote forward to localhost:22"),
            None
        );
    }
}
