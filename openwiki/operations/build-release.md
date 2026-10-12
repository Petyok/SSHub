---
type: Playbook
title: Build, Versioning & Release, Justfile recipes, features, shared target, odometer versioning, and release flow
description: How to build, test, version, and release SSHub. Justfile recipes, the default vendored OpenSSL feature and --no-default-features for local work, the shared Cargo target for worktrees, the odometer vX.Y.Z scheme (Z 0-9, Y 0-99) with the pre-commit auto-bump and branch-name hook, the explicit-kind just release flow, the cherry-pick hotfix flow, and the branch model.
resource: Justfile
tags: [build, release, versioning, just, worktrees, operations]
---

# Build, Versioning & Release

## Everyday commands

```bash
just test       # cargo test --no-default-features, then --test smoke / e2e / config_load
just build      # cargo build --release (default features: vendored OpenSSL)
just dry-run    # cargo run -- --dry-run
```

Gate before **every** push (AGENTS.md): `just test`, `cargo fmt`,
`cargo fmt --check`, `cargo clippy --all-targets`. [CI](ci-cd.md) runs clippy
with `-D warnings`, so any warning fails.

## Cargo features and profiles (`Cargo.toml`)

- `default = ["vendored"]`, `vendored = ["ssh2/vendored-openssl"]`: OpenSSL is
  compiled from source so `cargo install sshub` and the release tarballs need no
  system OpenSSL. `--no-default-features` links the system OpenSSL instead
  (distro packagers; local work, where it saves about 120 MB of target dir per
  profile and most of a cold build). `just test` uses it; anything that ships
  (`just build`, CI, the release workflow) stays on the default.
- `[profile.release]`: `lto = "thin"`, `codegen-units = 1`,
  `strip = "symbols"`. `[profile.dev]`: `debug = "line-tables-only"`, no debug
  info for dependencies.
- Test targets declared explicitly: `smoke` (`tests/smoke/binary_starts.rs`),
  `e2e` (`tests/e2e/mod.rs`), `config_load` (`tests/smoke/config_load.rs`);
  `tests/transfer_cli.rs` and `tests/transfer_engine.rs` are auto-discovered.
  Example `seed-demo` (`demo/seed_demo.rs`).

## Shared Cargo target for worktrees

All checkouts (main checkout plus every agent worktree under
`../.worktrees/<name>`) build into one `../.cargo-target`. Each checkout has an
untracked `.cargo/config.toml` with an absolute `build.target-dir`; there is no
`target` symlink (`cargo clean` deletes symlinks and silently splits the dir).
Never run two cargo builds against the shared target at once (fingerprint
races). A build in one worktree can finish instantly with another worktree's
`sshub` binary when its sources are older, so touch a source file before an A/B
comparison.

## Justfile recipes

| Recipe | What it does |
|---|---|
| `test` | The four `cargo test --no-default-features` runs above (see [testing](../testing/strategy.md)) |
| `coverage` | `cargo llvm-cov` summary plus `scripts/uncovered-functions.py`, the never-executed function list for `docs/coverage-map.md` |
| `build` | `cargo build --release` (prerequisite of `install` and `install-completions`) |
| `install` / `uninstall` | Binary to `~/.local/bin`, completions, man page, icon and `.desktop` entry (terminal detection kitty, ghostty, alacritty, foot, else xterm) |
| `install-completions` | bash and fish completions into auto-loaded dirs; a marked sourcing block appended to `~/.zshrc` |
| `man` | Preview `man/sshub.1` via `man -l` |
| `bump <patch\|minor\|major\|set> [version]` | Odometer bump in `Cargo.toml` and the `sshub` entry of `Cargo.lock` (no cargo invocation) |
| `release <minor\|patch\|X.Y.Z>` | Release flow below; refuses to run without a kind |
| `sync` | Merge `development` into `main` without a release (no tag, bump or changelog roll) |
| `setup-hooks` | `git config core.hooksPath .githooks`, once per checkout |
| `setup-shared-target` | Write `.cargo/config.toml` pointing at `../.cargo-target`, migrating any `./target` |
| `sweep [size]` | `cargo sweep --maxsize` on the shared target (default 4GB) |
| `worktree-add <name> [branch]` / `worktree-rm <name> [delete-branch]` | Create or remove `../.worktrees/<name>` (default branch `feature/<name>`), wiring the shared target; `worktree-rm` then runs `just sweep` when `cargo-sweep` is installed |
| `npm-build [version]` / `npm-publish [version]` | `npm/build.sh` from release tarballs (see [integrations](../integrations/external-terminals.md#npm-packaging-npm)) |
| `record-gifs [scenario ...]` | `demo/record.py`, asciicast plus `agg` (see [integrations](../integrations/external-terminals.md#demo-pipeline-demo)) |
| `dry-run` | Headless sanity check |

## Versioning: odometer `vX.Y.Z`

Z rolls 0 to 9 and carries into Y; Y rolls 0 to 99 and carries into X
(`0.9.9 + patch → 0.10.0`, `0.99.9 + patch → 1.0.0`, `0.99.0 + minor →
1.0.0`).

- **Z (patch)**: bumped on **every commit to `development`** by the tracked
  `.githooks/pre-commit` (`just bump patch`; skipped on other branches, during
  merge/rebase/cherry-pick, and when `just` is missing).
- **Y (minor)**: bumped by `just release minor`; resets Z.
- **X (major)**: manual milestone or carry.

The same hook first runs `scripts/check-branch-name.sh <branch>` (name shape
only; CI checks the commit types, see [CI](ci-cd.md#ciyml-branch-test-lint)).
`main` is not always `X.Y.0`: patch releases publish `development`'s current
`X.Y.Z` unchanged.

## Release flow (`just release <kind>`)

Run from a clean `development`; there is deliberately no default kind
(`patch` ships the current version, `minor` bumps Y, `X.Y.Z` sets it):

1. **Settle on `development`**: set the version, refuse if `vX.Y.Z` is already
   tagged, roll the CHANGELOG (`[Unreleased]` → `[X.Y.Z] - <date>` with a fresh
   empty `[Unreleased]`), commit `chore: prep release vX.Y.Z` with
   `--no-verify`, push.
2. **Merge and tag**: on `main`, `git merge --no-ff development -m "chore:
   release vX.Y.Z"`, annotated tag `vX.Y.Z`, `git push origin main
   --follow-tags`. The tag triggers the [release workflow](ci-cd.md): binaries,
   GitHub release, npm, crates.io.
3. **Converge**: fast-forward `development` to the release merge and push.

`git log --first-parent main` shows one commit per release; reverting a release
is `git revert -m 1 <merge>`. Pushing to protected `main` relies on the owner's
admin bypass.

**Hotfix that must exclude work already on `development`**: `just release`
cannot do it. CLAUDE.md § "Hotfix release that excludes work already on
`development`" is the canonical procedure: branch `fix/release-X.Y.Z` from
`origin/main`, cherry-pick the squashed fix commits, scrub the held-back feature
from what ships, `just bump set X.Y.Z` and roll the CHANGELOG by hand, verify
that tree, open a PR into `main` for CI only, merge `--no-ff` and tag locally,
then immediately merge `main` back into `development`.

## Branch model & contributor flow

Branches are cut from `development` and named by what the work is:
`feature/` (a `feat:` commit), `fix/`, `docs/`, `chore/` (chore, ci, build,
refactor, perf, style, test), `poc/` (epic child branches). Epics may use
`feature/<epic>` as an integration branch. PRs target `development`; releases
merge to `main`. Merged branches are deleted. The pinned pipeline (issue, claim,
branch, verify, adversarial review, PR) is
[docs/implementation-flow.md](../../docs/implementation-flow.md); agent-authored
GitHub comments end with `_Written by {Model} ({Platform}) on behalf of the
maintainer._` Canonical rules live in `AGENTS.md`, `CLAUDE.md` and
`CONTRIBUTING.md`; do not duplicate them here.
