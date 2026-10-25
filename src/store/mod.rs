mod command_history;
mod group_defaults;
pub use command_history::MAX_HOST_HISTORY;
mod hosts;
mod identities;
mod log_bookmarks;
mod migrate;
mod snippets;
pub mod transfer;
mod tunnels;
mod types;
mod uid;
mod workspaces;
pub use group_defaults::{ResolvedConnection, DEFAULT_PORT};
pub use hosts::is_option_like;
pub(crate) use hosts::{
    create_group_on, create_host_on, delete_group_on, delete_host_on, reject_option_like,
    update_group_on, update_host_on,
};
pub(crate) use hosts::{load_group_by_id, load_host_by_id};
pub(crate) use identities::{create_identity_on, delete_identity_on, update_identity_on};
pub(crate) use migrate::now_ts;
pub use transfer::{
    apply_transfer_plan, build_transfer_plan, tunnel_token_matches, unique_dest_name, DestIndex,
    GroupTransferMode, PlannedItem, TransferMode, TransferPlan, TransferResult, TransferSelection,
    TransferSnapshot,
};
pub use types::{
    AuthEvent, DeleteHostOutcome, DeleteIdentityOutcome, HostGroup, HostGroupUpdate, HostSource,
    HostUpdate, Identity, IdentityUpdate, LogBookmark, ManagedHost, NewHost, NewHostGroup,
    NewIdentity, NewLogBookmark, NewSnippet, NewTunnel, Snippet, SnippetUpdate,
    SshConfigHostImport, Tunnel, TunnelType, UpsertSshConfigOutcome, Workspace,
};
pub(crate) use uid::{
    group_id_by_uid_on, group_uid_on, host_id_by_uid_on, host_uid_on, identity_id_by_uid_on,
    identity_uid_on,
};
pub use uid::{
    uid_from_hex, uid_hex, GroupUid, HostUid, IdentityUid, Uid, DEFAULT_IDENTITY_UID,
    FAVORITES_GROUP_UID,
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
    capture: std::sync::OnceLock<Box<dyn LocalChangeSink>>,
}

/// What a local write touched, for [`LocalChangeSink`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeScope {
    Host(i64),
    Group(i64),
    Identity(i64),
    /// The row is gone; its stable uid is all that is left.
    Deleted(crate::sync::oplog::tier::EntityKind, Uid),
    /// Something that can move many rows (a group delete, a bulk transfer).
    Everything,
}

/// Told, after every local write to hosts, groups and identities, which rows
/// changed (host sync turns that into ops). It is NOT told about writes that
/// come from applying a synced op: those use the connection-level helpers.
pub trait LocalChangeSink: Send + Sync {
    fn changed(&self, conn: &Connection, scope: ChangeScope) -> Result<()>;
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
            capture: std::sync::OnceLock::new(),
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
            capture: std::sync::OnceLock::new(),
        };
        store.seed_default_identity()?;
        Ok(store)
    }

    /// Install the sink once; a second call is ignored.
    pub fn set_local_change_sink(&self, sink: Box<dyn LocalChangeSink>) {
        let _ = self.capture.set(sink);
    }

    pub(crate) fn notify_change(&self, scope: ChangeScope) -> Result<()> {
        match self.capture.get() {
            Some(sink) => self.with_conn(|conn| sink.changed(conn, scope)),
            None => Ok(()),
        }
    }

    /// The stable ids of every replicated row now (what a bulk operation may delete).
    pub(crate) fn entity_uids(
        &self,
    ) -> Result<std::collections::BTreeSet<(crate::sync::oplog::tier::EntityKind, Uid)>> {
        if self.capture.get().is_none() {
            return Ok(Default::default());
        }
        use crate::sync::oplog::tier::EntityKind;
        self.with_conn(|conn| {
            let mut out = std::collections::BTreeSet::new();
            for (kind, sql) in [
                (
                    EntityKind::Host,
                    "SELECT uid FROM hosts WHERE source = 'launcher' AND uid IS NOT NULL",
                ),
                (
                    EntityKind::Group,
                    "SELECT uid FROM host_groups WHERE reserved = 0 AND uid IS NOT NULL",
                ),
                (
                    EntityKind::Identity,
                    "SELECT uid FROM identities WHERE uid IS NOT NULL",
                ),
            ] {
                let mut st = conn.prepare(sql)?;
                for raw in st.query_map([], |r| r.get::<_, Vec<u8>>(0))? {
                    if let Ok(uid) = Uid::try_from(raw?) {
                        out.insert((kind, uid));
                    }
                }
            }
            Ok(out)
        })
    }

    /// After a bulk operation: whatever vanished is a deletion, then everything is reconciled.
    pub(crate) fn reconcile_after(
        &self,
        before: std::collections::BTreeSet<(crate::sync::oplog::tier::EntityKind, Uid)>,
    ) -> Result<()> {
        if self.capture.get().is_none() {
            return Ok(());
        }
        let now = self.entity_uids()?;
        for (kind, uid) in before.difference(&now) {
            self.notify_change(ChangeScope::Deleted(*kind, *uid))?;
        }
        self.notify_change(ChangeScope::Everything)
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
