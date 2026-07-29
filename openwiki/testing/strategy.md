---
type: Playbook
title: Testing Strategy — unit, smoke, e2e, and config test levels
description: SSHub's four test levels — unit tests with in-memory stores and mock resolvers, smoke tests driving the real binary, e2e tests injecting key events into App with test doubles, and config_load — plus the fixture and support-helper layout.
resource: tests/e2e/mod.rs
tags: [testing, e2e, fixtures, mocks, quality]
---

# Testing Strategy

Four levels, all run by `just test` (and [CI](../operations/ci-cd.md) runs `cargo test`):

| Level | Command | Location | Mechanism |
|---|---|---|---|
| Unit | `cargo test` | `src/**`, esp. `src/app/tests/` (11 files) | `MockResolver` (HashMap-backed `HostResolver`), `RecordingLauncher`, `LauncherStore::open_in_memory()`, `NoopPasswordStore` |
| Smoke | `cargo test --test smoke` | `tests/smoke/` | `assert_cmd` drives the real binary: `--help`, `--dry-run`, headless quit via `SSHUB_AUTO_QUIT`, CLI commands, config load, resolver |
| E2E | `cargo test --test e2e` | `tests/e2e/` (16 scenario modules) | Build `App` directly via `App::new_with_deps(AppDeps{…})` and inject `crossterm::event::KeyEvent`s into `app.handle_key()`; assert on `app.mode`, store contents, mock-launcher records |
| Config | `cargo test --test config_load` | `tests/smoke/config_load.rs` | Fixture `config.toml` creation/loading |

Note: README's "e2e via TestBackend" is loose — e2e tests drive `App` without rendering; `TestBackend` is used by the headless loop in `src/lib.rs` (`run_headless_loop`) and by render-oriented unit tests in `src/tui/`.

## Fixtures & support helpers (`tests/support/`, `tests/fixtures/`)

Included via `#[path = "../support/mod.rs"]`:

- `FixtureResolver` (`tests/support/fixture_resolver.rs`) — `HostResolver` reading `tests/fixtures/ssh_config` (three hosts: dev-local, staging-app, prod-db-01) with canned `ssh -G` output from `tests/fixtures/ssh_g/<alias>.txt`.
- `MockLauncher` (`tests/support/mock_launcher.rs`) — `TerminalLauncher` recording `last_host`, `last_ssh_argv`, `managed_connect` in an `Arc<Mutex<…>>`.
- `tests/fixtures/config.toml` — kitty launcher + appearance defaults.

E2E scenario modules cover: config_reload, connect_managed, first_run, group_crud, host_crud, host_detail, host_sort, hybrid_compat, import_export, keychain, metadata_persist, quick_connect, search_and_navigate, ssh_config_sync, termius_import, tunnel_form.

## Env-var isolation

Tests rely on `SSHUB_CONFIG_DIR` / `SSHUB_DATA_DIR` / `SSHUB_SSH_CONFIG` overrides to stay hermetic; `SSHUB_AUTO_QUIT` (`1` = quit after first draw, `q` = send quit key) drives headless runs. Legacy `SSH_LAUNCHER_*` aliases are still honored and intentionally covered by `tests/smoke/run_app_quit.rs`.

## Where to add tests when changing…

- App modes / key handling → `src/app/tests/` unit + a `tests/e2e/` scenario (see [TUI workflow](../workflows/tui.md)).
- Schema → migration round-trip through `LauncherStore::open_in_memory` (runs all migrations from scratch — see [data model](../architecture/data-model.md)).
- CLI → `tests/smoke/cli_commands.rs` (see [CLI](../workflows/cli.md)).
- Session/SFTP logic → `src/app/tests/session.rs`, `src/app/tests/sftp.rs`; keep SFTP logic in the pure `src/sftp/model.rs` where possible.
