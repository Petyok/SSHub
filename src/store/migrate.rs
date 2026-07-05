use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const SCHEMA_VERSION: i64 = 7;

const V2_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS host_groups (
    id           INTEGER PRIMARY KEY,
    name         TEXT NOT NULL UNIQUE,
    sort_order   INTEGER NOT NULL DEFAULT 0,
    created_at   INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS identities (
    id              INTEGER PRIMARY KEY,
    name            TEXT NOT NULL UNIQUE,
    username        TEXT,
    private_key     TEXT,
    certificate     TEXT,
    sort_order      INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS hosts (
    id              INTEGER PRIMARY KEY,
    name            TEXT NOT NULL UNIQUE,
    label           TEXT,
    address         TEXT NOT NULL,
    port            INTEGER NOT NULL DEFAULT 22,
    group_id        INTEGER REFERENCES host_groups(id) ON DELETE SET NULL,
    identity_id     INTEGER REFERENCES identities(id) ON DELETE SET NULL,
    os_icon         TEXT,
    tags            TEXT NOT NULL DEFAULT '[]',
    notes           TEXT,
    proxy_jump      TEXT,
    forward_agent   INTEGER NOT NULL DEFAULT 0,
    remote_command  TEXT,
    sort_order      INTEGER NOT NULL DEFAULT 0,
    favorite        INTEGER NOT NULL DEFAULT 0,
    last_connected  INTEGER,
    source          TEXT NOT NULL DEFAULT 'launcher',
    ssh_config_hash TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);
";

const LEGACY_METADATA_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS host_metadata (
    host_name      TEXT PRIMARY KEY,
    tags           TEXT,
    description    TEXT,
    environment    TEXT,
    favorite       INTEGER NOT NULL DEFAULT 0,
    last_connected INTEGER
);
";

pub(crate) fn run_migrations(conn: &Connection, launcher_path: &Path) -> Result<()> {
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    // Wait instead of failing with SQLITE_BUSY when another instance (or the
    // watcher-triggered reimport) holds the write lock.
    conn.execute_batch("PRAGMA busy_timeout = 5000;")?;

    // Run the whole chain atomically: a crash mid-migration must not leave the
    // schema half-upgraded with no recorded version step.
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(V2_SCHEMA)?;

    let current = schema_version(conn)?;
    if current >= SCHEMA_VERSION {
        return Ok(());
    }

    if current < 3 {
        migrate_v2_to_v3(conn)?;
    }

    if current < 4 {
        migrate_v3_to_v4(conn)?;
    }

    if current < 5 {
        migrate_v4_to_v5(conn)?;
    }

    if current < 6 {
        migrate_v5_to_v6(conn)?;
    }

    if current < 7 {
        migrate_v6_to_v7(conn)?;
    }

    // Runs last so all columns it writes to (e.g. environment) already exist.
    if current == 0 {
        migrate_legacy_metadata(conn, launcher_path)?;
    }

    set_schema_version(conn, SCHEMA_VERSION)?;
    tx.commit()?;
    Ok(())
}

fn schema_version(conn: &Connection) -> Result<i64> {
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'schema_version'",
        [],
        |row| row.get(0),
    )?;
    if exists == 0 {
        return Ok(0);
    }

    let count: i64 = conn.query_row("SELECT COUNT(*) FROM schema_version", [], |row| row.get(0))?;
    if count == 0 {
        return Ok(0);
    }

    conn.query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
        row.get(0)
    })
    .map_err(Into::into)
}

fn set_schema_version(conn: &Connection, version: i64) -> Result<()> {
    conn.execute("DELETE FROM schema_version", [])?;
    conn.execute(
        "INSERT INTO schema_version (version) VALUES (?1)",
        params![version],
    )?;
    Ok(())
}

fn migrate_legacy_metadata(conn: &Connection, launcher_path: &Path) -> Result<()> {
    let metadata_path = legacy_metadata_path(launcher_path);
    if !metadata_path.exists() {
        return Ok(());
    }

    let legacy = Connection::open(&metadata_path)
        .with_context(|| format!("open legacy metadata db at {}", metadata_path.display()))?;
    legacy.execute_batch(LEGACY_METADATA_SCHEMA)?;

    let mut stmt = legacy.prepare(
        "SELECT host_name, tags, description, environment, favorite, last_connected
         FROM host_metadata",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, Option<i64>>(5)?,
        ))
    })?;

    let now = now_ts();
    for row in rows {
        let (host_name, tags_raw, description, environment, favorite, last_connected) = row?;
        // A single corrupt tags blob must not brick the whole app on startup:
        // fall back to no tags instead of failing the migration.
        let tags = tags_from_json(tags_raw).unwrap_or_default();
        let tags_json = serde_json::to_string(&tags)?;

        conn.execute(
            "INSERT OR IGNORE INTO hosts
                (name, label, address, port, tags, notes, environment, favorite, last_connected,
                 source, created_at, updated_at)
             VALUES (?1, NULL, ?1, 22, ?2, ?3, ?4, ?5, ?6, 'ssh_config', ?7, ?7)",
            params![
                host_name,
                tags_json,
                description,
                environment,
                favorite,
                last_connected,
                now,
            ],
        )?;
    }

    Ok(())
}

fn legacy_metadata_path(launcher_path: &Path) -> PathBuf {
    launcher_path
        .parent()
        .map(|dir| dir.join("metadata.db"))
        .unwrap_or_else(|| PathBuf::from("metadata.db"))
}

fn tags_from_json(raw: Option<String>) -> Result<Vec<String>> {
    match raw {
        None => Ok(Vec::new()),
        Some(s) if s.is_empty() => Ok(Vec::new()),
        Some(s) => Ok(serde_json::from_str(&s)?),
    }
}

fn migrate_v2_to_v3(conn: &Connection) -> Result<()> {
    // Add has_password column to identities if not present
    let has_col: bool = conn
        .prepare(
            "SELECT COUNT(*) FROM pragma_table_info('identities') WHERE name = 'has_password'",
        )?
        .query_row([], |row| row.get::<_, i64>(0))
        .map(|c| c > 0)?;
    if !has_col {
        conn.execute_batch(
            "ALTER TABLE identities ADD COLUMN has_password INTEGER NOT NULL DEFAULT 0;",
        )?;
    }

    // Add has_password column to hosts if not present
    let has_col: bool = conn
        .prepare("SELECT COUNT(*) FROM pragma_table_info('hosts') WHERE name = 'has_password'")?
        .query_row([], |row| row.get::<_, i64>(0))
        .map(|c| c > 0)?;
    if !has_col {
        conn.execute_batch(
            "ALTER TABLE hosts ADD COLUMN has_password INTEGER NOT NULL DEFAULT 0;",
        )?;
    }

    Ok(())
}

fn migrate_v3_to_v4(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS auth_events (
            id          INTEGER PRIMARY KEY,
            host_name   TEXT NOT NULL,
            username    TEXT,
            via         TEXT,
            status      TEXT NOT NULL DEFAULT 'ok',
            note        TEXT,
            created_at  INTEGER NOT NULL
        );",
    )?;

    // Add ping_ms column to hosts if not present
    let has_col: bool = conn
        .prepare("SELECT COUNT(*) FROM pragma_table_info('hosts') WHERE name = 'ping_ms'")?
        .query_row([], |row| row.get::<_, i64>(0))
        .map(|c| c > 0)?;
    if !has_col {
        conn.execute_batch("ALTER TABLE hosts ADD COLUMN ping_ms INTEGER;")?;
    }

    Ok(())
}

fn migrate_v4_to_v5(conn: &Connection) -> Result<()> {
    // Add username column to hosts if not present
    let has_col: bool = conn
        .prepare("SELECT COUNT(*) FROM pragma_table_info('hosts') WHERE name = 'username'")?
        .query_row([], |row| row.get::<_, i64>(0))
        .map(|c| c > 0)?;
    if !has_col {
        conn.execute_batch("ALTER TABLE hosts ADD COLUMN username TEXT;")?;
    }
    Ok(())
}

fn migrate_v5_to_v6(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS tunnels (
            id            INTEGER PRIMARY KEY,
            host_id       INTEGER REFERENCES hosts(id) ON DELETE CASCADE,
            tunnel_type   TEXT NOT NULL DEFAULT 'L',
            local_port    INTEGER NOT NULL,
            remote_host   TEXT NOT NULL DEFAULT 'localhost',
            remote_port   INTEGER NOT NULL DEFAULT 0,
            label         TEXT,
            auto_connect  INTEGER NOT NULL DEFAULT 0,
            created_at    INTEGER NOT NULL,
            updated_at    INTEGER NOT NULL
        );",
    )?;
    Ok(())
}

fn migrate_v6_to_v7(conn: &Connection) -> Result<()> {
    let has_col: bool = conn
        .prepare("SELECT COUNT(*) FROM pragma_table_info('hosts') WHERE name = 'environment'")?
        .query_row([], |row| row.get::<_, i64>(0))
        .map(|c| c > 0)?;
    if !has_col {
        conn.execute_batch("ALTER TABLE hosts ADD COLUMN environment TEXT;")?;
    }
    Ok(())
}

pub(crate) fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn temp_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn migration_creates_v2_tables() {
        let dir = temp_dir();
        let db_path = dir.path().join("launcher.db");
        let conn = Connection::open(&db_path).unwrap();
        run_migrations(&conn, &db_path).unwrap();

        let tables: Vec<String> = conn
            .prepare(
                "SELECT name FROM sqlite_master
                 WHERE type = 'table' AND name IN ('host_groups', 'identities', 'hosts', 'schema_version')
                 ORDER BY name",
            )
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(Result::ok)
            .collect();

        assert_eq!(
            tables,
            vec![
                "host_groups".to_string(),
                "hosts".to_string(),
                "identities".to_string(),
                "schema_version".to_string(),
            ]
        );

        let version: i64 = conn
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn migration_imports_legacy_metadata_rows() {
        let dir = temp_dir();
        let metadata_path = dir.path().join("metadata.db");
        let launcher_path = dir.path().join("launcher.db");

        let legacy = Connection::open(&metadata_path).unwrap();
        legacy.execute_batch(LEGACY_METADATA_SCHEMA).unwrap();
        legacy
            .execute(
                "INSERT INTO host_metadata (host_name, tags, description, favorite, last_connected)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    "web-prod",
                    r#"["prod","web"]"#,
                    "Production web",
                    1_i64,
                    1_700_000_000_i64,
                ],
            )
            .unwrap();
        drop(legacy);

        let conn = Connection::open(&launcher_path).unwrap();
        run_migrations(&conn, &launcher_path).unwrap();

        let (name, source, tags, notes, favorite, last_connected): (
            String,
            String,
            String,
            Option<String>,
            i64,
            Option<i64>,
        ) = conn
            .query_row(
                "SELECT name, source, tags, notes, favorite, last_connected FROM hosts WHERE name = ?1",
                params!["web-prod"],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .unwrap();

        assert_eq!(name, "web-prod");
        assert_eq!(source, "ssh_config");
        assert_eq!(notes.as_deref(), Some("Production web"));
        assert_eq!(favorite, 1);
        assert_eq!(last_connected, Some(1_700_000_000));
        let parsed_tags: Vec<String> = serde_json::from_str(&tags).unwrap();
        assert_eq!(parsed_tags, vec!["prod", "web"]);
    }

    #[test]
    fn migration_imports_legacy_environment_and_tolerates_bad_tags() {
        let dir = temp_dir();
        let metadata_path = dir.path().join("metadata.db");
        let launcher_path = dir.path().join("launcher.db");

        let legacy = Connection::open(&metadata_path).unwrap();
        legacy.execute_batch(LEGACY_METADATA_SCHEMA).unwrap();
        legacy
            .execute(
                "INSERT INTO host_metadata (host_name, tags, description, environment, favorite)
                 VALUES ('envhost', 'not-json', NULL, 'prod', 0)",
                [],
            )
            .unwrap();
        drop(legacy);

        let conn = Connection::open(&launcher_path).unwrap();
        run_migrations(&conn, &launcher_path).unwrap();

        let (environment, tags): (Option<String>, String) = conn
            .query_row(
                "SELECT environment, tags FROM hosts WHERE name = 'envhost'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(environment.as_deref(), Some("prod"));
        assert_eq!(tags, "[]");
    }

    #[test]
    fn migration_is_idempotent() {
        let dir = temp_dir();
        let db_path = dir.path().join("launcher.db");
        let conn = Connection::open(&db_path).unwrap();
        run_migrations(&conn, &db_path).unwrap();
        run_migrations(&conn, &db_path).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn migration_skips_legacy_when_metadata_missing() {
        let dir = temp_dir();
        let db_path = dir.path().join("launcher.db");
        let conn = Connection::open(&db_path).unwrap();
        run_migrations(&conn, &db_path).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM hosts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}
