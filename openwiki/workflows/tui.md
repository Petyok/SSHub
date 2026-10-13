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

- **Fuzzy palette** (`/`, `screens/palette.rs`) — nucleo-powered quick-connect over all hosts. An unknown validated `[user@]host[:port]` (including bracketed IPv6) becomes an ad-hoc connect row; the destination is passed after `--` and is not saved.
- **Session picker** (`Alt+S`, `screens/session_picker.rs`) — searches open sessions and jumps to one; the same picker supports new sessions and the SFTP left-pane host selection. `Ctrl+Shift+T` from the picker opens a local-shell tab.
- **Tag filter** (`#`, `screens/tag_filter.rs`) — multi-tag AND filter.
- **Settings** (`Ctrl+H`, `screens/settings.rs`) — session logging, opaque background, OS logos, quit confirmation, startup animation. Writes `[appearance]` / `[session_logging]` in `config.toml`.
- **Keybind editor** (`Ctrl+K`, `screens/keybind_editor.rs`) — rebinds any action; persisted to `[keybinds]` in `config.toml` via `src/keybinds.rs`.
- **Help** (`?`, `screens/help.rs`) — searchable, scrollable keybinding reference; `Esc` clears a query before closing. The keybinding editor is searchable too and rebinds the filtered action; its row actions use `Ctrl+A` / `Ctrl+R` / `Ctrl+X`. Help is now `?` only; older `help = ["?", "Shift+H"]` settings migrate once because `H` opens the [known-hosts manager](known-hosts.md). Help is also the only in-app place that warns session logs capture echoed secrets (see [secrets](../security/secrets.md)).
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

Mouse capture is enabled via crossterm `EnableMouseCapture` in the [event loop](../architecture/overview.md); `App::handle_mouse` (`src/app/mouse.rs`) routes each event by mode. In sessions: terminal-style selection over the PTY. Everywhere else the left button goes through `App::mouse_select`: a drag selects what is on screen and copies it on release (`apply_panel_selection` in `src/tui/mod.rs` scrapes the drawn cells: the whole screen on the dashboard and under every overlay, `is_overlay_mode`, the body over a zoomed panel), and a press and release without movement is a click instead, which copies nothing. On the dashboard a click (`App::dashboard_click`, which also detects double-clicks) picks a tab or a row, focuses the hosts-tab panel under it (`panel_zoom_source` rects), and on the SFTP tab picks a picker host or a browser entry; a double-click connects, starts a tunnel or opens a folder. The SFTP browser records its pane rows in `App::click_map` as `ClickHit::SftpPane`, which the click and the wheel read; the other tabs still mirror their renderer's scroll math in `src/app/mouse.rs`.

Over an overlay that click goes to `App::click_at` (`src/app/field_click.rs`), which reads `App::click_map`: one-row targets the renderers record every frame (the map is cleared at the start of each frame) saying what was drawn where, a form field row (`ClickHit::Field`, focus it with the cursor under the pointer) or a picker row (`ClickHit::Row`, highlight it without choosing). The host, identity, keygen and group forms and the SFTP, import and broadcast prompts build a `screens::FormBody`, a paragraph that also knows which line shows which field; `FormBody::record_clicks` lays each field line out with ratatui's own `Wrap`, so word-wrapped values map exactly. The tunnel and snippet forms record their one-row fields directly, and pickers their rows (mostly through `App::record_list_rows`). While a popup slides open, `render_popup_open` moves its targets by the same offset it draws the popup at.

## Change guidance

- New overlay = new `AppMode` variant + key dispatch in `src/app/keys.rs` + render dispatch in `src/tui/mod.rs` + screen file. Follow `host_form.rs` as the fullest example.
- Keep popups inside `fit_popup` — it exists because unclamped rects panic on small terminals.
- E2E coverage for mode transitions lives in `tests/e2e/` (see [testing](../testing/strategy.md)); add a scenario when adding a mode.
