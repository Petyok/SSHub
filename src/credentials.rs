use anyhow::Result;
use std::collections::HashMap;
use std::fs;

const SERVICE: &str = "sshub";

pub trait PasswordStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>>;
    fn set(&self, key: &str, password: &str) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
    /// Fetch a host password named by a per-host `Attribute=value` spec
    /// (#142). A method so tests can answer without a session bus.
    fn lookup(&self, spec: &str) -> Result<String> {
        secret_service_lookup(spec)
    }
    /// Where this store lives, so another process (the askpass helper that
    /// ssh runs) can open the same one. `None` for a store that only exists
    /// inside this process.
    fn locator(&self) -> Option<StoreLocator> {
        None
    }
}

/// Enough to reopen a [`PasswordStore`] in another process: never a secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreLocator {
    Keyring {
        prefix: String,
    },
    File {
        path: std::path::PathBuf,
        prefix: String,
    },
}

impl StoreLocator {
    pub fn open(&self) -> Box<dyn PasswordStore> {
        match self {
            Self::Keyring { prefix } => {
                Box::new(NamespacedPasswordStore::new(Box::new(OsKeyring), prefix))
            }
            Self::File { path, prefix } => Box::new(NamespacedPasswordStore::new(
                Box::new(FilePasswordStore::new(path.clone())),
                prefix,
            )),
        }
    }

    fn prefixed(self, outer: &str) -> Self {
        match self {
            Self::Keyring { prefix } => Self::Keyring {
                prefix: format!("{outer}{prefix}"),
            },
            Self::File { path, prefix } => Self::File {
                path,
                prefix: format!("{outer}{prefix}"),
            },
        }
    }
}

/// The credential store of one profile: the OS keyring when usable, else the
/// profile's `credentials.json`, always under the profile's key prefix.
pub fn open_for_profile(paths: &crate::profile::ProfilePaths) -> Box<dyn PasswordStore> {
    let prefix = paths.credential_prefix();
    if check_keyring_available() {
        let _ = migrate_fallback_to_keyring(&paths.credentials_file());
        Box::new(NamespacedPasswordStore::new(Box::new(OsKeyring), prefix))
    } else {
        Box::new(NamespacedPasswordStore::new(
            Box::new(FilePasswordStore::new(paths.credentials_file())),
            prefix,
        ))
    }
}

/// Which credential-store entry holds a secret: a stored key (`host:3`,
/// `identity:7`) or a host's Secret Service lookup spec (#142).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SecretRef {
    Key(String),
    Lookup(String),
}

impl SecretRef {
    /// The secret now, `None` when the entry is missing or unreadable.
    pub fn read(&self, store: &dyn PasswordStore) -> Option<String> {
        match self {
            Self::Key(key) => store.get(key).ok().flatten(),
            Self::Lookup(spec) => store.lookup(spec).ok(),
        }
    }
}

/// Split a secret lookup spec into `(attribute, value)` at the first `=`.
// ponytail: one pair is the ceiling; `;`-separated pairs are the upgrade
// path if a single attribute ever stops being unique enough.
pub fn parse_secret_lookup(spec: &str) -> Result<(&str, &str)> {
    let Some((attr, value)) = spec.split_once('=') else {
        anyhow::bail!("secret lookup must be Attribute=value");
    };
    let (attr, value) = (attr.trim(), value.trim());
    if attr.is_empty() || value.is_empty() {
        anyhow::bail!("secret lookup must be Attribute=value");
    }
    Ok((attr, value))
}

/// Read the one Secret Service item whose attribute matches `spec` exactly
/// (KeePassXC exposes `Title`, `UserName`, `URL`, `Notes`, `Path`, `Uuid`).
#[cfg(target_os = "linux")]
pub fn secret_service_lookup(spec: &str) -> Result<String> {
    use dbus_secret_service::{EncryptionType, SecretService};
    let (attr, value) = parse_secret_lookup(spec)?;
    let ss = SecretService::connect(EncryptionType::Dh)?;
    let found = ss.search_items(HashMap::from([(attr, value)]))?;
    let mut items: Vec<_> = found.unlocked.into_iter().chain(found.locked).collect();
    match items.len() {
        0 => anyhow::bail!("no Secret Service entry has {attr}={value}"),
        1 => {}
        n => anyhow::bail!(
            "{n} Secret Service entries have {attr}={value}; use a unique attribute such as Uuid"
        ),
    }
    let item = items.remove(0);
    // The provider shows its own unlock/confirm prompt here.
    item.ensure_unlocked()?;
    Ok(String::from_utf8(item.get_secret()?)?)
}

#[cfg(not(target_os = "linux"))]
pub fn secret_service_lookup(_spec: &str) -> Result<String> {
    anyhow::bail!("secret lookup needs a Secret Service (Linux)")
}

/// Password store that prefixes every key with a namespace. Profiles use
/// `profile:<id>:` so identically named hosts in separate profiles never
/// share a keyring entry; compat mode uses an empty prefix (current keys).
pub struct NamespacedPasswordStore {
    inner: Box<dyn PasswordStore>,
    prefix: String,
}

impl NamespacedPasswordStore {
    pub fn new(inner: Box<dyn PasswordStore>, prefix: impl Into<String>) -> Self {
        Self {
            inner,
            prefix: prefix.into(),
        }
    }

    fn namespaced(&self, key: &str) -> String {
        format!("{}{}", self.prefix, key)
    }
}

impl PasswordStore for NamespacedPasswordStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        self.inner.get(&self.namespaced(key))
    }
    fn set(&self, key: &str, password: &str) -> Result<()> {
        self.inner.set(&self.namespaced(key), password)
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.inner.delete(&self.namespaced(key))
    }
    // The spec names a provider entry, not an SSHub key: no prefix.
    fn lookup(&self, spec: &str) -> Result<String> {
        self.inner.lookup(spec)
    }
    fn locator(&self) -> Option<StoreLocator> {
        self.inner.locator().map(|l| l.prefixed(&self.prefix))
    }
}

pub fn check_keyring_available() -> bool {
    store_available(&OsKeyring)
}

/// Whether `store` can actually hold a secret: write a unique probe entry,
/// read it back, and remove it. A read-only probe cannot see the failure that
/// matters here — with no session bus (`env -i`, no `DBUS_SESSION_BUS_ADDRESS`)
/// `get_password` fails with a dbus-autolaunch error no needle list matches,
/// so the old check reported `true`, the app picked the OS keyring, and every
/// remember-me write failed closed. Asking the store to do the real operation
/// sees it. The account is unique per call so concurrent startups never share it.
fn store_available(store: &dyn PasswordStore) -> bool {
    let account = format!(
        "sshub-probe-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    );
    if store.set(&account, "probe").is_err() {
        return false;
    }
    let ok = matches!(store.get(&account), Ok(Some(value)) if value == "probe");
    let _ = store.delete(&account);
    ok
}

pub fn migrate_fallback_to_keyring(path: &std::path::Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(path)?;
    let map: HashMap<String, String> = serde_json::from_str(&content)?;

    let keyring_store = OsKeyring;
    let mut failed = false;
    for (key, password) in &map {
        if let Err(e) = keyring_store.set(key, password) {
            eprintln!("Failed to migrate key {key} to keyring: {e}");
            failed = true;
        }
    }

    if !failed {
        let _ = fs::remove_file(path);
    }
    Ok(())
}

pub struct OsKeyring;

impl PasswordStore for OsKeyring {
    fn locator(&self) -> Option<StoreLocator> {
        Some(StoreLocator::Keyring {
            prefix: String::new(),
        })
    }
    fn get(&self, key: &str) -> Result<Option<String>> {
        let entry = keyring::Entry::new(SERVICE, key)?;
        match entry.get_password() {
            Ok(pw) => Ok(Some(pw)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(anyhow::anyhow!("keyring: {e}")),
        }
    }
    fn set(&self, key: &str, password: &str) -> Result<()> {
        let entry = keyring::Entry::new(SERVICE, key)?;
        entry.set_password(password)?;
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<()> {
        let entry = keyring::Entry::new(SERVICE, key)?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(anyhow::anyhow!("keyring: {e}")),
        }
    }
}

pub struct FilePasswordStore {
    path: std::path::PathBuf,
}

impl FilePasswordStore {
    pub fn new(path: std::path::PathBuf) -> Self {
        Self { path }
    }

    fn read_map(&self) -> Result<HashMap<String, String>> {
        if !self.path.exists() {
            return Ok(HashMap::new());
        }
        let content = fs::read_to_string(&self.path)
            .map_err(|e| anyhow::anyhow!("Failed to read credentials file: {e}"))?;
        let map = serde_json::from_str::<HashMap<String, String>>(&content)
            .map_err(|e| anyhow::anyhow!("Failed to parse credentials file: {e}"))?;
        Ok(map)
    }

    fn write_map(&self, map: &HashMap<String, String>) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
            crate::secure_fs::restrict_dir(parent);
        }
        let content = serde_json::to_string_pretty(map)?;
        let tmp = self.path.with_extension("json.tmp");
        if tmp.exists() {
            anyhow::ensure!(
                !std::fs::symlink_metadata(&tmp)?.file_type().is_symlink(),
                "refusing symlink credentials temporary file: {}",
                tmp.display()
            );
            fs::remove_file(&tmp)?;
        }

        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&tmp)
                .map_err(|e| {
                    anyhow::anyhow!("Failed to create secure temp credentials file: {e}")
                })?;
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
        }
        #[cfg(not(unix))]
        {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)?;
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
            crate::secure_fs::restrict_file(&tmp);
        }

        fs::rename(&tmp, &self.path)?;
        // Make the rename itself durable: without it a crash can leave the old
        // map after the caller was told the secret was saved.
        #[cfg(unix)]
        {
            if let Some(parent) = self.path.parent() {
                fs::File::open(parent)?.sync_all()?;
            }
        }
        Ok(())
    }

    /// Exclusive advisory lock around a read-modify-write. It sits on a sidecar
    /// file: `write_map` replaces the data file by rename, so a lock on the
    /// data file would guard an inode that is already gone. Two sshub
    /// processes (or an import next to the TUI) otherwise both read the old
    /// map and the later write drops the earlier one's secret. Released when
    /// the returned file is dropped.
    fn lock(&self) -> Result<fs::File> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
            crate::secure_fs::restrict_dir(parent);
        }
        let lock_path = self.path.with_extension("json.lock");
        let mut opts = fs::OpenOptions::new();
        opts.create(true).truncate(false).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let file = opts
            .open(&lock_path)
            .map_err(|e| anyhow::anyhow!("Failed to open credentials lock: {e}"))?;
        file.lock()
            .map_err(|e| anyhow::anyhow!("Failed to lock credentials file: {e}"))?;
        Ok(file)
    }
}

impl PasswordStore for FilePasswordStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        // Under the same lock as writers; an unlockable directory (read-only)
        // still reads, since the data file is replaced atomically by rename.
        let _lock = self.lock().ok();
        let map = self.read_map()?;
        Ok(map.get(key).cloned())
    }

    fn locator(&self) -> Option<StoreLocator> {
        Some(StoreLocator::File {
            path: self.path.clone(),
            prefix: String::new(),
        })
    }

    fn set(&self, key: &str, password: &str) -> Result<()> {
        let _lock = self.lock()?;
        let mut map = self.read_map()?;
        map.insert(key.to_string(), password.to_string());
        self.write_map(&map)
    }

    fn delete(&self, key: &str) -> Result<()> {
        let _lock = self.lock()?;
        let mut map = self.read_map()?;
        if map.remove(key).is_some() {
            self.write_map(&map)?;
        }
        Ok(())
    }
}

pub struct NoopPasswordStore;

impl PasswordStore for NoopPasswordStore {
    fn get(&self, _key: &str) -> Result<Option<String>> {
        Ok(None)
    }
    fn set(&self, _key: &str, _password: &str) -> Result<()> {
        Ok(())
    }
    fn delete(&self, _key: &str) -> Result<()> {
        Ok(())
    }
}

pub fn identity_key(id: i64) -> String {
    format!("identity:{id}")
}

pub fn host_key(id: i64) -> String {
    format!("host:{id}")
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Oracle: observed on a host with no session bus (`env -i`, no
    /// `DBUS_SESSION_BUS_ADDRESS`, probed 2026-09-18). There `OsKeyring::set`
    /// fails with "Platform secure storage failure: DBus error: Using X11 for
    /// dbus-daemon autolaunch was disabled at compile time, set your
    /// DBUS_SESSION_BUS_ADDRESS instead: ...", and `get` fails the same way —
    /// yet `check_keyring_available` still reported `true`, so the app picked
    /// the OS keyring and every remember-me write failed closed.
    struct UnreachableKeyring;

    impl PasswordStore for UnreachableKeyring {
        fn get(&self, _key: &str) -> Result<Option<String>> {
            Err(anyhow::anyhow!(
                "keyring: Platform secure storage failure: DBus error: Using X11 for dbus-daemon autolaunch was disabled at compile time, set your DBUS_SESSION_BUS_ADDRESS instead"
            ))
        }
        fn set(&self, _key: &str, _password: &str) -> Result<()> {
            Err(anyhow::anyhow!(
                "Platform secure storage failure: DBus error: Using X11 for dbus-daemon autolaunch was disabled at compile time, set your DBUS_SESSION_BUS_ADDRESS instead"
            ))
        }
        fn delete(&self, _key: &str) -> Result<()> {
            Err(anyhow::anyhow!(
                "Platform secure storage failure: DBus error: Using X11 for dbus-daemon autolaunch was disabled at compile time, set your DBUS_SESSION_BUS_ADDRESS instead"
            ))
        }
    }

    #[derive(Default)]
    struct WorkingStore {
        map: std::sync::Mutex<HashMap<String, String>>,
    }

    impl PasswordStore for WorkingStore {
        fn get(&self, key: &str) -> Result<Option<String>> {
            Ok(self.map.lock().unwrap().get(key).cloned())
        }
        fn set(&self, key: &str, password: &str) -> Result<()> {
            self.map
                .lock()
                .unwrap()
                .insert(key.to_string(), password.to_string());
            Ok(())
        }
        fn delete(&self, key: &str) -> Result<()> {
            self.map.lock().unwrap().remove(key);
            Ok(())
        }
    }

    #[test]
    fn probe_reports_unwritable_store_as_unavailable() {
        assert!(
            !store_available(&UnreachableKeyring),
            "env -i oracle: writes fail, so the store must not be selected"
        );
    }

    #[test]
    fn probe_reports_working_store_as_available() {
        assert!(store_available(&WorkingStore::default()));
    }

    #[test]
    fn test_file_password_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.json");
        let store = FilePasswordStore::new(path);

        let test_key = "test:host:123";
        let test_pw = "fallback_secret_pass";

        assert_eq!(store.get(test_key).unwrap(), None);

        store.set(test_key, test_pw).unwrap();

        assert_eq!(store.get(test_key).unwrap(), Some(test_pw.to_string()));

        store.delete(test_key).unwrap();

        assert_eq!(store.get(test_key).unwrap(), None);
    }

    /// Concurrent writers (two sshub processes, or the importer next to the TUI)
    /// must not lose each other's secret: each does read-modify-write of the
    /// whole map. Without the lock the later rename drops the earlier key.
    #[test]
    fn concurrent_sets_keep_every_secret() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.json");
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let path = path.clone();
                std::thread::spawn(move || {
                    // One store per thread: separate lock-file descriptors, as
                    // separate processes would have (flock is per open file).
                    let store = FilePasswordStore::new(path);
                    for i in 0..10 {
                        store.set(&format!("k{t}-{i}"), "pw").unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let store = FilePasswordStore::new(path);
        for t in 0..8 {
            for i in 0..10 {
                assert_eq!(
                    store.get(&format!("k{t}-{i}")).unwrap().as_deref(),
                    Some("pw"),
                    "k{t}-{i} lost"
                );
            }
        }
    }

    #[test]
    fn secret_lookup_spec_splits_at_the_first_equals() {
        assert_eq!(
            parse_secret_lookup("URL=ssh://a@b:22").unwrap(),
            ("URL", "ssh://a@b:22")
        );
        assert_eq!(
            parse_secret_lookup(" Notes = a=b ").unwrap(),
            ("Notes", "a=b")
        );
        for bad in ["URL", "=x", "URL=", " = "] {
            assert!(
                parse_secret_lookup(bad).is_err(),
                "{bad:?} must be rejected"
            );
        }
    }

    #[test]
    fn a_store_reopens_from_its_locator_with_the_same_namespace() {
        let dir = tempfile::tempdir().unwrap();
        let store = NamespacedPasswordStore::new(
            Box::new(FilePasswordStore::new(dir.path().join("c.json"))),
            "profile:4:",
        );
        store.set("host:1", "pw").unwrap();
        let locator = store.locator().unwrap();
        assert_eq!(
            locator,
            StoreLocator::File {
                path: dir.path().join("c.json"),
                prefix: "profile:4:".into()
            }
        );
        let reopened = locator.open();
        assert_eq!(
            SecretRef::Key("host:1".into()).read(reopened.as_ref()),
            Some("pw".into())
        );
        assert_eq!(
            SecretRef::Key("host:2".into()).read(reopened.as_ref()),
            None
        );
        assert!(NoopPasswordStore.locator().is_none());
    }
}
