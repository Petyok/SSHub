# Configuration & runtime environment

SSHub stores user-level configuration, database, and logs under XDG-style paths. The defaults can be overridden with environment variables.

## Paths

| Resource | Default | Override env var |
|----------|---------|------------------|
| Config file | `~/.config/sshub/config.toml` | `SSHUB_CONFIG_DIR` (fallback `SSH_LAUNCHER_CONFIG_DIR`) |
| Data dir / DB | `~/.local/share/sshub/launcher.db` | `SSHUB_DATA_DIR` (fallback `SSH_LAUNCHER_DATA_DIR`) |
| SSH config | `~/.ssh/config` | `SSHUB_SSH_CONFIG` (fallback `SSH_LAUNCHER_SSH_CONFIG`) |
| Session logs | `<data_dir>/logs/<host-dir>/` | via `SSHUB_DATA_DIR` |

Old `SSH_LAUNCHER_*` names are still accepted for backward compatibility.

## config.toml sections

`src/config.rs` defines `AppConfig`.

### `[appearance]`

- `show_detail_panel` — show the right-side host detail panel.
- `date_format` — date format for audit log.
- `disable_animation` — skip the startup splash.
- `confirm_quit` — prompt before `q` / `Ctrl+C`.
- `identity_columns` — identities grid width; `0` = auto.
- `os_logo` — render detected OS logo in host detail.
- `opaque_background` — solid backdrop for transparent terminals.

### `[session_logging]`

- `enabled` — global on/off switch.
- `max_file_bytes` — rotation size (default 10 MB).
- `retention_files` — number of segment files to keep per host (default 50).

### `[tunnel_reconnect]`

Global keep-alive reconnect backoff:

- `max_attempts` — 0 = unlimited; default 12.
- `initial_delay_ms` — first retry wait; default 1000.
- `max_delay_ms` — backoff cap; default 60000.
- `jitter_ratio` — ± spread; default 0.25.
- `stable_secs` — uptime required to count as stable; default 5.

These values are edited on the Tunnels tab with `R` and saved back to disk. The overlay displays delay fields in seconds for readability; `config.toml` still stores millisecond fields (`initial_delay_ms`, `max_delay_ms`).

### `[keybindings]`

Maps action names to arrays of key specs (see [Keybindings](../tui/keybindings.md)).

### Top-level fields

- `terminal` — `Kitty`, `Ghostty`, or `Custom`.
- `launch_command` — required when `terminal = "Custom"`.

The external launcher path is mostly legacy; the embedded PTY is now the default connect experience.

## Environment variables for CI / headless

- `SSHUB_DRY_RUN` / `SSH_LAUNCHER_DRY_RUN` — `run()` exits immediately, TUI never opens.
- `SSHUB_AUTO_QUIT` / `SSH_LAUNCHER_AUTO_QUIT`:
  - `1` — quit after first draw (used by smoke tests),
  - `q` — simulate pressing the quit key.
- `SSHUB_VERSION_LABEL` — override the version shown in the tab bar.

These are used in `src/lib.rs`, `src/main.rs`, and the smoke tests.

## Config loading behavior

- `config::load_config()` reads the file if it exists; otherwise returns defaults.
- Parent directory is created if needed.
- Saves preserve hand-written comments and unknown keys via `toml_edit`.
- Keybindings can be partially overridden; missing actions use code defaults.

## What to watch when changing config

- `src/config.rs` — add the new field with serde defaults so existing configs load.
- Default value functions are `default_true`, `default_session_log_max_bytes`, `default_tunnel_reconnect_*`, etc.
- `src/tui/screens/help.rs` and settings overlay rendering if a new setting is exposed in the TUI.
- `Justfile` / `tests/smoke/config_load.rs` if the smoke test asserts config creation.
