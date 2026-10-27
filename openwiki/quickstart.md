---
type: Product Overview
title: SSHub Quickstart
description: Entry point to the SSHub knowledge base. SSHub is a Rust terminal UI for managing and connecting to SSH hosts, combining ~/.ssh/config with isolated per-profile SQLite host databases, embedded PTY sessions with an in-app auth modal, SFTP, tunnels, broadcast, snippets and suggestions, identities, runtime themes, a headless CLI, and an audit log.
tags: [sshub, tui, ssh, rust, overview]
---

# SSHub Quickstart

SSHub (`sshub`, v0.18.2 in `Cargo.toml`) is a keyboard-driven terminal UI for
managing and connecting to SSH hosts. It merges your read-only `~/.ssh/config`
with a managed host database per profile (SQLite) and adds embedded in-TUI SSH
and local shell sessions with a masked authentication modal and a split view
with saved workspaces, an SFTP browser, tunnels with keep-alive reconnect,
broadcast commands, command snippets and ghost-text suggestions, a session log
browser, ssh-agent and certificate-aware identities, group-inherited
connection defaults, a network map of the jump hosts with a hop editor and
hop-by-hop test connect, OS detection with logos, runtime themes, a release
notice with in-place update, and a connection audit log. The dashboard has six
tabs: hosts, SFTP, tunnels, identities, audit, map. A headless CLI covers
scripting (`sshub exec`, `sshub host ...`, transfers, themes, `sshub map
--dot`). License: AGPL-3.0-or-later.

- Crate: `sshub` on crates.io (`cargo install sshub`), npm package
  `sshub-tui`; repo: github.com/Petyok/SSHub.
- Stack: Rust 2021, ratatui 0.30 + crossterm 0.29 (TUI), portable-pty +
  vt100 0.18 (embedded sessions, upstream vt100 with SSHub's own query
  answers, sequence translation and grid painter), rusqlite bundled (SQLite), ssh2 /
  libssh2 with OpenSSL vendored by default (SFTP), nucleo (fuzzy search),
  notify (file watcher), keyring + dbus-secret-service (credentials).
  **No async runtime**: a synchronous event loop polls every 50 ms
  (`src/lib.rs`).
- Single binary: `src/main.rs` dispatches askpass helper → global profile
  flags → `db` → `theme` → headless CLI subcommands → global flags → TUI.

## Install and run

```bash
cargo install sshub          # or: npm install -g sshub-tui, or: git clone ... && just install
sshub                        # launch the TUI
sshub --help                 # global options (--profile, --manage-profiles, --dry-run, --version)
sshub list                   # headless CLI (see workflows/cli.md)
```

Linux builds need `libdbus-1-dev` + `pkg-config`; `--no-default-features`
links the system OpenSSL instead of the vendored one. At runtime a usable
Secret Service provider (gnome-keyring, KWallet, KeePassXC) stores passwords;
without one SSHub falls back to an owner-only `credentials.json`.

## Data paths

| Resource | Default path (profile mode) | Override |
|---|---|---|
| Profile registry | `~/.local/share/sshub/state.toml` | none (compat mode has no registry) |
| Config | `~/.local/share/sshub/profiles/<name>/config.toml` | `SSHUB_CONFIG_DIR` (compat mode) |
| Databases | `~/.local/share/sshub/profiles/<name>/{launcher,metadata}.db` | `SSHUB_DATA_DIR` (compat mode) |
| Pre-migration snapshots | `profiles/<name>/launcher.db.bak-v<from>-<ts>` (newest 3) | follows the database |
| Fallback credentials | `profiles/<name>/credentials.json` | follows the data dir |
| Session logs | `profiles/<name>/logs/<host-dir>/` | follows the data dir |
| Tunnel PID files | `profiles/<name>/tunnels/` | follows the data dir |
| User themes | `profiles/<name>/themes/*.toml` (TUI and `sshub theme`) | compat: `<config dir>/themes/` |
| SSH config | `~/.ssh/config` | `[ssh] config_path` per profile; `SSHUB_SSH_CONFIG` wins |

One profile starts silently; several show a picker after the splash.
`sshub --profile NAME` bypasses it, `sshub --manage-profiles` forces it, and
`Alt+P` manages profiles inside the TUI. Headless commands without `--profile`
use the last-used profile and never open the picker. Setting `SSHUB_DATA_DIR`
or `SSHUB_CONFIG_DIR` (or the legacy `SSH_LAUNCHER_*` names) selects
compatibility mode without profiles. Legacy top-level data migrates into
`profiles/default`. See [themes](architecture/themes.md) for how the themes
directory is resolved.

## Where to go next

### Architecture
- [Runtime architecture](architecture/overview.md): dispatch, startup, the 50 ms event loop, `AppMode` and `active_tab`, render pipeline, workers and channels, source map.
- [Data model & storage](architecture/data-model.md): `launcher.db` schema v21 and its migrations, pre-migration snapshots, group defaults, transfer engine, `metadata.db`, the hybrid host model, `config.toml`.
- [Profiles](architecture/profiles.md): layout, compat mode, startup selection, in-app switching, legacy migration, cross-profile transfers.
- [Theme system internals](architecture/themes.md): registry, resolver, `ThemeManager`, picker; the authoring guide is `docs/theme-system.md`.

### Workflows
- [TUI dashboard](workflows/tui.md): tabs, overlays, keybindings, screens.
- [Network map](workflows/network-map.md): tab 6, the jump-host tree, hop editor, test connect, `sshub map --dot`.
- [Sessions & SFTP](workflows/sessions-sftp.md): embedded sessions, session switcher, credential delivery, terminal compatibility, OSC 52, logging, mosh, SFTP.
- [Session tools](workflows/session-tools.md): auth modal, snippets, ghost-text suggestions, command history, session log browser.
- [Broadcast](workflows/broadcast.md): one command across a group or tag.
- [Tunnels](workflows/tunnels.md): local/remote/dynamic forwards and keep-alive reconnect.
- [Known hosts manager](workflows/known-hosts.md): fingerprints, guarded deletion, host-key decisions.
- [Profiles & themes](workflows/profiles-themes.md): picker, profile manager, transfers, theme picker, `sshub theme`.
- [Headless CLI](workflows/cli.md): command tree, `exec`, `map --dot`, JSON output, exit codes.

### Domain
- [Hosts, groups & identities](domain/hosts-identities.md): host sources, nested groups with defaults and Favorites, identities, certificates, keygen and key push, imports.

### Security & integrations
- [Secrets & security](security/secrets.md): credential store, Secret Service lookup, askpass file and channel, jump-hop credentials, remember opt-in, host-key policy, OSC 52, permissions, input safety.
- [Integrations](integrations/external-terminals.md): external binaries, keyring, release check and update channels, npm packaging, demo pipeline.

### Operations & testing
- [Build, versioning & release](operations/build-release.md): Justfile, features, shared target, odometer versioning, release and hotfix flow.
- [CI & automation](operations/ci-cd.md): GitHub Actions workflows, branch-name check, OpenWiki bot.
- [Testing strategy](testing/strategy.md): unit / smoke / e2e / config / transfer levels, oracle tests, fixtures.

### Wiki maintenance
- [OpenWiki instructions](INSTRUCTIONS.md): generator prompt and the invariant checklist to verify before merging wiki changes.
- [Changelog coverage](changelog-coverage.md): CHANGELOG entries mapped to wiki pages.

## Contributing pointers

Pinned workflow: [docs/implementation-flow.md](../docs/implementation-flow.md)
(issue → claim → branch off `development` → verify → adversarial review → PR).
Run `just test`, `cargo fmt`, `cargo fmt --check` and
`cargo clippy --all-targets` before every push; CI enforces the same with
warnings denied. Branch names follow `feature/`, `fix/`, `docs/`, `chore/`,
`poc/`; see [build & release](operations/build-release.md).

## Backlog

- **Host-sync design** (`docs/host-sync-design.md`): P2P sync of hosts and
  credentials; design only, nothing in `src/`.
- **Detached tunnel PID files** (`src/tunnel/spawn.rs`): an acknowledged race
  (no lock between concurrent `tunnel start`) noted in the code and in
  [tunnels](workflows/tunnels.md). Recycled pids are handled: the pid file
  records the process start time.
- **Changelog follow-ups**: items still marked Pending in
  [changelog coverage](changelog-coverage.md).
