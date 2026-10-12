---
type: Workflow
title: SSH Tunnels - local/remote/dynamic forwards with keep-alive reconnect
description: SSHub defines and manages SSH tunnels (local -L, remote -R, dynamic SOCKS -D) from the tunnels tab or `sshub tunnel`, spawns them as monitored ssh children with BatchMode/askpass secret staging and group-inherited connection settings, re-syncs the list from the shared database every two seconds, and auto-reconnects keep-alive tunnels with exponential backoff configured in config.toml.
resource: src/tunnel/mod.rs
tags: [tunnels, ssh, keep-alive, reconnect, workflow]
---

# SSH Tunnels

Tunnels are defined in the TUI (tab 3, key `3`) or via the [CLI](cli.md) (`sshub tunnel create/start/stop`), stored in the `tunnels` table of `launcher.db` ([data model](../architecture/data-model.md)), and run as `ssh -N -L|-R|-D …` child processes.

## Tunnels tab keys

`App::handle_key_tunnels` (`src/app/tunnels.rs`): `a` new tunnel, `e` edit, `d` delete (confirmed; stops the child first), `Enter` start or stop (stopping also cancels a pending reconnect or a gave-up state), `x` kill the process, `R` reconnect settings, `j`/`k` move, `?` help. The form (`AppMode::TunnelForm`) picks the SSH server from a searchable host dropdown (`AppMode::TunnelHostPicker`), then type, local port, remote host/port, label, and the keep-alive flag. Mouse double-click on a row toggles it.

## Spawning (`src/tunnel/spawn.rs`)

- `build_tunnel_argv` builds `ssh -N` with `-L local:host:port`, `-R local:host:port`, or `-D local` (dynamic SOCKS). Port, ProxyJump (`-J`), agent forwarding (`-A`), identity file and username come from the **effective** connection (`store.resolve_connection`: host value, then group chain, then global), so [group defaults](../architecture/data-model.md#group-inherited-connection-defaults-srcstoregroup_defaultsrs) reach tunnels. The target goes through `ssh::safe_ssh_target`.
- `splice_tunnel_ssh_options` inserts `ConnectTimeout=30`, `ServerAliveInterval=10`, `ServerAliveCountMax=3`, `TCPKeepAlive=yes`, and:
  - `BatchMode=yes` when no secret is staged, because a background tunnel must never open `/dev/tty` over the TUI;
  - `BatchMode=no` plus `StrictHostKeyChecking=accept-new` when a stored secret is staged via `stage_tunnel_askpass` (file-based askpass, see [secrets](../security/secrets.md)).
- **Detached (CLI) mode**: `sshub tunnel start` without `--foreground` writes PID files under `<data dir>/tunnels/` (0700, `ensure_tunnel_pid_dir`) and marks the askpass file read-once. The code documents the known races: best-effort stale-PID and port checks with no locking, liveness via bare `kill(pid, 0)`, and a recycled-PID risk on `stop` (SIGTERM). Detached tunnels and TUI-managed tunnels are mutually invisible.

## In-process manager (`src/tunnel/mod.rs`)

`TunnelManager` owns the TUI's tunnel children, keeps a stderr tail for diagnostics, and checks health with `try_wait`. Tunnels marked **keep alive** (`auto_connect`) start when the TUI starts (`bootstrap_auto_connect_tunnels`) and reconnect when they drop:

- Backoff: exponential with deterministic jitter (`config::tunnel_backoff_delay`), configured in `[tunnel_reconnect]`: `max_attempts` (default 12, `0` = unlimited), `initial_delay_ms` (1000), `max_delay_ms` (60000), `stable_secs` (5; a child must survive this long before the attempt counter resets, a shorter run counts as a failure), `jitter_ratio` (0.25).
- Lifecycle events: `ReconnectEvent::{Attempt, Reconnected, GaveUp}`.
- The `R` overlay (`AppMode::TunnelReconnectSettings`, `src/tui/screens/tunnel_reconnect.rs`) edits Max attempts, Initial delay, Max delay, Stable time and Jitter: `j`/`k` select, `+`/`=`/Right increase, `-`/`_`/Left decrease, `*` resets the field; each change saves `config.toml` immediately.
- A user-initiated stop suppresses reconnect until the tunnel is started again.
- `App::tick_tunnels` runs every event-loop tick: one-time auto-start, then `sync_tunnels_from_store` at most every 2 seconds, `check_health`, and `tick_tunnel_reconnect`.

**Cross-window refresh**: the tunnel list is re-read from the shared store every 2 seconds, so rows added, edited or deleted by another SSHub window or by `sshub tunnel` appear without a restart. The selection is re-anchored by tunnel id, and running children are never killed because their row id vanished (an edit elsewhere is a delete plus an insert).

## Audit (`src/tunnel/audit.rs`)

`log_tunnel_reconnect_events` writes `retry` (attempt), `launched` (reconnected) and `fail` (gave up) rows to `auth_events` with `via = tunnel`; manual start (`launched`), stop (`ok`) and auto-start failures are logged from `src/app/tunnels.rs`. Tunnel churn therefore shows in the Audit tab and `sshub audit list --via tunnel`.

## Change guidance

- TUI tunnel behavior: `src/app/tunnels.rs`; tests in `tests/e2e/tunnel_form.rs`.
- If you touch detached mode, address (or re-document) the PID-file races above.
- Cross-profile copy/move of a tunnel: `sshub tunnel transfer` or the profile manager, see [profiles & themes](profiles-themes.md#moving-data-between-profiles).
