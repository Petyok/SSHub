---
type: Workflow
title: TUI Dashboard — Tabs, Overlays, and Keybindings
description: The keyboard-driven SSHub dashboard — five tabs (hosts, SFTP, tunnels, identities, audit), overlay screens (palette, tag filter, settings, keybind editor, help), default keybindings, and how screens map to AppMode in src/app and src/tui/screens.
resource: src/app/keys.rs
tags: [tui, keybindings, dashboard, workflow, ux]
---

# TUI Dashboard

The TUI is a bento-grid dashboard rendered by `src/tui/` on top of the [App state machine](../architecture/overview.md). Tabs are switched with `1`..`5` and tracked as `App.active_tab: usize`; every overlay (forms, pickers, confirms) is an `AppMode` variant with its screen in `src/tui/screens/`.

## Tabs

| # | Tab | Screen | Backed by |
|---|---|---|---|
| 1 | Hosts | `screens/hosts.rs` | [hosts & groups](../domain/hosts-identities.md); detail panel with OS logo, fact sheet, latency sparkline |
| 2 | SFTP | `screens/sftp.rs` | [dual-pane transfer browser](sessions-sftp.md#sftp) |
| 3 | Tunnels | `screens/tunnels.rs` | [tunnel manager](tunnels.md) |
| 4 | Identities ("Keys") | `screens/keys.rs` | [identities & ssh-agent](../domain/hosts-identities.md#identities) |
| 5 | Audit | `screens/audit.rs` | `auth_events` table ([data model](../architecture/data-model.md)) |

## Key overlays

- **Fuzzy palette** (`/`, `screens/palette.rs`) — nucleo-powered quick-connect over all hosts.
- **Tag filter** (`#`, `screens/tag_filter.rs`) — multi-tag AND filter.
- **Settings** (`Ctrl+H`, `screens/settings.rs`) — session logging, opaque background, OS logos, quit confirmation, startup animation. Writes `[appearance]` / `[session_logging]` in `config.toml`.
- **Keybind editor** (`Ctrl+K`, `screens/keybind_editor.rs`) — rebinds any action; persisted to `[keybinds]` in `config.toml` via `src/keybinds.rs`.
- **Help** (`?`, `screens/help.rs`) — scrollable keybinding reference; also the only in-app place that warns session logs capture echoed secrets (see [secrets](../security/secrets.md)).
- **Group manager** (`Shift+G`, `screens/group_manage.rs`) and **host form** (`a`/`e`, `screens/host_form.rs`) — CRUD for [hosts, groups & identities](../domain/hosts-identities.md).
- **Session host picker** (`Ctrl+T`, `screens/session_host_picker.rs`) — opens a new [embedded session](sessions-sftp.md) tab.

## Default keybindings (highlights)

Global: `Esc` back, `q` quit, `?` help, `Ctrl+K` keybind editor. `Tab` toggles the detail panel only on the Hosts tab; on SFTP it switches panes, and it has no detail-panel effect on Tunnels/Identities/Audit.
Hosts: `Enter` connect, `a`/`e`/`d`/`D` add/edit/delete/duplicate, `f` favorite, `s` sort mode, `/` search, `#` tags, `Shift+I`/`Shift+E` import/export ssh config, `Shift+T` Termius import.
Sessions: `Ctrl+D` detach (SSH keeps running), `Ctrl+W` close tab, `Ctrl+[`/`Ctrl+]` cycle tabs, `Ctrl+Shift+S` focus session from dashboard.
Tunnels: `Enter` start/stop/cancel-reconnect, `R` reconnect settings, `x` kill process.
Audit: `f` status filter (all/ok/fail), `r` range (all/today/week/month).

The full binding table lives in `README.md` (`## Keybindings`); rebinds made in the keybind editor take precedence over these defaults. (The man page documents only the CLI subcommands, not TUI keybindings.)

## Mouse

Click tabs, select rows, scroll panels, double-click to connect (`src/app/mouse.rs`). Enabled via crossterm `EnableMouseCapture` in the [event loop](../architecture/overview.md).

## Change guidance

- New overlay = new `AppMode` variant + key dispatch in `src/app/keys.rs` + render dispatch in `src/tui/mod.rs` + screen file. Follow `host_form.rs` as the fullest example.
- Keep popups inside `fit_popup` — it exists because unclamped rects panic on small terminals.
- E2E coverage for mode transitions lives in `tests/e2e/` (see [testing](../testing/strategy.md)); add a scenario when adding a mode.
