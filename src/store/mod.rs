mod hosts;
mod identities;
mod migrate;
mod tunnels;
mod types;

pub use types::{
    AuthEvent, DeleteHostOutcome, DeleteIdentityOutcome, HostGroup, HostGroupUpdate, HostSource,
    HostUpdate, Identity, IdentityUpdate, ManagedHost, NewHost, NewHostGroup, NewIdentity,
    NewTunnel, SshConfigHostImport, Tunnel, TunnelType, UpsertSshConfigOutcome,
};

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

/// SQLite-backed launcher store (hosts, groups, identities).
///
/// Replaces [`crate::metadata::MetadataDb`] for new R1 code paths. MVP still uses
/// `MetadataDb` until App wiring lands in later phases.
pub struct LauncherStore {
    conn: Mutex<Connection>,
}

impl LauncherStore {
    /// Open (or create) launcher database at `path`, run migrations, seed Default identity.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create launcher db directory {}", parent.display()))?;
            // The DB holds host addresses and the auth log; keep the directory
            // (and any SQLite -wal/-shm sidecars) owner-only.
            crate::secure_fs::restrict_dir(parent);
        }

        let conn = Connection::open(path)
            .with_context(|| format!("open launcher db at {}", path.display()))?;
        crate::secure_fs::restrict_file(path);
        migrate::run_migrations(&conn, path)?;

        let store = Self {
            conn: Mutex::new(conn),
        };
        store.seed_default_identity()?;
        Ok(store)
    }

    /// Open `launcher.db` under [`crate::config::data_dir`].
    pub fn open_default() -> Result<Self> {
        let data_dir = crate::config::data_dir()?;
        std::fs::create_dir_all(&data_dir)?;
        Self::open(data_dir.join("launcher.db"))
    }

    /// In-memory store for unit tests.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        // Point the "launcher path" into a throwaway location that cannot have
        // a metadata.db sibling: with ":memory:" the legacy-import step would
        // look for ./metadata.db in the CWD and silently import it in tests.
        let path = std::env::temp_dir().join("sshub-in-memory/launcher.db");
        migrate::run_migrations(&conn, &path)?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.seed_default_identity()?;
        Ok(store)
    }

    pub(crate) fn with_conn<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T>,
    {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("launcher store connection poisoned"))?;
        f(&conn)
    }

    /// Read a UI-state value by key (see [`Self::set_ui_state`]).
    pub fn get_ui_state(&self, key: &str) -> Result<Option<String>> {
        use rusqlite::OptionalExtension;
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT value FROM ui_state WHERE key = ?1",
                rusqlite::params![key],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
        })
    }

    /// Persist a small UI-state value (e.g. collapsed-group set as JSON).
    pub fn set_ui_state(&self, key: &str, value: &str) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO ui_state (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                rusqlite::params![key, value],
            )?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_default_uses_data_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("SSH_LAUNCHER_DATA_DIR", dir.path());

        let store = LauncherStore::open_default().unwrap();
        assert!(store.get_identity_by_name("Default").unwrap().is_some());

        assert!(dir.path().join("launcher.db").exists());
        std::env::remove_var("SSH_LAUNCHER_DATA_DIR");
    }
}
