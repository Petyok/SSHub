# Storage & data model

SSHub persists launcher-managed data in a single SQLite file (`launcher.db`) under the data directory. Imported `~/.ssh/config` hosts are read-only and re-imported on every launch and config change.

## Database

`src/store/mod.rs::LauncherStore` wraps a `rusqlite::Connection` protected by a `std::sync::Mutex`.

Opening logic:

- Creates parent directory, opens `launcher.db`, restricts directory/file permissions.
- Runs migrations from `src/store/migrate.rs`.
- Seeds a default identity named `Default`.

Legacy `metadata.db` in the same directory is still supported via `src/metadata/`. On first open, the launcher attempts a best-effort migration of legacy metadata; a corrupt/locked legacy db is skipped rather than aborting startup.

## Schema

The canonical schema version is `SCHEMA_VERSION` in `src/store/migrate.rs`. Recent changes:

- **v12** added `hosts.session_logging` (per-host tri-state override) and `auth_events.log_path` (session log directory on connect events).
- **v13** added `hosts.transport` (`ssh` or `mosh`) and `host_metadata.transport` for legacy ssh_config aliases.
- **v11** added the `host_group_memberships` join table for multiple groups per host.
- Earlier versions established `hosts`, `host_groups`, `identities`, `tunnels`, `auth_events`, and `ui_state`.

Core tables (check `src/store/migrate.rs` for exact DDL):

- `hosts` — managed + imported host rows. Key fields: `name`, `label`, `address`, `port`, `group_id`, `identity_id`, `os_icon`, `tags` (JSON), `notes`, `proxy_jump`, `forward_agent`, `remote_command`, `environment`, `sort_order`, `favorite`, `last_connected`, `source` (`launcher` or `ssh_config`), `ssh_config_hash`, `has_password`, `username`, `session_logging`.
- `host_groups` — nested groups: `id`, `name`, `sort_order`, `parent_id`, `default_identity_id`, `reserved`.
- `host_group_memberships` — many-to-many join between hosts and groups.
- `identities` — reusable identity records: `name`, `username`, `private_key`, `certificate`, `sort_order`, `has_password`.
- `tunnels` — `host_id`, `tunnel_type`, `local_port`, `remote_host`, `remote_port`, `label`, `auto_connect`.
- `auth_events` — audit log: `host_name`, `username`, `via`, `status`, `note`, `log_path`, `created_at`.
- `ui_state` — key/value UI state such as collapsed group keys.

## Domain types

`src/store/types.rs` defines the main entities:

- `ManagedHost` — full resolved host with joined group/identity.
- `NewHost` / `HostUpdate` — insert/update struct pair used by forms.
- `HostGroup` / `NewHostGroup` / `HostGroupUpdate` — group entities.
- `Identity` / `NewIdentity` / `IdentityUpdate` — identity entities.
- `Tunnel` / `NewTunnel` — tunnel entities.
- `AuthEvent` — audit log row.

`src/app/types.rs` adds UI-level types:

- `HostEntry` — either a `Legacy` ssh_config alias or a `Managed` row.
- `HostGroupSection` / `NavRow` / `VisualRow` — tree-list rendering data.
- `SortMode` — the host list sort order.
- `AuditFilter` / `AuditRange` — audit tab filters.

## Host sources

`HostSource` distinguishes:

- `Launcher` — managed in DB with full CRUD.
- `SshConfig` — imported from `~/.ssh/config`; read-only connection fields with metadata overlay.

`src/app/mod.rs::reload_hosts()` merges the two sources by name. Launcher rows win on name collision. Imported rows carry a hash of the ssh-config source so the sync logic can detect changes and remove stale aliases.

## Credentials

Stored passwords and key passphrases live in the OS keyring, not in the database. `src/credentials.rs` provides the `PasswordStore` abstraction. `keyring` is configured with platform-native features in `Cargo.toml`.

The `has_password` flag in `hosts` / `identities` only records whether a secret is expected to exist in the keyring.

## Audit log

`auth_events` records connection and tunnel events. Each row stores:

- `host_name`, optional `username`, optional `via` (e.g. `tunnel`, `embedded`),
- `status` (`launched`, `ok`, `fail`, …),
- `note` (human-readable detail),
- optional `log_path` (session log directory for connect events),
- `created_at` timestamp.

The audit tab shows the selected event's `note` (and log path when present) on a detail line above the table — not in per-row columns. Events sort `ORDER BY created_at DESC, id DESC`. Filters by status and date range.

## UI state

Small persistent UI settings such as the collapsed group set are stored as JSON values in `ui_state`.

## Migrations

Additions follow this pattern:

1. Append a migration to `src/store/migrate.rs`.
2. Bump `SCHEMA_VERSION`.
3. Add the corresponding field to `src/store/types.rs` and `src/app/*` forms/screens.
4. Add/update tests in `src/store/*` and `src/app/tests/`.

## What to watch when changing storage

- `src/store/migrate.rs` — any DDL change must be idempotent and preserve existing user data.
- `src/store/types.rs` — keep insert/update structs aligned with table columns.
- `src/store/hosts.rs`, `identities.rs`, `tunnels.rs` — SQL queries and transaction boundaries.
- `src/app/host_form.rs`, `host_detail.rs` — new fields need form fields and host detail rendering.
- `src/store/mod.rs` — `open_in_memory()` is used heavily in tests; migrations must run without a real filesystem.

Relevant tests: `src/store/*` unit tests, `src/app/tests/host_form.rs`, `src/app/tests/identity_group.rs`, `tests/e2e/mod.rs`.
