# Build, test & release

SSHub is a Rust application with a standard Cargo project layout. Development commands are wrapped in `Justfile` because several workflows (version bumps, releases) are easier to script there than in pure Cargo.

## Build

```bash
cargo build              # debug
cargo build --release    # release, used by just install
```

The release binary is a single executable named `sshub` (crate `sshub` via `src/main.rs`). There is also a library target (`src/lib.rs`) so tests and examples can link it.

## Test suites

Run everything with:

```bash
just test
```

Which executes:

```bash
cargo test               # unit tests inside src/
cargo test --test smoke  # binary smoke: help, dry-run, headless quit
cargo test --test e2e    # TUI scenario tests via TestBackend
cargo test --test config_load
```

Run a single e2e scenario by name:

```bash
cargo test --test e2e host_crud
```

## Format & lint

The project expects `cargo fmt` and `cargo clippy` to pass. Recent commits (e.g. `3c566a9`) are routinely CI-clean; run both before pushing.

## Versioning

Version is stored in `Cargo.toml` and must stay in sync with `Cargo.lock`.

- Odometer-style scheme: `X.Y.Z`, each field 0–9 with carry.
- **Patch (Z)** is bumped on every commit to `development` by a tracked pre-commit hook in `.githooks/pre-commit`: `just bump patch`.
- **Minor (Y)** is bumped when merging `development → main` for a feature release; resets Z to 0.
- **Major (X)** is bumped manually or by rollover (`0.9.9` → `1.0.0`).

Enable the hook once per clone:

```bash
just setup-hooks
```

Bump manually when needed:

```bash
just bump patch
just bump minor
just bump major
just bump set 0.9.0
```

## Release flow

Releases are cut from the `development` branch.

```bash
just release           # feature release, bumps Y, tags vX.Y.0
just release patch     # hotfix, tags current vX.Y.Z without bumping
just release 0.8.4     # explicit version
```

What `just release` does:

1. Settles version + CHANGELOG on `development` (`chore: prep release vX.Y.Z`).
2. Merges `development` into `main` with `--no-ff` (`chore: release vX.Y.Z`).
3. Tags `vX.Y.Z`; the GitHub Actions release workflow builds binaries and publishes to crates.io.
4. Fast-forwards `development` back to the release merge.

Only versions carried by `main` when the tag is pushed are published to crates.io.

## Install locally

```bash
just install   # build, copy to ~/.local/bin, install icon + desktop entry
```

`just uninstall` removes the binary, desktop entry, and icon.

Prebuilt Linux/macOS binaries are attached to GitHub releases.

## CI / headless environment variables

- `SSHUB_DRY_RUN` — exit before TUI.
- `SSHUB_AUTO_QUIT=1` — render one frame and exit.
- `SSHUB_AUTO_QUIT=q` — simulate quit key.
- `SSHUB_CONFIG_DIR`, `SSHUB_DATA_DIR`, `SSHUB_SSH_CONFIG` — override paths.

See [`operations/runbook.md`](operations/runbook.md) for path details.

## What to watch when changing the build

- `Cargo.toml` — feature flags matter: `keyring` needs a platform backend (Apple/Windows/Secret Service), `ssh2` uses vendored OpenSSL for static builds, `rusqlite` uses bundled SQLite.
- `Justfile` — version bump regexes depend on the exact `Cargo.toml` / `Cargo.lock` line format.
- `.githooks/pre-commit` — only runs on `development`; if you rename branches you must update the hook logic.

## Crate publish exclusions

`Cargo.toml` excludes demo media and CI config from the published crate to keep it small.
