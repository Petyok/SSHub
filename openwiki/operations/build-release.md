---
type: Playbook
title: Build, Versioning & Release — Justfile recipes, odometer versioning, and release flow
description: How to build, test, version, and release SSHub — Justfile recipes, the odometer vX.Y.Z versioning scheme with the pre-commit auto-bump hook, the feature/*→development→main branch model, and the one-command just release flow.
resource: Justfile
tags: [build, release, versioning, just, operations]
---

# Build, Versioning & Release

## Everyday commands

```bash
just build      # cargo build --release
just test       # unit + smoke + e2e + config_load (all four targets)
just dry-run    # cargo run -- --dry-run
```

Lint gate (run before **every** push; [CI](ci-cd.md) runs the same): `cargo fmt`, `cargo fmt --check`, `cargo clippy --all-targets`.

## Justfile recipes

| Recipe | What it does |
|---|---|
| `test` | All 4 test targets (see [testing](../testing/strategy.md)) |
| `build` | Release binary (prerequisite of `install`) |
| `install` / `uninstall` | Binary → `~/.local/bin`, completions, man page, icon + `.desktop` entry (kitty→ghostty→alacritty→foot→xterm detection) |
| `install-completions` | bash/fish completions to auto-load dirs; sourced zsh block appended to `~/.zshrc` |
| `man` | Preview `man/sshub.1` via `man -l` |
| `bump <patch|minor|major|set> [version]` | Odometer version bump in `Cargo.toml` + `Cargo.lock` (no cargo invocation) |
| `release [minor|patch|X.Y.Z]` | Full release: settle version on `development`, roll CHANGELOG, `--no-ff` merge to `main`, tag `vX.Y.Z`, push, ff `development` |
| `sync` | Merge `development`→`main` without a release (no tag/bump/changelog) |
| `setup-hooks` | One-time `git config core.hooksPath .githooks` per checkout |
| `record-gifs *tapes` | VHS + ffmpeg demo recording (see [integrations](../integrations/external-terminals.md#demo-pipeline)) |
| `dry-run` | Headless sanity check |

## Versioning — odometer `vX.Y.Z`

Each field rolls 0–9 and carries (`0.9.9 + patch → 1.0.0`):

- **Z (patch)** — bumped on **every commit to `development`** by the tracked pre-commit hook (`.githooks/pre-commit`, runs `just bump patch`; skipped on other branches and merges). Enable once per checkout with `just setup-hooks`.
- **Y (minor)** — bumped when merging `development → main` for a feature release; resets Z (`X.Y.0`).
- **X (major)** — manual milestone bump, or automatic carry on rollover.

`main` is not always `X.Y.0`: hotfix releases publish `development`'s current `X.Y.Z` unchanged.

## Release flow (`just release`)

Run from a clean `development`:

1. **Settle on `development`** — set the release version in `Cargo.toml` + lock, roll the CHANGELOG (`[Unreleased]` → `[X.Y.Z] - <date>`, fresh empty `[Unreleased]` on top), commit as `chore: prep release vX.Y.Z` with `--no-verify` (so the hook doesn't re-bump).
2. **Merge & tag** — `git merge --no-ff development` into `main` (`chore: release vX.Y.Z`), tag `vX.Y.Z`, push. The tag triggers the [release workflow](ci-cd.md) → binaries + crates.io.
3. **Converge** — fast-forward `development` to the release merge so both branches point at the same commit; the next dev commit hook-bumps to `X.Y.Z+1`.

Consequences: `git log --first-parent main` shows one commit per release; reverting a whole release is `git revert -m 1 <merge>`; reverting one feature is reverting its squashed dev commit. `just release patch` ships whatever `development` currently holds — land only the fix first if `development` carries unreleased work. Pushing to protected `main` relies on the owner's admin bypass.

## Branch model & contributor flow

`feature/*` branches cut from `development` → PR → `development` → release merges to `main`. Merged branches are deleted (GitHub auto-delete enabled). The pinned pipeline (issue → claim → branch → verify → adversarial review → PR) is [docs/implementation-flow.md](../../docs/implementation-flow.md); agent-authored GitHub comments must end with `_Written by {Model} ({Platform}) on behalf of the maintainer._` See `CONTRIBUTING.md` and `CLAUDE.md` (canonical rules; do not duplicate them here).
