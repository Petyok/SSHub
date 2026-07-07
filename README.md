# SSHub

A terminal UI for managing and connecting to SSH hosts. Combines your `~/.ssh/config` with a built-in host database, tunnels, key management, and an audit log -- all in one keyboard-driven interface.

> ⚠️ This project is 100% vibe-coded slop made with dynamic workflows using Claude Opus 4.8. Use at your own risk.

![SSHub demo](demo/gifs/overview.gif)

Connecting to a host — the session runs in an embedded PTY right inside the TUI:

![Connect demo](demo/gifs/connect.gif)

Adding a managed host and marking it as a favorite:

![Add host demo](demo/gifs/add-host.gif)

## Features

- **Embedded SSH sessions** — connect opens an in-TUI PTY; detach with Ctrl+D and return to the dashboard while SSH keeps running; multiple session tabs
- **Hosts** — browse, search, and connect. Fuzzy search with `/`, multi-tag AND filter with `#`, favorites, nested groups, manual sort order
- **Tunnels** — define and manage SSH tunnels (local/remote/dynamic SOCKS). Start, stop, and monitor from the TUI
- **Keys** — identity management with ssh-agent integration. Add/remove keys from agent, see loaded status
- **Audit** — log of all connection events with filtering by status (ok/fail) and time range (today/week/month)
- **Hybrid sources** — hosts from `~/.ssh/config` (read-only) and launcher-managed (full CRUD) merge without duplicates
- **Import/Export** — import from `~/.ssh/config` or Termius backups; export managed hosts back to ssh config format
- **Hot reload** — edits to `~/.ssh/config` update the host list live via file watcher
- **Configurable keybindings** — rebind any action via Ctrl+K; stored in `config.toml`
- **Mouse support** — click tabs, select rows, scroll panels, double-click to connect

## Install

Requires Rust toolchain (edition 2021) and `ssh` in `PATH`.

On Linux, building also needs the D-Bus client library for the keyring
(Secret Service) backend that stores host passwords and key passphrases:

```bash
# Debian/Ubuntu
sudo apt-get install -y libdbus-1-dev pkg-config
# Fedora
sudo dnf install -y dbus-devel pkgconf-pkg-config
# Arch
sudo pacman -S --needed dbus
```

At runtime, a Secret Service provider (gnome-keyring, KWallet, …) must be
running and unlocked for credentials to persist; otherwise SSHub warns and ssh
falls back to prompting.

```bash
git clone https://github.com/Petyok/SSHub.git
cd SSHub
just install    # builds release binary + desktop entry + ~/.local/bin/sshub
```

Or build only:

```bash
just build
cp target/release/sshub ~/.local/bin/
```

## Usage

```bash
sshub              # launch TUI
sshub --version    # print version
sshub --dry-run    # exit immediately (CI / scripts)
sshub --help       # show options
```

### Commands

```bash
# Wipe the launcher database — managed hosts, groups, identities, tunnels and
# the audit log. Irreversible, so it refuses unless you confirm. Your
# ~/.ssh/config (and the hosts imported from it) are left untouched.
sshub db purge --yes-i-am-stupid
```

### Data paths

| Resource   | Default path                          |
|------------|---------------------------------------|
| Config     | `~/.config/sshub/config.toml`         |
| Database   | `~/.local/share/sshub/launcher.db`    |
| SSH config | `~/.ssh/config`                       |

Override via environment variables: `SSHUB_CONFIG_DIR`, `SSHUB_DATA_DIR`, `SSHUB_SSH_CONFIG`.

## Keybindings

Defaults below. Rebind any action with **Ctrl+K** (saved to `config.toml`). Press `?` in-app for the full list.

### Global

| Key              | Action                          |
|------------------|---------------------------------|
| `1`..`4`         | Switch tab (hosts/tunnels/keys/audit) |
| `Tab`            | Toggle detail panel             |
| `Esc`            | Back / close overlay            |
| `Ctrl+K`         | Keybind editor                  |
| `?` / `Shift+H`  | Help screen                     |
| `q`              | Quit                            |

### Session (embedded PTY)

| Key                    | Action                              |
|------------------------|-------------------------------------|
| `Ctrl+T`               | New session tab (host picker)         |
| `Ctrl+W`               | Close session tab                   |
| `Ctrl+D`               | Detach to dashboard (SSH keeps running) |
| `Ctrl+[` / `Ctrl+]`   | Previous / next session tab         |
| `Ctrl+Shift+S`         | Focus session from dashboard        |

### Hosts (tab 1)

| Key                | Action                    |
|--------------------|---------------------------|
| `j`/`k` or arrows | Navigate                  |
| `Enter`            | Connect to host           |
| `a`                | Add host                  |
| `e`                | Edit host / group identity |
| `d`                | Delete host               |
| `D`                | Duplicate host            |
| `f`                | Toggle favorite           |
| `s`                | Cycle sort mode           |
| `/`                | Fuzzy search              |
| `#`                | Filter by tags (AND)      |
| `Shift+G`          | Manage groups (nested)    |
| `Shift+I`          | Import from ssh config    |
| `Shift+E`          | Export to ssh config      |
| `Shift+T`          | Import from Termius       |

### Tunnels (tab 2)

| Key       | Action              |
|-----------|----------------------|
| `a`       | Add tunnel           |
| `e`       | Edit tunnel          |
| `d`       | Delete tunnel        |
| `Enter`   | Start / stop tunnel  |
| `x`       | Kill tunnel process  |

### Keys (tab 3)

| Key        | Action                  |
|------------|--------------------------|
| `a`        | Add identity             |
| `e`        | Edit identity            |
| `d`        | Delete identity          |
| `r`        | Remove key from agent    |
| `Shift+A`  | Add key to agent         |

### Audit (tab 4)

| Key | Action                              |
|-----|--------------------------------------|
| `f` | Cycle filter (all / ok / fail)       |
| `r` | Cycle range (all / today / week / month) |

## Configuration

`~/.config/sshub/config.toml`:

```toml
[terminal]
# "kitty", "ghostty", or a custom command template
launcher = "kitty"
# custom_command = "alacritty -e ssh {host}"
```

## Development

```bash
just build             # release binary
just test              # all tests (unit + smoke + e2e + config)
cargo run -- --dry-run # quick sanity check
```

### Test levels

| Level    | Command                       | What it checks                       |
|----------|-------------------------------|--------------------------------------|
| Unit     | `cargo test`                  | Logic, parsers, fixtures -- no TTY   |
| Smoke    | `cargo test --test smoke`     | Binary starts, `--help`, `--dry-run` |
| E2E      | `cargo test --test e2e`       | TUI scenarios via TestBackend        |
| Config   | `cargo test --test config_load` | Config file creation and loading   |

### Environment variables

| Variable           | Purpose                                    |
|--------------------|--------------------------------------------|
| `SSHUB_CONFIG_DIR` | Override config directory                  |
| `SSHUB_DATA_DIR`   | Override data/SQLite directory             |
| `SSHUB_SSH_CONFIG`  | Override SSH config file path              |
| `SSHUB_DRY_RUN`    | Exit immediately without TUI              |
| `SSHUB_AUTO_QUIT`  | `1` = quit after first draw, `q` = send quit key |

## Tech stack

[Rust](https://www.rust-lang.org/) with [ratatui](https://ratatui.rs/) + [crossterm](https://github.com/crossterm-rs/crossterm) for the TUI, [rusqlite](https://github.com/rusqlite/rusqlite) (bundled SQLite) for storage, [nucleo](https://github.com/helix-editor/nucleo) for fuzzy search, [notify](https://github.com/notify-rs/notify) for file watching. No async runtime -- synchronous event loop with 50ms polling.

## License

[MIT](LICENSE)

## Changelog

See [CHANGELOG.md](CHANGELOG.md). Current release: **0.2.0**.
