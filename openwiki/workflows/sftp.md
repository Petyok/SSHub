# SFTP file transfer

SSHub has a built-in dual-pane SFTP browser. It runs `libssh2` on a dedicated worker thread so the synchronous TUI event loop never blocks on I/O.

## Architecture

Three layers:

1. **Model** — `src/sftp/model.rs` holds the pure UI state: local/remote directory listings, staged queue, selection, filter, progress.
2. **Transport** — `src/sftp/transport.rs` wraps `ssh2::Sftp` and performs synchronous file ops.
3. **Worker** — `src/sftp/worker.rs` owns the SSH session + SFTP channel on a background thread and turns `SftpCommand` messages into `SftpEvent` replies.

`src/app/sftp.rs` glues them together: it processes `SftpEvent`s each tick and mutates `SftpState`.

## Opening SFTP

- From the dashboard you can open the SFTP tab for the current host (`Ctrl+Shift+F` by default).
- The SFTP tab has its own host picker if no live session exists.
- `sftp_host` records the connected host name so the browser can round-trip back to an SSH session.

## Browser UI

The SFTP body is split into two panes:

- Left: local filesystem.
- Right: remote filesystem.

Key actions:

- Navigate with arrow keys / `j`/`k` / `Enter` to enter a directory.
- `/` filters files in the active pane.
- `Space` / `p` stages the selected item for transfer.
- `P` runs the staged queue.
- `s` jumps from SFTP back to the host's SSH session (round trip).
- `d` deletes a file or recursively deletes a directory.
- `n` creates a new directory.
- `R` renames/moves an item.
- `M` changes permissions with octal input.

The queue shows total bytes and a progress bar during transfers.

## Transfer queue

- Items are staged first, then executed one at a time by the worker.
- Recursive directory transfers walk the source tree and stage each file.
- Downloads use atomic temp-file + rename.
- Symlinks to files are followed and transfer with the real file's size; symlinks to directories or broken links are skipped so progress does not overshoot.

## Worker thread

`spawn_sftp_worker()` in `src/sftp/worker.rs`:

- Connects via `ssh2` (trust-on-first-use host-key verification).
- Listens on `SftpCommand` and executes commands in blocking calls.
- Sends `SftpEvent` results back over `sftp_rx`.
- Commands include `ListLocal`, `ListRemote`, `Stage`, `RunQueue`, `Mkdir`, `Rename`, `Remove`, `Chmod`, etc.

The main event loop drains all pending `SftpEvent`s each tick, so UI stays responsive even while large transfers run.

## Headless CLI

One-shot SFTP operations run without the TUI:
`sshub sftp ls|get|put|rm|mkdir|rename|chmod <host> ...`. Each subcommand drives
the same background worker synchronously and exits. The direct libssh2 transport
cannot chain a jump, so ProxyJump hosts are rejected up front. A local directory
passed to `put` transfers recursively; `get` needs `--recursive` to walk a
remote tree, and `sftp rm` requires `--yes`. See [cli.md](cli.md) for the full
command reference.

## What to watch when changing SFTP

- `src/sftp/transport.rs` — any new remote operation must have a matching command/event and handle errors gracefully.
- `src/sftp/worker.rs` — ensure the worker terminates cleanly when the tab closes or the app quits.
- `src/sftp/model.rs` — keep queue state consistent; prevent duplicate staging and re-entrancy while a queue is running.
- `src/app/sftp.rs` — keyboard dispatch and event handling must not block on missing fields.

Relevant tests: `src/app/tests/sftp.rs`, `tests/e2e/mod.rs` SFTP scenarios.
