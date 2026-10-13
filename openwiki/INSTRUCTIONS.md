---
type: Maintenance Guide
title: OpenWiki Instructions and Invariant Checklist
description: The OpenWiki generator prompt and the checklist of source facts (schema version, tabs, auth_events columns, event loop order, audit refresh, tunnel APIs, channels, themes, profiles) to verify against src/ before merging wiki changes.
resource: src/lib.rs
tags: [openwiki, maintenance, checklist, documentation]
---

# OpenWiki Instructions

## Generator prompt

A code wiki for this local repository. Prioritize a concise quickstart, architecture overview, source map, key workflows, domain concepts, operations/runbook notes, testing guidance, and integration points. Inspect git history to understand reasoning behind code changes and the progression of the repository. Keep pages grounded in the repository structure and recent code changes. Prefer practical navigation for engineers over generic summaries.

The bot that runs it is `.github/workflows/openwiki-update.yml` (manual
dispatch only; see [CI & automation](operations/ci-cd.md#openwiki-updateyml-wiki-bot)).
Its output is a starting point, not ground truth.

## Invariant checklist (verify against code before merge)

When editing or reviewing OpenWiki pages, confirm these facts still match
`src/`. Values below were verified at v0.17.7 (`0ffbc22`).

| Topic | Ground truth |
|-------|--------------|
| **Schema** | `SCHEMA_VERSION` **19** in `src/store/migrate.rs`. v14 `snippets`, v15 `log_bookmarks`, v16 `command_history` + `host_groups.default_*` + nullable `hosts.port` / `forward_agent` / `transport`, v17 membership repair, v18 `hosts.ping`, v19 `hosts.secret_lookup`. |
| **Migration snapshot** | `backup_before_migration` writes `launcher.db.bak-v<from>-<unix-ts>` with `VACUUM INTO` before any upgrade from version 1..18; `KEPT_BACKUPS = 3`; best-effort. |
| **Tabs** | Five dashboard tabs, `App.active_tab: usize` **0..4**: 0 hosts, 1 SFTP, 2 tunnels, 3 identities (keychain), 4 audit. Not an enum. |
| **Tab key handlers** | `AppMode::Normal` dispatch in `src/app/keys.rs::handle_key`: `handle_key_normal` (0 and fallback), `handle_key_sftp` (1), `handle_key_tunnels` (2), `handle_key_keychain` (3), `handle_key_audit` (4). |
| **AppMode** | 40 variants in `src/app/types.rs` (includes `AuthPrompt`, `ThemePicker`, `ProfilePicker`, `KeygenForm`, `LogBrowser`, `SnippetManage` / `SnippetForm` / `SnippetPicker`, `BroadcastPickTarget` / `BroadcastCommand` / `BroadcastPreview`, `KnownHosts`, `Notice`). |
| **`auth_events` columns** | `id`, `host_name`, `username`, `via`, `status`, `note`, `created_at`, `log_path`. The audit tab shows `note` (with `(logs in <path>)` appended when `log_path` is set, `audit_note`) on a detail line above the table, not as per-row columns. |
| **Event loop order** | `run_terminal_loop` (`src/lib.rs`): `terminal.size()` → every session `drain` (+ `resize` on size change) → key-push audit → `poll_authentication` → `relay_visible_session_clipboard` → diagnostics to SSH log → `Connecting`→`Session` promotion → `refresh_agent_info` → draw → `poll_keys_and_watcher` → `apply_pending_profile` → quit check. `poll_keys_and_watcher`: all queued key/mouse/paste events (`POLL_INTERVAL` 50 ms, 16 ms while animating) → watcher → ping → `sftp_rx` → `sftp_rx2` → `os_detect_rx` → `poll_update` → `tick_broadcast` → `tick_tunnels` → selection autoscroll → `refresh_auth_cache` → `detect_tab_switch`. Resize comes from `terminal.size()`, not crossterm resize events. Headless = one `TestBackend` 80x24 frame. |
| **Audit cache refresh** | `refresh_auth_cache()`: 10 s throttle in the poll loop, up to 500 rows, respects `audit_filter` / `audit_range`, also refreshes 7-day stats. `refresh_audit_events()`: immediate on filter/range change and on switching to tab 4 (key or mouse). |
| **Tunnel APIs** | `App::tick_tunnels()` (`src/app/tunnels.rs`): `bootstrap_auto_connect_tunnels` once → `sync_tunnels_from_store` every 2 s → `TunnelManager::check_health` → `tunnel::log_tunnel_reconnect_events` → `tick_tunnel_reconnect` (→ `TunnelManager::tick_reconnect`). Manager in `src/tunnel/mod.rs`, spawn and PID files in `src/tunnel/spawn.rs`. |
| **Channels** | `watcher_rx` (`ConfigWatcher`), `ping_rx`, `sftp_rx`, `sftp_rx2`, `os_detect_rx`, `update.rx` (`UpdateEvent`), `broadcast: Option<BroadcastState>` with `rx`. There is no `probe_rx`; `src/ssh/probe.rs` only defines `SshLogEntry` / `LogLevel`. |
| **Workers** | Ping every 30 s (`ping -c 1 -W 1`) only when `ping_live` and `[ping] enabled` or a host override says on; watcher debounce `WATCHER_DEBOUNCE` 300 ms on the ssh config's parent dir. |
| **AppDeps** | `resolver`, `metadata`, `store`, `password_store`. No `TerminalLauncher` exists any more. |
| **Askpass** | `maybe_run_askpass()` first in `main.rs`. Embedded interactive ssh: private socket channel (`src/session/askpass_channel.rs`, env `SSHUB_ASKPASS_SOCKET` / `_TOKEN` / `_CHANNEL`, 120 s timeout). Everything else with a stored secret: staged 0600 file (`SSHUB_ASKPASS_FILE`, `SSHUB_ASKPASS_OWNER`, jump-hop guard, `SSHUB_ASKPASS_READ_ONCE` for detached tunnels). |
| **Host-key options** | Embedded ssh sessions: `StrictHostKeyChecking=ask`, `NumberOfPasswordPrompts=3`, `LogLevel=DEBUG1`. CLI connect/exec, tunnels, key push with a secret, broadcast, OS detect: `accept-new`. |
| **Credentials** | `NamespacedPasswordStore` prefix `profile:<id>:` (empty in compat mode) over `OsKeyring` (service `sshub`) or `FilePasswordStore` (`credentials.json`). TUI probes the keyring at startup; CLI opens it lazily (`CliContext::password_store`, `OnceCell`). Secret precedence in `app::util::resolve_pending_secret`: keyed identity passphrase → `secret_lookup` (authoritative) → host password → identity secret. |
| **Profiles** | `state.toml` + `profiles/<name>/` under the data root; `.profile-id` in each dir; compat mode when `SSHUB_DATA_DIR` / `SSHUB_CONFIG_DIR` (or `SSH_LAUNCHER_*`) is set. Switch blockers in `App::profile_switch_blocker`. |
| **Themes** | 5 built-ins (`default`, `summer`, `aqua`, `fire`, `high-contrast`) from `assets/themes/`; 25 semantic slots, 234 component roles (233 with a semantic fallback, asserted in `src/theme/catalog.rs`); `MAX_USER_THEME_FILES = 256`, inheritance and reference depth 16. Themes dir = `<dir of config.toml>/themes`, for the TUI and for `sshub theme list|show` (`profile::locate_config_dir`). |
| **Config sections** | `appearance`, `session_logging`, `command_history`, `tunnel_reconnect`, `keybinds`, `clipboard`, `ping`, `updates`, `ssh` (`AppConfig` in `src/config.rs`). Retired: `appearance.opaque_background`. |
| **Global keys** | Defaults in `src/keybinds.rs`: help `?`, known hosts `H`, profiles `Alt+P`, log browser `Shift+L`, snippets `Shift+S`, in-session snippets `Ctrl+N`, update `Shift+U`, broadcast `b`, Settings `Ctrl+H` (hard-coded in `keys.rs`), keybind editor `Ctrl+K`. |
| **CLI subcommands** | `cli::is_subcommand`: `host`, `connect`, `exec`, `list`, `tunnel`, `group`, `groups`, `identity`, `tags`, `sync`, `import`, `export`, `completions`, `sftp`, `audit`; plus `db` and `theme` dispatched in `main.rs`. Transfers are `sshub <entity> transfer`. |
| **CI** | `ci.yml` jobs `branch` (PRs from this repo), `test` (ubuntu + macOS), `lint` (clippy `-D warnings`); `release.yml` jobs `build`, `release`, `npm`, `publish`; `openwiki-update.yml` and `strix-pentest.yml` are `workflow_dispatch` only. |

After substantive edits, update `openwiki/.last-update.json` (`gitHead`, `validatedAt`).
