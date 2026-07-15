# Operations & runbook

Day-to-day operations for SSHub users and operators.

**Contributor workflow** (issue → PR → merge, GitHub comment rules): [docs/implementation-flow.md](../../docs/implementation-flow.md) on `development`.

## Data paths

| Resource      | Default path                                              |
|---------------|-----------------------------------------------------------|
| Config        | `~/.config/sshub/config.toml`                             |
| Database      | `~/.local/share/sshub/launcher.db` (+ `-wal` / `-shm`)      |
| Session logs  | `~/.local/share/sshub/logs/<host-dir>/`                   |
| SSH config    | `~/.ssh/config`                                           |

Override environment variables:

- `SSHUB_CONFIG_DIR` (fallback: `SSH_LAUNCHER_CONFIG_DIR`)
- `SSHUB_DATA_DIR` (fallback: `SSH_LAUNCHER_DATA_DIR`)
- `SSHUB_SSH_CONFIG` (fallback: `SSH_LAUNCHER_SSH_CONFIG`)

`src/config.rs` resolves these; `src/secure_fs.rs` restricts DB directory permissions.

## Purging the database

To wipe the launcher database (managed hosts, groups, identities, tunnels, audit log):

```bash
sshub db purge --yes-i-am-stupid
```

This leaves `~/.ssh/config` and its imported hosts untouched. Orphaned keyring entries are not removed because they are stored outside the database.

## Credentials

Passwords and key passphrases are stored in the OS keyring via `keyring`:

- Secret Service / D-Bus on Linux (`libdbus-1-dev` required at build time).
- Apple native on macOS.
- Windows native on Windows.

At runtime a keyring provider must be running and unlocked; otherwise SSHub warns and ssh falls back to prompting.

The database only stores a `has_password` boolean flag; the actual secret is never in SQLite.

## Session logging

Opt-in transcript logging writes PTY output to plaintext files under `~/.local/share/sshub/logs/<host-dir>/`.

- Managed hosts: directory is `{sanitized-name}-{id}`.
- Pure ssh-config aliases: directory is `{sanitized-name}`; collisions may share a directory.

Global settings in `config.toml`:

```toml
[session_logging]
enabled = false
max_file_bytes = 10485760        # rotate when a segment reaches this size
retention_files = 50             # keep at most this many segment files per host
```

Per-host override in the host form: `inherit` / `on` / `off`.

**Security warning:** logs capture every byte echoed to the terminal, including passwords if they appear on screen. Keep log directories owner-only.

## Diagnostics

### App won't start

- Check that `ssh` is on `PATH`.
- Check D-Bus/libdbus availability on Linux build.
- Review `config.toml`; a malformed TOML will fail early in `src/config.rs`.

### TUI looks wrong / transparent background

- Toggle `Opaque background` in Settings (`Ctrl+H`) for transparent terminals.
- Disable OS logos or startup animation if rendering glitches occur.

### Keeps asking for password / key not loaded

- Confirm the keyring is unlocked.
- Confirm the identity key file path is correct and `ssh-agent` integration is working.
- Check `AgentInfo` in the Identities tab.

### Tunnel flaps / won't stay up

- Check `[tunnel_reconnect]` backoff settings.
- Inspect the tunnel's stderr snippet in the Tunnels tab.
- If no keyring credential is stored, tunnels run in `BatchMode=yes` and fail fast rather than prompt.

### Performance / file watcher issues

- The watcher monitors the parent directory of `~/.ssh/config` so rename-based saves trigger reloads.
- On macOS FSEvents can deliver bursts late; the watcher uses a 300 ms debounce (`WATCHER_DEBOUNCE`).

## CLI flags

```bash
sshub --help       # usage
sshub --version    # version
sshub --dry-run    # exit immediately
sshub db purge --yes-i-am-stupid
```

## Log files

SSHub does not write a centralized application log. Diagnostics come from:

- The SSH debug panel in the dashboard (ssh `ssh -v` output per connection).
- Tunnel stderr tails (shown in the Tunnels tab).
- Session transcript files (when session logging is on).

## OpenWiki automation

`.github/workflows/openwiki-update.yml` runs daily (and on `workflow_dispatch`) to refresh `openwiki/` via `openwiki code --update` and open a PR on branch `openwiki/update`.

Required repository secrets:

- `OPENROUTER_API_KEY` — LLM provider for OpenWiki updates.
- `LANGSMITH_API_KEY` — optional; enables LangSmith tracing when set.

Manual edits should be reviewed against the codebase; agent output can drift on API names, schema columns, and event-loop details.
