---
type: Product Overview
title: SSHub — Quickstart
description: Entry point to the SSHub knowledge base. SSHub is a Rust terminal UI for managing and connecting to SSH hosts, combining ~/.ssh/config with a built-in SQLite host database, embedded PTY sessions, SFTP, tunnels, identities, and an audit log.
tags: [sshub, tui, ssh, rust, overview]
---

# SSHub Quickstart

SSHub (`sshub`, v0.9.3 in `Cargo.toml`) is a keyboard-driven terminal UI for managing and connecting to SSH hosts. It merges your read-only `~/.ssh/config` with a fully managed host database (SQLite), and adds embedded in-TUI SSH sessions, an SFTP file browser, SSH tunnels with keep-alive reconnect, ssh-agent identity management, OS auto-detection with logos, and a connection audit log. It also ships a full headless CLI for scripting. License: AGPL-3.0-or-later.

- Crate: `sshub` on crates.io (`cargo install sshub`); repo: github.com/Petyok/SSHub
- Stack: Rust 2021, ratatui 0.30 + crossterm (TUI), portable-pty + vt100/tui-term (embedded sessions), rusqlite bundled (SQLite), ssh2/libssh2 with vendored OpenSSL (SFTP), nucleo (fuzzy search), notify (file watcher), keyring (OS secret store). **No async runtime** — a synchronous event loop polls every 50 ms (`src/lib.rs`).
- Single binary: `src/main.rs` dispatches askpass re-exec → `db` subcommand → headless CLI subcommands → global flags → the TUI.

## Install and run

```bash
cargo install sshub          # or: git clone … && just install
sshub                        # launch TUI
sshub --help                 # global options (--dry-run, --version)
sshub list                   # headless CLI (see workflows/cli.md)
```

Linux builds need `libdbus-1-dev` + `pkg-config` (Secret Service keyring backend). At runtime an unlocked Secret Service provider (gnome-keyring, KWallet) is required for password persistence; otherwise SSHub warns and ssh falls back to prompting.

## Data paths

| Resource | Default path | Override |
|---|---|---|
| Config | `~/.config/sshub/config.toml` | `SSHUB_CONFIG_DIR` |
| Databases | `~/.local/share/sshub/launcher.db` (+ `metadata.db`) | `SSHUB_DATA_DIR` |
| SSH config | `~/.ssh/config` | `SSHUB_SSH_CONFIG` |
| Session logs | `~/.local/share/sshub/logs/<host-dir>/` | — |

Legacy `SSH_LAUNCHER_*` env vars are still honored as fallbacks, and `~/.config/ssh-launcher` is auto-migrated to `~/.config/sshub` (`src/config.rs`).

## Where to go next

### Architecture
- [Runtime architecture](architecture/overview.md) — the 50 ms synchronous event loop, the `App` state machine (`AppMode` overlays, `active_tab`), the TUI render pipeline, and background workers.
- [Data model & storage](architecture/data-model.md) — `launcher.db` vs `metadata.db`, schema migrations, the hybrid ssh_config/managed host model, config file, and file watching.

### Workflows
- [TUI dashboard](workflows/tui.md) — tabs, overlays, keybindings, and screens.
- [Sessions & SFTP](workflows/sessions-sftp.md) — embedded PTY sessions, askpass, session logging, mosh, and the dual-pane SFTP browser.
- [Tunnels](workflows/tunnels.md) — local/remote/dynamic tunnels and keep-alive reconnect with backoff.
- [Headless CLI](workflows/cli.md) — full command tree, JSON output, exit codes.

### Domain
- [Hosts, groups & identities](domain/hosts-identities.md) — host sources, nested groups and Favorites, identities, ssh-agent, and Termius import.

### Operations & testing
- [Build, versioning & release](operations/build-release.md) — Justfile recipes, odometer versioning, pre-commit hook, release flow.
- [CI & automation](operations/ci-cd.md) — GitHub Actions workflows, including the OpenWiki wiki-update bot.
- [Testing strategy](testing/strategy.md) — unit / smoke / e2e / config levels, fixtures, and test doubles.

### Integrations & security
- [External terminal launchers & demo](integrations/external-terminals.md) — kitty/ghostty/custom launchers and the VHS demo pipeline.
- [Secrets, credentials & file security](security/secrets.md) — OS keyring, askpass staging, TOFU host keys, session-log exposure warning, permission hardening.

## Contributing pointers

Pinned workflow: [docs/implementation-flow.md](../docs/implementation-flow.md) (issue → claim → branch off `development` → verify → adversarial review → PR). Run `cargo fmt`, `cargo fmt --check`, and `cargo clippy --all-targets` before every push — CI enforces the same. See [Build, versioning & release](operations/build-release.md) for the branch model (`feature/* → development → main`).

## Backlog

- **Demo pipeline details** (`demo/` tapes, `record.sh`, `seed-demo.sh`) — only summarized under [integrations](integrations/external-terminals.md); deferred because it is contributor tooling, not product behavior.
- **Host-sync design** (`docs/host-sync-design.md`) — P2P sync design for epic #13; not yet implemented, documented only in the design doc.
- **Detached tunnel PID-file hardening** (`src/tunnel/spawn.rs`) — acknowledged races (no locking, recycled PIDs) noted in [tunnels](workflows/tunnels.md); behavior may change.
