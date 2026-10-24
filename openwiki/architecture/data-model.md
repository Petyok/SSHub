---
type: Data Model
title: Data Model & Storage, launcher.db v21, metadata.db, config.toml, and hybrid hosts
description: SSHub persists each profile's state in two SQLite databases (launcher.db at schema v21 for managed hosts, groups and their connection defaults, identities, tunnels, audit, snippets, log bookmarks, opt-in command history and saved split-view workspaces; metadata.db for legacy ssh_config aliases), snapshots launcher.db before every migration, keeps settings in config.toml, and merges managed hosts with read-only ~/.ssh/config aliases through HostResolver.
resource: src/store/migrate.rs
tags: [storage, sqlite, schema, migrations, config, hosts, architecture]
---

# Data Model & Storage

## Profile-owned storage

Every path below is resolved once into a `ProfilePaths` (`src/profile/mod.rs`)
and passed to stores, config, resolver, credentials, session logging, tunnels,
watcher and CLI context; no subsystem rediscovers paths from the environment.
Normal installs keep `state.toml` at the data root and one directory per
profile under `profiles/<name>/`; `SSHUB_DATA_DIR` / `SSHUB_CONFIG_DIR` (or
their `SSH_LAUNCHER_*` fallbacks) select compatibility mode, which uses the
override directories verbatim. Layout, picker and migration:
[profiles](profiles.md).

## Two SQLite databases

| DB | Path (profile mode) | Contents |
|---|---|---|
| `launcher.db` | `profiles/<name>/launcher.db` | Managed and imported hosts, groups, memberships, identities, tunnels, audit (`auth_events`), UI state, snippets, log bookmarks, command history, workspaces |
| `metadata.db` | `profiles/<name>/metadata.db` | `host_metadata` for **legacy** ssh_config-only aliases: tags, description, environment, favorite, last_connected, session_logging, transport, ping |

The split is historical: `metadata.db` predates `launcher.db` and still backs
per-alias metadata for hosts that exist only in the ssh config. `LauncherStore`'s
own doc comment still says the "MVP still uses `MetadataDb`"; treat the overlap
(`session_logging`, `transport`, `ping` exist on both `ManagedHost` and
`HostMetadata`) as an unfinished migration, not a pattern to copy. On a fresh
`launcher.db` (schema version 0) a best-effort one-way import copies
`host_metadata` rows into `hosts` as `source = 'ssh_config'`
(`migrate_legacy_metadata`); a locked or corrupt `metadata.db` is skipped with a
warning so it can never brick startup.

## launcher.db schema (`src/store/`)

`LauncherStore` wraps a `Mutex<Connection>` (rusqlite, bundled SQLite). Opening
creates the parent directory 0700 and the file 0600 (`secure_fs`), runs
migrations and seeds the "Default" identity. `run_migrations`
(`src/store/migrate.rs`):

1. `PRAGMA foreign_keys = ON`, `PRAGMA busy_timeout = 5000`.
2. **Pre-migration snapshot**: when the stored version is between 1 and
   `SCHEMA_VERSION - 1`, `backup_before_migration` writes
   `launcher.db.bak-v<from>-<unix-ts>` next to the database with
   `VACUUM INTO` (consistent even with WAL readers; cannot run inside a
   transaction, so it runs first) and `prune_backups` keeps the newest three
   (`KEPT_BACKUPS = 3`). Best-effort: a failed snapshot only warns.
3. One transaction: the base v2 schema (`CREATE TABLE IF NOT EXISTS`), then
   every `migrate_vN_to_vN+1` step below the current version, then the legacy
   metadata import (fresh DB only), then `schema_version` is set to
   **`SCHEMA_VERSION = 21`**. Column adds are guarded with `pragma_table_info`
   so re-runs are safe.

Migration history worth knowing:

| Version | Change |
|---|---|
| v3 | `has_password` on `identities` and `hosts` |
| v4 | `auth_events` table; `hosts.ping_ms` |
| v5, v7 | `hosts.username`, `hosts.environment` |
| v6 | `tunnels` table |
| v8 | `ui_state` key/value table |
| v9, v10 | `host_groups.default_identity_id`, `host_groups.parent_id` (nesting) |
| v11 | `host_groups.reserved` + `host_group_memberships`; reserved Favorites group created, memberships backfilled from `group_id` and `favorite` |
| v12 | `hosts.session_logging`, `auth_events.log_path` |
| v13 | `hosts.transport` |
| v14 | `snippets` table |
| v15 | `log_bookmarks` table |
| v16 | `command_history` table; `host_groups.default_username`, `default_port`, `default_proxy_jump`, `default_transport`, `default_forward_agent`; `hosts` rebuilt so `port`, `forward_agent`, `transport` are nullable (`NULL` = inherit), with old creation defaults (22 / off / `ssh`) migrated to `NULL` |
| v17 | Repair: re-seed `host_group_memberships` from `hosts.group_id` and the reserved Favorites group |
| v18 | `hosts.ping` (per-host ping override: `NULL` inherit, 1 on, 0 off) |
| v19 | `hosts.secret_lookup` (one `Attribute=value` Secret Service spec) |
| v20 | `workspaces` table (saved split-view layouts) |
| v21 | `hosts` and `identities` rebuilt with `AUTOINCREMENT`: ids are never reused, so a new host can no longer adopt the keyring secret (`host:<id>` / `identity:<id>`) a deleted one left behind. Child rows (memberships, tunnels, command history, host and group identity references) are copied out and put back around the `DROP TABLE` |

The v16 and v21 rebuilds carry the same hazard: `DROP TABLE hosts` fires every
child `ON DELETE CASCADE`, and 0.17.0 emptied memberships, tunnels and command
history that way. Rebuilds now copy those child rows into temp tables and put
them back; v17 repairs databases already damaged (only the primary group is
recoverable). This is why the pre-migration snapshot exists.

Tables:

- `hosts`: `id` (`AUTOINCREMENT`), `name` (unique), `label`, `address`, `port`, `group_id`
  (primary group, kept in sync with memberships), `identity_id`, `os_icon`,
  `tags` (JSON), `notes`, `proxy_jump`, `forward_agent`, `remote_command`,
  `sort_order`, `favorite`, `last_connected`, `source` (`launcher` |
  `ssh_config`), `ssh_config_hash`, timestamps, `has_password`, `ping_ms`,
  `username`, `environment`, `session_logging`, `transport`, `ping`,
  `secret_lookup`. Writes refuse a `name`, `address` or `username` starting with
  `-` (`reject_option_like`, see [secrets](../security/secrets.md#input-safety)).
  Stricter address / login checks (the destination allowlist,
  `ssh::destination_problem`: login and host-name characters only) run in the
  host form, `sshub host add|edit`, importers and at every point that builds ssh
  argv, not in the store.
- `host_groups`: nested via `parent_id`; `reserved` marks the built-in
  **Favorites** group (found by flag, never by name: a user's own "Favorites"
  group is never repurposed, the reserved one is renamed `Favorites (2)` on
  clash); `default_identity_id` plus the five v16 `default_*` columns.
- `host_group_memberships`: M:N, primary key `(host_id, group_id)`, both sides
  cascade.
- `identities`: `id` (`AUTOINCREMENT`), `name`, `username`, `private_key`, `certificate`,
  `has_password`, `sort_order`; seeded with "Default".
- `tunnels`: tunnel definitions, `host_id` cascades (see
  [tunnels](../workflows/tunnels.md)).
- `auth_events`: the audit log, columns `id`, `host_name`, `username`, `via`,
  `status`, `note`, `created_at`, `log_path`. `via` distinguishes connect
  paths; `tunnel`, `agent`, `exec` and `broadcast` have their own audit filter.
- `ui_state`: key/value (`collapsed_groups`, `ui_zoom` with the legacy `name_zoom` as fallback, the SFTP hidden-files flag).
- `snippets`: `name`, `command`, `description`, `tags` (JSON), timestamps
  (`src/store/snippets.rs`).
- `log_bookmarks`: `host_dir`, `file_name`, `line`, `name`, `created_at`. Keyed
  by the on-disk log location rather than a host id, so a bookmark survives
  deleting and re-adding a host (`src/store/log_bookmarks.rs`).
- `workspaces`: `name` (unique), `layout` (the pane tree as JSON: saved hosts
  by name and local shells per pane), timestamps (`src/store/workspaces.rs`).
  See [sessions & SFTP](../workflows/sessions-sftp.md).
- `command_history`: **local-only**, opt-in: `host_id` (cascade), `command`
  (1 to 4096 bytes), `last_used`, `use_count`, `created_at`,
  `UNIQUE(host_id, command)`, index `command_history_host_recent`. Written by
  `record_command` only for managed hosts, only when
  `[command_history].enabled` is on, and only for commands
  `command_safety::classify_command` calls safe; the per-host limit is capped at
  `MAX_HOST_HISTORY = 2000`. The classifier is best-effort, so persisted command
  text may still contain a secret it did not recognise.

CRUD is split across `src/store/hosts.rs`, `identities.rs`, `tunnels.rs`,
`snippets.rs`, `log_bookmarks.rs`, `command_history.rs`, `workspaces.rs` and
`group_defaults.rs`; DTOs (`ManagedHost`, `HostSource`, `Tunnel`, `AuthEvent`,
`NewHost`, `Snippet`, `LogBookmark`, ...) live in `src/store/types.rs`.
Credentials never enter SQLite (only `has_password` flags and the
`secret_lookup` spec); see [secrets](../security/secrets.md).

### Group-inherited connection defaults (`src/store/group_defaults.rs`)

Six connection fields inherit: identity, username, port, ProxyJump, transport,
agent forwarding. `LauncherStore::resolve_connection` returns a
`ResolvedConnection` per host: the host's own value, else the best group
default, else the global default (`DEFAULT_PORT = 22`, `ssh`, forwarding off;
identity, username and ProxyJump stay unset). "Best" is most-specific-wins over
the ancestor chains of every membership: smallest chain distance, then the
deeper group, then the primary group's chain, then the smaller group id. Tags
and favorites never inherit. Connect paths must build argv from the resolved
values (`app::util::resolved_session_argv`). `ResolvedConnection` also carries
`jump_argv` (the `-J` value, with a hop that exists only in SSHub expanded to
`user@address:port` by `expand_jump`) and `jump_error` (set when ProxyJump is
not valid hops or the destination is outside the allowlist: a caller that runs
ssh must refuse rather than connect without the bastion).

### Cross-profile transfer engine (`src/store/transfer.rs`)

`TransferSnapshot::capture` and `DestIndex::capture` copy plain DTOs out of the
source and destination stores, so planning never holds a store lock and applying
never holds two (the `LauncherStore` mutex is not reentrant).
`build_transfer_plan` / `apply_transfer_plan_with_secrets` copy or move hosts,
groups, identities, tunnels and audit events; clashes rename to `name (copy N)`
(`unique_dest_name`). A copy leaves secrets alone: copied hosts and identities
start with `has_password = false` (a host keeps its `secret_lookup` spec). A
**move** first commits the destination rows, then carries each moved host's
password and each moved identity's passphrase from the source credential store
into the destination one (`carry_secrets`, under the new ids, flagging the new
row `has_password`), and only then deletes the source rows. A row whose secret
cannot be read, written or flagged stays in the source (its destination copy is
removed again) and is listed in `TransferResult::carry_failures`; the caller
then runs `purge_source_secrets` on the source store for what was deleted
(failures surface as a warning). Callers: the profile manager, the guided
migration and `sshub <entity> transfer`
([profiles](profiles.md#cross-profile-transfers)).

### Destructive purge

`sshub [--profile NAME] db purge --yes-i-am-stupid` deletes the profile's
`launcher.db` and its `-wal`, `-shm`, `-journal` sidecars
(`src/lib.rs::purge_profile_database`). `~/.ssh/config` is untouched and
imported hosts reappear on next launch; keyring entries are orphaned.

## Hybrid host model (`src/hosts/loader.rs`, `src/ssh/`)

`load_merged_hosts` produces the unified host list:

1. DB hosts with `source = launcher` (full CRUD).
2. DB hosts with `source = ssh_config`: rows synced from the ssh config by
   `sync_ssh_config_hosts` (`src/ssh/import.rs`), keyed by `ssh_config_hash`;
   launcher rows are never overwritten. The fingerprint ends in
   `|proxycmd=yes` or `|proxycmd=no` (the only place the sync keeps whether
   `ssh -G` printed a `ProxyCommand`; the network map and test connect read it
   with `ssh_config_hash_has_proxy_command`). Adding that suffix changed every
   stored hash, so the first sync after the upgrade rewrites each ssh_config row
   once; tags, notes and favorites are kept by the upsert.
3. Remaining ssh_config aliases with no DB row surface as
   `HostEntry::Legacy { host, meta }`, with metadata from `metadata.db`.

Resolution goes through the `HostResolver` trait. `SshConfigResolver`
(`src/ssh/resolver.rs`) lists `Host` aliases itself (unquoting them the way
OpenSSH does, following `Include` up to depth 16) and shells out to
`ssh -F <cfg> -G <alias>` for effective options. The
ssh config is never written: export renders `exported.conf` with an atomic
write and `.bak` (`src/ssh/export.rs`). Export refuses a host it cannot render
safely (`token`: whitespace, control, quote, `#` or, for the `Host` line,
pattern characters `*?!,`; a leading `-`; an invalid ProxyJump), doubles `%` and
quotes key paths with spaces, so a host field cannot inject a `Host *` stanza or
a `ProxyCommand`.

Hosts, groups, favorites and identities as user-facing concepts:
[domain/hosts-identities](../domain/hosts-identities.md).

## Configuration (`src/config.rs`)

`profiles/<name>/config.toml` (compat mode: `<config dir>/config.toml`) is
created 0600 on first load. Sections of `AppConfig`:

- `[appearance]`: `disable_animation` (reduced motion), `confirm_quit`, `identity_columns`, `os_logo`,
  `transparent_sshub_background`, `transparent_session_background`,
  `active_theme` (default `"default"`, see [themes](themes.md)).
- `[session_logging]`: `enabled` (false), `max_file_bytes` (10 MiB),
  `retention_files` (50).
- `[command_history]`: `enabled` (false), `max_entries_per_host` (200, cap
  2000).
- `[tunnel_reconnect]`: `max_attempts` (12, 0 = unlimited), `initial_delay_ms`
  (1000), `max_delay_ms` (60000), `jitter_ratio` (0.25), `stable_secs` (5).
- `[keybinds]`: rebinds from the `Ctrl+K` editor (`src/keybinds.rs`).
- `[clipboard]`: `relay_from_pty` (true), the OSC 52 relay.
- `[ping]`: `enabled` (true).
- `[updates]`: `check` (true), the startup release check.
- `[ssh]`: `config_path`, a per-profile ssh config source (`~` expanded);
  `SSHUB_SSH_CONFIG` / `SSH_LAUNCHER_SSH_CONFIG` still win.

`save_config_at` deep-merges through `toml_edit` (comments and unknown keys
survive) and writes atomically via a 0600 temp file. Load runs one-shot
keybind migrations and drops retired keys (`RETIRED_KEYS`: only
`appearance.opaque_background`), persisting once. Legacy `terminal` /
`launch_command` keys from before 0.10.0 are simply ignored. Env overrides:
`SSHUB_CONFIG_DIR`, `SSHUB_DATA_DIR`, `SSHUB_SSH_CONFIG` (each with an
`SSH_LAUNCHER_*` fallback; empty values are ignored), plus `SSHUB_DRY_RUN` /
`SSHUB_AUTO_QUIT` for headless runs. `config_dir()` / `data_dir()` migrate the
old `~/.config/ssh-launcher` and `~/.local/share/ssh-launcher` trees when no
override is set; `config_dir_path()` resolves without side effects (used by
`sshub theme`).

## Change guidance

- Adding a column or table: bump `SCHEMA_VERSION`, add a `migrate_vN_to_vN+1`
  step with a `pragma_table_info` guard, and never `DROP` a parent table without
  carrying its cascading children across. Tests open in-memory stores
  (`LauncherStore::open_in_memory`) that run all migrations from scratch, and
  `migrate.rs` has upgrade tests from hand-written older schemas.
- `open_in_memory` points the launcher path into a temp dir so the legacy
  metadata import cannot pick up a stray `./metadata.db` from the CWD.
- The [file watcher](overview.md#file-watcher) reloads hosts only;
  `config.toml` edits made outside SSHub need a restart.
