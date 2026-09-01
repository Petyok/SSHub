use anyhow::Result;
use rusqlite::params;

use super::migrate::now_ts;
use super::types::{LogBookmark, NewLogBookmark};
use super::LauncherStore;

impl LauncherStore {
    pub fn create_log_bookmark(&self, bookmark: &NewLogBookmark) -> Result<i64> {
        let now = now_ts();
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO log_bookmarks (host_dir, file_name, line, name, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    bookmark.host_dir,
                    bookmark.file_name,
                    bookmark.line,
                    bookmark.name,
                    now,
                ],
            )?;
            Ok(conn.last_insert_rowid())
        })
    }

    pub fn list_log_bookmarks(&self) -> Result<Vec<LogBookmark>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, host_dir, file_name, line, name, created_at
                 FROM log_bookmarks ORDER BY created_at DESC, id DESC",
            )?;
            let rows = stmt.query_map([], row_to_bookmark)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(Into::into)
        })
    }

    pub fn delete_log_bookmark(&self, id: i64) -> Result<bool> {
        self.with_conn(|conn| {
            let affected = conn.execute("DELETE FROM log_bookmarks WHERE id = ?1", params![id])?;
            Ok(affected > 0)
        })
    }
}

fn row_to_bookmark(row: &rusqlite::Row<'_>) -> rusqlite::Result<LogBookmark> {
    Ok(LogBookmark {
        id: row.get(0)?,
        host_dir: row.get(1)?,
        file_name: row.get(2)?,
        line: row.get(3)?,
        name: row.get(4)?,
        created_at: row.get(5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_bookmark_crud() {
        let store = LauncherStore::open_in_memory().unwrap();
        let id = store
            .create_log_bookmark(&NewLogBookmark {
                host_dir: "web-1".into(),
                file_name: "1700000000-1-0.log".into(),
                line: 42,
                name: "deploy start".into(),
            })
            .unwrap();

        let all = store.list_log_bookmarks().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, id);
        assert_eq!(all[0].host_dir, "web-1");
        assert_eq!(all[0].line, 42);
        assert_eq!(all[0].name, "deploy start");

        assert!(store.delete_log_bookmark(id).unwrap());
        assert!(store.list_log_bookmarks().unwrap().is_empty());
        assert!(!store.delete_log_bookmark(id).unwrap());
    }

    #[test]
    fn bookmarks_are_newest_first() {
        let store = LauncherStore::open_in_memory().unwrap();
        for name in ["first", "second", "third"] {
            store
                .create_log_bookmark(&NewLogBookmark {
                    host_dir: "h".into(),
                    file_name: "f.log".into(),
                    line: 1,
                    name: name.into(),
                })
                .unwrap();
        }
        let names: Vec<String> = store
            .list_log_bookmarks()
            .unwrap()
            .into_iter()
            .map(|b| b.name)
            .collect();
        assert_eq!(names, vec!["third", "second", "first"]);
    }
}
