---
type: Reference
title: CI & Automation — GitHub Actions workflows
description: SSHub's three GitHub Actions workflows — ci.yml (test matrix + fmt/clippy lint gate), release.yml (tag-triggered binaries, GitHub release, crates.io publish), and openwiki-update.yml (scheduled OpenWiki documentation regeneration bot).
resource: .github/workflows/ci.yml
tags: [ci, github-actions, release, automation, operations]
---

# CI & Automation

## `ci.yml` — test + lint

Triggers: pushes to `main`/`master`/`development` and all PRs.

- **test** — matrix over ubuntu/macos: `cargo build --all-targets`, `cargo test`. Linux installs `libdbus-1-dev`/`pkg-config` for the keyring backend.
- **lint** — ubuntu: `cargo fmt --check` + `cargo clippy --all-targets`. Matches the local gate in [build & release](build-release.md).

## `release.yml` — binaries + crates.io

Trigger: tag push `v*` (created by `just release`).

- **build** — three targets (linux-x64, macOS arm64, macOS x64) → tar.gz artifacts.
- **release** — GitHub release via `softprops/action-gh-release` with `CHANGELOG.md` as the body.
- **publish** — crates.io publish using the `CARGO_REGISTRY_TOKEN` secret. Only versions carried by `main` when the tag is pushed get published.

## `openwiki-update.yml` — wiki bot

Trigger: `workflow_dispatch` + daily cron `0 8 * * *`. Installs the `openwiki` npm CLI and runs `openwiki code --update --print` (OpenRouter provider, LangSmith tracing), then opens a PR from branch `openwiki/update` via `peter-evans/create-pull-request`.

## Secrets used by workflows

`CARGO_REGISTRY_TOKEN` (crates.io publish), OpenRouter/LangSmith keys for the wiki bot (referenced by env name only; see the workflow file for exact variable names). Never commit values.
