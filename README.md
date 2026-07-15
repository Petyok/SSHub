# SSHub

[![crates.io](https://img.shields.io/crates/v/sshub.svg)](https://crates.io/crates/sshub)

A terminal UI for managing and connecting to SSH hosts. Combines your `~/.ssh/config` with a built-in host database, tunnels, key management, and an audit log -- all in one keyboard-driven interface.

> ⚠️ This project is 100% vibe-coded slop made with dynamic workflows using Claude Opus 4.8 + Fable 5. Use at your own risk.

![SSHub demo](https://raw.githubusercontent.com/Petyok/SSHub/main/demo/gifs/hero.gif)

Navigating the dashboard — nested host groups, the fuzzy palette (`/`), the group manager (`Shift+G`), and the multi-tag filter (`#`):

![Navigation demo](https://raw.githubusercontent.com/Petyok/SSHub/main/demo/gifs/navigate.gif)

Connecting to a host — the session runs in an embedded PTY right inside the TUI:

![Connect demo](https://raw.githubusercontent.com/Petyok/SSHub/main/demo/gifs/connect.gif)

Adding a managed host and marking it as a favorite:

![Add host demo](https://raw.githubusercontent.com/Petyok/SSHub/main/demo/gifs/add-host.gif)

Transferring files over SFTP — a dual-pane browser (remote / local) with a staged transfer queue:

![SFTP demo](https://raw.githubusercontent.com/Petyok/SSHub/main/demo/gifs/sftp.gif)

## Screenshots

The hosts dashboard — nested groups on the left; the selected host's card shows its auto-detected OS logo, fact sheet, and per-host latency, with live agent / ping panels alongside:

![Hosts dashboard](https://raw.githubusercontent.com/Petyok/SSHub/main/demo/screenshots/hosts.png)

Fuzzy quick-connect palette (`/`) and the multi-tag filter (`#`):

![Quick-connect palette](https://raw.githubusercontent.com/Petyok/SSHub/main/demo/screenshots/palette.png)
![Tag filter](https://raw.githubusercontent.com/Petyok/SSHub/main/demo/screenshots/tags.png)

Add/edit host form, the rebindable keybindings editor (`Ctrl+K`), and the scrollable help overlay (`?`):

![Add host form](https://raw.githubusercontent.com/Petyok/SSHub/main/demo/screenshots/add-host.png)
![Keybindings editor](https://raw.githubusercontent.com/Petyok/SSHub/main/demo/screenshots/keybindings.png)
![Help overlay](https://raw.githubusercontent.com/Petyok/SSHub/main/demo/screenshots/help.png)

The settings overlay (`Ctrl+H`) — toggle an opaque background, OS logos, quit confirmation, and the startup animation:

![Settings overlay](https://raw.githubusercontent.com/Petyok/SSHub/main/demo/screenshots/settings.png)

## Features

- **Embedded SSH sessions** — connect opens an in-TUI PTY; detach with Ctrl+D and return to the dashboard while SSH keeps running; multiple session tabs
- **Hosts** — browse, search, and connect. Fuzzy search with `/`, multi-tag AND filter with `#`, favorites, nested groups, manual sort order
- **SFTP file transfer** — a dual-pane browser (remote / local) with a staged transfer queue: navigate both sides, queue uploads and downloads (files or whole folders, transferred recursively), and run them with a progress bar. Manage files in place too: delete (`d`), new folder (`n`), rename/move (`R`), and change permissions (`M`, octal chmod)
- **OS auto-detection** — on first connect a background probe detects the remote distro and the host card renders its logo (Braille art in brand colors), just like Termius
- **Multiple groups & Favorites** — a host can belong to several groups at once; a reserved Favorites group and a ★ marker in the list, toggled with `f`
- **Tunnels** — define and manage SSH tunnels (local/remote/dynamic SOCKS). Start, stop, and monitor from the TUI
- **Keys** — identity management with ssh-agent integration. Add/remove keys from agent, see loaded status
- **Audit** — log of all connection events with filtering by status (ok/fail) and time range (today/week/month); session connect events record the path to the session log when logging is enabled
- **Session logging** — opt-in capture of PTY session output to `~/.local/share/sshub/logs/<host>/`. Enable globally in Settings (`Ctrl+H`) or override per host (`inherit` / `on` / `off`). **Logs capture everything echoed to the terminal, including passwords if they appear on screen.**
- **Settings overlay** (`Ctrl+H`) — toggle session logging, opaque background (for transparent terminals), OS logos, quit confirmation, and the startup animation
- **Hybrid sources** — hosts from `~/.ssh/config` (read-only) and launcher-managed (full CRUD) merge without duplicates
- **Import/Export** — import from `~/.ssh/config` or Termius backups; export managed hosts back to ssh config format
- **Hot reload** — edits to `~/.ssh/config` update the host list live via file watcher
- **Configurable keybindings** — rebind any action via Ctrl+K; stored in `config.toml`
- **Mouse support** — click tabs, select rows, scroll panels, double-click to connect

## Install

From [crates.io](https://crates.io/crates/sshub):

```bash
cargo install sshub
```

Requires a Rust toolchain (edition 2021) and `ssh` in `PATH`.

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

Prebuilt binaries for Linux and macOS are attached to each [GitHub release](https://github.com/Petyok/SSHub/releases).

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
| `1`..`5`         | Switch tab (hosts/sftp/tunnels/identities/audit) |
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
[session_logging]
enabled = false
max_file_bytes = 10485760   # rotate at 10 MiB
retention_files = 50        # keep newest 50 logs per host

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

[AGPL-3.0-or-later](LICENSE) — a copyleft license: forks and derivatives must
stay open under the same terms. (Versions ≤ 0.3.1 were released under MIT.)

## Changelog

See [CHANGELOG.md](CHANGELOG.md).
