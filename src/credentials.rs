use anyhow::Result;
use std::collections::HashMap;
use std::fs;

const SERVICE: &str = "sshub";

pub trait PasswordStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>>;
    fn set(&self, key: &str, password: &str) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
}

pub fn fallback_file_path() -> Result<std::path::PathBuf> {
    let dir = crate::config::data_dir()?;
    Ok(dir.join("credentials.json"))
}

pub fn check_keyring_available() -> bool {
    let entry = match keyring::Entry::new(SERVICE, "sshub-probe-availability") {
        Ok(entry) => entry,
        Err(_) => return false,
    };
    match entry.get_password() {
        Ok(_) => true,
        Err(keyring::Error::NoEntry) => true,
        Err(keyring::Error::PlatformFailure(e)) => {
            let err_str = e.to_string();
            !(err_str.contains("org.freedesktop.secrets")
                || err_str.contains("ServiceUnknown")
                || err_str.contains("not provided by any .service files"))
        }
        Err(_) => false,
    }
}

pub fn migrate_fallback_to_keyring() -> Result<()> {
    let path = fallback_file_path()?;
    if !path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(&path)?;
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
        let _ = fs::remove_file(&path);
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

        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)
                .map_err(|e| anyhow::anyhow!("Failed to create secure temp credentials file: {e}"))?;
            file.write_all(content.as_bytes())?;
        }
        #[cfg(not(unix))]
        {
            fs::write(&tmp, &content)?;
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
