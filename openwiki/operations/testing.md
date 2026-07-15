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

`tests/e2e/mod.rs` creates an `App` with a `TestBackend`, then sends synthetic crossterm key events (and some mouse events) to simulate user interactions. It asserts on frame output or on `App` state after a few ticks.

Key helpers:

- `App::test_new(...)` in `src/app/util.rs` builds an app with injected mock dependencies.
- `FixtureResolver` in `tests/support/` replaces real `ssh -G` calls.
- `MockLauncher` records external-terminal launch attempts.

## Headless / CI modes

- `SSHUB_DRY_RUN` — exit before launching the TUI.
- `SSHUB_AUTO_QUIT=1` — render one frame and exit.
- `SSHUB_AUTO_QUIT=q` — simulate the quit key.

These are exercised in `tests/smoke/binary_starts.rs`.

## Writing a new e2e test

1. Create an `App` through `App::test_new(...)` with the desired fixture resolver.
2. Set `app.mode` and any pre-conditions directly, or dispatch keys.
3. Call the rendering function and drain the event loop via `tick()` helpers.
4. Assert on `app.hosts`, `app.auth_events_cache`, frame output, or captured mock calls.

See `tests/e2e/mod.rs` for patterns around host CRUD, group management, keybindings, and session modes.

## What to watch when changing tests

- Many tests set environment variables directly with `std::env::set_var`; prefer `tempfile::TempDir` for data paths to avoid cross-test pollution.
- Frame-based assertions use labels/spans; text changes need matching test updates.
- The `TestBackend` path must be activated via `App::test_new` or equivalent; the normal terminal path will fail without a real TTY.
- CI runs with no TTY; always use the dry-run or `SSHUB_AUTO_QUIT` paths for smoke checks.

## CI

GitHub Actions is configured under `.github/workflows/`. It should mirror `just test` plus format/clippy checks.
