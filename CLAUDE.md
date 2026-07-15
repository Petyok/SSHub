# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Workflow rules

Pinned implementation flow: [docs/implementation-flow.md](docs/implementation-flow.md) (issue → claim → branch → verify → adversarial review → PR → merge).

- **Commit frequently.** After completing each logical unit of work (a bug fix, a feature, a refactor pass), create a commit immediately. Do not accumulate large uncommitted diffs across multiple tasks.
- **Branch model.** `main` is stable (releases + tags); `development` is the integration branch; features go on `feature/*` branches cut from `development`. Flow: `feature/* → development → main`. Releases merge `development` into `main` with a `--no-ff` merge commit `chore: release vX.Y.Z` (so `git log --first-parent main` shows one entry per release), bump the version + CHANGELOG, and push a `vX.Y.Z` tag (the release workflow builds binaries and publishes to crates.io). `main` and `development` converge at every release — see the Releasing section.
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

- **`just release`** (or `just release minor`) — feature release. `just bump minor`, tags `vX.Y.0`.
- **`just release patch`** — **hotfix**. Tags/publishes `development`'s **current** `vX.Y.Z` with **no bump**, so a fix can reach `main` + crates.io without pretending to be a new minor.
- **`just release X.Y.Z`** — release an **explicit** version (e.g. `just release 0.7.0` to jump ahead), no `--no-verify` dance needed.

Each release first **settles everything on `development`**: it sets the release version (Cargo.toml + lock), rolls the CHANGELOG (`[Unreleased]` → `[X.Y.Z] - <date>`, with a fresh empty `[Unreleased]` back on top) and commits that as `chore: prep release vX.Y.Z` (`--no-verify`, so the patch-bump hook doesn't move the version it just set). It then **merges `development` into `main` with a real merge commit** (`git merge --no-ff development -m "chore: release vX.Y.Z"`) and tags it (the tag triggers the release workflow → binaries + crates.io). The first-parent line of `main` is therefore one commit per release (`git log --first-parent main`), while blame/bisect/revert see the full feature history; reverting a whole release is `git revert -m 1 <merge>`, reverting one feature is a revert of its squashed dev commit (after reverting a merge, re-landing that history needs a revert of the revert). Finally the recipe **fast-forwards `development` to the release merge** (`git merge --ff-only main`), so both branches point at the same commit, ahead/behind stays clean, and the next dev commit hook-bumps to `X.Y.Z+1`. Docs fixes no longer need manual syncing to `main` — they ride the next release; if `main` ever gets a direct commit anyway, merge `main` into `development` before the next release. Pushing to protected `main` relies on the owner's admin bypass.

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

For current architecture details (tabs, schema, event loop, modules), see `openwiki/quickstart.md` and `openwiki/architecture/overview.md`.

<!-- OPENWIKI:START -->

## OpenWiki

This repository uses OpenWiki for recurring code documentation. Start with `openwiki/quickstart.md`, then follow its links to architecture, workflows, domain concepts, operations, integrations, testing guidance, and source maps.

Implementation flow: [docs/implementation-flow.md](../docs/implementation-flow.md).

The scheduled OpenWiki GitHub Actions workflow regenerates this wiki by default. After each automated or manual update, validate and correct pages against the codebase before merge — automated output is a starting point, not the source of truth.

<!-- OPENWIKI:END -->
