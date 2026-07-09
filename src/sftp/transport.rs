//! SFTP transport abstraction + the concrete libssh2 (`ssh2` crate) backend.
//!
//! The worker ([`super::worker`]) drives a [`SftpTransport`]; the trait exists
//! so the worker (and any future backend) is testable against a fake. The real
//! implementation, [`Ssh2Transport`], owns an `ssh2::Session` + `ssh2::Sftp`
//! and implements the app's trust-on-first-use (TOFU) host-key policy: an
//! unknown key is recorded and accepted, only a *changed* key is refused.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use ssh2::{CheckResult, KnownHostFileKind, KnownHostKeyFormat, Session};

use super::model::FileEntry;
use crate::session::PendingSecret;
use crate::ssh::agent::AgentInfo;
use crate::ssh::SshHost;

/// Behaviour every SFTP backend exposes to the worker. All calls are blocking.
pub trait SftpTransport {
    /// Establish the TCP connection, handshake, verify the host key (TOFU) and
    /// authenticate. Must be called once before any other method.
    fn connect(&mut self) -> Result<()>;

    /// List a remote directory. Names are file names only (no path).
    fn list_dir(&mut self, path: &Path) -> Result<Vec<FileEntry>>;

    /// Download `remote` to `local`, reporting `(transferred, total)` as it goes.
    fn download(
        &mut self,
        remote: &Path,
        local: &Path,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<()>;

    /// Upload `local` to `remote`, reporting `(transferred, total)` as it goes.
    fn upload(
        &mut self,
        local: &Path,
        remote: &Path,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<()>;
}

/// Chunk size for streaming transfers (also the progress throttle granularity).
const CHUNK: usize = 64 * 1024;

/// libssh2-backed transport over `ssh2::Session` / `ssh2::Sftp`.
pub struct Ssh2Transport {
    host: SshHost,
    secret: Option<PendingSecret>,
    agent: AgentInfo,
    session: Option<Session>,
    sftp: Option<ssh2::Sftp>,
}

impl Ssh2Transport {
    /// Build a transport for `host`. `secret` is the stored password/passphrase
    /// (if any) and `agent` describes the running ssh-agent (for agent auth).
    /// Nothing connects until [`connect`](SftpTransport::connect) is called.
    pub fn new(host: SshHost, secret: Option<PendingSecret>, agent: AgentInfo) -> Self {
        Self {
            host,
            secret,
            agent,
            session: None,
            sftp: None,
        }
    }

    /// Effective remote host:port, applying the SshHost fallbacks (host.rs:64).
    fn address(&self) -> (String, u16) {
        let host = self
            .host
            .hostname
            .clone()
            .unwrap_or_else(|| self.host.name.clone());
        let port = self.host.port.unwrap_or(22);
        (host, port)
    }

    /// Username for auth: the host's `user`, else `$USER`, else `root`.
    fn username(&self) -> String {
        self.host
            .user
            .clone()
            .or_else(|| std::env::var("USER").ok())
            .unwrap_or_else(|| "root".to_string())
    }

    /// Path to `~/.ssh/known_hosts`.
    fn known_hosts_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        Path::new(&home).join(".ssh").join("known_hosts")
    }

    fn sftp_ref(&mut self) -> Result<&ssh2::Sftp> {
        self.sftp
            .as_ref()
            .ok_or_else(|| anyhow!("SFTP subsystem not opened"))
    }

    /// Trust-on-first-use host-key verification. Mirrors the app's
    /// `StrictHostKeyChecking=accept-new`: an unknown key is recorded + accepted,
    /// only a *changed* key is refused (possible MITM).
    fn verify_host_key(session: &Session, host: &str, port: u16) -> Result<()> {
        let (key, key_type) = session
            .host_key()
            .ok_or_else(|| anyhow!("server did not present a host key"))?;

        let mut known = session
            .known_hosts()
            .context("failed to open known_hosts tracker")?;

        let path = Self::known_hosts_path();
        // Missing/empty known_hosts is fine — every host is then "not found"
        // and gets recorded below. Only surface real read errors.
        if path.exists() {
            known
                .read_file(&path, KnownHostFileKind::OpenSSH)
                .with_context(|| format!("failed to read {}", path.display()))?;
        }

        match known.check_port(host, port, key) {
            CheckResult::Match => Ok(()),
            CheckResult::Mismatch => Err(anyhow!(
                "remote host key changed - possible MITM; verify before connecting"
            )),
            CheckResult::NotFound => {
                // Trust on first use: record the key and persist it.
                let fmt = KnownHostKeyFormat::from(key_type);
                known
                    .add(host, key, "added by sshub (TOFU)", fmt)
                    .context("failed to record new host key")?;
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                known
                    .write_file(&path, KnownHostFileKind::OpenSSH)
                    .with_context(|| format!("failed to write {}", path.display()))?;
                Ok(())
            }
            CheckResult::Failure => Err(anyhow!("host key check failed")),
        }
    }

    /// Run the auth handshake per the available credential.
    fn authenticate(&self) -> Result<()> {
        let session = self.session.as_ref().unwrap();
        let user = self.username();

        match &self.secret {
            // A stored passphrase means "unlock this private key file".
            Some(PendingSecret::Passphrase(pass)) => {
                let key_path = self.host.identity_file.as_ref().ok_or_else(|| {
                    anyhow!("a passphrase is stored but no identity file is configured")
                })?;
                let key_path = crate::app::shellexpand_home(key_path);
                session
                    .userauth_pubkey_file(&user, None, &key_path, Some(pass))
                    .context("public-key authentication failed")?;
            }
            // A stored password → password auth.
            Some(PendingSecret::Password(pw)) => {
                session
                    .userauth_password(&user, pw)
                    .context("password authentication failed")?;
            }
            // No stored secret: use the agent if one is running, else fail with
            // a clear message (interactive prompts aren't possible here).
            None => {
                if self.agent.socket_path.is_some() {
                    session
                        .userauth_agent(&user)
                        .context("ssh-agent authentication failed")?;
                } else {
                    return Err(anyhow!(
                        "no credential available: no stored secret and no ssh-agent"
                    ));
                }
            }
        }

        if !session.authenticated() {
            return Err(anyhow!("authentication did not complete"));
        }
        Ok(())
    }
}

impl SftpTransport for Ssh2Transport {
    fn connect(&mut self) -> Result<()> {
        if self.host.proxy_jump.is_some() {
            return Err(anyhow!("SFTP via ProxyJump isn't supported yet"));
        }

        let (host, port) = self.address();
        let tcp = TcpStream::connect((host.as_str(), port))
            .with_context(|| format!("could not connect to {host}:{port}"))?;

        let mut session = Session::new().context("failed to create SSH session")?;
        session.set_tcp_stream(tcp);
        session.handshake().context("SSH handshake failed")?;

        Self::verify_host_key(&session, &host, port)?;

        self.session = Some(session);
        self.authenticate()?;

        let sftp = self
            .session
            .as_ref()
            .unwrap()
            .sftp()
            .context("failed to open SFTP subsystem")?;
        self.sftp = Some(sftp);
        Ok(())
    }

    fn list_dir(&mut self, path: &Path) -> Result<Vec<FileEntry>> {
        let sftp = self.sftp_ref()?;
        let raw = sftp
            .readdir(path)
            .with_context(|| format!("failed to list {}", path.display()))?;

        let mut entries: Vec<FileEntry> = raw
            .into_iter()
            .filter_map(|(p, stat)| {
                let name = p.file_name()?.to_string_lossy().to_string();
                if name.is_empty() || name == "." || name == ".." {
                    return None;
                }
                Some(FileEntry {
                    name,
                    is_dir: stat.is_dir(),
                    size: stat.size.unwrap_or(0),
                })
            })
            .collect();

        // Directories first, then case-insensitive by name.
        entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(entries)
    }

    fn download(
        &mut self,
        remote: &Path,
        local: &Path,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<()> {
        let sftp = self.sftp_ref()?;
        let mut remote_file = sftp
            .open(remote)
            .with_context(|| format!("failed to open remote {}", remote.display()))?;
        let total = remote_file.stat().ok().and_then(|s| s.size).unwrap_or(0);

        if let Some(parent) = local.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let mut out = std::fs::File::create(local)
            .with_context(|| format!("failed to create local {}", local.display()))?;

        let mut buf = vec![0u8; CHUNK];
        let mut transferred: u64 = 0;
        progress(0, total);
        loop {
            let n = remote_file
                .read(&mut buf)
                .with_context(|| format!("read error on {}", remote.display()))?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])
                .with_context(|| format!("write error on {}", local.display()))?;
            transferred += n as u64;
            progress(transferred, total.max(transferred));
        }
        out.flush().ok();
        Ok(())
    }

    fn upload(
        &mut self,
        local: &Path,
        remote: &Path,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<()> {
        let mut in_file = std::fs::File::open(local)
            .with_context(|| format!("failed to open local {}", local.display()))?;
        let total = in_file.metadata().map(|m| m.len()).unwrap_or(0);

        let sftp = self.sftp_ref()?;
        let mut remote_file = sftp
            .create(remote)
            .with_context(|| format!("failed to create remote {}", remote.display()))?;

        let mut buf = vec![0u8; CHUNK];
        let mut transferred: u64 = 0;
        progress(0, total);
        loop {
            let n = in_file
                .read(&mut buf)
                .with_context(|| format!("read error on {}", local.display()))?;
            if n == 0 {
                break;
            }
            remote_file
                .write_all(&buf[..n])
                .with_context(|| format!("write error on {}", remote.display()))?;
            transferred += n as u64;
            progress(transferred, total.max(transferred));
        }
        Ok(())
    }
}
