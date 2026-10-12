---
type: API Reference
title: Headless CLI - full command tree, JSON output, and exit codes
description: SSHub's scriptable command-line interface (src/cli) covering hosts, exec, groups with connection defaults, identities, tunnels, SFTP, audit, cross-profile transfer, headless theme checks, import/export/sync, and completions, with --profile selection, --format json, and stable exit codes 0/1/2 (124 for `exec --timeout`).
resource: src/cli/mod.rs
tags: [cli, automation, json, reference, workflow]
---

# Headless CLI

Beyond the TUI, `sshub` exposes a scriptable CLI. `src/main.rs` dispatches in this order:

1. `session::askpass::maybe_run_askpass()` (the binary re-executed as `SSH_ASKPASS`).
2. Global profile flags are extracted from everything before a `--` (`profile::extract_startup_flags`): `--profile NAME` selects a profile (unknown names fail with the list), `--manage-profiles` forces the TUI picker. Arguments after `--` belong to a remote command and are never scanned.
3. `db` subcommand.
4. `theme` subcommand, dispatched before any database is opened.
5. `cli::is_subcommand` (cheap string check), then `run_cli`: `resolve_cli_profile` picks `--profile` or the last-used profile (never a picker), and `CliContext::bootstrap_with` (`src/cli/context.rs`) loads that profile's config, both databases, the resolver and keyring, so the CLI shares state with the TUI ([data model](../architecture/data-model.md)).
6. Global `--help` / `--version` / `--dry-run`, otherwise the TUI. An unknown positional first argument exits 2 with a hint instead of launching the full-screen app.

Directory overrides (`SSHUB_CONFIG_DIR`, `SSHUB_DATA_DIR`) put SSHub in compatibility mode, where `--profile` is rejected.

Parsing is hand-rolled (`src/cli/parse.rs`), output DTOs live in `src/cli/output.rs`. Conventions:

- `--format plain|json` on listing/show commands (plain default); `theme show` takes `toml|json`.
- Exit codes: `0` success, `1` operational failure, `2` usage error, `124` when `exec --timeout` kills the run. `host connect` and `exec` propagate the child ssh exit code (ssh's own failures are 255).
- Destructive commands refuse without `--yes`; `sshub db purge` requires `--yes-i-am-stupid`.

## Command tree

| Command | Subcommands | Notes |
|---|---|---|
| `host` (aliases `list`, `connect`) | `list show connect resolve search add edit rename delete duplicate transfer` | `list [--tag T]... [--group G] [--sort label\|last-connected\|favorite\|group\|manual]`. `add --name --address [--port] [--username] [--label] [--identity] [--group G]... [--tags a,b] [--notes] [--proxy-jump] [--forward-agent\|--no-forward-agent] [--remote-command] [--transport ssh\|mosh] [--session-log inherit\|on\|off] [--ping inherit\|on\|off] [--os-icon] [--favorite] [--secret-lookup ATTR=VALUE] [--password-stdin]`. `edit --name NAME` takes `--set-*` / `--clear-*` pairs for the same fields plus `--set-group G...`, `--set-ping`, `--set-secret-lookup` / `--clear-secret-lookup`, `--set-password-stdin` / `--clear-password`. `duplicate --name NAME [--new-name NEW]`. `connect` runs ssh or mosh as a foreground child with inherited stdio (`Command::spawn` + `wait`), wrapped in `script(1)` when session logging applies. `--secret-lookup` reads the host password from the single Secret Service entry (for example KeePassXC) whose attribute matches exactly. |
| `exec` | none | `sshub exec <host> [--tty] [--timeout SECS] [--format plain\|json] [-v] -- <command>` (or one quoted command): one non-interactive command on a saved host (`src/cli/exec.rs`). Reuses the connect argv (`resolved_session_argv` / `session_argv_for_entry` + `prepare_cli_connect_argv`), then adds `-T` (or `-tt` with `--tty`), `-o RemoteCommand=none` (a stored or ssh_config remote command must not win) and `-o BatchMode=yes` when no stored secret is staged, so a script never sits on a prompt. stdio passes through, stdin is inherited; `--timeout` puts the child in its own process group and SIGKILLs the group. `--format json` buffers `{host, command, exit_code, stdout, stderr, duration_ms}`. Audited as `via exec` without the command text. No session transcript; mosh hosts are refused. |
| `group` (alias `groups` = `group list`) | `list show add edit delete transfer` | Nested groups via `--parent`. Connection defaults inherited by member hosts: `add --default-identity --default-username --default-port --default-proxy-jump --default-transport ssh\|mosh --default-fwd-agent\|--no-default-fwd-agent`; `edit` has matching `--set-default-*` / `--clear-default-*` flags plus `--set-name`, `--set-parent` / `--clear-parent`, `--set-sort-order`. |
| `identity` | `list show add edit delete agent-remove transfer` | `add --private-key`, `--certificate`, `--password-stdin` for the passphrase; `agent-remove` = `ssh-add -d`, audited as `via agent`. |
| `tunnel` | `list show create start stop delete transfer` | `<id>` accepts id, label or local port. `create --host --type local\|remote\|dynamic --local-port [--remote-host] [--remote-port] [--label] [--keep-alive]`. `start` is detached by default (PID files), `--foreground` runs it under the reconnect manager ([tunnels](tunnels.md)). |
| `sftp` | `ls get put rm mkdir rename chmod` | One-shot over a direct host; ProxyJump refused; `get`/`put`/`rm` take `--recursive` ([SFTP](sessions-sftp.md#sftp-srcsftp)). |
| `audit` | `list stats` | `list [--status all\|ok\|fail\|retry] [--via all\|connect\|tunnel\|agent\|exec] [--host NAME] [--limit N] [--days N]`; `stats [--days N] [--via ...] [--include-retry]`. `connect` means interactive connects only. Broadcast rows (`via broadcast`) appear under `all`. |
| `theme` | `check list show` | Headless, opens no database (`src/cli/theme.rs`): `check <file>` validates a draft theme, `list` shows built-in and installed themes with state, `show <id> [--resolved]` prints the source or the flattened theme. Reads `~/.config/sshub/themes/` (see [profiles & themes](profiles-themes.md#themes)). |
| `tags` | none | List all tags. |
| `import` / `sync` / `export` | none | `import [--from ssh\|termius\|putty\|mremoteng] [--dry-run] [PATH]`; `sync` refreshes ssh_config rows; `export [--stdout] [-o PATH]`. |
| `completions` | `bash zsh fish` | `[--cache PATH]`; installed by `just install-completions`. |
| `db` | `purge` | Deletes the selected profile's `launcher.db` plus `-wal`/`-shm`/`-journal` sidecars; keyring entries are left orphaned. |

**Cross-profile transfer** (`src/cli/transfer.rs`, engine `src/store/transfer.rs`): `sshub host|group|identity transfer --name NAME --to PROFILE [--move] [--yes]`, `sshub tunnel transfer <id|label|port> --to PROFILE [--move] [--yes]`, and `--with-hosts` for groups. The plan (renames and blocked items) always prints; it applies only after `y` at the prompt or with `--yes`. `--move` deletes the source originals after copying. Identities carry the key path only, never secret material; the reserved Favorites group cannot be transferred. The TUI flow is in [profiles & themes](profiles-themes.md#moving-data-between-profiles).

Examples:

```bash
sshub host add --name prod-web --address 10.0.0.5 --port 22 --username deploy --group prod --tags web,prod
sshub group edit --name prod --set-default-identity deploy-key --set-default-port 2222
sshub tunnel create --host prod-web --type local --local-port 8080 --remote-host localhost --remote-port 80
sshub sftp get prod-web /var/log/app.log ./app.log
sshub exec prod-web --timeout 30 -- systemctl is-active nginx
sshub --profile work host transfer --name prod-web --to personal
sshub audit list --status fail --days 7
```

Per-command help: `sshub <command> --help` (`src/cli/help.rs`); for `exec`, a `-h` after `--` belongs to the remote command. The man page (`man/sshub.1`, preview with `just man`) covers the same surface.

## Change guidance

- New subcommand: register in `src/cli/mod.rs` (`is_subcommand` + `run_subcommand`), add a module under `src/cli/`, its help block in `src/cli/help.rs` and the section in `main.rs::print_help`, list it in `src/cli/completions.rs` (`TOP_LEVEL` plus the bash/zsh/fish bodies), extend the man page, the README table and this page.
- CLI smoke coverage lives in `tests/smoke/cli_commands.rs` (drives the real binary via `assert_cmd`); transfer has `tests/transfer_cli.rs` and `tests/transfer_engine.rs`. See [testing](../testing/strategy.md).
- Keep exit codes stable; scripts depend on them.
