# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Workflow rules

- **Commit frequently.** After completing each logical unit of work (a bug fix, a feature, a refactor pass), create a commit immediately. Do not accumulate large uncommitted diffs across multiple tasks.
- **Branch model.** `main` is stable (releases + tags); `development` is the integration branch; features go on `feature/*` branches cut from `development`. Flow: `feature/* → development → main`. Releases happen by merging `development → main`, bumping the version + CHANGELOG, and pushing a `vX.Y.Z` tag (the release workflow builds binaries and publishes to crates.io).
- **Delete merged branches.** The repo has "Automatically delete head branches" enabled, so merging a PR on GitHub removes the branch (keep the "Delete branch" box checked). For a local/CLI merge, delete it yourself right after: `git branch -d <branch>` and `git push origin --delete <branch>`. Never leave merged branches lingering.

## Versioning (`vX.Y.Z`)

Odometer scheme — each field rolls 0–9 and carries:

- **Z (patch)** — bump on **every commit to `development`**: `just bump patch`. It's the running odometer counter within a dev cycle **and** the version a hotfix release ships as-is.
- **Y (minor)** — bump when **merging `development → main`** for a feature release; this resets Z to 0: `just bump minor`. A minor release is `X.Y.0`.
- **X (major)** — bump **manually** for a milestone, or automatically by carry when the odometer rolls over (`0.9.9 + patch → 1.0.0`): `just bump major`.

`main` is **not** always `X.Y.0`: feature releases land as `X.Y.0`, but hotfix (patch) releases publish `development`'s current `X.Y.Z` unchanged (see `just release patch` below).

`just bump <patch|minor|major>` edits `Cargo.toml` + `Cargo.lock` with carry (`0.4.9 + patch → 0.5.0`). Only versions carried by `main` when a `vX.Y.Z` tag is pushed get published to crates.io (see the release workflow).

The patch bump is automated by a tracked `pre-commit` hook (`.githooks/pre-commit`) that runs `just bump patch` on every commit **to `development`** (skipped on other branches and during merges). Git hooks aren't shared on clone, so enable them once per checkout: `just setup-hooks` (sets `core.hooksPath .githooks`).

Releasing is one command, run from a clean `development`:

- **`just release`** (or `just release minor`) — feature release. Merges `development → main`, runs `just bump minor`, commits, tags `vX.Y.0`, pushes.
- **`just release patch`** — **hotfix**. Merges `development → main` and tags/publishes `development`'s **current** `vX.Y.Z` with **no bump**, so a fix can reach `main` + crates.io without pretending to be a new minor.

Both then tag (the tag triggers the release workflow → binaries + crates.io) and fast-forward `development` back to the released commit so both branches share a baseline and the next release merges cleanly. Pushing to protected `main` relies on the owner's admin bypass.

`just release patch` ships **whatever `development` currently holds** — it's the fast path when `development` == what you want on `main`. If `development` carries unreleased work you don't want in the hotfix, land the fix on `development` alone first (or handle the cherry-pick manually) before releasing.

## Build & test commands

```bash
# Build
cargo build

# Run all tests (unit + integration)
just test

# Equivalent manual:
cargo test                         # unit tests in src/
cargo test --test smoke            # binary smoke: help, dry-run, headless quit
cargo test --test e2e              # TUI scenarios via TestBackend
cargo test --test config_load      # config.toml create/load

# Run specific e2e test
cargo test --test e2e host_crud

# Dry-run (no TUI, safe for CI)
cargo run -- --dry-run
```

## Architecture

**Stack:** ratatui 0.30 + crossterm (TUI), portable-pty + vt100 (embedded SSH sessions via `tui-term`; upstream vt100 0.16, no vendored fork), nucleo (fuzzy search), rusqlite/bundled (SQLite), notify (file watcher), serde + toml + toml_edit (config). No async runtime — synchronous event loop with `crossterm::event::poll` at 50ms intervals. File watcher runs on a separate thread, sends events via `std::sync::mpsc::Receiver`.

**Entry point:** `src/main.rs` (binary) → `src/lib.rs` (`run_app()`) → `App::new()` + event loop. `AppDeps` struct enables dependency injection for tests (resolver, metadata store, launcher store, terminal launcher).

**Hybrid host sources (R1):** Hosts come from two origins — `launcher` (managed in-app, full CRUD) and `ssh_config` (imported from `~/.ssh/config`, read-only connection fields, metadata overlay). `reload_hosts()` merges launcher DB rows, imported ssh_config rows, and live resolver aliases without duplicating by name. Launcher rows win on name collision.

**Key modules:**

| Module | Purpose |
|--------|---------|
| `app.rs` | `App` state, `AppMode`, `SortMode`, `HostEntry` enum (Legacy/Managed), key/mouse dispatch, tab routing |
| `store/` | `LauncherStore` — SQLite v10 with `hosts`, `host_groups`, `identities`, `tunnels`, `auth_events` tables. CRUD + migrations |
| `metadata/` | Legacy `MetadataDb` (MVP). Still used by old code paths; `HostMetadata` struct |
| `ssh/` | `SshHost`, `HostResolver` trait, `SshConfigResolver` (shells out to `ssh -G`), import/export, agent detection, probe |
| `tunnel.rs` | `TunnelManager` — spawn/monitor/kill SSH -N -L/-R/-D child processes |
| `tui/mod.rs` | Top-level render dispatcher — `active_tab` (0-3) controls body; overlays for forms, help, confirm dialogs |
| `tui/screens/` | Tab renderers: `hosts.rs`, `tunnels.rs`, `keys.rs`, `audit.rs`, plus `host_form.rs`, `group_form.rs`, `help.rs` |
| `tui/widgets/` | Reusable widgets: `search_bar`, `host_list`, `detail_panel`, `status_bar`, `middle_stack`, `footer`, `panel_box` |
| `launcher/` | `TerminalLauncher` trait, `KittyLauncher`, `GhosttyLauncher`, `CustomLauncher` (template from config) |
| `search.rs` | nucleo wrapper for fuzzy filtering |
| `text_input.rs` | Vim-like modal text input widget |
| `watcher.rs` | `notify`-based file watcher, sends `WatchEvent` over channel |
| `config.rs` | `AppConfig` (TOML), XDG paths, env var overrides |
| `import/` | SSH config and Termius backup importers |

**Tab system:** `active_tab` (0=hosts, 1=tunnels, 2=keys, 3=audit) controls both rendering and key dispatch. Number keys 1-4 switch tabs. Each tab has its own `handle_key_*()` method.

**App mode flow:** `AppMode` determines rendering and key handling. `Normal` dispatches by `active_tab`. `Search` activates on `/`. `HostForm` / `GroupForm` / `IdentityForm` / `TunnelForm` are popup forms. `ConfirmDelete` and `ConfirmDiscard` show confirmation popups. `Help` renders the help screen.

**Test infrastructure:** `tests/support/` provides `FixtureResolver` (reads `tests/fixtures/ssh_config` and `tests/fixtures/ssh_g/*.txt` instead of real SSH) and `MockLauncher` (records launch calls). E2E tests use `TestBackend` and simulate key events. Smoke tests run headless via `SSHUB_AUTO_QUIT`.

**Environment variables for CI/headless:**
- `SSHUB_CONFIG_DIR` — override config directory
- `SSHUB_DATA_DIR` — override data/SQLite directory
- `SSHUB_SSH_CONFIG` — override SSH config path
- `SSHUB_DRY_RUN` — `run()` exits immediately without TUI
- `SSHUB_AUTO_QUIT` — `1` = quit after first draw, `q` = send quit key

**SQLite schema (v10):** `hosts` (id, name, label, address, port, group_id FK, identity_id FK, os_icon, tags JSON, notes, proxy_jump, forward_agent, remote_command, sort_order, favorite, last_connected, source, ssh_config_hash, timestamps), `host_groups` (id, name, sort_order, parent_id FK — nested groups), `identities` (id, name, username, private_key, certificate, sort_order), `tunnels` (id, host_id FK, tunnel_type, local_port, remote_host, remote_port, label, auto_connect, timestamps), `auth_events` (id, host_id, host_name, event_type, status, detail, created_at). `SCHEMA_VERSION` is the source of truth in `src/store/migrate.rs`; migrations run inside one transaction. Legacy `metadata.db` is migrated to `launcher.db` on first open — best-effort, so a corrupt/locked legacy db is skipped rather than aborting startup.