# Changelog

All notable changes to SSHub are documented in this file.

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
