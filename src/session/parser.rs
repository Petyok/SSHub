//! VT100 parser wrapper. Maintains an in-memory `vt100::Screen` that the
//! renderer reads via `tui-term`, relays OSC 52 clipboard writes that
//! applications inside the PTY emit, and answers the terminal status queries
//! they send — `vt100` implements none of the latter, and an application that
//! asks and hears nothing back blocks until its own timeout. In front of the
//! parser, [`XtermCompat`] translates the few xterm sequences stock `vt100`
//! drops into ones it does understand.

use unicode_width::UnicodeWidthChar;

/// Largest decoded payload we'll relay from the PTY to the host clipboard
/// (64 KiB). Keeps a remote from flooding the clipboard with a huge write.
const CLIPBOARD_RELAY_MAX_BYTES: usize = 64 * 1024;

/// How many clipboard writes we buffer between drains. A remote stuck in a
/// copy loop can't grow the queue without bound; the excess is dropped.
const CLIPBOARD_RELAY_MAX_QUEUED: usize = 8;

/// Largest queue of answers to terminal status queries we hold between drains
/// (1 KiB). Every answer is a dozen bytes at most, so this is far more than any
/// real application asks for in one frame — it exists so a remote stuck in a
/// query loop can't make us buffer unbounded input for it.
const REPLY_QUEUE_MAX_BYTES: usize = 1024;

/// Longest CSI sequence held back waiting for its final byte (256 bytes).
/// Real ones are a few dozen bytes; a longer one is passed through unchanged
/// instead of being buffered without bound.
const CSI_HOLD_MAX: usize = 256;

const ESC: u8 = 0x1b;

/// Where [`XtermCompat`] stands in the byte stream: the slice of vte's state
/// machine that tells printed text, a CSI sequence and string payloads apart.
#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Scan {
    #[default]
    Ground,
    /// `ESC` seen and held, the next byte decides what it starts.
    Esc,
    /// Inside `ESC [`, held until the final byte.
    Csi,
    /// Passed through until a final byte in `lo..=0x7e`: an `ESC (`-style
    /// sequence (`0x30`), or a CSI past [`CSI_HOLD_MAX`] (`0x40`).
    Pass(u8),
    /// OSC payload, ended by BEL or `ESC`.
    Osc,
    /// DCS parameters, up to the final byte that starts the payload.
    DcsHead,
    /// DCS payload, ended by `ESC` or a raw C1 ST (`0x9c`).
    Dcs,
    /// SOS/PM/APC payload, ended by `ESC`.
    Str,
}

/// Rewrites xterm sequences stock `vt100` 0.16 ignores into ones it handles,
/// and clamps the counts it would loop on for seconds. btop, for one, places
/// everything with HVP (`CSI r;c f`); vt100 ignored it, so each frame landed
/// where the previous one stopped. Diagnosed in PR #139.
///
/// Only CSI sequences are touched, and only when held whole: a sequence split
/// across PTY reads is kept back and finished on the next read. Bytes inside
/// OSC/DCS/APC strings are never rewritten; when one ends at a bare `ESC`, a
/// CAN goes first so vt100 leaves the string at the same point vte does.
#[derive(Default)]
struct XtermCompat {
    scan: Scan,
    /// The held `ESC` or `ESC [ …` sequence.
    seq: Vec<u8>,
    /// Last graphic character printed, for REP. Kept across reads.
    last: Option<char>,
    /// A UTF-8 character being assembled, possibly across reads.
    utf8: [u8; 4],
    utf8_len: u8,
    utf8_need: u8,
}

impl XtermCompat {
    /// Translate `bytes`, appending the result to `out` in order.
    fn feed(&mut self, bytes: &[u8], size: (u16, u16), out: &mut Vec<u8>) {
        let mut span = 0;
        for (i, &b) in bytes.iter().enumerate() {
            if !matches!(self.scan, Scan::Esc | Scan::Csi) {
                match (self.scan, b) {
                    (_, ESC) => {
                        out.extend_from_slice(&bytes[span..i]);
                        span = i + 1;
                        // vte ends a string or aborts a sequence on this ESC,
                        // before seeing what follows it. Holding the ESC would
                        // leave vte inside, swallowing REP output and C0
                        // controls sent ahead of it; CAN ends it right here.
                        if self.scan != Scan::Ground {
                            out.push(0x18);
                        }
                        self.hold_esc();
                    }
                    (Scan::Ground, _) => self.track(b),
                    (_, 0x18 | 0x1a) | (Scan::Osc, 0x07) | (Scan::Dcs, 0x9c) => {
                        self.scan = Scan::Ground;
                    }
                    (Scan::DcsHead, 0x40..=0x7e) => self.scan = Scan::Dcs,
                    (Scan::Pass(lo), _) if (lo..=0x7e).contains(&b) => {
                        self.scan = Scan::Ground;
                        // An overlong ICH/IL/SD still carries a count vt100
                        // would loop on; CAN aborts it before dispatch.
                        if lo == 0x40 && matches!(b, b'@' | b'L' | b'T') {
                            out.extend_from_slice(&bytes[span..i]);
                            out.push(0x18);
                            span = i + 1;
                        }
                    }
                    _ => {}
                }
                continue;
            }
            span = i + 1;
            match b {
                // A new ESC aborts the held sequence (CSI) or is ignored after
                // it (ESC ESC); either way the held bytes did nothing yet.
                ESC => self.hold_esc(),
                0x18 | 0x1a => {
                    self.seq.push(b);
                    out.extend_from_slice(&self.seq);
                    self.scan = Scan::Ground;
                }
                // Other C0 controls run mid-sequence without ending it, so
                // running them before the held bytes changes nothing.
                0x00..=0x1f => out.push(b),
                // vte ignores DEL and high bytes mid-sequence.
                0x7f..=0xff => {}
                b'[' if self.scan == Scan::Esc => {
                    self.seq.push(b);
                    self.scan = Scan::Csi;
                }
                _ if self.scan == Scan::Esc => {
                    self.seq.push(b);
                    out.extend_from_slice(&self.seq);
                    self.scan = match b {
                        b']' => Scan::Osc,
                        b'P' => Scan::DcsHead,
                        b'X' | b'^' | b'_' => Scan::Str,
                        0x20..=0x2f => Scan::Pass(0x30),
                        _ => Scan::Ground,
                    };
                }
                0x40..=0x7e => {
                    self.seq.push(b);
                    self.dispatch(size, out);
                    self.scan = Scan::Ground;
                }
                _ => {
                    self.seq.push(b);
                    if self.seq.len() > CSI_HOLD_MAX {
                        out.extend_from_slice(&self.seq);
                        self.scan = Scan::Pass(0x40);
                    }
                }
            }
        }
        if span < bytes.len() {
            out.extend_from_slice(&bytes[span..]);
        }
    }

    fn hold_esc(&mut self) {
        self.seq.clear();
        self.seq.push(ESC);
        self.utf8_len = 0;
        self.scan = Scan::Esc;
    }

    /// Follow printed text to know the last graphic character, the one REP
    /// repeats. Mirrors what vt100 draws as a cell of its own: no controls, no
    /// U+FFFD, no C1, no zero-width marks (those stack onto the cell before).
    fn track(&mut self, b: u8) {
        match b {
            0x20..=0x7e => {
                self.last = Some(char::from(b));
                self.utf8_len = 0;
            }
            0x80..=0xbf if self.utf8_len > 0 => {
                self.utf8[usize::from(self.utf8_len)] = b;
                self.utf8_len += 1;
                if self.utf8_len == self.utf8_need {
                    let utf8 = &self.utf8[..usize::from(self.utf8_len)];
                    let c = std::str::from_utf8(utf8)
                        .ok()
                        .and_then(|s| s.chars().next());
                    if let Some(c) = c.filter(|&c| c != '\u{fffd}' && c.width().unwrap_or(0) > 0) {
                        self.last = Some(c);
                    }
                    self.utf8_len = 0;
                }
            }
            0xc2..=0xf4 => {
                self.utf8[0] = b;
                self.utf8_len = 1;
                // 110xxxxx, 1110xxxx, 11110xxx: the leading ones are the length.
                self.utf8_need = b.leading_ones() as u8;
            }
            _ => self.utf8_len = 0,
        }
    }

    /// Write the complete CSI sequence in `seq`, rewritten if vt100 lacks it.
    fn dispatch(&mut self, (rows, cols): (u16, u16), out: &mut Vec<u8>) {
        let end = self.seq.len() - 1;
        let (body, fin) = (&self.seq[2..end], self.seq[end]);
        // Plain parameters only: a private marker or intermediate makes it
        // another sequence (`CSI ? u` is the kitty query, not SCORC).
        let plain = body
            .iter()
            .all(|&b| b.is_ascii_digit() || b == b';' || b == b':');
        let bare = body.is_empty();
        let n = param(body);
        match fin {
            b's' if bare => out.extend_from_slice(b"\x1b7"),
            b'u' if bare => out.extend_from_slice(b"\x1b8"),
            // HVP, HPA, HPR, VPR: same parameters as CUP, CHA, CUF, CUD.
            b'f' | b'`' | b'a' | b'e' if plain => {
                self.seq[end] = match fin {
                    b'f' => b'H',
                    b'`' => b'G',
                    b'a' => b'C',
                    _ => b'B',
                };
                out.extend_from_slice(&self.seq);
            }
            // REP, bounded to `cols` characters (two rows of wide ones);
            // dropped with nothing to repeat.
            b'b' if plain => {
                if let Some(c) = self.last {
                    let mut buf = [0; 4];
                    let count = usize::from(n.max(1).min(cols));
                    let c = c.encode_utf8(&mut buf).as_bytes();
                    for _ in 0..count {
                        out.extend_from_slice(c);
                    }
                }
            }
            // vt100 loops `n` times for ICH, IL and SD; past the grid it is
            // the same blank, only seconds slower.
            b'@' if plain && n > cols => out.extend_from_slice(format!("\x1b[{cols}@").as_bytes()),
            b'L' | b'T' if plain && n > rows => {
                out.extend_from_slice(format!("\x1b[{rows}{}", char::from(fin)).as_bytes());
            }
            _ => out.extend_from_slice(&self.seq),
        }
    }
}

/// Value of one CSI parameter: its leading digits, saturating like vte.
fn param(p: &[u8]) -> u16 {
    p.iter()
        .take_while(|b| b.is_ascii_digit())
        .fold(0u16, |n, &d| {
            n.saturating_mul(10).saturating_add(u16::from(d - b'0'))
        })
}

/// Exact decoded byte length of a base64 payload, without decoding it. The
/// payload is relayed verbatim, so a real decoder would be pure waste — this
/// only exists to enforce the size cap and to size the "n bytes" notice.
pub(crate) fn decoded_len(b64: &[u8]) -> usize {
    let full = b64.len() / 4 * 3;
    match b64.len() % 4 {
        // Well-formed: subtract whatever padding is present.
        0 => full.saturating_sub(b64.iter().rev().take_while(|&&c| c == b'=').count().min(2)),
        // Unpadded tail: 2 chars carry 1 byte, 3 chars carry 2.
        2 => full + 1,
        3 => full + 2,
        // len % 4 == 1 is not valid base64; treat the stray char as nothing.
        _ => full,
    }
}

/// Why a clipboard write coming out of the PTY never made it to the queue.
/// The two reasons are counted apart so the notice can name them: one is a
/// single write past the size cap, the other a remote stuck in a copy loop.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ClipboardDrops {
    /// Writes whose decoded payload exceeded [`CLIPBOARD_RELAY_MAX_BYTES`].
    pub(crate) oversize: usize,
    /// Writes that arrived with the queue already at [`CLIPBOARD_RELAY_MAX_QUEUED`].
    pub(crate) queue_full: usize,
}

/// Everything the emulator has to answer for, collected per drain: OSC 52
/// clipboard writes headed for the real terminal, and answers to terminal
/// status queries headed back into the PTY.
///
/// Without this, `vt100` hands both to the default `Callbacks for ()` impl,
/// which silently drops them. A dropped clipboard write means anything copying
/// inside the PTY (herdr, tmux, neovim, lazygit…) appears to work but never
/// reaches the system clipboard; a dropped *query* means the application waits
/// for an answer that never comes.
#[derive(Default)]
struct PtyCallbacks {
    /// Pending base64 payloads, in arrival order.
    pending: Vec<String>,
    /// Writes rejected since the last drain, by reason.
    drops: ClipboardDrops,
    /// Answers to terminal status queries, waiting to go back into the PTY.
    replies: Vec<u8>,
}

impl vt100::Callbacks for PtyCallbacks {
    fn copy_to_clipboard(&mut self, _: &mut vt100::Screen, _ty: &[u8], data: &[u8]) {
        // An empty payload is a clipboard *clear* on terminals that honour it.
        // We neither forward it nor count it as a drop: a remote must not be
        // able to wipe the local clipboard, and nothing was lost worth naming.
        if data.is_empty() {
            return;
        }
        // Order matters — a huge write is reported as oversize even when the
        // queue happens to be full as well.
        if decoded_len(data) > CLIPBOARD_RELAY_MAX_BYTES {
            self.drops.oversize += 1;
            return;
        }
        if self.pending.len() >= CLIPBOARD_RELAY_MAX_QUEUED {
            self.drops.queue_full += 1;
            return;
        }
        // vt100 already guaranteed every byte is in the base64 alphabet
        // (including '='), so this is ASCII and the payload passes through
        // unchanged — no decode, no re-encode.
        if let Ok(payload) = std::str::from_utf8(data) {
            self.pending.push(payload.to_string());
        }
    }

    // `paste_from_clipboard` is deliberately left as the no-op default:
    // answering `ESC]52;c;?BEL` would let any host we're SSH'd into *read* the
    // local clipboard, which is far worse than a write and buys us nothing.

    /// vt100 implements no terminal *query* at all, so every one of them lands
    /// here — and an application that asks and hears nothing back blocks until
    /// its own timeout expires. atuin's history search dies outright ("The
    /// cursor position could not be read within a normal duration", #113), and
    /// every crossterm-based TUI stalls two seconds at startup probing for the
    /// kitty keyboard protocol. Answering is simply what the terminal on the
    /// other side does when `ssh` runs without sshub in front of it.
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        _i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        // Private sequences (`CSI ? … u`, the kitty keyboard protocol query;
        // `CSI > c`, secondary device attributes) stay unanswered on purpose.
        // We don't speak them, and claiming otherwise is worse than silence:
        // crossterm reads a missing `?u` reply *plus* the DA1 answer below as
        // a definitive "not supported", which is the truth.
        if i1.is_some() {
            return;
        }
        let param = params.first().and_then(|p| p.first().copied()).unwrap_or(0);
        let reply = match (c, param) {
            // DSR 6 — cursor position report. Our grid mirrors the remote
            // screen, so its cursor *is* the answer. Reported 1-based, and
            // clamped: vt100 parks the cursor one column *past* the right
            // margin after a character lands in the last column (the pending
            // wrap is only resolved when the next one arrives), so a prompt
            // that fills the line would otherwise be reported at column
            // `cols + 1` — a real terminal answers `cols`, and code measuring
            // the room left underflows on anything else. Origin mode is the
            // one case we still get wrong: vt100 handles DECOM itself and
            // exposes no accessor, so the row here is absolute where a
            // conformant terminal would report it relative to the region.
            ('n', 6) => {
                let (row, col) = screen.cursor_position();
                let (rows, cols) = screen.size();
                let (row, col) = ((row + 1).min(rows), (col + 1).min(cols));
                format!("\x1b[{row};{col}R").into_bytes()
            }
            // DSR 5 — device status. Nothing can go wrong in an in-memory grid.
            ('n', 5) => b"\x1b[0n".to_vec(),
            // DA1 — device attributes. VT100 with the advanced video option is
            // the honest floor for what vt100 emulates; callers only care that
            // an answer arrives at all, not what it claims.
            ('c', 0) => b"\x1b[?1;2c".to_vec(),
            _ => return,
        };
        if self.replies.len() + reply.len() <= REPLY_QUEUE_MAX_BYTES {
            self.replies.extend_from_slice(&reply);
        }
    }
}

pub struct ParserState {
    inner: vt100::Parser<PtyCallbacks>,
    compat: XtermCompat,
    /// Translator output for the read being processed; reused.
    out: Vec<u8>,
}

impl ParserState {
    pub fn new(rows: u16, cols: u16) -> Self {
        Self {
            inner: vt100::Parser::new_with_callbacks(rows, cols, 10_000, PtyCallbacks::default()),
            compat: XtermCompat::default(),
            out: Vec::new(),
        }
    }

    /// Take the answers to terminal queries seen since the last call. Unlike a
    /// clipboard write these go straight back into the PTY, not to the host
    /// terminal, and every session owes them whether or not it is on screen.
    pub(crate) fn take_replies(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.inner.callbacks_mut().replies)
    }

    /// Take the drops recorded since the last call, resetting the counters.
    pub(crate) fn take_clipboard_drops(&mut self) -> ClipboardDrops {
        std::mem::take(&mut self.inner.callbacks_mut().drops)
    }

    /// Take the clipboard writes seen since the last call. Each entry is a
    /// base64 payload ready to hand to [`crate::osc52::write_b64`].
    pub(crate) fn take_clipboard_writes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.inner.callbacks_mut().pending)
    }

    pub fn process(&mut self, bytes: &[u8]) {
        let size = self.inner.screen().size();
        let mut out = std::mem::take(&mut self.out);
        out.clear();
        self.compat.feed(bytes, size, &mut out);
        self.inner.process(&out);
        self.out = out;
    }

    pub fn set_size(&mut self, rows: u16, cols: u16) {
        self.inner.screen_mut().set_size(rows, cols);
    }

    pub fn screen(&self) -> &vt100::Screen {
        self.inner.screen()
    }

    /// Current scrollback offset (0 = pinned to bottom).
    pub fn scrollback(&self) -> usize {
        self.inner.screen().scrollback()
    }

    pub fn set_scrollback(&mut self, rows: usize) {
        // vt100 caps the value at `scrollback.len()` internally; the
        // out-of-range panic that forced our old vendored fork was fixed
        // upstream in 0.16, so any value up to the full buffer is safe.
        self.inner.screen_mut().set_scrollback(rows);
    }

    /// Bump the scrollback offset up by `rows` (showing older content).
    pub fn scroll_up(&mut self, rows: usize) {
        let next = self.scrollback().saturating_add(rows);
        self.set_scrollback(next);
    }

    /// Reduce the scrollback offset by `rows` (toward the live view).
    pub fn scroll_down(&mut self, rows: usize) {
        let next = self.scrollback().saturating_sub(rows);
        self.set_scrollback(next);
    }

    pub fn snap_to_bottom(&mut self) {
        self.inner.screen_mut().set_scrollback(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drops(oversize: usize, queue_full: usize) -> ClipboardDrops {
        ClipboardDrops {
            oversize,
            queue_full,
        }
    }

    fn parser_with(rows: u16, cols: u16, stream: &[u8]) -> ParserState {
        let mut p = ParserState::new(rows, cols);
        p.process(stream);
        p
    }

    /// Reproduces the bug: scrolling past the screen height used to panic
    /// (vt100 0.15.2 underflow). Vendored patch must keep it from crashing
    /// and must let us actually read older rows.
    #[test]
    fn scrollback_beyond_screen_height_does_not_panic() {
        // Print 100 numbered lines on a 10-row terminal.
        let mut bytes = Vec::new();
        for i in 1..=100 {
            bytes.extend_from_slice(format!("line-{i:03}\r\n").as_bytes());
        }
        let mut p = parser_with(10, 80, &bytes);

        // Way past one screen — would have panicked pre-patch.
        p.set_scrollback(60);
        assert_eq!(p.scrollback(), 60);

        // Top visible row should be ~50 rows back from "line-100".
        let first_visible_text: String = (0..10)
            .filter_map(|col| p.screen().cell(0, col).map(|c| c.contents()))
            .collect();
        assert!(
            first_visible_text.starts_with("line-"),
            "top row should be a numbered line, got {first_visible_text:?}"
        );
    }

    #[test]
    fn snap_returns_to_zero_offset() {
        let mut p = ParserState::new(10, 80);
        p.process(b"hello\r\n");
        p.set_scrollback(5);
        p.snap_to_bottom();
        assert_eq!(p.scrollback(), 0);
    }

    // ── OSC 52 clipboard relay ────────────────────────────────────
    //
    // An app running inside the PTY (herdr, tmux, neovim, lazygit…) copies by
    // writing `ESC ] 52 ; c ; <base64> BEL` to its stdout — which is our PTY.
    // vt100 parses that and hands it to `Callbacks::copy_to_clipboard`, whose
    // default `()` impl silently drops it. We queue it instead so `drain()` can
    // re-emit it toward the real terminal.

    #[test]
    fn osc52_copy_is_queued_for_relay() {
        // base64("GEHEIM") == "R0VIRUlN"
        let mut p = parser_with(10, 80, b"\x1b]52;c;R0VIRUlN\x07");
        assert_eq!(p.take_clipboard_writes(), vec!["R0VIRUlN".to_string()]);
    }

    #[test]
    fn padded_base64_is_relayed() {
        // Guards the whole feature: vt100's BASE64 alphabet includes '=', so a
        // padded payload (what herdr actually emits) must survive. If '=' ever
        // stopped being accepted upstream, every short copy would break.
        let mut p = parser_with(10, 80, b"\x1b]52;c;aGVsbG8=\x07");
        assert_eq!(p.take_clipboard_writes(), vec!["aGVsbG8=".to_string()]);
    }

    #[test]
    fn take_clipboard_writes_drains() {
        let mut p = parser_with(10, 80, b"\x1b]52;c;R0VIRUlN\x07");
        assert_eq!(p.take_clipboard_writes().len(), 1);
        assert!(p.take_clipboard_writes().is_empty());
    }

    #[test]
    fn oversized_clipboard_write_is_dropped() {
        // 90_000 base64 chars ≈ 67.5 KiB decoded — past the 64 KiB cap.
        let mut p = parser_with(10, 80, &oversized_copy());
        assert!(p.take_clipboard_writes().is_empty());
        assert_eq!(p.take_clipboard_drops(), drops(1, 0));
    }

    #[test]
    fn queue_is_capped() {
        let mut stream = Vec::new();
        for _ in 0..20 {
            stream.extend_from_slice(b"\x1b]52;c;R0VIRUlN\x07");
        }
        let mut p = parser_with(10, 80, &stream);
        assert_eq!(p.take_clipboard_writes().len(), CLIPBOARD_RELAY_MAX_QUEUED);
        assert_eq!(
            p.take_clipboard_drops(),
            drops(0, 20 - CLIPBOARD_RELAY_MAX_QUEUED)
        );
    }

    /// An OSC 52 write whose decoded payload is past the size cap.
    fn oversized_copy() -> Vec<u8> {
        let mut stream = b"\x1b]52;c;".to_vec();
        stream.extend(std::iter::repeat_n(b'A', 90_000));
        stream.push(0x07);
        stream
    }

    #[test]
    fn empty_payload_is_ignored_entirely() {
        // `ESC]52;c;BEL` clears the clipboard on terminals that honour it. We
        // neither relay it nor treat it as a drop: an empty write must not
        // wipe the user's clipboard and must not claim anything happened.
        let mut p = parser_with(10, 80, b"\x1b]52;c;\x07");
        assert!(p.take_clipboard_writes().is_empty());
        assert_eq!(p.take_clipboard_drops(), ClipboardDrops::default());
    }

    #[test]
    fn oversize_and_queue_full_are_counted_separately() {
        // The two drop reasons are different failures — one is a single huge
        // write, the other a remote in a copy loop — so the notice must be
        // able to tell them apart.
        let mut stream = oversized_copy();
        for _ in 0..20 {
            stream.extend_from_slice(b"\x1b]52;c;R0VIRUlN\x07");
        }
        let mut p = parser_with(10, 80, &stream);
        assert_eq!(p.take_clipboard_writes().len(), CLIPBOARD_RELAY_MAX_QUEUED);
        assert_eq!(
            p.take_clipboard_drops(),
            drops(1, 20 - CLIPBOARD_RELAY_MAX_QUEUED)
        );
    }

    #[test]
    fn taking_drops_resets_the_counters() {
        let mut p = parser_with(10, 80, &oversized_copy());
        assert_eq!(p.take_clipboard_drops(), drops(1, 0));
        assert_eq!(p.take_clipboard_drops(), ClipboardDrops::default());
    }

    #[test]
    fn primary_selection_is_relayed_as_clipboard() {
        // vt100 hands us selector `p` (X11 primary selection) too. We
        // deliberately normalise every selector to `c` in the shared helper,
        // so the payload must reach the queue unchanged.
        let mut p = parser_with(10, 80, b"\x1b]52;p;R0VIRUlN\x07");
        assert_eq!(p.take_clipboard_writes(), vec!["R0VIRUlN".to_string()]);
    }

    #[test]
    fn paste_query_is_not_answered() {
        // `ESC]52;c;?BEL` asks us to hand the clipboard *back* to the remote.
        // Answering would let any host we're SSH'd into read the local
        // clipboard, so it must produce nothing at all.
        let mut p = parser_with(10, 80, b"\x1b]52;c;?\x07");
        assert!(p.take_clipboard_writes().is_empty());
    }

    #[test]
    fn invalid_base64_is_ignored() {
        // vt100 routes non-base64 payloads to `unhandled_osc`, never to us.
        let mut p = parser_with(10, 80, b"\x1b]52;c;not base64!\x07");
        assert!(p.take_clipboard_writes().is_empty());
    }

    #[test]
    fn osc52_does_not_reach_the_grid() {
        // Regression: the sequence must stay invisible. If it ever landed on a
        // cell the user would see escape gibberish mid-session.
        let mut p = parser_with(10, 80, b"before\x1b]52;c;R0VIRUlN\x07after");
        assert_eq!(p.screen().contents().trim(), "beforeafter");
        assert_eq!(p.take_clipboard_writes().len(), 1);
    }

    // ── Terminal status queries ───────────────────────────────────
    //
    // vt100 implements none of these, so they arrive at `unhandled_csi`. An
    // application that asks and hears nothing back hangs on its own timeout.

    #[test]
    fn cursor_position_report_answers_the_grid_cursor() {
        // `CSI 6 n` is what crossterm's `cursor::position()` writes, and what
        // atuin's history search needs before it can draw (#113). The answer is
        // 1-based, so a cursor parked after "hi" on the first row is (1, 3).
        let mut p = parser_with(10, 80, b"hi\x1b[6n");
        assert_eq!(p.take_replies(), b"\x1b[1;3R".to_vec());
    }

    #[test]
    fn cursor_position_report_follows_the_cursor() {
        // Same query from elsewhere on the grid must not hand back a constant.
        let mut p = parser_with(10, 80, b"\x1b[5;7H\x1b[6n");
        assert_eq!(p.take_replies(), b"\x1b[5;7R".to_vec());
    }

    #[test]
    fn cursor_position_report_is_clamped_at_the_right_margin() {
        // vt100 parks the cursor at column `cols` (0-based, i.e. one past the
        // last cell) once a character lands in the last column, resolving the
        // wrap only when the next one arrives. Unclamped that reports column
        // `cols + 1` — a column the terminal does not have. A prompt that
        // fills the line and then asks where it is gets this every time, and
        // whoever measures the room left from it underflows.
        let mut p = parser_with(3, 10, b"0123456789\x1b[6n");
        assert_eq!(p.screen().cursor_position(), (0, 10), "vt100 behaviour");
        assert_eq!(p.take_replies(), b"\x1b[1;10R".to_vec());
    }

    #[test]
    fn device_status_report_answers_ok() {
        let mut p = parser_with(10, 80, b"\x1b[5n");
        assert_eq!(p.take_replies(), b"\x1b[0n".to_vec());
    }

    #[test]
    fn device_attributes_are_answered_with_and_without_a_param() {
        // crossterm probes kitty-keyboard support with `ESC[?u ESC[c` and waits
        // two seconds for *either* reply. The DA1 answer is what ends that wait.
        for query in [&b"\x1b[c"[..], &b"\x1b[0c"[..]] {
            let mut p = parser_with(10, 80, query);
            assert_eq!(p.take_replies(), b"\x1b[?1;2c".to_vec(), "query {query:?}");
        }
    }

    #[test]
    fn private_queries_stay_unanswered() {
        // We don't speak the kitty keyboard protocol (`CSI ? u`) or secondary
        // device attributes (`CSI > c`). Silence is the honest answer, and it's
        // what makes crossterm conclude "unsupported" once DA1 arrives.
        let mut p = parser_with(10, 80, b"\x1b[?u\x1b[>c\x1b[?6n");
        assert!(p.take_replies().is_empty());
    }

    #[test]
    fn unknown_dsr_parameters_are_not_answered() {
        // Making something up for a query we don't recognise is worse than not
        // replying: the application would parse our answer as the wrong event.
        let mut p = parser_with(10, 80, b"\x1b[n\x1b[99n");
        assert!(p.take_replies().is_empty());
    }

    #[test]
    fn take_replies_drains() {
        let mut p = parser_with(10, 80, b"\x1b[5n");
        assert_eq!(p.take_replies().len(), 4);
        assert!(p.take_replies().is_empty());
    }

    #[test]
    fn reply_queue_is_capped() {
        // A remote spinning on `CSI 5 n` must not grow our buffer without
        // bound between drains.
        let mut stream = Vec::new();
        for _ in 0..1000 {
            stream.extend_from_slice(b"\x1b[5n");
        }
        let mut p = parser_with(10, 80, &stream);
        // The invariant is the bound, not an exact number: only whole replies
        // are queued, so where the cap lands depends on how long they are.
        let queued = p.take_replies().len();
        assert!(queued <= REPLY_QUEUE_MAX_BYTES, "over the cap: {queued}");
        assert!(
            queued > REPLY_QUEUE_MAX_BYTES - 4,
            "cap not actually reached: {queued}"
        );
    }

    #[test]
    fn queries_do_not_reach_the_grid() {
        // Regression: the query must stay invisible. If it ever landed on a
        // cell the user would see escape gibberish mid-session.
        let mut p = parser_with(10, 80, b"before\x1b[6nafter");
        assert_eq!(p.screen().contents().trim(), "beforeafter");
        assert_eq!(p.take_replies(), b"\x1b[1;7R".to_vec());
    }

    #[test]
    fn decoded_len_matches_real_decode() {
        // Exact decoded size without pulling in a base64 decoder — the payload
        // is relayed verbatim, so decoding it would be pure waste.
        assert_eq!(decoded_len(b""), 0);
        assert_eq!(decoded_len(b"R0VIRUlN"), 6); // "GEHEIM"
        assert_eq!(decoded_len(b"aGVsbG8="), 5); // "hello", 1 pad
        assert_eq!(decoded_len(b"aGk="), 2); // "hi",    1 pad
        assert_eq!(decoded_len(b"YQ=="), 1); // "a",     2 pads
        assert_eq!(decoded_len(b"YWJjZA=="), 4); // "abcd",  2 pads
    }

    #[test]
    fn decoded_len_handles_unpadded_input() {
        // Some senders omit padding; vt100 accepts it, so we must size it right.
        assert_eq!(decoded_len(b"aGVsbG8"), 5); // "hello" unpadded
        assert_eq!(decoded_len(b"aGk"), 2); // "hi"    unpadded
        assert_eq!(decoded_len(b"YQ"), 1); // "a"     unpadded
    }

    // ── xterm compatibility translator ────────────────────────────
    //
    // Stock vt100 0.16 drops these xterm sequences; `XtermCompat` rewrites
    // them into ones it handles before the bytes reach the parser.

    /// Run `chunks` through the translator alone, as successive PTY reads.
    fn translate(size: (u16, u16), chunks: &[&[u8]]) -> Vec<u8> {
        let mut compat = XtermCompat::default();
        let mut out = Vec::new();
        for chunk in chunks {
            compat.feed(chunk, size, &mut out);
        }
        out
    }

    fn row(p: &ParserState, r: usize) -> String {
        let (_, cols) = p.screen().size();
        p.screen().rows(0, cols).nth(r).unwrap_or_default()
    }

    #[test]
    fn hvp_places_text_like_cup() {
        // btop positions everything with HVP; ignored, a frame lands on one line.
        let p = parser_with(24, 80, b"\x1b[2J\x1b[HAAA\x1b[10;20fBBB");
        assert_eq!(row(&p, 0), "AAA");
        assert_eq!(row(&p, 9), format!("{}BBB", " ".repeat(19)));
    }

    #[test]
    fn hpa_hpr_vpr_move_like_cha_cuf_cud() {
        assert_eq!(row(&parser_with(5, 20, b"\x1b[5`X"), 0), "    X");
        assert_eq!(row(&parser_with(5, 20, b"AB\x1b[3aC"), 0), "AB   C");
        assert_eq!(row(&parser_with(5, 20, b"\x1b[2eX"), 2), "X");
    }

    #[test]
    fn scosc_and_scorc_save_and_restore_the_cursor() {
        let p = parser_with(10, 20, b"\x1b[5;5H\x1b[s\x1b[1;1HX\x1b[uY");
        assert_eq!(row(&p, 4), "    Y");
    }

    #[test]
    fn only_bare_csi_s_and_u_mean_cursor_save() {
        // `CSI ? u` (kitty query), `CSI > 1 u`, `CSI < u`, `CSI 1 u` must not
        // restore the cursor saved at (2, 2).
        let p = parser_with(
            10,
            20,
            b"\x1b[3;3H\x1b7\x1b[1;1H\x1b[?u\x1b[>1u\x1b[<u\x1b[1u",
        );
        assert_eq!(p.screen().cursor_position(), (0, 0));
        // `CSI 1;5 s` (DECSLRM) must not save over it.
        let p = parser_with(10, 20, b"\x1b[3;3H\x1b7\x1b[5;5H\x1b[1;5s\x1b[1;1H\x1b8");
        assert_eq!(p.screen().cursor_position(), (2, 2));
    }

    #[test]
    fn rewrites_survive_every_split_between_reads() {
        let stream: &[u8] =
            b"\x1b[2J\x1b[HAAA\x1b[10;20fBBB\x1b[5`X\x1b[s\x1b[3;3HY\x1b[uZ\xe4\xb8\xad\x1b[2b";
        let p = parser_with(24, 80, stream);
        assert_eq!(row(&p, 2), "  Y");
        assert_eq!(row(&p, 9), "    XZ中中中       BBB");
        let whole = p.screen().contents_formatted();
        for k in 0..=stream.len() {
            let mut p = ParserState::new(24, 80);
            p.process(&stream[..k]);
            p.process(&stream[k..]);
            assert_eq!(p.screen().contents_formatted(), whole, "split at {k}");
        }
        let mut p = ParserState::new(24, 80);
        for b in stream {
            p.process(std::slice::from_ref(b));
        }
        assert_eq!(p.screen().contents_formatted(), whole, "byte at a time");
    }

    #[test]
    fn rep_repeats_the_last_character() {
        assert_eq!(row(&parser_with(5, 20, b"a\x1b[3b"), 0), "aaaa");
        assert_eq!(row(&parser_with(5, 20, b"a\x1b[b"), 0), "aa");
        assert_eq!(row(&parser_with(5, 20, b"a\x1b[0b"), 0), "aa");
    }

    #[test]
    fn rep_is_bounded_by_the_row_width() {
        let p = parser_with(3, 10, b"x\x1b[65535b");
        assert_eq!(p.screen().contents().matches('x').count(), 11);
    }

    #[test]
    fn rep_remembers_a_wide_character_across_reads() {
        let wide = "中".as_bytes();
        let mut p = ParserState::new(5, 20);
        p.process(&wide[..1]);
        p.process(&wide[1..]);
        p.process(b"\x1b[2b");
        assert_eq!(row(&p, 0), "中中中");
    }

    #[test]
    fn rep_with_nothing_printed_before_it_is_dropped() {
        assert_eq!(row(&parser_with(5, 20, b"\x1b[5b"), 0), "");
        // Payload bytes of a string are not printed text.
        assert_eq!(row(&parser_with(5, 20, b"a\x1b]0;bbb\x07\x1b[b"), 0), "aa");
        assert_eq!(row(&parser_with(5, 20, b"a\x1bPq#0\x1b\\\x1b[b"), 0), "aa");
    }

    #[test]
    fn string_payloads_pass_through_unchanged() {
        // Only a CAN lands in front of each ESC that ends a string.
        let stream: &[u8] = b"\x1b]0;[1;2f title [s\x07\x1b]52;c;R0VIRUlN\x1b\\\
            \x1bP1$r[5;5f\x1b\\\x1b_[u\x1b\\";
        let expected: &[u8] = b"\x1b]0;[1;2f title [s\x07\x1b]52;c;R0VIRUlN\x18\x1b\\\
            \x1bP1$r[5;5f\x18\x1b\\\x1b_[u\x18\x1b\\";
        for k in 0..=stream.len() {
            assert_eq!(
                translate((24, 80), &[&stream[..k], &stream[k..]]),
                expected,
                "split {k}"
            );
        }
    }

    #[test]
    fn a_string_ended_by_a_bare_esc_is_left_at_once() {
        // vte ends the string at the ESC itself; holding that ESC used to keep
        // vt100 inside the string while REP output and controls went in.
        assert_eq!(row(&parser_with(5, 20, b"a\x1b]0;t\x1b[2bZ"), 0), "aaaZ");
        assert_eq!(row(&parser_with(5, 20, b"\x1b]0;t\x1b[2bZ"), 0), "Z");
        let mut p = parser_with(5, 20, b"a\x1b]52;c;R0VIRUlN\x1b[2bxyz");
        assert_eq!(p.take_clipboard_writes(), vec!["R0VIRUlN".to_string()]);
        assert_eq!(row(&p, 0), "aaaxyz");
        let stream = b"ab\x1b]0;t\x1b\nY";
        let mut raw = vt100::Parser::new(5, 20, 0);
        raw.process(stream);
        assert_eq!(raw.screen().cursor_position(), (1, 2));
        assert_eq!(
            parser_with(5, 20, stream).screen().cursor_position(),
            (1, 2)
        );
    }

    #[test]
    fn an_esc_aborting_a_held_csi_drops_it() {
        // `ESC [ 1` never completes; emitting it ahead of the REP output made
        // the first repeated `a` its final byte.
        assert_eq!(row(&parser_with(5, 20, b"a\x1b[1\x1b[2bZ"), 0), "aaaZ");
    }

    #[test]
    fn a_raw_c1_st_ends_a_dcs_for_rep() {
        assert_eq!(row(&parser_with(5, 20, b"a\x1bPq\x9cz\x1b[3b"), 0), "azzzz");
    }

    #[test]
    fn rep_skips_zero_width_marks() {
        let p = parser_with(5, 20, "e\u{301}\x1b[2b".as_bytes());
        let cell = |c| p.screen().cell(0, c).unwrap().contents().to_string();
        assert_eq!(
            (cell(0), cell(1), cell(2)),
            ("e\u{301}".into(), "e".into(), "e".into())
        );
    }

    /// Streams made only of sequences the translator must leave alone render
    /// exactly as on stock vt100: same screen, cursor and replies, also when
    /// split at any offset. Fixed seed, so a failure reproduces.
    #[test]
    fn untouched_sequences_render_like_stock_vt100() {
        const PIECES: &[&[u8]] = &[
            b"a",
            b"Z",
            b"x ",
            "é".as_bytes(),
            "中".as_bytes(),
            b"\r",
            b"\n",
            b"\t",
            b"\x08",
            b"\x07",
            b"\x18",
            b"\x1a",
            b"\x1b]0;t[1;2f\x07",
            b"\x1b]2;[s\x1b\\",
            b"\x1b]0;bare[",
            b"\x1b]52;c;R0VIRUlN\x07",
            b"\x1b]52;c;aGk=\x1b",
            b"\x1bP1$r[5f\x1b\\",
            b"\x1bPq#0",
            b"\x1b_[u\x1b\\",
            b"\x1b^x\x1b",
            b"\x1b[?25l",
            b"\x1b[?25h",
            b"\x1b[?u",
            b"\x1b[>c",
            b"\x1b[?6n",
            b"\x1b[>1u",
            b"\x1b[<u",
            b"\x1b[6n",
            b"\x1b[5n",
            b"\x1b[c",
            b"\x1b[3;4H",
            b"\x1b[2J",
            b"\x1b[K",
            b"\x1b[1;31m",
            b"\x1b[5;6\x1b",
            b"\x1b[?3;4\x18",
            b"\x1b(B",
            b"\x1b7",
            b"\x1b8",
            b"\x1b",
        ];
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for n in 0..3000 {
            let len = 1 + next() % 40;
            let stream: Vec<u8> = (0..len)
                .flat_map(|_| {
                    PIECES[(next() % PIECES.len() as u64) as usize]
                        .iter()
                        .copied()
                })
                .collect();
            let splits: Vec<usize> = if n < 30 {
                (0..=stream.len()).collect()
            } else {
                vec![0]
            };
            for k in splits {
                // Raw vt100 gets the same two reads: vte 0.15 itself mishandles
                // a UTF-8 character split across reads, which is not ours to
                // compare against.
                let mut raw =
                    vt100::Parser::new_with_callbacks(6, 20, 10_000, PtyCallbacks::default());
                raw.process(&stream[..k]);
                raw.process(&stream[k..]);
                let want = (
                    raw.screen().contents_formatted(),
                    raw.screen().cursor_position(),
                );
                let want_replies = raw.callbacks().replies.clone();
                let mut p = ParserState::new(6, 20);
                p.process(&stream[..k]);
                p.process(&stream[k..]);
                let got = (
                    p.screen().contents_formatted(),
                    p.screen().cursor_position(),
                );
                assert!(
                    got == want,
                    "stream {n} split {k}: {:?}",
                    String::from_utf8_lossy(&stream)
                );
                assert_eq!(
                    p.take_replies(),
                    want_replies,
                    "replies, stream {n} split {k}"
                );
            }
        }
    }

    #[test]
    fn osc52_relay_survives_split_reads() {
        let mut p = ParserState::new(10, 80);
        for chunk in [&b"\x1b]5"[..], b"2;c;R0VI", b"RUlN\x07"] {
            p.process(chunk);
        }
        assert_eq!(p.take_clipboard_writes(), vec!["R0VIRUlN".to_string()]);
    }

    #[test]
    fn ich_il_sd_counts_are_clamped_to_the_grid() {
        // vt100 loops once per count: a single `CSI 65535 @` takes seconds.
        let t = |s: &[u8]| translate((24, 80), &[s]);
        assert_eq!(t(b"\x1b[65535@"), b"\x1b[80@");
        assert_eq!(t(b"\x1b[65535:1@"), b"\x1b[80@");
        assert_eq!(t(b"\x1b[65535L"), b"\x1b[24L");
        assert_eq!(t(b"\x1b[65535T"), b"\x1b[24T");
        assert_eq!(t(b"\x1b[80@\x1b[24L\x1b[2T"), b"\x1b[80@\x1b[24L\x1b[2T");

        let p = parser_with(24, 80, b"abc\x1b[1;1H\x1b[65535@");
        assert_eq!(row(&p, 0), "");
        let p = parser_with(5, 10, b"1\r\n2\r\n3\x1b[1;1H\x1b[65535L");
        assert_eq!(p.screen().contents().trim(), "");
        let p = parser_with(5, 10, b"1\r\n2\r\n3\x1b[65535T");
        assert_eq!(p.screen().contents().trim(), "");
    }

    #[test]
    fn overlong_count_sequence_is_aborted() {
        // Zero padding pushes it past the hold limit; it must not reach vt100
        // with its 65535 count intact.
        let mut ich = b"\x1b[".to_vec();
        ich.extend(std::iter::repeat_n(b'0', 300));
        ich.extend_from_slice(b"65535@");
        let mut aborted = ich.clone();
        *aborted.last_mut().unwrap() = 0x18;
        assert_eq!(translate((24, 80), &[&ich]), aborted);

        let mut stream = b"abc\x1b[1;1H".to_vec();
        stream.extend_from_slice(&ich);
        assert_eq!(row(&parser_with(24, 80, &stream), 0), "abc");
    }

    #[test]
    fn replies_keep_their_order_across_rewrites_and_split_reads() {
        let mut p = ParserState::new(10, 80);
        p.process(b"\x1b[5n\x1b[2;3f\x1b[6");
        p.process(b"n\x1b[?u\x1b[>c\x1b[c");
        assert_eq!(p.take_replies(), b"\x1b[0n\x1b[2;3R\x1b[?1;2c".to_vec());
    }

    #[test]
    fn smallest_pty_survives_wrapping_and_wide_text() {
        let (rows, cols) = crate::session::pty_size(0, 0);
        let mut p = ParserState::new(rows, cols);
        p.process("abcdefghijklmnop中文中文\r\nx中".as_bytes());
    }
}
