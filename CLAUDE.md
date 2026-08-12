# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Multi-agent worktrees

Stay in `ssh-tui/` for the primary checkout. For parallel agents, use
`just worktree-add <name>` (creates `../.worktrees/<name>` and points its
`target/` at shared `../.cargo-target`). Repair main symlink with
`just setup-shared-target`. Never run concurrent cargo builds on the shared
target. See [AGENTS.md](AGENTS.md).

## Workflow rules

Pinned implementation flow: [docs/implementation-flow.md](docs/implementation-flow.md) (issue → claim → branch → verify → adversarial review → PR → merge).

- **GitHub comments from agents** must always end with `_Written by {Model} ({Platform}) on behalf of the maintainer._` — see [implementation-flow § GitHub comments](docs/implementation-flow.md#github-comments-ai-agents).
- **Lint before push.** Always run `cargo fmt`, `cargo fmt --check`, and `cargo clippy --all-targets` locally before every push — CI runs the same checks; do not skip them.
- **CI before Done.** After opening or updating a PR, wait until GitHub Actions is green on that PR (`gh pr checks`). Local green is not Done; fix CI failures before marking the task complete.
- **Respect the community.** Every contribution from outside the maintainer is credited by handle, always — see [implementation-flow § Respect the community](docs/implementation-flow.md#respect-the-community).
- **Oracle tests.** When touching code that re-implements what an external tool already knows (`ssh`, `ssh-keygen`, a foreign export format), the test must ask that tool — a mock only proves the code agrees with itself. Read [docs/oracle-tests.md](docs/oracle-tests.md) before adding tests there.

- **Commit frequently.** After completing each logical unit of work (a bug fix, a feature, a refactor pass), create a commit immediately. Do not accumulate large uncommitted diffs across multiple tasks.
- **Branch model.** `main` is stable (releases + tags); `development` is the integration branch; work goes on branches cut from `development` and named by what it is — `feature/`, `fix/`, `docs/`, `chore/`, `poc/` — per [AGENTS.md § Branch naming](AGENTS.md#branch-naming), which `scripts/check-branch-name.sh` enforces in the `pre-commit` hook and in CI. Large epics may use `feature/<epic>` as an integration branch with child `poc/*` or stage branches; child exploratory PRs may target that integration branch, while the final production PR targets `development`. Normal flow remains `feature/* → development → main`. Releases merge `development` into `main` with a `--no-ff` merge commit `chore: release vX.Y.Z` (so `git log --first-parent main` shows one entry per release), bump the version + CHANGELOG, and push a `vX.Y.Z` tag (the release workflow builds binaries and publishes to crates.io). `main` and `development` converge at every release — see the Releasing section.
- **Delete merged branches.** The repo has "Automatically delete head branches" enabled, so merging a PR on GitHub removes the branch (keep the "Delete branch" box checked). For a local/CLI merge, delete it yourself right after: `git branch -d <branch>` and `git push origin --delete <branch>`. Never leave merged branches lingering.

## Versioning (`vX.Y.Z`)

Odometer scheme — **Z (patch)** rolls 0–9, **Y (minor)** rolls 0–99, and they carry:

- **Z (patch)** — bump on **every commit to `development`**: `just bump patch`. It's the running odometer counter within a dev cycle **and** the version a hotfix release ships as-is. Rolls 0–9, carrying into Y.
- **Y (minor)** — bump when **merging `development → main`** for a feature release; this resets Z to 0: `just bump minor`. A minor release is `X.Y.0`. Rolls 0–99, carrying into X (so `0.99.0 → 1.0.0`).
- **X (major)** — bump **manually** for a milestone, or automatically by carry when the odometer rolls over (`0.99.0 + minor → 1.0.0`, `0.99.9 + patch → 1.0.0`): `just bump major`.

`main` is **not** always `X.Y.0`: feature releases land as `X.Y.0`, but hotfix (patch) releases publish `development`'s current `X.Y.Z` unchanged (see `just release patch` below).

`just bump <patch|minor|major>` edits `Cargo.toml` + `Cargo.lock` with carry (`0.4.9 + patch → 0.5.0`). Only versions carried by `main` when a `vX.Y.Z` tag is pushed get published to crates.io (see the release workflow).

The patch bump is automated by a tracked `pre-commit` hook (`.githooks/pre-commit`) that runs `just bump patch` on every commit **to `development`** (skipped on other branches and during merges). Git hooks aren't shared on clone, so enable them once per checkout: `just setup-hooks` (sets `core.hooksPath .githooks`).

Releasing is one command, run from a clean `development`:

- **`just release`** (or `just release minor`) — feature release. `just bump minor`, tags `vX.Y.0`.
- **`just release patch`** — **hotfix**. Tags/publishes `development`'s **current** `vX.Y.Z` with **no bump**, so a fix can reach `main` + crates.io without pretending to be a new minor.
- **`just release X.Y.Z`** — release an **explicit** version (e.g. `just release 0.7.0` to jump ahead), no `--no-verify` dance needed.

Each release first **settles everything on `development`**: it sets the release version (Cargo.toml + lock), rolls the CHANGELOG (`[Unreleased]` → `[X.Y.Z] - <date>`, with a fresh empty `[Unreleased]` back on top) and commits that as `chore: prep release vX.Y.Z` (`--no-verify`, so the patch-bump hook doesn't move the version it just set). It then **merges `development` into `main` with a real merge commit** (`git merge --no-ff development -m "chore: release vX.Y.Z"`) and tags it (the tag triggers the release workflow → binaries + crates.io). The first-parent line of `main` is therefore one commit per release (`git log --first-parent main`), while blame/bisect/revert see the full feature history; reverting a whole release is `git revert -m 1 <merge>`, reverting one feature is a revert of its squashed dev commit (after reverting a merge, re-landing that history needs a revert of the revert). Finally the recipe **fast-forwards `development` to the release merge** (`git merge --ff-only main`), so both branches point at the same commit, ahead/behind stays clean, and the next dev commit hook-bumps to `X.Y.Z+1`. Docs fixes no longer need manual syncing to `main` — they ride the next release; if `main` ever gets a direct commit anyway, merge `main` into `development` before the next release. Pushing to protected `main` relies on the owner's admin bypass.

`just release patch` ships **whatever `development` currently holds** — it's the fast path when `development` == what you want on `main`. If `development` carries unreleased work you don't want in the hotfix, use the cherry-pick flow below.

### Hotfix release that excludes work already on `development`

For "ship these fixes now, hold that feature back". `just release` cannot do it: it refuses to run outside `development` and releases everything sitting there. Cut the release from `main` instead — worked example: [v0.14.2](https://github.com/Petyok/SSHub/pull/108), three fixes shipped while `sshub exec` stayed behind.

1. **Branch from the released state**, not from `development`: `git checkout -b fix/release-X.Y.Z origin/main`.
2. **Cherry-pick the fixes**, the squashed commit of each PR rather than its merge commit (`git cherry-pick <sha>`, no `-m 1`) — the history reads as the fixes themselves. Conflicts are normal and are the point: a test file that grew a block for the held-back feature will conflict, and the resolution is to keep only what ships.
3. **Scrub the excluded feature out of what ships.** Its CHANGELOG entry stays behind on `development`, and any *other* entry that merely mentions the feature must lose that mention — a released changelog naming a command the released binary does not have is a bug report waiting to happen. Grep the tree (`src/`, `man/`, `README.md`, completions, help text) before believing it is gone.
4. **Set the version and roll the CHANGELOG by hand**: `just bump set X.Y.Z`, then `[Unreleased]` → `## [X.Y.Z] - <date>` with a fresh empty `[Unreleased]` on top. Commit as `chore: prep release vX.Y.Z`.
5. **Verify the release tree, not `development`**: `cargo fmt --check`, `cargo clippy --all-targets`, `just test`, and a `--release` build that proves the excluded feature is actually absent (`sshub <cmd>` exits 2, `sshub --help` never names it).
6. **Open a PR into `main` for CI only** — and do not merge it with the button: the button writes `Merge pull request …`, and `main`'s first-parent line must read one `chore: release vX.Y.Z` per release. GitHub marks the PR merged on its own once the commits land.
7. **Merge and tag locally**: `git merge --no-ff fix/release-X.Y.Z -m "chore: release vX.Y.Z"` on `main`, push, then `git tag -a vX.Y.Z` and push the tag — the tag is what triggers binaries, crates.io and npm, so nothing is published until that push.
8. **Merge `main` back into `development` right away** (not `--ff-only`; `development` has commits `main` does not). The CHANGELOG conflicts by design: keep the held-back entries under `[Unreleased]`, take the released section as it shipped, and let `development` adopt the released version so the patch odometer continues from it.

Skipping step 8 is the one mistake that bites later: `main` then carries a fix `development` lacks, and the next `development → main` merge can quietly revert it.

## Build & test commands

```bash
# Build
cargo build

# Run all tests (unit + integration)
just test

# Lint (required before every push — matches CI)
cargo fmt
cargo fmt --check
cargo clippy --all-targets

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

The scheduled OpenWiki GitHub Actions workflow refreshes the repository wiki. Do not hand-edit generated OpenWiki pages unless explicitly asked; prefer updating source code/docs and letting OpenWiki regenerate.

<!-- OPENWIKI:END -->
