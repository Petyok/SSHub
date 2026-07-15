# Testing

SSHub has unit tests embedded in source files, integration tests in `tests/`, and e2e tests that drive the ratatui UI through `TestBackend`.

## Test targets

| Test | Command | Location | Purpose |
|------|---------|----------|---------|
| Unit | `cargo test` | `src/**/*.rs` (inline `#[cfg(test)]`) | Core logic, store, parsing |
| Smoke | `cargo test --test smoke` | `tests/smoke/` | Binary starts, `--dry-run`, headless quit |
| E2E | `cargo test --test e2e` | `tests/e2e/` | TUI scenarios via key events |
| Config load | `cargo test --test config_load` | `tests/smoke/config_load.rs` | config.toml create/load |

`just test` runs all four in sequence.

## Unit tests

Larger modules have dedicated test submodules:

- `src/app/tests/` — `host_crud`, `host_detail`, `host_form`, `identity_group`, `keybind`, `misc`, `session`, `sftp`, `tags`.
- `src/tunnel.rs` — tunnel manager tests.
- `src/watcher.rs` — config watcher tests.
- `src/store/` — migration and CRUD tests.

Many tests use `LauncherStore::open_in_memory()` and `App::test_new(...)` with dependency injection.

## E2E / TestBackend

`tests/e2e/*.rs` builds an `App` via `App::new_with_deps(...)`, then dispatches synthetic input with `app.handle_key(...)` (and occasionally `handle_mouse` / `handle_paste`). Most scenarios assert on `App` state directly; frame assertions can use `TestBackend` + `terminal.draw(|f| sshub::tui::render(f, &app))` when needed. There is no `tick()` test helper.

Key helpers:

- `App::new_with_deps(...)` / `App::test_new(...)` in `src/app/util.rs` builds an app with injected mock dependencies.
- `FixtureResolver` in `tests/support/` replaces real `ssh -G` calls.
- `MockLauncher` records external-terminal launch attempts.

## Headless / CI modes

- `SSHUB_DRY_RUN` — exit before launching the TUI.
- `SSHUB_AUTO_QUIT=1` — render one frame and exit.
- `SSHUB_AUTO_QUIT=q` — simulate the quit key.

These are exercised in `tests/smoke/binary_starts.rs`.

## Writing a new e2e test

1. Create an `App` through `App::new_with_deps(...)` / `App::test_new(...)` with the desired fixture resolver.
2. Set `app.mode` and any pre-conditions directly, or dispatch keys via `app.handle_key(...)`.
3. Optionally draw one frame with `TestBackend` when asserting on rendered text.
4. Assert on `app.hosts`, `app.auth_events_cache`, frame buffer, or captured mock calls.

See `tests/e2e/mod.rs` for patterns around host CRUD, group management, keybindings, and session modes.

## What to watch when changing tests

- Many tests set environment variables directly with `std::env::set_var`; prefer `tempfile::TempDir` for data paths to avoid cross-test pollution.
- Frame-based assertions use labels/spans; text changes need matching test updates.
- The `TestBackend` path must be activated via `App::test_new` or equivalent; the normal terminal path will fail without a real TTY.
- CI runs with no TTY; always use the dry-run or `SSHUB_AUTO_QUIT` paths for smoke checks.

## CI

GitHub Actions (`.github/workflows/ci.yml`) runs two jobs:

- **test** (ubuntu + macOS): `cargo build --all-targets`, then `cargo test` (all targets — unit tests in `src/` plus `smoke`, `e2e`, `config_load` integration tests).
- **lint** (ubuntu): `cargo fmt --check` and `cargo clippy --all-targets`.

`just test` is the local convenience wrapper for the test targets; CI does not invoke `just` but runs the same `cargo test` surface.
