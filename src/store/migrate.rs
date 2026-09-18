use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const SCHEMA_VERSION: i64 = 17;

/// Name of the reserved, auto-created "Favorites" group. Membership in it is the
/// source of truth for a host's favourite status.
pub(crate) const FAVORITES_GROUP_NAME: &str = "Favorites";

const V2_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS host_groups (
    id           INTEGER PRIMARY KEY,
    name         TEXT NOT NULL UNIQUE,
    sort_order   INTEGER NOT NULL DEFAULT 0,
    parent_id    INTEGER REFERENCES host_groups(id) ON DELETE SET NULL,
    default_identity_id INTEGER REFERENCES identities(id) ON DELETE SET NULL,
    default_username TEXT,
    default_port INTEGER,
    default_proxy_jump TEXT,
    default_transport TEXT,
    default_forward_agent INTEGER,
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
    port            INTEGER,
    group_id        INTEGER REFERENCES host_groups(id) ON DELETE SET NULL,
    identity_id     INTEGER REFERENCES identities(id) ON DELETE SET NULL,
    os_icon         TEXT,
    tags            TEXT NOT NULL DEFAULT '[]',
    notes           TEXT,
    proxy_jump      TEXT,
    forward_agent   INTEGER,
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

/// How many `launcher.db.bak-v*` snapshots to keep next to the database.
const KEPT_BACKUPS: usize = 3;

/// Snapshot the database next to itself before a migration rewrites it.
///
/// `VACUUM INTO` (not a file copy) because it is a single statement that
/// writes a consistent snapshot even with a WAL and other readers attached.
/// Best-effort by design: a failed snapshot must not stop a user from opening
/// their hosts, so it warns and carries on.
fn backup_before_migration(conn: &Connection, launcher_path: &Path, from_version: i64) {
    let mut name = launcher_path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".bak-v{from_version}-{}", now_ts()));
    let dest = launcher_path.with_file_name(name);
    if dest.exists() {
        return;
    }
    match conn.execute("VACUUM INTO ?1", params![dest.to_string_lossy()]) {
        Ok(_) => prune_backups(launcher_path),
        Err(e) => eprintln!(
            "sshub: could not back up {} before migrating: {e:#}",
            launcher_path.display()
        ),
    }
}

/// Keep the newest [`KEPT_BACKUPS`] snapshots, delete the rest.
fn prune_backups(launcher_path: &Path) {
    let Some(dir) = launcher_path.parent() else {
        return;
    };
    let prefix = format!(
        "{}.bak-v",
        launcher_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
    );
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut backups: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().starts_with(&prefix))
                .unwrap_or(false)
        })
        .collect();
    if backups.len() <= KEPT_BACKUPS {
        return;
    }
    // The name ends in the unix timestamp it was taken at, so the byte order
    // is the time order — no metadata call needed.
    backups.sort();
    for old in &backups[..backups.len() - KEPT_BACKUPS] {
        let _ = std::fs::remove_file(old);
    }
}

pub(crate) fn run_migrations(conn: &Connection, launcher_path: &Path) -> Result<()> {
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    // Wait instead of failing with SQLITE_BUSY when another instance (or the
    // watcher-triggered reimport) holds the write lock.
    conn.execute_batch("PRAGMA busy_timeout = 5000;")?;

    // Snapshot first: a migration rewrites tables in place, and v0.17.0 proved
    // what a wrong one costs. Nothing to save for a database that does not
    // exist yet (version 0) or is already current. Must run before the
    // transaction below — `VACUUM INTO` cannot run inside one.
    let before = schema_version(conn)?;
    if before > 0 && before < SCHEMA_VERSION {
        backup_before_migration(conn, launcher_path, before);
    }

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

    if current < 8 {
        migrate_v7_to_v8(conn)?;
    }

    if current < 9 {
        migrate_v8_to_v9(conn)?;
    }

    if current < 10 {
        migrate_v9_to_v10(conn)?;
    }

    if current < 11 {
        migrate_v10_to_v11(conn)?;
    }

    if current < 12 {
        migrate_v11_to_v12(conn)?;
    }

    if current < 13 {
        migrate_v12_to_v13(conn)?;
    }

    if current < 14 {
        migrate_v13_to_v14(conn)?;
    }

    if current < 15 {
        migrate_v14_to_v15(conn)?;
    }
    if current < 16 {
        conn.execute_batch(
            "CREATE TABLE command_history (
            id INTEGER PRIMARY KEY,
            host_id INTEGER NOT NULL REFERENCES hosts(id) ON DELETE CASCADE,
            command TEXT NOT NULL CHECK(length(CAST(command AS BLOB)) BETWEEN 1 AND 4096),
            last_used INTEGER NOT NULL,
            use_count INTEGER NOT NULL DEFAULT 1,
            created_at INTEGER NOT NULL,
            UNIQUE(host_id, command));
            CREATE INDEX command_history_host_recent ON command_history(host_id,last_used DESC);",
        )?;
    }

    if current < 16 {
        migrate_v15_to_v16(conn)?;
    }

    if current < 17 {
        migrate_v16_to_v17(conn)?;
    }

    // Runs last so all columns it writes to (e.g. environment) already exist.
    // Best-effort: a corrupt or locked legacy metadata.db must not abort the
    // whole migration (which would roll back the schema and, since the version
    // never advances, brick every subsequent launch too). Skip it and carry on.
    if current == 0 {
        if let Err(e) = migrate_legacy_metadata(conn, launcher_path) {
            eprintln!("sshub: skipping legacy metadata import: {e:#}");
        }
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

fn migrate_v7_to_v8(conn: &Connection) -> Result<()> {
    // Small key/value store for UI state that isn't host data (e.g. which
    // groups are collapsed in the tree).
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS ui_state (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );",
    )?;
    Ok(())
}

fn migrate_v8_to_v9(conn: &Connection) -> Result<()> {
    // A group can name a default identity; new hosts added to the group
    // inherit it automatically.
    let has_col: bool = conn
        .prepare(
            "SELECT COUNT(*) FROM pragma_table_info('host_groups') WHERE name = 'default_identity_id'",
        )?
        .query_row([], |row| row.get::<_, i64>(0))
        .map(|c| c > 0)?;
    if !has_col {
        conn.execute_batch(
            "ALTER TABLE host_groups
                ADD COLUMN default_identity_id INTEGER
                REFERENCES identities(id) ON DELETE SET NULL;",
        )?;
    }
    Ok(())
}

fn migrate_v9_to_v10(conn: &Connection) -> Result<()> {
    // Groups can nest: a group may name a parent group. Deleting a parent
    // promotes its children to the top level (ON DELETE SET NULL).
    let has_col: bool = conn
        .prepare("SELECT COUNT(*) FROM pragma_table_info('host_groups') WHERE name = 'parent_id'")?
        .query_row([], |row| row.get::<_, i64>(0))
        .map(|c| c > 0)?;
    if !has_col {
        conn.execute_batch(
            "ALTER TABLE host_groups
                ADD COLUMN parent_id INTEGER
                REFERENCES host_groups(id) ON DELETE SET NULL;",
        )?;
    }
    Ok(())
}

fn migrate_v10_to_v11(conn: &Connection) -> Result<()> {
    // Hosts can belong to several groups at once. A join table replaces the
    // single `hosts.group_id` FK as the source of truth (the column is kept for
    // back-compat but no longer authoritative). "Favorites" becomes a real,
    // reserved group; membership in it is the favourite flag.

    // 1. Reserved marker on groups (Favorites can't be renamed/deleted).
    let has_reserved: bool = conn
        .prepare("SELECT COUNT(*) FROM pragma_table_info('host_groups') WHERE name = 'reserved'")?
        .query_row([], |row| row.get::<_, i64>(0))
        .map(|c| c > 0)?;
    if !has_reserved {
        conn.execute_batch(
            "ALTER TABLE host_groups ADD COLUMN reserved INTEGER NOT NULL DEFAULT 0;",
        )?;
    }

    // 2. The membership join table.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS host_group_memberships (
            host_id  INTEGER NOT NULL REFERENCES hosts(id) ON DELETE CASCADE,
            group_id INTEGER NOT NULL REFERENCES host_groups(id) ON DELETE CASCADE,
            PRIMARY KEY (host_id, group_id)
        );",
    )?;

    // 3. Ensure exactly one reserved Favorites group exists, sorted to the very
    // top. NEVER repurpose a user's pre-existing group: if the "Favorites" name
    // is already taken by a normal group, register the reserved group under a
    // distinct label (the app finds it by the `reserved` flag, not the name).
    let now = now_ts();
    let reserved_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM host_groups WHERE reserved = 1",
        [],
        |r| r.get(0),
    )?;
    let fav_id: i64 = if reserved_count > 0 {
        conn.query_row(
            "SELECT id FROM host_groups WHERE reserved = 1 ORDER BY id LIMIT 1",
            [],
            |r| r.get(0),
        )?
    } else {
        let mut name = FAVORITES_GROUP_NAME.to_string();
        let mut suffix = 2;
        while conn.query_row(
            "SELECT COUNT(*) FROM host_groups WHERE name = ?1",
            params![name],
            |r| r.get::<_, i64>(0),
        )? > 0
        {
            name = format!("{FAVORITES_GROUP_NAME} ({suffix})");
            suffix += 1;
        }
        conn.execute(
            "INSERT INTO host_groups (name, sort_order, reserved, created_at)
             VALUES (?1, -1000, 1, ?2)",
            params![name, now],
        )?;
        conn.last_insert_rowid()
    };

    // 4. Backfill memberships from the legacy single group_id and favourite flag.
    conn.execute_batch(
        "INSERT OR IGNORE INTO host_group_memberships (host_id, group_id)
             SELECT id, group_id FROM hosts WHERE group_id IS NOT NULL;",
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO host_group_memberships (host_id, group_id)
             SELECT id, ?1 FROM hosts WHERE favorite = 1",
        params![fav_id],
    )?;

    Ok(())
}

fn migrate_v11_to_v12(conn: &Connection) -> Result<()> {
    let has_hosts_col: bool = conn
        .prepare("SELECT COUNT(*) FROM pragma_table_info('hosts') WHERE name = 'session_logging'")?
        .query_row([], |row| row.get::<_, i64>(0))
        .map(|c| c > 0)?;
    if !has_hosts_col {
        conn.execute_batch("ALTER TABLE hosts ADD COLUMN session_logging INTEGER;")?;
    }

    let has_log_path: bool = conn
        .prepare("SELECT COUNT(*) FROM pragma_table_info('auth_events') WHERE name = 'log_path'")?
        .query_row([], |row| row.get::<_, i64>(0))
        .map(|c| c > 0)?;
    if !has_log_path {
        conn.execute_batch("ALTER TABLE auth_events ADD COLUMN log_path TEXT;")?;
    }

    Ok(())
}

fn migrate_v12_to_v13(conn: &Connection) -> Result<()> {
    let has_transport: bool = conn
        .prepare("SELECT COUNT(*) FROM pragma_table_info('hosts') WHERE name = 'transport'")?
        .query_row([], |row| row.get::<_, i64>(0))
        .map(|c| c > 0)?;
    if !has_transport {
        conn.execute_batch("ALTER TABLE hosts ADD COLUMN transport TEXT NOT NULL DEFAULT 'ssh';")?;
    }
    Ok(())
}

fn migrate_v13_to_v14(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS snippets (
            id           INTEGER PRIMARY KEY,
            name         TEXT NOT NULL,
            command      TEXT NOT NULL,
            description  TEXT,
            tags         TEXT NOT NULL DEFAULT '[]',
            created_at   INTEGER NOT NULL,
            updated_at   INTEGER NOT NULL
        );",
    )?;
    Ok(())
}

fn migrate_v14_to_v15(conn: &Connection) -> Result<()> {
    // Named jump-points into session logs. Keyed by the on-disk host directory
    // and segment file name plus a line offset, not a host row id, so a
    // bookmark survives deleting and re-adding the host.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS log_bookmarks (
            id          INTEGER PRIMARY KEY,
            host_dir    TEXT NOT NULL,
            file_name   TEXT NOT NULL,
            line        INTEGER NOT NULL,
            name        TEXT NOT NULL,
            created_at  INTEGER NOT NULL
        );",
    )?;
    Ok(())
}
fn migrate_v15_to_v16(conn: &Connection) -> Result<()> {
    // Group-level connection defaults (issue #74): identity already existed;
    // username, port, ProxyJump, transport and agent-forwarding join it.
    // Hosts resolve host-explicit -> nearest ancestor group -> global default,
    // so the host columns become nullable (`NULL` = inherit).
    for (column, ddl) in [
        (
            "default_username",
            "ALTER TABLE host_groups ADD COLUMN default_username TEXT;",
        ),
        (
            "default_port",
            "ALTER TABLE host_groups ADD COLUMN default_port INTEGER;",
        ),
        (
            "default_proxy_jump",
            "ALTER TABLE host_groups ADD COLUMN default_proxy_jump TEXT;",
        ),
        (
            "default_transport",
            "ALTER TABLE host_groups ADD COLUMN default_transport TEXT;",
        ),
        (
            "default_forward_agent",
            "ALTER TABLE host_groups ADD COLUMN default_forward_agent INTEGER;",
        ),
    ] {
        let has_col: bool = conn
            .prepare(&format!(
                "SELECT COUNT(*) FROM pragma_table_info('host_groups') WHERE name = '{column}'"
            ))?
            .query_row([], |row| row.get::<_, i64>(0))
            .map(|c| c > 0)?;
        if !has_col {
            conn.execute_batch(ddl)?;
        }
    }

    // `hosts.port`, `hosts.forward_agent` and `hosts.transport` were NOT NULL
    // with creation-time defaults (22 / off / ssh), which made "unset" (and
    // therefore inheritance) unrepresentable. Rebuild the table nullable.
    // Rows that still carry the old creation defaults become NULL so
    // long-standing untouched hosts start inheriting; genuinely custom values
    // are preserved verbatim.
    let hosts_nullable: bool = conn
        .prepare(
            "SELECT COUNT(*) FROM pragma_table_info('hosts')
             WHERE name IN ('port', 'forward_agent', 'transport') AND \"notnull\" = 0",
        )?
        .query_row([], |row| row.get::<_, i64>(0))
        .map(|c| c == 3)?;
    if !hosts_nullable {
        // `DROP TABLE hosts` below runs an implicit DELETE of every row, and
        // with `PRAGMA foreign_keys = ON` that fires `ON DELETE CASCADE` on
        // every child table — emptying host_group_memberships, tunnels and
        // command_history (v0.17.0 shipped exactly that: every host came back
        // ungrouped). `defer_foreign_keys` does not help: it defers the
        // *checking* of violations to commit, not the cascade actions
        // themselves, and `PRAGMA foreign_keys` is a no-op inside the
        // transaction this runs in. So the child rows are copied out and put
        // back: host ids are preserved by the rebuild, so they still match.
        conn.execute_batch("PRAGMA defer_foreign_keys = ON;")?;
        conn.execute_batch(
            "CREATE TEMP TABLE hosts_rebuild_memberships AS
                 SELECT * FROM host_group_memberships;
             CREATE TEMP TABLE hosts_rebuild_tunnels AS SELECT * FROM tunnels;
             CREATE TEMP TABLE hosts_rebuild_history AS SELECT * FROM command_history;",
        )?;
        conn.execute_batch(
            "CREATE TABLE hosts_new (
                id              INTEGER PRIMARY KEY,
                name            TEXT NOT NULL UNIQUE,
                label           TEXT,
                address         TEXT NOT NULL,
                port            INTEGER,
                group_id        INTEGER REFERENCES host_groups(id) ON DELETE SET NULL,
                identity_id     INTEGER REFERENCES identities(id) ON DELETE SET NULL,
                os_icon         TEXT,
                tags            TEXT NOT NULL DEFAULT '[]',
                notes           TEXT,
                proxy_jump      TEXT,
                forward_agent   INTEGER,
                remote_command  TEXT,
                sort_order      INTEGER NOT NULL DEFAULT 0,
                favorite        INTEGER NOT NULL DEFAULT 0,
                last_connected  INTEGER,
                source          TEXT NOT NULL DEFAULT 'launcher',
                ssh_config_hash TEXT,
                created_at      INTEGER NOT NULL,
                updated_at      INTEGER NOT NULL,
                has_password    INTEGER NOT NULL DEFAULT 0,
                ping_ms         INTEGER,
                username        TEXT,
                environment     TEXT,
                session_logging INTEGER,
                transport       TEXT
            );",
        )?;
        conn.execute_batch(
            "INSERT INTO hosts_new
                (id, name, label, address, port, group_id, identity_id, os_icon,
                 tags, notes, proxy_jump, forward_agent, remote_command, sort_order,
                 favorite, last_connected, source, ssh_config_hash, created_at,
                 updated_at, has_password, ping_ms, username, environment,
                 session_logging, transport)
             SELECT id, name, label, address,
                CASE WHEN port = 22 THEN NULL ELSE port END,
                group_id, identity_id, os_icon, tags, notes, proxy_jump,
                CASE WHEN forward_agent = 0 THEN NULL ELSE forward_agent END,
                remote_command, sort_order, favorite, last_connected, source,
                ssh_config_hash, created_at, updated_at, has_password, ping_ms,
                username, environment, session_logging,
                CASE WHEN transport = 'ssh' THEN NULL ELSE transport END
             FROM hosts;",
        )?;
        conn.execute_batch("DROP TABLE hosts;")?;
        conn.execute_batch("ALTER TABLE hosts_new RENAME TO hosts;")?;
        conn.execute_batch(
            "INSERT OR IGNORE INTO host_group_memberships
                 SELECT * FROM hosts_rebuild_memberships;
             INSERT OR IGNORE INTO tunnels SELECT * FROM hosts_rebuild_tunnels;
             INSERT OR IGNORE INTO command_history SELECT * FROM hosts_rebuild_history;
             DROP TABLE hosts_rebuild_memberships;
             DROP TABLE hosts_rebuild_tunnels;
             DROP TABLE hosts_rebuild_history;",
        )?;
    }
    Ok(())
}

/// Repair databases that v0.17.0 already emptied.
///
/// The v16 rebuild cascade-deleted `host_group_memberships`, so every host
/// rendered as ungrouped even though `hosts.group_id` survived the copy. The
/// group tree reads memberships, so re-seed them from the column that lived:
/// `set_host_groups` keeps `group_id` pointing at the primary membership, so
/// it is current even for hosts regrouped under 0.17.0. `INSERT OR IGNORE`
/// only adds, so a database that was never damaged is untouched.
///
/// Memberships beyond the primary group cannot be recovered — `group_id` holds
/// one — and cascaded `tunnels` rows are gone for good.
fn migrate_v16_to_v17(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "INSERT OR IGNORE INTO host_group_memberships (host_id, group_id)
             SELECT id, group_id FROM hosts WHERE group_id IS NOT NULL;
         INSERT OR IGNORE INTO host_group_memberships (host_id, group_id)
             SELECT h.id, g.id FROM hosts h, host_groups g
             WHERE h.favorite = 1 AND g.reserved = 1;",
    )?;
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
    fn migration_v13_to_v14_creates_snippets() {
        let dir = temp_dir();
        let db_path = dir.path().join("launcher.db");
        let conn = Connection::open(&db_path).unwrap();
        run_migrations(&conn, &db_path).unwrap();

        // Simulate a pre-v14 database: remove every post-v13 table before
        // rolling the recorded version back and re-running the chain.
        conn.execute_batch(
            "DROP TABLE snippets; DROP TABLE log_bookmarks; DROP TABLE command_history;",
        )
        .unwrap();
        set_schema_version(&conn, 13).unwrap();
        run_migrations(&conn, &db_path).unwrap();

        let has_snippets: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='snippets'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(has_snippets, 1);

        let version: i64 = conn
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    /// Frozen copy of the v15 schema: the oracle this migration is tested
    /// against. It must NOT track the live `V2_SCHEMA` — if the rebuild below
    /// drops a column or mistranslates a sentinel, this fixture still
    /// represents what real v15 databases look like and the test fails.
    const V15_FROZEN_SCHEMA: &str = "
CREATE TABLE schema_version (version INTEGER NOT NULL);
INSERT INTO schema_version (version) VALUES (15);
CREATE TABLE identities (
    id              INTEGER PRIMARY KEY,
    name            TEXT NOT NULL UNIQUE,
    username        TEXT,
    private_key     TEXT,
    certificate     TEXT,
    sort_order      INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL,
    has_password    INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE host_groups (
    id           INTEGER PRIMARY KEY,
    name         TEXT NOT NULL UNIQUE,
    sort_order   INTEGER NOT NULL DEFAULT 0,
    parent_id    INTEGER REFERENCES host_groups(id) ON DELETE SET NULL,
    default_identity_id INTEGER REFERENCES identities(id) ON DELETE SET NULL,
    reserved     INTEGER NOT NULL DEFAULT 0,
    created_at   INTEGER NOT NULL
);
CREATE TABLE hosts (
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
    updated_at      INTEGER NOT NULL,
    has_password    INTEGER NOT NULL DEFAULT 0,
    ping_ms         INTEGER,
    username        TEXT,
    environment     TEXT,
    session_logging INTEGER,
    transport       TEXT NOT NULL DEFAULT 'ssh'
);
CREATE TABLE host_group_memberships (
    host_id  INTEGER NOT NULL REFERENCES hosts(id) ON DELETE CASCADE,
    group_id INTEGER NOT NULL REFERENCES host_groups(id) ON DELETE CASCADE,
    PRIMARY KEY (host_id, group_id)
);
CREATE TABLE tunnels (
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
);
INSERT INTO host_groups (name, sort_order, reserved, created_at)
    VALUES ('prod', 0, 0, 1700000000);
";

    /// The database is snapshotted before a migration touches it.
    ///
    /// Oracle: the snapshot itself — open it and it must still be the old
    /// schema with the old rows, whatever the migration did to the original.
    #[test]
    fn migration_snapshots_the_database_first_and_keeps_three() {
        let dir = temp_dir();
        let db_path = dir.path().join("launcher.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(V15_FROZEN_SCHEMA).unwrap();
        conn.execute_batch(
            "INSERT INTO hosts (name, address, port, forward_agent, transport,
                                created_at, updated_at)
             VALUES ('a', '10.0.0.1', 22, 0, 'ssh', 1700000000, 1700000000);",
        )
        .unwrap();
        drop(conn);

        let migrated = Connection::open(&db_path).unwrap();
        run_migrations(&migrated, &db_path).unwrap();
        drop(migrated);

        let backups = || {
            let mut found: Vec<_> = std::fs::read_dir(dir.path())
                .unwrap()
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("launcher.db.bak-v")
                })
                .collect();
            found.sort();
            found
        };
        let snapshot = backups();
        assert_eq!(snapshot.len(), 1, "one snapshot per migrated launch");
        assert!(
            snapshot[0]
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains(".bak-v15-"),
            "the name records the version migrated away from: {:?}",
            snapshot[0]
        );

        // The snapshot is the pre-migration database, not a copy of the new one.
        let old = Connection::open(&snapshot[0]).unwrap();
        let version: i64 = old
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(version, 15, "snapshot must predate the migration");
        let port: i64 = old
            .query_row("SELECT port FROM hosts WHERE name = 'a'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(port, 22, "snapshot keeps the pre-migration row verbatim");
        drop(old);

        // Only the newest three survive. Timestamps are seconds, so write the
        // extra names directly rather than migrating five times in one second.
        for ts in [1, 2, 3, 4] {
            std::fs::copy(
                &snapshot[0],
                dir.path().join(format!("launcher.db.bak-v15-{ts}")),
            )
            .unwrap();
        }
        prune_backups(&db_path);
        let kept = backups();
        assert_eq!(kept.len(), KEPT_BACKUPS, "pruning keeps three: {kept:?}");
        assert!(
            kept.iter()
                .all(|p| !p.file_name().unwrap().to_string_lossy().ends_with("-1")),
            "the oldest must be the one dropped: {kept:?}"
        );
    }

    /// Rows a host owns must survive the v16 table rebuild.
    ///
    /// v0.17.0 lost every one of them: `DROP TABLE hosts` runs an implicit
    /// DELETE, and with `PRAGMA foreign_keys = ON` that fires the children's
    /// `ON DELETE CASCADE`. Users opened the app to find every host ungrouped
    /// and their tunnels gone. Oracle: SQLite itself — the counts before the
    /// migration are the counts after it.
    #[test]
    fn migration_v16_rebuild_keeps_memberships_and_tunnels() {
        let dir = temp_dir();
        let db_path = dir.path().join("launcher.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(V15_FROZEN_SCHEMA).unwrap();
        conn.execute_batch(
            "INSERT INTO hosts (name, address, port, group_id, forward_agent,
                                transport, created_at, updated_at)
             VALUES ('a', '10.0.0.1', 22,
                     (SELECT id FROM host_groups WHERE name = 'prod'), 0, 'ssh',
                     1700000000, 1700000000),
                    ('b', '10.0.0.2', 2222,
                     (SELECT id FROM host_groups WHERE name = 'prod'), 1, 'mosh',
                     1700000000, 1700000000);
             INSERT INTO host_group_memberships (host_id, group_id)
                 SELECT id, group_id FROM hosts;
             INSERT INTO tunnels (host_id, tunnel_type, local_port, remote_host,
                                  remote_port, created_at, updated_at)
                 VALUES ((SELECT id FROM hosts WHERE name = 'a'), 'L', 8080,
                         'localhost', 80, 1700000000, 1700000000);",
        )
        .unwrap();
        drop(conn);

        let migrated = Connection::open(&db_path).unwrap();
        run_migrations(&migrated, &db_path).unwrap();
        let memberships: i64 = migrated
            .query_row("SELECT COUNT(*) FROM host_group_memberships", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(memberships, 2, "the rebuild must not cascade groups away");
        let tunnels: i64 = migrated
            .query_row("SELECT COUNT(*) FROM tunnels", [], |r| r.get(0))
            .unwrap();
        assert_eq!(tunnels, 1, "the rebuild must not cascade tunnels away");
        let host_of_tunnel: String = migrated
            .query_row(
                "SELECT h.name FROM tunnels t JOIN hosts h ON h.id = t.host_id",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            host_of_tunnel, "a",
            "restored rows must still point at their host"
        );
    }

    /// A database v0.17.0 already emptied gets its groups back on upgrade.
    #[test]
    fn migration_v17_reseeds_memberships_wiped_by_v16() {
        let dir = temp_dir();
        let db_path = dir.path().join("launcher.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(V15_FROZEN_SCHEMA).unwrap();
        conn.execute_batch(
            "INSERT INTO hosts (name, address, port, group_id, forward_agent,
                                transport, created_at, updated_at)
             VALUES ('a', '10.0.0.1', 22,
                     (SELECT id FROM host_groups WHERE name = 'prod'), 0, 'ssh',
                     1700000000, 1700000000);
             INSERT INTO host_group_memberships (host_id, group_id)
                 SELECT id, group_id FROM hosts;",
        )
        .unwrap();
        drop(conn);

        // Reproduce the damage 0.17.0 left behind: schema at 16, memberships
        // cascaded away, `hosts.group_id` still holding the assignment.
        let damaged = Connection::open(&db_path).unwrap();
        run_migrations(&damaged, &db_path).unwrap();
        damaged
            .execute_batch(
                "DELETE FROM host_group_memberships;
                 DELETE FROM schema_version;
                 INSERT INTO schema_version (version) VALUES (16);",
            )
            .unwrap();
        drop(damaged);

        let repaired = Connection::open(&db_path).unwrap();
        run_migrations(&repaired, &db_path).unwrap();
        let restored: i64 = repaired
            .query_row(
                "SELECT COUNT(*) FROM host_group_memberships m
                 JOIN hosts h ON h.id = m.host_id AND h.group_id = m.group_id",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(restored, 1, "the repair must re-seed from hosts.group_id");

        // Idempotent: a second pass adds nothing and drops nothing.
        repaired
            .execute_batch(
                "DELETE FROM schema_version; INSERT INTO schema_version (version) VALUES (16);",
            )
            .unwrap();
        run_migrations(&repaired, &db_path).unwrap();
        let again: i64 = repaired
            .query_row("SELECT COUNT(*) FROM host_group_memberships", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(again, 1, "re-running the repair must not duplicate rows");
    }

    #[test]
    fn migration_v15_to_v16_adds_group_defaults_and_nulls_sentinels() {
        // Hand-computed expectations: the untouched row (creation defaults
        // 22/off/ssh) must come back NULL (inheriting), the custom row
        // (2222/on/mosh) must survive verbatim, and the new group columns
        // must accept a write+read round-trip through the store.
        let dir = temp_dir();
        let db_path = dir.path().join("launcher.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(V15_FROZEN_SCHEMA).unwrap();
        conn.execute(
            "INSERT INTO hosts
                (name, address, port, group_id, forward_agent, transport,
                 created_at, updated_at)
             VALUES
                ('untouched', '10.0.0.1', 22,
                 (SELECT id FROM host_groups WHERE name = 'prod'), 0, 'ssh',
                 1700000000, 1700000000),
                ('custom', '10.0.0.2', 2222,
                 (SELECT id FROM host_groups WHERE name = 'prod'), 1, 'mosh',
                 1700000000, 1700000000)",
            [],
        )
        .unwrap();
        drop(conn);

        let migrated = Connection::open(&db_path).unwrap();
        run_migrations(&migrated, &db_path).unwrap();
        let version: i64 = migrated
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, 17);
        for column in [
            "default_username",
            "default_port",
            "default_proxy_jump",
            "default_transport",
            "default_forward_agent",
        ] {
            let has: i64 = migrated
                .query_row(
                    &format!(
                        "SELECT COUNT(*) FROM pragma_table_info('host_groups') \
                         WHERE name = '{column}'"
                    ),
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(has, 1, "host_groups.{column} must exist");
        }
        drop(migrated);

        let store = crate::store::LauncherStore::open(&db_path).unwrap();
        let untouched = store.get_host_by_name("untouched").unwrap().unwrap();
        assert_eq!(untouched.port, None, "default port 22 becomes inheriting");
        assert_eq!(
            untouched.forward_agent, None,
            "default forward_agent off becomes inheriting"
        );
        assert_eq!(
            untouched.transport, None,
            "default transport ssh becomes inheriting"
        );
        let custom = store.get_host_by_name("custom").unwrap().unwrap();
        assert_eq!(custom.port, Some(2222));
        assert_eq!(custom.forward_agent, Some(true));
        assert_eq!(
            custom.transport,
            Some(crate::session_transport::SessionTransport::Mosh)
        );

        // The new group columns round-trip through the store API.
        let prod = store.list_groups().unwrap();
        let prod = prod.iter().find(|g| g.name == "prod").unwrap();
        let updated = store
            .update_group(
                prod.id,
                &crate::store::HostGroupUpdate {
                    default_username: Some(Some("deploy".into())),
                    default_port: Some(Some(2200)),
                    default_proxy_jump: Some(Some("bastion".into())),
                    default_transport: Some(Some(crate::session_transport::SessionTransport::Mosh)),
                    default_forward_agent: Some(Some(true)),
                    ..Default::default()
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(updated.default_username.as_deref(), Some("deploy"));
        assert_eq!(updated.default_port, Some(2200));
        assert_eq!(updated.default_proxy_jump.as_deref(), Some("bastion"));
        assert_eq!(
            updated.default_transport,
            Some(crate::session_transport::SessionTransport::Mosh)
        );
        assert_eq!(updated.default_forward_agent, Some(true));
        // Untouched now inherits the group values end to end.
        let resolved = store.resolve_connection(&untouched).unwrap();
        assert_eq!(resolved.port, 2200);
        assert_eq!(resolved.username.as_deref(), Some("deploy"));
    }

    #[test]
    fn migration_from_v13_creates_both_snippets_and_log_bookmarks() {
        // Guards the #118/#123 merge order: a database jumping straight from v13
        // to the current version in one launch must run both new steps, so both
        // tables exist. A fresh DB (current = 0) exercises the same chain.
        for start in [0_i64, 13] {
            let dir = temp_dir();
            let db_path = dir.path().join("launcher.db");
            let conn = Connection::open(&db_path).unwrap();
            run_migrations(&conn, &db_path).unwrap();
            if start == 13 {
                conn.execute_batch(
                    "DROP TABLE snippets; DROP TABLE log_bookmarks; DROP TABLE command_history;",
                )
                .unwrap();
                set_schema_version(&conn, 13).unwrap();
                run_migrations(&conn, &db_path).unwrap();
            }

            let tables: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master
                     WHERE type='table' AND name IN ('snippets', 'log_bookmarks')",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(tables, 2, "both tables must exist (start={start})");

            let version: i64 = conn
                .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(version, SCHEMA_VERSION);
        }
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
    fn migration_creates_favorites_group_and_membership_table() {
        let dir = temp_dir();
        let db_path = dir.path().join("launcher.db");
        let conn = Connection::open(&db_path).unwrap();
        run_migrations(&conn, &db_path).unwrap();

        // The reserved Favorites group exists exactly once.
        let (count, reserved): (i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), COALESCE(MAX(reserved), 0) FROM host_groups WHERE name = ?1",
                params![FAVORITES_GROUP_NAME],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(reserved, 1);

        // The membership join table exists.
        let table: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table' AND name = 'host_group_memberships'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(table, 1);
    }

    #[test]
    fn migration_backfills_memberships_from_group_id_and_favorite() {
        // Simulate a pre-v11 db: run migrations (creates v11), then drop the
        // membership rows and re-insert legacy-style data, then re-run the v11
        // backfill by clearing and calling it directly.
        let dir = temp_dir();
        let db_path = dir.path().join("launcher.db");
        let conn = Connection::open(&db_path).unwrap();
        run_migrations(&conn, &db_path).unwrap();

        let now = now_ts();
        conn.execute(
            "INSERT INTO host_groups (name, sort_order, created_at) VALUES ('prod', 0, ?1)",
            params![now],
        )
        .unwrap();
        let gid: i64 = conn
            .query_row("SELECT id FROM host_groups WHERE name = 'prod'", [], |r| {
                r.get(0)
            })
            .unwrap();
        conn.execute(
            "INSERT INTO hosts (name, address, port, group_id, favorite, created_at, updated_at)
             VALUES ('h1', '10.0.0.1', 22, ?1, 1, ?2, ?2)",
            params![gid, now],
        )
        .unwrap();
        let hid: i64 = conn
            .query_row("SELECT id FROM hosts WHERE name = 'h1'", [], |r| r.get(0))
            .unwrap();

        // Re-run the backfill (idempotent via INSERT OR IGNORE).
        migrate_v10_to_v11(&conn).unwrap();

        let groups: Vec<i64> = conn
            .prepare(
                "SELECT group_id FROM host_group_memberships WHERE host_id = ?1 ORDER BY group_id",
            )
            .unwrap()
            .query_map(params![hid], |r| r.get(0))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        let fav_id: i64 = conn
            .query_row(
                "SELECT id FROM host_groups WHERE name = ?1",
                params![FAVORITES_GROUP_NAME],
                |r| r.get(0),
            )
            .unwrap();
        assert!(groups.contains(&gid), "host should be in its prod group");
        assert!(
            groups.contains(&fav_id),
            "favourite host should be in Favorites"
        );
    }

    #[test]
    fn migration_does_not_hijack_a_user_group_named_favorites() {
        // A user upgrading from an older schema who already has an ordinary
        // group literally named "Favorites" must keep it — the reserved group
        // is created separately under a distinct name.
        let dir = temp_dir();
        let db_path = dir.path().join("launcher.db");
        let conn = Connection::open(&db_path).unwrap();
        // Build the base tables + advance to v10 so v11 runs against real data.
        conn.execute_batch(V2_SCHEMA).unwrap();
        let now = now_ts();
        conn.execute(
            "INSERT INTO host_groups (name, sort_order, created_at) VALUES (?1, 5, ?2)",
            params![FAVORITES_GROUP_NAME, now],
        )
        .unwrap();
        let user_gid: i64 = conn
            .query_row(
                "SELECT id FROM host_groups WHERE name = ?1",
                params![FAVORITES_GROUP_NAME],
                |r| r.get(0),
            )
            .unwrap();

        migrate_v10_to_v11(&conn).unwrap();

        // The user's group is untouched (not reserved) …
        let user_reserved: i64 = conn
            .query_row(
                "SELECT reserved FROM host_groups WHERE id = ?1",
                params![user_gid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(user_reserved, 0, "user group must not be repurposed");
        // … and a distinct reserved group exists.
        let reserved_id: i64 = conn
            .query_row("SELECT id FROM host_groups WHERE reserved = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_ne!(reserved_id, user_gid);
    }

    #[test]
    fn migration_v13_transport_defaults_ssh() {
        // Since v16 the column is nullable (`NULL` = inherit): a fresh row
        // without an explicit transport stores NULL, and the effective
        // transport is still ssh through the global default.
        let dir = temp_dir();
        let db_path = dir.path().join("launcher.db");
        let conn = Connection::open(&db_path).unwrap();
        run_migrations(&conn, &db_path).unwrap();
        let now = now_ts();
        conn.execute(
            "INSERT INTO hosts (name, address, port, source, sort_order, created_at, updated_at)
             VALUES ('h', '1.2.3.4', 22, 'launcher', 0, ?1, ?1)",
            params![now],
        )
        .unwrap();
        let transport: Option<String> = conn
            .query_row("SELECT transport FROM hosts WHERE name = 'h'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(transport, None);
        migrate_v12_to_v13(&conn).unwrap();
        drop(conn);
        let store = crate::store::LauncherStore::open(&db_path).unwrap();
        let host = store.get_host_by_name("h").unwrap().unwrap();
        assert_eq!(
            store.resolve_connection(&host).unwrap().transport,
            crate::session_transport::SessionTransport::Ssh
        );
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
