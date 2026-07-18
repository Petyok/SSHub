---
type: Data Model
title: Data Model & Storage — launcher.db, metadata.db, config, and hybrid hosts
description: SSHub persists state in two SQLite databases (launcher.db for managed hosts/groups/identities/tunnels/audit, metadata.db for legacy ssh_config hosts), a TOML config file, and merges managed hosts with read-only ~/.ssh/config aliases through the HostResolver abstraction.
resource: src/store/mod.rs
tags: [storage, sqlite, schema, config, hosts, architecture]
---

# Data Model & Storage

## Two SQLite databases

| DB | Path | Contents |
|---|---|---|
| `launcher.db` | `$SSHUB_DATA_DIR/launcher.db` | Managed hosts, host groups, identities, tunnels, auth events (audit), UI state |
| `metadata.db` | `$SSHUB_DATA_DIR/metadata.db` | `host_metadata` for **legacy** ssh_config-only hosts: tags, description, environment, favorite, last_connected, session_logging, transport |

The split is historical: `metadata.db` predates the launcher database and still backs per-alias metadata for hosts that exist only in `~/.ssh/config`. `LauncherStore`'s own docs admit the "MVP still uses MetadataDb" — treat the overlap (e.g. `session_logging` and `transport` exist on both `ManagedHost` and `HostMetadata`) as an acknowledged incomplete migration, not a design to copy. A best-effort one-way import from legacy `metadata.db` runs on fresh `launcher.db` creation (`src/store/migrate.rs`); a locked legacy DB must never brick launch.

## launcher.db schema (`src/store/`)

`LauncherStore` wraps a `Mutex<Connection>` (rusqlite, bundled SQLite) with `PRAGMA foreign_keys=ON` and `busy_timeout=5000`. Migrations use a custom `schema_version` table — base v2 schema plus stepwise `migrate_vN_to_vN+1` functions up to **SCHEMA_VERSION 13**, all in one transaction; column adds are guarded with `pragma_table_info` so re-runs are safe.

Tables:

- `hosts` — managed hosts plus imported ssh_config rows; key columns: `source` (`launcher` | `ssh_config`), `ssh_config_hash` (drift detection), `transport`, `session_logging`, `os_icon`.
- `host_groups` — nested via `parent_id`; a `reserved` flag marks the built-in **Favorites** group (found by flag, never by name — a user's own group named "Favorites" is never repurposed).
- `host_group_memberships` — M:N; a host can belong to several groups at once.
- `identities` — seeded with a "Default" identity.
- `tunnels` — tunnel definitions (see [tunnels](../workflows/tunnels.md)).
- `auth_events` — the audit log, including `log_path` for session-connect events when logging is enabled.
- `ui_state` — collapsed groups, ui_zoom.

CRUD is split across `src/store/hosts.rs`, `identities.rs`, `tunnels.rs`; DTOs (`ManagedHost`, `HostSource`, `Tunnel`, `AuthEvent`, `NewHost`, …) live in `src/store/types.rs`. Secrets are **never** in SQLite — only `has_password` flags; actual secrets are in the OS keyring (see [secrets](../security/secrets.md)).

`sshub db purge --yes-i-am-stupid` deletes `launcher.db` and its sidecars (`src/lib.rs::purge_database`); `~/.ssh/config` is untouched and imported hosts reappear on next launch. Keyring entries are deliberately orphaned.

## Hybrid host model (`src/hosts/loader.rs`, `src/ssh/`)

`load_merged_hosts` produces the unified host list:

1. DB hosts with `source=launcher` (full CRUD).
2. DB hosts with `source=ssh_config` — rows synced from the user's ssh config by `sync_ssh_config_hosts` (`src/ssh/import.rs`), keyed by `ssh_config_hash`, refreshed on `sshub sync` / import; launcher rows are never overwritten.
3. Remaining unresolved ssh_config aliases surface as `HostEntry::Legacy { host, meta }`, with metadata from `metadata.db`.

Resolution goes through the `HostResolver` trait; `SshConfigResolver` (`src/ssh/resolver.rs`) parses `Host` aliases itself (following `Include` directives, depth-capped at 16) and shells out to `ssh -F <cfg> -G <alias>` for effective options. The user's own `~/.ssh/config` is never written — export renders to `exported.conf` with an atomic write and `.bak` (`src/ssh/export.rs`), and `conf_val` flattens CR/LF so a host field can't inject a `Host *` stanza.

Hosts, groups, favorites, and identities as user-facing concepts: [domain/hosts-identities](../domain/hosts-identities.md).

## Configuration (`src/config.rs`)

`~/.config/sshub/config.toml` (created on first run) sections:

- `[terminal]` — `launcher` = `kitty` | `ghostty` | custom command template (see [integrations](../integrations/external-terminals.md))
- `[appearance]` — opaque background, OS logos, quit confirmation, startup animation
- `[session_logging]` — `enabled`, `max_file_bytes` (default 10 MiB), `retention_files` (50)
- `[tunnel_reconnect]` — `max_attempts`, `initial_delay_ms`, `max_delay_ms`, `stable_secs`, `jitter_ratio` (consumed by `config::tunnel_backoff_delay`; see [tunnels](../workflows/tunnels.md))
- `[keybinds]` — user rebinds from the Ctrl+K editor (`src/keybinds.rs`)

`save_config` deep-merges through `toml_edit`, preserving comments and unknown keys. Env overrides: `SSHUB_CONFIG_DIR`, `SSHUB_DATA_DIR`, `SSHUB_SSH_CONFIG` (with `SSH_LAUNCHER_*` legacy fallbacks), plus `SSHUB_DRY_RUN` / `SSHUB_AUTO_QUIT` for headless runs. A staged-rename migration moves `~/.config/ssh-launcher` → `~/.config/sshub` via a `.migrating` sibling so a crash mid-copy can't freeze a partial config.

## Change guidance

- Adding a column or table: bump `SCHEMA_VERSION`, add a `migrate_vN_to_vN+1` step, and prefer `pragma_table_info` guards — tests open in-memory stores (`LauncherStore::open_in_memory`) and run all migrations from scratch.
- In-memory tests point the launcher path into a temp dir so the legacy metadata import can't pick up a stray `./metadata.db` from the CWD.
- The [file watcher](overview.md#file-watcher) only reloads hosts; config.toml changes require restart.
