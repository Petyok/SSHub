---
type: Architecture
title: Runtime Architecture, Event Loop, App State, and Source Map
description: How SSHub runs. main.rs dispatch, profile resolution, the synchronous 50 ms event loop in src/lib.rs driving the App state machine (40 AppMode variants plus a numeric active_tab), the ratatui render pipeline in src/tui, the mpsc background workers, and a module-by-module source map.
resource: src/lib.rs
tags: [architecture, event-loop, tui, ratatui, app-state, source-map]
---

# Runtime Architecture

SSHub has **no async runtime**. One binary (`src/main.rs`) either acts as an
askpass helper, runs a headless CLI command, or starts the TUI. The TUI is one
synchronous event loop in `src/lib.rs`; concurrency is plain threads talking
over `std::sync::mpsc` channels.

## Binary dispatch (`src/main.rs`)

Order matters and is fixed:

1. `session::askpass::maybe_run_askpass()` runs **first**: when ssh re-executes
   the binary as its `SSH_ASKPASS` helper it answers and exits before argv or the
   TUI are touched (see [secrets](../security/secrets.md#ssh_askpass)).
2. Global profile flags (`--profile NAME`, `--profile=NAME`,
   `--manage-profiles`) are extracted by `profile::extract_startup_flags`, but
   only from the arguments **before** a `--`, so a remote command's own flags
   (`sshub exec web -- aws --profile prod ...`) are not swallowed.
3. `db` (`db purge --yes-i-am-stupid`), then `theme` (`check|list|show`, headless
   by contract: no database is opened), then the subcommands accepted by
   `cli::is_subcommand` (`host`, `connect`, `exec`, `list`, `tunnel`, `group`,
   `groups`, `identity`, `tags`, `sync`, `import`, `export`, `completions`,
   `sftp`, `audit`). See [CLI](../workflows/cli.md).
4. `--help`, `--version`; an unknown positional exits 2; `--dry-run` exits 0.
5. `sshub::run_with(startup)` starts the TUI.

## Startup (`src/lib.rs::run_app_with`)

```text
parse global flags -> profile::resolve_startup -> (picker session: splash + picker)
  -> load_profile_app (config.toml, App::new_with_profile, config watcher)
  -> record last-used profile -> optional release check -> run_terminal_loop
```

- `profile::resolve_startup` returns `Startup::Silent(paths)` (one profile,
  `--profile`, compat mode, or no terminal) or `Startup::Picker`. The picker
  runs in its own small loop (`run_picker_loop`) on the embedded `default`
  theme, after the intro splash. Details: [profiles](profiles.md).
- `App::new_with_profile` opens `metadata.db` and `launcher.db` (running
  migrations), builds the `SshConfigResolver`, probes the keyring
  (`credentials::check_keyring_available`, a write/read/delete round trip) and
  wraps either `OsKeyring` or the `credentials.json` `FilePasswordStore` in a
  `NamespacedPasswordStore`, loads themes, starts the ping worker via
  `reload_hosts`, and spawns the OS-detect worker. First run with no hosts
  opens `Help`.
- Without a terminal on stdout the app renders one frame on a `TestBackend`
  (`run_headless_loop`) and needs `SSHUB_AUTO_QUIT` (`q` sends the quit key and
  confirms the dialog), otherwise it errors.
- Interactive runs with `[updates] check = true` and no `SSHUB_AUTO_QUIT` start
  the release check (`App::start_update_check`, worker in `src/update.rs`).

## The event loop (`run_terminal_loop`)

`POLL_INTERVAL = 50ms`; while a panel animation plays (`app.animating()`) the
poll window drops to 16 ms. Each iteration:

1. `terminal.size()` becomes `app.terminal_area`. Every session is `drain`ed
   (so background tabs keep up); each session on screen is `resize`d to the grid
   of its pane (`App::pane_views`, a no-op when unchanged), sessions out of view
   keep their last size and exited ones are never resized. The size comes from
   `terminal.size()`, not from crossterm resize events.
2. Finished key-push sessions are written to the audit log.
3. `app.poll_authentication()` collects secrets answered in auth modals.
4. `app.relay_visible_session_clipboard()` relays OSC 52 writes of the visible
   session only (in a split, the focused pane's), dropping the rest (before diagnostics, so a failed relay is
   logged in the same frame).
5. Session diagnostics feed the SSH log panel; a newly connected session clears
   its host's SSH log; a visible session that produced output is promoted from
   `Connecting` to `Session`.
6. `app.refresh_agent_info()`, then `terminal.draw(|f| tui::render(f, app))`.
7. `poll_keys_and_watcher(app)`: drains **all** queued crossterm events
   (`handle_key`, `handle_mouse`, `handle_paste`; draining one per frame made
   pastes crawl at about 20 chars/s), then non-blocking drains in this order:
   config watcher (any event triggers `reload_hosts`), ping results (30-sample
   ring per host, `PING_UNREACHABLE` marker), SFTP events (`sftp_rx` then the
   left pane's `sftp_rx2`), OS-detect results, `poll_update`, `tick_broadcast`,
   `tick_tunnels`, selection edge-autoscroll, `refresh_auth_cache` (10 s
   throttle) and finally `detect_tab_switch` (must run after the drains so a
   mode change from a worker event arms its slide in the same tick).
8. `apply_pending_profile(app, load_profile_app)`: an in-app profile switch
   builds the replacement `App` first and swaps only on success.
9. `should_quit` → `app.shutdown_all()` and exit.

## App state machine (`src/app/`)

`App` (`src/app/mod.rs`) holds all UI state plus injected dependencies
(`AppDeps`), which are the main test seams:

| Dep | Type | Production impl |
|---|---|---|
| `resolver` | `Box<dyn HostResolver>` | `SshConfigResolver` (`src/ssh/resolver.rs`) |
| `metadata` | `Arc<dyn MetadataStore>` | `MetadataDb` (`src/metadata/db.rs`) |
| `store` | `Arc<LauncherStore>` | `src/store/mod.rs` |
| `password_store` | `Box<dyn PasswordStore>` | `NamespacedPasswordStore` over `OsKeyring` or `FilePasswordStore` ([secrets](../security/secrets.md)) |

- **Modes**: `AppMode` (`src/app/types.rs`) has 40 variants: `Normal`,
  `Search`, `TagFilter`, `HostDetail`, the forms (`HostForm`, `IdentityForm`,
  `KeygenForm`, `GroupForm`, `TunnelForm`, `SnippetForm`), pickers
  (`GroupFieldPicker`, `TunnelHostPicker`, `SessionPicker`, `FieldPicker`,
  `PushKeyHostPicker`, `PushKeyIdentityPicker`, `SnippetPicker`), overlays
  (`GroupManage`, `KeybindEditor`, `Settings`, `ThemePicker`, `ProfilePicker`,
  `TunnelReconnectSettings`, `Help`, `Palette`, `ImportPrompt`, `SftpPrompt`,
  `KnownHosts`, `LogBrowser`, `SnippetManage`, `Notice`, `AuthPrompt`), the
  broadcast wizard (`BroadcastPickTarget`, `BroadcastCommand`,
  `BroadcastPreview`), confirmations (`ConfirmQuit`, `ConfirmDelete`,
  `ConfirmDiscard`) and the session pair `Connecting` / `Session`.
- **Key dispatch** (`src/app/keys.rs::handle_key`): `ForceQuit` first, then
  `Normal`-only globals (local shell, profile manager `Alt+P`, log browser
  `Shift+L`, snippet manager `Shift+S`, keybind editor, Settings `Ctrl+H`,
  session-strip binds), then a `match self.mode` to the per-mode handler in
  `src/app/*.rs`.
- **Tabs are not an enum**: `App.active_tab: usize`, 0 hosts
  (`handle_key_normal`), 1 SFTP (`handle_key_sftp`), 2 tunnels
  (`handle_key_tunnels`), 3 identities (`handle_key_keychain`), 4 audit
  (`handle_key_audit`). No type safety; grep `active_tab` when adding one.
- Keybindings are data: `KeyAction` plus `KeybindsConfig` in `src/keybinds.rs`;
  `App::is_action` matches a key against the configured binds.

## Render pipeline (`src/tui/`)

`tui::render` (`src/tui/mod.rs`):

1. `Connecting | Session` render the fullscreen session view
   ([sessions](../workflows/sessions-sftp.md)), painted by
   `src/session/render.rs` (including ghost-text suggestions).
2. Otherwise the bento-grid dashboard from `src/tui/dashboard_layout.rs`:
   header, tab bar, body, footer. The tab body is dispatched on `active_tab`;
   footer hints are built per tab in `src/tui/mod.rs`.
3. Overlays are dispatched on `app.mode`. `fit_popup` clamps popup rects so a
   terminal smaller than the popup cannot panic `u16::clamp`.
4. Motion: `src/tui/tween.rs` (eased slides), `src/tui/blit.rs` (cell-level
   slide/fade copies), `TAB_ANIM = 220 ms`; all motion is skipped with
   `[appearance] disable_animation`. Startup splash: `src/tui/animation.rs`
   (33 ms poll).

- **Screens** (`src/tui/screens/`): audit, auth, broadcast, field_picker,
  group_form, group_manage, help, host_form, keybind_editor, keychain, keygen,
  keys, known_hosts, log_browser, palette, push_key_pickers, session_picker,
  settings, sftp, snippet_form, snippet_manage, snippet_picker, tag_filter,
  theme_picker, tunnel_reconnect, tunnels.
- **Widgets** (`src/tui/widgets/`): footer, header, hosts_panel, middle_stack,
  panel_box, right_stack, tab_bar.
- **Colours** come only from the active `ResolvedTheme` (`app.theme()`);
  `src/tui/theme.rs` keeps only value-to-role mappings. See [themes](themes.md).

## Background workers and channels

| Worker | Spawned by | Channel on `App` | Drained in |
|---|---|---|---|
| Config watcher (`src/watcher.rs`) | `attach_config_watcher` | `watcher_rx` | `poll_keys_and_watcher` |
| Ping (`src/ping.rs`, `ping -c 1 -W 1` every 30 s) | `start_ping_worker` | `ping_rx` | same |
| SFTP (`src/sftp/worker.rs`) | SFTP tab | `sftp_rx`, `sftp_rx2` | same |
| OS detect (`src/osinfo/detect.rs`) | `new_with_profile` | `os_detect_rx` | same |
| Release check / install (`src/update.rs`) | `start_update_check`, `press_update_key` | `update.rx` | `poll_update` |
| Broadcast pool (`src/broadcast/mod.rs`) | broadcast wizard | `broadcast: Option<BroadcastState>`, field `rx` | `tick_broadcast` |
| Tunnels (`src/tunnel/`) | `TunnelManager` children | polled, no channel | `tick_tunnels` |
| PTY reader/writer (`src/session/pty.rs`) | each session | `PtyRuntime` | `Session::drain` |

- The ping worker only runs when `App::ping_live` is set (tests never probe),
  `[ping] enabled` is on (or a host overrides it to on) and there are targets.
  Replacing `ping_rx` drops the old receiver, so the previous worker exits.
- `tick_tunnels`: auto-connect bootstrap once, a re-read of the tunnel list from
  the store every 2 s (`sync_tunnels_from_store`), then
  `TunnelManager::check_health`, audit of reconnect events, and
  `tick_tunnel_reconnect` (backoff from `[tunnel_reconnect]`). See
  [tunnels](../workflows/tunnels.md).
- PTY writes go through a bounded `sync_channel` to a writer thread, so a remote
  that stops reading its input cannot block the frame loop.
- `src/ssh/probe.rs` now only defines `SshLogEntry` / `LogLevel`; the SSH log
  panel is fed by connect/session diagnostics, not by a background probe.

## File watcher

`src/watcher.rs`: a `notify` watcher monitors the ssh config's **parent directory**, not the file
(editor rename-saves swap inodes and detach file-level watches). Events are
debounced (`WATCHER_DEBOUNCE = 300 ms`) and trigger `app.reload_hosts()`.
`config.toml` changes are not watched.

## Source map

| Path | What lives there |
|---|---|
| `src/main.rs` | Dispatch order above, `db purge`, global `--help` |
| `src/lib.rs` | `run_app_with`, picker loop, event loop, headless loop, `purge_profile_database` |
| `src/app/` | `App` and one file per concern: `keys.rs` dispatch, `connect.rs` / `session_spawn.rs` / `session.rs` sessions, `auth.rs` auth modal, `adhoc.rs` connect-without-saving, `local_shell.rs`, `session_picker.rs`, `sftp.rs`, `tunnels.rs`, `audit.rs`, `broadcast.rs`, `snippets.rs`, `suggestions.rs` + `ghost.rs` (ghost text), `log_browser.rs`, `keygen.rs`, `push_key.rs`, `identities.rs`, `groups.rs`, `host_crud.rs`, `host_form.rs`, `host_detail.rs`, `hostlist.rs`, `tags.rs`, `import.rs`, `mouse.rs`, `theme_picker.rs`, `profile_manager.rs`, `profile_transfer.rs`, `update.rs`, `util.rs`, `types.rs`, `tests/` |
| `src/tui/` | Render pipeline, screens, widgets, layout, motion helpers |
| `src/theme/` | Runtime theme system ([themes](themes.md)) |
| `src/profile/` | Profile layout, picker, legacy migration ([profiles](profiles.md)) |
| `src/store/` | `LauncherStore` and `launcher.db` schema, group defaults, snippets, log bookmarks, command history, cross-profile transfer engine ([data model](data-model.md)) |
| `src/metadata/` | `metadata.db` for legacy ssh_config aliases |
| `src/hosts/` | `load_merged_hosts`, host CRUD helpers |
| `src/ssh/` | Resolver, argv builders (`host.rs`, `safe_ssh_target`), agent, key files, certificates (`cert.rs`), ssh_config import/export |
| `src/session/` | Embedded PTY sessions: `pty.rs`, `parser.rs` (vt100 plus query answers and OSC 52 capture), `render.rs`, `askpass.rs` (staged file), `askpass_channel.rs` (private socket), `interactive_auth.rs`, `auth_prompt.rs`, `history.rs`, `keys.rs` |
| `src/session_log.rs`, `src/log_browser.rs` | Session log writer and read-only browser backend |
| `src/suggestions.rs`, `src/command_safety.rs` | Local suggestion ranking and the shared command indexing classifier |
| `src/broadcast/` | Pure broadcast engine and bounded worker pool |
| `src/sftp/` | libssh2 transport, worker thread, pure model |
| `src/tunnel/` | `TunnelManager`, spawn, PID files, reconnect audit |
| `src/known_hosts.rs` | Known hosts manager backend |
| `src/credentials.rs` | `PasswordStore` implementations and Secret Service lookup |
| `src/update.rs` | Release check and in-place update |
| `src/import/` | Termius CSV, PuTTY, mRemoteNG importers |
| `src/osinfo/` | OS detection and logos |
| `src/cli/` | Headless CLI (including `exec.rs`, `theme.rs`, `transfer.rs`) |
| `src/config.rs`, `src/keybinds.rs` | `config.toml` model and keybindings |
| `src/osc52.rs`, `src/secure_fs.rs`, `src/text_input.rs`, `src/search.rs`, `src/ping.rs`, `src/session_transport.rs`, `src/watcher.rs` | Small shared helpers |

## Where to go next

- [Data model & storage](data-model.md): what the event loop loads and persists.
- [Profiles](profiles.md) and [themes](themes.md): the two startup-time subsystems.
- [TUI dashboard workflow](../workflows/tui.md): user-facing tabs, keys and screens.
- [Testing strategy](../testing/strategy.md): how `AppDeps` doubles and `TestBackend` make this loop testable.
