---
type: Workflow
title: Embedded Sessions & SFTP - PTY sessions, session logging, mosh, and file transfer
description: How SSHub runs ssh and local shells inside the TUI via portable-pty + vt100 (src/session), the session switcher and in-session keys, credential delivery through the SSH_ASKPASS channel, the xterm compatibility layer, OSC 52 relay, mouse rules, opt-in session logging with rotation, mosh transport, and the dual-pane SFTP browser with a second-server mode and staged transfer queue (src/sftp).
resource: src/session/mod.rs
tags: [session, pty, sftp, mosh, workflow, file-transfer]
---

# Embedded Sessions & SFTP

Snippets, ghost-text suggestions, the session log browser and the interactive authentication modal have their own page: [session tools](session-tools.md).

## Embedded SSH sessions (`src/session/`)

Connecting to a host spawns `ssh` (or `mosh`) on a pseudo-TTY; output is parsed through `vt100` and drawn fullscreen by `parser::draw_screen`. Sessions coexist as tabs; detaching returns to the dashboard while ssh keeps running, and the dashboard header shows a session strip.

In-session keys (`App::handle_key_session`, `src/app/session.rs`; all rebindable):

| Key | Action |
|---|---|
| `Ctrl+D` | Detach to the dashboard |
| `Ctrl+T` | New session tab (opens the host picker) |
| `Ctrl+W` | Close the tab |
| `Ctrl+[` / `Ctrl+]` (also `Ctrl+PageUp` / `Ctrl+PageDown`) | Previous / next tab |
| `Alt+S` | Session switcher |
| `Ctrl+Shift+T` | Local shell tab |
| `Ctrl+Shift+F` | Open the SFTP tab for this host |
| `Ctrl+N` | [Snippet picker](session-tools.md#command-snippets) (running sessions only) |
| `Tab` / `Ctrl+F` | Accept a visible [ghost completion](session-tools.md#ghost-text-suggestions); Esc dismisses it |
| `PageUp` / `PageDown` | Local scrollback, half a screen per press. On the alternate screen (vim, tmux, less) they are forwarded to the remote instead |
| `Esc` while connecting | Cancel the connection |
| `Ctrl+O` while connecting | Expand or collapse the ssh debug log on the connect screen |

Every other key snaps the view back to live and is encoded to xterm bytes by `session/keys.rs`; `Ctrl+C` always reaches the remote.

**Session switcher** (`Alt+S`, `src/app/session_picker.rs`, `screens/session_picker.rs`): a type-to-filter list of open sessions matching name, user or address, each with its ordinal, endpoint and a phase badge `conn` / `up` / `exit`. Enter jumps to the session (with the tab slide animation when opened from a session), Esc closes, `Ctrl+Shift+T` closes it and opens a local shell. The same picker, with a host list instead of sessions, serves `Ctrl+T` (new session tab) and the SFTP left-pane server choice. The switcher refuses to open when there are no sessions.

**Dashboard session strip** (`handle_key_background_sessions`): with sessions running, every dashboard tab accepts `Ctrl+Shift+S` (resume the selected session), `Ctrl+[`/`Ctrl+]` (move the strip selection without entering), `Alt+S`, `Ctrl+T`, `Ctrl+W`, `Ctrl+Shift+F`. The footer leads with `resume` (`KeybindsConfig::session_footer_hints`).

**Local shell** (`src/app/local_shell.rs`): `Ctrl+Shift+T` from any dashboard tab, a session, or the switcher opens a tab running `$SHELL` (falling back to `/bin/sh`) with stderr merged onto the PTY so the prompt renders. Detach and close behave like ssh tabs.

### Split view and workspaces (#73)

The session body can be divided into 2 to 4 panes (`src/app/panes.rs`): a layout tree of depth 2 with equal halves, whose leaves hold a `SessionId` (stable for a session's life, unlike its index in `App::sessions`). `App::pane_views` is the one geometry function; the renderer, the mouse, the theme protection pass (`session::render::remote_pty_rects`) and the per-frame resize all read pane rects from it. The focused pane's session is `App::active_session`, so keys, paste, the ghost, snippets and the OSC 52 relay follow focus. Slides do not run while split.

Resize contract: `Session::spawn` and `Session::resize` take the grid size of the pane (callers own the chrome); `resize` is a no-op for an unchanged size and never touches an `Exited` session. Each frame, `sync_panes` reconciles the layout with `active_session`, then every session on screen is resized to its pane; one out of view keeps its last size. New sessions are spawned at the grid of the pane they land in (`App::new_session_grid`). `ParserState::set_size` is vt100's own resize: a row shrink scrolls the top into history so the cursor row survives, a grow pulls history back, and the scroll region and scrollback offset reset; columns are cut, not reflowed.

Workspaces (`src/app/workspaces.rs`, table `workspaces`, schema v20) store the layout tree as JSON with saved hosts by name and local shells per pane; opening one lays the panes out first, then spawns each session into its pane.

Key pieces:

- **`pty.rs`**: `PtyRuntime::spawn(argv, rows, cols, env, siphon_stderr)` over `portable-pty`; reader threads send `PtyEvent::{Bytes, Stderr, SshLog, Exited}` to the [event loop](../architecture/overview.md). For ssh the child's stderr is siphoned through a side FIFO (`StderrFifo`: `mkfifo` 0600 inside a private 0700 random-named temp dir, a failure to create it fails the spawn loudly; opened `O_RDWR | O_NONBLOCK`), and for the `ssh` client ssh's own log (`-E`, with `LogLevel=DEBUG1`) goes to a second FIFO as `PtyEvent::SshLog`, via an `sh -c 'p=$1; shift; exec "$p" -E "$SSHUB_LOG_FIFO" "$@" 2>"$SSHUB_STDERR_FIFO"'` wrapper. Only that log channel is parsed for `Authenticated to …`, host-key and changed-key evidence (`Session::push_ssh_log`), because a server's login banner arrives on stderr and could otherwise pose as ssh output. `TERM=xterm-256color` and `COLORTERM=truecolor` are forced. Writes (keys, pastes, query answers) go through one writer thread in order; paste drops ESC and other control bytes. Drop sends SIGHUP, then SIGTERM, then kills the process group.
- **Connect argv** (`prepare_session_connect_argv`, `src/app/util.rs`): embedded ssh runs with `-t -o StrictHostKeyChecking=ask -o NumberOfPasswordPrompts=3 -o LogLevel=DEBUG1`, so unknown or changed host keys and password prompts surface in the [auth modal](session-tools.md#interactive-authentication-modal) instead of being auto-accepted. `-t` gives a startup command (`remote_command`, or ssh_config `RemoteCommand`) a remote terminal, which ssh requests on its own only for a login shell. A stored startup command (the trailing `-- <cmd>` from `build_ssh_argv`) gets `\nexec "$SHELL" -l` appended (`keep_login_shell`), so it runs at login and the session goes on in the login shell; a command starting with `exec` replaces that shell and ends the session with it. An ssh_config `RemoteCommand` is left to ssh. `sshub connect` gets the same through `prepare_cli_session_argv`; `sshub exec` drops the startup command instead.
- **Credentials** (`askpass.rs`, `askpass_channel.rs`, `interactive_auth.rs`, `hop_auth.rs`): every interactive ssh session (argv[0] `ssh`, not a key push) gets a private Unix-socket askpass channel: `SSH_ASKPASS=<self exe>` plus `SSHUB_ASKPASS_SOCKET` / `SSHUB_ASKPASS_TOKEN` / `SSHUB_ASKPASS_CHANNEL`. `main.rs` calls `maybe_run_askpass()` first, so the re-executed binary forwards the prompt to the TUI. The destination's stored secret answers only a prompt of its own class (never a 2FA code or a host-key question) from the destination ssh (identified by pid, so a ProxyJump hop never receives it); anything else opens the modal. A ProxyJump hop that asks for a password or passphrase is answered once with that hop host's own stored secret, fetched when it asks (`HopCred`, `hop_auth::find_for`): the prompt must read `user@host's password:` / `Enter passphrase for key '<path>':` for a saved hop AND the asking ssh's last argument must be that hop, so one hop's secret never reaches another hop or a prompt text a hostile server chose. Mosh with a stored secret and key-push sessions use a 0600 staged file under `$XDG_RUNTIME_DIR` (`AskpassSecret`, `SSHUB_ASKPASS_FILE`); hop secrets are never written to it, the file holds references into the credential store that the helper resolves when a hop asks. Only when that file path cannot be staged is the stored secret typed into the PTY, at most once and only when the cursor sits right after a password or passphrase prompt (`maybe_send_pending_secret`, `ends_with_prompt`). OpenSSH older than 8.4 ignores `SSH_ASKPASS_REQUIRE` and gets the prompt in the terminal. Details: [private channel](../security/secrets.md#private-channel-srcsessionaskpass_channelrs) and [secrets](../security/secrets.md).
- **`mod.rs`**: `Session` lifecycle `SessionPhase::{Connecting, Running, Exited}`; the connect animation stays up until a stored secret is delivered or `REVEAL_TIMEOUT` (6 s) passes. Past the timeout it gives way only once the host answered: PTY output arrived or ssh authenticated (a startup command may print nothing). `FAILURE_EXPLANATIONS` turns common ssh failures into plain-language reasons on the connect screen. The first `debug1: Server host key:` fingerprint is cached and shown there.
- **`parser.rs`**: `vt100::Parser` with 10 000 rows of scrollback behind `XtermCompat`, which rewrites the xterm sequences `vt100` 0.18 still drops (HPR, VPR, SCOSC/SCORC), caps the REP count, renders DEC Special Graphics line drawing as Unicode, and tracks synchronized output (DEC mode 2026) so the pane keeps showing the last complete frame mid-repaint. It also answers terminal status queries (bounded reply queue) so remote apps do not block on a timeout, and `draw_screen` paints the grid into a ratatui buffer.
- **`render.rs`**: header (tab strip and hints from `session_header_hints`), body, footer, copy toast, ghost text.
- **OSC 52 relay**: applications inside the PTY may request a clipboard write; only the visible session relays non-empty payloads (`App::relay_visible_session_clipboard`). Payloads are capped at 64 KiB and at most 8 writes are queued; background sessions, the dashboard, SFTP and tunnels discard requests immediately. Clipboard reads (`ESC]52;c;?BEL`) stay unanswered. Disable with `[clipboard] relay_from_pty = false`; the raw sequence can still land in enabled session logs.

### Mouse and selection in a session

`handle_mouse_session` with `session::keys::selects_locally`: when the remote has not enabled mouse reporting, the mouse is local (drag selects, release copies through OSC 52, the wheel scrolls scrollback by 3 lines, dragging past the edge autoscrolls). When reporting is on, the mouse is forwarded only if the remote is also on the alternate screen; `Shift` inverts the decision in either direction. On the alternate screen without reporting, a wheel notch becomes arrow keys for the app (xterm alternateScroll); `Shift`+wheel scrolls local scrollback instead. Forwarded events outside the grid cells (header, footer) are dropped.

## Session logging (`src/session_log.rs`)

Opt-in capture of PTY output to plain-text files under `<data dir>/logs/<host-dir>/`, which is `~/.local/share/sshub/profiles/<name>/logs/` in profile mode and `~/.local/share/sshub/logs/` (or the `SSHUB_DATA_DIR` override) in compatibility mode. Enabled globally in Settings ("Session logging") or per host (`inherit` / `on` / `off`, host form "Session log" or `sshub host add --session-log`).

- `SessionLogWriter` is append-only; rotates at `[session_logging].max_file_bytes` (default 10 MiB) with `{secs}-{pid}-{id}[-{serial}].log` names and prunes by mtime to `retention_files` (default 50) per host directory; dirs 0700, files 0600 via `secure_fs`.
- Managed hosts log to `{sanitized-name}-{id}` directories (`host_log_dir_name`); ssh_config aliases without a launcher row use the sanitized name alone, so colliding names can share a directory.
- Session-connect audit events record the log path in `auth_events.log_path`.
- **Security:** logs capture everything echoed to the terminal, including typed passwords. The help overlay carries this warning; see [secrets](../security/secrets.md).
- `wrap_script_command()` wraps `sshub host connect` in `script(1)` when logging applies outside the embedded PTY; `sshub exec` never writes a transcript.
- Browse, search and bookmark logs in the TUI with `Shift+L`: [session log browser](session-tools.md#session-log-browser).

## Transport: ssh or mosh (`src/session_transport.rs`)

Per-host `SessionTransport::{Ssh, Mosh}`, set in the host form, by `--transport ssh|mosh`, or inherited from a group default (`resolved_session_argv`). Embedded sessions use `mosh` when selected (`build_mosh_argv` in `src/ssh/host.rs`; `StrictHostKeyChecking=accept-new` is injected into the inner ssh only when a secret is staged). Tunnels, SFTP, broadcast and `sshub exec` are ssh-only; `exec` refuses mosh hosts.

## SFTP (`src/sftp/`)

The SFTP tab first shows a host picker (same group tree and fold keys as the hosts tab; `/` searches, Enter connects). The browser is dual-pane: **left = local, right = remote** (`Tab` switches focus). The local pane shows `$HOME` as `~`; `..` is a selectable parent row in both panes.

Browser keys (`handle_key_sftp_browser`, `src/app/sftp.rs`):

| Key | Action |
|---|---|
| `Enter` / `Backspace` | Enter directory / go to parent |
| `Left` / `Right` | Stage the focused selection toward the left (download) or right (upload) pane; files or whole folders, recursive |
| `u` | Remove the most recently staged item |
| `c` | Run the queue |
| `d` / `n` / `R` / `M` | Delete (confirmed), new folder, rename/move, chmod (octal, seeded with the current mode); frozen while a queue runs |
| `r` | Re-list both panes |
| `.` | Show or hide dotfiles (persisted in `ui_state` as `sftp_show_hidden`); a dot-prefixed search also reveals matches |
| `/` | Filter the focused pane |
| `s` | Open an SSH session to the same host (SFTP stays open) |
| `o` / `O` | Point the left pane at a second server (session picker) / back to the local filesystem |
| `Esc` | Disconnect back to the host picker |

The queue stays open while a run is in flight: newly staged items join the next pass. While a queue runs, the remote pane cannot navigate (its worker is busy) but the local pane can.

**Second-server mode**: after `o`, both panes are remote. Each item is relayed through an owner-only local temp directory (`sshub-relay-*`), one leg at a time: the source worker downloads, then the destination worker uploads, and the item leaves the queue only after the far-end write completes (`sftp_relay_step`).

**Connection target** (`app::util::sftp_ssh_host`, used for the right pane and the second server): libssh2 cannot read `~/.ssh/config`, so a managed host is first resolved through `store.resolve_connection` (group defaults apply; a failure falls back to the stored row). For a managed row with `source = ssh_config`, the alias is then resolved live through the resolver (`ssh -G`), and its user, identity file and certificate file fill only the fields the store leaves empty; SSHub-managed values always win.

An unreachable host fails after an 8 s TCP connect timeout with a modal notice instead of a blank browser.

- **`transport.rs`**: `SftpTransport` trait (test seam) with `Ssh2Transport` over the `ssh2` crate (libssh2, vendored OpenSSL). TOFU host-key policy mirroring `accept-new`: unknown keys are appended to `~/.ssh/known_hosts` by hand (`known_hosts_entry`) because libssh2's writer would drop unparsable lines; a changed key is a hard "possible MITM" error. Auth order: stored passphrase with the identity file, stored password, else ssh-agent, else an unencrypted identity file.
- **`worker.rs`**: the blocking transport lives on a dedicated thread (`spawn_sftp_worker`) serving `SftpCommand::{ListDir, RunQueue, Remove, Mkdir, Rename, Chmod, Cancel}` and emitting `SftpEvent::{Connected, ConnectFailed, DirListing, Progress, TransferDone, QueueDone, OpDone, Error}`. Progress events are throttled to every 64 KiB; recursive transfers pre-plan the tree; symlinks are never descended.
- **`model.rs`**: pure, I/O-free dual-pane state (`SftpState`, `Pane`, `FileEntry`, `QueuedTransfer`, `Direction`, `Side`), easy to unit test.

Limitations: **ProxyJump hosts are refused** with a notice ("SFTP via ProxyJump isn't supported yet"), in the picker and for the second server. SFTP is also exposed headlessly: `sshub sftp ls|get|put|rm|mkdir|rename|chmod` (see [CLI](cli.md)).

## Change guidance

- Session regressions: `src/app/tests/session.rs`, `tests/e2e/connect_managed.rs`, `tests/e2e/session_switcher.rs`, `tests/e2e/local_shell.rs`.
- SFTP logic belongs in `model.rs` (pure) where possible; transport changes go through the `SftpTransport` trait so tests can substitute a fake. See `tests/e2e/quick_connect.rs` and `src/app/tests/sftp.rs`.
