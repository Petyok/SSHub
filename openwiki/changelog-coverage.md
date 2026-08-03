---
type: Audit Notebook
title: Changelog to OpenWiki Coverage
description: Ledger of changelog entries reviewed against OpenWiki pages.
resource: CHANGELOG.md
tags: [changelog, coverage, audit, openwiki]
---

# Changelog to OpenWiki Coverage

This notebook records which changelog entries have corresponding, accurate
OpenWiki coverage. OpenWiki updates it during every regeneration run. Entries
must link to specific wiki pages, not only broad index pages. `Pending` means
source and wiki comparison still needs work; `Covered` means page content and
cross-links were checked against current code.

## Coverage Ledger

| Changelog entry | Wiki page(s) | Status | Notes |
| --- | --- | --- | --- |
| 0.13.0: Known hosts manager | Pending | Pending | Verify overlay, keybinding, deletion guards, fingerprint flow, and security behavior. |
| 0.13.0: OSC 52 clipboard relay | Pending | Pending | Verify session-only relay, bounds, config, and clipboard-read refusal. |
| 0.13.0: Terminal-stream demo recording | Pending | Pending | Verify recording pipeline and timing guarantees. |
| 0.13.0: SFTP local path display | Pending | Pending | Verify local `$HOME` collapsing and remote path behavior. |
| 0.13.0: Broadcast commands | [Broadcast commands](workflows/broadcast.md) | Covered | Verify against `src/broadcast/` and app/TUI wiring on each run. |

## Run Protocol

1. Read `CHANGELOG.md` from last recorded `gitHead` through current `HEAD`.
2. Split entries by feature or behavior change; do not collapse unrelated items.
3. Inspect implementation and tests for each entry.
4. Link each covered entry to exact wiki pages and mark status.
5. Add or revise pages and indexes for pending entries.
6. Record current `gitHead`, run date, and unresolved entries below.

## Last Audit

- `gitHead`: `a45a2a4`
- `auditedAt`: `2026-08-03`
- `model`: `openai/gpt-5.6-luna`
- `unresolved`: Known hosts manager, OSC 52 clipboard relay, terminal-stream demo recording, SFTP local path display
