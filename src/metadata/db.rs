use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::Mutex;

use super::HostMetadata;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS host_metadata (
    host_name      TEXT PRIMARY KEY,
    tags           TEXT,
    description    TEXT,
    environment    TEXT,
    favorite       INTEGER NOT NULL DEFAULT 0,
    last_connected INTEGER
);
";

/// SQLite-backed host metadata store.
pub trait MetadataStore: Send + Sync {
    fn get(&self, host_name: &str) -> Result<Option<HostMetadata>>;
    fn get_all(&self) -> Result<Vec<HostMetadata>>;
    fn upsert(&self, meta: &HostMetadata) -> Result<()>;
    fn toggle_favorite(&self, host_name: &str) -> Result<bool>;
    fn set_last_connected(&self, host_name: &str, timestamp: i64) -> Result<()>;
    fn ensure_defaults(&self, host_names: &[String]) -> Result<()>;
}

#[derive(Debug)]
pub struct MetadataDb {
    conn: Mutex<Connection>,
}

impl MetadataDb {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create metadata db directory {}", parent.display()))?;
            crate::secure_fs::restrict_dir(parent);
        }
        let conn = Connection::open(path)
            .with_context(|| format!("open metadata db at {}", path.display()))?;
        crate::secure_fs::restrict_file(path);
        conn.execute_batch("PRAGMA busy_timeout = 5000;")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn with_conn<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T>,
    {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("metadata db connection poisoned"))?;
        f(&conn)
    }
}

impl Default for MetadataDb {
    fn default() -> Self {
        Self::open_in_memory().expect("open in-memory metadata db")
    }
}

impl MetadataStore for MetadataDb {
    fn get(&self, host_name: &str) -> Result<Option<HostMetadata>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT host_name, tags, description, environment, favorite, last_connected
                 FROM host_metadata WHERE host_name = ?1",
            )?;
            stmt.query_row(params![host_name], row_to_metadata)
                .optional()
                .map_err(Into::into)
        })
    }

    fn get_all(&self) -> Result<Vec<HostMetadata>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT host_name, tags, description, environment, favorite, last_connected
                 FROM host_metadata ORDER BY host_name",
            )?;
            let rows = stmt.query_map([], row_to_metadata)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        })
    }

    fn upsert(&self, meta: &HostMetadata) -> Result<()> {
        let tags_json = tags_to_json(&meta.tags)?;
        let favorite = i64::from(meta.favorite);

        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO host_metadata
                    (host_name, tags, description, environment, favorite, last_connected)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(host_name) DO UPDATE SET
                    tags = excluded.tags,
                    description = excluded.description,
                    environment = excluded.environment,
                    favorite = excluded.favorite,
                    last_connected = excluded.last_connected",
                params![
                    meta.host_name,
                    tags_json,
                    meta.description,
                    meta.environment,
                    favorite,
                    meta.last_connected,
                ],
            )?;
            Ok(())
        })
    }

    fn toggle_favorite(&self, host_name: &str) -> Result<bool> {
        let current: Option<i64> = self.with_conn(|conn| {
            conn.query_row(
                "SELECT favorite FROM host_metadata WHERE host_name = ?1",
                params![host_name],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(Into::into)
        })?;

        let new_favorite = match current {
            Some(v) => v == 0,
            None => true,
        };

        if let Some(old) = current {
            self.with_conn(|conn| {
                conn.execute(
                    "UPDATE host_metadata SET favorite = ?1 WHERE host_name = ?2",
                    params![i64::from(new_favorite), host_name],
                )?;
                Ok(())
            })?;
            Ok(old != 0)
        } else {
            self.upsert(&HostMetadata {
                host_name: host_name.to_string(),
                favorite: new_favorite,
                ..Default::default()
            })?;
            Ok(true)
        }
    }

    fn set_last_connected(&self, host_name: &str, timestamp: i64) -> Result<()> {
        let updated: usize = self.with_conn(|conn| {
            conn.execute(
                "UPDATE host_metadata SET last_connected = ?1 WHERE host_name = ?2",
                params![timestamp, host_name],
            )
            .map_err(Into::into)
        })?;

        if updated == 0 {
            self.upsert(&HostMetadata {
                host_name: host_name.to_string(),
                last_connected: Some(timestamp),
                ..Default::default()
            })?;
        }

        Ok(())
    }

    fn ensure_defaults(&self, host_names: &[String]) -> Result<()> {
        for host_name in host_names {
            if self.get(host_name)?.is_none() {
                self.upsert(&HostMetadata::new(host_name))?;
            }
        }
        Ok(())
    }
}

fn tags_to_json(tags: &[String]) -> Result<String> {
    Ok(serde_json::to_string(tags)?)
}

fn tags_from_json(raw: Option<String>) -> Result<Vec<String>> {
    match raw {
        None => Ok(Vec::new()),
        Some(s) if s.is_empty() => Ok(Vec::new()),
        Some(s) => Ok(serde_json::from_str(&s)?),
    }
}

fn row_to_metadata(row: &rusqlite::Row<'_>) -> rusqlite::Result<HostMetadata> {
    let tags_raw: Option<String> = row.get(1)?;
    let tags = tags_from_json(tags_raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            1,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
        )
    })?;

    Ok(HostMetadata {
        host_name: row.get(0)?,
        tags,
        description: row.get(2)?,
        environment: row.get(3)?,
        favorite: row.get::<_, i64>(4)? != 0,
        last_connected: row.get(5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> MetadataDb {
        MetadataDb::open_in_memory().unwrap()
    }

    #[test]
    fn get_missing_returns_none() {
        let db = db();
        assert!(db.get("missing").unwrap().is_none());
    }

    #[test]
    fn upsert_and_get_roundtrip() {
        let db = db();
        let meta = HostMetadata {
            host_name: "web".into(),
            tags: vec!["prod".into(), "db".into()],
            description: Some("Primary web".into()),
            environment: Some("prod".into()),
            favorite: true,
            last_connected: Some(1_700_000_000),
        };

        db.upsert(&meta).unwrap();
        assert_eq!(db.get("web").unwrap(), Some(meta));
    }

    #[test]
    fn tags_json_roundtrip() {
        let db = db();
        let meta = HostMetadata {
            host_name: "db".into(),
            tags: vec!["staging".into(), "postgres".into()],
            ..Default::default()
        };

        db.upsert(&meta).unwrap();
        let loaded = db.get("db").unwrap().unwrap();
        assert_eq!(loaded.tags, meta.tags);
    }

    #[test]
    fn get_all_returns_sorted_rows() {
        let db = db();
        db.upsert(&HostMetadata::new("zebra")).unwrap();
        db.upsert(&HostMetadata::new("alpha")).unwrap();

        let all = db.get_all().unwrap();
        assert_eq!(
            all.iter().map(|m| m.host_name.as_str()).collect::<Vec<_>>(),
            vec!["alpha", "zebra"]
        );
    }

    #[test]
    fn toggle_favorite_flips_existing() {
        let db = db();
        db.upsert(&HostMetadata::new("web")).unwrap();

        assert!(!db.toggle_favorite("web").unwrap());
        assert!(db.get("web").unwrap().unwrap().favorite);

        assert!(db.toggle_favorite("web").unwrap());
        assert!(!db.get("web").unwrap().unwrap().favorite);
    }

    #[test]
    fn toggle_favorite_inserts_missing_host() {
        let db = db();
        assert!(db.toggle_favorite("new-host").unwrap());
        assert!(db.get("new-host").unwrap().unwrap().favorite);
    }

    #[test]
    fn set_last_connected_updates_existing() {
        let db = db();
        db.upsert(&HostMetadata::new("web")).unwrap();

        db.set_last_connected("web", 1_234).unwrap();
        assert_eq!(db.get("web").unwrap().unwrap().last_connected, Some(1_234));
    }

    #[test]
    fn set_last_connected_inserts_missing_host() {
        let db = db();
        db.set_last_connected("web", 9_999).unwrap();
        assert_eq!(db.get("web").unwrap().unwrap().last_connected, Some(9_999));
    }

    #[test]
    fn ensure_defaults_inserts_only_missing() {
        let db = db();
        db.upsert(&HostMetadata {
            host_name: "existing".into(),
            favorite: true,
            ..Default::default()
        })
        .unwrap();

        db.ensure_defaults(&["existing".into(), "new".into()])
            .unwrap();

        assert!(db.get("existing").unwrap().unwrap().favorite);
        assert_eq!(db.get("new").unwrap().unwrap().host_name, "new");
        assert_eq!(db.get_all().unwrap().len(), 2);
    }
}
