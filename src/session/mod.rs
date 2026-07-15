//! Embedded PTY-backed SSH session.
//!
//! Replaces the external kitty/ghostty launcher. When a host is connected,
//! sshub spawns ssh on a pseudo-TTY, parses output through a VT100 emulator,
//! and renders the terminal grid fullscreen inside the existing ratatui app.

pub mod askpass;
pub mod keys;
pub mod parser;
pub mod pty;
pub mod render;

use std::time::{Duration, Instant};

use anyhow::Result;

pub use parser::ParserState;
pub use pty::{PtyEvent, PtyRuntime};

/// Lifecycle of an embedded SSH session.
#[derive(Debug)]
pub enum SessionPhase {
    /// Child spawned, no bytes received yet.
    Connecting { started_at: Instant },
    /// First bytes received; live PTY.
    Running { started_at: Instant },
    /// Child exited. Screen frozen; any key returns to dashboard.
    Exited { status: String, at: Instant },
}

impl SessionPhase {
    pub fn is_terminal(&self) -> bool {
        matches!(self, SessionPhase::Exited { .. })
    }
}

/// Configuration for spawning the embedded session.
#[derive(Clone)]
pub struct SessionConfig {
    /// Full argv. argv\[0\] is the program (typically `ssh`).
    pub argv: Vec<String>,
    /// Display name shown in the header (host alias or label).
    pub display_name: String,
    /// Resolved host metadata used by the header + connect animation.
    pub meta: SessionMeta,
    /// One-shot secret typed into the PTY when ssh prompts. Cleared after
    /// the first send. `None` when no credential is stored or the host
    /// uses an unlocked key / agent.
    pub pending_secret: Option<PendingSecret>,
}

/// Auto-respond once to either a password or passphrase prompt.
#[derive(Debug, Clone)]
pub enum PendingSecret {
    /// Sent on `password:`-style prompts only.
    Password(String),
    /// Sent on `Enter passphrase for …` prompts only.
    Passphrase(String),
}

impl PendingSecret {
    pub fn value(&self) -> &str {
        match self {
            PendingSecret::Password(s) | PendingSecret::Passphrase(s) => s.as_str(),
        }
    }
}

/// Host metadata captured at connect time. Used by the header bar and the
/// scripted ConnectScreen.
#[derive(Debug, Clone, Default)]
pub struct SessionMeta {
    /// Remote username, if known.
    pub user: Option<String>,
    /// Hostname or IP we're trying to reach (post-resolve).
    pub address: Option<String>,
    /// Port (defaults to 22 if unknown).
    pub port: Option<u16>,
    /// Path to the private key, if one is bound to this host.
    pub identity: Option<String>,
    /// Jump host fqdn, if proxy_jump is configured.
    pub proxy_jump: Option<String>,
    /// Launcher DB row id, if this is a managed host. Lets the header look
    /// up active tunnels.
    pub host_id: Option<i64>,
}

/// One active embedded session.
/// In-app text selection over the PTY grid. Rows are in visible-viewport
/// coordinates (row 0 = top of the body) but stored as `i32` so a drag that
/// autoscrolls can carry the anchor *past* the top/bottom of the window
/// (negative or `>= rows`) without clamping — otherwise everything that scrolls
/// off the top is forgotten. `selection_finish` walks the scrollback to collect
/// the full span. Anchored locally so it survives repaints and scrolls the way
/// the outer terminal's native (Shift+drag) selection can't.
#[derive(Debug, Clone, Copy)]
pub struct Selection {
    /// Cell where the drag began. Row is a signed viewport row (may be
    /// off-screen after autoscroll).
    pub anchor: (i32, u16),
    /// Current end cell (moves while dragging).
    pub cursor: (i32, u16),
    /// True while the mouse button is held.
    pub dragging: bool,
}

impl Selection {
    /// (start, end) in reading order, so start <= end.
    pub fn ordered(&self) -> ((i32, u16), (i32, u16)) {
        if self.anchor <= self.cursor {
            (self.anchor, self.cursor)
        } else {
            (self.cursor, self.anchor)
        }
    }

    /// Whether a visible cell (row, col) falls inside the linear selection span.
    /// `row` is an on-screen viewport row; the off-screen part of the span is
    /// naturally excluded because those rows aren't rendered.
    pub fn contains(&self, row: u16, col: u16) -> bool {
        let row = row as i32;
        let (start, end) = self.ordered();
        if row < start.0 || row > end.0 {
            return false;
        }
        let lo = if row == start.0 { start.1 } else { 0 };
        let hi = if row == end.0 { end.1 } else { u16::MAX };
        col >= lo && col <= hi
    }

    fn is_empty(&self) -> bool {
        self.anchor == self.cursor
    }
}

pub struct Session {
    pub display_name: String,
    pub meta: SessionMeta,
    pub phase: SessionPhase,
    pub runtime: PtyRuntime,
    pub parser: ParserState,
    /// First argv element the user actually saw — the `$ ssh …` line of
    /// the ConnectScreen renders from this.
    pub display_argv: Vec<String>,
    /// Original SessionConfig kept so Ctrl+T can duplicate this tab into a
    /// fresh PTY without re-walking the host lookup.
    pub config: SessionConfig,
    /// Stored secret to auto-type at the next matching prompt. Cleared after
    /// it fires once, so retries on wrong passwords don't loop forever.
    pending_secret: Option<PendingSecret>,
    /// Tracks whether we've already typed a secret this session. Used to
    /// avoid spamming the remote with the same wrong password on retry.
    secret_sent: bool,
    /// Diagnostic strings produced during the session lifetime. Drained by the
    /// main loop into `app.ssh_log`.
    diagnostics: Vec<String>,
    /// Secret staged for `SSH_ASKPASS`; kept alive so its temp file is removed
    /// when the session ends. `Some` means ssh answers auth prompts silently
    /// via the helper (no visible prompt, no typing into the PTY).
    _askpass: Option<askpass::AskpassSecret>,
    /// Whether the askpass path is active (auth handled invisibly).
    use_askpass: bool,
    /// True once we've seen real bytes from ssh (i.e. the remote actually
    /// responded). Distinguishes a genuine connection from the timeout
    /// fail-open reveal, so the header never claims "connected" while ssh is
    /// still stuck on the TCP connect.
    connected: bool,
    /// Accumulated ssh stderr (the `-v` handshake), routed off the PTY via a
    /// side FIFO. Rendered as the connect spinner's debug tail / expanded log,
    /// and scanned for the real "authenticated to" connected marker.
    debug_log: String,
    /// Whether the connect screen shows the full debug log instead of the
    /// spinner + tail. Toggled by the user.
    debug_expanded: bool,
    /// True once any bytes have landed on the PTY (stdout/tty) — i.e. there is
    /// real shell/prompt content to show. Gates the connect-timeout fail-open:
    /// with `-v` debug now off the PTY, revealing a blank grid for a host that
    /// never answered is useless, so we keep the spinner until either real PTY
    /// content arrives or the child exits.
    saw_pty_bytes: bool,
    /// Active in-app mouse text selection over the grid, if any.
    pub selection: Option<Selection>,
    /// Transient "copied" toast: (message, shown_at). Rendered in a corner for
    /// a few seconds after a copy-on-select.
    pub copy_notice: Option<(String, Instant)>,
    /// While a selection drag is held past the top/bottom edge, the view keeps
    /// scrolling on each poll tick so you can select content beyond the
    /// viewport. `(dir, col)`: dir `+1` scrolls toward newer output (bottom
    /// edge), `-1` toward older (top edge); `col` is the pointer column to keep
    /// extending the selection to. `None` = not autoscrolling.
    drag_autoscroll: Option<(i32, u16)>,
    /// Optional PTY transcript writer; closed on session end.
    log: Option<crate::session_log::SessionLogWriter>,
}

impl Session {
    /// Spawn the child on a freshly allocated PTY and start the reader thread.
    pub fn spawn(
        config: SessionConfig,
        rows: u16,
        cols: u16,
        log: Option<crate::session_log::SessionLogWriter>,
    ) -> Result<Self> {
        // Reserve 1 row for header + 1 row for footer; ensure non-zero PTY.
        let pty_rows = rows.saturating_sub(2).max(1);
        let pty_cols = cols.max(1);

        // Prefer handing the secret to ssh via SSH_ASKPASS so the passphrase/
        // password prompt never shows on screen. Falls back to typing into the
        // PTY (below) if we can't stage it or ssh is too old to honour it.
        let mut env: Vec<(String, String)> = Vec::new();
        let mut askpass = None;
        let mut use_askpass = false;
        if let Some(secret) = config.pending_secret.as_ref() {
            if let Ok(exe) = std::env::current_exe() {
                if let Ok(guard) = askpass::AskpassSecret::new(secret.value()) {
                    env = guard.env(&exe);
                    askpass = Some(guard);
                    use_askpass = true;
                }
            }
        }

        let runtime = PtyRuntime::spawn(&config.argv, pty_rows, pty_cols, &env)?;
        let parser = ParserState::new(pty_rows, pty_cols);

        let display_argv = config.argv.clone();

        // Transparent auth log: say exactly how the stored secret is delivered.
        let mut diagnostics = Vec::new();
        if config.pending_secret.is_some() {
            diagnostics.push(if use_askpass {
                "auth: credential handed to ssh via SSH_ASKPASS".to_string()
            } else {
                "auth: SSH_ASKPASS unavailable — will type at the prompt".to_string()
            });
        }

        Ok(Self {
            display_name: config.display_name.clone(),
            meta: config.meta.clone(),
            phase: SessionPhase::Connecting {
                started_at: Instant::now(),
            },
            runtime,
            parser,
            display_argv,
            pending_secret: config.pending_secret.clone(),
            secret_sent: false,
            diagnostics,
            _askpass: askpass,
            use_askpass,
            connected: false,
            debug_log: String::new(),
            debug_expanded: false,
            saw_pty_bytes: false,
            selection: None,
            copy_notice: None,
            drag_autoscroll: None,
            config,
            log,
        })
    }

    /// Attach a session log writer after the PTY child is running.
    pub fn set_log(&mut self, log: crate::session_log::SessionLogWriter) {
        self.log = Some(log);
    }

    // ── Text selection ────────────────────────────────────────────

    /// Begin a selection at visible cell (row, col).
    pub fn selection_start(&mut self, row: u16, col: u16) {
        self.selection = Some(Selection {
            anchor: (row as i32, col),
            cursor: (row as i32, col),
            dragging: true,
        });
    }

    /// Extend the active drag to (row, col).
    pub fn selection_extend(&mut self, row: u16, col: u16) {
        if let Some(sel) = self.selection.as_mut() {
            if sel.dragging {
                sel.cursor = (row as i32, col);
            }
        }
    }

    /// Finish the drag; return the selected text (trailing spaces trimmed per
    /// line) if the selection covers anything, else `None`.
    pub fn selection_finish(&mut self) -> Option<String> {
        self.drag_autoscroll = None;
        let sel = self.selection.as_ref()?;
        let (start, end) = sel.ordered();
        let empty = sel.is_empty();
        if let Some(sel) = self.selection.as_mut() {
            sel.dragging = false;
        }
        if empty {
            self.selection = None;
            return None;
        }
        let (rows, cols) = self.parser.screen().size();
        // contents_between's end column is exclusive; +1 to include the cell
        // under the cursor, clamped to the row width.
        let end_col = end.1.saturating_add(1).min(cols);

        // Fast path: the whole span is within the current visible window, so a
        // single contents_between (which honours wrapped lines) suffices.
        let text = if start.0 >= 0 && end.0 < rows as i32 {
            self.parser
                .screen()
                .contents_between(start.0 as u16, start.1, end.0 as u16, end_col)
        } else {
            // Slow path: the drag autoscrolled, so part of the span is off the
            // current window. Walk the scrollback row by row and stitch it up.
            self.collect_scrolled_selection(start, end, end_col)
        };
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    /// Collect a selection whose rows extend beyond the current visible window
    /// by scrolling the view so each row becomes addressable, then reading it.
    /// Restores the original scrollback position afterwards so the view doesn't
    /// jump. `start`/`end` are ordered signed viewport rows; `end_col` is
    /// exclusive.
    fn collect_scrolled_selection(
        &mut self,
        start: (i32, u16),
        end: (i32, u16),
        end_col: u16,
    ) -> String {
        let cur = self.parser.scrollback() as i32;
        let (rows, cols) = self.parser.screen().size();
        let mut out = String::new();
        for r in start.0..=end.0 {
            // Bring current-frame row `r` on-screen: scrolling to `cur - r`
            // lands it at visible row 0 (clamped to the real buffer bounds).
            let want = (cur - r).max(0) as usize;
            self.parser.set_scrollback(want);
            let actual = self.parser.scrollback() as i32;
            let v = r + (actual - cur); // visible row of `r` at this scrollback
            if (0..rows as i32).contains(&v) {
                let sc = if r == start.0 { start.1 } else { 0 };
                let ec = if r == end.0 { end_col } else { cols };
                if sc < ec {
                    let line = self
                        .parser
                        .screen()
                        .contents_between(v as u16, sc, v as u16, ec);
                    out.push_str(&line);
                }
            }
            if r != end.0 {
                out.push('\n');
            }
        }
        // Snap back to where the user left the view.
        self.parser.set_scrollback(cur.max(0) as usize);
        out
    }

    /// Drop any selection (e.g. a plain click or Esc).
    pub fn selection_clear(&mut self) {
        self.selection = None;
        self.drag_autoscroll = None;
    }

    /// Arm/disarm edge autoscroll for the in-progress selection drag. Called
    /// from the mouse handler: `Some(+1)` when the pointer is at/below the
    /// bottom edge, `Some(-1)` at/above the top edge, `None` when back inside
    /// the viewport. `col` is the pointer column so the tick keeps growing the
    /// selection along the edge.
    pub fn set_drag_autoscroll(&mut self, dir: Option<i32>, col: u16) {
        self.drag_autoscroll = dir.map(|d| (d.signum(), col));
    }

    /// Advance edge autoscroll by one row. Call once per poll tick so a drag
    /// held past an edge keeps scrolling even when the mouse stops moving.
    /// No-ops unless a drag is active and armed, and stops at the scrollback
    /// bounds (nothing left to reveal).
    pub fn selection_autoscroll_tick(&mut self) {
        let Some((dir, col)) = self.drag_autoscroll else {
            return;
        };
        if !self.selection.as_ref().is_some_and(|s| s.dragging) {
            self.drag_autoscroll = None;
            return;
        }
        let (rows, _) = self.parser.screen().size();
        if dir > 0 {
            // Toward newer output: nothing below once we're at the live bottom.
            if self.parser.scrollback() == 0 {
                return;
            }
            self.parser.scroll_down(1);
            self.selection_scroll_shift(-1);
            self.selection_extend(rows.saturating_sub(1), col);
        } else {
            // Toward older output: stop when scrollback can't grow further.
            let before = self.parser.scrollback();
            self.parser.scroll_up(1);
            if self.parser.scrollback() == before {
                return;
            }
            self.selection_scroll_shift(1);
            self.selection_extend(0, col);
        }
    }

    /// Shift the selection vertically so it tracks the same text when the view
    /// scrolls by `delta` rows (positive = content moved down / scrolled back).
    /// Rows are signed and left unclamped so the anchor can travel off-screen
    /// during an autoscrolling drag without losing its true position.
    pub fn selection_scroll_shift(&mut self, delta: i32) {
        if let Some(sel) = self.selection.as_mut() {
            sel.anchor.0 += delta;
            sel.cursor.0 += delta;
        }
    }

    /// Record a transient "copied" toast.
    pub fn set_copy_notice(&mut self, msg: String) {
        self.copy_notice = Some((msg, Instant::now()));
    }

    /// Drain accumulated diagnostic strings (e.g. for the SSH log panel).
    pub fn take_diagnostics(&mut self) -> Vec<String> {
        std::mem::take(&mut self.diagnostics)
    }

    /// Drain PTY events from the reader thread into the parser. Non-blocking;
    /// safe to call every frame. After each batch of bytes lands we scan the
    /// screen for an unanswered password / passphrase prompt and auto-type
    /// the stored secret exactly once.
    pub fn drain(&mut self) {
        let mut had_bytes = false;
        let mut had_stderr = false;
        while let Some(event) = self.runtime.try_recv() {
            match event {
                PtyEvent::Bytes(bytes) => {
                    self.parser.process(&bytes);
                    if let Some(log) = self.log.as_mut() {
                        if log.append(&bytes).is_err() {
                            self.diagnostics
                                .push("session log write failed; logging disabled".into());
                            self.log = None;
                        }
                    }
                    had_bytes = true;
                    self.saw_pty_bytes = true;
                }
                PtyEvent::Stderr(bytes) => {
                    // ssh's `-v` handshake — kept off the PTY grid, accumulated
                    // for the connect spinner's debug view. Cap the buffer so a
                    // long-lived session can't grow it without bound.
                    self.debug_log.push_str(&String::from_utf8_lossy(&bytes));
                    if self.debug_log.len() > DEBUG_LOG_CAP {
                        let cut = self.debug_log.len() - DEBUG_LOG_CAP;
                        self.debug_log.drain(..cut);
                    }
                    had_stderr = true;
                }
                PtyEvent::Exited(status) => {
                    self.runtime.reap_child();
                    self.diagnostics
                        .push(format!("session: ssh exited ({status})"));
                    self.phase = SessionPhase::Exited {
                        status,
                        at: Instant::now(),
                    };
                }
            }
        }
        if had_stderr {
            // The "authenticated to" marker arrives on stderr, so re-check even
            // when no PTY bytes landed this tick.
            self.maybe_detect_connected();
        }
        if had_bytes {
            self.maybe_send_pending_secret();
            self.maybe_detect_connected();
            self.maybe_reveal();
        }
        // Safety net: reveal after the timeout even with no output at all, so a
        // session blocked on auth never hangs the connect screen forever.
        self.reveal_on_timeout();
    }

    /// Reveal the live terminal once the connect timeout elapses — but only if
    /// real PTY content has arrived (a prompt/banner/host-key question we might
    /// have failed to auto-answer). With `-v` debug now siphoned off the PTY, a
    /// host that never answered has a blank grid, so failing open there would
    /// just hide the spinner + debug tail behind emptiness. Those sessions stay
    /// on the spinner until the child exits.
    fn reveal_on_timeout(&mut self) {
        if let SessionPhase::Connecting { started_at } = self.phase {
            if started_at.elapsed() >= REVEAL_TIMEOUT && self.saw_pty_bytes {
                self.diagnostics
                    .push("auth: connect timed out — showing live terminal".into());
                self.phase = SessionPhase::Running {
                    started_at: Instant::now(),
                };
            }
        }
    }

    /// Decide whether to switch from the scripted connect animation to the
    /// live terminal. For a session armed with a stored credential we keep the
    /// animation playing over the banner + `password:` prompt and only reveal
    /// once the secret has been auto-typed — so the user never sees the raw
    /// auth handshake flicker by. Sessions without a stored secret (key auth,
    /// manual password) reveal as soon as the first bytes arrive. A timeout
    /// fails open so a prompt we couldn't match (or interactive auth) is never
    /// hidden forever.
    fn maybe_reveal(&mut self) {
        let SessionPhase::Connecting { started_at } = self.phase else {
            return;
        };
        // A host-key verification prompt needs a yes/no from the user right
        // now — reveal immediately even for an armed session, or the connect
        // silently stalls behind the animation.
        let elapsed = started_at.elapsed();
        if self.awaiting_host_verification()
            || should_reveal(self.was_armed(), self.secret_sent, elapsed)
        {
            if self.was_armed() && !self.secret_sent && elapsed >= REVEAL_TIMEOUT {
                self.diagnostics
                    .push("auth: no matching prompt within timeout — showing live terminal".into());
            }
            self.phase = SessionPhase::Running {
                started_at: Instant::now(),
            };
        }
    }

    /// Whether ssh has genuinely reached the remote (real bytes seen), as
    /// opposed to the connect screen being revealed by the timeout fail-open.
    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// Whether we're being asked to accept an unknown host key. ssh prints the
    /// "authenticity of host" block partly on stderr and the yes/no prompt on
    /// the tty, so check both the live screen and the captured debug log.
    fn awaiting_host_verification(&self) -> bool {
        let tail = current_screen_tail(self.parser.screen()).to_ascii_lowercase();
        if HOST_VERIFY_NEEDLES.iter().any(|n| tail.contains(n)) {
            return true;
        }
        let debug = self.debug_log.to_ascii_lowercase();
        HOST_VERIFY_NEEDLES.iter().any(|n| debug.contains(n))
    }

    /// Latch `connected` once ssh's `-v` output shows the real auth-success
    /// marker. This is the only honest "connected" signal — the mere arrival
    /// of bytes doesn't count, since `-v` prints local debug lines before the
    /// TCP connect even completes. The debug stream now lives in `debug_log`
    /// (siphoned off the PTY), so no screen scrubbing is needed: the PTY only
    /// ever carries the post-auth shell (banner + prompt).
    fn maybe_detect_connected(&mut self) {
        if self.connected {
            return;
        }
        let text = self.debug_log.to_ascii_lowercase();
        if CONNECTED_NEEDLES.iter().any(|n| text.contains(n)) {
            self.connected = true;
        }
    }

    /// The accumulated ssh `-v` debug output (host-key search, kex, auth).
    pub fn debug_log(&self) -> &str {
        &self.debug_log
    }

    /// True when ssh refused because the server's host key CHANGED versus
    /// known_hosts (a mismatch), as opposed to a merely-unknown first-time host.
    /// This is the case worth offering to accept — a changed key can be a
    /// legitimate server rebuild or a MITM, so the user must opt in.
    pub fn host_key_changed(&self) -> bool {
        let log = self.debug_log.to_ascii_lowercase();
        log.contains("host identification has changed")
            || (log.contains("host key for") && log.contains("has changed"))
    }

    /// The known_hosts host spec to purge when accepting a changed key —
    /// `[addr]:port` for a non-default port, plain `addr` for port 22. `None`
    /// when the remote address is unknown.
    pub fn known_hosts_spec(&self) -> Option<String> {
        let addr = self.meta.address.as_ref()?;
        let port = self.meta.port.unwrap_or(22);
        Some(if port == 22 {
            addr.clone()
        } else {
            format!("[{addr}]:{port}")
        })
    }

    /// Whether the connect screen should show the full debug log.
    pub fn debug_expanded(&self) -> bool {
        self.debug_expanded
    }

    /// Flip between the spinner+tail view and the full debug log.
    pub fn toggle_debug_expanded(&mut self) {
        self.debug_expanded = !self.debug_expanded;
    }

    /// A short, human-readable explanation for a connect that ended without
    /// ever reaching a shell. Maps the common ssh/network errors (captured on
    /// stderr) to plain language; falls back to the raw error line, then to the
    /// child's exit status.
    pub fn failure_reason(&self) -> String {
        let log = self.debug_log.to_ascii_lowercase();
        for (needle, explanation) in FAILURE_EXPLANATIONS {
            if log.contains(needle) {
                return (*explanation).to_string();
            }
        }
        // No known pattern — surface the most telling raw line: ssh prints its
        // fatal error without the `debug1:` prefix, so prefer a non-debug line.
        if let Some(line) = self
            .debug_log
            .lines()
            .rev()
            .map(|l| l.trim())
            .find(|l| !l.is_empty() && !l.starts_with("debug"))
        {
            return line.to_string();
        }
        // Nothing useful on stderr at all — report how the child died.
        match &self.phase {
            SessionPhase::Exited { status, .. } => format!("ssh exited ({status})"),
            _ => "connection failed".to_string(),
        }
    }

    /// If the live screen ends with a prompt that matches our stored secret
    /// kind, type it now. Fires at most once per session so a wrong password
    /// doesn't loop the connect.
    fn maybe_send_pending_secret(&mut self) {
        // When askpass is active, ssh gets the secret itself — never type into
        // the PTY. This is only a fallback for when askpass couldn't be staged.
        if self.use_askpass || self.secret_sent || self.pending_secret.is_none() {
            return;
        }
        let secret = self.pending_secret.as_ref().unwrap().clone();
        let line = line_before_cursor(self.parser.screen());
        let lower = line.to_ascii_lowercase();

        let (matched, kind) = match secret {
            PendingSecret::Password(_) => (ends_with_prompt(&lower, PASSWORD_NEEDLES), "password"),
            PendingSecret::Passphrase(_) => {
                (ends_with_prompt(&lower, PASSPHRASE_NEEDLES), "passphrase")
            }
        };

        if !matched {
            return;
        }

        let mut bytes = secret.value().as_bytes().to_vec();
        bytes.push(b'\r');
        match self.runtime.write(&bytes) {
            Ok(()) => {
                // Don't log the byte count — it leaks the secret's length.
                self.diagnostics
                    .push(format!("auth: provided stored {kind}"));
            }
            Err(e) => {
                self.diagnostics
                    .push(format!("auth: could not provide {kind}: {e:#}"));
            }
        }
        self.secret_sent = true;
        self.pending_secret = None;
    }

    /// Has this session been armed with a stored secret? Used by callers
    /// (the main loop) to decide whether to surface an "armed but never
    /// matched" diagnostic when the session ends.
    pub fn was_armed(&self) -> bool {
        // When askpass handles auth there is no on-screen prompt to hide, so
        // the connect screen should reveal like an ordinary key-auth session
        // (immediately) rather than waiting to type a secret.
        self.config.pending_secret.is_some() && !self.use_askpass
    }

    /// Whether the auto-typer has fired.
    pub fn secret_was_sent(&self) -> bool {
        self.secret_sent
    }

    /// Snapshot of the bottom rows of the visible screen — used to surface
    /// diagnostics like "armed but no prompt seen, screen shows X".
    pub fn screen_tail_snippet(&self) -> String {
        current_screen_tail(self.parser.screen())
    }

    /// Update both the PTY size and the parser grid. Body rows = total - header - footer.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        let pty_rows = rows.saturating_sub(2).max(1);
        let pty_cols = cols.max(1);
        self.parser.set_size(pty_rows, pty_cols);
        let _ = self.runtime.resize(pty_rows, pty_cols);
    }

    /// Write raw bytes (already-encoded keystroke) to the PTY.
    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.runtime.write(bytes)
    }

    /// Forward a clipboard paste to the PTY. When the remote application has
    /// enabled bracketed-paste mode (DECSET 2004) — as terminal-aware editors
    /// like vim do — wrap the payload in `ESC[200~ … ESC[201~` so the remote
    /// treats it as a single paste and disables autoindent / comment
    /// continuation. Without this, pasting into vim's insert mode re-triggers
    /// autoindent per line, cascading indentation and repeating the comment
    /// leader. If the remote hasn't asked for bracketed paste, send raw so the
    /// markers don't leak in as literal text.
    pub fn write_paste(&mut self, text: &[u8]) -> Result<()> {
        let payload = encode_paste(text, self.parser.screen().bracketed_paste());
        self.runtime.write(&payload)
    }
}

/// Map an (unclamped) body-local pointer row to a selection-autoscroll
/// direction: `Some(1)` at/below the last row (reveal newer output), `Some(-1)`
/// at/above the first row (reveal older), `None` when inside the viewport.
/// Pulled out of the mouse handler so the edge decision is unit-testable.
pub fn edge_autoscroll_dir(raw_row: i32, rows: u16) -> Option<i32> {
    if raw_row >= rows as i32 - 1 {
        Some(1)
    } else if raw_row <= 0 {
        Some(-1)
    } else {
        None
    }
}

/// Wrap `text` in bracketed-paste markers when the remote requested that mode,
/// otherwise return it unchanged. Pulled out of [`Session::write_paste`] so the
/// framing is unit-testable without a live PTY.
fn encode_paste(text: &[u8], bracketed: bool) -> Vec<u8> {
    if bracketed {
        let mut buf = Vec::with_capacity(text.len() + 12);
        buf.extend_from_slice(b"\x1b[200~");
        buf.extend_from_slice(text);
        buf.extend_from_slice(b"\x1b[201~");
        buf
    } else {
        text.to_vec()
    }
}

/// How long an armed session keeps the connect animation up while waiting to
/// auto-type a credential. If no matching prompt appears within this window
/// (unrecognised prompt wording, interactive/2FA auth, key auth that still
/// emits a banner), we reveal the live terminal so the user can take over.
const REVEAL_TIMEOUT: Duration = Duration::from_secs(6);

/// Whether to switch from the connect animation to the live terminal.
/// Unarmed sessions reveal immediately; armed ones stay hidden until the
/// secret is typed, or until the reveal timeout fails open.
fn should_reveal(armed: bool, secret_sent: bool, elapsed: Duration) -> bool {
    !armed || secret_sent || elapsed >= REVEAL_TIMEOUT
}

/// Things ssh / sshd actually say when asking for a password. Keep the
/// list small and substring-checked to tolerate locale variations and
/// banner prefixes. Lower-case only.
const PASSWORD_NEEDLES: &[&str] = &[
    "password:",
    "password: ",
    "'s password:",
    "(current) unix password:",
    "current password:",
];

/// Stems we look for in passphrase prompts.
const PASSPHRASE_NEEDLES: &[&str] = &["passphrase for", "enter passphrase"];

/// Phrases ssh prints when it needs the user to accept an unknown host key
/// ("Are you sure you want to continue connecting (yes/no/[fingerprint])?").
/// This needs interactive input we can't supply, so the connect screen must
/// reveal the live terminal immediately instead of hiding it.
const HOST_VERIFY_NEEDLES: &[&str] = &[
    "authenticity of host",
    "continue connecting",
    "key fingerprint is",
];

/// Real markers ssh (`-v`) prints once it has genuinely authenticated to the
/// remote. Seeing any of these is the only honest "connected" signal. Lower
/// case only.
const CONNECTED_NEEDLES: &[&str] = &["authenticated to ", "authenticated ("];

/// Cap on the retained ssh `-v` debug buffer (bytes). Old output past this is
/// dropped from the front so a long session can't grow it without bound.
const DEBUG_LOG_CAP: usize = 64 * 1024;

/// Ordered (lowercase needle → plain-language reason) map for failed connects.
/// First match wins, so keep more specific patterns before generic ones.
const FAILURE_EXPLANATIONS: &[(&str, &str)] = &[
    (
        "could not resolve hostname",
        "Could not resolve hostname — check the address or your DNS.",
    ),
    (
        "name or service not known",
        "Could not resolve hostname — check the address or your DNS.",
    ),
    (
        "connection timed out",
        "Connection timed out — host is unreachable, down, or firewalled.",
    ),
    (
        "operation timed out",
        "Connection timed out — host is unreachable, down, or firewalled.",
    ),
    (
        "connection refused",
        "Connection refused — nothing is listening on that port.",
    ),
    (
        "no route to host",
        "No route to host — the network is unreachable.",
    ),
    (
        "network is unreachable",
        "Network is unreachable — check your connection or VPN.",
    ),
    (
        "host key verification failed",
        "Host key verification failed — the server's key changed or is unknown.",
    ),
    (
        "permission denied",
        "Authentication failed — key or password rejected (permission denied).",
    ),
    (
        "too many authentication failures",
        "Too many authentication failures — the server rejected every key tried.",
    ),
    ("connection reset", "Connection reset by the remote host."),
    (
        "connection closed",
        "Connection closed by the remote host before authentication.",
    ),
];

/// Substring match across the screen tail. Tolerant to position so prompts
/// like "deploy@host's password:" or a prompt followed by a trailing space
/// still match.
#[cfg(test)]
fn contains_prompt(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

/// A prompt is only "live" when the cursor sits right after it, waiting for
/// input. Matching the *end* of the cursor line (not a substring anywhere in
/// the tail) prevents auto-typing the secret into a shell because a banner or
/// MOTD merely *mentions* e.g. "change your password:".
/// The line must both mention the needle and end with `:` — i.e. look like an
/// input prompt the cursor is parked on, not prose that scrolled past.
fn ends_with_prompt(line: &str, needles: &[&str]) -> bool {
    let trimmed = line.trim_end();
    trimmed.ends_with(':') && needles.iter().any(|n| trimmed.contains(n.trim_end()))
}

/// Text of the cursor row up to (and excluding) the cursor column.
fn line_before_cursor(screen: &vt100::Screen) -> String {
    let (rows, _) = screen.size();
    if rows == 0 {
        return String::new();
    }
    let (cursor_row, cursor_col) = screen.cursor_position();
    let row = cursor_row.min(rows - 1);
    let mut out = String::new();
    for col in 0..cursor_col {
        if let Some(cell) = screen.cell(row, col) {
            if cell.has_contents() {
                out.push_str(&cell.contents());
            } else {
                out.push(' ');
            }
        }
    }
    out
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(log) = self.log.take() {
            let _ = log.close();
        }
        // PtyRuntime::Drop kills the child and joins the reader thread.
    }
}

/// Return a few rows of the screen ending at the cursor row, as a single
/// string. Used by the prompt scanner.
///
/// The window is anchored on the *cursor row*, not the physical bottom of the
/// grid: a freshly-cleared ssh session prints its banner and `password:`
/// prompt at the TOP of a tall PTY, leaving the bottom rows blank. Reading the
/// physical bottom would see "(blank)" and miss the prompt entirely (which is
/// exactly the bug where stored passwords were never auto-typed). The cursor
/// sits on the prompt line, so anchoring there works whether the prompt is at
/// the top of a fresh screen or at the bottom of a scrolled shell.
fn current_screen_tail(screen: &vt100::Screen) -> String {
    let (rows, cols) = screen.size();
    if rows == 0 {
        return String::new();
    }
    let (cursor_row, _) = screen.cursor_position();
    let last = cursor_row.min(rows - 1);
    let start = last.saturating_sub(3);
    let mut out = String::new();
    for row in start..=last {
        for col in 0..cols {
            if let Some(cell) = screen.cell(row, col) {
                if cell.has_contents() {
                    out.push_str(&cell.contents());
                }
            }
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod selection_tests {
    use super::Selection;

    fn sel(a: (i32, u16), c: (i32, u16)) -> Selection {
        Selection {
            anchor: a,
            cursor: c,
            dragging: false,
        }
    }

    #[test]
    fn contains_is_reading_order_and_direction_agnostic() {
        // Selection from (1,3) to (3,2): partial first row, full middle, partial last.
        let s = sel((1, 3), (3, 2));
        // First row: only cols >= 3.
        assert!(!s.contains(1, 2));
        assert!(s.contains(1, 3));
        assert!(s.contains(1, 999));
        // Middle row: everything.
        assert!(s.contains(2, 0));
        assert!(s.contains(2, 500));
        // Last row: only cols <= 2.
        assert!(s.contains(3, 2));
        assert!(!s.contains(3, 3));
        // Outside the row span.
        assert!(!s.contains(0, 3));
        assert!(!s.contains(4, 0));
        // Dragging upward (cursor before anchor) selects the same span.
        assert!(sel((3, 2), (1, 3)).contains(2, 10));
    }

    #[test]
    fn single_row_selection_bounds() {
        let s = sel((5, 4), (5, 8));
        assert!(!s.contains(5, 3));
        assert!(s.contains(5, 4));
        assert!(s.contains(5, 8));
        assert!(!s.contains(5, 9));
        assert!(!s.contains(4, 6));
    }

    #[test]
    fn anchor_scrolled_above_the_window_still_selects_visible_rows() {
        // After autoscrolling down, the anchor sits at a negative viewport row
        // (above the top). Visible rows 0..=cursor must still be highlighted —
        // the off-screen part is just not rendered, not lost.
        let s = sel((-5, 2), (3, 6));
        assert!(s.contains(0, 0)); // top visible row, fully inside
        assert!(s.contains(2, 40));
        assert!(s.contains(3, 6));
        assert!(!s.contains(3, 7)); // past the end column on the last row
        assert!(!s.contains(4, 0)); // below the selection
    }
}

#[cfg(test)]
mod autoscroll_tests {
    use super::edge_autoscroll_dir;

    #[test]
    fn edge_direction_by_pointer_row() {
        let rows = 24;
        // Inside the viewport → no autoscroll.
        assert_eq!(edge_autoscroll_dir(1, rows), None);
        assert_eq!(edge_autoscroll_dir(22, rows), None);
        // At/below the last row → scroll toward newer output.
        assert_eq!(edge_autoscroll_dir(23, rows), Some(1));
        assert_eq!(edge_autoscroll_dir(40, rows), Some(1));
        // At/above the first row (or into the header, negative) → older output.
        assert_eq!(edge_autoscroll_dir(0, rows), Some(-1));
        assert_eq!(edge_autoscroll_dir(-3, rows), Some(-1));
    }
}

#[cfg(test)]
mod paste_tests {
    use super::*;

    #[test]
    fn wraps_only_when_bracketed_paste_enabled() {
        let text = b"# header\n    indented\n";
        // Remote asked for bracketed paste (e.g. vim): wrap in ESC[200~/ESC[201~.
        assert_eq!(
            encode_paste(text, true),
            b"\x1b[200~# header\n    indented\n\x1b[201~".to_vec()
        );
        // Remote did not: send raw so markers don't leak in as literal text.
        assert_eq!(encode_paste(text, false), text.to_vec());
    }
}

#[cfg(test)]
mod prompt_tests {
    use super::*;

    #[test]
    fn stderr_is_siphoned_off_the_pty() {
        // stdout must land on the PTY grid; stderr must be routed through the
        // side FIFO into debug_log — never onto the grid.
        let config = SessionConfig {
            argv: vec![
                "sh".into(),
                "-c".into(),
                "printf OUT_MARKER; printf ERR_MARKER 1>&2".into(),
            ],
            display_name: "t".into(),
            meta: SessionMeta::default(),
            pending_secret: None,
        };
        let mut s = Session::spawn(config, 24, 80, None).unwrap();

        // Pump the reader threads; both writes are tiny and immediate.
        let mut got_err = false;
        for _ in 0..200 {
            s.drain();
            got_err = s.debug_log().contains("ERR_MARKER");
            let got_out = s.screen_tail_snippet().contains("OUT_MARKER");
            if got_err && got_out {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(
            got_err,
            "stderr should be captured in debug_log, got {:?}",
            s.debug_log()
        );
        assert!(
            s.screen_tail_snippet().contains("OUT_MARKER"),
            "stdout should be on the PTY grid, tail: {:?}",
            s.screen_tail_snippet()
        );
        assert!(
            !s.debug_log().contains("OUT_MARKER"),
            "stdout must not leak into the debug log"
        );
        assert!(
            !s.screen_tail_snippet().contains("ERR_MARKER"),
            "stderr must not appear on the PTY grid"
        );
    }

    #[test]
    fn failed_connect_surfaces_reason_in_debug_log() {
        // Mimic an unreachable host: ssh writes the error to stderr and exits
        // with nothing on stdout. The reason must end up in debug_log (so the
        // render layer can show it) and the session must never latch connected.
        let config = SessionConfig {
            argv: vec![
                "sh".into(),
                "-c".into(),
                "printf 'connect to host x port 22: Connection timed out' 1>&2; exit 1".into(),
            ],
            display_name: "x".into(),
            meta: SessionMeta::default(),
            pending_secret: None,
        };
        let mut s = Session::spawn(config, 24, 80, None).unwrap();
        // The child's exit and its stderr bytes race between the two reader
        // threads; the main loop keeps draining regardless, so wait for both.
        let mut exited = false;
        for _ in 0..200 {
            s.drain();
            exited = matches!(s.phase, SessionPhase::Exited { .. });
            if exited && s.debug_log().to_ascii_lowercase().contains("timed out") {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(exited, "child should have exited");
        assert!(!s.is_connected(), "must not latch connected on failure");
        assert!(
            s.debug_log()
                .to_ascii_lowercase()
                .contains("connection timed out"),
            "failure reason should be in debug_log, got {:?}",
            s.debug_log()
        );
    }

    #[test]
    fn detects_password_prompt() {
        // The real scanner lowercases the whole tail first.
        let cases = [
            "deploy@host's password: ",
            "Password:",
            "deploy@10.0.0.1's password:",
            "(current) UNIX password:",
        ];
        for c in cases {
            assert!(
                contains_prompt(&c.to_ascii_lowercase(), PASSWORD_NEEDLES),
                "should match password prompt in {c:?}"
            );
        }
        assert!(!contains_prompt("hello world", PASSWORD_NEEDLES));
    }

    #[test]
    fn reveal_policy_hides_auth_handshake_for_armed_sessions() {
        let z = Duration::from_secs(0);
        // Unarmed (key auth / manual): reveal as soon as bytes arrive.
        assert!(should_reveal(false, false, z));
        // Armed, password not yet typed: keep the animation up.
        assert!(!should_reveal(true, false, z));
        // Armed, password typed: reveal the post-auth shell.
        assert!(should_reveal(true, true, z));
        // Armed, prompt never matched: fail open after the timeout.
        assert!(should_reveal(true, false, REVEAL_TIMEOUT));
        assert!(!should_reveal(
            true,
            false,
            REVEAL_TIMEOUT - Duration::from_millis(1)
        ));
    }

    #[test]
    fn host_key_verification_prompt_is_detected() {
        let mut parser = vt100::Parser::new(40, 100, 0);
        parser.process(
            b"The authenticity of host 'srv (10.0.0.1)' can't be established.\r\n\
              ED25519 key fingerprint is SHA256:abc123def456.\r\n\
              This key is not known by any other names.\r\n\
              Are you sure you want to continue connecting (yes/no/[fingerprint])? ",
        );
        let tail = current_screen_tail(parser.screen()).to_ascii_lowercase();
        assert!(
            HOST_VERIFY_NEEDLES.iter().any(|n| tail.contains(n)),
            "host-key prompt must be detected, tail: {tail:?}"
        );
    }

    #[test]
    fn screen_tail_finds_prompt_at_top_of_tall_screen() {
        // Regression: a fresh ssh session prints its banner + password prompt
        // at the top of a tall PTY, leaving the bottom blank. The scanner must
        // still see the prompt (it used to read the physical bottom 3 rows and
        // find "(blank)", so the stored password was never auto-typed).
        let mut parser = vt100::Parser::new(40, 100, 0);
        parser.process(
            b"** WARNING: connection is not using a post-quantum key exchange algorithm.\r\n\
              ** This session may be vulnerable to \"store now, decrypt later\" attacks.\r\n\
              su-adm@10.100.19.105's password: ",
        );
        let tail = current_screen_tail(parser.screen());
        assert!(
            contains_prompt(&tail.to_ascii_lowercase(), PASSWORD_NEEDLES),
            "scanner must find the top-of-screen prompt, got tail: {tail:?}"
        );
    }

    #[test]
    fn motd_mentioning_password_does_not_trigger_autotype() {
        // A banner that *mentions* "password:" mid-text must not match: the
        // scanner now looks only at the cursor line, which must end with ':'.
        let mut parser = vt100::Parser::new(40, 100, 0);
        parser.process(
            b"* Policy: you must change your password: rotate it every 90 days.\r\n\
              Loading profile...\r\n",
        );
        let line = line_before_cursor(parser.screen());
        assert!(
            !ends_with_prompt(&line.to_ascii_lowercase(), PASSWORD_NEEDLES),
            "MOTD text must not look like a live prompt, got line: {line:?}"
        );

        // A real prompt (cursor parked right after "password: ") still matches.
        parser.process(b"deploy@host's password: ");
        let line = line_before_cursor(parser.screen());
        assert!(
            ends_with_prompt(&line.to_ascii_lowercase(), PASSWORD_NEEDLES),
            "live prompt must match, got line: {line:?}"
        );
    }

    #[test]
    fn passphrase_prompt_matches_at_cursor_line() {
        let mut parser = vt100::Parser::new(10, 100, 0);
        parser.process(b"Enter passphrase for key '/home/me/.ssh/id_rsa': ");
        let line = line_before_cursor(parser.screen());
        assert!(ends_with_prompt(
            &line.to_ascii_lowercase(),
            PASSPHRASE_NEEDLES
        ));
    }

    #[test]
    fn detects_passphrase_prompt() {
        let cases = [
            "Enter passphrase for key '/home/me/.ssh/id_rsa':",
            "Enter passphrase for /home/me/.ssh/id_rsa:",
            "enter passphrase:",
        ];
        for c in cases {
            assert!(
                contains_prompt(&c.to_ascii_lowercase(), PASSPHRASE_NEEDLES),
                "should match passphrase prompt in {c:?}"
            );
        }
    }
}
