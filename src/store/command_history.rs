//! Local-only history. Never include this table in host export/sync payloads.
use super::LauncherStore;
use crate::command_safety::{classify_command, CommandSafety};
use crate::session::history::HistoryEntry;
use anyhow::Result;
use rusqlite::params;

pub const MAX_HOST_HISTORY: usize = 2000;
impl LauncherStore {
    pub fn record_command(
        &self,
        host_id: Option<i64>,
        command: &str,
        limit: usize,
    ) -> Result<bool> {
        let Some(host_id) = host_id else {
            return Ok(false);
        };
        if limit == 0 || classify_command(command) != CommandSafety::Safe {
            return Ok(false);
        }
        let command = command.trim();
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM hosts WHERE id=?1)", [host_id], |r| r.get(0))?;
            if !exists { return Ok(false); }
            let now = super::migrate::now_ts();
            // Move reused rows to the newest id so second-resolution timestamps
            // and wall-clock changes cannot evict the command just submitted.
            tx.execute("INSERT INTO command_history(host_id,command,last_used,use_count,created_at)
                VALUES(?1,?2,?3,1,?3) ON CONFLICT(host_id,command) DO UPDATE SET
                id=(SELECT MAX(id)+1 FROM command_history),
                last_used=excluded.last_used,use_count=MIN(command_history.use_count+1,9223372036854775807)", params![host_id, command, now])?;
            tx.execute("DELETE FROM command_history WHERE host_id=?1 AND id NOT IN
                (SELECT id FROM command_history WHERE host_id=?1 ORDER BY id DESC LIMIT ?2)", params![host_id, limit.min(MAX_HOST_HISTORY) as i64])?;
            tx.commit()?;
            Ok(true)
        })
    }
    pub fn command_history(&self, host_id: i64, limit: usize) -> Result<Vec<HistoryEntry>> {
        self.with_conn(|conn| {
            let mut statement = conn.prepare(
                "SELECT command,last_used,use_count FROM command_history
                WHERE host_id=?1 ORDER BY id DESC LIMIT ?2",
            )?;
            let rows = statement.query_map(
                params![host_id, limit.min(MAX_HOST_HISTORY) as i64],
                |row| {
                    Ok(HistoryEntry {
                        command: row.get(0)?,
                        last_used: row.get(1)?,
                        use_count: row.get(2)?,
                    })
                },
            )?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(Into::into)
        })
    }
    pub fn clear_command_history(&self, host_id: Option<i64>) -> Result<()> {
        self.with_conn(|conn| {
            match host_id {
                Some(id) => {
                    conn.execute("DELETE FROM command_history WHERE host_id=?1", [id])?;
                }
                None => {
                    conn.execute("DELETE FROM command_history", [])?;
                }
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn host(store: &LauncherStore, name: &str) -> i64 {
        store
            .with_conn(|conn| {
                conn.execute(
                    "INSERT INTO hosts(name,address,created_at,updated_at) VALUES(?1,?1,0,0)",
                    [name],
                )?;
                Ok(conn.last_insert_rowid())
            })
            .unwrap()
    }
    #[test]
    fn migrated_history_is_bounded_isolated_and_cascades() {
        let store = LauncherStore::open_in_memory().unwrap();
        let first = host(&store, "first");
        let second = host(&store, "second");
        assert!(!store.record_command(None, "ls", 2).unwrap());
        assert!(!store
            .record_command(Some(first), "mysql -pSECRET", 2)
            .unwrap());
        assert!(!store
            .record_command(Some(first), &"x".repeat(4097), 2)
            .unwrap());
        store.record_command(Some(first), "ls", 2).unwrap();
        store.record_command(Some(first), "ls", 2).unwrap();
        assert_eq!(store.command_history(first, 2).unwrap()[0].use_count, 2);
        store.record_command(Some(second), "df", 2).unwrap();
        store.record_command(Some(first), "pwd", 2).unwrap();
        store.record_command(Some(first), "whoami", 2).unwrap();
        assert_eq!(store.command_history(first, 200).unwrap().len(), 2);
        assert_eq!(store.command_history(second, 200).unwrap()[0].command, "df");
        store
            .with_conn(|conn| {
                conn.execute("DELETE FROM hosts WHERE id=?1", [first])?;
                Ok(())
            })
            .unwrap();
        assert!(store.command_history(first, 200).unwrap().is_empty());
        store.clear_command_history(None).unwrap();
        assert!(store.command_history(second, 200).unwrap().is_empty());
    }
    #[test]
    fn recently_reused_command_survives_same_second_pruning() {
        let store = LauncherStore::open_in_memory().unwrap();
        let id = host(&store, "fixture");
        store.record_command(Some(id), "ls", 2).unwrap();
        store.record_command(Some(id), "pwd", 2).unwrap();
        store
            .with_conn(|conn| {
                conn.execute(
                    "UPDATE command_history SET last_used=?1",
                    [super::super::migrate::now_ts() + 3600],
                )?;
                Ok(())
            })
            .unwrap();
        store.record_command(Some(id), "ls", 2).unwrap();
        store.record_command(Some(id), "whoami", 2).unwrap();
        let entries = store.command_history(id, 2).unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|e| e.command.as_str())
                .collect::<Vec<_>>(),
            ["whoami", "ls"]
        );
        assert_eq!(entries[1].use_count, 2);
        assert!(!store.record_command(Some(id), "ls\n", 2).unwrap());
    }
    #[cfg(unix)]
    #[test]
    fn history_database_remains_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("launcher.db");
        let _store = LauncherStore::open(&path).unwrap();
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
