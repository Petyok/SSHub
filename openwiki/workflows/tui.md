---
type: Workflow
title: TUI Dashboard - Tabs, Overlays, and Keybindings
description: The keyboard-driven SSHub dashboard with five tabs (hosts, SFTP, tunnels, identities, audit), the dashboard panel grid, every overlay (palette, session switcher, settings, theme picker, profiles, snippets, log browser, broadcast, keybind editor, help), default keybindings from src/keybinds.rs, and how screens map to AppMode.
resource: src/app/keys.rs
tags: [tui, keybindings, dashboard, workflow, ux]
---

# TUI Dashboard

The TUI is a bento-grid dashboard rendered by `src/tui/` on top of the [App state machine](../architecture/overview.md). Tabs are tracked as `App.active_tab: usize` (0-based); every overlay (forms, pickers, confirms) is an `AppMode` variant in `src/app/types.rs`, dispatched in `App::handle_key` (`src/app/keys.rs`) and drawn from `src/tui/mod.rs` with its screen under `src/tui/screens/`.

All bindings below are defaults from `KeybindsConfig::default()` in `src/keybinds.rs`; every `KeyAction` (86 of them, listed in `KeyAction::ALL`) can be rebound, so the code always reads the configured chord. A few overlay-internal keys are hard-coded and noted as such.

## Tabs

Order and digits come from `src/tui/widgets/tab_bar.rs` and `App::try_tab_switch`:

| Key | `active_tab` | Tab | Body renderer | Backed by |
|---|---|---|---|---|
| `1` / `h` | 0 | hosts | `render_hosts_body` in `src/tui/mod.rs` (widgets `hosts_panel`, `middle_stack`, `right_stack`) | [hosts & groups](../domain/hosts-identities.md) |
| `2` | 1 | sftp | `screens/sftp.rs` | [SFTP browser](sessions-sftp.md#sftp-srcsftp) |
| `3` | 2 | tunnels | `screens/tunnels.rs` | [tunnel manager](tunnels.md) |
| `4` / `i` | 3 | identities | `screens/keys.rs` | [identities & ssh-agent](../domain/hosts-identities.md#identities) |
| `5` | 4 | audit | `screens/audit.rs` | `auth_events` ([data model](../architecture/data-model.md)) |

Configs written before the SFTP tab existed are re-digited once by `KeybindsConfig::migrate_pre_sftp_tabs`.

## Hosts tab panels

The hosts tab is a panel grid (`PanelId` in `src/app/types.rs`): `Hosts` (left column), `Detail` / `Agent` / `Latency` (middle), `Recent` / `Auth` / `Ping` (right), an `SshLog` strip along the bottom, and a floating `Broadcast` panel while a [broadcast](broadcast.md) run is live. `Alt+Arrows` move focus spatially; `z` or `Alt+Enter` zooms the focused panel to the full body (Esc unzooms). In a zoomed panel, `j`/`k`/PageUp/PageDown scroll; Enter connects from the zoomed Ping or Recent panel; `f`/`r` cycle status and range in the zoomed Auth panel; `d` removes the selected key in the zoomed Agent panel (`ssh-add -d`, needs a file path); mouse drag copies the panel text via OSC 52.

## Overlays

| Overlay (`AppMode`) | Open with | Screen | Notes |
|---|---|---|---|
| Fuzzy palette (`Palette`) | `/` (hosts tab) | `palette.rs` | nucleo quick-connect over all hosts. A query shaped like `[user@]host[:port]` (bracketed IPv6 too) that matches no saved host adds an ad-hoc row (`src/app/adhoc.rs`); the destination goes after `--` and nothing is saved. Letters are always query text; arrows move. |
| Tag filter (`TagFilter`) | `#` | `tag_filter.rs` | Multi-tag AND filter; Space toggles live, Esc closes. Esc on the hosts tab clears active tag filters. |
| Session switcher (`SessionPicker`) | `Alt+S` (dashboard with sessions, or in a session) | `session_picker.rs` | One picker, three purposes (`SessionPickerPurpose`): switch session, new session tab (`Ctrl+T`), SFTP left-pane server (`o` in SFTP). See [sessions](sessions-sftp.md#embedded-ssh-sessions-srcsession). |
| Settings (`Settings`) | `Ctrl+H` (hard-coded, dashboard) | `settings.rs` | Rows from `SETTINGS_ITEMS`: Theme..., SSHub transparent, Session transparent, Show OS logos, Confirm before quit, Disable startup animation, Session logging, Ping hosts, Check for updates, Profiles..., Persist command history, Maximum commands per host (Left/Right, 1 to 2000), and three Enter-to-clear history rows. Space/Enter toggles and saves to the active `config.toml` immediately. |
| Theme picker (`ThemePicker`) | Enter on Settings "Theme..." | `theme_picker.rs` | Navigation previews live; only Enter persists. See [profiles & themes](profiles-themes.md). |
| Profile manager (`ProfilePicker`) | `Alt+P` (dashboard) or Settings "Profiles..." | `src/profile/picker.rs` | Create, rename, delete, switch, transfer. See [profiles & themes](profiles-themes.md). |
| Keybind editor (`KeybindEditor`) | `Ctrl+K` (dashboard) | `keybind_editor.rs` | Type to filter by label or bind text; Enter captures a replacement chord, `Ctrl+A` appends one, `Ctrl+R` resets to default, `Ctrl+X` unbinds. Saved to `[keybinds]`. |
| Help (`Help`) | `?` | `help.rs` | Searchable, scrollable reference; Esc clears the query before closing. Older `help = ["?", "Shift+H"]` configs lose `Shift+H` once (`migrate_help_frees_known_hosts`) because `H` opens the [known-hosts manager](known-hosts.md). |
| Group manager (`GroupManage`) / group form (`GroupForm`) | `Shift+G`; `Ctrl+G` or `e` on a group header edits the selected group; `Ctrl+Shift+G` deletes it | `group_manage.rs`, `group_form.rs` | Group form carries name, parent and group defaults (identity, username, port, ProxyJump, transport, agent forwarding). |
| Host form (`HostForm`) | `a` / `e` (hosts tab) | `host_form.rs` | Save with `F2`/`Ctrl+S`; `Ctrl+R` reveals and copies a stored secret, `Ctrl+Y` copies it. |
| Snippet manager (`SnippetManage`) | `Shift+S` (dashboard) | `snippet_manage.rs` | See [session tools](session-tools.md#command-snippets). |
| Session log browser (`LogBrowser`) | `Shift+L` (dashboard) | `log_browser.rs` | See [session tools](session-tools.md#session-log-browser). |
| Broadcast wizard (`BroadcastPickTarget` / `BroadcastCommand` / `BroadcastPreview`) | `b` (hosts tab) | `broadcast.rs` | See [broadcast](broadcast.md). |
| Push key pickers (`PushKeyIdentityPicker` / `PushKeyHostPicker`) | `Shift+P` on hosts tab (pick identity) or identities tab (pick host) | `push_key_pickers.rs` | Appends the public key to `~/.ssh/authorized_keys` through an embedded ssh session. |
| Keygen form (`KeygenForm`) | `g` (identities tab) | `keygen.rs` | ed25519 or rsa-4096; creates the identity row and stores the passphrase. |
| Known hosts (`KnownHosts`) | `H` (identities tab) | `known_hosts.rs` | [Known-hosts manager](known-hosts.md). |
| Tunnel reconnect settings (`TunnelReconnectSettings`) | `R` (tunnels tab, hard-coded) | `tunnel_reconnect.rs` | [Tunnels](tunnels.md#in-process-manager-srctunnelmodrs). |
| Auth modal (`AuthPrompt`) | raised by an ssh prompt | `auth.rs` | [Interactive authentication](session-tools.md#interactive-authentication-modal). |
| Notice (`Notice`) | errors that must not be missed | in `src/tui/mod.rs` | Any key dismisses. |
| Quit confirm (`ConfirmQuit`) | `q` / `Ctrl+C` when "Confirm before quit" is on, or always while an update is installing | | A second `Ctrl+C` forces the quit. |

## Default keybindings (highlights)

Global (dashboard): `q` quit, `Ctrl+C` force quit, `?` help, `Ctrl+K` keybind editor, `Ctrl+H` settings, `Alt+P` profiles, `Shift+L` log browser, `Shift+S` snippets, `Ctrl+Shift+T` local shell tab. `Tab` toggles the detail panel on the hosts tab and switches panes on SFTP.

Hosts: `Enter` connect (or fold on a group header), `a`/`e`/`d`/`Shift+D` add/edit/delete/duplicate, `f` favorite, `s` cycle sort (label, last connected, favorite first, group+label, manual), `Ctrl+Up`/`Ctrl+Down` move host in manual order, `Shift+Up`/`Shift+Down` jump between groups, `Space` fold, `Left`/`Right` fold in/out, `Shift+Z` collapse or expand all, `+`/`-` hosts-column zoom, `c` clear and `y` copy the SSH log, `Shift+I`/`Shift+E` import/export ssh config, `Shift+T` Termius import, `b` broadcast, `x` cancel broadcast or clear its error toasts, `Shift+P` push key, `Shift+U` update (install a known release, else check now; hosts tab only).

Sessions on the dashboard (only while at least one session exists): `Ctrl+Shift+S` resume, `Ctrl+[`/`Ctrl+]` (or `Ctrl+PageUp`/`Ctrl+PageDown`) move along the session strip, `Alt+S` switcher, `Ctrl+T` new tab, `Ctrl+W` close, `Ctrl+Shift+F` SFTP for the selected session's host. In-session keys are on the [sessions page](sessions-sftp.md#embedded-ssh-sessions-srcsession).

Identities: arrows move in the card grid, `[`/`]` fewer/more columns, `a` add, `g` generate, `e` edit, `d` delete, `p`/`r` add to / remove from ssh-agent, `Shift+P` push key, `H` known hosts, Esc back to hosts.

Tunnels: `Enter` start/stop (also cancels a pending reconnect), `a`/`e`/`d` add/edit/delete, `x` kill process, `R` reconnect settings.

Audit: `f` status filter (all/ok/fail), `r` range (all/24h/week/month).

The footer hints are built per tab in `footer_keybinds` (`src/tui/mod.rs`) and pin `resume`, `help`, `quit` (and an available update) to the tail so truncation never hides them. The full binding table is in `README.md` (`## Keybindings`) and the help overlay; the man page covers the CLI only.

## Mouse

`src/app/mouse.rs`, enabled through crossterm mouse capture in the [event loop](../architecture/overview.md):

- Click a tab to switch; click a host row to select, double-click (within 400 ms) to connect; click a group header to fold. Tunnels: click selects, double-click toggles. Identities: click selects a card. Audit: click selects a row; clicking a status or range label in the top strip applies it.
- Wheel: hosts column moves the selection by 3, elsewhere on the hosts tab it scrolls the SSH log; tunnels/identities/audit move the selection by 1. A zoomed panel takes the wheel and drag-to-copy.
- Overlays ignore the mouse (only `AppMode::Normal`, `Connecting` and `Session` handle it). In-session mouse rules are on the [sessions page](sessions-sftp.md#mouse-and-selection-in-a-session).

## Change guidance

- New overlay = new `AppMode` variant + key dispatch in `src/app/keys.rs` + render dispatch in `src/tui/mod.rs` + screen file. Follow `host_form.rs` as the fullest example.
- New rebindable key = `KeyAction` variant, `ALL` entry, `label`, `KeybindsConfig` field with a default, and both `binds`/`set` arms (`src/keybinds.rs`); `an_empty_table_loads_every_default` fails if a default is missing.
- Keep popups inside `fit_popup`: unclamped rects panic on small terminals.
- E2E coverage for mode transitions lives in `tests/e2e/` (see [testing](../testing/strategy.md)); add a scenario when adding a mode.
