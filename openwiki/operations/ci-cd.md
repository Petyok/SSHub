---
type: Reference
title: CI & Automation — GitHub Actions workflows
description: SSHub's four GitHub Actions workflows — ci.yml (test matrix + fmt/clippy lint gate), release.yml (tag-triggered binaries, GitHub release, crates.io publish), openwiki-update.yml (scheduled OpenWiki documentation regeneration bot), and strix-pentest.yml (PR security scan).
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

Trigger: `workflow_dispatch` + daily cron `0 8 * * *`. Installs the `openwiki` npm CLI and runs `openwiki code --update --print` (OpenRouter provider, model `z-ai/glm-5.2`, LangSmith tracing), then opens a PR from branch `openwiki/update` via `peter-evans/create-pull-request`. The PR path filter includes the workflow file itself, so bot config edits ride along with doc updates.

## `strix-pentest.yml` — PR security scan

Trigger: every pull request. Runs [Strix](https://strix.ai) in quick scan mode (`strix -n -t ./ --scan-mode quick`) against the checked-out tree. Auth: `STRIX_LLM` secret for the model name, and `LLM_API_KEY` mapped to the existing `OPENROUTER_API_KEY` secret (no extra key needed).

## Secrets used by workflows

`CARGO_REGISTRY_TOKEN` (crates.io publish), OpenRouter/LangSmith keys for the wiki bot, and `STRIX_LLM` for the PR pentest scan (referenced by env name only; see the workflow files for exact variable names). Never commit values.
