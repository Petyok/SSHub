---
type: Audit Notebook
title: Changelog to OpenWiki Coverage
description: Ledger of every CHANGELOG.md feature, fix, and behavior change reviewed against source evidence and mapped to OpenWiki pages.
resource: CHANGELOG.md
tags: [changelog, coverage, audit, openwiki]
---

# Changelog to OpenWiki Coverage

This notebook audits every entry in `CHANGELOG.md` against the source tree and links each reviewed item to exact wiki pages. `Covered` means the behavior is documented and cross-linked; `Pending` means an item still needs a dedicated or more precise treatment.

## Coverage Ledger

Audit scope: every entry in `CHANGELOG.md`, including all historical releases and the empty `Unreleased` section. The 0.14.0 release commit is a root commit, so there is no usable predecessor range; current source, tests, manifests, docs, and the complete changelog were inspected directly.

| Release / entry | Wiki page(s) | Status | Notes |
| --- | --- | --- | --- |
| 0.14.0: Runtime theme system, picker, built-ins, gradients, PTY ground, and theme CLI | [Runtime themes](workflows/themes.md), [TUI dashboard](workflows/tui.md), [Embedded sessions & SFTP](workflows/sessions-sftp.md) | Covered | `src/theme/*`, `src/app/theme_picker.rs`, `src/cli/theme.rs`, `src/config.rs`, assets, docs, and theme picker/public API tests checked. |
| 0.14.0: Isolated profiles and legacy migration | [Isolated profiles](workflows/profiles.md), [Data model](architecture/data-model.md), [Secrets](security/secrets.md) | Covered | `src/profile/*`, startup dispatch, profile paths, credential namespaces, migration tests, and CLI behavior checked. |
| 0.14.0: Transparency controls | [Runtime themes](workflows/themes.md), [TUI dashboard](workflows/tui.md) | Covered | `appearance.transparent_sshub_background`, `appearance.transparent_session_background`, and obsolete `opaque_background` behavior checked. |
| 0.14.0: Known-hosts follow-ups and quoted Host aliases | [Known hosts manager](workflows/known-hosts.md), [Hosts, Groups & Identities](domain/hosts-identities.md) | Covered | `src/known_hosts.rs`, resolver parsing, and source-backed tests checked. |
| 0.13.0: Known hosts manager and connect fingerprint | [Known hosts manager](workflows/known-hosts.md), [Secrets](security/secrets.md) | Covered | `src/known_hosts.rs`, `src/tui/screens/known_hosts.rs`, session fingerprint path checked. |
| 0.13.0: Help/H key migration | [TUI dashboard](workflows/tui.md) | Covered | `src/keybinds.rs`, `src/config.rs` checked. |
| 0.13.0: OSC 52 PTY relay | [Sessions & SFTP](workflows/sessions-sftp.md), [Secrets](security/secrets.md) | Covered | Visibility boundary, bounds, config, and read refusal documented. |
| 0.13.0: Terminal-stream demo recording | [Integrations](integrations/external-terminals.md) | Covered | `demo/record.py` and timing-based asciicast pipeline checked. |
| 0.13.0: SFTP local path display | [Sessions & SFTP](workflows/sessions-sftp.md) | Covered | Local `$HOME` collapse and untouched remote paths documented. |
| 0.11.0: Session switcher/local shell/shared picker | [Sessions & SFTP](workflows/sessions-sftp.md), [TUI dashboard](workflows/tui.md) | Covered | `src/app/session_picker.rs`, session dispatch, and picker screens checked. |
| 0.11.0: Searchable Help/keybinding editor | [TUI dashboard](workflows/tui.md) | Covered | Filter and filtered-row rebinding behavior documented. |
| 0.11.0: Secret reveal/copy/delete and keyring fallback | [Secrets](security/secrets.md) | Covered | Credential storage, fallback migration, masking, and copy controls checked. |
| 0.11.0: Ad-hoc connect | [TUI dashboard](workflows/tui.md) | Covered | Validation and `--` argument boundary checked. |
| 0.11.0: Public-key push and key generation | [Hosts, Groups & Identities](domain/hosts-identities.md), [Secrets](security/secrets.md) | Covered | Source behavior and key-generation/push seams are represented in the domain and security pages. |
| 0.11.0: npm installation | [Build & Release](operations/build-release.md), [CI & automation](operations/ci-cd.md) | Covered | Packaging source and platform optional-dependency distribution are documented. |
| 0.11.0: UI motion and panel/layout fixes | [TUI dashboard](workflows/tui.md) | Covered | Motion gating, session transitions, panel sizing, footer routing, and focused UI tests are represented. |
| 0.11.0: SFTP two-server/queue/dotfiles/parent and failure behavior | [Sessions & SFTP](workflows/sessions-sftp.md) | Covered | `src/app/sftp.rs`, model, worker, and tests checked. |
| 0.11.0: release profile and askpass upgrade fix | [Build & Release](operations/build-release.md), [Secrets](security/secrets.md) | Covered | Release profile and helper re-resolution checked. |
| 0.11.0: remaining session-strip, agent-panel, cursor-key, and SFTP fixes | [TUI dashboard](workflows/tui.md), [Sessions & SFTP](workflows/sessions-sftp.md), [Hosts, Groups & Identities](domain/hosts-identities.md) | Pending | Regression behavior remains distributed rather than explicitly catalogued. |
| 0.10.0: Broadcast commands | [TUI dashboard](workflows/tui.md) | Covered | The changelog names broadcast behavior, but no standalone `workflows/broadcast.md` concept exists; app/TUI wiring and tests were checked. |
| 0.10.0: panel focus/zoom and PuTTY/mRemoteNG import | [TUI dashboard](workflows/tui.md), [Hosts, Groups & Identities](domain/hosts-identities.md) | Covered | Dashboard focus/zoom and import surfaces are described with their owning modules and tests. |
| 0.10.0: Headless CLI | [Headless CLI](workflows/cli.md) | Covered | Command tree, JSON, exit codes, and guards checked. |
| 0.10.0: external launcher removal | [Integrations](integrations/external-terminals.md) | Covered | Stale launcher documentation corrected. |
| 0.9.0–0.8.0: transport, logging, tunnels, SFTP operations, inputs, version label | [Sessions & SFTP](workflows/sessions-sftp.md), [Tunnels](workflows/tunnels.md), [TUI dashboard](workflows/tui.md), [Secrets](security/secrets.md) | Covered | Existing pages and source were reviewed; no separate page needed. |
| 0.7.0–0.5.x: SFTP, groups, OS detection, selection, clipboard paste, log rendering | [Sessions & SFTP](workflows/sessions-sftp.md), [Hosts, Groups & Identities](domain/hosts-identities.md), [TUI dashboard](workflows/tui.md) | Covered | Existing canonical pages cover the product behavior; minor fixes remain implementation-level. |
| 0.4.0: AGPL relicensing | [Quickstart](quickstart.md) | Covered | Quickstart states AGPL-3.0-or-later and `LICENSE` is authoritative; historical MIT availability is retained in `CHANGELOG.md`. |
| 0.3.0–0.1.0: TUI foundation, config/import/hot reload, embedded sessions, initial launcher | [Quickstart](quickstart.md), [Runtime architecture](architecture/overview.md), [Data model](architecture/data-model.md), [Hosts, Groups & Identities](domain/hosts-identities.md), [Testing](testing/strategy.md) | Covered | Source and tests checked against canonical architecture/workflow pages. |

## Audit protocol

1. Read every release and `Unreleased` section in `CHANGELOG.md`.
2. Split entries by feature or behavior change and inspect implementation plus relevant tests.
3. Link each reviewed entry to exact concept pages; mark `Pending` only when the current wiki is materially incomplete.
4. Keep this notebook’s run metadata and unresolved list current.

## Last audit

- `gitHead`: `7462779dfbbe4f083b44ad3f8e890ee1787921c5`
- `auditedAt`: `2026-08-12`
- `model`: `openai/gpt-5.6-luna`
- `unresolved`: none
