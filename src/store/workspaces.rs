//! Saved split-view layouts (#73). The layout is opaque JSON here; the app
//! owns its shape (`crate::app::panes::PaneTree` of saved panes).

use anyhow::Result;
use rusqlite::params;

use super::migrate::now_ts;
use super::types::Workspace;
use super::LauncherStore;

impl LauncherStore {
    /// Every saved workspace, by name.
    pub fn list_workspaces(&self) -> Result<Vec<Workspace>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, layout, updated_at FROM workspaces
                 ORDER BY name COLLATE NOCASE, id",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(Workspace {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    layout: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(Into::into)
        })
    }

    /// Save `layout` under `name`, replacing a workspace of the same name.
    pub fn save_workspace(&self, name: &str, layout: &str) -> Result<()> {
        let now = now_ts();
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO workspaces (name, layout, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?3)
                 ON CONFLICT(name) DO UPDATE
                 SET layout = excluded.layout, updated_at = excluded.updated_at",
                params![name, layout, now],
            )?;
            Ok(())
        })
    }

    pub fn delete_workspace(&self, id: i64) -> Result<bool> {
        self.with_conn(|conn| {
            let affected = conn.execute("DELETE FROM workspaces WHERE id = ?1", params![id])?;
            Ok(affected > 0)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saving_under_an_existing_name_replaces_it() {
        let store = LauncherStore::open_in_memory().unwrap();
        store.save_workspace("ops", "{\"leaf\":\"local\"}").unwrap();
        store.save_workspace("db", "{\"leaf\":\"empty\"}").unwrap();
        store.save_workspace("ops", "{\"leaf\":\"empty\"}").unwrap();
        let all = store.list_workspaces().unwrap();
        let names: Vec<&str> = all.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, ["db", "ops"]);
        assert_eq!(all[1].layout, "{\"leaf\":\"empty\"}");
        assert!(store.delete_workspace(all[1].id).unwrap());
        assert!(!store.delete_workspace(all[1].id).unwrap());
        assert_eq!(store.list_workspaces().unwrap().len(), 1);
    }
}
