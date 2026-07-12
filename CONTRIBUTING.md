# Contributing to SSHub

Thanks for your interest in contributing. Here's how to get started.

## Getting started

1. Fork and clone the repo
2. Install Rust (edition 2021) and `just` (optional, for running the full test suite)
3. Run `cargo build` to verify everything compiles
4. Run `just test` (or `cargo test`) to confirm tests pass

## Branch model

- `main` receives **releases only** — one `chore: release vX.Y.Z` merge per
  release. Never target it with a PR.
- `development` is the integration branch. All work lands here first.
- Features/fixes go on `feature/*` branches **cut from `development`**.

Flow: `feature/* → development → main (release, maintainer-only)`.

## Making changes

1. Create a branch from `development` (not `main`)
2. Make your changes
3. **Run the full test suite: `just test` — it must be green before you push.**
   Not just your new test: the whole suite, because unit tests share one
   process and one machine state (see Tests below). CI runs the same suite
   plus `cargo fmt --check` and `cargo clippy --all-targets` on every PR.
4. Run `cargo fmt` and `cargo clippy` and fix any warnings
5. Update `CHANGELOG.md` under `[Unreleased]` for any user-visible change
6. Update `README.md` / the in-app help if behaviour or requirements change
7. Do **not** bump the version in `Cargo.toml` — versioning is automated
   (a pre-commit hook on `development` plus the release process)
8. Open a pull request against `development`

## Pull requests

- **Title**: conventional-commit style — `feat(scope): ...`, `fix(scope): ...`,
  `docs: ...`, `refactor: ...`, `test: ...`, `chore: ...`
- **Description**: what changed and *why*, how you tested it, and anything
  reviewers should look at closely. Bullet points are fine.
- Keep PRs focused — one feature or fix per PR.
- Changes touching **credentials, key handling, or anything
  security-sensitive** should say so explicitly in the description; silent
  changes to the security model will be bounced.

## AI involvement

All pull requests and issues in this repo are reviewed and triaged through
Claude (Opus 4.8 or Fable 5). Comments signed by Claude are written by the
model on behalf of the maintainer.

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

Rules that keep the suite green:

- **Unit tests run in parallel in one process.** `std::env::set_var` is
  process-global — setting a var your code path reads (e.g. `SSHUB_DATA_DIR`)
  races with every other test that resolves the same path. Prefer injecting
  paths/dependencies (see `AppDeps` and `tests/support/`) over env vars.
- **Never touch real user state.** No real `~/.ssh`, no real OS keyring, no
  real config/data dirs. Use `tempfile` and fixtures. A test that passes by
  hitting the developer's actual keyring isn't testing your code.
- **A test must exercise the code it claims to cover** on every machine —
  including one where the "unhappy path" you're testing doesn't naturally
  occur. If the fallback only triggers when a service is absent, structure
  the code so the fallback is testable directly.

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

By contributing, you agree that your contributions will be licensed under
[AGPL-3.0-or-later](LICENSE) — the project's copyleft license: forks and
derivatives must stay open under the same terms.
