//! Crash-safe migration of a legacy top-level install into `profiles/default`.
//!
//! The legacy layout kept `launcher.db`, `metadata.db`, `credentials.json`,
//! `logs/`, and `tunnels/` directly in the data directory, with `config.toml`
//! in the config directory. Migration **copies** (never moves) those files
//! into a staging directory, validates the copy, renames the staging dir into
//! `profiles/default`, and only then writes `state.toml`. Legacy files stay
//! intact so an interrupted migration retries without data loss and a
//! downgrade to an older sshub still boots.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::{new_profile_id, ProfileRecord, ProfileState, RootDirs, DEFAULT_PROFILE_NAME};

const LOCK_FILE: &str = ".migration.lock";
const STAGING_DIR: &str = ".staging-default";
const PROFILE_ID_FILE: &str = ".profile-id";
const LEGACY_DATA_FILES: [&str; 3] = ["launcher.db", "metadata.db", "credentials.json"];
const LEGACY_DATA_DIRS: [&str; 2] = ["logs", "tunnels"];
/// SQLite sidecars that must travel with their database.
const DB_SIDECARS: [&str; 3] = ["-wal", "-shm", "-journal"];

/// Record the stable profile id inside the profile directory so adoption of
/// orphaned directories (crash between rename and state write) keeps the id.
pub fn write_profile_id(dir: &Path, id: &str) -> Result<()> {
    let path = dir.join(PROFILE_ID_FILE);
    std::fs::write(&path, id)?;
    crate::secure_fs::restrict_file(&path);
    Ok(())
}

pub fn read_profile_id(dir: &Path) -> Option<String> {
    std::fs::read_to_string(dir.join(PROFILE_ID_FILE))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// True when the legacy top-level layout has anything worth migrating.
pub fn legacy_installation_present(roots: &RootDirs) -> bool {
    let data = &roots.data_root;
    for file in LEGACY_DATA_FILES {
        if data.join(file).exists() {
            return true;
        }
    }
    for dir in LEGACY_DATA_DIRS {
        if data.join(dir).is_dir() {
            return true;
        }
    }
    roots.config_root.join("config.toml").exists()
}

/// Run the full migration. Requires `state.toml` to be absent (callers check).
pub fn run_legacy_migration(roots: &RootDirs) -> Result<()> {
    let store = crate::credentials::OsKeyring;
    run_legacy_migration_with_store(roots, &store)
}

pub fn run_legacy_migration_with_store(
    roots: &RootDirs,
    credential_store: &dyn crate::credentials::PasswordStore,
) -> Result<()> {
    let _lock = MigrationLock::acquire(roots)?;

    // Another process may have finished the migration while we waited.
    if ProfileState::load(&roots.data_root)?.is_some() {
        return Ok(());
    }

    let profiles_dir = super::ProfilePaths::profiles_dir(&roots.data_root);
    std::fs::create_dir_all(&profiles_dir)?;
    crate::secure_fs::restrict_dir(&profiles_dir);

    let final_dir = profiles_dir.join(DEFAULT_PROFILE_NAME);
    if final_dir.exists() {
        // Profile directory already present without state.toml — adoption
        // handles it; do not clobber.
        return Ok(());
    }

    let staging = profiles_dir.join(STAGING_DIR);
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)
        .with_context(|| format!("create staging dir {}", staging.display()))?;
    crate::secure_fs::restrict_dir(&staging);

    let copied = copy_legacy_files(roots, &staging)?;
    validate_copy(roots, &staging, &copied)?;

    let profile_id = new_profile_id();
    write_profile_id(&staging, &profile_id)?;
    rekey_credentials(&staging, &profile_id, credential_store);

    std::fs::rename(&staging, &final_dir)
        .with_context(|| format!("rename {} -> {}", staging.display(), final_dir.display()))?;
    crate::secure_fs::restrict_dir(&final_dir);

    // State is written only after the profile directory is complete.
    let state = ProfileState {
        profiles: vec![ProfileRecord {
            id: profile_id,
            name: DEFAULT_PROFILE_NAME.to_string(),
        }],
        last_used: None,
    };
    let mut state = state;
    state.last_used = state.profiles.first().map(|p| p.id.clone());
    state.save(&roots.data_root)?;
    Ok(())
}

/// Copy whatever legacy files exist into staging. Returns the relative file
/// paths copied (for validation). A config-only or database-only install is
/// fine — each source is copied if present.
fn copy_legacy_files(roots: &RootDirs, staging: &Path) -> Result<Vec<PathBuf>> {
    let mut copied = Vec::new();
    let data = &roots.data_root;

    for file in LEGACY_DATA_FILES {
        let src = data.join(file);
        if !src.exists() {
            continue;
        }
        ensure_regular_file(&src)?;
        std::fs::copy(&src, staging.join(file))
            .with_context(|| format!("copy {} to staging", src.display()))?;
        crate::secure_fs::restrict_file(&staging.join(file));
        copied.push(PathBuf::from(file));
        if file.ends_with(".db") {
            for suffix in DB_SIDECARS {
                let sidecar = data.join(format!("{file}{suffix}"));
                if sidecar.exists() {
                    ensure_regular_file(&sidecar)?;
                    std::fs::copy(&sidecar, staging.join(format!("{file}{suffix}")))
                        .with_context(|| format!("copy {}", sidecar.display()))?;
                    crate::secure_fs::restrict_file(&staging.join(format!("{file}{suffix}")));
                    copied.push(PathBuf::from(format!("{file}{suffix}")));
                }
            }
        }
    }

    for dir in LEGACY_DATA_DIRS {
        let src = data.join(dir);
        if !src.is_dir() {
            continue;
        }
        ensure_directory(&src)?;
        copy_dir_recursive(&src, &staging.join(dir))
            .with_context(|| format!("copy {} to staging", src.display()))?;
        copied.push(PathBuf::from(dir));
    }

    let legacy_config = roots.config_root.join("config.toml");
    if legacy_config.exists() {
        ensure_regular_file(&legacy_config)?;
        std::fs::copy(&legacy_config, staging.join("config.toml"))
            .with_context(|| format!("copy {}", legacy_config.display()))?;
        crate::secure_fs::restrict_file(&staging.join("config.toml"));
        copied.push(PathBuf::from("config.toml"));
    }

    Ok(copied)
}

fn ensure_regular_file(path: &Path) -> Result<()> {
    let metadata =
        std::fs::symlink_metadata(path).with_context(|| format!("stat {}", path.display()))?;
    anyhow::ensure!(
        metadata.file_type().is_file(),
        "refusing non-regular migration source: {}",
        path.display()
    );
    Ok(())
}

fn ensure_directory(path: &Path) -> Result<()> {
    let metadata =
        std::fs::symlink_metadata(path).with_context(|| format!("stat {}", path.display()))?;
    anyhow::ensure!(
        metadata.file_type().is_dir(),
        "refusing non-directory migration source: {}",
        path.display()
    );
    Ok(())
}

/// Compare sizes of every copied file/directory root so a truncated copy
/// never becomes the live profile.
fn validate_copy(roots: &RootDirs, staging: &Path, copied: &[PathBuf]) -> Result<()> {
    for rel in copied {
        let src = if rel == Path::new("config.toml") {
            roots.config_root.join(rel)
        } else {
            roots.data_root.join(rel)
        };
        let dst = staging.join(rel);
        let src_meta =
            std::fs::metadata(&src).with_context(|| format!("stat {}", src.display()))?;
        let dst_meta =
            std::fs::metadata(&dst).with_context(|| format!("stat {}", dst.display()))?;
        if src_meta.is_file() {
            anyhow::ensure!(
                src_meta.len() == dst_meta.len(),
                "migration copy verification failed for {}",
                rel.display()
            );
        } else {
            anyhow::ensure!(
                dst_meta.is_dir(),
                "migration copy verification failed for {}",
                rel.display()
            );
        }
    }
    Ok(())
}

/// Namespace the migrated credentials to the new default profile id.
///
/// The fallback `credentials.json` is rewritten in place (inside staging);
/// keyring entries are re-keyed from `host:{id}` to `profile:{id}:host:{id}`
/// using the ids recorded in the migrated launcher database. Both directions
/// are best-effort: a keyring failure must not abort the migration.
fn rekey_credentials(
    staging: &Path,
    profile_id: &str,
    credential_store: &dyn crate::credentials::PasswordStore,
) {
    let prefix = format!("profile:{profile_id}:");

    let file_path = staging.join("credentials.json");
    if file_path.exists() {
        match std::fs::read_to_string(&file_path)
            .ok()
            .and_then(|content| serde_json::from_str::<HashMap<String, String>>(&content).ok())
        {
            Some(map) => {
                let rekeyed: HashMap<String, String> = map
                    .into_iter()
                    .map(|(key, value)| {
                        if key.starts_with(&prefix) {
                            (key, value)
                        } else {
                            (format!("{prefix}{key}"), value)
                        }
                    })
                    .collect();
                if let Ok(content) = serde_json::to_string_pretty(&rekeyed) {
                    let _ = std::fs::write(&file_path, content);
                }
            }
            None => {
                eprintln!(
                    "warning: could not re-key {}; passwords may need re-entry",
                    file_path.display()
                );
            }
        }
    }

    // Keyring: only keys whose ids the migrated database knows about.
    let db_path = staging.join("launcher.db");
    if !db_path.exists() {
        return;
    }
    let conn = match rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) {
        Ok(conn) => conn,
        Err(_) => return,
    };
    let mut keys = Vec::new();
    for table in ["hosts", "identities"] {
        let kind = if table == "hosts" { "host" } else { "identity" };
        if let Ok(mut stmt) = conn.prepare(&format!("SELECT id FROM {table}")) {
            if let Ok(rows) = stmt.query_map([], |row| row.get::<_, i64>(0)) {
                for id in rows.flatten() {
                    keys.push((format!("{kind}:{id}"), format!("{prefix}{kind}:{id}")));
                }
            }
        }
    }
    for (old_key, new_key) in keys {
        match credential_store.get(&old_key) {
            Ok(Some(secret)) => {
                // Keep legacy keys until committed state exists. If migration
                // is interrupted and retried with a new profile ID, secrets
                // remain recoverable under their original keys.
                let _ = credential_store.set(&new_key, &secret);
            }
            Ok(None) => {}
            Err(_) => {}
        }
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dest_path = dst.join(entry.file_name());
        if file_type.is_symlink() {
            anyhow::bail!(
                "refusing symlink during migration: {}",
                entry.path().display()
            );
        }
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &dest_path)?;
        } else {
            std::fs::copy(entry.path(), &dest_path)?;
        }
    }
    Ok(())
}

/// Exclusive migration lock so two concurrent launches cannot race on the
/// staging directory. Stale locks (crash mid-migration) are broken by age:
/// the lock file records its creation time and is taken over after 10 minutes.
struct MigrationLock {
    path: PathBuf,
}

impl MigrationLock {
    fn acquire(roots: &RootDirs) -> Result<Self> {
        use std::io::Write;
        let path = roots.data_root.join(LOCK_FILE);
        let max_age = std::time::Duration::from_secs(600);
        for attempt in 0..50 {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    let _ = writeln!(file, "{}", std::process::id());
                    crate::secure_fs::restrict_file(&path);
                    return Ok(Self { path });
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    if lock_is_stale(&path, max_age) && !lock_owner_alive(&path) {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    if attempt == 49 {
                        anyhow::bail!(
                            "profile migration already in progress (lock: {})",
                            path.display()
                        );
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                Err(err) => {
                    return Err(err).with_context(|| format!("create {}", path.display()));
                }
            }
        }
        anyhow::bail!("profile migration lock busy: {}", path.display())
    }
}

fn lock_is_stale(path: &Path, max_age: std::time::Duration) -> bool {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| std::time::SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age > max_age)
}

fn lock_owner_alive(path: &Path) -> bool {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(pid) = raw.trim().parse::<u32>() else {
        return false;
    };
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

impl Drop for MigrationLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct FakeStore {
        values: Mutex<HashMap<String, String>>,
        deleted: Mutex<Vec<String>>,
    }

    impl FakeStore {
        fn new(values: &[(&str, &str)]) -> Self {
            Self {
                values: Mutex::new(
                    values
                        .iter()
                        .map(|(key, value)| ((*key).into(), (*value).into()))
                        .collect(),
                ),
                deleted: Mutex::new(Vec::new()),
            }
        }
    }

    impl crate::credentials::PasswordStore for FakeStore {
        fn get(&self, key: &str) -> Result<Option<String>> {
            Ok(self.values.lock().unwrap().get(key).cloned())
        }

        fn set(&self, key: &str, password: &str) -> Result<()> {
            self.values
                .lock()
                .unwrap()
                .insert(key.into(), password.into());
            Ok(())
        }

        fn delete(&self, key: &str) -> Result<()> {
            self.deleted.lock().unwrap().push(key.into());
            self.values.lock().unwrap().remove(key);
            Ok(())
        }
    }

    #[test]
    fn rekey_keeps_legacy_key_for_retry() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("credentials.json"), r#"{"host:1":"secret"}"#).unwrap();
        let conn = rusqlite::Connection::open(staging.join("launcher.db")).unwrap();
        conn.execute("CREATE TABLE hosts (id INTEGER)", []).unwrap();
        conn.execute("CREATE TABLE identities (id INTEGER)", [])
            .unwrap();
        conn.execute("INSERT INTO hosts (id) VALUES (1)", [])
            .unwrap();
        let store = FakeStore::new(&[("host:1", "secret")]);

        rekey_credentials(&staging, "profile-id", &store);

        assert!(store.deleted.lock().unwrap().is_empty());
        assert_eq!(
            store
                .values
                .lock()
                .unwrap()
                .get("profile:profile-id:host:1"),
            Some(&"secret".to_string())
        );
    }

    #[cfg(unix)]
    #[test]
    fn recursive_copy_rejects_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        let dst = dir.path().join("dst");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(dir.path().join("secret"), "secret").unwrap();
        std::os::unix::fs::symlink(dir.path().join("secret"), src.join("link")).unwrap();

        assert!(copy_dir_recursive(&src, &dst).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn live_old_lock_is_not_stale() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lock");
        std::fs::write(&path, std::process::id().to_string()).unwrap();
        assert!(lock_owner_alive(&path));
    }
}
