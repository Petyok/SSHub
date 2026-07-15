# Architecture overview

SSHub is a single-process TUI application with a synchronous event loop and several background worker threads. The design prioritizes determinism and simple threading over an async runtime.

## Stack & dependencies

- **TUI**: `ratatui` 0.30 + `crossterm` 0.29. Renders into the alternate screen, supports mouse, bracketed paste.
- **PTY sessions**: `portable-pty` spawns `ssh` on a pseudo-TTY; `vt100` emulates the terminal grid; `tui-term` renders it inside ratatui.
- **Persistence**: `rusqlite` with bundled SQLite in `~/.local/share/sshub/launcher.db`.
- **Crypto / OS integration**: `keyring` with platform backends (Apple/Windows/Secret Service) for passwords and key passphrases.
- **SFTP**: `ssh2` with vendored OpenSSL for self-contained builds.
- **Search**: `nucleo` for fuzzy host / command palette.
- **Config**: `serde` + `toml`/`toml_edit` to load and preserve comments on save.
- **File watcher**: `notify` 7 watches the `~/.ssh/config` parent directory.

No async runtime. Cross-thread communication uses `std::sync::mpsc` channels.

## Lifecycle

```
src/main.rs
    maybe_run_askpass()          # SSH_ASKPASS helper short-circuit
    handle --help / --version / db purge
    sshub::run()

src/lib.rs
    run_app()
      config::load_config()
      App::new(config)
      attach_config_watcher()    # hot reload of ~/.ssh/config
      run_terminal_loop() / run_headless_loop()
```

`App::new()` (in `src/app/mod.rs`) wires together:

- a host resolver (`ssh::SshConfigResolver` or a test resolver),
- the SQLite `LauncherStore`,
- the legacy `MetadataDb`,
- a `TerminalLauncher` chosen from `config.terminal` (Kitty / Ghostty / Custom),
- a `PasswordStore` implementation via `credentials.rs`.

This is captured in `AppDeps` and enables dependency injection for tests.

## Event loop

The terminal loop (`src/lib.rs`) polls `crossterm::event::poll(POLL_INTERVAL)` every 50 ms.

For each tick:

1. Drain crossterm events (key, mouse, resize) and call `app.handle_event(...)`.
2. Drain background channels:
   - `watcher_rx` for `~/.ssh/config` changes,
   - `ping_rx` for latency results,
   - `probe_rx` for OS detection / SSH handshake log lines,
   - `os_detect_rx` for OS logo probes,
   - `sftp_rx` for SFTP worker events,
   - `app.tick_tunnels()` for tunnel health checks and keep-alive reconnect timers (`check_health` + `tick_reconnect`).
3. Drain embedded sessions (`Session::drain()` reads PTY bytes each frame; user input is written from key handlers).
4. If the terminal was resized, propagate it to sessions.
5. Render one frame.

The headless loop (`run_headless_loop`) does the same without alternate-screen drawing for CI smoke tests.

## Application state

`App` in `src/app/mod.rs` is the central state bag. It holds:

- `hosts`, `filtered_indices`, `selected`: the host list and current selection.
- `group_sections` / `nav_rows` / `collapsed_groups`: nested group tree structure.
- `active_tab`: 0 hosts, 1 sftp, 2 tunnels, 3 identities, 4 audit.
- `mode`: `AppMode` determines rendering and key dispatch (Normal, Search, HostForm, Session, ...).
- `sessions`: open embedded PTY sessions with a tab strip shown in the header.
- `sftp`, `tunnel_manager`, `ping_*`, `os_detect_*`: domain-specific runtime state.
- `config`: the loaded `AppConfig`.
- `auth_events_cache`, etc.: denormalized UI caches refreshed on demand.

`src/app/types.rs` defines the core mode and data enums: `AppMode`, `SortMode`, `HostEntry`, `HostGroupSection`, `NavRow`, `VisualRow`, `AuditFilter`, `AuditRange`, `SETTINGS_ITEMS`, `TUNNEL_RECONNECT_FIELDS`.

## Rendering

`src/tui/mod.rs::render()` is the single frame entry point.

- Embedded sessions (`Connecting` / `Session` modes) take over the whole frame via `src/session/render.rs`.
- Otherwise the dashboard chrome is drawn: header, session strip, tab bar, body, footer.
- The active tab dispatches body rendering to `render_hosts_body`, `render_sftp_body`, `render_tunnels_body`, `render_keys_body`, `render_audit_body`.
- Overlay popups are rendered on top based on `AppMode`.

`src/tui/theme.rs`, `src/tui/dashboard_layout.rs`, and `src/tui/widgets/*` provide reusable layout + widgets. `src/tui/animation.rs` handles the startup splash.

## Input dispatch

`src/app/mod.rs` implements `handle_event`. It routes:

- Global actions (quit, help, keybind editor, settings, tab switching, zoom) first.
- Mode-specific handling next (`handle_key_search`, `handle_key_host_form`, `handle_key_session`, ...).
- Tab-specific handling last (`handle_key_hosts`, `handle_key_sftp`, `handle_key_tunnels`, `handle_key_keys`, `handle_key_audit`).

`src/app/mouse.rs` maps mouse clicks / scrolls to the same actions.

Keybindings are user-remappable: actions are defined in `src/keybinds.rs`, loaded/saved to `config.toml`, and parsed in `src/app/util.rs` (`parse_keyspec`).

## Background workers

| Worker            | File(s)                              | Channel into app        | Purpose |
|-------------------|--------------------------------------|-------------------------|---------|
| SSH config watcher| `src/watcher.rs`                     | `watcher_rx`            | Detect renames/saves of `~/.ssh/config` and reload hosts. |
| Ping              | `src/ping.rs`                        | `ping_rx`               | ICMP echo selected/visible hosts for latency sparkline. |
| OS detection      | `src/osinfo/detect.rs`               | `os_detect_rx`          | SSH into a host once and parse `/etc/os-release` for the logo. |
| SFTP              | `src/sftp/worker.rs`, `transport.rs` | `sftp_rx`               | Run libssh2 commands off the UI thread. |
| Tunnel stderr     | `src/tunnel.rs`                      | internal `Arc<Mutex>`   | Drain `ssh -N` stderr for error diagnostics. |

## Module responsibilities

| Module            | What it owns |
|-------------------|--------------|
| `src/app/*`       | State, validation, workflows, input handling, tests. |
| `src/tui/*`       | Pure rendering and layout, no state mutation beyond local UI caches. |
| `src/session/*`   | Embedded PTY lifecycle, VT100 parsing, askpass secret injection, render. |
| `src/session_log.rs` | Rotating PTY transcript files under the data dir. |
| `src/tunnel.rs`   | Spawn/monitor `ssh -N` child processes and reconnect backoff. |
| `src/sftp/*`      | SFTP model, synchronous libssh2 transport on a worker thread. |
| `src/ssh/*`       | config parsing/import/export, resolver, agent info, host-key probe, argv builders. |
| `src/store/*`     | SQLite `LauncherStore`, schema migrations, CRUD. |
| `src/metadata/*`  | Legacy `MetadataDb`; still used for host metadata overlays. |
| `src/launcher/*`  | External-terminal launchers (still present; embedded PTY is the default path). |
| `src/import/*`    | Termius backup importer. |
| `src/watcher.rs`  | `notify`-based config hot reload. |
| `src/keybinds.rs` | Remappable keybinding definitions and defaults. |
| `src/osinfo/*`    | OS detection + Braille/ANSI logos. |
| `src/text_input.rs`| Cursor-aware text input widget used in forms and prompts. |
| `src/search.rs`   | `nucleo` fuzzy search wrapper. |
| `src/ping.rs`     | Host reachability / latency worker. |

## Threading invariants

- The main thread owns `App` and does all rendering and most state mutation.
- Worker threads own their I/O (`watcher`, `ping`, `os_detect`, `sftp`, tunnel stderr drain, PTY child IO threads).
- Data flows in via `std::sync::mpsc::Receiver` or shared `Arc<Mutex<...>>` snapshots.
- The event loop polls/drains each channel every tick, so workers never block UI input.

## Extension points

- Add a tab: bump constants in `app/types.rs`, add rendering in `tui/mod.rs`, add key handling in `app/keys.rs` and `app/mod.rs`, add footer hints in `tui/widgets/footer.rs`.
- Add a config section: extend `AppConfig` in `src/config.rs`, add defaults, save with `toml_edit`.
- Add a keybind action: add the action to `keybinds.rs` defaults + `KeyAction` enum, wire it in `app/keys.rs`, and render a hint in the footer/help.
- Add a migration: append to `src/store/migrate.rs` and bump `SCHEMA_VERSION`.
