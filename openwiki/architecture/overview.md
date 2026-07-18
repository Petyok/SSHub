---
type: Architecture
title: Runtime Architecture — Event Loop, App State, and TUI
description: How SSHub runs — a synchronous 50ms event loop in src/lib.rs driving the App state machine (AppMode overlays + active_tab), the ratatui render pipeline in src/tui, and mpsc-based background workers for watcher, ping, SFTP, OS detection, and tunnels.
resource: src/lib.rs
tags: [architecture, event-loop, tui, ratatui, app-state]
---

# Runtime Architecture

SSHub has **no async runtime**. Everything is driven by one synchronous event loop in `src/lib.rs` (`run()` → `run_app()`), with concurrency handled by background threads communicating over `std::sync::mpsc` channels.

## The event loop (`src/lib.rs`)

`POLL_INTERVAL = 50ms`. Each frame:

1. **Drain sessions** — every open embedded session's PTY is drained and resized; a `Connecting` session is promoted to `Session` on first output.
2. **Draw** — `terminal.draw` renders via `tui::render`.
3. **Drain input** — `poll_keys_and_watcher` drains *all* queued crossterm events per frame (the code notes that draining only one would make "paste into an embedded session crawl at ~20 chars/sec"), then non-blocking `try_recv` drains of every worker channel:
   - config [file watcher](#file-watcher) → `app.reload_hosts()`
   - ping worker (30 s interval, ring buffer of 30 samples per host)
   - SFTP worker events → `apply_sftp_event` (see [sessions & SFTP](../workflows/sessions-sftp.md))
   - SSH probe logs, OS-detect worker results
   - `tick_tunnels()` keep-alive (see [tunnels](../workflows/tunnels.md))
   - auth-events cache refresh (10 s)

A headless variant (`run_headless_loop`) renders once on a ratatui `TestBackend` and quits — used by `SSHUB_AUTO_QUIT`/`--dry-run` and by smoke tests (see [testing strategy](../testing/strategy.md)).

## App state machine (`src/app/`)

`App` (`src/app/mod.rs`) holds all UI state plus injected dependencies (`AppDeps`), which are the main test seams:

| Dep | Trait | Production impl |
|---|---|---|
| `resolver` | `HostResolver` | `SshConfigResolver` (`src/ssh/resolver.rs`) |
| `metadata` | `MetadataStore` | `MetadataDb` (`src/metadata/db.rs`) |
| `store` | — | `LauncherStore` (`src/store/mod.rs`) |
| `launcher` | `TerminalLauncher` | kitty/ghostty/custom ([integrations](../integrations/external-terminals.md)) |
| `password_store` | `PasswordStore` | `OsKeyring` ([secrets](../security/secrets.md)) |

- **Modes**: `AppMode` (`src/app/types.rs`) has ~26 variants — `Normal`, `Search`, `TagFilter`, `HostDetail`, `HostForm`, `IdentityForm`, `GroupForm`/`GroupManage`, `TunnelForm`, `Palette`, `Settings`, `KeybindEditor`, `Help`, `ConfirmQuit`/`ConfirmDelete`/`ConfirmDiscard`, `Connecting`, `Session`, etc. Overlays are modes; key dispatch lives in `src/app/keys.rs` and per-mode handlers in `src/app/*.rs`.
- **Tabs are not an enum**: `App.active_tab: usize` (0–4 = hosts, sftp, tunnels, identities, audit). Be careful when adding tabs — there is no type safety here.
- First run with no hosts drops straight into `Help` mode.
- The `TerminalLauncher` dependency is retained but dead at runtime: sessions run in the embedded PTY (src/session/), and the CLI `sshub host connect` path spawns ssh/mosh directly via std::process::Command (src/cli/host.rs cmd_connect), bypassing TerminalLauncher entirely. The trait is exercised only by its own module unit tests and test doubles; App.launcher itself is never called from any production code path.

## Render pipeline (`src/tui/`)

`tui::render` (`src/tui/mod.rs`):

1. Optional opaque-background backfill (Settings toggle for transparent terminals).
2. `Connecting | Session` modes render the fullscreen session view ([sessions](../workflows/sessions-sftp.md)).
3. Otherwise the bento-grid dashboard chrome from `src/tui/dashboard_layout.rs`: header, tab bar, three-column body, footer. Tab body is dispatched by `active_tab`. (`src/tui/layout.rs`'s older `root_layout` appears superseded — likely dead code.)
4. Overlay popups are dispatched on `app.mode`. `fit_popup` clamps popup rects because `u16::clamp` would otherwise panic and "crash the whole TUI on a terminal smaller than the popup".

- **Screens** (`src/tui/screens/`): hosts, sftp, tunnels, keys (identities), audit, help, palette, settings, keybind_editor, host_form, group_form, group_manage, field_picker, session_host_picker, tag_filter, tunnel_reconnect, keychain.
- **Widgets** (`src/tui/widgets/`): header, footer, tab_bar, status_bar, host_list, hosts_panel, detail_panel, middle_stack (host card / agent / latency + SSH log panel), right_stack (recent hosts, auth sparkline, ping), panel_box.
- **Theme** (`src/tui/theme.rs`): fixed hex palette (BG `#0b0d10`, green accent `#9ec99b`) with semantic style helpers and a sparkline ramp. Startup animation: `src/tui/animation.rs` (33 ms loop, toggleable in Settings).

## File watcher (`src/watcher.rs`)

A `notify` watcher monitors the ssh config's **parent directory**, not the file — editor rename-saves swap inodes and silently detach file-level watches. Events are debounced (300 ms) on a thread and delivered as `WatchEvent::ConfigChanged`, which triggers `app.reload_hosts()` in the event loop. This is the "hot reload" feature.

## Where to go next

- [Data model & storage](data-model.md) — what the event loop loads and persists.
- [TUI dashboard workflow](../workflows/tui.md) — user-facing tabs, keys, and screens built on this state machine.
- [Testing strategy](../testing/strategy.md) — how `AppDeps` doubles and `TestBackend` make this loop testable.
