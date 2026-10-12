---
type: Architecture
title: Profiles, isolated workspaces, startup picker, legacy migration, and cross-profile transfers
description: How src/profile isolates each profile's databases, config.toml, fallback credentials, logs and tunnel state under profiles/<name>/, how startup picks a profile (silent, picker, --profile, compat mode), how the in-app manager switches profiles safely, how legacy installs migrate, and how the store transfer engine copies or moves entities between profiles.
resource: src/profile/mod.rs
tags: [profiles, storage, startup, migration, transfer, architecture]
---

# Profiles

A profile is an isolated SSHub workspace. Everything profile-owned is reached
through one immutable `ProfilePaths` built at startup (`src/profile/mod.rs`);
subsystems must not rediscover paths from the environment. User-facing keys and
flows: [profiles & themes workflow](../workflows/profiles-themes.md).

## Layout

```text
~/.local/share/sshub/            data root (config::data_dir)
├── state.toml                   ProfileState: profiles [{id, name}], last_used
└── profiles/<name>/             ProfilePaths::root
    ├── .profile-id              stable id, used to adopt orphaned dirs
    ├── launcher.db (+ .bak-v*)  see data-model
    ├── metadata.db
    ├── config.toml
    ├── credentials.json         only when the OS keyring is unavailable
    ├── themes/                  user themes the TUI loads (see themes)
    ├── logs/<host-dir>/         session logs
    └── tunnels/                 detached tunnel PID files
```

- `ProfileRecord { id, name }`: the name is the directory name
  (`validate_profile_name`: non-empty, at most 64 characters, no leading `.`,
  no path separators or control characters); the id (`new_profile_id`, `p` plus
  hex) is stable across renames.
- `ProfilePaths::credential_prefix()` is `profile:<id>:` in profile mode and
  empty in compat mode. `NamespacedPasswordStore` prefixes every keyring or
  `credentials.json` key with it, so identically named hosts in two profiles
  never share a secret (see [secrets](../security/secrets.md)).
- The ssh config source per profile (`ssh_config_path_for_profile`):
  `SSHUB_SSH_CONFIG` / `SSH_LAUNCHER_SSH_CONFIG`, else the profile's
  `[ssh] config_path`, else `~/.ssh/config`.

## Compatibility mode

Setting `SSHUB_DATA_DIR` or `SSHUB_CONFIG_DIR` (or the `SSH_LAUNCHER_*`
fallbacks) makes `resolve_roots` return `compat = true`: `compat_paths` uses the
data override as the profile root and `<config dir>/config.toml`, with no
`profiles/` nesting, no `state.toml`, an empty credential prefix, and
`--profile` rejected. Tests and scripted installs rely on this. The in-app
manager refuses to open in compat mode.

## Startup selection (`resolve_startup`)

1. Compat mode → `Startup::Silent(compat_paths)`.
2. `ensure_layout`: create the data root and `profiles/` (0700); load
   `state.toml`, sweeping half-deleted `.deleting-<id>` dirs; otherwise run the
   legacy migration, adopt existing profile directories (crash between rename
   and state write), or create `default` on a fresh install.
3. `--profile NAME` → that profile (unknown names list the available ones).
4. One profile and no `--manage-profiles` → silent.
5. Several profiles (or `--manage-profiles`): interactive → `Startup::Picker`,
   non-interactive (no terminal, `SSHUB_AUTO_QUIT`, every headless CLI command
   via `main.rs::resolve_cli_profile`) → the last-used profile, silently.

The startup picker (`ProfilePicker::new`, `src/profile/picker.rs`) runs after
the intro splash in `lib.rs::run_picker_loop`, on the embedded `default` theme;
`picker_animation_enabled` honours the last-used profile's reduced-motion
setting. It owns no database: create, rename and delete touch only profile
directories and `state.toml`. `Esc` cancels startup. The launched profile is
recorded as `last_used` (`lib.rs::record_last_used`, skipped in compat mode).

## In-app manager and switching (`src/app/profile_manager.rs`)

`KeyAction::ProfilesManage` (default `Alt+P`) or the Settings row opens
`ProfilePicker::in_app` as `AppMode::ProfilePicker`. A single-profile install
opens straight into "create"; the active profile cannot be renamed or deleted,
and the last profile cannot be deleted.

Choosing a profile sets `App::pending_profile`. After the frame's input,
`lib.rs::apply_pending_profile` first checks `profile_switch_blocker` (live SSH
or local shell sessions, SFTP connections or transfers, a broadcast, running
tunnels or pending reconnects all block), then builds the replacement `App`
with `load_profile_app`, and only on success swaps it in, carrying over the
terminal area and the release-check state. A failure leaves the current
workspace untouched and shows the error in the picker.

`delete_profile` stops the profile's detached tunnels, renames the directory to
`.deleting-<id>`, saves `state.toml`, removes the directory (each step rolls the
previous ones back on failure) and only then best-effort deletes the profile's
`profile:<id>:host:N` / `profile:<id>:identity:N` keyring entries (ids read from
its `launcher.db` beforehand). It never touches external ssh config.
`rename_profile` moves the directory and keeps the id.

## Legacy migration (`src/profile/migrate.rs`)

Installs from before profiles kept `launcher.db`, `metadata.db`,
`credentials.json`, `logs/` and `tunnels/` at the data root and `config.toml`
in the config dir. `run_legacy_migration` takes `.migration.lock` (stale after
10 minutes), **copies** (never moves) those files and their SQLite sidecars into
a staging dir, validates sizes, renames staging to `profiles/default`, re-keys
credentials (`host:{id}` → `profile:{id}:host:{id}` in the keyring and in the
staged `credentials.json`, best-effort), and only then writes `state.toml`.
Legacy files stay, so an interrupted migration retries and a downgrade still
boots.

## Cross-profile transfers

Engine: `src/store/transfer.rs` ([data model](data-model.md#cross-profile-transfer-engine-srcstoretransferrs)).
Front ends: the profile manager (`t` / `T` in the list view arms
`App::pending_transfer`; flow and rendering in `src/app/profile_transfer.rs`:
destination, scope, plan, explicit `y`) and the CLI
(`src/cli/transfer.rs`):

```bash
sshub host transfer --name NAME --to PROFILE [--move] [--yes]
sshub group transfer --name NAME --to PROFILE [--move] [--with-hosts] [--yes]
sshub identity transfer --name NAME --to PROFILE [--move] [--yes]
sshub tunnel transfer <id|label|port|tunnel#ID|:PORT> --to PROFILE [--move] [--yes]
```

Rules enforced by the engine:

- Every plan needs explicit confirmation (`--yes` or the `y/N` prompt; `y` in
  the TUI), even with nothing blocked.
- Name clashes in the destination become `name (copy N)`.
- Hosts with a live session or a running tunnel are planned as `blocked` and
  skipped (the headless CLI can only see running tunnels).
- Destination writes run in one transaction and roll back on the first
  failure; a move deletes from the source in a second transaction only after
  the copy succeeded.
- Identities are always copied; key files and secrets travel as paths and flags
  only (new rows start with `has_password = false`). Groups land flattened at
  top level; hosts land with `source = launcher`.
