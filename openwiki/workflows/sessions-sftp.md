---
type: Workflow
title: Embedded Sessions & SFTP — PTY sessions, session logging, mosh, and file transfer
description: How SSHub runs SSH inside the TUI via portable-pty + vt100 (src/session), stages credentials through SSH_ASKPASS re-exec, optionally logs session output with rotation, supports mosh transport, and provides a dual-pane SFTP browser over libssh2 with a staged transfer queue (src/sftp).
resource: src/session/mod.rs
tags: [session, pty, sftp, mosh, workflow, file-transfer]
---

# Embedded Sessions & SFTP

## Embedded SSH sessions (`src/session/`)

Connecting to a host spawns `ssh` (or `mosh`) on a pseudo-TTY; output is parsed through `vt100` and rendered fullscreen in ratatui via `tui-term`. Multiple sessions coexist as background tabs — `Ctrl+D` detaches to the dashboard while SSH keeps running.

Key pieces:

- **`pty.rs`** — `PtyRuntime::spawn(argv, rows, cols, env)` over `portable-pty`; a reader thread streams `PtyEvent::{Bytes, Stderr, Exited}` over mpsc to the [event loop](../architecture/overview.md). The child's stderr is siphoned through a side FIFO (`StderrFifo`, `mkfifo` 0600, opened `O_RDWR|O_NONBLOCK` so empty reads yield EAGAIN instead of EOF) via an `sh -c 'exec "$@" 2>"$SSHUB_STDERR_FIFO"'` wrapper — this keeps `ssh -v` noise off the render grid. `TERM` is forced to `xterm-256color` because kitty's terminfo breaks remotes. Drop escalates SIGHUP → SIGTERM → kill on the process group.
- **`askpass.rs`** — credentials reach ssh without PTY typing: the secret is written to a 0600 file in `$XDG_RUNTIME_DIR`, the child gets `SSH_ASKPASS=<self exe>`, `SSH_ASKPASS_REQUIRE=force`, `SSHUB_ASKPASS_FILE=<path>`, and `main.rs` calls `maybe_run_askpass()` first so the re-executed binary prints the secret and exits. Details and caveats: [secrets](../security/secrets.md).
- **`mod.rs`** — `Session` lifecycle `SessionPhase::{Connecting, Running, Exited}`. `PendingSecret::{Password, Passphrase}` is auto-typed at most once (prompt matched via a cursor-parked needle scan, not MOTD text) so wrong-password retries don't loop; askpass is preferred, PTY typing is the fallback.
- **`parser.rs`** — `vt100::Parser` with 10k scrollback. A `clear_buffer()` helper exists to wipe the buffer post-auth but is currently dead code (never called); `-v` handshake noise is siphoned off via the stderr FIFO into `debug_log`, not the parser buffer. **`keys.rs`** encodes crossterm keys to xterm bytes; **`render.rs`** draws header/body/footer and the copy toast.

## Session logging (`src/session_log.rs`)

Opt-in capture of PTY output to plain-text logs under `~/.local/share/sshub/logs/<host-dir>/`. Enabled globally in Settings or per host (`inherit` / `on` / `off` — stored on `ManagedHost` or `HostMetadata`, see [data model](../architecture/data-model.md)).

- `SessionLogWriter` is append-only; rotates at `[session_logging].max_file_bytes` (default 10 MiB) with `{secs}-{pid}-{n}[-serial].log` names, prunes by mtime to `retention_files` (default 50) per host; dirs 0700 / files 0600 via `secure_fs`.
- Managed hosts log to `{name}-{id}` directories; pure ssh_config aliases without a launcher row may share a directory when sanitized names collide.
- Session-connect audit events record the log path in `auth_events.log_path`.
- **Security:** logs capture everything echoed to the terminal, including typed passwords. This warning is in the [TUI help screen](tui.md) and the [secrets page](../security/secrets.md).
- `wrap_script_command()` wraps external launches in `script(1)` (Linux/macOS) when logging applies outside the embedded PTY.

## Transport: ssh or mosh (`src/session_transport.rs`)

Per-host `SessionTransport::{Ssh, Mosh}` selected in the host form. Embedded sessions use `mosh` when selected (`build_mosh_argv` in `src/ssh/host.rs`, with an injected ssh accept-new option); **tunnels and SFTP are always ssh-only**.

## SFTP (`src/sftp/`)

A dual-pane (remote/local) file browser with a staged transfer queue: navigate both sides, queue uploads/downloads (files or whole folders, recursive), and run the queue with a progress bar. In-place ops: delete `d`, new folder `n`, rename/move `R`, chmod `M` (octal).

- **`transport.rs`** — `SftpTransport` trait (test seam) with `Ssh2Transport` over the `ssh2` crate (libssh2, vendored OpenSSL so `cargo install` needs no system libssh2). TOFU host-key policy mirroring `accept-new`: unknown keys are appended to `~/.ssh/known_hosts` **manually** because libssh2's `write_file` would drop unparsable lines; a *changed* key is a hard MITM error. Auth order: stored passphrase → `userauth_pubkey_file`; password → `userauth_password`; else agent / unencrypted key.
- **`worker.rs`** — the blocking transport lives on a dedicated thread (`spawn_sftp_worker`) serving `SftpCommand::{ListDir, RunQueue, Remove, Mkdir, Rename, Chmod, Cancel}` and emitting `SftpEvent::{Connected, ConnectFailed, DirListing, Progress, TransferDone, QueueDone, OpDone, Error}`. Progress events are throttled at 64 KiB; recursive transfers pre-plan the tree (still polling Cancel); symlinks are never descended.
- **`model.rs`** — pure, I/O-free dual-pane state (`Pane`, `FileEntry`, `QueuedTransfer`, `Direction`) — easy to unit test.

Limitations: **ProxyJump hosts are refused** (libssh2 transport can't chain; `src/app/sftp.rs`). SFTP is also exposed headlessly — `sshub sftp ls|get|put|rm|mkdir|rename|chmod` (see [CLI](cli.md)).

## Change guidance

- Session regressions: `src/app/tests/session.rs` + `tests/e2e/connect_managed.rs`.
- SFTP logic changes belong in `model.rs` (pure) where possible; transport changes go through the `SftpTransport` trait so tests can substitute a fake. See `tests/e2e/quick_connect.rs` and `src/app/tests/sftp.rs`.
