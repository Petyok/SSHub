# Session logging

Session logging is opt-in. When enabled, SSHub writes a plain-text transcript of the embedded PTY session to disk as bytes are received.

## Configuration

Two levels:

1. **Global** — `[session_logging]` in `config.toml`:
   - `enabled` — master switch (default false).
   - `max_file_bytes` — rotation size (default 10 MB).
   - `retention_files` — number of segments to keep per host (default 50).
2. **Per-host override** — host form offers `inherit` / `on` / `off`.
   - `inherit` follows the global setting.
   - `on` / `off` override it.

`src/session_log.rs::effective_enabled()` resolves the final value at connect time.

## Storage layout

Logs live under `<data_dir>/logs/<host-dir>/`:

- Managed hosts use `{sanitized-name}-{id}` to avoid collisions between names that sanitize to the same string.
- Pure `~/.ssh/config` aliases without a launcher row use only the sanitized name, so aliases that collide after sanitization may share a directory.

Each segment file name contains timestamp, pid, open counter, and optional serial: `{secs}-{pid}-{open_id}[-{serial}].log`. Rolling to a new segment happens when `max_file_bytes` is exceeded.

## Lifecycle

`src/session_log.rs::SessionLogWriter`:

- `open()` creates the host directory and current segment file.
- `write()` appends PTY bytes, rotating to a new segment when size cap is reached.
- `flush()` / Drop close the writer and clean up retention.

Important rules from recent work:

- The writer opens **only after** the PTY spawn succeeds; failed connections do not create empty logs.
- The writer is attached to the `Session` with `set_log()`.
- File descriptors must be closed cleanly on session end to avoid leaks; see commits around `e03544f` and `2278c22`.

## Security warning

Logs capture **everything echoed to the terminal**, including passwords or secrets if they are rendered on screen. The README and in-app hint call this out explicitly.

## Audit integration

When a logged connect succeeds, the audit event stores the log directory in `log_path` and may also mention it in `note`. The audit tab renders the combined text on the detail line above the table (`src/tui/screens/audit.rs::audit_note`). See `src/app/connect.rs` for the wiring.

## Schema

Session logging required database schema v12 (`src/store/migrate.rs`). The `hosts` table gained `session_logging` to persist the per-host override (encoded `None`/`0`/`1` for `Inherit`/`Off`/`On`).

## What to watch when changing session logging

- `src/session_log.rs` — writer lifecycle, rotation, retention, and directory sanitation are correctness-critical.
- `src/session/mod.rs` — `Session::set_log()` must receive bytes without blocking the PTY thread.
- `src/app/connect.rs` — only attach the writer after successful spawn; propagate errors as user notices.
- `src/store/migrate.rs` / `src/store/types.rs` / `src/app/host_form.rs` — keep the per-host tri-state roundtrip correct.
- Tests: `src/app/tests/session.rs`, `src/store/*` schema tests, manual check with `SSHUB_DATA_DIR` set to a temp path.
