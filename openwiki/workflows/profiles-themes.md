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
4. **Plan**: the planned items with renames and blocked reasons. `m` toggles copy/move (move deletes the originals after copying), `w` toggles with-hosts for a group, `y` applies; an empty plan refuses. `Esc` cancels at any step.

Identities carry the key path only, never secrets; the reserved Favorites group cannot be transferred. The same engine ([cross-profile transfers](../architecture/profiles.md#cross-profile-transfers)) backs `sshub host|group|identity|tunnel transfer ... --to PROFILE` ([CLI](cli.md)).

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

**User themes** are `*.toml` files whose file stem is the theme id. The TUI loads them from the `themes/` directory beside the active `config.toml`, which is `~/.local/share/sshub/profiles/<name>/themes/` in profile mode and `~/.config/sshub/themes/` in compatibility mode (`App::new_with_profile`). The headless commands read `~/.config/sshub/themes/` (`src/cli/theme.rs`):

```bash
sshub theme list                                   # built-ins and installed themes, with state
sshub theme show aqua > ~/.config/sshub/themes/aqua-custom.toml
sshub theme check ~/.config/sshub/themes/aqua-custom.toml
sshub theme show aqua-custom --resolved --format json
```

`theme check` exits 0 when valid (warnings allowed), 1 on validation or file errors, 2 on usage errors, and never opens a database. The theme format is documented in `docs/theme-system.md`.

## Change guidance

- Profile manager and transfer UI: `src/app/tests/profile_manager.rs`, `src/app/tests/transfer_plan_confirm.rs`; engine and CLI: `tests/transfer_engine.rs`, `tests/transfer_cli.rs`.
- Theme picker: `src/app/tests/theme_picker.rs`, `tests/e2e/theme_picker.rs`. Only `commit_theme_picker` may write; navigation, reload and cancel must stay in memory.
