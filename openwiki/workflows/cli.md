---
type: API Reference
title: Headless CLI — full command tree, JSON output, and exit codes
description: SSHub's scriptable command-line interface (src/cli) covering hosts, exec, groups, identities, tunnels, SFTP, audit, import/export/sync, and completions, with --format json support and stable exit codes 0/1/2 (plus 124 for `exec --timeout`).
resource: src/cli/mod.rs
tags: [cli, automation, json, reference, workflow]
---

# Headless CLI

Beyond the TUI, `sshub` exposes a full scriptable CLI. `src/main.rs` dispatches in order: askpass re-exec → `db` subcommand → `cli::is_subcommand` (cheap string check, no bootstrap) → global flags → TUI. `CliContext::bootstrap()` (`src/cli/context.rs`) loads config, opens both databases, builds the resolver and `OsKeyring`, and loads merged hosts — so the CLI shares all state with the TUI ([data model](../architecture/data-model.md)).

Parsing is hand-rolled (`src/cli/parse.rs`), output DTOs in `src/cli/output.rs`. Conventions:

- `--format plain|json` on listing/show commands (plain default).
- Exit codes: `0` success, `1` operational failure, `2` usage/bad flags, `124` when `exec --timeout` kills the run. `host connect` and `exec` propagate the child ssh exit code.
- Destructive commands refuse without `--yes`; `sshub db purge` requires `--yes-i-am-stupid`.
- Unknown positional first arg exits 2 with a hint (avoids launching a full-screen TUI on a typo).

## Command tree

| Command | Subcommands | Notes |
|---|---|---|
| `host` (alias `list`, `connect`) | `list show connect resolve search add edit rename delete duplicate` | `add` takes `--name --address --port --username --group --tags`; `connect` runs ssh/mosh as a foreground child process with inherited stdio (Command::spawn + wait), propagating its exit code; it does not use the TUI embedded-PTY session module or the (dead-code) external TerminalLauncher |
| `exec` | — | `sshub exec <host> [--tty] [--timeout SECS] [--format plain\|json] -- <command>`: one non-interactive command on a saved host (`src/cli/exec.rs`). Reuses the connect argv (`session_argv_for_entry` + `prepare_cli_connect_argv`), then adds `-T`/`-tt`, `-o RemoteCommand=none` (a per-host stored remote command must not win, and a config-level one makes ssh refuse the run) and `-o BatchMode=yes` when no stored secret is staged, so a script can never sit on a prompt. stdio passes through, stdin inherited; `--timeout` spawns into its own process group and SIGKILLs the group (a ProxyJump helper would otherwise hold the pipes open). Audited as `via exec`, without the command string. No session transcript; mosh refused |
| `group` (alias `groups`) | `list show add edit delete` | Nested groups via parent |
| `identity` | `list show add edit delete agent-remove` | `add --private-key`, `--password-stdin` for secrets; `agent-remove` = `ssh-add -d` |
| `tunnel` | `list show create start stop delete` | `start` is detached by default (PID files), `--foreground` runs with keep-alive ([tunnels](tunnels.md)) |
<!-- openwiki: broken internal link [sessions-sftp.md#sftp] heading anchor "sftp" does not exist in "sessions-sftp.md". Fix the href or restore the target, then delete this comment. -->
| `sftp` | `ls get put rm mkdir rename chmod` | One-shot over a direct host; no ProxyJump ([sessions & SFTP](sessions-sftp.md#sftp)) |
| `audit` | `list stats` | `--status ok|fail`, `--days N`, `--via all|connect|tunnel|agent|exec` (`connect` means interactive connects only) |
| `tags` | — | List all tags |
| `import` / `sync` / `export` | — | ssh config import, row refresh, `export --stdout|-o` |
| `completions` | `bash zsh fish` | Installed by `just install-completions` |
| `db` | `purge` | Deletes `launcher.db` + sidecars only |

Examples (from `README.md`):

```bash
sshub host add --name prod-web --address 10.0.0.5 --port 22 --username deploy --group prod --tags web,prod
sshub tunnel create --host prod-web --type local --local-port 8080 --remote-host localhost --remote-port 80
sshub sftp get prod-web /var/log/app.log ./app.log
sshub exec prod-web --timeout 30 -- systemctl is-active nginx
sshub audit list --status fail --days 7
```

Per-command help: `sshub <command> --help`; the man page (`man/sshub.1`, preview with `just man`) covers the same surface.

## Change guidance

- New subcommand: register in `src/cli/mod.rs` (`is_subcommand` + `run_subcommand`), add a module under `src/cli/`, add its help block in `src/cli/help.rs` and the section in `main.rs::print_help`, list it in `src/cli/completions.rs` (`TOP_LEVEL` plus the bash/zsh/fish bodies), extend the man page, the README table and this page.
- CLI smoke coverage lives in `tests/smoke/cli_commands.rs` (drives the real binary via `assert_cmd`) — see [testing](../testing/strategy.md).
- Keep exit codes stable; scripts depend on them.
