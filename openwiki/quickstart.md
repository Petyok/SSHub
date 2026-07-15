# SSHub

SSHub is a keyboard-driven Terminal UI for managing and connecting to SSH hosts. It merges your read-only `~/.ssh/config` with a launcher-managed SQLite database, gives you nested groups, fuzzy search, tags, embedded PTY sessions, SFTP transfers, SSH tunnels, identity/key management, and an audit log — all inside one ratatui application.

This wiki is a practical map for engineers working on the codebase. Start here, then follow the section links for deeper dives.

## What this codebase is

- A Rust (edition 2021) ratatui + crossterm app with a **synchronous 50 ms event loop** — no async runtime.
- **Hybrid host model**: hosts come from `~/.ssh/config` (read-only connection fields) and from `launcher.db` (managed in-app, full CRUD). They merge by name without duplicates.
- **Embedded SSH.** Connections run in an in-app pseudo-TTY (`portable-pty` + `vt100` + `tui-term`). Detach with `Ctrl+D` to return to the dashboard while SSH keeps running; use session tabs (`Ctrl+T`, `Ctrl+[/]`) to manage multiple connections.
- **Native SFTP** via `libssh2` on a background worker thread, with a dual-pane local/remote browser and staged recursive transfers.
- **Native SSH tunnels** spawned as `ssh -N -L/-R/-D` children, with a recent keep-alive/reconnect supervisor and configurable exponential backoff.
- **SQLite persistence**: `launcher.db` holds hosts, groups, identities, tunnels, auth events, and UI state. Schema migrations live in `src/store/migrate.rs`.

## Repository layout

```
/
├── Cargo.toml              # crate config, keyring/ssh2 features
├── Justfile                # build, test, release, install recipes
├── README.md               # user-facing docs, keybindings, install
├── CLAUDE.md               # team workflow, versioning, architecture
├── CHANGELOG.md            # release notes
├── src/
│   ├── main.rs             # CLI entry + db purge subcommand
│   ├── lib.rs              # run(), run_app(), purge_database()
│   ├── config.rs           # AppConfig, TOML load, XDG/env paths
│   ├── app/                # App state, input handling, domain logic
│   ├── app/types.rs        # AppMode, SortMode, HostEntry, settings enums
│   ├── tui/                # ratatui rendering: screens, widgets, theme
│   ├── session/            # embedded PTY sessions + askpass
│   ├── session_log.rs      # opt-in PTY transcript logging
│   ├── tunnel.rs           # TunnelManager + keep-alive reconnect
│   ├── sftp/               # SFTP model, transport, worker
│   ├── ssh/                # ssh config parsing, import/export, agent, probe
│   ├── store/              # LauncherStore + SQLite migrations
│   ├── metadata/           # legacy MetadataDb overlay
│   ├── launcher/           # external-terminal launchers (kitty/ghostty/custom)
│   ├── watcher.rs          # hot-reload file watcher for ssh config
│   ├── keybinds.rs         # user-remappable keybindings
│   └── osinfo/             # remote OS detection + logo widget
├── tests/
│   ├── e2e/                # TestBackend TUI scenario tests
│   ├── smoke/              # binary help/dry-run/config load
│   ├── support/            # FixtureResolver, MockLauncher
│   └── fixtures/           # ssh_config / ssh -G fixtures
└── docs/
    ├── host-sync-design.md # planned P2P sync feature (not implemented)
    └── termius-export-format.md
```

## Build & run

```bash
# Build debug
cargo build

# Run TUI
cargo run

# Dry-run / headless CI check
cargo run -- --dry-run

# Run all tests (unit + integration)
just test

# Equivalent manual runs
cargo test
cargo test --test smoke
cargo test --test e2e
cargo test --test config_load
```

The release flow is codified in `Justfile`: `just release` (feature) / `just release patch` (hotfix). See [`operations/build-test-release.md`](operations/build-test-release.md).

## Data paths

| Resource      | Default path                                                         |
|---------------|----------------------------------------------------------------------|
| Config        | `~/.config/sshub/config.toml`                                        |
| Database      | `~/.local/share/sshub/launcher.db` (+ `-wal`/`-shm`)                 |
| Session logs  | `~/.local/share/sshub/logs/<host-dir>/` (opt-in)                     |
| SSH config    | `~/.ssh/config` (read-only source + import/export target)            |

Override via environment variables: `SSHUB_CONFIG_DIR`, `SSHUB_DATA_DIR`, `SSHUB_SSH_CONFIG`.

Run `sshub db purge --yes-i-am-stupid` to wipe the launcher database. This removes managed hosts, groups, identities, tunnels, and the audit log but **does not** touch `~/.ssh/config`.

## Recently active areas

The current `development` branch (HEAD `3c566a9`) has been focused on two big features:

1. **Tunnel keep-alive / reconnect** (`src/tunnel.rs`, `src/app/tunnels.rs`, `src/tui/screens/tunnels.rs`, `src/config.rs`):
   - Per-tunnel `auto_connect` column toggles keep-alive.
   - Keep-alive tunnels start on app launch and reconnect on unexpected exit.
   - Exponential backoff, jitter, max attempts, stable-uptime threshold configurable under `[tunnel_reconnect]` in `config.toml` and editable in-app with `R`.

2. **Session logging** (`src/session_log.rs`, `src/session/mod.rs`, `src/app/connect.rs`, `src/store/migrate.rs` schema v12):
   - Opt-in capture of embedded PTY output to plaintext files.
   - Global toggle in Settings (`Ctrl+H`) or per-host tri-state `inherit`/`on`/`off`.
   - Audit connect events show the log directory path.
   - Logs capture everything echoed to the terminal, **including passwords** — documented as a security warning.

## Where to go next

- [`architecture/overview.md`](architecture/overview.md) — modules, event loop, render loop, dependency injection
- [`architecture/source-map.md`](architecture/source-map.md) — key files by domain
- [`workflows/connecting.md`](workflows/connecting.md) — embedded SSH sessions, secrets, session logging
- [`workflows/tunnels.md`](workflows/tunnels.md) — tunnel types, keep-alive, reconnect supervisor
- [`workflows/sftp.md`](workflows/sftp.md) — dual-pane SFTP browser, worker thread, file ops
- [`data-models/storage.md`](data-models/storage.md) — SQLite schema, entities, migrations
- [`tui/keybindings.md`](tui/keybindings.md) — keybind system and modes
- [`operations/build-test-release.md`](operations/build-test-release.md) — build, CI, versioning, release
- [`operations/runbook.md`](operations/runbook.md) — env vars, troubleshooting, DB purge, logs
- [`integrations/import-export.md`](integrations/import-export.md) — ssh config hot reload, Termius import/export

## Navigation conventions

- Source paths in this wiki are relative to the repository root.
- Code references link to the file only; line numbers are intentionally omitted because they drift. Use grep to locate symbols.
- `CLAUDE.md` and `README.md` are primary source docs; this wiki is a synthesized navigation layer over them.

## Backlog

- **P2P host sync** — `docs/host-sync-design.md` describes a Shamir-quorum, hash-chained device sync. It is not implemented yet and not referenced in production code.
