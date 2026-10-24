//! Where a profile's device identity lives on disk (contract C8).
//!
//! `<profile root>/sync/device.key` is mode 0600 and holds
//! [`DeviceIdentity::to_encrypted`] output. The passphrase is 256 random bits
//! kept in the profile's credential store under [`KEY_NAME`]: the OS keyring
//! normally, so the file alone is useless; in the `credentials.json` fallback
//! the passphrase sits next to the file and the protection is the 0600 mode,
//! exactly as for every other SSHub secret there. Nothing here prompts, so a
//! headless `sync-serve` can load the key.

use anyhow::{bail, Context, Result};
use rand_core::{OsRng, RngCore};
use std::io::Write;
use std::path::PathBuf;

use crate::credentials::PasswordStore;
use crate::profile::ProfilePaths;
use crate::sync::crypto::DeviceIdentity;

/// Credential-store entry holding the file passphrase (the store adds the
/// profile prefix).
pub const KEY_NAME: &str = "sync-device-key";

pub fn key_path(paths: &ProfilePaths) -> PathBuf {
    paths.root.join("sync").join("device.key")
}

/// Load this profile's device identity, creating it on first use.
///
/// A key file whose passphrase is missing is an error, never a silent
/// regeneration: a new identity would orphan every op the old one signed.
pub fn load_or_init_identity(
    paths: &ProfilePaths,
    store: &dyn PasswordStore,
) -> Result<DeviceIdentity> {
    let path = key_path(paths);
    let passphrase = store
        .get(KEY_NAME)
        .context("read the device key passphrase")?;
    match (path.exists(), passphrase) {
        (true, Some(pass)) => {
            let blob = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
            DeviceIdentity::from_encrypted(&blob, &pass)
                .with_context(|| format!("decrypt {}", path.display()))
        }
        (true, None) => bail!(
            "{} exists but its passphrase is missing from the credential store; \
             restore the credential store entry '{KEY_NAME}' or remove the file to enrol again",
            path.display()
        ),
        (false, _) => {
            let id = DeviceIdentity::generate();
            let mut raw = [0u8; 32];
            OsRng.fill_bytes(&mut raw);
            let pass: String = raw.iter().map(|b| format!("{b:02x}")).collect();
            // Passphrase first: a key file without it could never be opened.
            store
                .set(KEY_NAME, &pass)
                .context("store the device key passphrase")?;
            write_private(&path, &id.to_encrypted(&pass))?;
            Ok(id)
        }
    }
}

fn write_private(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().context("device key path has no parent")?;
    std::fs::create_dir_all(dir)?;
    crate::secure_fs::restrict_dir(dir);
    let tmp = path.with_extension("key.tmp");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts
        .open(&tmp)
        .with_context(|| format!("create {}", tmp.display()))?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    crate::secure_fs::restrict_file(&tmp);
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::MemoryPasswordStore;

    fn paths(root: &std::path::Path) -> ProfilePaths {
        ProfilePaths {
            data_root: root.to_path_buf(),
            id: "p1".into(),
            name: "test".into(),
            root: root.to_path_buf(),
            config_file: root.join("config.toml"),
            ssh_config: root.join("ssh_config"),
            compat: false,
        }
    }

    #[test]
    fn first_load_creates_a_private_encrypted_file_and_later_loads_return_the_same_key() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(dir.path());
        let store = MemoryPasswordStore::default();
        let a = load_or_init_identity(&p, &store).unwrap();
        let file = key_path(&p);
        let blob = std::fs::read(&file).unwrap();
        assert_eq!(&blob[..4], b"SSHK", "the file is the encrypted key format");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode =
                |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&file), 0o600);
            assert_eq!(mode(file.parent().unwrap()), 0o700);
        }
        assert!(!file.with_extension("key.tmp").exists());
        let b = load_or_init_identity(&p, &store).unwrap();
        assert_eq!(a.public(), b.public());
        // Stable id, so the device keeps its ops across restarts.
        assert_eq!(a.public().device_id(), b.public().device_id());
    }

    #[test]
    fn the_file_alone_is_useless_and_a_missing_passphrase_never_regenerates() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(dir.path());
        let store = MemoryPasswordStore::default();
        let a = load_or_init_identity(&p, &store).unwrap();
        // Another credential store (another machine, wiped keyring): refuse.
        let other = MemoryPasswordStore::default();
        let err = load_or_init_identity(&p, &other).unwrap_err().to_string();
        assert!(err.contains("passphrase is missing"), "{err}");
        assert!(key_path(&p).exists(), "the file is left alone");
        // A wrong passphrase is an error too.
        other.set(KEY_NAME, "00").unwrap();
        assert!(load_or_init_identity(&p, &other).is_err());
        // The right store still works.
        assert_eq!(
            load_or_init_identity(&p, &store).unwrap().public(),
            a.public()
        );
    }

    #[test]
    fn identities_are_per_profile() {
        let d1 = tempfile::tempdir().unwrap();
        let d2 = tempfile::tempdir().unwrap();
        let s1 = MemoryPasswordStore::default();
        let s2 = MemoryPasswordStore::default();
        let a = load_or_init_identity(&paths(d1.path()), &s1).unwrap();
        let b = load_or_init_identity(&paths(d2.path()), &s2).unwrap();
        assert_ne!(a.public().device_id(), b.public().device_id());
    }

    #[test]
    fn a_stale_passphrase_without_a_file_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(dir.path());
        let store = MemoryPasswordStore::default();
        store.set(KEY_NAME, "stale").unwrap();
        let id = load_or_init_identity(&p, &store).unwrap();
        assert_ne!(store.get(KEY_NAME).unwrap().unwrap(), "stale");
        assert_eq!(
            load_or_init_identity(&p, &store).unwrap().public(),
            id.public()
        );
    }
}
