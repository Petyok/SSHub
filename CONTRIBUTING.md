# Contributing to SSHub

Thanks for your interest in contributing. Here's how to get started.

## Getting started

1. Fork and clone the repo
2. Install Rust (edition 2021) and `just` (optional, for running the full test suite)
3. Run `cargo build` to verify everything compiles
4. Run `just test` (or `cargo test`) to confirm tests pass

## Making changes

1. Create a branch from `main`
2. Make your changes
3. Run the full test suite: `just test`
4. Run `cargo clippy` and fix any warnings
5. Commit with a clear message describing what and why
6. Open a pull request against `main`

## Code style

- Follow standard Rust conventions (`rustfmt` defaults)
- Keep comments minimal -- explain *why*, not *what*
- No unnecessary abstractions; straightforward code over clever code
- Match existing patterns in the codebase

## Architecture overview

The app is a synchronous event loop (no async runtime) built on ratatui + crossterm:

- `src/app.rs` -- central state machine, key/mouse dispatch
- `src/tui/` -- rendering (mod.rs dispatches by tab, screens/ for full views, widgets/ for reusable components)
- `src/store/` -- SQLite CRUD (hosts, groups, identities, tunnels, auth events)
- `src/ssh/` -- SSH config parsing, host resolution, agent detection, probe
- `src/tunnel.rs` -- tunnel process management (spawn/monitor/kill)
- `src/launcher/` -- terminal launcher implementations (kitty, ghostty, custom)

## Tests

All tests run without a TTY or real `~/.ssh/config`. Fixtures live in `tests/fixtures/`.

| Level  | What to add                                               |
|--------|-----------------------------------------------------------|
| Unit   | `#[cfg(test)]` module in the same file as your code       |
| E2E    | `tests/e2e/` -- TUI scenarios using TestBackend           |
| Smoke  | `tests/smoke/` -- binary-level checks (help, dry-run)     |

Run a specific test:

```bash
cargo test --test e2e host_crud
cargo test my_unit_test_name
```

## Reporting bugs

Open an issue with:
- What you expected vs. what happened
- Steps to reproduce
- Terminal emulator and OS
- Output of `sshub --help` (shows version)

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
