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
}

impl Session {
    /// Spawn the child on a freshly allocated PTY and start the reader thread.
    pub fn spawn(config: SessionConfig, rows: u16, cols: u16) -> Result<Self> {
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
            config,
        })
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
    (
        "connection reset",
        "Connection reset by the remote host.",
    ),
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
        let mut s = Session::spawn(config, 24, 80).unwrap();

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
        let mut s = Session::spawn(config, 24, 80).unwrap();
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
            s.debug_log().to_ascii_lowercase().contains("connection timed out"),
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
