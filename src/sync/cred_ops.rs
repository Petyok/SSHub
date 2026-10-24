//! Credential replication: what a credential contains, how it is written into the op-log under
//! the ring's mode, how an identity key reaches a file ssh can read, and the opt-in migration of
//! secrets that so far lived only in the local keyring or `credentials.json`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, ensure, Context, Result};
use serde_json::{json, Value};
use zeroize::Zeroize;

use super::crypto::{
    dek_generate, dek_seal, hybrid_wrap, shamir_split_at, DevicePublic, ALG_CURRENT,
};
use super::oplog::{CredMode, HostUid, Op, OpBody, WrappedShare};
use super::quorum::{share_recipients, vault_pub, vault_wrap, SHARE_K};
use super::share_release::wrapped_share_plaintext;
use crate::credentials::PasswordStore;
use crate::store::{uid_hex, LauncherStore};

/// The plaintext of a credential before it is sealed under its DEK. Strict: unknown fields and
/// versions are refused, so an old build never half-reads a newer record.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct CredPayload {
    pub password: Option<String>,
    /// Private key text (an identity's key file contents).
    pub private_key: Option<String>,
}

impl std::fmt::Debug for CredPayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CredPayload(..)")
    }
}

impl Drop for CredPayload {
    fn drop(&mut self) {
        self.password.zeroize();
        self.private_key.zeroize();
    }
}

impl CredPayload {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = json!({ "v": 1 });
        if let Some(p) = &self.password {
            v["password"] = json!(p);
        }
        if let Some(k) = &self.private_key {
            v["private_key"] = json!(k);
        }
        serde_json::to_vec(&v).expect("json")
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self> {
        let v: Value = serde_json::from_slice(b).context("credential payload is not JSON")?;
        let m = v
            .as_object()
            .context("credential payload is not an object")?;
        ensure!(
            m.get("v") == Some(&json!(1)),
            "unsupported credential payload version"
        );
        let mut out = Self::default();
        for (k, val) in m {
            match k.as_str() {
                "v" => {}
                "password" => {
                    out.password = Some(
                        val.as_str()
                            .context("password is not a string")?
                            .to_string(),
                    )
                }
                "private_key" => {
                    out.private_key = Some(
                        val.as_str()
                            .context("private_key is not a string")?
                            .to_string(),
                    )
                }
                other => bail!("unknown credential field {other}"),
            }
        }
        Ok(out)
    }
}

/// Seal `payload` under a fresh DEK and wrap the DEK for the ring's mode. The caller appends the result.
pub fn cred_update_body(
    store: &LauncherStore,
    me: &DevicePublic,
    host: &HostUid,
    payload: &CredPayload,
) -> Result<OpBody> {
    cred_update_body_for(
        store,
        me,
        host,
        payload,
        super::ceremony::effective_cred_mode(store)?,
    )
}

/// [`cred_update_body`] for an explicit mode (a mode change writes the new shape before the mode
/// itself has been applied).
pub fn cred_update_body_for(
    store: &LauncherStore,
    me: &DevicePublic,
    host: &HostUid,
    payload: &CredPayload,
    mode: CredMode,
) -> Result<OpBody> {
    let dek = zeroize::Zeroizing::new(dek_generate());
    let ciphertext = dek_seal(&dek, &payload.to_bytes());
    let wrapped_shares = match mode {
        CredMode::Vault => {
            let vault = vault_pub(store)?
                .context("no vault is set up on this device: run ring setup or import the kit")?;
            vec![vault_wrap(&vault, host, &dek)?]
        }
        CredMode::Quorum => {
            super::quorum::paper_rule(store, super::quorum::live_holder_count(store)?)?;
            let recipients = share_recipients(store, me)?;
            ensure!(
                recipients.len() >= SHARE_K as usize,
                "quorum mode needs at least two holders (a second device, the phone or the paper)"
            );
            ensure!(recipients.len() <= 255, "too many holders");
            let indices: Vec<u8> = (1..=recipients.len() as u8).collect();
            let shares = shamir_split_at(&dek, SHARE_K, &indices);
            recipients
                .iter()
                .zip(&shares)
                .map(|((id, public), share)| WrappedShare {
                    recipient: id.clone(),
                    alg: ALG_CURRENT,
                    sealed: hybrid_wrap(public, &wrapped_share_plaintext(host, share)),
                })
                .collect()
        }
    };
    Ok(OpBody::CredUpdate {
        host_uid: *host,
        ciphertext,
        wrapped_shares,
    })
}

/// Create or replace the credential of `host` (a host or identity uid) and chain the op.
pub fn cred_put(
    store: &LauncherStore,
    id: &super::crypto::DeviceIdentity,
    host: &HostUid,
    payload: &CredPayload,
) -> Result<Op> {
    store.sync_append_local(id, cred_update_body(store, &id.public(), host, payload)?)
}

/// An identity key written to a file ssh can read; removed on drop.
pub struct MaterialisedKey {
    path: PathBuf,
}

impl MaterialisedKey {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for MaterialisedKey {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Write the key of an opened identity credential to `dir/<uid>` with mode 0600 (create-new, so an
/// existing file is never followed or overwritten). `dir` should be tmpfs: the caller decides.
pub fn materialise_key(
    payload: &CredPayload,
    dir: &Path,
    identity_uid: &HostUid,
) -> Result<Option<MaterialisedKey>> {
    let Some(key) = &payload.private_key else {
        return Ok(None);
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        let path = dir.join(uid_hex(identity_uid));
        let _ = fs::remove_file(&path);
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        f.write_all(key.as_bytes())?;
        if !key.ends_with('\n') {
            f.write_all(b"\n")?;
        }
        f.flush()?;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
        Ok(Some(MaterialisedKey { path }))
    }
    #[cfg(not(unix))]
    {
        let _ = (dir, identity_uid, key);
        bail!("materialising keys needs a unix file mode")
    }
}

/// What a migration read and would replicate.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MigrationReport {
    /// Local keyring keys (`host:7`, `identity:3`) re-encrypted into `CredUpdate` ops.
    pub migrated: Vec<String>,
    /// Keys that had nothing to read.
    pub skipped: Vec<String>,
}

/// Opt-in, at ring setup: READ the local secrets of hosts and identities (and identity key files)
/// and write them as encrypted replicated credentials. `local` must be the profile's ungated
/// store (a gated one answers `Locked` for synced keys). The local copies stay until
/// [`remove_local_copies`] after the user confirmed.
pub fn migrate_local_secrets(
    store: &LauncherStore,
    id: &super::crypto::DeviceIdentity,
    local: &dyn PasswordStore,
) -> Result<MigrationReport> {
    struct Row {
        key: String,
        uid: HostUid,
        key_path: Option<String>,
    }
    let rows: Vec<Row> = store.with_conn(|c| {
        let mut out = Vec::new();
        let mut st = c.prepare(
            "SELECT id, uid FROM hosts WHERE uid IS NOT NULL AND has_password = 1 ORDER BY id",
        )?;
        for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))? {
            let (rid, uid) = r?;
            out.push(Row {
                key: crate::credentials::host_key(rid),
                uid: uid.try_into().map_err(|_| anyhow!("host uid"))?,
                key_path: None,
            });
        }
        let mut st = c.prepare(
            "SELECT id, uid, private_key FROM identities WHERE uid IS NOT NULL ORDER BY id",
        )?;
        for r in st.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Vec<u8>>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })? {
            let (rid, uid, key_path) = r?;
            out.push(Row {
                key: crate::credentials::identity_key(rid),
                uid: uid.try_into().map_err(|_| anyhow!("identity uid"))?,
                key_path,
            });
        }
        Ok(out)
    })?;
    let mut report = MigrationReport::default();
    for row in rows {
        let password = local.get(&row.key).ok().flatten();
        let private_key = row
            .key_path
            .as_deref()
            .filter(|p| !p.is_empty())
            .and_then(|p| fs::read_to_string(crate::ssh::expand_tilde(p)).ok())
            .filter(|t| crate::ssh::looks_like_private_key(t));
        if password.is_none() && private_key.is_none() {
            report.skipped.push(row.key);
            continue;
        }
        cred_put(
            store,
            id,
            &row.uid,
            &CredPayload {
                password,
                private_key,
            },
        )?;
        report.migrated.push(row.key);
    }
    Ok(report)
}

/// After the user confirmed that the replicated copies open: drop the local keyring entries.
pub fn remove_local_copies(local: &dyn PasswordStore, report: &MigrationReport) -> Result<()> {
    for k in &report.migrated {
        local.delete(k)?;
    }
    Ok(())
}

// ------------------------------------------------------------------ private ssh-agent

/// How `ssh-add` obtains a key's passphrase without a terminal: the program and environment of the
/// askpass helper (`SSH_ASKPASS_REQUIRE=force` is set on top).
#[derive(Clone, Debug, Default)]
pub struct AskpassEnv {
    pub program: PathBuf,
    pub env: Vec<(String, String)>,
}

/// A key held by the agent: the public key file ssh selects it with (public data, safe on disk).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentKey {
    pub public_file: PathBuf,
    /// The `ssh-add -L` line (type, base64, comment).
    pub public_line: String,
}

/// A private `ssh-agent`, one per process, that holds synced identity keys so that no private key byte
/// is ever written to disk on any OS. The agent socket lives in a 0700 directory; the key goes in over
/// stdin (`ssh-add -`) with a lifetime; `ssh` is told `-o IdentityAgent=<socket> -o IdentitiesOnly=yes
/// -i <public key file>`. Dropping the agent kills it and removes the directory.
pub struct PrivateAgent {
    child: std::process::Child,
    dir: PathBuf,
    sock: PathBuf,
    next: usize,
}

impl PrivateAgent {
    /// Start `ssh-agent -D -a <socket>` in a fresh private directory under the temp dir.
    pub fn start() -> Result<Self> {
        use rand_core::{OsRng, RngCore};
        let mut r = [0u8; 4];
        OsRng.fill_bytes(&mut r);
        // Short on purpose: a unix socket path is limited to about 100 bytes (macOS temp dirs are long).
        let dir = std::env::temp_dir().join(format!("sshub-a{}", hex::encode(r)));
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new()
                .mode(0o700)
                .create(&dir)
                .with_context(|| format!("create {}", dir.display()))?;
        }
        #[cfg(not(unix))]
        bail!("a private ssh-agent needs unix sockets");
        let sock = dir.join("s");
        let child = std::process::Command::new("ssh-agent")
            .arg("-D")
            .arg("-a")
            .arg(&sock)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| {
                let _ = fs::remove_dir_all(&dir);
                anyhow!("could not start ssh-agent (is the OpenSSH client installed?): {e}")
            })?;
        let agent = Self {
            child,
            dir,
            sock,
            next: 0,
        };
        for _ in 0..100 {
            if agent.sock.exists() {
                return Ok(agent);
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        bail!("ssh-agent did not create its socket")
    }

    pub fn socket(&self) -> &Path {
        &self.sock
    }

    fn ssh_add(
        &self,
        args: &[&str],
        stdin: Option<&[u8]>,
        askpass: Option<&AskpassEnv>,
    ) -> Result<std::process::Output> {
        let mut cmd = std::process::Command::new("ssh-add");
        cmd.args(args)
            .env("SSH_AUTH_SOCK", &self.sock)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if let Some(a) = askpass {
            cmd.env("SSH_ASKPASS", &a.program)
                .env("SSH_ASKPASS_REQUIRE", "force")
                .env_remove("DISPLAY");
            cmd.env("DISPLAY", "sshub");
            for (k, v) in &a.env {
                cmd.env(k, v);
            }
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow!("could not run ssh-add: {e}"))?;
        if let Some(bytes) = stdin {
            child
                .stdin
                .take()
                .context("ssh-add stdin")?
                .write_all(bytes)?;
        } else {
            drop(child.stdin.take());
        }
        Ok(child.wait_with_output()?)
    }

    fn listed(&self) -> Result<Vec<String>> {
        let out = self.ssh_add(&["-L"], None, None)?;
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with("The agent has no"))
            .map(String::from)
            .collect())
    }

    /// Add a private key (PEM text) for `ttl_secs` and write its public key file. A passphrase-protected
    /// key needs `askpass`.
    pub fn add_key(
        &mut self,
        private_key: &str,
        ttl_secs: u32,
        askpass: Option<&AskpassEnv>,
    ) -> Result<AgentKey> {
        let before = self.listed()?;
        let mut pem = zeroize::Zeroizing::new(private_key.as_bytes().to_vec());
        if !pem.ends_with(b"\n") {
            pem.push(b'\n');
        }
        let out = self.ssh_add(&["-t", &ttl_secs.to_string(), "-"], Some(&pem), askpass)?;
        ensure!(
            out.status.success(),
            "ssh-add refused the key: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
        let line = self
            .listed()?
            .into_iter()
            .find(|l| !before.contains(l))
            .context("the key was added but the agent does not list it (already present?)")?;
        self.next += 1;
        let public_file = self.dir.join(format!("k{}.pub", self.next));
        fs::write(&public_file, format!("{line}\n"))?;
        Ok(AgentKey {
            public_file,
            public_line: line,
        })
    }

    /// The `ssh` arguments that use only this agent and this key.
    pub fn ssh_args(&self, key: &AgentKey) -> Vec<String> {
        vec![
            "-o".into(),
            format!("IdentityAgent={}", self.sock.display()),
            "-o".into(),
            "IdentitiesOnly=yes".into(),
            "-i".into(),
            key.public_file.display().to_string(),
        ]
    }
}

impl Drop for PrivateAgent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[cfg(test)]
mod agent_tests {
    use super::*;
    use std::process::Command;

    fn have(tool: &str) -> bool {
        Command::new(tool).arg("-V").output().is_ok() || Command::new(tool).output().is_ok()
    }

    fn keygen(dir: &Path, name: &str, pass: &str) -> (String, String) {
        let f = dir.join(name);
        let st = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", pass, "-C", "t", "-f"])
            .arg(&f)
            .status()
            .unwrap();
        assert!(st.success());
        (
            fs::read_to_string(&f).unwrap(),
            fs::read_to_string(f.with_extension("pub")).unwrap(),
        )
    }

    #[test]
    fn the_agent_holds_the_key_ssh_selects_it_and_nothing_private_touches_the_disk() {
        if !have("ssh-agent") || !have("ssh-keygen") {
            eprintln!("skipped: no OpenSSH client tools");
            return;
        }
        let work = std::env::temp_dir().join(format!("sshub-agent-test-{}", std::process::id()));
        fs::create_dir_all(&work).unwrap();
        let (pem, public) = keygen(&work, "plain", "");
        let (pem_enc, public_enc) = keygen(&work, "locked", "correct horse");
        let askpass = work.join("askpass.sh");
        fs::write(&askpass, "#!/bin/sh\necho 'correct horse'\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&askpass, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let ask = AskpassEnv {
            program: askpass.clone(),
            env: vec![],
        };

        let (sock, dir);
        {
            let mut agent = PrivateAgent::start().unwrap();
            sock = agent.socket().to_path_buf();
            dir = sock.parent().unwrap().to_path_buf();
            let k = agent.add_key(&pem, 600, None).unwrap();
            let want: Vec<&str> = public.split_whitespace().take(2).collect();
            let got: Vec<&str> = k.public_line.split_whitespace().take(2).collect();
            assert_eq!(got, want, "the agent lists the key's public half");
            let args = agent.ssh_args(&k);
            assert_eq!(
                &args[..4],
                [
                    "-o",
                    &format!("IdentityAgent={}", sock.display()),
                    "-o",
                    "IdentitiesOnly=yes"
                ]
            );
            assert_eq!(args[5], k.public_file.display().to_string());

            // A passphrase-protected key needs the askpass and works with it; without it, it is refused.
            assert!(agent.add_key(&pem_enc, 600, None).is_err());
            let k2 = agent.add_key(&pem_enc, 600, Some(&ask)).unwrap();
            assert_eq!(
                k2.public_line.split_whitespace().nth(1),
                public_enc.split_whitespace().nth(1)
            );
            // Garbage is refused and nothing is added.
            assert!(agent.add_key("not a key", 600, None).is_err());

            // The directory holds a socket and public files: no private key byte.
            for entry in fs::read_dir(&dir).unwrap() {
                let p = entry.unwrap().path();
                if p.is_file() {
                    assert!(
                        !fs::read_to_string(&p)
                            .unwrap_or_default()
                            .contains("PRIVATE KEY"),
                        "{}",
                        p.display()
                    );
                }
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
                    0o700
                );
            }
        }
        // Dropping removes the directory (and the agent with it).
        assert!(!dir.exists() && !sock.exists());
        let _ = fs::remove_dir_all(&work);
    }
}
