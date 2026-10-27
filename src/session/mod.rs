//! Embedded PTY-backed SSH session.
//!
//! Replaces the external kitty/ghostty launcher. When a host is connected,
//! sshub spawns ssh on a pseudo-TTY, parses output through a VT100 emulator,
//! and renders the terminal grid fullscreen inside the existing ratatui app.

pub mod askpass;
pub mod askpass_channel;
pub mod auth_prompt;
pub mod history;
pub mod hop_auth;
pub mod interactive_auth;
pub mod keys;
pub mod parser;
pub mod pty;
pub mod render;
pub mod share_net;

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;

pub use parser::ParserState;
pub use pty::{PtyEvent, PtyRuntime};
pub use share_net::NetShare;

/// Smallest grid a session is ever given. vt100 0.16 panicked below 2x2
/// (wrapping text on a 1-row grid, a wide character on a 1-column grid);
/// 0.17 fixed both, and the floor stays as the smallest grid a shell is of
/// use in. The renderer clips whatever no longer fits the pane.
pub(crate) fn floor_grid((rows, cols): (u16, u16)) -> (u16, u16) {
    (rows.max(2), cols.max(2))
}

/// Identity of an open session, stable for its whole life. `App::sessions` is
/// a `Vec` that shifts on every close, so anything that must keep pointing at
/// one session (a pane in a split view) holds this instead of an index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionId(u64);

impl SessionId {
    fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

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
    /// Identity being pushed if this is a key push session.
    pub key_push_identity: Option<String>,
    /// Actual unique host name (entry.name()).
    pub host_name: String,
    /// The jump hosts' own secrets, for a session without the interactive
    /// channel (key push): staged beside `pending_secret`, so a hop's prompt is
    /// answered with that hop's password and never with the destination's.
    pub hops: Vec<hop_auth::HopSecret>,
}

/// Auto-respond once to either a password or passphrase prompt. `Debug` is
/// written by hand: the value never reaches `{:?}`, a log or a panic message.
#[derive(Clone)]
pub enum PendingSecret {
    /// Sent on `password:`-style prompts only.
    Password(String),
    /// Sent on `Enter passphrase for …` prompts only.
    Passphrase(String),
}

impl std::fmt::Debug for PendingSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PendingSecret::Password(_) => "PendingSecret::Password(..)",
            PendingSecret::Passphrase(_) => "PendingSecret::Passphrase(..)",
        })
    }
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
    /// Stable identity; see [`SessionId`].
    pub id: SessionId,
    pub display_name: String,
    pub meta: SessionMeta,
    pub phase: SessionPhase,
    pub runtime: PtyRuntime,
    pub parser: ParserState,
    pub history: history::SessionHistory,
    pub(crate) local_shell: bool,
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
    pub auth: Option<interactive_auth::InteractiveAuth>,
    /// True once we've seen real bytes from ssh (i.e. the remote actually
    /// responded). Distinguishes a genuine connection from the timeout
    /// fail-open reveal, so the header never claims "connected" while ssh is
    /// still stuck on the TCP connect.
    connected: bool,
    /// Accumulated ssh stderr (the `-v` handshake), routed off the PTY via a
    /// side FIFO. Rendered as the connect spinner's debug tail / expanded log,
    /// and scanned for the real "authenticated to" connected marker.
    debug_log: String,
    /// The part of `debug_log` only ssh itself can write: its `-E` log (a
    /// server banner lands on stderr, never here). Every security decision —
    /// `connected`, host-key fingerprint, changed-key repair target — reads
    /// this, not `debug_log`. Children without a separate log channel (mosh,
    /// test doubles) feed it from stderr, the best they have.
    ssh_log: String,
    /// Whether the child has its own ssh log channel, see `ssh_log`.
    log_channel: bool,
    /// Whether this machine's network is shared with the host (#174), read
    /// from the sidecar ssh's own `-E` log (never a PTY, never stderr, where a
    /// server's banner lands).
    net_share: NetShare,
    /// The second ssh connection that carries the proxy. Dropped with the
    /// session; on Linux it also ends with sshub however sshub ends, elsewhere
    /// everything but a SIGKILL of sshub (see [`share_net::Sidecar`]).
    sidecar: Option<share_net::Sidecar>,
    /// First host-key fingerprint seen in `debug_log` (first-wins). Cached so a
    /// hostile banner that floods the 64 KB ring buffer cannot replace the
    /// genuine KEX line with a forged `debug1: Server host key:` after eviction.
    host_key_fingerprint: Option<String>,
    /// Whether the connect screen shows the full debug log instead of the
    /// spinner + tail. Toggled by the user.
    debug_expanded: bool,
    /// True once any bytes have landed on the PTY (stdout/tty) — i.e. there is
    /// real shell/prompt content to show. Gates the connect-timeout fail-open:
    /// with `-v` debug now off the PTY, revealing a blank grid for a host that
    /// never answered is useless, so we keep the spinner until either real PTY
    /// content arrives or the child exits.
    saw_pty_bytes: bool,
    /// PTY events taken in one [`Self::drain`]; reused.
    pty_events: Vec<PtyEvent>,
    /// Active in-app mouse text selection over the grid, if any.
    pub selection: Option<Selection>,
    /// Transient "copied" toast: (message, shown_at). Rendered in a corner for
    /// a few seconds after a copy-on-select.
    pub copy_notice: Option<(String, Instant)>,
    /// Whether this session has already been told what the wheel does on the
    /// alternate screen. Said once, not on every notch.
    alt_scroll_hinted: bool,
    /// While a selection drag is held past the top/bottom edge, the view keeps
    /// scrolling on each poll tick so you can select content beyond the
    /// viewport. `(dir, col)`: dir `+1` scrolls toward newer output (bottom
    /// edge), `-1` toward older (top edge); `col` is the pointer column to keep
    /// extending the selection to. `None` = not autoscrolling.
    drag_autoscroll: Option<(i32, u16)>,
    pub key_push_identity: Option<String>,
    pub logged_exit: bool,
    pub host_name: String,
    /// Optional PTY transcript writer; closed on session end.
    log: Option<crate::session_log::SessionLogWriter>,
    /// Remaining byte allowance for terminal-query answers, and when it was
    /// last topped up. See [`REPLY_BURST_BYTES`].
    reply_tokens: usize,
    reply_refilled_at: Instant,
}

impl Session {
    /// Spawn the child on a freshly allocated PTY of `grid` = `(rows, cols)`
    /// and start the reader thread.
    ///
    /// `grid` is the size of the pane the session is shown in, chrome already
    /// subtracted by the caller: the session knows nothing about headers,
    /// footers or splits. Spawning at the destination size matters, because
    /// ssh sends it to the remote with the PTY request; a wrong first size costs
    /// the remote a `SIGWINCH` and a full repaint before anything was typed.
    pub fn spawn(
        config: SessionConfig,
        grid: (u16, u16),
        log: Option<crate::session_log::SessionLogWriter>,
    ) -> Result<Self> {
        Self::spawn_inner(config, grid, log, true)
    }

    /// Like [`Self::spawn`] but with the child's stderr merged onto the PTY
    /// instead of siphoned into the ssh `-v` debug log. Local shells need
    /// this: interactive shells print PS1 to stderr, and with the siphon
    /// the grid stays blank so the session never leaves Connecting.
    pub fn spawn_with_merged_stderr(
        config: SessionConfig,
        grid: (u16, u16),
        log: Option<crate::session_log::SessionLogWriter>,
    ) -> Result<Self> {
        Self::spawn_inner(config, grid, log, false)
    }

    /// Spawn the child on a freshly allocated PTY and start the reader thread.
    fn spawn_inner(
        config: SessionConfig,
        grid: (u16, u16),
        log: Option<crate::session_log::SessionLogWriter>,
        siphon_stderr: bool,
    ) -> Result<Self> {
        let (pty_rows, pty_cols) = floor_grid(grid);
        // OpenSSH before 8.4 ignores `SSH_ASKPASS_REQUIRE` and asks on the PTY,
        // where no answer ever arrives: leave such an ssh to the live terminal.
        let program = config.argv.first().map(String::as_str);
        let askpass_ok = program != Some("ssh") || ssh_honors_askpass_require();
        let interactive =
            askpass_ok && config.key_push_identity.is_none() && matches!(program, Some("ssh"));
        let mut auth = if interactive {
            Some(interactive_auth::InteractiveAuth::new()?)
        } else {
            None
        };

        // Prefer handing the secret to ssh via SSH_ASKPASS so the passphrase/
        // password prompt never shows on screen. Falls back to typing into the
        // PTY (below) if we can't stage it or ssh is too old to honour it.
        let mut env: Vec<(String, String)> = Vec::new();
        let mut askpass = None;
        let mut use_askpass = false;
        let mut askpass_error = None;
        if let Some(auth) = auth.as_ref() {
            env = auth.channel_env(&askpass::helper_exe()?);
            use_askpass = true;
        } else if askpass_ok && (config.pending_secret.is_some() || !config.hops.is_empty()) {
            match askpass::helper_exe() {
                Ok(exe) => {
                    let secret = config.pending_secret.as_ref();
                    if let Ok(guard) = askpass::AskpassSecret::staged(secret, &config.hops) {
                        env = guard.env(&exe);
                        askpass = Some(guard);
                        use_askpass = true;
                    }
                }
                // Worth saying out loud: without it every stored secret goes
                // undelivered and ssh reports a rejected key, which sends the
                // user looking at the server instead of at the upgrade.
                Err(e) => askpass_error = Some(e.to_string()),
            }
        }

        let runtime = PtyRuntime::spawn(&config.argv, pty_rows, pty_cols, &env, siphon_stderr)?;
        if let Some(auth) = auth.as_mut() {
            // `sh -c 'exec ssh …'` keeps the pid, so this is the destination ssh.
            auth.destination_pid = runtime.child_pid();
        }
        let parser = ParserState::new(pty_rows, pty_cols);

        let log_channel = runtime.has_log_channel();
        let display_argv = config.argv.clone();

        // Transparent auth log: say exactly how the stored secret is delivered.
        let mut diagnostics = Vec::new();
        if config.pending_secret.is_some() {
            diagnostics.push(if use_askpass {
                "auth: credential handed to ssh via SSH_ASKPASS".to_string()
            } else if let Some(e) = askpass_error.as_deref() {
                format!("auth: SSH_ASKPASS unavailable ({e}) — will type at the prompt")
            } else {
                "auth: SSH_ASKPASS unavailable — will type at the prompt".to_string()
            });
        }

        Ok(Self {
            id: SessionId::next(),
            display_name: config.display_name.clone(),
            meta: config.meta.clone(),
            phase: SessionPhase::Connecting {
                started_at: Instant::now(),
            },
            runtime,
            parser,
            history: history::SessionHistory::default(),
            local_shell: false,
            display_argv,
            pending_secret: config.pending_secret.clone(),
            secret_sent: false,
            diagnostics,
            _askpass: askpass,
            use_askpass,
            auth,
            connected: false,
            debug_log: String::new(),
            ssh_log: String::new(),
            log_channel,
            net_share: NetShare::Off,
            sidecar: None,
            host_key_fingerprint: None,
            debug_expanded: false,
            saw_pty_bytes: false,
            pty_events: Vec::new(),
            selection: None,
            copy_notice: None,
            alt_scroll_hinted: false,
            drag_autoscroll: None,
            key_push_identity: config.key_push_identity.clone(),
            logged_exit: false,
            host_name: config.host_name.clone(),
            config,
            log,
            reply_tokens: REPLY_BURST_BYTES,
            reply_refilled_at: Instant::now(),
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
        let (rows, cols) = self.parser.size();
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
        let (rows, cols) = self.parser.size();
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
        let (rows, _) = self.parser.size();
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

    /// Scroll the view by `rows` (positive = toward older output) and carry a
    /// live selection along by however far the view *actually* moved.
    ///
    /// It can move less than asked, or not at all: the top of the buffer clamps,
    /// and the alternate screen keeps no scrollback whatsoever (vt100 gives that
    /// grid zero rows). Shifting the selection by the requested amount anyway
    /// slid the highlight across text that stayed exactly where it was — inside
    /// `tmux` the wheel appeared to move the selection instead of the screen.
    pub fn scroll_with_selection(&mut self, rows: i32) {
        let before = self.parser.scrollback() as i32;
        if rows >= 0 {
            self.parser.scroll_up(rows as usize);
        } else {
            self.parser.scroll_down(rows.unsigned_abs() as usize);
        }
        let moved = self.parser.scrollback() as i32 - before;
        if moved != 0 {
            self.selection_scroll_shift(moved);
        }
    }

    /// Say once per session what a wheel notch on the alternate screen does,
    /// and name the tmux switch that makes the wheel scroll tmux's own history.
    ///
    /// It fires on the notch rather than on "we think tmux is running": from the
    /// PTY we see a byte stream, not processes, and tmux's status line — the
    /// only thing that would give it away — is the first thing people re-style.
    /// The moment of confusion is the honest trigger, and the advice holds for
    /// whatever owns the alternate screen.
    pub fn hint_alternate_scroll(&mut self) {
        if self.alt_scroll_hinted {
            return;
        }
        self.alt_scroll_hinted = true;
        self.set_copy_notice(
            "wheel \u{2192} \u{2191} keys here \u{2014} tmux scrolls with: set -g mouse on"
                .to_string(),
        );
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
        let mut events = std::mem::take(&mut self.pty_events);
        while let Some(event) = self.runtime.try_recv() {
            events.push(event);
        }
        self.handle_pty_events(&mut events);
        self.pty_events = events;
    }

    fn handle_pty_events(&mut self, events: &mut Vec<PtyEvent>) {
        // Translate every read up front: a synchronized-output frame is then
        // rasterized once per drain, not once per small read. Each read still
        // reaches vt100 on its own below, so everything checked per read sees
        // the same screen as before.
        for event in events.iter() {
            if let PtyEvent::Bytes(bytes) = event {
                self.parser.queue(bytes);
            }
        }
        let now = Instant::now();
        let mut had_bytes = false;
        let mut had_stderr = false;
        for event in events.drain(..) {
            match event {
                PtyEvent::Bytes(bytes) => {
                    self.parser.apply_next(now);
                    if !self.history_capture_allowed() {
                        self.history.input.invalidate_pending_input();
                    }
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
                    push_capped(&mut self.debug_log, &bytes);
                    if !self.log_channel {
                        self.push_ssh_log(&bytes);
                    }
                    had_stderr = true;
                }
                PtyEvent::SshLog(bytes) => {
                    push_capped(&mut self.debug_log, &bytes);
                    self.push_ssh_log(&bytes);
                    had_stderr = true;
                }
                PtyEvent::Exited(reason) => {
                    let exit_status = self.runtime.reap_child();
                    let status_str = match exit_status {
                        Some(es) => {
                            if es.success() {
                                "success".to_string()
                            } else {
                                format!("code {}", es.exit_code())
                            }
                        }
                        None => reason,
                    };
                    self.diagnostics
                        .push(format!("session: ssh exited ({status_str})"));
                    self.phase = SessionPhase::Exited {
                        status: status_str,
                        at: Instant::now(),
                    };
                }
            }
        }
        if had_bytes {
            // Secret first: both write to the same PTY, and a reply queued
            // ahead of the password would be read as part of it by whatever
            // asks for a line. Answering after leaves the reply as harmless
            // leftover input instead.
            self.maybe_send_pending_secret();
            self.answer_terminal_queries();
            self.maybe_reveal();
        }
        self.poll_net_share();
        // `debug_log` (the only input) changes only on stderr.
        if had_stderr {
            self.maybe_detect_connected();
        }
        // Safety net: reveal after the timeout even with no output at all, so a
        // session blocked on auth never hangs the connect screen forever.
        self.reveal_on_timeout();
        if self.phase.is_terminal() {
            self.stop_net_share();
            if let Some(auth) = self.auth.as_mut() {
                auth.finish(self.connected);
            }
            self.pending_secret = None;
            self.config.pending_secret = None;
            self._askpass = None;
        } else if self.connected {
            // Authenticated: the askpass listener has served its purpose, so
            // close it now instead of holding the socket until session exit.
            // Skips the poll/cancelled path below on purpose: a closed channel
            // reads as cancelled, and that would kill a healthy child.
            if let Some(auth) = self.auth.as_mut() {
                auth.finish(true);
            }
            // The PTY-typed copy must go too: with askpass active the secret
            // is delivered invisibly and `maybe_send_pending_secret` never
            // fires, so without this the plaintext lingers after connect.
            self.pending_secret = None;
            // Duplicating a tab must resolve credentials again, not retain a
            // second long-lived copy in SessionConfig.
            self.config.pending_secret = None;
        } else if let Some(auth) = self.auth.as_mut() {
            auth.poll(&mut self.pending_secret);
            if auth.cancelled() {
                self.cancel_authentication();
            }
            // Duplicating a tab must resolve credentials again, not retain a
            // second long-lived copy in SessionConfig.
            self.config.pending_secret = None;
        }
    }

    /// Close the helper first, then kill the child; keep the ordinary failure screen.
    pub fn cancel_authentication(&mut self) {
        if let Some(auth) = self.auth.as_mut() {
            auth.cancel();
        }
        self.pending_secret = None;
        self.config.pending_secret = None;
        self.runtime.terminate_child();
        self.phase = SessionPhase::Exited {
            status: "authentication cancelled".into(),
            at: Instant::now(),
        };
    }

    /// Write the emulator's answers to terminal status queries back into the
    /// PTY. Driven from [`Self::drain`] rather than from the frame, because
    /// unlike a clipboard write there is nothing to decide: an application
    /// waiting on a reply blocks whether or not its tab is the visible one.
    fn answer_terminal_queries(&mut self) {
        let replies = self.parser.take_replies();
        if replies.is_empty() {
            return;
        }
        let now = Instant::now();
        let refill = (now.duration_since(self.reply_refilled_at).as_millis() as usize)
            .saturating_mul(REPLY_BYTES_PER_SEC)
            / 1000;
        if refill > 0 {
            self.reply_tokens = self
                .reply_tokens
                .saturating_add(refill)
                .min(REPLY_BURST_BYTES);
            self.reply_refilled_at = now;
        }
        // Whole batch or nothing: half an escape sequence in the remote's input
        // is worse than a query left unanswered.
        if replies.len() > self.reply_tokens {
            return;
        }
        self.reply_tokens -= replies.len();
        // Neither an over-budget batch nor a failed write says anything out
        // loud. The application that asked reports its own timeout, which tells
        // the user more than a line in the SSH log would — and a diagnostic here
        // would be pushed on every frame for as long as the flood lasts.
        let _ = self.runtime.write(&replies);
    }

    /// Throw away whatever the PTY asked us to copy, including the drop
    /// counters. Used for every session that is not the visible one this
    /// frame, so a background tab can neither relay now nor replay later.
    pub(crate) fn discard_clipboard_writes(&mut self) {
        let _ = self.parser.take_clipboard_writes();
        let _ = self.parser.take_clipboard_drops();
    }

    /// Forward OSC 52 clipboard writes emitted by applications inside the PTY
    /// to the terminal hosting sshub. Without this the sequence dies in our
    /// vt100 parser: a nested multiplexer (herdr, tmux) or an editor with
    /// `clipboard=osc52` appears to copy — the selection even highlights —
    /// but nothing ever reaches the system clipboard.
    ///
    /// Only the session the user is actually looking at may relay; that
    /// decision belongs to the frame, so this is driven from
    /// [`crate::App::relay_visible_session_clipboard`] and every other session
    /// discards instead. Relaying means the remote side can write to the local
    /// clipboard, so the outcome is always visible: a clipboard change the user
    /// didn't ask for is worth noticing, and so is one that was thrown away.
    ///
    /// The relay path itself never puts the payload into the notice, the
    /// diagnostics or the session log — but the OSC 52 sequence travels in the
    /// raw PTY stream, so an enabled session log can already contain it. This
    /// neither adds it nor removes it.
    ///
    /// The sink is injectable because the real one writes to the process's
    /// stdout, which the test harness does *not* capture (it only intercepts
    /// the `print!` macros), so exercising it under `cargo test` would clobber
    /// the clipboard of whoever ran the suite.
    pub(crate) fn relay_clipboard_writes_with(
        &mut self,
        mut emit: impl FnMut(&str) -> std::io::Result<()>,
    ) {
        let mut relayed = None;
        for payload in self.parser.take_clipboard_writes() {
            match emit(&payload) {
                // Several writes in one tick collapse into one notice (the last
                // one wins), so a copy loop can't flood the corner of the screen.
                Ok(()) => relayed = Some(parser::decoded_len(payload.as_bytes())),
                // Prefixed `session:` so the event loop's connected-session
                // filter keeps it — see [`keep_diagnostic`].
                Err(e) => self
                    .diagnostics
                    .push(format!("session: clipboard relay failed: {e}")),
            }
        }
        let drops = self.parser.take_clipboard_drops();
        if let Some(msg) = clipboard_notice(relayed, drops) {
            self.set_copy_notice(msg);
        }
    }

    /// Reveal the live terminal once the connect timeout elapses — but only if
    /// the host is there: real PTY content arrived (a prompt/banner/host-key
    /// question we might have failed to auto-answer), or ssh authenticated. A
    /// startup command may print nothing at all, and an authenticated session
    /// held behind the spinner looked like a connect that never finished
    /// (#171). With `-v` debug siphoned off the PTY, a host that never answered
    /// has a blank grid, so failing open there would just hide the spinner +
    /// debug tail behind emptiness. Those sessions stay on the spinner until
    /// the child exits.
    fn reveal_on_timeout(&mut self) {
        if let SessionPhase::Connecting { started_at } = self.phase {
            if started_at.elapsed() >= REVEAL_TIMEOUT && (self.saw_pty_bytes || self.connected) {
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
    ///
    /// Mosh does not run `ssh -v`, so it never produces this marker — and
    /// `Running` alone (a timeout reveal counts) is not post-auth evidence.
    /// Mosh stays unauthenticated until real connected evidence arrives.
    fn maybe_detect_connected(&mut self) {
        if self.connected {
            return;
        }
        let text = self.ssh_log.to_ascii_lowercase();
        if CONNECTED_NEEDLES.iter().any(|n| text.contains(n)) {
            self.connected = true;
            // Authentication consumed whatever was typed pre-auth (a typed
            // password, never a command) and a fresh shell line begins: drop
            // the stale uncertainty so the first post-auth command records.
            // Transition-only — later output must not wipe a live input line.
            self.history.input.reset();
        }
    }

    /// Record bytes only ssh could have written (see `ssh_log`); the host-key
    /// fingerprint is cached first-wins so a flood cannot evict the real one.
    fn push_ssh_log(&mut self, bytes: &[u8]) {
        push_capped(&mut self.ssh_log, bytes);
        if self.host_key_fingerprint.is_none() {
            self.host_key_fingerprint =
                crate::known_hosts::host_key_fingerprint_from_log(&self.ssh_log);
        }
    }

    /// Where sharing this machine's network with the host stands.
    pub fn net_share(&self) -> NetShare {
        self.net_share
    }

    /// Whether this tab can share the network: a plain `ssh` session. Mosh,
    /// a local shell and a key push cannot.
    pub fn can_share_net(&self) -> bool {
        !self.local_shell
            && self.key_push_identity.is_none()
            && self.config.argv.first().map(String::as_str) == Some("ssh")
    }

    /// Start sharing: a second `ssh -N` to the same host that only carries the
    /// proxy (see [`share_net::sidecar_argv`]). The session is not touched.
    /// `permit_remote_open` is [`share_net::PERMIT_REMOTE_OPEN`] outside tests.
    pub fn start_net_share(
        &mut self,
        secret: Option<&PendingSecret>,
        hops: &[hop_auth::HopSecret],
        permit_remote_open: &str,
    ) -> Result<()> {
        anyhow::ensure!(
            self.can_share_net(),
            "only ssh sessions can share the network"
        );
        self.stop_net_share();
        let argv = share_net::sidecar_argv(
            &self.config.argv,
            secret.is_some() || !hops.is_empty(),
            permit_remote_open,
        )
        .ok_or_else(|| anyhow::anyhow!("only ssh sessions can share the network"))?;
        self.sidecar = Some(share_net::Sidecar::spawn(argv, secret, hops)?);
        self.net_share = NetShare::Starting;
        Ok(())
    }

    /// Stop sharing: the sidecar and its process group are gone when this
    /// returns.
    pub fn stop_net_share(&mut self) {
        self.sidecar = None;
        self.net_share = NetShare::Off;
    }

    /// Pid of the proxy ssh, for tests.
    #[cfg(test)]
    fn sidecar_pid(&self) -> Option<u32> {
        self.sidecar.as_ref().map(share_net::Sidecar::pid)
    }

    /// Read the sidecar's log: the allocated port, or why it ended. A sidecar
    /// that ends on its own never leaves a stale port behind.
    fn poll_net_share(&mut self) {
        let Some(sidecar) = self.sidecar.as_mut() else {
            return;
        };
        let (lines, ended) = sidecar.poll();
        for line in lines {
            match share_net::parse_log_line(&line) {
                Some(NetShare::Port(port)) if self.net_share == NetShare::Starting => {
                    self.net_share = NetShare::Port(port);
                    self.diagnostics.push(share_net::hint(port));
                }
                Some(NetShare::Failed) => self.net_share = NetShare::Failed,
                _ => {}
            }
        }
        if ended {
            let reason = sidecar.failure_reason();
            self.sidecar = None;
            self.net_share = NetShare::Failed;
            self.diagnostics.push(format!(
                "session: sharing this machine's network failed: {reason}"
            ));
        } else if self.net_share == NetShare::Failed {
            // The server refused the forward but ssh is still up: end it.
            let reason = sidecar.failure_reason();
            self.sidecar = None;
            self.diagnostics.push(format!(
                "session: sharing this machine's network failed: {reason}"
            ));
        }
    }

    /// The accumulated ssh `-v` debug output (host-key search, kex, auth).
    pub fn debug_log(&self) -> &str {
        &self.debug_log
    }

    /// Cached first host-key fingerprint from the `-v` log, if any.
    pub fn host_key_fingerprint(&self) -> Option<&str> {
        self.host_key_fingerprint.as_deref()
    }

    /// Whether `spec` (ssh's lookup name: `host` or `[host]:port`) is the host
    /// this session connects to: its address, a host argument of the command,
    /// or an explicit `HostKeyAlias`.
    fn names_this_session(&self, spec: &str) -> bool {
        let host = spec
            .strip_prefix('[')
            .and_then(|rest| rest.split_once("]:"))
            .map_or(spec, |(host, _)| host);
        self.meta.address.as_deref() == Some(host)
            || self.config.argv.iter().skip(1).any(|arg| {
                arg.rsplit_once('@').map_or(arg.as_str(), |(_, h)| h) == host
                    || arg
                        .strip_prefix("HostKeyAlias=")
                        .or_else(|| arg.strip_prefix("-oHostKeyAlias="))
                        == Some(spec)
            })
    }

    /// True when ssh refused because the server's host key CHANGED versus
    /// known_hosts (a mismatch), as opposed to a merely-unknown first-time host.
    /// This is the case worth offering to accept — a changed key can be a
    /// legitimate server rebuild or a MITM, so the user must opt in.
    pub fn host_key_changed(&self) -> bool {
        let log = self.ssh_log.to_ascii_lowercase();
        log.contains("host identification has changed")
            || (log.contains("host key for") && log.contains("has changed"))
    }

    /// Use OpenSSH's actual offending file and lookup name, not ~/.ssh defaults
    /// or the display address (HostKeyAlias and custom ports can differ).
    pub fn changed_key_target(&self) -> Option<(String, std::path::PathBuf)> {
        let mut specs = self.ssh_log.lines().filter_map(|line| {
            line.strip_prefix("Host key for ")?
                .split_once(" has changed")
                .map(|(host, _)| host)
        });
        let spec = specs.next()?;
        // ssh's stderr also carries whatever a server's banner prints, so the
        // name must be one this session connects to, not just any host.
        if !self.names_this_session(spec) {
            return None;
        }
        if spec.is_empty()
            || spec.starts_with('-')
            || spec.contains([',', '*', '?'])
            || spec.chars().any(|c| c.is_whitespace() || c.is_control())
            || specs.any(|other| other != spec)
        {
            return None;
        }
        let mut files = self.ssh_log.lines().filter_map(|line| {
            let line = line.strip_prefix("Offending ")?;
            let (_, location) = line.split_once(" key in ")?;
            let (file, row) = location.rsplit_once(':')?;
            if row.parse::<usize>().ok()? == 0 {
                return None;
            }
            let path = std::path::PathBuf::from(file);
            path.is_absolute().then_some(path)
        });
        let file = files.next()?;
        if files.any(|other| other != file) {
            return None;
        }
        Some((spec.to_owned(), file))
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
            PendingSecret::Password(_) => (
                auth_prompt::PromptClass::classify(&lower) == auth_prompt::PromptClass::Password
                    && ends_with_prompt(&lower, PASSWORD_NEEDLES),
                "password",
            ),
            PendingSecret::Passphrase(_) => (
                auth_prompt::PromptClass::classify(&lower) == auth_prompt::PromptClass::Passphrase
                    && ends_with_prompt(&lower, PASSPHRASE_NEEDLES),
                "passphrase",
            ),
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

    /// Resize the PTY and the parser grid to `grid` = `(rows, cols)`, the size
    /// of the pane the session is shown in.
    ///
    /// A no-op when the size is unchanged, so callers may call it every frame:
    /// the kernel would not signal an unchanged size either, but the parser's
    /// held sync frame would be dropped for nothing. An exited session is
    /// never resized: no program is left to repaint, and a narrower grid cuts
    /// its frozen screen for good (vt100 does not reflow columns).
    pub fn resize(&mut self, grid: (u16, u16)) {
        let (rows, cols) = floor_grid(grid);
        if self.phase.is_terminal() || self.parser.size() == (rows, cols) {
            return;
        }
        self.parser.set_size(rows, cols);
        let _ = self.runtime.resize(rows, cols);
    }

    /// Bare `Running` gate for explicit snippet insertion. `Running` may be a
    /// timeout reveal rather than proof of authentication, but insertion only
    /// ever writes bytes the user explicitly chose (a picker selection), so
    /// it is safe to offer on mosh sessions that never produce the ssh `-v`
    /// connected marker. History capture and ghost text stay on the honest
    /// `is_live_authenticated` signal.
    pub(crate) fn is_running(&self) -> bool {
        matches!(self.phase, SessionPhase::Running { .. }) && !self.runtime.is_closed()
    }

    /// Running may be a timeout reveal, not proof that SSH authenticated.
    pub(crate) fn is_live_authenticated(&self) -> bool {
        matches!(self.phase, SessionPhase::Running { .. })
            && (self.is_connected() || self.local_shell)
            && !self.runtime.is_closed()
    }

    /// Conservative prompt-context guard for best-effort history capture.
    pub fn history_capture_allowed(&self) -> bool {
        let screen = self.parser.screen();
        let line = line_before_cursor(screen).to_ascii_lowercase();
        self.is_live_authenticated()
            && !screen.alternate_screen()
            && self.parser.scrollback() == 0
            && ![
                "password",
                "passphrase",
                "verification code",
                "one-time",
                "token",
                "secret",
            ]
            .iter()
            .any(|needle| line.contains(needle))
    }

    /// Observe only explicit local input; auth responses and terminal replies
    /// deliberately bypass this path.
    pub fn observe_key(&mut self, key: crossterm::event::KeyEvent) -> Option<String> {
        let allowed = self.history_capture_allowed();
        self.history.input.key(key, allowed)
    }

    pub fn observe_paste(&mut self, text: &str) {
        let allowed = self.history_capture_allowed();
        self.history.input.paste(text, allowed);
    }

    /// Write raw bytes (already-encoded keystroke) to the PTY.
    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        // Channel authentication owns all user input until the actual SSH
        // success marker, including the gaps between helper invocations.
        // Emulator protocol replies use runtime.write directly and still flow.
        if self.auth.is_some() && !self.connected {
            return Ok(());
        }
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
        if self.auth.is_some() && !self.connected {
            return Ok(());
        }
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
    let text = sanitize_paste(text);
    if bracketed {
        let mut buf = Vec::with_capacity(text.len() + 12);
        buf.extend_from_slice(b"\x1b[200~");
        buf.extend_from_slice(&text);
        buf.extend_from_slice(b"\x1b[201~");
        buf
    } else {
        text
    }
}

/// Drop what could end the paste early or act as a command: ESC (so clipboard
/// text cannot carry `ESC[201~` and run the rest as typed input), the other C0
/// controls and DEL, and the C1 controls (UTF-8 `C2 80..9F`). Tab, LF and CR
/// stay. Terminals do the same to a bracketed paste.
fn sanitize_paste(text: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    let mut bytes = text.iter().copied().peekable();
    while let Some(b) = bytes.next() {
        match b {
            b'\t' | b'\n' | b'\r' => out.push(b),
            0x00..=0x1f | 0x7f => {}
            0xc2 if matches!(bytes.peek(), Some(0x80..=0x9f)) => {
                bytes.next();
            }
            _ => out.push(b),
        }
    }
    out
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

/// `(major, minor)` of an `ssh -V` banner (`OpenSSH_8.2p1 Ubuntu-4…`).
fn openssh_version(banner: &str) -> Option<(u32, u32)> {
    let rest = banner.split("OpenSSH_").nth(1)?;
    let (major, rest) = rest.split_once('.')?;
    let minor: String = rest.chars().take_while(char::is_ascii_digit).collect();
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// Whether the installed `ssh` honours `SSH_ASKPASS_REQUIRE` (OpenSSH 8.4+).
/// An older one asks on the PTY and never calls the helper, so no stored or
/// typed answer can reach it. Unknown versions count as supported.
fn ssh_honors_askpass_require() -> bool {
    static HONORED: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
        let Ok(out) = std::process::Command::new("ssh").arg("-V").output() else {
            return true;
        };
        let banner = String::from_utf8_lossy(&out.stderr).into_owned()
            + &String::from_utf8_lossy(&out.stdout);
        banner_honors_askpass_require(&banner)
    });
    *HONORED
}

/// `SSH_ASKPASS_REQUIRE` arrived in OpenSSH 8.4 (release notes: "ssh(1),
/// ssh-add(1): add SSH_ASKPASS_REQUIRE"; `man ssh` of 8.3 does not list it).
fn banner_honors_askpass_require(banner: &str) -> bool {
    openssh_version(banner).is_none_or(|version| version >= (8, 4))
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
/// case only. The mux entry covers ControlMaster slaves: auth happened on
/// the master, so the slave never prints `Authenticated to` — but
/// `mux_client_request_session` proves the master accepted our channel and
/// the PTY goes straight to a live shell (verified against OpenSSH_10.5p1).
const CONNECTED_NEEDLES: &[&str] = &[
    "authenticated to ",
    "authenticated (",
    "mux_client_request_session",
];

/// How an embedded ssh is asked for the debug output scanned for
/// [`CONNECTED_NEEDLES`]. Not `-v`: ssh hands `-v` on to every ProxyJump hop,
/// whose own `Authenticated to …` reaches stderr. The embedded ssh logs to
/// its own `-E` FIFO (see `pty::StderrFifo`) and only that stream is scanned,
/// so neither a hop's stderr nor a server banner can fake the marker.
pub(crate) const SSH_DEBUG_ARGS: [&str; 2] = ["-o", "LogLevel=DEBUG1"];

/// Append `bytes` to a debug buffer, dropping old output past
/// [`DEBUG_LOG_CAP`] from the front.
fn push_capped(buf: &mut String, bytes: &[u8]) {
    buf.push_str(&String::from_utf8_lossy(bytes));
    if buf.len() > DEBUG_LOG_CAP {
        let mut cut = buf.len() - DEBUG_LOG_CAP;
        while cut > 0 && !buf.is_char_boundary(cut) {
            cut -= 1;
        }
        buf.drain(..cut);
    }
}

/// Cap on the retained ssh `-v` debug buffer (bytes). Old output past this is
/// dropped from the front so a long session can't grow it without bound.
const DEBUG_LOG_CAP: usize = 64 * 1024;

/// Token bucket over the bytes of terminal-query answers we write back into the
/// PTY, as a burst allowance and a sustained refill rate. Real applications ask
/// a handful of times per keystroke, so neither is ever felt; the sustained rate
/// is a fraction of what a person typing already writes into the same PTY.
///
/// It exists because the remote decides how often it asks, and the write queue
/// it answers into is shared with the user's keystrokes. Unthrottled, a host in
/// a query loop fills the PTY write queue on its own and
/// the typing gets dropped instead. Keeping the answers to a trickle leaves the
/// queue for the person at the keyboard.
///
/// What it does *not* do is prevent the freeze: measured against the blocking
/// write this bucket only moved the wedge from under a second to ~80 s
/// (4096 + 256/s reaching the master's ~20 KiB limit at t≈64 s). That is what
/// the writer thread is for.
const REPLY_BURST_BYTES: usize = 4096;
const REPLY_BYTES_PER_SEC: usize = 256;

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
    let (cursor_row, cursor_col) = screen.cursor_position();
    let row = cursor_row.min(rows.get() - 1);
    let mut out = String::new();
    for col in 0..cursor_col {
        if let Some(cell) = screen.cell(row, col) {
            if cell.has_contents() {
                out.push_str(cell.contents());
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
    let (cursor_row, _) = screen.cursor_position();
    let last = cursor_row.min(rows.get() - 1);
    let start = last.saturating_sub(3);
    let mut out = String::new();
    for row in start..=last {
        for col in 0..cols.get() {
            if let Some(cell) = screen.cell(row, col) {
                if cell.has_contents() {
                    out.push_str(cell.contents());
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

/// The one line shown after a batch of PTY clipboard writes, or `None` when
/// nothing worth reporting happened.
///
/// A batch that both copied and dropped has to say both: reporting only the
/// success would hide the loss behind good news.
fn clipboard_notice(relayed_bytes: Option<usize>, drops: parser::ClipboardDrops) -> Option<String> {
    let dropped = match (drops.oversize, drops.queue_full) {
        (0, 0) => None,
        (n, 0) => Some(format!("{n} oversized {}", plural_writes(n))),
        (0, n) => Some(format!("{n} over the queue limit")),
        (a, b) => Some(format!(
            "{a} oversized {}, {b} over the queue limit",
            plural_writes(a)
        )),
    };
    match (relayed_bytes, dropped) {
        (Some(bytes), None) => Some(format!("remote copied {bytes} bytes to clipboard")),
        (Some(bytes), Some(d)) => Some(format!(
            "remote copied {bytes} bytes to clipboard; dropped {d}"
        )),
        (None, Some(d)) => Some(format!("clipboard relay dropped {d}")),
        (None, None) => None,
    }
}

fn plural_writes(n: usize) -> &'static str {
    if n == 1 {
        "write"
    } else {
        "writes"
    }
}

/// Should a session diagnostic reach the SSH log panel?
///
/// Before a session is connected everything is interesting — it is the record
/// of a handshake that may be failing. Once connected the loop would otherwise
/// bury the panel under routine chatter, so only lines that stay relevant are
/// kept: anything the session itself reports (`session:` — exits, and the
/// clipboard relay failing) plus unresolved auth problems.
///
/// The event loop calls exactly this function, so a diagnostic can be tested
/// against the contract that actually governs it.
pub(crate) fn keep_diagnostic(line: &str, connected: bool) -> bool {
    !connected || line.starts_with("session:") || line.starts_with("auth: could not")
}

#[cfg(test)]
mod diagnostic_filter_tests {
    use super::keep_diagnostic;

    #[test]
    fn everything_survives_before_connect() {
        assert!(keep_diagnostic("auth: trying key", false));
    }

    #[test]
    fn connected_keeps_only_session_and_auth_failures() {
        assert!(keep_diagnostic("session: ssh exited (code 1)", true));
        assert!(keep_diagnostic("auth: could not read key", true));
        assert!(!keep_diagnostic("auth: trying key", true));
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

    #[test]
    fn paste_cannot_smuggle_escape_or_control_bytes() {
        // A clipboard carrying the end marker would otherwise end the bracket
        // early and run the tail as typed keys.
        let evil = "a\x1b[201~rm -rf ~\x07\x7f\u{9b}2J\tb\r\nc é".as_bytes();
        let expect = b"a[201~rm -rf ~2J\tb\r\nc \xc3\xa9".to_vec();
        assert_eq!(sanitize_paste(evil), expect);
        let wrapped = encode_paste(evil, true);
        assert_eq!(
            wrapped.iter().filter(|&&b| b == 0x1b).count(),
            2,
            "only our two markers"
        );
    }
}

#[cfg(test)]
mod prompt_tests {
    use super::*;

    #[test]
    fn auth_changed_key_parser_requires_unambiguous_exact_target() {
        let config = SessionConfig {
            hops: Vec::new(),
            argv: vec!["sh".into(), "-c".into(), "sleep 30".into()],
            display_name: "offline".into(),
            meta: SessionMeta {
                address: Some("alias".into()),
                port: Some(2222),
                ..SessionMeta::default()
            },
            pending_secret: None,
            key_push_identity: None,
            host_name: "offline".into(),
        };
        let mut session = Session::spawn(config, (24, 80), None).unwrap();
        session.ssh_log = "Offending ED25519 key in /tmp/custom known_hosts:7\nHost key for [alias]:2222 has changed and you have requested strict checking.\n".into();
        assert_eq!(
            session.changed_key_target(),
            Some(("[alias]:2222".into(), "/tmp/custom known_hosts".into()))
        );
        for log in [
            "Offending ED25519 key in relative:7\nHost key for alias has changed\n",
            "Offending ED25519 key in /tmp/keys:0\nHost key for alias has changed\n",
            "Offending ED25519 key in /tmp/keys:1\nHost key for alias has changed\nHost key for other has changed\n",
            "Offending ED25519 key in /tmp/keys:1\nOffending RSA key in /tmp/other:2\nHost key for alias has changed\n",
            // A banner naming another host must not aim the repair at that host.
            "Offending ED25519 key in /home/u/.ssh/known_hosts:1\nHost key for github.com has changed and you have requested strict checking.\n",
        ] {
            session.ssh_log = log.into();
            assert!(session.changed_key_target().is_none(), "ambiguous or invalid target must not be repaired");
        }
    }

    #[test]
    fn askpass_require_needs_openssh_8_4() {
        assert!(!banner_honors_askpass_require("OpenSSH_8.3p1 Ubuntu"));
        assert!(!banner_honors_askpass_require("OpenSSH_7.9p1, OpenSSL"));
        assert!(banner_honors_askpass_require("OpenSSH_8.4p1, OpenSSL"));
        assert!(banner_honors_askpass_require("OpenSSH_10.5p1, OpenSSL"));
        // Not OpenSSH, or unreadable: do not take the helper away.
        assert!(banner_honors_askpass_require("dropbear"));
    }

    #[test]
    fn openssh_versions_are_read_from_the_ssh_v_banner() {
        assert_eq!(
            openssh_version("OpenSSH_8.2p1 Ubuntu-4ubuntu0.5, OpenSSL 1.1.1f"),
            Some((8, 2))
        );
        assert_eq!(
            openssh_version("OpenSSH_10.0p2 Debian-7, OpenSSL 3.5.1"),
            Some((10, 0))
        );
        assert_eq!(openssh_version("OpenSSH_for_Windows_9.5p1"), None);
        assert_eq!(openssh_version("dropbear"), None);
    }

    #[test]
    fn auth_input_guard_preserves_terminal_protocol_replies() {
        // The DSR 5 answer (`ESC[0n`) is a fixed 4 bytes, so `dd count=4` is
        // exact, not a guess — deliberately no `-icanon min/time` bound here
        // (cf. the variable-length CPR helper below): a child-side read
        // timeout would turn a merely late reply into a silent early exit
        // and an empty `od` print, i.e. a false red. The poll deadline below
        // is the only bound, and it reports screen state so the next failure
        // says whether the query, the reply, or the child went missing.
        // Explicit `/bin/sh`: bare `sh` resolves via PATH (bash on macOS,
        // dash on Ubuntu) — the script is POSIX either way, but the spawn
        // must not depend on the runner's PATH.
        let config = SessionConfig {
            hops: Vec::new(),
            argv: vec!["/bin/sh".into(), "-c".into(), "stty raw -echo; printf '\\033[5n'; dd bs=1 count=4 2>/dev/null | od -An -tx1; sleep 30".into()],
            display_name: "offline".into(), meta: SessionMeta::default(),
            pending_secret: None, key_push_identity: None, host_name: "offline".into(),
        };
        let mut session = Session::spawn(config, (24, 80), None).unwrap();
        session.auth = Some(interactive_auth::InteractiveAuth::new().unwrap());
        // 10s, not 3s: CI runners stall PTY spawn + shell startup far longer
        // than a local box (cf. the 10s `tick_until` in app/auth.rs and the
        // macOS-conditional watcher timeouts). The contract is unchanged —
        // the query MUST be answered.
        let deadline = Instant::now() + Duration::from_secs(10);
        // Match the hex bytes with the spacing squeezed out: GNU `od` prints
        // ` 1b 5b 30 6e`, BSD `od` (macOS) pads to `  1b  5b  30  6e`, so a
        // literal single-spaced needle failed every macOS run while Linux
        // passed. Drain before testing, too — the pre-drain sample could
        // assert on bytes it had not read yet.
        let answered = |s: &Session| {
            s.parser
                .screen()
                .contents()
                .split_whitespace()
                .collect::<String>()
                .contains("1b5b306e")
        };
        loop {
            session.drain();
            if answered(&session) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "terminal status query was not answered, tail: {:?}, connected: {}, terminal: {}",
                session.screen_tail_snippet(),
                session.is_connected(),
                session.phase.is_terminal(),
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!session.is_connected());
    }

    #[test]
    fn mosh_running_alone_does_not_latch_connected() {
        let config = SessionConfig {
            hops: Vec::new(),
            argv: vec!["sh".into(), "-c".into(), "sleep 120".into()],
            display_name: "mosh-host".into(),
            meta: SessionMeta::default(),
            pending_secret: None,
            key_push_identity: None,
            host_name: "mosh-host".into(),
        };
        let mut s = Session::spawn(config, (10, 40), None).unwrap();
        s.config.argv = vec!["mosh".into(), "host".into()];
        s.phase = SessionPhase::Running {
            started_at: Instant::now(),
        };
        assert!(!s.is_connected());
        s.drain();
        // Running alone is not post-auth evidence (mosh has no ssh -v
        // marker): connected stays false until real evidence arrives.
        assert!(!s.is_connected());
    }

    #[test]
    fn connected_session_tears_down_askpass_channel_without_cancelling() {
        // A connected session must not hold its askpass listener socket until
        // exit — but the closed channel must not read as cancellation and kill
        // the child on the next drain.
        let config = SessionConfig {
            hops: Vec::new(),
            argv: vec!["sh".into(), "-c".into(), "sleep 120".into()],
            display_name: "t".into(),
            meta: SessionMeta::default(),
            pending_secret: None,
            key_push_identity: None,
            host_name: "t".into(),
        };
        let mut s = Session::spawn(config, (24, 80), None).unwrap();
        s.auth = Some(interactive_auth::InteractiveAuth::new().unwrap());
        assert!(s.auth.as_ref().unwrap().channel_open());
        let env = s
            .auth
            .as_ref()
            .unwrap()
            .channel_env(std::path::Path::new("unused"));
        let get = |name: &str| env.iter().find(|(key, _)| key == name).unwrap().1.clone();
        let path = std::path::PathBuf::from(get(askpass_channel::SOCKET_ENV));
        let token = get(askpass_channel::TOKEN_ENV);
        s.connected = true;
        s.drain();
        let auth = s.auth.as_ref().unwrap();
        assert!(
            !auth.channel_open(),
            "askpass channel must close once connected"
        );
        assert!(auth.channel_env(std::path::Path::new("unused")).is_empty());
        assert!(askpass_channel::request_answer(
            &path,
            &token,
            "Password:",
            None,
            Duration::from_secs(2),
        )
        .is_err());
        assert!(
            !matches!(s.phase, SessionPhase::Exited { .. }),
            "a connected session must not be cancelled"
        );
        // A second drain must not cancel either: the closed channel still reads
        // as `cancelled`, so the guard has to hold every time.
        s.drain();
        assert!(!matches!(s.phase, SessionPhase::Exited { .. }));
    }

    #[test]
    fn stderr_is_siphoned_off_the_pty() {
        // stdout must land on the PTY grid; stderr must be routed through the
        // side FIFO into debug_log — never onto the grid.
        let config = SessionConfig {
            hops: Vec::new(),
            argv: vec![
                "sh".into(),
                "-c".into(),
                "printf OUT_MARKER; printf ERR_MARKER 1>&2".into(),
            ],
            display_name: "t".into(),
            meta: SessionMeta::default(),
            pending_secret: None,
            key_push_identity: None,
            host_name: "t".into(),
        };
        let mut s = Session::spawn(config, (24, 80), None).unwrap();

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
    fn mosh_running_without_post_auth_evidence_rejects_history_capture() {
        // Running alone is not post-auth evidence for mosh: history capture
        // must stay gated until trustworthy connected evidence arrives.
        let config = SessionConfig {
            hops: Vec::new(),
            argv: vec!["sh".into(), "-c".into(), "sleep 120".into()],
            display_name: "mosh-host".into(),
            meta: SessionMeta::default(),
            pending_secret: None,
            key_push_identity: None,
            host_name: "mosh-host".into(),
        };
        let mut s = Session::spawn(config, (10, 40), None).unwrap();
        s.config.argv = vec!["mosh".into(), "host".into()];
        s.phase = SessionPhase::Running {
            started_at: Instant::now(),
        };
        s.drain();
        assert!(!s.is_connected());
        assert!(!s.is_live_authenticated());
        assert!(!s.history_capture_allowed());
        // Independent oracle: the history store must hold exactly 0 rows. Drive
        // a full command through `observe_key` — every keystroke must return
        // the app layer's "do not record" signal (`record_session_command`
        // early-returns on `None`), so no candidate can ever reach the store.
        assert!(s.history.entries.is_empty());
        for ch in "echo hi".chars() {
            let key = crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(ch),
                crossterm::event::KeyModifiers::NONE,
            );
            assert!(s.observe_key(key).is_none());
        }
        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        assert!(s.observe_key(enter).is_none());
        assert_eq!(
            s.history.entries.len(),
            0,
            "pre-auth keystrokes must leave exactly 0 history rows"
        );
    }

    #[test]
    fn mux_slave_latches_connected_on_master_session_marker() {
        // Oracle: real OpenSSH_10.5p1 mux-slave stderr, captured against a
        // tempdir sshd with the slave reusing the master via ControlMaster
        // auto. The slave never prints `Authenticated to` — auth happened
        // on the master — but `mux_client_request_session` proves the
        // master accepted our channel, so the PTY goes straight to a live
        // shell with no prompt to gate on. Without the needle the latch
        // never fires and suggestions stay dark on a connected session.
        if std::process::Command::new("sh")
            .arg("-c")
            .arg("exit 0")
            .output()
            .is_err()
        {
            eprintln!("skipping: no sh binary");
            return;
        }
        let config = SessionConfig {
            hops: Vec::new(),
            argv: vec![
                "sh".into(),
                "-c".into(),
                "printf 'debug1: mux_client_request_session: master session id: 2\\n' >&2; exec sleep 30".into(),
            ],
            display_name: "mux-slave".into(),
            meta: SessionMeta::default(),
            pending_secret: None,
            key_push_identity: None,
            host_name: "mux-slave".into(),
        };
        let mut s = Session::spawn(config, (10, 40), None).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            s.drain();
            if s.is_connected() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "mux marker never latched connected, debug: {:?}",
                s.debug_log()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_server_banner_cannot_latch_connected() {
        // The pre-auth banner reaches ssh's stderr verbatim; the real `-E` log
        // is a different stream. A banner reading "Authenticated to …" (and
        // forging a host-key-changed block) must change nothing; the genuine
        // log line must.
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("ssh");
        std::fs::write(
            &fake,
            "#!/bin/sh\n[ \"$1\" = -E ] || exit 3\n\
             printf 'Authenticated to evil ([1.2.3.4]:22)\\nHost key for victim has changed\\n' >&2\n\
             sleep 1\n\
             printf 'Authenticated to real ([1.2.3.4]:22) using \"password\".\\n' > \"$2\"\n\
             exec sleep 30\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let config = SessionConfig {
            hops: Vec::new(),
            argv: vec![fake.to_string_lossy().into_owned(), "real".into()],
            display_name: "banner".into(),
            meta: SessionMeta::default(),
            pending_secret: None,
            key_push_identity: None,
            host_name: "banner".into(),
        };
        let mut s = Session::spawn(config, (10, 40), None).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !s.debug_log().contains("Authenticated to evil") {
            s.drain();
            assert!(std::time::Instant::now() < deadline, "banner never arrived");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!s.is_connected(), "banner text must not latch connected");
        assert!(
            !s.host_key_changed(),
            "banner text must not forge a host-key change"
        );
        while !s.is_connected() {
            s.drain();
            assert!(
                std::time::Instant::now() < deadline,
                "the real log line never latched connected: {:?}",
                s.debug_log()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn local_shell_with_merged_stderr_reveals_on_its_prompt() {
        // Oracle: a real interactive `/bin/sh` on a real PTY. POSIX shells
        // print PS1 to STDERR, not stdout. `open_local_shell` spawns
        // exactly this (`local_shell_argv` yields `vec![shell]` with no
        // extra args) via `spawn_with_merged_stderr`, which leaves stderr
        // on the PTY instead of siphoning it into the ssh `-v` debug log.
        // The prompt is the only output an idle shell ever produces: with
        // the siphon the grid stays blank and both reveal paths (which
        // need grid bytes) park the tab on the spinner forever.
        if std::process::Command::new("sh")
            .arg("-c")
            .arg("exit 0")
            .output()
            .is_err()
        {
            eprintln!("skipping: no sh binary");
            return;
        }
        let config = SessionConfig {
            hops: Vec::new(),
            argv: vec!["sh".into()],
            display_name: "local".into(),
            meta: SessionMeta::default(),
            pending_secret: None,
            key_push_identity: None,
            host_name: "local".into(),
        };
        let mut s = Session::spawn_with_merged_stderr(config, (24, 80), None).unwrap();
        s.local_shell = true;
        // Bounded poll past REVEAL_TIMEOUT: a shell that never reveals
        // must fail the assert, not park the test binary.
        let mut running = false;
        for _ in 0..100 {
            s.drain();
            if matches!(s.phase, SessionPhase::Running { .. }) {
                running = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(
            running,
            "local `sh` never left Connecting (grid: {:?}, debug: {:?})",
            s.screen_tail_snippet(),
            s.debug_log()
        );
        assert!(
            s.is_live_authenticated(),
            "a revealed local shell must read as live-authenticated"
        );
        assert!(
            s.screen_tail_snippet().contains('$') || s.screen_tail_snippet().contains('#'),
            "shell prompt must be visible on the grid, tail: {:?}",
            s.screen_tail_snippet()
        );
    }

    #[test]
    fn stored_password_is_typed_at_a_live_prompt_when_askpass_is_off() {
        // The PTY-typing fallback covers every ssh too old (or too odd)
        // to honour SSH_ASKPASS_REQUIRE. Oracle: a real child on a real
        // PTY printing `host's password: ` and reading one line — the
        // shape of sshd's password prompt at the session layer.
        if std::process::Command::new("sh")
            .arg("-c")
            .arg("exit 0")
            .output()
            .is_err()
        {
            eprintln!("skipping: no sh binary");
            return;
        }
        let config = SessionConfig {
            hops: Vec::new(),
            argv: vec![
                "sh".into(),
                "-c".into(),
                r#"printf "test@host's password: "; read -r pw; printf 'GOT:%s\n' "$pw""#.into(),
            ],
            display_name: "t".into(),
            meta: SessionMeta::default(),
            pending_secret: Some(PendingSecret::Password("s3cret".into())),
            key_push_identity: None,
            host_name: "t".into(),
        };
        let mut s = Session::spawn(config, (24, 80), None).unwrap();
        // Force the fallback: in-test the askpass helper stages fine
        // (the test binary exists), which would skip the PTY path.
        s.use_askpass = false;
        let mut got = false;
        for _ in 0..200 {
            s.drain();
            if s.screen_tail_snippet().contains("GOT:") {
                got = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            got,
            "child never answered, tail: {:?}",
            s.screen_tail_snippet()
        );
        assert!(
            s.screen_tail_snippet().contains("GOT:s3cret"),
            "fallback must type the stored secret, tail: {:?}",
            s.screen_tail_snippet()
        );
        assert!(s.secret_was_sent());
    }

    #[test]
    fn draining_the_pty_does_not_relay_by_itself() {
        // Draining happens for every session, every frame — including the ones
        // nobody is looking at. Deciding whether a write may reach the host
        // clipboard is the frame's job (it knows which tab is on screen), so
        // `drain` must leave the queue alone.
        let config = SessionConfig {
            hops: Vec::new(),
            argv: vec![
                "sh".into(),
                "-c".into(),
                r"printf '\033]52;c;R0VIRUlN\007MARKER'".into(),
            ],
            display_name: "t".into(),
            meta: SessionMeta::default(),
            pending_secret: None,
            key_push_identity: None,
            host_name: "t".into(),
        };
        let mut s = Session::spawn(config, (24, 80), None).unwrap();

        for _ in 0..200 {
            s.drain();
            if s.screen_tail_snippet().contains("MARKER") {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            s.screen_tail_snippet().contains("MARKER"),
            "PTY output never arrived, tail: {:?}",
            s.screen_tail_snippet()
        );

        assert!(
            s.copy_notice.is_none(),
            "drain must not announce a copy on its own"
        );
        assert_eq!(
            s.parser.take_clipboard_writes(),
            vec!["R0VIRUlN".to_string()],
            "drain must leave the clipboard write for the frame to decide on"
        );
    }

    #[test]
    fn one_drain_of_small_sync_reads_takes_one_snapshot() {
        // A remote flushing `ESU BSU` in many tiny reads must not cost one
        // full-grid rasterization per read on the UI thread.
        let config = SessionConfig {
            hops: Vec::new(),
            argv: vec!["true".into()],
            display_name: "t".into(),
            meta: SessionMeta::default(),
            pending_secret: None,
            key_push_identity: None,
            host_name: "t".into(),
        };
        let mut s = Session::spawn(config, (24, 80), None).unwrap();
        let mut events: Vec<PtyEvent> = (0..16)
            .map(|i| PtyEvent::Bytes(format!("\x1b[?2026l{i:x}\x1b[?2026h").into_bytes()))
            .collect();
        s.handle_pty_events(&mut events);
        assert_eq!(s.parser.snapshots, 1);
        assert!(s.parser.screen().contents().starts_with("0123456789abcdef"));
    }

    /// End-to-end for #113: a child asks the terminal where the cursor is and
    /// *reads the answer back*. Nothing in the parser unit tests proves the
    /// reply actually leaves the process, and this is exactly the loop that
    /// hangs atuin — it asks, waits two seconds, and errors out.
    ///
    /// `stty raw -echo` so the reply arrives byte-for-byte rather than
    /// line-buffered and echoed back at us, and `-icanon min 0 time 5` so the
    /// read returns after half a second with whatever arrived — reading a fixed
    /// byte count instead would have to guess the answer's length, and would
    /// silently truncate it (leaving the tail in the input queue) the moment the
    /// cursor is anywhere but the home position. `tr` strips the escape so the
    /// marker is printable on the grid.
    fn asks_for_the_cursor_position(prelude: &str) -> Session {
        let script = format!(
            r"stty raw -echo -icanon min 0 time 5; {prelude} printf '\033[6n'; \
              r=$(dd bs=1 count=32 2>/dev/null | tr -d '\033['); \
              stty sane; printf 'CPR<%s>\r\n' $r"
        );
        let config = SessionConfig {
            hops: Vec::new(),
            argv: vec!["sh".into(), "-c".into(), script],
            display_name: "t".into(),
            meta: SessionMeta::default(),
            pending_secret: None,
            key_push_identity: None,
            host_name: "t".into(),
        };
        let mut s = Session::spawn(config, (24, 80), None).unwrap();
        // Bounded poll: a child that never gets its answer must fail the assert,
        // not park the test binary. `Drop for PtyRuntime` kills the process
        // group on the way out, so a `dd` still blocked on the read is reaped.
        for _ in 0..300 {
            s.drain();
            if s.screen_tail_snippet().contains("CPR<") {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        s
    }

    #[test]
    fn the_pty_child_receives_a_cursor_position_report() {
        let s = asks_for_the_cursor_position("");
        assert!(
            s.screen_tail_snippet().contains("CPR<1;1R>"),
            "child never got its cursor position back, tail: {:?}",
            s.screen_tail_snippet()
        );
    }

    #[test]
    fn the_report_the_child_receives_is_clamped_at_the_right_margin() {
        // The grid is 80 wide, so 80 characters leave the cursor in vt100's
        // pending-wrap state at column 80 (0-based) — one past the last cell.
        // Unclamped the child would be told `1;81R`, a column its terminal does
        // not have, and code measuring the room left from it underflows. This is
        // the state a prompt that fills the line leaves behind, i.e. the moment
        // #113's reporter presses Up.
        let s = asks_for_the_cursor_position(r"printf '%080d' 0;");
        assert!(
            s.screen_tail_snippet().contains("CPR<1;80R>"),
            "child should be told column 80 of 80, tail: {:?}",
            s.screen_tail_snippet()
        );
    }

    #[test]
    fn discarding_clipboard_writes_keeps_the_query_answers() {
        // The clipboard relay is gated on visibility: a background tab discards
        // instead of relaying. Query answers must never be gated the same way —
        // the application waits for its reply whether or not the user is looking
        // at its tab — so the discard path has to leave them alone. Both come
        // out of the same `PtyCallbacks`, which is what makes this worth an
        // assertion rather than a comment.
        let mut s = scratch_session();
        s.parser.process(b"\x1b]52;c;R0VIRUlN\x07\x1b[5n");
        s.discard_clipboard_writes();
        assert!(
            s.parser.take_clipboard_writes().is_empty(),
            "the clipboard write is the one that gets discarded"
        );
        assert_eq!(
            s.parser.take_replies(),
            b"\x1b[0n".to_vec(),
            "the query answer must survive the discard"
        );
    }

    /// Build a throwaway session for tests that only need a live `Session`.
    fn scratch_session() -> Session {
        let config = SessionConfig {
            hops: Vec::new(),
            argv: vec!["true".into()],
            display_name: "t".into(),
            meta: SessionMeta::default(),
            pending_secret: None,
            key_push_identity: None,
            host_name: "t".into(),
        };
        Session::spawn(config, (24, 80), None).unwrap()
    }
    use crate::test_support::tempdir_sshd;

    #[test]
    fn real_ssh_master_latches_connected_through_drain() {
        // Oracle: a real OpenSSH client (`ssh -v`) against a tempdir `sshd`
        // with key auth — the normal/master path (no ControlMaster). The
        // client must print `Authenticated to …` on the siphoned stderr so
        // `maybe_detect_connected` latches, and the banner must reveal
        // `Running`, earning `is_live_authenticated` through `drain` alone.
        let Some(sshd) = tempdir_sshd() else {
            return;
        };
        let user = std::env::var("USER").unwrap_or_else(|_| "root".into());
        let known = sshd.dir.path().join("known_hosts");
        let config = SessionConfig {
            hops: Vec::new(),
            argv: vec![
                "ssh".into(),
                "-v".into(),
                "-F".into(),
                "/dev/null".into(),
                "-i".into(),
                sshd.user_key.to_str().unwrap().into(),
                "-o".into(),
                "StrictHostKeyChecking=accept-new".into(),
                "-o".into(),
                format!("UserKnownHostsFile={}", known.display()),
                "-o".into(),
                "IdentitiesOnly=yes".into(),
                "-o".into(),
                "BatchMode=yes".into(),
                "-p".into(),
                sshd.port.to_string(),
                format!("{user}@127.0.0.1"),
                "printf 'READY\\n'; exec sleep 60".into(),
            ],
            display_name: "oracle".into(),
            meta: SessionMeta::default(),
            pending_secret: None,
            key_push_identity: None,
            host_name: "oracle".into(),
        };
        let mut s = Session::spawn(config, (24, 80), None).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            s.drain();
            if s.is_live_authenticated() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "real ssh master never authenticated (connected={} phase={:?} debug_tail={:?})",
                s.is_connected(),
                s.phase,
                s.debug_log().chars().rev().take(400).collect::<String>(),
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            s.history_capture_allowed(),
            "capture must be allowed on a clean remote prompt"
        );
    }

    #[test]
    fn proxyjump_hop_authenticating_never_reads_as_connected() {
        // Oracle: real OpenSSH through a real jump (#140), the tempdir sshd
        // serving both hops: the jump accepts our key, the destination is an
        // unknown user. The jump's own `Authenticated to …` reaches the same
        // stderr, and when it latched `connected` the askpass channel closed
        // before the destination could ask for its password: exit 255. A
        // session whose destination never authenticated must never read
        // connected.
        let Some(sshd) = tempdir_sshd() else {
            return;
        };
        let dir = sshd.dir.path();
        let user = std::env::var("USER").unwrap_or_else(|_| "root".into());
        let known = dir.join("known_hosts");
        let host_key = std::fs::read_to_string(sshd.host_key.with_extension("pub")).unwrap();
        std::fs::write(&known, format!("[127.0.0.1]:{} {host_key}", sshd.port)).unwrap();
        // `-F` is the one client setting ssh hands on to the jump hop.
        let client_config = dir.join("ssh_config");
        std::fs::write(
            &client_config,
            format!(
                "IdentityFile {}\nIdentitiesOnly yes\nUserKnownHostsFile {}\nBatchMode yes\n",
                sshd.user_key.display(),
                known.display(),
            ),
        )
        .unwrap();
        let argv = crate::app::prepare_session_connect_argv(
            vec![
                "ssh".into(),
                "-F".into(),
                client_config.to_str().unwrap().into(),
                "-J".into(),
                format!("{user}@127.0.0.1:{}", sshd.port),
                "-p".into(),
                sshd.port.to_string(),
                "sshub-no-such-user@127.0.0.1".into(),
            ],
            false,
        );
        let config = SessionConfig {
            hops: Vec::new(),
            argv,
            display_name: "jump".into(),
            meta: SessionMeta::default(),
            pending_secret: None,
            key_push_identity: None,
            host_name: "jump".into(),
        };
        let mut s = Session::spawn(config, (24, 80), None).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !s.phase.is_terminal() {
            s.drain();
            assert!(
                !s.is_connected(),
                "the jump hop's marker latched connected: {:?}",
                s.debug_log()
            );
            assert!(
                std::time::Instant::now() < deadline,
                "ssh never exited: {:?}",
                s.debug_log()
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        // Not vacuous: the jump hop did authenticate before the destination failed.
        let log = std::fs::read_to_string(&sshd.log).unwrap();
        assert!(
            log.contains(&format!("Accepted publickey for {user}")),
            "{log}"
        );
        assert!(log.contains("Invalid user sshub-no-such-user"), "{log}");
    }

    /// The ssh argv to the tempdir sshd as the TUI builds it for a host whose
    /// startup command is `command`: `build_ssh_argv`, with a client config for
    /// the test key. Not yet through `prepare_session_connect_argv`.
    fn oracle_argv(
        sshd: &crate::test_support::TempdirSshd,
        command: Option<&str>,
        client_config_extra: &str,
    ) -> Vec<String> {
        let dir = sshd.dir.path();
        let known = dir.join("known_hosts");
        let host_key = std::fs::read_to_string(sshd.host_key.with_extension("pub")).unwrap();
        std::fs::write(&known, format!("[127.0.0.1]:{} {host_key}", sshd.port)).unwrap();
        let client_config = dir.join("ssh_config");
        std::fs::write(
            &client_config,
            format!(
                "IdentitiesOnly yes\nUserKnownHostsFile {}\nBatchMode yes\n{client_config_extra}",
                known.display(),
            ),
        )
        .unwrap();
        let mut host = crate::ssh::SshHost::new("oracle");
        host.hostname = Some("127.0.0.1".into());
        host.port = Some(sshd.port);
        host.user = Some(crate::test_support::current_user());
        host.identity_file = Some(sshd.user_key.to_str().unwrap().into());
        host.remote_command = command.map(Into::into);
        let mut argv = crate::ssh::build_ssh_argv(&host);
        argv.splice(1..1, ["-F".into(), client_config.to_str().unwrap().into()]);
        argv
    }

    fn oracle_session(argv: Vec<String>) -> Session {
        let config = SessionConfig {
            hops: Vec::new(),
            argv: crate::app::prepare_session_connect_argv(argv, false),
            display_name: "oracle".into(),
            meta: SessionMeta::default(),
            pending_secret: None,
            key_push_identity: None,
            host_name: "oracle".into(),
        };
        Session::spawn(config, (24, 80), None).unwrap()
    }

    /// A session to the tempdir sshd built the way the TUI builds one for a
    /// host whose startup command is `command`.
    fn startup_command_session(sshd: &crate::test_support::TempdirSshd, command: &str) -> Session {
        oracle_session(oracle_argv(sshd, Some(command), ""))
    }

    #[test]
    fn a_startup_command_runs_in_a_remote_terminal() {
        // Oracle: real OpenSSH against a tempdir sshd (#171). ssh asks for a
        // remote terminal on its own only for a login shell, so a startup
        // command ran without one: `sudo su root` printed no prompt.
        let Some(sshd) = tempdir_sshd() else {
            return;
        };
        let mut s = startup_command_session(&sshd, "tty; exec sleep 60");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let text = loop {
            s.drain();
            let text = s.parser.screen().contents();
            if text.contains("/dev/") || text.contains("not a tty") {
                break text;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "no output from the startup command (phase={:?}): {:?}",
                s.phase,
                s.debug_log()
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        assert!(!text.contains("not a tty"), "{text:?}");
    }

    #[test]
    fn a_startup_command_leaves_the_login_shell() {
        // Oracle: real OpenSSH against a tempdir sshd (#171). A startup
        // command runs at login and the session goes on: `cd /` used to be
        // the whole session, which ended as soon as it returned.
        let Some(sshd) = tempdir_sshd() else {
            return;
        };
        let mut s = startup_command_session(&sshd, "cd /");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !s.is_connected() {
            s.drain();
            assert!(
                std::time::Instant::now() < deadline,
                "never authenticated: {:?}",
                s.debug_log()
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        s.write(b"echo \"pwd=$PWD\"\r").unwrap();
        loop {
            s.drain();
            let text = s.parser.screen().contents();
            if text.lines().any(|l| l.trim_end() == "pwd=/") {
                break;
            }
            assert!(
                !s.phase.is_terminal() && std::time::Instant::now() < deadline,
                "no shell in / after the startup command (phase={:?}): {text:?}",
                s.phase
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// A loopback HTTP server answering every connection with `body`; the
    /// counter is how many connections it took.
    fn http_server(body: &'static str) -> (u16, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = hits.clone();
        std::thread::spawn(move || {
            for mut conn in listener.incoming().flatten() {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let mut request = [0u8; 1024];
                let _ = conn.read(&mut request);
                let _ = write!(
                    conn,
                    "HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        (port, hits)
    }

    /// Drain `s` until `done` holds, or fail after 20 s with ssh's log.
    fn drain_until(s: &mut Session, what: &str, done: impl Fn(&Session) -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !done(s) {
            s.drain();
            assert!(
                !s.phase.is_terminal() && std::time::Instant::now() < deadline,
                "{what} (phase={:?}): {:?} / {:?}",
                s.phase,
                s.parser.screen().contents(),
                s.debug_log()
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    fn curl_missing() -> bool {
        std::process::Command::new("curl")
            .arg("--version")
            .output()
            .is_err()
            && crate::test_support::tool_missing("curl")
    }

    /// The exit code the typed command printed as `refused-rc=<n>`. The typed
    /// command itself echoes `refused-rc=$?`, which is no number.
    fn refused_rc(text: &str) -> Option<i32> {
        text.lines()
            .filter_map(|l| l.trim_end().rsplit_once("refused-rc=").map(|(_, n)| n))
            .find_map(|n| n.parse::<i32>().ok())
    }

    /// In the session's shell: fetch `allowed`, then `other`, both through the
    /// SOCKS proxy on `port`. Returns the screen once the second curl is done.
    fn fetch_allowed_and_other(s: &mut Session, port: u16, allowed: u16, other: u16) -> String {
        drain_until(s, "never authenticated", Session::is_connected);
        let proxy = format!("socks5h://127.0.0.1:{port}");
        s.write(
            format!(
                "curl -sS -m 10 -x {proxy} http://127.0.0.1:{allowed}/; echo; \
                 curl -sS -m 10 -x {proxy} http://127.0.0.1:{other}/; echo \"refused-rc=$?\"\r"
            )
            .as_bytes(),
        )
        .unwrap();
        drain_until(s, "no result from curl", |s| {
            refused_rc(&s.parser.screen().contents()).is_some()
        });
        s.parser.screen().contents()
    }

    /// A connected session to the tempdir sshd with sharing toggled on, the
    /// allowed destinations set to `permit`.
    fn sharing_session(argv: Vec<String>, permit: &str) -> Session {
        let mut s = oracle_session(argv);
        drain_until(&mut s, "never authenticated", Session::is_connected);
        s.start_net_share(None, &[], permit).unwrap();
        assert_eq!(s.net_share(), NetShare::Starting);
        s
    }

    fn shared_port(s: &mut Session) -> u16 {
        drain_until(s, "ssh never reported the port", |s| {
            matches!(s.net_share(), NetShare::Port(_))
        });
        let NetShare::Port(port) = s.net_share() else {
            unreachable!()
        };
        port
    }

    fn pid_alive(pid: u32) -> bool {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }

    /// Whether something accepts on the host's loopback port; the tempdir sshd
    /// is this machine, so the "host side" of the proxy is reachable directly.
    fn accepts(port: u16) -> bool {
        std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()
    }

    fn wait_until(what: &str, done: impl Fn() -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done() {
            assert!(std::time::Instant::now() < deadline, "{what}");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn toggling_the_share_on_gives_the_host_a_proxy_to_the_allowed_ports_only() {
        // Oracle: real OpenSSH against a tempdir sshd, and real curl on the
        // "host" side of the session (#174). The port comes from the sidecar's
        // own log; a fetch through it reaches the one allowed local web
        // server, one to a server outside the allowed list is refused, and the
        // session itself carries no forward.
        if curl_missing() {
            return;
        }
        let Some(sshd) = tempdir_sshd() else {
            return;
        };
        let (allowed, allowed_hits) = http_server("shared-net-reply");
        let (other, other_hits) = http_server("must-stay-unreachable");
        let mut s = sharing_session(
            oracle_argv(&sshd, None, ""),
            &format!("127.0.0.1:{allowed}"),
        );
        assert!(!s.config.argv.iter().any(|a| a == "-R"));
        let port = shared_port(&mut s);
        // The hint for the log panel carries the port and both commands.
        let hint = s
            .take_diagnostics()
            .into_iter()
            .find(|d| d.contains("socks5h"))
            .expect("a hint line for the ssh log");
        assert!(hint.starts_with("session:") && hint.contains(&format!("127.0.0.1:{port}")));
        assert!(crate::session::keep_diagnostic(&hint, true));

        let text = fetch_allowed_and_other(&mut s, port, allowed, other);
        assert!(
            text.lines().any(|l| l.trim_end() == "shared-net-reply"),
            "{text:?}"
        );
        assert_ne!(refused_rc(&text), Some(0), "{text:?}");
        assert!(!text.contains("must-stay-unreachable"), "{text:?}");
        assert_eq!(allowed_hits.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(other_hits.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn toggling_the_share_off_ends_the_proxy_and_its_ssh() {
        // Oracle: real OpenSSH (#174). Off must kill the sidecar: its pid is
        // gone and the port on the host stops accepting, while the session
        // still works.
        let Some(sshd) = tempdir_sshd() else {
            return;
        };
        let mut s = sharing_session(oracle_argv(&sshd, None, ""), share_net::PERMIT_REMOTE_OPEN);
        let port = shared_port(&mut s);
        let pid = s.sidecar_pid().expect("a sidecar");
        assert!(pid_alive(pid) && accepts(port), "setup: proxy is up");

        s.stop_net_share();
        assert_eq!(s.net_share(), NetShare::Off);
        assert!(!pid_alive(pid), "the sidecar ssh outlived the toggle");
        wait_until("the proxy port still accepts", || !accepts(port));
        s.write(b"echo \"still-$((20+22))\"\r").unwrap();
        drain_until(&mut s, "the session did not go on", |s| {
            s.parser
                .screen()
                .contents()
                .lines()
                .any(|l| l.trim_end() == "still-42")
        });
    }

    #[test]
    fn dropping_the_session_ends_the_proxy_and_its_ssh() {
        // Oracle: real OpenSSH (#174). Closing a session, or quitting SSHub,
        // drops it: the sidecar must never be orphaned.
        let Some(sshd) = tempdir_sshd() else {
            return;
        };
        let mut s = sharing_session(oracle_argv(&sshd, None, ""), share_net::PERMIT_REMOTE_OPEN);
        let port = shared_port(&mut s);
        let pid = s.sidecar_pid().expect("a sidecar");
        assert!(pid_alive(pid) && accepts(port), "setup: proxy is up");

        drop(s);
        assert!(!pid_alive(pid), "the sidecar ssh outlived its session");
        wait_until("the proxy port still accepts", || !accepts(port));
    }

    #[test]
    fn a_host_that_refuses_forwarding_shows_failed_and_the_session_goes_on() {
        // Oracle: real OpenSSH against a tempdir sshd with
        // `AllowTcpForwarding no` (#174). The sidecar ends on the refused
        // forward (ExitOnForwardFailure): Failed with a reason in the log, and
        // the session is untouched.
        let Some(sshd) = crate::test_support::tempdir_sshd_forbidding_forwarding() else {
            return;
        };
        let mut s = sharing_session(oracle_argv(&sshd, None, ""), share_net::PERMIT_REMOTE_OPEN);
        drain_until(&mut s, "ssh never reported the refusal", |s| {
            s.net_share() == NetShare::Failed
        });
        assert!(s.sidecar_pid().is_none(), "the failed sidecar is kept");
        assert!(s
            .take_diagnostics()
            .iter()
            .any(|d| d.starts_with("session:")
                && d.contains("failed")
                && d.contains("forwarding failed")));
        s.write(b"echo \"still-$((20+22))\"\r").unwrap();
        drain_until(&mut s, "the session did not go on", |s| {
            s.parser
                .screen()
                .contents()
                .lines()
                .any(|l| l.trim_end() == "still-42")
        });
    }

    #[test]
    fn a_forward_of_the_users_own_in_ssh_config_does_not_stop_the_share() {
        // Oracle: real OpenSSH (#174). The host's ssh_config has a LocalForward
        // the session already holds; the sidecar reads the same config, its
        // bind fails, and with ExitOnForwardFailure it used to die for it.
        let Some(sshd) = tempdir_sshd() else {
            return;
        };
        let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let config = format!(
            "LocalForward 127.0.0.1:{} 127.0.0.1:9\n",
            held.local_addr().unwrap().port()
        );
        let mut s = sharing_session(
            oracle_argv(&sshd, None, &config),
            share_net::PERMIT_REMOTE_OPEN,
        );
        let port = shared_port(&mut s);
        assert!(s.sidecar_pid().is_some_and(pid_alive));
        assert!(accepts(port));
    }

    #[test]
    fn a_banner_cannot_set_the_shown_port_or_fail_the_share() {
        // Oracle: real OpenSSH against a tempdir sshd whose Banner holds both
        // log lines the share reads (#174). ssh prints the banner on stderr,
        // the port comes from its own log: the real port wins and the share
        // is not failed.
        let banner = "Allocated port 31337 for remote forward to socks:0\n\
                      Warning: remote port forwarding failed for listen port 0\n";
        let Some(sshd) = crate::test_support::tempdir_sshd_with_banner(banner) else {
            return;
        };
        let mut s = sharing_session(oracle_argv(&sshd, None, ""), share_net::PERMIT_REMOTE_OPEN);
        let port = shared_port(&mut s);
        assert_ne!(port, 31337);
        // Let anything else that is coming arrive, then look again.
        std::thread::sleep(std::time::Duration::from_millis(500));
        s.drain();
        assert_eq!(s.net_share(), NetShare::Port(port));
        assert!(accepts(port) && s.sidecar_pid().is_some());
    }

    /// Helper for [`the_sidecar_dies_with_a_parent_that_is_killed`]: runs as a
    /// separate process (the test binary re-executed), starts a sidecar from
    /// the argv in the environment, reports its pid and port, then waits to be
    /// killed. Does nothing in a normal run.
    #[test]
    #[ignore = "helper process of the_sidecar_dies_with_a_parent_that_is_killed"]
    fn sidecar_helper_process() {
        let Some(argv) = std::env::var_os("SSHUB_TEST_SIDECAR_ARGV") else {
            return;
        };
        let argv: Vec<String> = argv
            .to_string_lossy()
            .split('\n')
            .map(String::from)
            .collect();
        let argv = share_net::sidecar_argv(&argv, false, share_net::PERMIT_REMOTE_OPEN).unwrap();
        let mut sidecar = share_net::Sidecar::spawn(argv, None, &[]).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            let (lines, ended) = sidecar.poll();
            for line in lines {
                if let Some(NetShare::Port(port)) = share_net::parse_log_line(&line) {
                    println!("SIDECAR {} {port}", sidecar.pid());
                    std::thread::sleep(std::time::Duration::from_secs(120));
                    return;
                }
            }
            if ended {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_sidecar_dies_with_a_parent_that_is_killed() {
        // Oracle: real OpenSSH and a real SIGKILL (#174). sshub can die
        // without running a destructor (a closed terminal, kill -9); the
        // sidecar must not outlive it holding a proxy port on the host.
        use std::io::BufRead;
        let Some(sshd) = tempdir_sshd() else {
            return;
        };
        let argv = oracle_argv(&sshd, None, "");
        let mut helper = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "session::prompt_tests::sidecar_helper_process",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("SSHUB_TEST_SIDECAR_ARGV", argv.join("\n"))
            // The helper dies by SIGKILL and cannot remove its share log dir;
            // under the sshd's tempdir it goes when the test ends.
            .env("TMPDIR", sshd.dir.path())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        /// Whatever happens, nothing of the helper or its sidecar survives.
        struct Cleanup(std::process::Child, Option<u32>);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
                if let Some(pid) = self.1 {
                    unsafe { libc::kill(pid as i32, libc::SIGKILL) };
                }
            }
        }
        let stdout = helper.stdout.take().unwrap();
        let mut guard = Cleanup(helper, None);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout)
                .lines()
                .map_while(Result::ok)
            {
                // libtest writes "test <name> ... " in front of what a test prints.
                if let Some((_, rest)) = line.split_once("SIDECAR ") {
                    let _ = tx.send(rest.to_string());
                    return;
                }
            }
        });
        let report = rx
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("the helper never reported a sidecar");
        let (pid, port) = report.split_once(' ').unwrap();
        let (pid, port): (u32, u16) = (pid.parse().unwrap(), port.parse().unwrap());
        guard.1 = Some(pid);
        assert!(pid_alive(pid) && accepts(port), "setup: proxy is up");

        unsafe { libc::kill(guard.0.id() as i32, libc::SIGKILL) };
        let _ = guard.0.wait();
        wait_until("the sidecar outlived a killed parent", || {
            !pid_alive(pid) && !accepts(port)
        });
    }

    /// Kills the user's mux master when dropped, so a failing test leaves none.
    struct MuxMaster(Vec<String>);

    impl Drop for MuxMaster {
        fn drop(&mut self) {
            let mut exit = self.0.clone();
            exit.splice(1..1, ["-O".to_string(), "exit".to_string()]);
            let _ = std::process::Command::new(&exit[0])
                .args(&exit[1..])
                .output();
        }
    }

    #[test]
    fn the_proxy_ignores_the_users_control_master() {
        // Oracle: real OpenSSH 10.5p1 (#174). With a ControlMaster to the host
        // already running, a plain `-R` is forwarded to the master, which has
        // no `PermitRemoteOpen`: the proxy then reached ports outside the
        // allowed list. The sidecar must not ride the master.
        if curl_missing() {
            return;
        }
        let Some(sshd) = tempdir_sshd() else {
            return;
        };
        // `%C` is a 40-character hash, and ssh appends a random suffix while
        // binding: under macOS's long per-user TMPDIR that passed the ~104
        // byte limit of a Unix socket path and the master never started. The
        // port alone names this test's one master.
        let control = format!(
            "ControlMaster auto\nControlPath {}/cm-%p\nControlPersist 30\n",
            sshd.dir.path().display()
        );
        let argv = oracle_argv(&sshd, None, &control);
        let _master = MuxMaster(argv.clone());
        let mut start = argv.clone();
        start.splice(1..1, ["-fN".to_string()]);
        let out = std::process::Command::new(&start[0])
            .args(&start[1..])
            .output()
            .unwrap();
        assert!(out.status.success(), "no master: {out:?}");
        let mut check = argv.clone();
        check.splice(1..1, ["-O".to_string(), "check".to_string()]);
        let out = std::process::Command::new(&check[0])
            .args(&check[1..])
            .output()
            .unwrap();
        assert!(out.status.success(), "setup: master not running: {out:?}");

        let (allowed, _) = http_server("shared-net-reply");
        let (other, other_hits) = http_server("must-stay-unreachable");
        let mut s = sharing_session(argv, &format!("127.0.0.1:{allowed}"));
        let port = shared_port(&mut s);
        let text = fetch_allowed_and_other(&mut s, port, allowed, other);
        assert_ne!(refused_rc(&text), Some(0), "{text:?}");
        assert!(!text.contains("must-stay-unreachable"), "{text:?}");
        assert_eq!(other_hits.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    /// Run the sidecar for `argv` on its own until `done` holds of its log
    /// lines, or it ends. Returns the lines and the failure reason.
    fn run_sidecar(argv: &[String], done: impl Fn(&str) -> bool) -> (Vec<String>, String) {
        let sidecar_argv = share_net::sidecar_argv(argv, false, share_net::PERMIT_REMOTE_OPEN)
            .expect("an ssh argv");
        let mut sidecar = share_net::Sidecar::spawn(sidecar_argv, None, &[]).unwrap();
        let mut seen = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            let (lines, ended) = sidecar.poll();
            let hit = lines.iter().any(|l| done(l));
            seen.extend(lines);
            if hit || ended {
                return (seen, sidecar.failure_reason());
            }
            assert!(
                std::time::Instant::now() < deadline,
                "no sidecar log: {seen:?}"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn the_sidecar_never_runs_the_hosts_startup_command() {
        // Oracle: real OpenSSH (#174). The session's argv ends in the stored
        // startup command; the sidecar only carries the proxy, so that command
        // must not run a second time on the host.
        let Some(sshd) = tempdir_sshd() else {
            return;
        };
        let marker = sshd.dir.path().join("startup-ran");
        let argv = oracle_argv(&sshd, Some(&format!("touch {}", marker.display())), "");
        let (lines, _) = run_sidecar(&argv, |l| l.starts_with("Allocated port "));
        assert!(
            lines.iter().any(|l| l.starts_with("Allocated port ")),
            "{lines:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(!marker.exists(), "the sidecar ran the startup command");
    }

    #[test]
    fn a_host_that_needs_a_password_fails_fast_with_the_reason() {
        // Oracle: real OpenSSH against a tempdir sshd that wants a password
        // (#174). With no stored secret the sidecar runs in BatchMode and ends
        // instead of hanging on a prompt nobody sees.
        let Some(sshd) = crate::test_support::tempdir_sshd_asking_for_a_password() else {
            return;
        };
        let argv = oracle_argv(&sshd, None, "");
        let (_, reason) = run_sidecar(&argv, |_| false);
        assert_eq!(
            reason,
            "needs a login without prompts: key or stored password"
        );
    }

    #[test]
    fn an_authenticated_session_that_prints_nothing_still_reveals() {
        // Oracle: real OpenSSH against a tempdir sshd (#171). The connect
        // screen gave way only to output, so a startup command that prints
        // nothing kept an authenticated session behind the spinner for good.
        let Some(sshd) = tempdir_sshd() else {
            return;
        };
        let mut s = startup_command_session(&sshd, "exec sleep 60");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !s.is_connected() {
            s.drain();
            assert!(
                std::time::Instant::now() < deadline,
                "never authenticated: {:?}",
                s.debug_log()
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            !s.saw_pty_bytes && matches!(s.phase, SessionPhase::Connecting { .. }),
            "setup: authenticated and silent (phase={:?})",
            s.phase
        );
        // Past the reveal timeout, without waiting it out.
        s.phase = SessionPhase::Connecting {
            started_at: Instant::now() - REVEAL_TIMEOUT,
        };
        s.drain();
        assert!(
            matches!(s.phase, SessionPhase::Running { .. }),
            "{:?}",
            s.phase
        );
    }

    #[test]
    fn clipboard_writes_from_the_pty_are_relayed_and_announced() {
        // An app inside the PTY (herdr, tmux, an editor with clipboard=osc52)
        // copies by writing OSC 52 to its stdout. vt100 hands that to the
        // default no-op callback, where it used to vanish — the selection
        // highlighted but the clipboard never changed.
        let mut s = scratch_session();
        // base64("GEHEIM") == "R0VIRUlN" → 6 bytes decoded.
        s.parser.process(b"BEFORE\x1b]52;c;R0VIRUlN\x07AFTER");

        let mut sent = Vec::new();
        s.relay_clipboard_writes_with(|payload| {
            sent.push(payload.to_string());
            Ok(())
        });

        assert_eq!(sent, vec!["R0VIRUlN".to_string()]);
        assert_eq!(
            s.copy_notice.as_ref().map(|(m, _)| m.as_str()),
            Some("remote copied 6 bytes to clipboard"),
            "a relayed clipboard write must be visible to the user"
        );

        // The sequence itself must never become text on the grid.
        let tail = s.screen_tail_snippet();
        assert!(tail.contains("BEFOREAFTER"), "tail: {tail:?}");
        assert!(!tail.contains("52;c;"), "escape sequence leaked: {tail:?}");
    }

    #[test]
    fn failing_clipboard_relay_is_reported_not_swallowed() {
        // A failed relay must not claim success: no notice, but a diagnostic.
        let mut s = scratch_session();
        s.parser.process(b"\x1b]52;c;R0VIRUlN\x07");

        s.relay_clipboard_writes_with(|_| Err(std::io::Error::other("boom")));

        assert!(
            s.copy_notice.is_none(),
            "a failed relay must not announce a copy"
        );
        assert!(
            s.take_diagnostics()
                .iter()
                .any(|d| d.contains("clipboard relay failed")),
            "a failed relay must surface in diagnostics"
        );
    }

    #[test]
    fn a_mixed_batch_names_the_copy_and_the_drops() {
        // A batch that both relayed something and dropped something must say
        // so: the success message alone would hide the loss.
        let mut s = scratch_session();
        let mut stream = b"\x1b]52;c;R0VIRUlN\x07\x1b]52;c;".to_vec();
        stream.extend(std::iter::repeat_n(b'A', 90_000));
        stream.push(0x07);
        s.parser.process(&stream);

        s.relay_clipboard_writes_with(|_| Ok(()));

        assert_eq!(
            s.copy_notice.as_ref().map(|(m, _)| m.as_str()),
            Some("remote copied 6 bytes to clipboard; dropped 1 oversized write"),
        );
    }

    #[test]
    fn drops_alone_are_still_announced() {
        // Nothing was relayed, but the user must learn a copy was thrown away.
        let mut s = scratch_session();
        let mut stream = b"\x1b]52;c;".to_vec();
        stream.extend(std::iter::repeat_n(b'A', 90_000));
        stream.push(0x07);
        for _ in 0..20 {
            stream.extend_from_slice(b"\x1b]52;c;R0VIRUlN\x07");
        }
        s.parser.process(&stream);
        // Take the queued writes away, so only the drops remain to report.
        let _ = s.parser.take_clipboard_writes();

        s.relay_clipboard_writes_with(|_| Ok(()));

        assert_eq!(
            s.copy_notice.as_ref().map(|(m, _)| m.as_str()),
            Some("clipboard relay dropped 1 oversized write, 12 over the queue limit"),
        );
    }

    #[test]
    fn an_empty_payload_produces_no_notice_at_all() {
        // `ESC]52;c;BEL` must not relay, must not clear, must not claim success.
        let mut s = scratch_session();
        s.parser.process(b"\x1b]52;c;\x07");

        let mut sent = 0usize;
        s.relay_clipboard_writes_with(|_| {
            sent += 1;
            Ok(())
        });

        assert_eq!(sent, 0);
        assert!(s.copy_notice.is_none());
    }

    #[test]
    fn discarded_clipboard_writes_are_never_resent() {
        // A session that isn't the visible one drops what it saw. Bringing it
        // to the front later must not replay the remote's old copy.
        let mut s = scratch_session();
        s.parser.process(b"\x1b]52;c;R0VIRUlN\x07");

        s.discard_clipboard_writes();

        let mut sent = Vec::new();
        s.relay_clipboard_writes_with(|p| {
            sent.push(p.to_string());
            Ok(())
        });
        assert!(sent.is_empty(), "a discarded write must never be resent");
        assert!(s.copy_notice.is_none(), "a discarded write must be silent");
    }

    #[test]
    fn discarding_also_clears_drop_counters() {
        // Otherwise the drops of an invisible session would surface the moment
        // it becomes visible.
        let mut s = scratch_session();
        let mut stream = b"\x1b]52;c;".to_vec();
        stream.extend(std::iter::repeat_n(b'A', 90_000));
        stream.push(0x07);
        s.parser.process(&stream);

        s.discard_clipboard_writes();
        s.relay_clipboard_writes_with(|_| Ok(()));

        assert!(s.copy_notice.is_none());
    }

    #[test]
    fn a_relay_failure_survives_the_connected_session_filter() {
        // The event loop drops most diagnostics once a session is connected.
        // A failed relay is exactly the case that must still reach the log, so
        // it has to satisfy the real filter, not a look-alike.
        let mut s = scratch_session();
        s.parser.process(b"\x1b]52;c;R0VIRUlN\x07");
        s.relay_clipboard_writes_with(|_| Err(std::io::Error::other("boom")));

        let kept: Vec<String> = s
            .take_diagnostics()
            .into_iter()
            .filter(|line| keep_diagnostic(line, true))
            .collect();

        assert!(
            kept.iter().any(|d| d.contains("clipboard relay failed")),
            "a failed relay must survive the connected-session filter, kept: {kept:?}"
        );
    }

    #[test]
    fn clipboard_queue_is_emptied_by_relaying() {
        // Guards against re-sending the same payload on the next drain.
        let mut s = scratch_session();
        s.parser.process(b"\x1b]52;c;R0VIRUlN\x07");

        let mut sent = 0usize;
        s.relay_clipboard_writes_with(|_| {
            sent += 1;
            Ok(())
        });
        s.relay_clipboard_writes_with(|_| {
            sent += 1;
            Ok(())
        });

        assert_eq!(sent, 1, "each clipboard write must be relayed exactly once");
    }

    #[test]
    fn failed_connect_surfaces_reason_in_debug_log() {
        // Mimic an unreachable host: ssh writes the error to stderr and exits
        // with nothing on stdout. The reason must end up in debug_log (so the
        // render layer can show it) and the session must never latch connected.
        let config = SessionConfig {
            hops: Vec::new(),
            argv: vec![
                "sh".into(),
                "-c".into(),
                "printf 'connect to host x port 22: Connection timed out' 1>&2; exit 1".into(),
            ],
            display_name: "x".into(),
            meta: SessionMeta::default(),
            pending_secret: None,
            key_push_identity: None,
            host_name: "x".into(),
        };
        let mut s = Session::spawn(config, (24, 80), None).unwrap();
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
        let mut parser = ParserState::new(40, 100);
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
        let mut parser = ParserState::new(40, 100);
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
        let mut parser = ParserState::new(40, 100);
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
        let mut parser = ParserState::new(10, 100);
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
