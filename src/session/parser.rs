//! VT100 parser wrapper. Maintains an in-memory `vt100::Screen` that the
//! renderer reads via `tui-term`, relays OSC 52 clipboard writes that
//! applications inside the PTY emit, and answers the terminal status queries
//! they send — `vt100` implements none of the latter, and an application that
//! asks and hears nothing back blocks until its own timeout.

use std::time::{Duration, Instant};

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

/// Real terminals fail open synchronized output rather than allowing a broken
/// program to freeze the display forever. One second is intentionally generous
/// for remote links while still bounding the damage from a missing ESU.
const SYNC_RENDER_HOLD_MAX: Duration = Duration::from_secs(1);

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalQuery {
    DeviceStatus,
    CursorPosition { private: bool },
    PrimaryDeviceAttributes,
    SecondaryDeviceAttributes,
    KittyKeyboard,
    CharacterSize,
    SynchronizedUpdateMode,
}

/// Observe queries using the same ANSI state machine as the screen parser.
/// This preserves split sequences, cancellations and OSC/DCS string context.
#[derive(Default)]
struct QueryObserver {
    query: Option<TerminalQuery>,
}

impl vte::Perform for QueryObserver {
    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        ignore: bool,
        action: char,
    ) {
        if ignore {
            return;
        }
        let mut params = params.iter();
        let Some(&[param]) = params.next() else {
            return;
        };
        if params.next().is_some() {
            return;
        }
        self.query = match (intermediates, action, param) {
            ([], 'n', 5) => Some(TerminalQuery::DeviceStatus),
            ([], 'n', 6) => Some(TerminalQuery::CursorPosition { private: false }),
            ([b'?'], 'n', 6) => Some(TerminalQuery::CursorPosition { private: true }),
            ([], 'c', 0) => Some(TerminalQuery::PrimaryDeviceAttributes),
            ([b'>'], 'c', 0) => Some(TerminalQuery::SecondaryDeviceAttributes),
            ([b'?'], 'u', 0) => Some(TerminalQuery::KittyKeyboard),
            ([], 't', 18) => Some(TerminalQuery::CharacterSize),
            ([b'?', b'$'], 'p', 2026) => Some(TerminalQuery::SynchronizedUpdateMode),
            _ => None,
        };
    }
}

pub struct ParserState {
    inner: vt100::Parser<PtyCallbacks>,
    query_parser: vte::Parser,
    sync_starts: u32,
    sync_started_at: Option<Instant>,
}

impl ParserState {
    pub fn new(rows: u16, cols: u16) -> Self {
        Self {
            inner: vt100::Parser::new_with_callbacks(rows, cols, 10_000, PtyCallbacks::default()),
            query_parser: vte::Parser::new(),
            sync_starts: 0,
            sync_started_at: None,
        }
    }

    fn answer_query(&mut self, query: TerminalQuery) {
        let reply = {
            let screen = self.inner.screen();
            match query {
                TerminalQuery::DeviceStatus => b"\x1b[0n".to_vec(),
                TerminalQuery::CursorPosition { private } => {
                    let (row, col) = screen.cursor_position();
                    let (rows, cols) = screen.size();
                    let (row, col) = ((row + 1).min(rows), (col + 1).min(cols));
                    if private {
                        format!("\x1b[?{row};{col}R").into_bytes()
                    } else {
                        format!("\x1b[{row};{col}R").into_bytes()
                    }
                }
                TerminalQuery::PrimaryDeviceAttributes => b"\x1b[?1;2c".to_vec(),
                TerminalQuery::SecondaryDeviceAttributes => b"\x1b[>0;0;0c".to_vec(),
                // SSHub does not implement kitty's enhanced keyboard protocol;
                // an explicit flags=0 reply is better than a two-second probe
                // timeout and is the truthful capability answer.
                TerminalQuery::KittyKeyboard => b"\x1b[?0u".to_vec(),
                TerminalQuery::CharacterSize => {
                    let (rows, cols) = screen.size();
                    format!("\x1b[8;{rows};{cols}t").into_bytes()
                }
                TerminalQuery::SynchronizedUpdateMode => {
                    let state = if screen.synchronized_update() { 1 } else { 2 };
                    format!("\x1b[?2026;{state}$y").into_bytes()
                }
            }
        };
        let callbacks = self.inner.callbacks_mut();
        if callbacks.replies.len() + reply.len() <= REPLY_QUEUE_MAX_BYTES {
            callbacks.replies.extend_from_slice(&reply);
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
        let mut observer = QueryObserver::default();
        let mut processed = 0;
        for (index, byte) in bytes.iter().enumerate() {
            self.query_parser
                .advance(&mut observer, std::slice::from_ref(byte));
            if let Some(query) = observer.query.take() {
                // Answer before later writes in this PTY read change the state.
                self.inner.process(&bytes[processed..=index]);
                processed = index + 1;
                self.answer_query(query);
            }
        }
        self.inner.process(&bytes[processed..]);

        let starts = self.inner.screen().synchronized_update_starts();
        if self.inner.screen().synchronized_update() {
            if starts != self.sync_starts {
                self.sync_started_at = Some(Instant::now());
            }
        } else {
            self.sync_started_at = None;
        }
        self.sync_starts = starts;
    }

    /// Whether the visible renderer should keep the previous complete frame.
    /// The parser still consumes all bytes while held; only presentation is
    /// delayed until ESU, matching DEC synchronized-output semantics.
    pub fn should_hold_render(&self) -> bool {
        self.inner.screen().synchronized_update()
            && self
                .sync_started_at
                .is_some_and(|at| at.elapsed() < SYNC_RENDER_HOLD_MAX)
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

    #[test]
    fn xterm_hvp_positions_instead_of_appending() {
        let p = parser_with(12, 80, b"\x1b[2J\x1b[HAAA\x1b[10;20fBBB");
        assert_eq!(p.screen().cell(9, 19).unwrap().contents(), "B");
        assert_eq!(p.screen().cell(9, 20).unwrap().contents(), "B");
        assert_eq!(p.screen().cell(9, 21).unwrap().contents(), "B");
    }

    #[test]
    fn scosc_scorc_restore_cursor_for_tui_overlays() {
        let p = parser_with(8, 40, b"\x1b[2;3H\x1b[s\x1b[7;30H\x1b[uX");
        assert_eq!(p.screen().cell(1, 2).unwrap().contents(), "X");
    }

    #[test]
    fn synchronized_update_holds_until_esu() {
        let mut p = ParserState::new(10, 40);
        p.process(b"\x1b[?2026hhalf a frame");
        assert!(p.should_hold_render());
        p.process(b"\x1b[?2026l");
        assert!(!p.should_hold_render());
    }

    #[test]
    fn split_cursor_query_still_gets_one_reply() {
        let mut p = ParserState::new(10, 40);
        p.process(b"\x1b[4;7H\x1b[");
        assert!(p.take_replies().is_empty());
        p.process(b"6n");
        assert_eq!(p.take_replies(), b"\x1b[4;7R");
        assert!(p.take_replies().is_empty());
    }

    #[test]
    fn decrqm_reports_synchronized_update_state() {
        let mut p = ParserState::new(10, 40);
        p.process(b"\x1b[?2026$p");
        assert_eq!(p.take_replies(), b"\x1b[?2026;2$y");
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
    fn private_queries_are_answered() {
        let mut p = parser_with(10, 80, b"\x1b[?u\x1b[>c\x1b[?6n");
        assert_eq!(p.take_replies(), b"\x1b[?0u\x1b[>0;0;0c\x1b[?1;1R");
    }

    #[test]
    fn queries_answer_state_at_each_sequence_completion() {
        let mut p = parser_with(
            10,
            80,
            b"a\x1b[6nb\x1b[6n\x1b[?2026h\x1b[?2026$p\x1b[?2026l\x1b[?2026$p",
        );
        assert_eq!(
            p.take_replies(),
            b"\x1b[1;2R\x1b[1;3R\x1b[?2026;1$y\x1b[?2026;2$y"
        );
    }

    #[test]
    fn osc_payload_and_canceled_sequences_are_not_queries() {
        let mut p = ParserState::new(10, 80);
        p.process(b"\x1b]0;title[6");
        p.process(b"n\x07\x1b[6\x18n\x1b[6;1n\x1b[6:1n");
        assert!(p.take_replies().is_empty());
        p.process(b"\x1b[5n");
        assert_eq!(p.take_replies(), b"\x1b[0n");
    }

    #[test]
    fn new_synchronized_update_resets_timeout_after_same_chunk_esu() {
        let mut p = parser_with(10, 80, b"\x1b[?2026h");
        p.sync_started_at = Some(Instant::now() - SYNC_RENDER_HOLD_MAX);
        assert!(!p.should_hold_render());
        p.process(b"\x1b[?2026l\x1b[?2026h");
        assert!(p.should_hold_render());
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
