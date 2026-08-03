# Isolated profiles with startup picker (#17)

## Problem

SSHub currently resolves config, data, SSH config, credentials, logs, and
tunnel state independently. A profile needs to select one coherent workspace
before any of those resources are opened. Single-profile installations must
retain current startup behavior; multiple profiles must remain isolated.

## Decisions

- Resolve one immutable `ProfilePaths` value at startup and pass it to every
  subsystem. No store, config, resolver, credential, log, or tunnel code may
  rediscover profile paths from process-global environment state.
- Store profiles under the data root. Each profile owns `launcher.db`,
  `metadata.db`, `config.toml`, fallback credentials, session logs, and tunnel
  runtime state.
- Keep `state.toml` at the data-root level. It records profile names, stable
  profile IDs, and the last-used profile.
- Keep audit data per profile. A merged audit view is out of scope for v1.
- Profile switching is restart-only for v1.
- Use a standalone startup picker, not an `AppMode`; `App` assumes stores and
  other dependencies already exist.
- Add `--manage-profiles` so users can open the picker when only one profile
  exists. `--profile NAME` bypasses the picker.
- Make `--profile` available to both TUI and headless CLI commands so commands
  cannot accidentally operate on `default`.

## Layout

```text
~/.local/share/sshub/
├── state.toml
└── profiles/
    ├── default/
    │   ├── launcher.db
    │   ├── metadata.db
    │   ├── config.toml
    │   ├── credentials.json
    │   ├── logs/
    │   └── tunnels/
    └── work/
        └── ...
```

Profile config may select an SSH config source:

```toml
[ssh]
config_path = "~/.ssh/config"
```

Resolution order is `SSHUB_SSH_CONFIG`, legacy SSH config override, profile
config, then `~/.ssh/config`. External SSH config files are not copied or
modified by profile creation.

## Path model

Add a profile module containing state, validation, and paths. The resolved
object should contain the profile name/ID, root, data/config directories,
database paths, and resolved SSH config path.

Refactor these entry points to accept resolved paths:

- `App::new` and on-disk dependency construction
- `CliContext::bootstrap`
- config load/save
- `SshConfigResolver`
- credentials, session logging, tunnel runtime, and database purge
- config watcher setup

Environment overrides remain compatibility mode. When `SSHUB_DATA_DIR` or
`SSHUB_CONFIG_DIR` is set, use that directory directly as the selected root and
skip profile discovery/migration. Legacy `SSH_LAUNCHER_*` fallbacks retain
current behavior. This preserves tests that point both overrides at a temp
directory and prevents accidental `profiles/default` nesting.

## Startup flow

```text
parse global flags
  -> initialize terminal
  -> render intro splash
  -> select profile, if required
  -> load selected profile config
  -> construct App with ProfilePaths
  -> run dashboard
```

Selection rules:

- No profiles: create and select `default` without showing picker.
- One profile: select silently unless `--manage-profiles` was supplied.
- More than one profile: show picker after splash.
- `--profile NAME`: select directly and report available names on failure.
- Initial cursor: last-used profile, falling back to first profile.
- Enter launches; arrow keys and number keys select.
- Create, rename, and delete are picker actions.
- Escape cancels startup cleanly.

## Profile state and safety

Use stable IDs in addition to display names so credential namespaces survive a
rename. Validate names as safe path components: non-empty, trimmed, no path
separators or dot components, and bounded length.

Profile rename must reject an existing destination, rename the directory,
update state atomically, and preserve the stable ID. Profile deletion requires
confirmation, refuses to delete the final profile, never touches external SSH
config, and handles profile-owned keyring credentials explicitly.

Profile credential keys must include the stable profile ID (or an equivalent
namespace); host names alone are insufficient because separate profiles may
contain identically named hosts.

## Migration

When the new profile layout is absent and legacy SSHub data exists, migrate the
old top-level files into `profiles/default/`:

- `launcher.db`, `metadata.db`, SQLite sidecars
- fallback credentials, logs, and tunnel state
- old `config.toml` from the config root

Use a migration lock and a staging directory. Copy and validate before renaming
the staging directory into `profiles/default`; write `state.toml` only after the
profile is complete. Leave legacy files intact on first migration and record a
marker so interrupted migration can be retried without data loss. A config-only
or database-only legacy install should also produce a usable default profile.

## Picker management

The picker is a small pre-`App` TUI component. It owns no launcher database;
profile CRUD operates on profile directories and `state.toml` only. Profile
resources are opened after selection, avoiding leaked connections and making
unknown-profile errors cheap.

## CLI behavior

Support both `--profile NAME` and `--profile=NAME` before the TUI or command:

```text
sshub --profile work
sshub --profile work host list
sshub --profile personal audit list
sshub --profile work db purge --yes-i-am-stupid
```

Global parsing must happen before subcommand bootstrap. The selected profile is
passed into `CliContext`; CLI commands must not call global path helpers to
choose another profile.

## Implementation phases

1. Add profile state/path models, validation, atomic persistence, and tests.
2. Add explicit path-aware config, store, resolver, and runtime constructors.
3. Add crash-safe legacy migration into `profiles/default`.
4. Add global `--profile` and `--manage-profiles` parsing.
5. Refactor terminal startup so picker renders between splash and `App`.
6. Implement picker CRUD and last-used persistence.
7. Namespace credentials and route logs, tunnels, purge, watcher, and CLI
   operations through `ProfilePaths`.
8. Update README, in-app help, changelog, and OpenWiki data-path/architecture
   docs.

## Acceptance criteria

- Single-profile startup has no picker or added friction.
- Multiple profiles never share profile-owned databases, config, logs, tunnel
  state, or fallback credentials.
- `--profile` works for TUI and headless commands.
- Unknown profiles fail before opening a profile database.
- Last-used profile controls picker cursor position.
- Rename/delete failures cannot corrupt `state.toml`.
- Interrupted migration is retryable and does not lose legacy data.
- Existing directory override tests continue using exact override paths.
- Audit queries remain profile-local.
- Unit tests cover state, validation, migration, path isolation, and CLI
  selection; TestBackend smoke coverage covers picker rendering and input.
