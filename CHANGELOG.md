# Changelog

All notable changes to SSHub are documented in this file.

## [Unreleased]

## [0.9.0] - 2026-07-16

### Added

- **Mosh transport** — per-host `Transport` field (`ssh` or `mosh`) in the host form
  and detail panel. Embedded sessions use `mosh` when selected; tunnels and SFTP stay
  ssh-only. Graceful error when `mosh` is not installed.
  (Schema v13.)
- **Session logging** — opt-in capture of embedded SSH session PTY output to plain-text
  files under `~/.local/share/sshub/logs/<host-dir>/`, with rotated segment files inside
  (managed hosts use `{sanitized-name}-{id}`). Toggle globally in Settings (`Ctrl+H`) or
  per host in the host form (`inherit` / `on` / `off`). Size-based rotation and per-host
  retention cap are configurable in `config.toml`. The audit tab shows the log directory
  path for each connect event. Pure `~/.ssh/config` aliases without a launcher
  row may share a log directory when sanitized host names collide. **Warning:** logs
  capture everything echoed to the terminal, including passwords if they appear on
  screen. (Schema v12.)
- **Tunnel keep alive** — per-tunnel **Keep alive** toggle in the tunnel form
  (uses existing `auto_connect` column). Enabled tunnels start on app launch and
  automatically reconnect after unexpected exit with exponential backoff, jitter,
  and a capped retry count (`[tunnel_reconnect]` in `config.toml`, editable on the
  Tunnels tab with `R`: `max_attempts`, delays in seconds, stable time, jitter).
  Manual stop or kill disables the retry loop until the tunnel is started again
  (until the next app launch for keep-alive auto-start). The Tunnels tab shows
  `starting` / `reconnecting` / `gave up` with attempt counter; audit logs
  reconnect attempts. A tunnel must stay up for `stable_secs` (default 5s) before
  it counts as reconnected. Background ssh uses `ServerAliveInterval`,
  `ServerAliveCountMax`, and `TCPKeepAlive` so dead paths (e.g. VPN dropped) tear
  down instead of leaving a stale local listener.

### Fixed

- **Session keybind hints** — the connected-session header and dashboard footer
  now show your configured keybindings (from `config.toml` / the keybind editor)
  instead of hardcoded `Ctrl+T` / `Ctrl+D` defaults. Connecting-screen hints
  (`expand log`, `cancel`) follow the same config.
- **Tunnel auth on TUI** — background `ssh -N` tunnels no longer open `/dev/tty`
  for password prompts (which painted over the dashboard and stole mouse/keyboard
  input). Tunnels use `BatchMode=yes` when no stored credential is available
  (fail fast with an error in the Tunnels tab) and `SSH_ASKPASS` when a host or
  identity password is in the keyring.

## [0.8.0] - 2026-07-12

### Added

- **SFTP file operations** — manage files directly in the browser: delete a
  file or folder (`d`, recursive for non-empty directories), create a new folder
  (`n`), rename/move (`R`), and change permissions (`M`, octal chmod). Remote ops
  run on the background worker; local ops use the filesystem directly.
- **Recursive directory transfers** — staging a directory now uploads/downloads
  the whole tree, with a progress bar over the total bytes.
- **Cursor navigation in text fields** — `←`/`→` move the edit cursor, `Home`/`End`
  jump to the edges, and `Delete` removes the character ahead of the cursor, across
  every form and prompt (host, identity, group, tunnel, SFTP mkdir/rename, import).
  Previously the cursor was pinned to the end and only backspace worked.
- **`SSHUB_VERSION_LABEL`** — override the version shown in the tab bar: set it
  empty to hide the version, or to a custom string. (Unset keeps the build version.)

### Fixed

- **SFTP transfers follow symlinks properly** — a symlink to a file transfers
  with the target's size (progress no longer overshoots), and symlinks to
  directories or broken links are skipped instead of failing the whole queue.
- **Help overlay** — scrolling is clamped to the rendered content, so it no
  longer overshoots into blank space.
- **Settings overlay** — the footer hint no longer spills onto the popup border.

## [0.7.0] - 2026-07-10

### Added

- **SFTP file transfer** — a new tab with a dual-pane browser (local left / remote right): browse both sides, filter with `/`, stage uploads and downloads into a queue, and run them sequentially with a progress bar. Native libssh2 transport on a background thread, trust-on-first-use host-key verification, atomic temp-file + rename writes. Open it for the current SSH host with `Ctrl+Shift+F`; jump back to that host's session with `s`.
- **OS auto-detection** — a background probe on first connect detects the remote distro; the host card renders its logo as crisp Braille art in brand colors (font-logos + chafa), like Termius.
- **Multiple groups per host** — hosts can belong to several groups at once, picked via a multi-select checkbox in the host form. A reserved **Favorites** group with a ★ marker in the list, toggled with `f`. (Schema v11: `host_group_memberships` join table.)
- **Settings overlay** (`Ctrl+H`) — toggle an opaque background (for transparent terminals), OS logos, quit confirmation, and the startup animation.
- **Richer host card** — fact sheet with user@host:port, group, key/identity, tags, and last-connected, next to the OS logo.
- **Accept a changed host key** — when a server's fingerprint changes, a prompt offers to purge the stale `known_hosts` entry and reconnect.
- **Version shown in the tab bar.**

### Changed

- **Tab order** — SFTP is inserted as the 2nd tab: `1` hosts, `2` sftp, `3` tunnels, `4` identities, `5` audit. Existing custom tab keybinds are migrated on config load.
- **Latency panel** now reflects the selected host, not an all-hosts aggregate.
- **ssh log lines wrap** instead of truncating with an ellipsis.

### Fixed

- SFTP picker: connect to the host you filtered to (a stale index could connect to the wrong host once the search cleared); queue re-entry guard; navigation frozen during a running queue; stale remote listings dropped; search input captured before tab-switch so typed letters don't fire tab binds.
- Keybinds migration persists so it runs exactly once.

## [0.5.7] - 2026-07-09

### Fixed

- Text selection keeps its full range when an autoscrolling drag carries it off-screen

## [0.5.6] - 2026-07-08

### Added

- **Autoscroll while selecting** — extending a selection past the viewport edge scrolls the session

### Fixed

- Long ssh log lines are clamped with an ellipsis so they stay inside the box
- Clipboard pastes are forwarded into sessions as bracketed paste
- The connect command stays visible in the ssh log after connecting
- `j`/`k` can be typed in search and the palette; dropped bare-key type-ahead

## [0.5.0] - 2026-07-08

### Added

- **Mouse text selection** — select text in an embedded session with the mouse and copy on release

## [0.4.0] - 2026-07-08

### Changed

- **Relicensed from MIT to AGPL-3.0-or-later** starting this release — a strong copyleft so forks and derivatives must stay open under the same terms. Versions ≤ 0.3.1 remain available under MIT.

## [0.3.1] - 2026-07-08

### Changed

- README images use absolute URLs so they render on the crates.io page; added a crates.io version badge and documented `cargo install sshub` plus the prebuilt release binaries. (Docs only — no code changes.)

## [0.3.0] - 2026-07-08

### Changed

- **ratatui 0.30** — upgraded the TUI stack; dropped the vendored vt100 fork in favour of upstream vt100 0.16 (which now carries the scrollback fix)
- **Scrollable help** — the `?` overlay scrolls (↑↓/j k/PgUp/PgDn/Home/End) with a pinned footer instead of truncating
- **Visible popup frames** — modal overlays (help, keybindings, palette, tag filter, group/field pickers, import prompt) now draw a distinct border so they read as dialogs
- **ssh_config Include** — importing hosts now follows `Include` directives (with tilde/relative/glob resolution)
- **Live hot reload** — the config watcher watches the containing directory, so editor rename-saves (vim `:w`, VSCode) keep triggering reloads
- **Config saves preserve** hand-written comments and any keys SSHub doesn't model
- **Installable crate** — publishable to crates.io (`cargo install sshub`); the demo fixture seeder moved from a binary to an example so it isn't installed

### Fixed

Security:

- Custom launcher now rejects `<`/`>` redirection in host fields (shell injection)
- ssh_config export flattens newlines in fields (config-directive injection)
- Private-key passphrase is delivered to `ssh-keygen` via `SSH_ASKPASS`, not as an argv argument visible in `ps`

Stability & correctness:

- Popups no longer panic on small terminals (a `clamp(min > max)` crash)
- Dashboard, host-form, and confirm text no longer overflow their borders on narrow terminals or when zoomed
- Editing a running tunnel now stops the old `ssh -N` child (was orphaned, holding its port); tunnel stderr is drained to avoid a pipe-buffer stall
- Mouse-wheel scrolling reaches hosts past group headers
- Confirm/keybind matching: `Shift+Y`/`Shift+N` bindings work, and editor-captured single-letter bindings fire correctly
- New hosts get a distinct `sort_order`, so Manual-mode reordering works
- A corrupt or locked legacy `metadata.db` no longer bricks startup
- The Audit tab's filter/range survives the periodic 10s refresh; the ping worker is no longer orphaned when the host list reloads to empty
- Confirm-delete wraps long host names; keybind labels no longer overflow the value column

## [0.2.0] - 2026-07-07

### Added

- **Embedded SSH sessions** — connect runs in an in-TUI PTY; detach with Ctrl+D and return to the dashboard while SSH keeps running
- **Session tab strip** — open sessions shown in the header; Ctrl+T opens host picker, Ctrl+[/] switches tabs, Ctrl+Shift+S focuses a session from the dashboard
- **Configurable keybindings** — 61 actions in `config.toml`; command palette and keybind editor
- **Nested host groups** — groups can contain sub-groups; redesigned group management overlays
- **Connect spinner** — visual feedback while SSH connects; `ssh -v` debug routed off the PTY
- **Failure screen** — plain-language disconnect reason with dismiss
- **Multi-tag filter** — AND filter via `#` tag picker
- **Group jump** — quick navigation between groups
- **Reachable ping stats** — latency sparkline in host detail
- **Tag-filter picker** — visible UI for tag selection; empty groups hidden while filtering

### Changed

- Import/export tests use explicit export paths to avoid parallel env races
- Process cleanup on quit — detached SSH children killed via `App::shutdown_all()` and `Drop`

### Fixed

- Connect spinner stays visible for unreachable hosts
- Enter confirms multi-tag selection
- Duplicate `KeyCode::Tab` arm in key event mapping
- Group tree order in manager; edit group on `E`; themed dropdown pickers

## [0.1.0] - 2026-06-01

Initial release — TUI launcher for SSH hosts with hybrid `~/.ssh/config` + launcher DB, tunnels, keys, audit log, fuzzy search, and file watcher hot reload.
