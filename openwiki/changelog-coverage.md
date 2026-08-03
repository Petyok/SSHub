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

| Release / entry | Wiki page(s) | Status | Notes |
| --- | --- | --- | --- |
| 0.13.0: Known hosts manager and connect fingerprint | [Known hosts manager](workflows/known-hosts.md), [Secrets](security/secrets.md) | Covered | `src/known_hosts.rs`, `src/tui/screens/known_hosts.rs`, session fingerprint path checked. |
| 0.13.0: Help/H key migration | [TUI dashboard](workflows/tui.md) | Covered | `src/keybinds.rs`, `src/config.rs` checked. |
| 0.13.0: OSC 52 PTY relay | [Sessions & SFTP](workflows/sessions-sftp.md), [Secrets](security/secrets.md) | Covered | Visibility boundary, bounds, config, and read refusal documented. |
| 0.13.0: Terminal-stream demo recording | [Integrations](integrations/external-terminals.md) | Covered | `demo/record.py` and timing-based asciicast pipeline checked. |
| 0.13.0: SFTP local path display | [Sessions & SFTP](workflows/sessions-sftp.md) | Covered | Local `$HOME` collapse and untouched remote paths documented. |
| 0.11.0: Session switcher/local shell/shared picker | [Sessions & SFTP](workflows/sessions-sftp.md), [TUI dashboard](workflows/tui.md) | Covered | `src/app/session_picker.rs`, session dispatch, and picker screens checked. |
| 0.11.0: Searchable Help/keybinding editor | [TUI dashboard](workflows/tui.md) | Covered | Filter and filtered-row rebinding behavior documented. |
| 0.11.0: Secret reveal/copy/delete and keyring fallback | [Secrets](security/secrets.md) | Covered | Credential storage, fallback migration, masking, and copy controls checked. |
| 0.11.0: Ad-hoc connect | [TUI dashboard](workflows/tui.md) | Covered | Validation and `--` argument boundary checked. |
| 0.11.0: Public-key push and key generation | [Hosts, Groups & Identities](domain/hosts-identities.md), [Secrets](security/secrets.md) | Pending | Source behavior audited; dedicated user workflow remains to be expanded. |
| 0.11.0: npm installation | [Build & Release](operations/build-release.md), [CI & automation](operations/ci-cd.md) | Pending | Packaging source audited; npm distribution details need a focused section. |
| 0.11.0: UI motion and panel/layout fixes | [TUI dashboard](workflows/tui.md) | Pending | Source and tests audited; motion-specific behavior is not fully represented. |
| 0.11.0: SFTP two-server/queue/dotfiles/parent and failure behavior | [Sessions & SFTP](workflows/sessions-sftp.md) | Covered | `src/app/sftp.rs`, model, worker, and tests checked. |
| 0.11.0: release profile and askpass upgrade fix | [Build & Release](operations/build-release.md), [Secrets](security/secrets.md) | Covered | Release profile and helper re-resolution checked. |
| 0.11.0: remaining session-strip, agent-panel, cursor-key, and SFTP fixes | [TUI dashboard](workflows/tui.md), [Sessions & SFTP](workflows/sessions-sftp.md), [Hosts, Groups & Identities](domain/hosts-identities.md) | Pending | Regression behavior remains distributed rather than explicitly catalogued. |
| 0.10.0: Broadcast commands | [TUI dashboard](workflows/tui.md) | Covered | The changelog names broadcast behavior, but no standalone `workflows/broadcast.md` concept exists; app/TUI wiring and tests were checked. |
| 0.10.0: panel focus/zoom and PuTTY/mRemoteNG import | [TUI dashboard](workflows/tui.md), [Hosts, Groups & Identities](domain/hosts-identities.md) | Pending | Source audited; detailed workflows remain to be added. |
| 0.10.0: Headless CLI | [Headless CLI](workflows/cli.md) | Covered | Command tree, JSON, exit codes, and guards checked. |
| 0.10.0: external launcher removal | [Integrations](integrations/external-terminals.md) | Covered | Stale launcher documentation corrected. |
| 0.9.0–0.8.0: transport, logging, tunnels, SFTP operations, inputs, version label | [Sessions & SFTP](workflows/sessions-sftp.md), [Tunnels](workflows/tunnels.md), [TUI dashboard](workflows/tui.md), [Secrets](security/secrets.md) | Covered | Existing pages and source were reviewed; no separate page needed. |
| 0.7.0–0.5.x: SFTP, groups, OS detection, selection, clipboard paste, log rendering | [Sessions & SFTP](workflows/sessions-sftp.md), [Hosts, Groups & Identities](domain/hosts-identities.md), [TUI dashboard](workflows/tui.md) | Covered | Existing canonical pages cover the product behavior; minor fixes remain implementation-level. |
| 0.4.0: AGPL relicensing | [Quickstart](quickstart.md) | Pending | License is authoritative in `LICENSE`; quickstart names the current license but lacks release-history detail. |
| 0.3.0–0.1.0: TUI foundation, config/import/hot reload, embedded sessions, initial launcher | [Quickstart](quickstart.md), [Runtime architecture](architecture/overview.md), [Data model](architecture/data-model.md), [Hosts, Groups & Identities](domain/hosts-identities.md), [Testing](testing/strategy.md) | Covered | Source and tests checked against canonical architecture/workflow pages. |

## Audit protocol

1. Read every release and `Unreleased` section in `CHANGELOG.md`.
2. Split entries by feature or behavior change and inspect implementation plus relevant tests.
3. Link each reviewed entry to exact concept pages; mark `Pending` only when the current wiki is materially incomplete.
4. Keep this notebook’s run metadata and unresolved list current.

## Last audit

- `gitHead`: `6e10d8891834a4e32c6850d0443460022e06c77c`
- `auditedAt`: `2026-08-03`
- `model`: `moonshotai/kimi-k3`
- `unresolved`: 0.11.0 public-key push/key generation; npm installation details; 0.11.0 motion and remaining regression fixes; 0.10.0 panel focus/zoom and PuTTY/mRemoteNG import; 0.4.0 relicensing history
