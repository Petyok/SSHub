---
type: Workflow
title: Profiles & Themes - switching workspaces, moving data between them, and picking a theme
description: User workflows for isolated profiles (startup picker, --profile, the Alt+P profile manager with create/rename/delete/switch and the staged cross-profile transfer) and for color themes (Settings theme picker with live preview, user theme files, and the headless `sshub theme` commands). Internals live in the architecture profiles and themes pages.
resource: src/app/profile_manager.rs
tags: [profiles, themes, transfer, settings, workflow, tui]
---

# Profiles & Themes

Storage layout, migration and the transfer engine are described in [profiles architecture](../architecture/profiles.md); theme files, slots and the registry in [themes architecture](../architecture/themes.md). This page covers what a user does.

## Profiles

A profile is an isolated workspace with its own `launcher.db`, `metadata.db`, `config.toml` (settings, keybinds, theme), ssh_config source, logs, tunnel PID files and credential namespace, under `~/.local/share/sshub/profiles/<name>/`. Setting `SSHUB_CONFIG_DIR` or `SSHUB_DATA_DIR` switches to compatibility mode: no profiles, `--profile` is rejected, and the profile manager is unavailable.

**Choosing at startup**: with one profile startup is unchanged; with several, a picker appears after the splash. `sshub --profile NAME` skips it, `sshub --manage-profiles` forces it. Headless commands use `--profile` or the last-used profile and never show a picker ([CLI](cli.md)).

**Profile manager** (`Alt+P` on the dashboard, or Enter on Settings "Profiles..."; `AppMode::ProfilePicker`, `src/app/profile_manager.rs` over `src/profile/picker.rs`). With a single profile it opens straight into creating a second one. List keys:

| Key | Action |
|---|---|
| `j`/`k`, arrows, `1`..`9` | Move / jump |
| `Enter` | Switch to the highlighted profile (closes if it is already active) |
| `n` | Create a profile (name editor; Enter saves, Esc cancels) |
| `r` | Rename (not the active profile) |
| `d` | Delete with a `y` confirmation (not the active profile, not the last one) |
| `t` / `T` | Start a cross-profile transfer |
| `Esc` | Close |

Switching happens in place (`apply_pending_profile` in `src/lib.rs` swaps in a fresh `App` for the new profile). It is refused while any of these is live (`profile_switch_blocker`): an embedded or local-shell session, an SFTP connection or transfer, a broadcast, or a TUI tunnel or pending reconnect. The footer advertises the key as "Add profile" (one profile) or "Manage profiles" with the active name.

### Moving data between profiles

`t` in the manager list starts a staged flow (`src/app/profile_transfer.rs`, engine `src/store/transfer.rs`):

1. **Destination**: type to fuzzy-filter the other profiles, arrows move, Enter picks.
2. **Scope**: `h` the host selected on the hosts tab, `g` the group selected in the group manager (open it with `Shift+G` first), `i` the selected identity, `t` the selected tunnel.
3. **Group mode** (groups only): `g` group only (member hosts stay) or `w` group with its hosts.
4. **Plan**: the planned items with renames and blocked reasons. `m` toggles copy/move (move deletes the originals after copying and carries the moved hosts' saved passwords and the moved identities' passphrases into the destination profile's credential store, then removes them from the source; a copy carries no secrets; a host or identity whose secret could not be carried is not deleted from the source and a popup lists it; a secret that could not be removed from the source is reported too), `w` toggles with-hosts for a group, `y` applies; an empty plan refuses. `Esc` cancels at any step.

**Host transfer screen** (`m` at the scope step, `src/app/host_transfer.rs`, `src/tui/screens/host_transfer.rs`): an SFTP-style dual-pane screen. Left = the active profile's hosts, right = the destination profile's, both grouped (group headers, "(no group)" last) and alphabetical. `↑↓`/`PageUp`/`PageDown` move, `Tab` switches pane, `/` filters the focused pane (name, address or group; `Esc` clears, `Enter` keeps), `→` stages the left selection for a **push**, `←` stages the right selection for a **pull**, `Enter`/`Space` stage toward the other pane, `a` stages every shown host, a group row stages its shown hosts, `u` unstages the last, `x` clears the queue, `c` continues, `Esc` cancels the whole transfer. A queue strip lists what is staged; staged hosts wear a `→`/`←` badge. Pushes and pulls can be mixed: they run one direction at a time, push first, each as its own guided migration (below) ending in its own plan review and `y`. A pull reads the destination profile and writes the active one; `m` copy/move, the secret carry and the loss notes work the same way (the other profile's credential store is the source). Live sessions and running tunnels only block pushes, because only the active profile has them.

**Guided migration** (what `c` starts for each direction): up to five checklists with the staged hosts pre-checked, then the same plan review.

1. **Hosts**: the hosts staged for this direction, checked; `Space` toggles, `a` all, `n` none, `Enter` next (needs one host). Reachable with `b` from the first dependent step.
2. **Groups** the chosen hosts belong to (Favorites excluded), all checked. A group lands flattened at top level with its defaults (username, port, ProxyJump, transport, agent forwarding); each chosen host is wired to it. An unchecked group stays behind and its hosts land without it. A move deletes a group from the source only when none of its hosts stays there; otherwise the source keeps it.
3. **Tunnels** of the chosen hosts, all checked.
4. **Identities** the chosen hosts use, plus the default identity of every chosen group, all checked. An unchecked identity stays behind and its host (or group) lands without one.
5. **Audit logs**: the chosen hosts that have audit events, all checked. The events land under the host's destination name with their original timestamps; a move deletes them from the source (only for hosts it actually moved). Session log files are not copied, `log_path` keeps pointing at the source profile.

A step with nothing to list is skipped. `b` goes back a step (from the plan review to the last step, from the first step back to the host screen with the queue intact); going back and forward again keeps the rows you unchecked (rows of hosts you dropped disappear). Hosts that land without an unchecked group or identity are listed in a `note:` line of the plan review. A ProxyJump naming a host that is not migrated dangles. A move leaves a tunnel you unchecked in the source without a host and with auto-connect off, and carries the moved hosts' saved passwords to the destination. The plan review lists `[audit] events of 'host'` rows; `m` copy/move and `y` work as above. Only checked rows travel (`TransferSelection::no_auto_carry`). The wizard names tunnels by `tunnel#<id>`, and the planner falls back to that handle whenever a label is shared with another tunnel, so a shared label never reaches a tunnel of an unchosen host.

Identities carry the key path; saved passwords and passphrases travel only on a move; the reserved Favorites group cannot be transferred. The same engine ([cross-profile transfers](../architecture/profiles.md#cross-profile-transfers)) backs `sshub host|group|identity|tunnel transfer ... --to PROFILE` ([CLI](cli.md)).

## Themes

**Theme picker**: `Ctrl+H` opens Settings; Enter on the first row, "Theme...", opens `AppMode::ThemePicker` (`src/app/theme_picker.rs`, screen `src/tui/screens/theme_picker.rs`). Moving the selection previews that theme on the whole UI immediately; nothing is written until Enter.

| Key | Action |
|---|---|
| `j`/`k`, arrows, PageUp/PageDown, Home/End | Move and preview |
| `Shift+Up` / `Shift+Down` | Scroll the diagnostics footer (ignored roles, warnings) |
| `r` | Reload theme files from disk |
| `Enter` | Save the previewed theme as `appearance.active_theme` in the active `config.toml` |
| `Esc` | Restore the theme that was active when the picker opened |

Rows that cannot be previewed or saved (invalid files) are listed with their reason.

**User themes** are `*.toml` files whose file stem is the theme id. The TUI loads them from the `themes/` directory beside the active `config.toml`, which is `~/.local/share/sshub/profiles/<name>/themes/` in profile mode and `~/.config/sshub/themes/` in compatibility mode (`App::new_with_profile`). `sshub theme list|show` read the same directory for the profile `--profile` names, else the last used one (`profile::locate_config_dir`, no side effects; the config directory in compatibility mode). `src/cli/theme.rs`:

```bash
sshub theme list                                   # built-ins and installed themes, with state
sshub theme show aqua > ~/.local/share/sshub/profiles/<name>/themes/aqua-custom.toml
sshub theme check ~/.local/share/sshub/profiles/<name>/themes/aqua-custom.toml
sshub theme show aqua-custom --resolved --format json
```

`theme check` exits 0 when valid (warnings allowed), 1 on validation or file errors, 2 on usage errors, and never opens a database. The theme format is documented in `docs/theme-system.md`.

## Change guidance

- Profile manager and transfer UI: `src/app/tests/profile_manager.rs`, `src/app/tests/transfer_plan_confirm.rs`; engine and CLI: `tests/transfer_engine.rs`, `tests/transfer_cli.rs`.
- Theme picker: `src/app/tests/theme_picker.rs`, `tests/e2e/theme_picker.rs`. Only `commit_theme_picker` may write; navigation, reload and cancel must stay in memory.
