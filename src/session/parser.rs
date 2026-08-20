//! VT100 parser wrapper. Maintains an in-memory `vt100::Screen` that the
//! renderer reads via `tui-term`, relays OSC 52 clipboard writes that
//! applications inside the PTY emit, and answers the terminal status queries
//! they send — `vt100` implements none of the latter, and an application that
//! asks and hears nothing back blocks until its own timeout.

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
}

impl ParserState {
    pub fn new(rows: u16, cols: u16) -> Self {
        Self {
            inner: vt100::Parser::new_with_callbacks(rows, cols, 10_000, PtyCallbacks::default()),
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
}
