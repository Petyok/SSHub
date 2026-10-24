//! Credential replication: what a credential contains, how it is written into the op-log under
//! the ring's mode, how an identity key reaches a file ssh can read, and the opt-in migration of
//! secrets that so far lived only in the local keyring or `credentials.json`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, ensure, Context, Result};
use serde_json::{json, Value};
use zeroize::Zeroize;

use super::crypto::{dek_generate, dek_seal, hybrid_wrap, shamir_split_at, ALG_CURRENT};
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
        let m = v.as_object().context("credential payload is not an object")?;
        ensure!(m.get("v") == Some(&json!(1)), "unsupported credential payload version");
        let mut out = Self::default();
        for (k, val) in m {
            match k.as_str() {
                "v" => {}
                "password" => out.password = Some(val.as_str().context("password is not a string")?.to_string()),
                "private_key" => out.private_key = Some(val.as_str().context("private_key is not a string")?.to_string()),
                other => bail!("unknown credential field {other}"),
            }
        }
        Ok(out)
    }
}

/// Seal `payload` under a fresh DEK and wrap the DEK for the ring's mode. The caller appends the result.
pub fn cred_update_body(store: &LauncherStore, host: &HostUid, payload: &CredPayload) -> Result<OpBody> {
    cred_update_body_for(store, host, payload, store.sync_cred_mode()?)
}

/// [`cred_update_body`] for an explicit mode (a mode change writes the new shape before the mode
/// itself has been applied).
pub fn cred_update_body_for(store: &LauncherStore, host: &HostUid, payload: &CredPayload, mode: CredMode) -> Result<OpBody> {
    let dek = zeroize::Zeroizing::new(dek_generate());
    let ciphertext = dek_seal(&dek, &payload.to_bytes());
    let wrapped_shares = match mode {
        CredMode::Vault => {
            let vault = vault_pub(store)?.context("no vault is set up on this device: run ring setup or import the kit")?;
            vec![vault_wrap(&vault, host, &dek)?]
        }
        CredMode::Quorum => {
            let recipients = share_recipients(store)?;
            ensure!(recipients.len() >= SHARE_K as usize, "quorum mode needs at least two holders (a second device, the phone or the paper)");
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
    Ok(OpBody::CredUpdate { host_uid: *host, ciphertext, wrapped_shares })
}

/// Create or replace the credential of `host` (a host or identity uid) and chain the op.
pub fn cred_put(store: &LauncherStore, id: &super::crypto::DeviceIdentity, host: &HostUid, payload: &CredPayload) -> Result<Op> {
    store.sync_append_local(id, cred_update_body(store, host, payload)?)
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
pub fn materialise_key(payload: &CredPayload, dir: &Path, identity_uid: &HostUid) -> Result<Option<MaterialisedKey>> {
    let Some(key) = &payload.private_key else { return Ok(None) };
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
        fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        let path = dir.join(uid_hex(identity_uid));
        let _ = fs::remove_file(&path);
        let mut f = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path)?;
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
pub fn migrate_local_secrets(store: &LauncherStore, id: &super::crypto::DeviceIdentity, local: &dyn PasswordStore) -> Result<MigrationReport> {
    struct Row {
        key: String,
        uid: HostUid,
        key_path: Option<String>,
    }
    let rows: Vec<Row> = store.with_conn(|c| {
        let mut out = Vec::new();
        let mut st = c.prepare("SELECT id, uid FROM hosts WHERE uid IS NOT NULL AND has_password = 1 ORDER BY id")?;
        for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))? {
            let (rid, uid) = r?;
            out.push(Row { key: crate::credentials::host_key(rid), uid: uid.try_into().map_err(|_| anyhow!("host uid"))?, key_path: None });
        }
        let mut st = c.prepare("SELECT id, uid, private_key FROM identities WHERE uid IS NOT NULL ORDER BY id")?;
        for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?, r.get::<_, Option<String>>(2)?)))? {
            let (rid, uid, key_path) = r?;
            out.push(Row { key: crate::credentials::identity_key(rid), uid: uid.try_into().map_err(|_| anyhow!("identity uid"))?, key_path });
        }
        Ok(out)
    })?;
    let mut report = MigrationReport::default();
    for row in rows {
        let password = local.get(&row.key).ok().flatten();
        let private_key = row.key_path.as_deref().filter(|p| !p.is_empty()).and_then(|p| fs::read_to_string(crate::ssh::expand_tilde(p)).ok()).filter(|t| crate::ssh::looks_like_private_key(t));
        if password.is_none() && private_key.is_none() {
            report.skipped.push(row.key);
            continue;
        }
        cred_put(store, id, &row.uid, &CredPayload { password, private_key })?;
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
