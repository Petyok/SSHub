---
type: Documentation Index
title: "Architecture"
description: "Files and subdirectories in Architecture."
---

# Files

- [Data Model & Storage — launcher.db, metadata.db, config, and hybrid hosts](data-model.md) - SSHub persists state in two SQLite databases (launcher.db for managed hosts/groups/identities/tunnels/audit, metadata.db for legacy ssh_config hosts), a TOML config file, and merges managed hosts with read-only ~/.ssh/config aliases through the HostResolver abstraction.
- [Runtime Architecture — Event Loop, App State, and TUI](overview.md) - How SSHub runs — a synchronous 50ms event loop in src/lib.rs driving the App state machine (AppMode overlays + active_tab), the ratatui render pipeline in src/tui, and mpsc-based background workers for watcher, ping, SFTP, OS detection, and tunnels.
