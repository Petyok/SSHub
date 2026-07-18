---
type: Workflow
title: SSH Tunnels — local/remote/dynamic forwards with keep-alive reconnect
description: SSHub defines and manages SSH tunnels (local -L, remote -R, dynamic SOCKS -D), spawns them as monitored child processes with BatchMode/askpass secret staging, and auto-reconnects dropped keep-alive tunnels with exponential backoff configured in config.toml.
resource: src/tunnel/mod.rs
tags: [tunnels, ssh, keep-alive, reconnect, workflow]
---

# SSH Tunnels

Tunnels are defined in the TUI (tab 3) or via the [CLI](cli.md) (`sshub tunnel create/start/stop`), stored in the `tunnels` table of `launcher.db` ([data model](../architecture/data-model.md)), and run as `ssh -N -L|-R|-D …` child processes.

## Spawning (`src/tunnel/spawn.rs`)

- `build_tunnel_argv` maps tunnel type to `ssh -N` with `-L` (local), `-R` (remote), or `-D` (dynamic SOCKS).
- `splice_tunnel_ssh_options` injects:
  - `BatchMode=yes` when no secret is staged — tunnels "must never open /dev/tty … writes over the TUI";
  - `BatchMode=no` + `StrictHostKeyChecking=accept-new` when a secret is staged via `stage_tunnel_askpass` (same re-exec mechanism as [session askpass](../security/secrets.md));
  - `ServerAliveInterval=10` / `ServerAliveCountMax=3` so dead forwards are detected.
- **Detached (CLI) mode**: `sshub tunnel start` without `--foreground` writes PID files under `$SSHUB_DATA_DIR/tunnels/`. The code documents the known races: no locking, liveness via bare `kill(pid, 0)`, and a recycled-PID SIGTERM risk on `stop`. Detached tunnels and TUI-managed tunnels are mutually invisible.

## In-process manager (`src/tunnel/mod.rs`)

`TunnelManager` owns child processes for the running TUI, capturing a stderr tail for diagnostics and checking health with `try_wait`. Tunnels marked **keep alive** (`auto_connect`) start on launch and reconnect when they drop:

- Backoff: exponential with deterministic jitter via `config::tunnel_backoff_delay`, configured in `[tunnel_reconnect]` — `max_attempts` (0 = unlimited), `initial_delay_ms` (1 s), `max_delay_ms` (60 s), `stable_secs` (uptime that resets the attempt counter; a spawn younger than this counts as failed), `jitter_ratio` (0.25).
- Lifecycle surfaces as `ReconnectEvent::{Attempt, Reconnected, GaveUp}`; the `R` overlay (`screens/tunnel_reconnect.rs`) edits delays in seconds.
- A user-initiated stop suppresses reconnect; `x` kills the process.
- The [event loop](../architecture/overview.md) drives this through `tick_tunnels()` each frame.

## Audit (`src/tunnel/audit.rs`)

Reconnect lifecycle (retry / launched / fail) is written to the `auth_events` audit table with `via = tunnel`, so tunnel churn appears in the Audit tab alongside connection events.

## Change guidance

- TUI tunnel behavior: `src/app/tunnels.rs`, tests in `tests/e2e/tunnel_form.rs`.
- If you touch detached mode, address (or at least re-document) the PID-file races above — they're a known sharp edge tracked in the quickstart [backlog](../quickstart.md#backlog).
