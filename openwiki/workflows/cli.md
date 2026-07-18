---
type: API Reference
title: Headless CLI — full command tree, JSON output, and exit codes
description: SSHub's scriptable command-line interface (src/cli) covering hosts, groups, identities, tunnels, SFTP, audit, import/export/sync, and completions, with --format json support and stable exit codes 0/1/2.
resource: src/cli/mod.rs
tags: [cli, automation, json, reference, workflow]
---

# Headless CLI

Beyond the TUI, `sshub` exposes a full scriptable CLI. `src/main.rs` dispatches in order: askpass re-exec → `db` subcommand → `cli::is_subcommand` (cheap string check, no bootstrap) → global flags → TUI. `CliContext::bootstrap()` (`src/cli/context.rs`) loads config, opens both databases, builds the resolver and `OsKeyring`, and loads merged hosts — so the CLI shares all state with the TUI ([data model](../architecture/data-model.md)).

Parsing is hand-rolled (`src/cli/parse.rs`), output DTOs in `src/cli/output.rs`. Conventions:

- `--format plain|json` on listing/show commands (plain default).
- Exit codes: `0` success, `1` operational failure, `2` usage/bad flags. `host connect` propagates the child ssh exit code.
- Destructive commands refuse without `--yes`; `sshub db purge` requires `--yes-i-am-stupid`.
- Unknown positional first arg exits 2 with a hint (avoids launching a full-screen TUI on a typo).

## Command tree

| Command | Subcommands | Notes |
|---|---|---|
| `host` (alias `list`, `connect`) | `list show connect resolve search add edit rename delete duplicate` | `add` takes `--name --address --port --username --group --tags`; `connect` runs ssh/mosh as a foreground child process with inherited stdio (Command::spawn + wait), propagating its exit code; it does not use the TUI embedded-PTY session module or the (dead-code) external TerminalLauncher |
| `group` (alias `groups`) | `list show add edit delete` | Nested groups via parent |
| `identity` | `list show add edit delete agent-remove` | `add --private-key`, `--password-stdin` for secrets; `agent-remove` = `ssh-add -d` |
| `tunnel` | `list show create start stop delete` | `start` is detached by default (PID files), `--foreground` runs with keep-alive ([tunnels](tunnels.md)) |
| `sftp` | `ls get put rm mkdir rename chmod` | One-shot over a direct host; no ProxyJump ([sessions & SFTP](sessions-sftp.md#sftp)) |
| `audit` | `list stats` | `--status ok|fail`, `--days N` |
| `tags` | — | List all tags |
| `import` / `sync` / `export` | — | ssh config import, row refresh, `export --stdout|-o` |
| `completions` | `bash zsh fish` | Installed by `just install-completions` |
| `db` | `purge` | Deletes `launcher.db` + sidecars only |

Examples (from `README.md`):

```bash
sshub host add --name prod-web --address 10.0.0.5 --port 22 --username deploy --group prod --tags web,prod
sshub tunnel create --host prod-web --type local --local-port 8080 --remote-host localhost --remote-port 80
sshub sftp get prod-web /var/log/app.log ./app.log
sshub audit list --status fail --days 7
```

Per-command help: `sshub <command> --help`; the man page (`man/sshub.1`, preview with `just man`) covers the same surface.

## Change guidance

- New subcommand: register in `src/cli/mod.rs` (`is_subcommand` + `run_subcommand`), add a module under `src/cli/`, extend the man page and README table.
- CLI smoke coverage lives in `tests/smoke/cli_commands.rs` (drives the real binary via `assert_cmd`) — see [testing](../testing/strategy.md).
- Keep exit codes stable; scripts depend on them.
