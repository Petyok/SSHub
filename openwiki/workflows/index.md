---
type: Documentation Index
title: "Workflows"
description: "Files and subdirectories in Workflows."
---

# Files

- [Headless CLI — full command tree, JSON output, and exit codes](cli.md) - SSHub's scriptable command-line interface (src/cli) covering hosts, groups, identities, tunnels, SFTP, audit, import/export/sync, and completions, with --format json support and stable exit codes 0/1/2.
- [Embedded Sessions & SFTP — PTY sessions, session logging, mosh, and file transfer](sessions-sftp.md) - How SSHub runs SSH inside the TUI via portable-pty + vt100 (src/session), stages credentials through SSH_ASKPASS re-exec, optionally logs session output with rotation, supports mosh transport, and provides a dual-pane SFTP browser over libssh2 with a staged transfer queue (src/sftp).
- [TUI Dashboard — Tabs, Overlays, and Keybindings](tui.md) - The keyboard-driven SSHub dashboard — five tabs (hosts, SFTP, tunnels, identities, audit), overlay screens (palette, tag filter, settings, keybind editor, help), default keybindings, and how screens map to AppMode in src/app and src/tui/screens.
- [SSH Tunnels — local/remote/dynamic forwards with keep-alive reconnect](tunnels.md) - SSHub defines and manages SSH tunnels (local -L, remote -R, dynamic SOCKS -D), spawns them as monitored child processes with BatchMode/askpass secret staging, and auto-reconnects dropped keep-alive tunnels with exponential backoff configured in config.toml.
