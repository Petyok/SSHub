---
type: Reference
title: CI & Automation, GitHub Actions workflows and repo scripts
description: SSHub's four GitHub Actions workflows. ci.yml (branch-name check, ubuntu/macOS test matrix, fmt and clippy -D warnings), release.yml (tag-triggered binaries, GitHub release, npm via OIDC, crates.io), openwiki-update.yml (manual OpenWiki regeneration PR), strix-pentest.yml (manual security scan), plus scripts/check-branch-name.sh and the pre-commit hook.
resource: .github/workflows/ci.yml
tags: [ci, github-actions, release, automation, operations]
---

# CI & Automation

## `ci.yml`: branch, test, lint

Triggers: pushes to `main`, `master`, `development`, and every pull request.

- **branch** ("Branch naming"): pull requests from this repository only (forks
  are skipped). Runs `scripts/check-branch-name.sh --self-test`, then collects
  the conventional-commit type of every commit in `base..head` and runs
  `scripts/check-branch-name.sh "$HEAD_REF" <types>`: the prefix must be known
  and at least one commit must justify it (`feature:feat`, `fix:fix`,
  `docs:docs`, `chore:chore,ci,build,refactor,perf,style,test`, `poc:` any).
  Exempt: `main`, `master`, `development`, `HEAD`, `openwiki/update`.
- **test**: matrix `ubuntu-latest`, `macos-latest`; `cargo build --all-targets`,
  `cargo test` (default features, so vendored OpenSSL). Linux installs
  `libdbus-1-dev` and `pkg-config` for the keyring backend.
- **lint**: ubuntu; `cargo fmt --check` and
  `cargo clippy --all-targets -- -D warnings`.

## `release.yml`: binaries, npm, crates.io

Trigger: tag push `v*` (created by `just release`). `permissions: contents:
write`.

- **build**: `x86_64-unknown-linux-gnu` (ubuntu), `aarch64-apple-darwin` and
  `x86_64-apple-darwin` (macOS). `cargo build --release --target ...`, a
  `--dry-run` smoke of the built binary (except the cross-built Intel Mac, which
  is checked with `file` and ad-hoc `codesign --sign -` plus `--verify`), then
  `sshub-<tag>-<target>.tar.gz`.
- **release**: `softprops/action-gh-release` with the tarballs and
  `CHANGELOG.md` as the body.
- **npm**: Node 24, latest npm, `id-token: write` (npm OIDC trusted
  publishing, no token secret); `npm/build.sh "<version>" --publish` from the
  downloaded tarballs.
- **publish**: `cargo publish --token ${{ secrets.CARGO_REGISTRY_TOKEN }}`.

## `openwiki-update.yml`: wiki bot

Trigger: **`workflow_dispatch` only** (there is no schedule; the file also
declares a leftover `scan_mode` input copied from the Strix workflow that
nothing uses). Installs the `openwiki` npm CLI on Node 22 and runs
`openwiki code --update --print "<audit prompt>"` with
`OPENWIKI_PROVIDER=openrouter`, `OPENWIKI_MODEL_ID=z-ai/glm-5.3-flash`,
`OPENROUTER_API_KEY` and optional LangSmith tracing (`LANGSMITH_API_KEY`). The
prompt makes `CHANGELOG.md` the change ledger and asks for
`openwiki/changelog-coverage.md` to be maintained. `peter-evans/create-pull-request`
opens a PR from branch `openwiki/update` into `development`, adding only
`openwiki`, `AGENTS.md` and `CLAUDE.md`.

Generated pages are a starting point, not ground truth: validate them against
`src/` with the checklist in [INSTRUCTIONS.md](../INSTRUCTIONS.md) and update
`openwiki/.last-update.json` after manual correction (see
`docs/implementation-flow.md`).

## `strix-pentest.yml`: security scan

Trigger: **`workflow_dispatch` only** (scans cost OpenRouter credits). Installs
[Strix](https://strix.ai) and runs `strix -n -t ./ --scan-mode <input, default
quick>` with `STRIX_LLM=openai/gpt-5.6-luna` set inline,
`LLM_API_KEY` from the `OPENROUTER_API_KEY` secret and
`LLM_API_BASE=https://openrouter.ai/api/v1`.

## Secrets used by workflows

`CARGO_REGISTRY_TOKEN` (crates.io), `OPENROUTER_API_KEY` (wiki bot and Strix),
`LANGSMITH_API_KEY` (optional tracing). npm publishing uses OIDC instead of a
token. Never commit values.

## Local hooks and scripts

- `.githooks/pre-commit` (enable with `just setup-hooks`): runs
  `scripts/check-branch-name.sh <branch>` (shape only) and, on `development`
  outside merges, `just bump patch`. See [build & release](build-release.md).
- `scripts/check-branch-name.sh`: the shared checker; `--self-test` runs its
  case table.
- `scripts/uncovered-functions.py`: used by `just coverage`.
- `scripts/gen_os_logos.py`: regenerates `assets/os_logos_large.json` from
  fastfetch (see `scripts/README.md`).
