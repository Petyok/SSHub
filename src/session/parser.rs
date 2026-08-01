//! VT100 parser wrapper. Maintains an in-memory `vt100::Screen` that the
//! renderer reads via `tui-term`, and relays OSC 52 clipboard writes that
//! applications inside the PTY emit.

/// Largest decoded payload we'll relay from the PTY to the host clipboard
/// (64 KiB). Keeps a remote from flooding the clipboard with a huge write.
const CLIPBOARD_RELAY_MAX_BYTES: usize = 64 * 1024;

/// How many clipboard writes we buffer between drains. A remote stuck in a
/// copy loop can't grow the queue without bound; the excess is dropped.
const CLIPBOARD_RELAY_MAX_QUEUED: usize = 8;

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

/// Collects OSC 52 clipboard writes coming out of the PTY so the session can
/// re-emit them toward the real terminal.
///
/// Without this, `vt100` parses `ESC ] 52 ; c ; <base64> BEL` and hands it to
/// the default `Callbacks for ()` impl, which silently drops it — so anything
/// copying inside the PTY (herdr, tmux, neovim, lazygit…) appears to work but
/// never reaches the system clipboard.
#[derive(Default)]
pub struct ClipboardRelay {
    /// Pending base64 payloads, in arrival order.
    pending: Vec<String>,
}

impl vt100::Callbacks for ClipboardRelay {
    fn copy_to_clipboard(&mut self, _: &mut vt100::Screen, _ty: &[u8], data: &[u8]) {
        if self.pending.len() >= CLIPBOARD_RELAY_MAX_QUEUED
            || decoded_len(data) > CLIPBOARD_RELAY_MAX_BYTES
        {
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
}

/// Re-emit an already-base64-encoded payload as OSC 52 on our own stdout, so
/// the terminal hosting SSHub puts it on the system clipboard.
///
/// The selector is pinned to `c` rather than forwarded: some terminals discard
/// the whole sequence when they see a selector they don't know.
pub fn emit_osc52_b64(payload: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    out.write_all(format!("\x1b]52;c;{payload}\x07").as_bytes())?;
    out.flush()
}

pub struct ParserState {
    inner: vt100::Parser<ClipboardRelay>,
}

impl ParserState {
    pub fn new(rows: u16, cols: u16) -> Self {
        Self {
            inner: vt100::Parser::new_with_callbacks(rows, cols, 10_000, ClipboardRelay::default()),
        }
    }

    /// Take the clipboard writes seen since the last call. Each entry is a
    /// base64 payload ready to hand to [`emit_osc52_b64`].
    pub fn take_clipboard_writes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.inner.callbacks_mut().pending)
    }

    pub fn process(&mut self, bytes: &[u8]) {
        self.inner.process(bytes);
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
        let mut stream = b"\x1b]52;c;".to_vec();
        stream.extend(std::iter::repeat_n(b'A', 90_000));
        stream.push(0x07);
        let mut p = parser_with(10, 80, &stream);
        assert!(p.take_clipboard_writes().is_empty());
    }

    #[test]
    fn queue_is_capped() {
        let mut stream = Vec::new();
        for _ in 0..20 {
            stream.extend_from_slice(b"\x1b]52;c;R0VIRUlN\x07");
        }
        let mut p = parser_with(10, 80, &stream);
        assert_eq!(p.take_clipboard_writes().len(), CLIPBOARD_RELAY_MAX_QUEUED);
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
}
