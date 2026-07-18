# Headless CLI

SSHub ships a full command-line interface alongside the TUI. Every launcher
resource (hosts, groups, identities, tunnels, the audit log) is reachable from
scripts without opening the terminal UI, and SFTP file operations run one-shot
over a direct host. The CLI is hand-rolled (no clap): `main.rs` recognizes a
subcommand token before bootstrapping a `CliContext`, then dispatches through
`src/cli/mod.rs::run_subcommand`.

## Invocation model

- `sshub <command> [args]` runs a subcommand and exits; no TUI is drawn.
- `sshub <command> --help` (or `-h`) prints a per-command usage block from
  `src/cli/help.rs` instead of the global help.
- Bare `sshub` with no recognized subcommand launches the TUI as before.

Dispatch lives in `src/cli/mod.rs`. Each family has its own module:
`host.rs`, `group.rs`, `identity.rs`, `tunnel.rs`, `sftp.rs`, `audit.rs`,
`inventory.rs` (tags/sync/import/export), and `completions.rs`. Shared argument
parsing is in `src/cli/parse.rs`; output formatting in `src/cli/output.rs`.

## Command tree

### Hosts (`host`, with aliases)

```
sshub host list [--tag TAG]... [--group GROUP] [--sort MODE] [--format plain|json]
sshub host show <name> [--format plain|json]
sshub host connect <name> [-v|--verbose]
sshub host resolve <name> [-v|--verbose] [--format plain|json]
sshub host search <query> [--format plain|json]
sshub host add --name NAME --address ADDR [--port N] [--username USER]
              [--group GROUP]... [--identity NAME] [--tags a,b] [--label ...]
              [--notes ...] [--proxy-jump ...] [--remote-command ...]
              [--forward-agent|--no-forward-agent] [--transport ssh|mosh]
              [--session-log inherit|on|off] [--os-icon ...] [--favorite]
              [--password-stdin]
sshub host edit --name NAME --set-* ...    (at least one --set-* / --clear-* flag)
sshub host rename --name NAME --new-name NAME [--strict]
sshub host delete --name NAME --yes
sshub host duplicate --name NAME [--new-name NAME] [--format plain|json]
```

Top-level aliases forward into the `host` family:

- `sshub connect <name>` is `sshub host connect <name>`.
- `sshub list ...` is `sshub host list ...`.
- `sshub groups ...` is `sshub group list ...`.

`resolve` prints the effective SSH argv and metadata for a host without
connecting. `connect` inherits stdio, records an audit event, and returns the
child ssh exit code.

### Groups (`group`)

```
sshub group list [--all] [--format plain|json]
sshub group show <name> [--format plain|json]
sshub group add --name NAME [--parent GROUP] [--default-identity NAME] [--sort-order N]
sshub group edit --name NAME [--set-name ...] [--set-parent ...] [--clear-parent]
                 [--set-default-identity ...] [--clear-default-identity] [--set-sort-order N]
sshub group delete --name NAME --yes
```

### Identities (`identity`)

```
sshub identity list [--format plain|json]
sshub identity show <name> [--format plain|json]
sshub identity add --name NAME [--username USER] [--private-key PATH]
                   [--certificate PATH] [--password-stdin]
sshub identity edit --name NAME [--set-name ...] [--set-username ...] [--clear-username]
                    [--set-private-key PATH] [--clear-private-key]
                    [--set-certificate PATH] [--clear-certificate]
                    [--password-stdin] [--clear-password]
sshub identity delete --name NAME --yes
sshub identity agent-remove --name NAME
```

`agent-remove` runs `ssh-add -d` for the identity's private key and logs the
result to the audit log. Deleting an identity still referenced by hosts fails
with exit code `2`.

### Tunnels (`tunnel`)

```
sshub tunnel list [--format plain|json]
sshub tunnel show <token> [--format plain|json]
sshub tunnel create --host NAME --local-port N [--type local|remote|dynamic]
                    [--remote-host HOST] [--remote-port N] [--label ...] [--keep-alive]
sshub tunnel start <token> [--foreground]
sshub tunnel stop <token>
sshub tunnel delete <token> --yes
```

`<token>` resolves a tunnel by id, label, or local port. `start` spawns a
detached background `ssh -N` by default and prints its pid; `start --foreground`
runs the tunnel in the current process with the keep-alive reconnect loop and
blocks until it stops or gives up.

### SFTP (`sftp`)

One-shot file operations over a direct host (ProxyJump hosts are rejected up
front, since the libssh2 transport cannot chain a jump). Each subcommand drives
the background SFTP worker synchronously.

```
sshub sftp ls <host> [remote-path] [--format plain|json]
sshub sftp get <host> <remote-path> [local-path] [--recursive]
sshub sftp put <host> <local-path> [remote-path] [--recursive]
sshub sftp rm <host> <remote-path> [--recursive] --yes
sshub sftp mkdir <host> <remote-path>
sshub sftp rename <host> <from> <to>
sshub sftp chmod <host> <mode> <remote-path>       (mode is octal, e.g. 644)
```

A local directory passed to `put` is always transferred recursively; `get`
needs `--recursive` to walk a remote tree.

### Audit log (`audit`)

```
sshub audit list [--status all|ok|fail|retry] [--via all|connect|tunnel|agent]
                 [--host HOST] [--limit N] [--days N] [--format plain|json]
sshub audit stats [--days N] [--via all|connect|tunnel|agent]
                  [--include-retry] [--format plain|json]
```

### Inventory (`tags`, `sync`, `import`, `export`)

```
sshub tags [--format plain|json]
sshub sync                                  # refresh ssh_config rows in the DB
sshub import                                # import hosts from ~/.ssh/config
sshub export [--stdout] [-o PATH]           # write an ssh_config snippet
```

### Completions (`completions`)

```
sshub completions bash|zsh|fish [--cache PATH]
```

Generates a static completion tree (top-level commands, per-family subcommands)
plus the current host names. `--cache PATH` writes the host-name list to a file
and reuses it on later runs, so completion stays fast on large inventories.
Without a cache the generator shells out to `sshub host list --format json`.

### Database (`db`)

```
sshub db purge --yes-i-am-stupid
```

Wipes the launcher database (managed hosts, groups, identities, tunnels, audit
log). Irreversible, so it carries a stronger confirmation flag than the ordinary
`--yes` guard. `~/.ssh/config` and the hosts imported from it are untouched.

## Output format contract

Commands that produce a listing or a record accept `--format plain|json`:

- `plain` (the default) is line-oriented human-readable text.
- `json` is pretty-printed JSON, stable enough to pipe into `jq`.

Commands that only report an action (create, delete, sync, import) print a
short status line and do not take `--format`. `parse_format` in
`src/cli/parse.rs` is the single source of truth for the flag.

## Exit-code convention

Every subcommand returns one of three process exit codes:

| Code | Meaning                                                        |
|------|---------------------------------------------------------------|
| `0`  | Success.                                                       |
| `1`  | Operational failure (host not found, connect failed, IO error, a refused destructive command without `--yes`). |
| `2`  | Usage error or bad flags (unknown subcommand, missing required argument, invalid value). |

The helpers `usage()` (exit 2), `fail()` (exit 1), and `fail_code(msg, code)`
in `src/cli/parse.rs` centralize these. A few semantic conflicts also map to
`2`, for example deleting an identity that is still in use by hosts.

## Destructive-confirmation rules

Commands that remove data refuse to run unless confirmed:

- `host delete`, `group delete`, `identity delete`, `tunnel delete`, and
  `sftp rm` require `--yes`. Without it they print a diagnostic and exit `1`.
- `db purge` requires the stronger `--yes-i-am-stupid` flag, because it wipes
  the entire launcher database.

The confirmation flag constant is `CONFIRM_YES` (`--yes`) in
`src/cli/parse.rs`.

## What to watch when changing the CLI

- `src/cli/mod.rs` - `is_subcommand` (the pre-bootstrap string check) and
  `run_subcommand` must stay in sync with the modules they dispatch to.
- `src/cli/help.rs` - per-command usage text mirrors the real flags; update it
  whenever a command's arguments change.
- `src/cli/parse.rs` - shared flag parsing, format parsing, and the exit-code
  helpers; keep the `0` / `1` / `2` convention consistent.
- `src/cli/completions.rs` - the static command tree (`TOP_LEVEL`, `*_SUB`
  arrays) must list every command and subcommand, or completion drifts from
  reality.

Relevant tests: the smoke suite exercises `--help` and dry-run; unit tests live
alongside the CLI modules (for example `completions.rs` cache round-trip and
`audit.rs` filter validation).
