# Connecting to hosts

SSHub connects to hosts using an **embedded PTY** inside the ratatui frame. Sessions can remain active after you detach (`Ctrl+D` by default), and multiple sessions are managed as tabs.

## Connection flow

1. User selects a host in the dashboard and presses the connect key (`Enter` by default).
2. `app/connect.rs::connect_host_entry()` resolves:
   - the stored secret (password or key passphrase) from the keyring,
   - the SSH argv via `src/ssh/host.rs::build_ssh_argv`,
   - host metadata (user, address, port, identity, proxy jump) for the session header.
3. A `SessionConfig` is built with `argv`, `display_name`, `meta`, and the optional `PendingSecret`.
4. `Session::spawn()` in `src/session/mod.rs` opens a PTY and runs `ssh` inside it.
5. The app switches to `AppMode::Connecting` and shows the connect screen, which streams the real `ssh -v` handshake.
6. Once bytes are received, the session enters the `Running` phase and the app switches to `AppMode::Session`.

The connect screen shows real `ssh` output rather than a scripted animation. This is done by injecting `-v` into the SSH argv.

## Secrets & askpass

`src/session/askpass.rs` provides an `SSH_ASKPASS` helper. The binary checks `maybe_run_askpass()` at the top of `main.rs`; when ssh re-executes the binary as the askpass helper, it emits the staged secret and exits before touching the TUI.

Secret resolution rules (`src/app/connect.rs`):

- A host-level stored credential is sent at `password:`-style prompts.
- An identity-level stored credential is sent at `Enter passphrase for …` prompts.
- If a stored secret is present, SSH is told `StrictHostKeyChecking=accept-new` so fresh host keys do not deadlock askpass. Changed host keys are still refused.
- When no credential is stored, ssh runs with `BatchMode=yes` inside the PTY so prompts do not steal the TUI.

For tunnels, a similar askpass flow is used (`src/tunnel.rs::stage_tunnel_askpass`), but tunnels run with `Stdio::null` stdin and no PTY, so askpass is the only way to deliver secrets without blocking the dashboard.

## Embedded session runtime

`src/session/mod.rs` defines `SessionPhase`:

- `Connecting` — child spawned, waiting for first bytes.
- `Running` — live PTY.
- `Exited { status, at }` — child exited; any key returns to dashboard.

`src/session/pty.rs` runs the PTY:

- One thread pumps bytes from the PTY master into a `PtyEvent` channel.
- User keystrokes from the main event loop are written to the PTY slave.
- Resizes propagate from ratatui to the PTY.

`src/session/parser.rs` feeds raw bytes into the `vt100` emulator and exposes a `ParserState` snapshot for rendering.

`src/session/render.rs` draws the terminal grid, header, status bar, and exit screen. It also handles mouse text selection.

## Session tabs

Multiple sessions can coexist:

- `Ctrl+T` opens the session host picker for a new tab.
- `Ctrl+[/]` switch previous/next tab.
- `Ctrl+W` closes the current tab (kills the SSH child).
- `Ctrl+D` detaches from the current tab and returns to the dashboard; the SSH child keeps running.
- `Ctrl+Shift+S` focuses an existing session tab from the dashboard.
- The dashboard header shows a strip of open session tabs.

Session keybindings are now rendered from the configured keymap (`config.toml` or keybind editor), not hardcoded defaults. See `src/session/render.rs` and `src/tui/widgets/footer.rs`.

## Session logging

Session logging is opt-in and was added in the Unreleased development cycle (schema v12).

- Global toggle in Settings (`Ctrl+H`) under `session_logging.enabled`.
- Per-host tri-state override in the host form: `inherit` (default), `on`, `off`.
- When active, PTY output is appended to rotating plaintext files under `~/.local/share/sshub/logs/<host-dir>/`.
- `SessionLogWriter` (`src/session_log.rs`) handles rotation by size and per-host retention.
- The audit tab shows the log directory path for each connect event.

**Security warning:** logs capture everything the terminal prints, including passwords if they appear on screen. The feature is off by default and the README and in-app help call this out.

## What to watch when changing connection code

- `src/app/connect.rs` — secret resolution, argv building, session logging setup, auth event recording.
- `src/session/mod.rs` — `Session::spawn`, `Session::tick`, `Session::resize`.
- `src/session/pty.rs` — thread safety of PTY reads/writes.
- `src/session/render.rs` — selection bounds, header/footer hints.
- `src/session_log.rs` — file lifecycle, rotation, retention; make sure `SessionLogWriter` is dropped or reset between connections.
- `src/ssh/agent.rs` and `src/credentials.rs` — changes to how agent / keyring state is read.

Relevant tests: `src/app/tests/session.rs`, `tests/e2e/mod.rs` connect scenarios.
