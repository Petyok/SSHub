use anyhow::Result;
use std::collections::HashMap;
use std::fs;

const SERVICE: &str = "sshub";

pub trait PasswordStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>>;
    fn set(&self, key: &str, password: &str) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
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
        }
        #[cfg(not(unix))]
        {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)?;
            file.write_all(content.as_bytes())?;
            crate::secure_fs::restrict_file(&tmp);
        }

        fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

impl PasswordStore for FilePasswordStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        let map = self.read_map()?;
        Ok(map.get(key).cloned())
    }

    fn set(&self, key: &str, password: &str) -> Result<()> {
        let mut map = self.read_map()?;
        map.insert(key.to_string(), password.to_string());
        self.write_map(&map)
    }

    fn delete(&self, key: &str) -> Result<()> {
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
}
