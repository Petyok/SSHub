# Changelog

All notable changes to SSHub are documented in this file.

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
