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

    /// Remove a remote path. A file is unlinked; a directory is removed
    /// recursively (contents first, then the directory itself).
    /// Follow-symlink stat: `(is_dir, size)` of the resolved target. Transfer
    /// planning uses it to classify symlinks — a symlink-to-file transfers
    /// with the target's size, symlink-to-dir and broken links are skipped.
    fn stat(&mut self, path: &Path) -> Result<(bool, u64)>;

    fn remove(&mut self, path: &Path, is_dir: bool) -> Result<()>;

    /// Create a remote directory (mode 0755).
    fn mkdir(&mut self, path: &Path) -> Result<()>;

    /// Rename / move a remote path. Fails if `to` already exists.
    fn rename(&mut self, from: &Path, to: &Path) -> Result<()>;

    /// Set the permission bits of a remote path (chmod).
    fn chmod(&mut self, path: &Path, mode: u32) -> Result<()>;
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
                // Trust on first use. Append the entry ourselves rather than
                // libssh2 write_file, which rewrites the whole file and would
                // silently drop lines it can't parse (@cert-authority, @revoked,
                // certificate entries, unsupported key types).
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                match keytype_name(key_type) {
                    Some(kt_name) => {
                        let hostspec = if port == 22 {
                            host.to_string()
                        } else {
                            format!("[{host}]:{port}")
                        };
                        // Prefix a newline if the existing file doesn't already
                        // end in one, so we never concatenate onto a prior entry.
                        let needs_nl = std::fs::read(&path)
                            .ok()
                            .map(|b| !b.is_empty() && b.last() != Some(&b'\n'))
                            .unwrap_or(false);
                        let line = format!(
                            "{}{hostspec} {kt_name} {}\n",
                            if needs_nl { "\n" } else { "" },
                            b64encode(key)
                        );
                        let mut f = std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&path)
                            .with_context(|| format!("failed to open {}", path.display()))?;
                        f.write_all(line.as_bytes())
                            .with_context(|| format!("failed to append to {}", path.display()))?;
                    }
                    None => {
                        // Unknown key type we can't format — fall back to libssh2.
                        let fmt = KnownHostKeyFormat::from(key_type);
                        known
                            .add(host, key, "added by sshub (TOFU)", fmt)
                            .context("failed to record new host key")?;
                        known
                            .write_file(&path, KnownHostFileKind::OpenSSH)
                            .with_context(|| format!("failed to write {}", path.display()))?;
                    }
                }
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
            // No stored secret: try the ssh-agent, then an unencrypted identity
            // file, before giving up (interactive prompts aren't possible here).
            None => {
                let mut ok = false;
                if self.agent.socket_path.is_some() {
                    ok = session.userauth_agent(&user).is_ok() && session.authenticated();
                }
                if !ok {
                    if let Some(key_path) = self.host.identity_file.as_ref() {
                        let key_path = crate::app::shellexpand_home(key_path);
                        ok = session
                            .userauth_pubkey_file(&user, None, &key_path, None)
                            .is_ok()
                            && session.authenticated();
                    }
                }
                if !ok {
                    return Err(anyhow!(
                        "authentication failed: no stored password/passphrase, ssh-agent, or usable unencrypted key"
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
                    is_symlink: is_symlink(&stat),
                    // Low 12 bits: permission + setuid/setgid/sticky.
                    perm: stat.perm.map(|p| p & 0o7777),
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
        // Stream to a sibling temp, then rename over the destination on success —
        // an aborted/failed transfer never truncates an existing local file.
        let tmp = temp_sibling(local);
        let stream = (|| -> Result<()> {
            let mut out = std::fs::File::create(&tmp)
                .with_context(|| format!("failed to create local {}", tmp.display()))?;
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
        })();
        match stream {
            Ok(()) => std::fs::rename(&tmp, local)
                .map_err(|e| {
                    // Don't leave the streamed temp behind if the rename fails.
                    let _ = std::fs::remove_file(&tmp);
                    e
                })
                .with_context(|| format!("failed to finalize {}", local.display())),
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                Err(e)
            }
        }
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
        // Upload to a remote sibling temp, then rename over the destination.
        let tmp = temp_sibling(remote);
        let stream = (|| -> Result<()> {
            let mut remote_file = sftp
                .create(&tmp)
                .with_context(|| format!("failed to create remote {}", tmp.display()))?;
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
        })();
        match stream {
            Ok(()) => {
                // Prefer an atomic overwrite; fall back to unlink+rename for SFTP
                // servers without the posix-rename/overwrite extension.
                let renamed = sftp
                    .rename(&tmp, remote, Some(ssh2::RenameFlags::OVERWRITE))
                    .or_else(|_| {
                        let _ = sftp.unlink(remote);
                        sftp.rename(&tmp, remote, None)
                    });
                if renamed.is_err() {
                    let _ = sftp.unlink(&tmp);
                }
                renamed.with_context(|| format!("failed to finalize remote {}", remote.display()))
            }
            Err(e) => {
                let _ = sftp.unlink(&tmp);
                Err(e)
            }
        }
    }

    fn stat(&mut self, path: &Path) -> Result<(bool, u64)> {
        let sftp = self.sftp_ref()?;
        let st = sftp
            .stat(path)
            .with_context(|| format!("failed to stat {}", path.display()))?;
        Ok((st.is_dir(), st.size.unwrap_or(0)))
    }

    fn remove(&mut self, path: &Path, is_dir: bool) -> Result<()> {
        let sftp = self.sftp_ref()?;
        if is_dir {
            remove_dir_recursive(sftp, path)
        } else {
            sftp.unlink(path)
                .with_context(|| format!("failed to delete {}", path.display()))
        }
    }

    fn mkdir(&mut self, path: &Path) -> Result<()> {
        let sftp = self.sftp_ref()?;
        sftp.mkdir(path, 0o755)
            .with_context(|| format!("failed to create {}", path.display()))
    }

    fn rename(&mut self, from: &Path, to: &Path) -> Result<()> {
        let sftp = self.sftp_ref()?;
        // No flags → the server refuses to clobber an existing target, so a
        // rename onto an existing name surfaces an error instead of destroying it.
        sftp.rename(from, to, None)
            .with_context(|| format!("failed to rename {} to {}", from.display(), to.display()))
    }

    fn chmod(&mut self, path: &Path, mode: u32) -> Result<()> {
        let sftp = self.sftp_ref()?;
        let stat = ssh2::FileStat {
            size: None,
            uid: None,
            gid: None,
            perm: Some(mode),
            atime: None,
            mtime: None,
        };
        sftp.setstat(path, stat)
            .with_context(|| format!("failed to chmod {} to {:o}", path.display(), mode))
    }
}

/// S_IFLNK (0o120000) in the file-type bits of the mode. `readdir` returns
/// lstat-style attributes, so this identifies the link itself, not its target.
fn is_symlink(stat: &ssh2::FileStat) -> bool {
    stat.perm.map(|p| p & 0o170000 == 0o120000).unwrap_or(false)
}

/// Recursively delete a remote directory: contents first (files unlinked,
/// subdirs recursed), then the now-empty directory via `rmdir`. `readdir`
/// yields file names, so child paths are rebuilt as `dir.join(name)`.
fn remove_dir_recursive(sftp: &ssh2::Sftp, dir: &Path) -> Result<()> {
    let entries = sftp
        .readdir(dir)
        .with_context(|| format!("failed to list {}", dir.display()))?;
    for (p, stat) in entries {
        let Some(name) = p.file_name() else { continue };
        let name = name.to_string_lossy();
        if name.is_empty() || name == "." || name == ".." {
            continue;
        }
        let child = dir.join(name.as_ref());
        // A symlink (even to a directory) is unlinked, never followed — otherwise
        // we'd recurse through the link and delete files outside this tree.
        if stat.is_dir() && !is_symlink(&stat) {
            remove_dir_recursive(sftp, &child)?;
        } else {
            sftp.unlink(&child)
                .with_context(|| format!("failed to delete {}", child.display()))?;
        }
    }
    sftp.rmdir(dir)
        .with_context(|| format!("failed to remove directory {}", dir.display()))
}

/// A hidden sibling temp path next to `path`, for atomic write-then-rename.
fn temp_sibling(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    let tmp_name = format!(".{name}.sshub-part");
    match path.parent() {
        Some(parent) => parent.join(tmp_name),
        None => PathBuf::from(tmp_name),
    }
}

/// OpenSSH known_hosts key-type token for a libssh2 host-key type, if we can
/// serialize it ourselves. None → let libssh2 write the file instead.
fn keytype_name(kt: ssh2::HostKeyType) -> Option<&'static str> {
    use ssh2::HostKeyType::*;
    match kt {
        Rsa => Some("ssh-rsa"),
        Dss => Some("ssh-dss"),
        Ecdsa256 => Some("ecdsa-sha2-nistp256"),
        Ecdsa384 => Some("ecdsa-sha2-nistp384"),
        Ecdsa521 => Some("ecdsa-sha2-nistp521"),
        Ed25519 => Some("ssh-ed25519"),
        _ => None,
    }
}

/// Standard base64 (with padding) — enough to serialize a known_hosts key blob.
fn b64encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            T[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}
