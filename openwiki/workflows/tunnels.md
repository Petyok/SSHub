# Tunnels

SSHub manages SSH tunnels as child `ssh -N` processes. A tunnel entry stores host binding, port spec, and type; `TunnelManager` spawns, monitors, stops, and reconnects those children.

## Tunnel data model

`src/store/types.rs` defines `Tunnel`:

- `id`, `host_id` (FK to `ManagedHost`)
- `tunnel_type`: `Local`, `Remote`, or `Dynamic`
- `local_port`, `remote_host`, `remote_port` (remote_host/port unused for Dynamic)
- `label`
- `auto_connect` — reused as the **Keep alive** toggle

The `auto_connect` column historically meant "start on app launch". It has been broadened to mean "keep this tunnel alive": start on launch **and** reconnect after unexpected exit.

## Tunnel lifecycle

`src/tunnel.rs::TunnelManager` owns:

- `processes: HashMap<i64, TunnelProcess>` — currently running children.
- `reconnect: HashMap<i64, ReconnectState>` — retry timers for keep-alive tunnels.
- `user_stopped: HashSet<i64>` — tunnels explicitly stopped by the user (no auto reconnect).
- `terminal_errors: HashMap<i64, String>` — last error for non-keep-alive tunnels.

Each `TunnelProcess` tracks:

- `started_at` — used to judge whether the tunnel survived long enough to count as stable.
- `proving: bool` — true until the child has been up for `TunnelReconnectConfig::stable_secs`.
- `stderr_tail: Arc<Mutex<String>>` — last ~4 KB of stderr for diagnostics.

### Actions

- **Start** — `TunnelManager::start()` builds `ssh -N ... -L/-R/-D <spec>` with `ServerAliveInterval`, `ServerAliveCountMax`, and `TCPKeepAlive` options so dead paths tear down quickly. It stops an existing process first if one exists.
- **Stop** — `TunnelManager::stop()` kills the child and inserts the id into `user_stopped` so keep-alive does not immediately retry.
- **Toggle** — `TunnelManager::toggle()` flips between start/stop. A stop on toggle also stops the proving child and cancels any pending reconnect.
- **Kill** — `TunnelManager::kill()` is the hard emergency stop mapped to `x` in the tunnels tab.
- **Reconnect settings** — global backoff knobs edited in-app with `R`; stored under `[tunnel_reconnect]` in `config.toml`.

The app polls `TunnelManager` every event-loop tick. `poll()` checks child exit status, schedules reconnects, and emits `ReconnectEvent::Attempt / Reconnected / GaveUp`.

## Keep-alive reconnect semantics

Keep-alive behavior is controlled by `TunnelReconnectConfig` in `src/config.rs`:

- `max_attempts` — 0 means unlimited.
- `initial_delay_ms`, `max_delay_ms` — exponential backoff bounds.
- `jitter_ratio` — random spread around each delay (~default 25%).
- `stable_secs` — child must stay up this long before a spawn counts as successful (default 5s).

When a child exits:

1. If uptime >= `stable_secs`, the attempt counter resets.
2. Otherwise the attempt counter increments and the next retry is scheduled.
3. If `max_attempts` is exceeded, the tunnel enters `GaveUp` state.
4. Manual stop/kill/toggle marks the tunnel as user-stopped and disables retries.

Recent fixes (commits around `839eb9c` through `3c566a9`) tightened the proving logic so a freshly started child is treated as active before it becomes stable (`proving = true`), and so manual stop correctly cancels retries.

## UI / status rendering

The Tunnels tab (`src/tui/screens/tunnels.rs`) shows each tunnel's:

- type + port spec,
- status label: `up`, `down`, `starting`, `reconnecting`, `gave up`, `error`,
- attempt counter while reconnecting,
- last error snippet.

Footer hints and the global overlay `src/tui/screens/tunnel_reconnect.rs` expose the five tunable backoff fields.

## Auth for tunnels

Tunnels run in the background without a terminal. The askpass dance is identical in spirit to sessions (`src/session/askpass.rs`) but staged from `src/tunnel.rs::stage_tunnel_askpass`:

- If a stored credential exists, `SSH_ASKPASS` is wired up.
- If no credential exists, `BatchMode=yes` is set so the tunnel fails fast with an error instead of prompting on `/dev/tty`.

## What to watch when changing tunnels

- `src/tunnel.rs` — any change to child lifecycle, reconnect math, or askpass is high-risk for orphaned processes or port leaks.
- `src/app/tunnels.rs` — UI action dispatch must keep `user_stopped` / `proving` / `reconnect` state consistent.
- `src/store/tunnels.rs` — SQL for `auto_connect` roundtrip.
- `src/config.rs` — `TunnelReconnectConfig` fields and default bounds.
- `src/tui/screens/tunnels.rs`, `tunnel_reconnect.rs` — status labels and settings UI must stay in sync with `TunnelStatus` / `ReconnectEvent`.

Relevant tests: `src/app/tests/mod.rs` tunnel helpers, `src/store/tunnels.rs` auto_connect tests, manual `just test` to catch clippy/rustfmt regressions.
