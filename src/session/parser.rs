//! VT100 parser wrapper. Maintains an in-memory `vt100::Screen` that the
//! renderer draws with [`draw_screen`], relays OSC 52 clipboard writes that
//! applications inside the PTY emit, and answers the terminal status queries
//! they send — `vt100` implements none of the latter, and an application that
//! asks and hears nothing back blocks until its own timeout. In front of the
//! parser, [`XtermCompat`] translates the few xterm sequences `vt100` drops
//! into ones it does understand, draws DEC Special Graphics (line drawing) as
//! the Unicode glyphs, and tracks synchronized output (DEC mode 2026) so the
//! pane can show the last complete frame mid-repaint.

use std::collections::VecDeque;
use std::num::NonZeroU16;
use std::time::{Duration, Instant};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Clear, Widget};

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
    /// Passed through until a final byte in `lo..=0x7e`: an `ESC #`-style
    /// sequence (`0x30`), or a CSI past [`CSI_HOLD_MAX`] (`0x40`).
    Pass(u8),
    /// `ESC (` or `ESC )`, designating G0 or G1 by its final byte; `plain`
    /// until another intermediate makes it a set we don't model.
    Scs { slot: usize, plain: bool },
    /// OSC payload, ended by BEL or `ESC`.
    Osc,
    /// DCS parameters, up to the final byte that starts the payload.
    DcsHead,
    /// DCS payload, ended by `ESC` or a raw C1 ST (`0x9c`).
    Dcs,
    /// SOS/PM/APC payload, ended by `ESC`.
    Str,
}

/// Rewrites the xterm sequences `vt100` 0.18 still ignores into ones it
/// handles (HPR and VPR, SCOSC and SCORC), and caps the REP count it would
/// otherwise loop on: 4 KiB of `X CSI 65535 b` cost it 1.3 s.
///
/// Only CSI sequences and, while DEC Special Graphics is in use, printed
/// ASCII are touched; a CSI sequence only when held whole: one split across
/// PTY reads is kept back and finished on the next read. Bytes inside
/// OSC/DCS/APC strings are never rewritten; when one ends at a bare `ESC`, a
/// CAN goes first so vt100 leaves the string at the same point vte does.
#[derive(Default)]
struct XtermCompat {
    scan: Scan,
    /// The held `ESC` or `ESC [ …` sequence.
    seq: Vec<u8>,
    /// Synchronized output (DEC mode 2026) is set: the app is mid-frame.
    sync: bool,
    /// Index in `marks` of the frame mark that opened the transaction still
    /// open, if it opened in the current batch.
    open_frame: Option<usize>,
    /// Points in the current batch's output where `ParserState` must act.
    marks: Vec<(usize, Mark)>,
    /// Character sets in use, and the copy DECSC saved for DECRC.
    charsets: Charsets,
    saved: Charsets,
}

/// Something `ParserState` must do at a point in the translated stream.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    /// A synchronized-output transaction opens here: the bytes before it are
    /// the last complete frame.
    Frame,
    /// DECRQM for mode 2026, with whether the mode was set at that point.
    ModeQuery(bool),
}

/// G0 and G1 as `ESC (` and `ESC )` designated them, and which one SO/SI put
/// in use. Only DEC Special Graphics (`0`) is modelled, every other set
/// counts as ASCII. vt100 ignores all of it, so ncurses boxes (`ESC ( 0`,
/// then `lqqk`) printed as letters. Issue #147.
#[derive(Default, Clone, Copy)]
struct Charsets {
    /// G0 and G1 hold DEC Special Graphics.
    graphics: [bool; 2],
    /// SO shifted G1 in; SI shifts G0 back.
    shifted: bool,
}

/// What DEC Special Graphics draws for `0x5f..=0x7e`: the glyphs of vte's
/// own charset map (blank, diamond, control pictures, box drawing).
const DEC_GRAPHICS: [char; 32] = [
    ' ', '◆', '▒', '␉', '␌', '␍', '␊', '°', '±', '␤', '␋', '┘', '┐', '┌', '└', '┼', '⎺', '⎻', '─',
    '⎼', '⎽', '├', '┤', '┴', '┬', '│', '≤', '≥', 'π', '≠', '£', '·',
];

impl Charsets {
    /// SO and SI switch sets; any other byte leaves them.
    fn shift(&mut self, b: u8) {
        match b {
            0x0e => self.shifted = true,
            0x0f => self.shifted = false,
            _ => {}
        }
    }

    /// The glyph printing `b` draws, or `None` if it draws itself.
    fn glyph(self, b: u8) -> Option<char> {
        let graphics = self.graphics[usize::from(self.shifted)];
        (graphics && (0x5f..=0x7e).contains(&b)).then(|| DEC_GRAPHICS[usize::from(b - 0x5f)])
    }
}

impl XtermCompat {
    /// Translate `bytes` for a grid `cols` wide, appending the result to `out`
    /// in order.
    fn feed(&mut self, bytes: &[u8], cols: u16, out: &mut Vec<u8>) {
        let mut span = 0;
        for (i, &b) in bytes.iter().enumerate() {
            if !matches!(self.scan, Scan::Esc | Scan::Csi) {
                match (self.scan, b) {
                    (_, ESC) => {
                        out.extend_from_slice(&bytes[span..i]);
                        span = i + 1;
                        // vte ends a string or aborts a sequence on this ESC,
                        // before seeing what follows it. Holding the ESC would
                        // leave vte inside, swallowing C0 controls sent ahead
                        // of the held bytes; CAN ends it right here.
                        if self.scan != Scan::Ground {
                            out.push(0x18);
                        }
                        self.hold_esc();
                    }
                    // vt100 knows no charsets: it gets the glyph itself.
                    (Scan::Ground, _) => match self.charsets.glyph(b) {
                        Some(c) => {
                            out.extend_from_slice(&bytes[span..i]);
                            span = i + 1;
                            out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
                        }
                        None => self.charsets.shift(b),
                    },
                    (_, 0x18 | 0x1a) | (Scan::Osc, 0x07) | (Scan::Dcs, 0x9c) => {
                        self.scan = Scan::Ground;
                    }
                    (Scan::Pass(_) | Scan::Scs { .. }, 0x0e | 0x0f) => self.charsets.shift(b),
                    (Scan::Scs { slot, .. }, 0x20..=0x2f) => {
                        self.scan = Scan::Scs { slot, plain: false };
                    }
                    (Scan::Scs { slot, plain }, 0x30..=0x7e) => {
                        self.charsets.graphics[slot] = plain && b == b'0';
                        self.scan = Scan::Ground;
                    }
                    (Scan::DcsHead, 0x40..=0x7e) => self.scan = Scan::Dcs,
                    (Scan::Pass(lo), _) if (lo..=0x7e).contains(&b) => {
                        self.scan = Scan::Ground;
                        // An overlong REP still carries a count vt100 would
                        // loop on; CAN aborts it before dispatch.
                        if lo == 0x40 && b == b'b' {
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
                0x00..=0x1f => {
                    self.charsets.shift(b);
                    out.push(b);
                }
                // vte ignores DEL and high bytes mid-sequence.
                0x7f..=0xff => {}
                b'[' if self.scan == Scan::Esc => {
                    self.seq.push(b);
                    self.scan = Scan::Csi;
                }
                _ if self.scan == Scan::Esc => {
                    self.seq.push(b);
                    out.extend_from_slice(&self.seq);
                    match b {
                        // RIS resets every mode: synchronized output, charsets.
                        b'c' => {
                            self.sync = false;
                            self.open_frame = None;
                            self.charsets = Charsets::default();
                            self.saved = Charsets::default();
                        }
                        // DECSC and DECRC carry the charsets with the cursor.
                        b'7' => self.saved = self.charsets,
                        b'8' => self.charsets = self.saved,
                        _ => {}
                    }
                    self.scan = match b {
                        b']' => Scan::Osc,
                        b'P' => Scan::DcsHead,
                        b'X' | b'^' | b'_' => Scan::Str,
                        b'(' => Scan::Scs {
                            slot: 0,
                            plain: true,
                        },
                        b')' => Scan::Scs {
                            slot: 1,
                            plain: true,
                        },
                        0x20..=0x2f => Scan::Pass(0x30),
                        _ => Scan::Ground,
                    };
                }
                0x40..=0x7e => {
                    self.seq.push(b);
                    self.dispatch(cols, out);
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
        self.scan = Scan::Esc;
    }

    /// Write the complete CSI sequence in `seq`, rewritten if vt100 lacks it.
    fn dispatch(&mut self, cols: u16, out: &mut Vec<u8>) {
        let end = self.seq.len() - 1;
        let (body, fin) = (&self.seq[2..end], self.seq[end]);
        // Plain parameters only: a private marker or intermediate makes it
        // another sequence (`CSI ? u` is the kitty query, not SCORC).
        let plain = body
            .iter()
            .all(|&b| b.is_ascii_digit() || b == b';' || b == b':');
        let bare = body.is_empty();
        match fin {
            // DECSET/DECRST 2026, alone or combined (`CSI ? 25;2026 h`).
            // vt100 still gets the bytes; it ignores the mode itself.
            b'h' | b'l'
                if body.first() == Some(&b'?')
                    && body[1..].split(|&b| b == b';').any(|p| param(p) == 2026) =>
            {
                if fin == b'h' && !self.sync {
                    self.sync = true;
                    self.open_frame = Some(self.marks.len());
                    self.marks.push((out.len(), Mark::Frame));
                }
                if fin == b'l' {
                    self.sync = false;
                    self.open_frame = None;
                }
                out.extend_from_slice(&self.seq);
            }
            // DECRQM 2026: answered in stream order by `ParserState`.
            b'p' if body == b"?2026$" => {
                self.marks.push((out.len(), Mark::ModeQuery(self.sync)));
                out.extend_from_slice(&self.seq);
            }
            b's' if bare => {
                self.saved = self.charsets;
                out.extend_from_slice(b"\x1b7");
            }
            b'u' if bare => {
                self.charsets = self.saved;
                out.extend_from_slice(b"\x1b8");
            }
            // HPR, VPR: same parameters as CUF, CUD.
            b'a' | b'e' if plain => {
                self.seq[end] = if fin == b'a' { b'C' } else { b'B' };
                out.extend_from_slice(&self.seq);
            }
            // REP, bounded to `cols` characters (two rows of wide ones).
            b'b' if plain && param(body) > cols => {
                out.extend_from_slice(format!("\x1b[{cols}b").as_bytes());
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
                let (row, col) = ((row + 1).min(rows.get()), (col + 1).min(cols.get()));
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
        self.queue_reply(&reply);
    }
}

impl PtyCallbacks {
    fn queue_reply(&mut self, reply: &[u8]) {
        if self.replies.len() + reply.len() <= REPLY_QUEUE_MAX_BYTES {
            self.replies.extend_from_slice(reply);
        }
    }
}

/// Longest a synchronized update may freeze the pane (1 s), counted from when
/// the held frame was taken. An app that opens a transaction and never closes
/// it must not freeze the session; a repeated BSU takes no new frame, so it
/// cannot extend the hold either.
const SYNC_HOLD_MAX: Duration = Duration::from_secs(1);

pub struct ParserState {
    inner: vt100::Parser<PtyCallbacks>,
    compat: XtermCompat,
    /// Translated reads queued by [`Self::queue`], up to `done` already fed
    /// to vt100; reused.
    out: Vec<u8>,
    /// End offset in `out` of each queued read not yet fed.
    ends: VecDeque<usize>,
    done: usize,
    /// Next entry of `compat.marks` to act on.
    next_mark: usize,
    /// The pane as of the last complete frame, drawn instead of the live grid
    /// while a synchronized update (DEC mode 2026) is open. Rasterized at most
    /// once per batch; a clone of the screen would drag 10k scrollback rows.
    frame: Buffer,
    /// `frame` matches the current grid size and the live (not scrolled) view.
    frame_ok: bool,
    /// When `frame` was rasterized.
    taken: Instant,
    #[cfg(test)]
    pub(super) snapshots: usize,
}

impl ParserState {
    pub fn new(rows: u16, cols: u16) -> Self {
        Self {
            inner: vt100::Parser::new_with_callbacks(
                nonzero(rows),
                nonzero(cols),
                10_000,
                PtyCallbacks::default(),
            ),
            compat: XtermCompat::default(),
            out: Vec::new(),
            ends: VecDeque::new(),
            done: 0,
            next_mark: 0,
            frame: Buffer::empty(Rect::default()),
            frame_ok: false,
            taken: Instant::now(),
            #[cfg(test)]
            snapshots: 0,
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
        self.process_at(bytes, Instant::now());
    }

    fn process_at(&mut self, bytes: &[u8], now: Instant) {
        self.queue(bytes);
        self.apply_next(now);
    }

    /// Translate one PTY read and queue it for [`Self::apply_next`]. Queue all
    /// reads of a batch before applying any: the held frame is then rasterized
    /// once, at the batch's last frame boundary, instead of once per read.
    pub(crate) fn queue(&mut self, bytes: &[u8]) {
        let (_, cols) = self.size();
        self.compat.feed(bytes, cols, &mut self.out);
        self.ends.push_back(self.out.len());
    }

    /// Feed the oldest queued read to vt100, as of `now`.
    pub(crate) fn apply_next(&mut self, now: Instant) {
        let Some(end) = self.ends.pop_front() else {
            return;
        };
        // Only the transaction still open at the end of the batch matters:
        // its opening BSU is where the last complete frame ends.
        while let Some(&(at, mark)) = self.compat.marks.get(self.next_mark) {
            if at >= end {
                break;
            }
            self.next_mark += 1;
            if mark == Mark::Frame && self.compat.open_frame != Some(self.next_mark - 1) {
                continue;
            }
            self.inner.process(&self.out[self.done..at]);
            self.done = at;
            match mark {
                Mark::Frame => self.snapshot(now),
                // DECRPM: 1 = set, 2 = reset. Queued with vt100's own answers,
                // so replies keep stream order.
                Mark::ModeQuery(set) => self.inner.callbacks_mut().queue_reply(if set {
                    b"\x1b[?2026;1$y"
                } else {
                    b"\x1b[?2026;2$y"
                }),
            }
        }
        self.inner.process(&self.out[self.done..end]);
        self.done = end;
        if self.ends.is_empty() {
            self.out.clear();
            self.done = 0;
            self.compat.marks.clear();
            self.compat.open_frame = None;
            self.next_mark = 0;
        }
    }

    /// Rasterize the grid as the held frame, as [`draw_screen`] draws it live.
    fn snapshot(&mut self, now: Instant) {
        // A scrolled-back view is not the frame the app drew.
        self.frame_ok = self.scrollback() == 0;
        if self.frame_ok {
            let (rows, cols) = self.size();
            let area = Rect::new(0, 0, cols, rows);
            self.frame.resize(area);
            draw_screen(self.inner.screen(), area, &mut self.frame, true);
            self.taken = now;
            #[cfg(test)]
            {
                self.snapshots += 1;
            }
        }
    }

    /// The pane to draw instead of the live grid while the app is mid-frame,
    /// or `None` to draw live. `live_only` is for overlays that read the live
    /// grid (selection, ghost text). Never held while scrolled back, after a
    /// resize until the next frame, or once the frame is [`SYNC_HOLD_MAX`] old.
    pub(crate) fn held_frame(&self, now: Instant, live_only: bool) -> Option<&Buffer> {
        let hold = self.compat.sync
            && self.frame_ok
            && !live_only
            && self.scrollback() == 0
            && now.saturating_duration_since(self.taken) < SYNC_HOLD_MAX;
        hold.then_some(&self.frame)
    }

    /// Resize the grid to `rows` x `cols`. vt100 keeps the cursor row: a
    /// shrink cuts blank rows below the cursor, then scrolls the top into
    /// history; a grow pulls history back down. It also resets the scroll
    /// region and the scrollback offset, as xterm does. Columns are cut, not
    /// reflowed.
    pub fn set_size(&mut self, rows: u16, cols: u16) {
        self.inner.set_size(nonzero(rows), nonzero(cols));
        // The held frame has the old size: draw live until the next one.
        self.frame_ok = false;
    }

    pub fn screen(&self) -> &vt100::Screen {
        self.inner.screen()
    }

    /// The grid size as (rows, cols).
    pub fn size(&self) -> (u16, u16) {
        let (rows, cols) = self.inner.screen().size();
        (rows.get(), cols.get())
    }

    /// Current scrollback offset (0 = pinned to bottom).
    pub fn scrollback(&self) -> usize {
        self.inner.screen().scrollback()
    }

    pub fn set_scrollback(&mut self, rows: usize) {
        // vt100 caps the value at `scrollback.len()` internally, so any value
        // up to the full buffer is safe.
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

/// vt100 takes no empty grid; the PTY never has one (`floor_grid`).
fn nonzero(n: u16) -> NonZeroU16 {
    NonZeroU16::new(n).unwrap_or(NonZeroU16::MIN)
}

/// Draw `screen` into `area` of `buf`: each cell's text, colours and
/// attributes, then the cursor when `cursor` is set and the app has not hidden
/// it, as a gray block on a blank cell and reversed over text. Cells past the
/// grid are cleared.
pub(crate) fn draw_screen(screen: &vt100::Screen, area: Rect, buf: &mut Buffer, cursor: bool) {
    Clear.render(area, buf);
    for row in 0..area.height {
        for col in 0..area.width {
            let Some(cell) = screen.cell(row, col) else {
                continue;
            };
            let out = &mut buf[(area.x + col, area.y + row)];
            if cell.has_contents() {
                out.set_symbol(cell.contents());
            }
            let mut modifier = Modifier::empty();
            for (on, m) in [
                (cell.bold(), Modifier::BOLD),
                (cell.dim(), Modifier::DIM),
                (cell.italic(), Modifier::ITALIC),
                (cell.underline(), Modifier::UNDERLINED),
                (cell.inverse(), Modifier::REVERSED),
            ] {
                if on {
                    modifier |= m;
                }
            }
            out.set_style(
                Style::reset()
                    .fg(color(cell.fgcolor()))
                    .bg(color(cell.bgcolor()))
                    .add_modifier(modifier),
            );
        }
    }
    if !cursor || screen.hide_cursor() {
        return;
    }
    // Scrolled back, the cursor's row moves down with the live screen.
    let (row, col) = screen.cursor_position();
    let row = row.saturating_add(u16::try_from(screen.scrollback()).unwrap_or(u16::MAX));
    if row >= area.height || col >= area.width {
        return;
    }
    let Some(cell) = screen.cell(row, col) else {
        return;
    };
    let out = &mut buf[(area.x + col, area.y + row)];
    if cell.has_contents() {
        out.set_style(Style::default().add_modifier(Modifier::REVERSED));
    } else {
        out.set_symbol("\u{2588}");
        out.set_style(Style::default().fg(Color::Gray));
    }
}

fn color(c: vt100::Color) -> Color {
    match c {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
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
    // vt100 0.18 drops these xterm sequences; `XtermCompat` rewrites them
    // into ones it handles before the bytes reach the parser.

    /// Run `chunks` through the translator alone, as successive PTY reads.
    fn translate(cols: u16, chunks: &[&[u8]]) -> Vec<u8> {
        let mut compat = XtermCompat::default();
        let mut out = Vec::new();
        for chunk in chunks {
            compat.feed(chunk, cols, &mut out);
        }
        out
    }

    fn row(p: &ParserState, r: usize) -> String {
        let (_, cols) = p.size();
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
                translate(80, &[&stream[..k], &stream[k..]]),
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
        let mut raw = vt100::Parser::new(nonzero(5), nonzero(20), 0);
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

    // ncurses draws boxes by designating DEC Special Graphics (`ESC ( 0`) and
    // printing ASCII letters; vt100 ignores the designation, so nvtop's graph
    // borders came out as `lqqk` (#147).

    #[test]
    fn line_drawing_draws_boxes_until_ascii_comes_back() {
        // smacs and rmacs as TERM=xterm-256color has them.
        let p = parser_with(5, 20, b"\x1b(0lqqk\r\nx\x1b(B hi \x1b(0x\r\nmqqj\x1b(B lqk");
        assert_eq!(row(&p, 0), "┌──┐");
        assert_eq!(row(&p, 1), "│ hi │");
        assert_eq!(row(&p, 2), "└──┘ lqk");
        // Only `_` through `~` are remapped.
        assert_eq!(row(&parser_with(5, 20, b"\x1b(0AZ09^_`a~"), 0), "AZ09^ ◆▒·");
    }

    #[test]
    fn so_and_si_switch_between_g0_and_g1() {
        assert_eq!(row(&parser_with(5, 20, b"\x1b)0q\x0eq\x0fq"), 0), "q─q");
        // With ASCII in G1, SO hides line drawing designated into G0.
        assert_eq!(row(&parser_with(5, 20, b"\x1b(0\x0eq\x0fq"), 0), "q─");
    }

    #[test]
    fn only_esc_paren_0_designates_line_drawing() {
        // `ESC ( A` and `ESC ( % 5` name sets we draw as ASCII; an aborted
        // `ESC (` changes nothing, so its `0` prints.
        let p = parser_with(5, 20, b"\x1b(0q\x1b(Aq\x1b(0\x1b(%5q\x1b(0\x1b(\x180q");
        assert_eq!(row(&p, 0), "─qq0─");
    }

    #[test]
    fn rep_repeats_the_line_glyph() {
        assert_eq!(
            row(&parser_with(5, 20, b"\x1b(0q\x1b[3b\x1b(Bq"), 0),
            "────q"
        );
    }

    #[test]
    fn cursor_save_and_restore_carry_the_charsets() {
        let p = parser_with(5, 20, b"\x1b7\x1b(0q\x1b8\x1b[Bq");
        assert_eq!((row(&p, 0), row(&p, 1)), ("─".into(), "q".into()));
        let p = parser_with(5, 20, b"\x1b(0\x1b[s\x1b(Bq\x1b[u\x1b[Bq");
        assert_eq!((row(&p, 0), row(&p, 1)), ("q".into(), "─".into()));
        // RIS resets the charsets in use and the saved copy.
        assert_eq!(row(&parser_with(5, 20, b"\x1b)0\x0e\x1bcq"), 0), "q");
        assert_eq!(row(&parser_with(5, 20, b"\x1b(0\x1b7\x1bc\x1b8q"), 0), "q");
    }

    #[test]
    fn line_drawing_survives_every_split_between_reads() {
        let stream: &[u8] = b"\x1b(0lq\x1b[2bk\x1b(Bq\x1b)0\x0ex\x0fx\x1b7\x1b(0\x1b8q";
        let p = parser_with(5, 20, stream);
        assert_eq!(row(&p, 0), "┌───┐q│xq");
        let whole = p.screen().contents_formatted();
        for k in 0..=stream.len() {
            let mut p = ParserState::new(5, 20);
            p.process(&stream[..k]);
            p.process(&stream[k..]);
            assert_eq!(p.screen().contents_formatted(), whole, "split at {k}");
        }
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
            b"\x0e",
            b"\x0f",
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
                let mut raw = vt100::Parser::new_with_callbacks(
                    nonzero(6),
                    nonzero(20),
                    10_000,
                    PtyCallbacks::default(),
                );
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
    fn overlong_rep_is_aborted() {
        // Zero padding pushes it past the hold limit, out of reach of the
        // count cap; it must not reach vt100 with its 65535 count intact.
        let mut rep = b"\x1b[".to_vec();
        rep.extend(std::iter::repeat_n(b'0', 300));
        rep.extend_from_slice(b"65535b");
        let mut aborted = rep.clone();
        *aborted.last_mut().unwrap() = 0x18;
        assert_eq!(translate(80, &[&rep]), aborted);

        let mut stream = b"a".to_vec();
        stream.extend_from_slice(&rep);
        assert_eq!(row(&parser_with(24, 80, &stream), 0), "a");
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
        let (rows, cols) = crate::session::floor_grid((0, 0));
        let mut p = ParserState::new(rows, cols);
        p.process("abcdefghijklmnop中文中文\r\nx中".as_bytes());
    }

    // ── Synchronized output (DEC mode 2026) ───────────────────────
    //
    // Inside `CSI ? 2026 h` … `CSI ? 2026 l` the pane shows the last complete
    // frame, so a read that ends mid-repaint is never drawn half-way.

    const BSU: &str = "\x1b[?2026h";
    const ESU: &str = "\x1b[?2026l";

    /// The held pane as text, or `None` when the pane draws live.
    fn held(p: &ParserState, now: Instant) -> Option<String> {
        p.held_frame(now, false).map(|b| {
            let a = b.area;
            (0..a.height)
                .map(|y| (0..a.width).map(|x| b[(x, y)].symbol()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n")
        })
    }

    fn ms(t: Instant, n: u64) -> Instant {
        t + Duration::from_millis(n)
    }

    #[test]
    fn sync_holds_the_last_complete_frame_until_esu() {
        let t = Instant::now();
        let mut p = ParserState::new(5, 20);
        p.process(format!("BEFORE{BSU}PARTIAL").as_bytes());
        let shown = held(&p, t).expect("held mid-frame");
        assert!(
            shown.contains("BEFORE") && !shown.contains("PARTIAL"),
            "{shown}"
        );
        assert!(
            p.screen().contents().contains("BEFOREPARTIAL"),
            "vt100 still gets it all"
        );
        p.process(format!(" DONE{ESU}").as_bytes());
        assert_eq!(held(&p, ms(t, 100)), None);
    }

    #[test]
    fn sync_hold_is_capped_from_the_snapshot() {
        let t = Instant::now();
        let mut p = ParserState::new(5, 20);
        p.process_at(format!("A{BSU}B").as_bytes(), t);
        assert!(held(&p, ms(t, 500)).is_some());
        assert!(held(&p, ms(t, 999)).is_some());
        assert_eq!(held(&p, ms(t, 1000)), None, "cap fired");
        p.process_at(b"C", ms(t, 1100));
        assert_eq!(
            held(&p, ms(t, 1100)),
            None,
            "live until the transaction ends"
        );
        // The next transaction takes a fresh frame and may hold again.
        p.process_at(format!("{ESU}D{BSU}E").as_bytes(), ms(t, 1700));
        assert!(held(&p, ms(t, 1700)).is_some_and(|s| s.contains("ABCD") && !s.contains('E')));
    }

    #[test]
    fn repeated_bsu_cannot_extend_the_hold() {
        let t = Instant::now();
        let mut p = ParserState::new(5, 20);
        p.process_at(format!("A{BSU}B").as_bytes(), t);
        p.process_at(format!("{BSU}C").as_bytes(), ms(t, 600));
        assert!(held(&p, ms(t, 600)).is_some());
        assert_eq!(held(&p, ms(t, 1000)), None);
    }

    #[test]
    fn a_stale_frame_never_comes_back() {
        // A selection drawn live for 5 s, then cleared: the 5 s old frame must
        // not be shown for another second.
        let t = Instant::now();
        let mut p = ParserState::new(5, 20);
        p.process_at(format!("A{BSU}B").as_bytes(), t);
        assert!(p.held_frame(ms(t, 100), true).is_none());
        assert!(p.held_frame(ms(t, 5000), true).is_none());
        assert_eq!(held(&p, ms(t, 5050)), None, "selection cleared");
        // A background tab's frame taken 10 s ago, first drawn when switched to.
        let mut bg = ParserState::new(5, 20);
        bg.process_at(format!("A{BSU}B").as_bytes(), t);
        assert_eq!(held(&bg, ms(t, 10_000)), None);
    }

    #[test]
    fn esu_bsu_loop_keeps_advancing_frames() {
        let t = Instant::now();
        let mut p = ParserState::new(5, 20);
        for i in 1..=5u64 {
            let at = ms(t, i * 1500);
            p.process_at(format!("{ESU}\x1b[2J\x1b[HFRAME {i}{BSU}").as_bytes(), at);
            // Frames further apart than the cap: each new one is held anyway.
            let shown = held(&p, at).expect("each new frame is held");
            assert!(shown.contains(&format!("FRAME {i}")), "{shown}");
        }
        // Several transactions in one read: the frame before the last BSU,
        // rasterized once.
        let before = p.snapshots;
        p.process_at(
            format!("{ESU}X{BSU}x{ESU}Y{BSU}y{ESU}Z{BSU}z").as_bytes(),
            ms(t, 9000),
        );
        assert_eq!(p.snapshots, before + 1);
        let shown = held(&p, ms(t, 9000)).unwrap();
        assert!(shown.contains("XxYyZ") && !shown.contains('z'), "{shown}");
    }

    #[test]
    fn a_batch_of_reads_takes_one_snapshot() {
        // Many small reads queued together (one `Session::drain`) cost one
        // rasterization, of the frame at the batch's last BSU.
        let t = Instant::now();
        let mut p = ParserState::new(5, 20);
        let reads: Vec<String> = (0..8).map(|i| format!("{ESU}{i}{BSU}")).collect();
        for r in &reads {
            p.queue(r.as_bytes());
        }
        for _ in &reads {
            p.apply_next(t);
        }
        assert_eq!(p.snapshots, 1);
        assert!(held(&p, t).is_some_and(|s| s.starts_with("01234567")));
        // A transaction still open from an earlier batch keeps its frame.
        p.queue(b"more");
        p.queue(b" still");
        p.apply_next(t);
        p.apply_next(t);
        assert_eq!(p.snapshots, 1);
        assert!(held(&p, t).is_some_and(|s| !s.contains("more")));
    }

    #[test]
    fn sync_hold_is_bypassed_when_scrolled_resized_or_overlaid() {
        let t = Instant::now();
        let mut p = ParserState::new(5, 20);
        for i in 0..20 {
            p.process(format!("line {i}\r\n").as_bytes());
        }
        p.process(format!("{BSU}partial").as_bytes());
        assert!(held(&p, t).is_some());
        assert!(
            p.held_frame(t, true).is_none(),
            "selection / ghost read the live grid"
        );
        p.set_scrollback(3);
        assert_eq!(held(&p, t), None, "scrolled back");
        p.set_scrollback(0);
        assert!(held(&p, t).is_some());
        p.set_size(6, 30);
        assert_eq!(held(&p, t), None, "first frame after a resize");
        p.process(format!("{ESU}{BSU}").as_bytes());
        assert_eq!(p.held_frame(t, false).map(|b| b.area.width), Some(30));
    }

    #[test]
    fn decrqm_2026_is_answered_in_stream_order() {
        let mut p = ParserState::new(5, 20);
        p.process(format!("\x1b[?2026$p\x1b[5n{BSU}\x1b[?2026$p\x1b[6n").as_bytes());
        assert_eq!(
            p.take_replies(),
            b"\x1b[?2026;2$y\x1b[0n\x1b[?2026;1$y\x1b[1;1R"
        );
        p.process(b"\x1b[?20");
        p.process(format!("26$p{ESU}\x1b[?2026$p\x1b[?2027$p\x1b[?u\x1b[>c").as_bytes());
        assert_eq!(p.take_replies(), b"\x1b[?2026;1$y\x1b[?2026;2$y");
    }

    #[test]
    fn combined_decset_params_toggle_sync() {
        let t = Instant::now();
        let mut p = ParserState::new(5, 20);
        p.process(b"A\x1b[?25;2026hB");
        assert!(held(&p, t).is_some_and(|s| s.contains('A') && !s.contains('B')));
        p.process(b"\x1b[?1049;2026l");
        assert_eq!(held(&p, t), None);
    }

    #[test]
    fn ris_ends_the_hold() {
        let t = Instant::now();
        let mut p = ParserState::new(5, 20);
        p.process(format!("A{BSU}B").as_bytes());
        assert!(held(&p, t).is_some());
        p.process(b"\x1bc");
        assert_eq!(held(&p, t), None);
    }

    // ── Row shrink keeps the cursor row (#73) ───────────────────────
    //
    // Shrinking a window with the prompt on the last row scrolls the top into
    // history; the prompt and the output just above it stay on screen.

    /// Every row the screen holds, oldest scrollback first, then the screen.
    fn history(p: &ParserState) -> Vec<String> {
        let mut screen = p.screen().clone();
        let (rows, cols) = p.size();
        screen.set_scrollback(usize::MAX);
        let mut out: Vec<String> = Vec::new();
        // Walk up from the oldest window one screen at a time; each window
        // starts `offset` rows above the live top.
        let mut offset = screen.scrollback();
        while offset > 0 {
            screen.set_scrollback(offset);
            let take = offset.min(usize::from(rows));
            out.extend(screen.rows(0, cols).take(take));
            offset -= take;
        }
        screen.set_scrollback(0);
        out.extend(screen.rows(0, cols));
        out
    }

    /// Lines `1..=n`, then a prompt on what is then the last row.
    fn shell_output(rows: u16, n: usize) -> ParserState {
        let mut p = ParserState::new(rows, 40);
        let mut stream: String = (1..=n).map(|i| format!("{i}\r\n")).collect();
        stream.push_str("PROMPT$ ");
        p.process(stream.as_bytes());
        assert_eq!(p.screen().cursor_position(), (rows - 1, 8));
        p
    }

    #[test]
    fn shrinking_rows_scrolls_the_top_away_and_keeps_the_prompt() {
        let mut p = shell_output(46, 100);
        p.set_size(23, 40);
        // The prompt is still where the shell left its cursor, on the new last
        // row, with the last output right above it.
        assert_eq!(p.screen().cursor_position(), (22, 8));
        let rows: Vec<String> = p.screen().rows(0, 40).collect();
        assert_eq!(rows[22].trim_end(), "PROMPT$");
        assert_eq!(rows[21].trim_end(), "100");
        // Nothing was lost: every line is in history, in order, once.
        let lines: Vec<String> = history(&p)
            .into_iter()
            .map(|l| l.trim_end().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        let mut want: Vec<String> = (1..=100).map(|i| i.to_string()).collect();
        want.push("PROMPT$".into());
        assert_eq!(lines, want);
        // Growing back pulls history down: the screen is what it was.
        p.set_size(46, 40);
        assert_eq!(p.screen().cursor_position(), (45, 8));
        let rows: Vec<String> = p.screen().rows(0, 40).collect();
        assert_eq!(rows[45].trim_end(), "PROMPT$");
        assert_eq!(rows[0].trim_end(), "56");
        assert_eq!(
            history(&p).iter().filter(|l| !l.trim().is_empty()).count(),
            101
        );
    }
}
