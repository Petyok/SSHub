---
type: Audit Notebook
title: Changelog to OpenWiki Coverage
description: Audit notebook mapping every CHANGELOG.md feature, fix, and behavior change to source-backed OpenWiki documentation.
resource: CHANGELOG.md
tags: [changelog, coverage, audit, openwiki]
openwiki:
  roles: [repository, testing]
  change_kinds: [documentation-audit]
  source_paths: [CHANGELOG.md]
---

# Changelog to OpenWiki Coverage

Every entry in `CHANGELOG.md` was reviewed against the current source tree and mapped to the narrowest canonical page. `Covered` means the behavior is represented sufficiently for a future change; `Pending` records a material documentation gap.

## Coverage ledger

| Release / reviewed entry | Wiki page(s) | Status | Evidence / disposition |
|---|---|---|---|
| 0.15.1: terminal query replies | [Sessions & SFTP](workflows/sessions-sftp.md#embedded-ssh-sessions-srcsession) | Covered | `src/session/parser.rs`, `src/session/mod.rs`, parser tests cover supported and intentionally unanswered CSI queries. |
| 0.15.1: bounded PTY writer | [Sessions & SFTP](workflows/sessions-sftp.md#embedded-ssh-sessions-srcsession) | Covered | `src/session/pty.rs` writer thread and queue behavior documented; session tests own regression coverage. |
| 0.15.0: `exec` | [Headless CLI](workflows/cli.md#command-tree), [Secrets](security/secrets.md#input-safety-details-worth-preserving) | Covered | `src/cli/exec.rs`, `tests/smoke/cli_commands.rs`; timeout, mosh, BatchMode, JSON, audit privacy documented. |
| 0.15.0: system OpenSSL opt-out | [Build & Release](operations/build-release.md#openssl-feature-boundary) | Covered | `Cargo.toml` feature wiring and release constraint documented. |
| 0.15.0: identity reload and dashboard notices | [Hosts, Groups & Identities](domain/hosts-identities.md), [TUI dashboard](workflows/tui.md) | Covered | `src/app/push_key.rs`, `src/app/mod.rs`; behavior grouped with owning workflows. |
| 0.14.2: profile-aware theme persistence | [Themes and Profiles](workflows/themes-profiles.md#runtime-themes) | Covered | `src/theme/manager.rs`, picker and e2e tests document real `Enter` persistence. |
| 0.14.2: leading-dash host injection | [Secrets](security/secrets.md#input-safety-details-worth-preserving), [Hosts, Groups & Identities](domain/hosts-identities.md) | Covered | `src/ssh/host.rs`, import and host validation paths; write and connect boundaries documented. |
| 0.14.0: runtime themes, profiles, transparency | [Themes and Profiles](workflows/themes-profiles.md), [Data model](architecture/data-model.md), [TUI dashboard](workflows/tui.md) | Covered | Theme/profile lifecycle and ownership are canonical in the new workflow page. |
| 0.14.0: known-host follow-ups and quoted aliases | [Known Hosts](workflows/known-hosts.md), [Hosts & Identities](domain/hosts-identities.md) | Covered | Parser, overlay, resolver, and focused tests checked. |
| 0.13.0: known-host manager, help migration, OSC 52 relay, demo recording, SFTP path display | [Known Hosts](workflows/known-hosts.md), [TUI dashboard](workflows/tui.md), [Sessions & SFTP](workflows/sessions-sftp.md), [Integrations](integrations/external-terminals.md) | Covered | Existing pages plus source/tests reviewed. |
| 0.11.0: session switcher, searchable overlays, secrets, ad-hoc connect, local shell, SFTP improvements | [TUI dashboard](workflows/tui.md), [Sessions & SFTP](workflows/sessions-sftp.md), [Secrets](security/secrets.md) | Covered | Existing canonical workflows and e2e/app tests cover behavior. |
| 0.11.0: public-key push/key generation | [Hosts, Groups & Identities](domain/hosts-identities.md), [Secrets](security/secrets.md) | Covered | Existing identity workflow plus `src/app/push_key.rs`, `src/app/keygen.rs` and identity tests. |
| 0.11.0: npm installation | [Build & Release](operations/build-release.md) | Covered | Packaging is grouped with release distribution; `npm/*`, `package.json`, and release workflow are evidence. |
| 0.11.0: motion and panel/layout fixes | [TUI dashboard](workflows/tui.md) | Covered | Grouped with UI lifecycle/rendering; source and app tests are the focused seam. |
| 0.10.0: broadcast, panel focus/zoom, PuTTY/mRemoteNG import, CLI, launcher removal | [TUI dashboard](workflows/tui.md), [Hosts & Identities](domain/hosts-identities.md), [Headless CLI](workflows/cli.md), [Integrations](integrations/external-terminals.md) | Covered | Runtime and import boundaries are represented; no separate broadcast concept needed. |
| 0.9.0–0.8.0: mosh, logging, tunnels, SFTP operations, input and release changes | [Sessions & SFTP](workflows/sessions-sftp.md), [Tunnels](workflows/tunnels.md), [Secrets](security/secrets.md), [Build & Release](operations/build-release.md) | Covered | Canonical workflow pages cover the shipped behavior and safety limits. |
| 0.7.0–0.5.x: SFTP, groups, OS detection, selection, clipboard, rendering | [Sessions & SFTP](workflows/sessions-sftp.md), [Hosts & Identities](domain/hosts-identities.md), [TUI dashboard](workflows/tui.md) | Covered | Existing source-backed pages cover product behavior and regression seams. |
| 0.4.0: AGPL relicensing | [Quickstart](quickstart.md), [Build & Release](operations/build-release.md) | Covered | Current license and release history are stated; `LICENSE` is authoritative. |
| 0.3.0–0.1.0: foundation, config/import/reload, embedded sessions, launcher | [Quickstart](quickstart.md), [Runtime Architecture](architecture/overview.md), [Data model](architecture/data-model.md), [Testing](testing/strategy.md) | Covered | Foundational behavior is represented in canonical architecture pages. |
| Unreleased | — | Covered | Section is empty in `CHANGELOG.md`; no source-backed entry to map. |

## Audit protocol

1. Read every release and `Unreleased` section in `CHANGELOG.md`.
2. Split entries by feature or behavior change and inspect implementation plus focused tests.
3. Link each reviewed entry to exact concept pages; use `Pending` only for a material gap.
4. Re-run this notebook audit when the changelog or source behavior changes.

## Last audit

- `gitHead`: `b042b846ba02cb8de811e014c5a9e5fcd3a0fd47`
- `auditedAt`: `2026-08-20`
- `model`: `openai/gpt-5.6-luna`
- `unresolved`: none
