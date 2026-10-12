---
type: Playbook
title: Testing Strategy, unit, smoke, e2e, config, transfer, and oracle tests
description: SSHub's test layout. In-crate unit tests (src/app/tests with in-memory stores and mock resolvers, render tests on TestBackend, allocation proofs), the smoke, e2e and config_load targets, the auto-discovered transfer tests, oracle tests against real ssh, ssh-keygen and a tempdir sshd, fixtures, env isolation, and the just test and coverage recipes.
resource: tests/e2e/mod.rs
tags: [testing, e2e, fixtures, oracle-tests, quality]
---

# Testing Strategy

`just test` runs `cargo test --no-default-features` (every target), then the
`smoke`, `e2e` and `config_load` targets again by name. [CI](../operations/ci-cd.md)
runs plain `cargo test` on ubuntu and macOS with default features. The tree has
about 1,680 `#[test]` functions.

| Level | Command | Location | Mechanism |
|---|---|---|---|
| Unit | `cargo test --lib` | inline `#[cfg(test)]` modules across `src/`, plus `src/app/tests/` (19 modules) | `test_app` / `test_store` helpers in `src/app/tests/mod.rs`: HashMap-backed `MockResolver`, `LauncherStore::open_in_memory()`, in-memory `MemoryPasswordStore`; key events fed to `App::handle_key` |
| Smoke | `cargo test --test smoke` | `tests/smoke/binary_starts.rs` with `cli_commands`, `resolver`, `run_app_quit`, `theme_public_api` | `assert_cmd` drives the real binary: `--help`, `--dry-run`, headless quit via `SSHUB_AUTO_QUIT`, CLI commands; `theme_public_api` uses the theme API from outside the crate |
| E2E | `cargo test --test e2e` | `tests/e2e/` (22 scenario modules) | Build `App` with `App::new_with_deps(AppDeps { .. })` and inject `crossterm` `KeyEvent`s; assert on `app.mode` and store contents (a few modules, e.g. `broadcast`, `theme_picker`, also render to a `TestBackend`) |
| Config | `cargo test --test config_load` | `tests/smoke/config_load.rs` | Fixture `config.toml` creation and loading |
| Transfer | `cargo test --test transfer_engine` / `--test transfer_cli` | `tests/transfer_engine.rs`, `tests/transfer_cli.rs` (auto-discovered targets) | Contract tests for `store::transfer` and the `cli::transfer` plan/args layer |

`src/app/tests/` modules: broadcast, cert_agent, ghost, host_crud, host_detail,
host_form, identity_group, keybind, log_browser, misc, profile_manager,
session, sftp, snippets, suggestions, tags, theme_picker,
transfer_plan_confirm, transport.

E2E modules: adhoc_connect, broadcast, config_reload, connect_managed,
first_run, group_crud, host_crud, host_detail, host_sort, hybrid_compat,
import_export, keychain, local_shell, metadata_persist, quick_connect,
search_and_navigate, session_switcher, snippet_crud, ssh_config_sync,
termius_import, theme_picker, tunnel_form.

Rendering is tested with ratatui's `TestBackend` inside `src/tui/` and the
screen modules (new overlays need a render smoke test, per AGENTS.md). The
`default` dashboard is pinned cell for cell by
`tests/fixtures/theme/default-dashboard.buffer`, and the theme role catalogue
by `src/theme/role_matrix.snapshot`. `src/test_alloc.rs` is the single
`#[global_allocator]` that counts per-thread allocations for tests proving a
hot path allocation-free; `src/test_support.rs` holds shared render fixtures.

## Oracle tests

Code that re-implements what an external tool knows must be tested against that
tool ([docs/oracle-tests.md](../../docs/oracle-tests.md)). Each one skips, not
fails, when the tool is missing:

- `ssh/resolver.rs::listed_aliases_round_trip_through_real_ssh`: every listed
  alias must resolve with the real `ssh -G`.
- `known_hosts.rs`: assertions derived from real `ssh-keygen -F` / `-R`.
- `ssh/cert.rs` and `src/app/tests/cert_agent.rs`: certificates made and read
  with the real `ssh-keygen`.
- `session/mod.rs`: a key-auth **tempdir `sshd`** on a free 127.0.0.1 port
  (`tempdir_sshd`) backs `real_ssh_master_latches_connected_through_drain` and a
  ProxyJump test proving a jump host's authentication never marks the session
  connected.

Still missing per that doc: an `ssh/export.rs` round trip through `ssh -G`, and
real (not agent-authored) PuTTY, mRemoteNG and Termius export files.

## Fixtures & support (`tests/support/`, `tests/fixtures/`)

- `tests/support/mod.rs` (included with `#[path = "../support/mod.rs"]`) only
  provides `FixtureResolver` (`fixture_resolver.rs`): a `HostResolver` reading
  `tests/fixtures/ssh_config` (dev-local, staging-app, prod-db-01) with canned
  `ssh -G` output from `tests/fixtures/ssh_g/<alias>.txt`.
- `tests/fixtures/config.toml`: a minimal `[appearance]` table.
- `tests/fixtures/theme/default-dashboard.buffer`: the golden dashboard.

## Env-var isolation

Tests stay hermetic through `SSHUB_CONFIG_DIR` / `SSHUB_DATA_DIR` /
`SSHUB_SSH_CONFIG` (compat mode: no profiles, empty credential prefix).
`SSHUB_AUTO_QUIT=q` sends the quit key (and confirms the dialog); any other
value quits after the first frame. Legacy `SSH_LAUNCHER_*` aliases are covered
by `tests/smoke/run_app_quit.rs`. In-crate tests that mutate process env must
serialize through `config::with_test_config_dir` (or the `lib.rs` test
`lock_home`). Never touch the real `~/.ssh`, keyring or config dirs: a compat
profile still writes unprefixed `host:N` entries to a reachable Secret Service,
so manual smoke runs should point `DBUS_SESSION_BUS_ADDRESS` at a dead socket.

## Coverage

`just coverage` (cargo-llvm-cov) prints the summary and the functions no test
executes even once, the input for `docs/coverage-map.md`; read that file for
why the never-executed list matters more than the percentage.

## Where to add tests when changing...

- App modes / key handling: `src/app/tests/` plus a `tests/e2e/` scenario (see
  [TUI workflow](../workflows/tui.md)); new key actions need the full
  `keybinds.rs` insertion (enum, `ALL`, label, config field, default, binds,
  set).
- Schema: an upgrade test in `src/store/migrate.rs` from the previous schema,
  plus `LauncherStore::open_in_memory` round trips (see
  [data model](../architecture/data-model.md)).
- CLI: `tests/smoke/cli_commands.rs` (see [CLI](../workflows/cli.md)).
- Sessions/SFTP: `src/app/tests/session.rs`, `src/app/tests/sftp.rs`, inline
  tests in `src/session/`; keep SFTP logic in the pure `src/sftp/model.rs`.
- Anything that parses an external tool's output: an oracle test.
