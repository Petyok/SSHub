//! Dashboard header: 3-line compact wordmark + stats + clock.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::Frame;

use crate::tui::theme;

/// Compact wordmark — 3 rows, split at col 6 into BRIGHT / GREEN.
const WORDMARK: [(&str, &str); 3] = [
    (
        "\u{2584}\u{2596}\u{2584}\u{2596}\u{2596}\u{2596}",
        "\u{2596}\u{2596}\u{258c}",
    ),
    (
        "\u{259a} \u{259a} ",
        "\u{2599}\u{258c}\u{258c}\u{258c}\u{259b}\u{2596}",
    ),
    (
        "\u{2584}\u{258c}\u{2584}\u{258c}\u{258c}\u{258c}",
        "\u{2599}\u{258c}\u{2599}\u{2598}",
    ),
];

/// Render the 3-line header into `area` (expected height == 3).
///
/// * `host_count` — total hosts
/// * `online` / `slow` / `down` — status counts
/// * `clock` — pre-formatted string like `"Tue 10:42:11"`
pub fn render_header(
    frame: &mut Frame,
    area: Rect,
    host_count: usize,
    online: usize,
    slow: usize,
    down: usize,
    clock: &str,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let buf = frame.buffer_mut();

    // ── Wordmark (left side, 3 rows) ──────────────────────
    for (row_idx, (left, right)) in WORDMARK.iter().enumerate() {
        let y = area.y + row_idx as u16;
        if y >= area.y + area.height {
            break;
        }
        buf.set_string(area.x + 1, y, left, Style::default().fg(theme::BRIGHT));
        let right_x = area.x + 1 + unicode_width(left) as u16;
        buf.set_string(right_x, y, right, Style::default().fg(theme::GREEN));
    }

    // ── Stats line (row 1 = middle row, after wordmark) ───
    if area.height >= 2 {
        let stats_y = area.y + 1;
        let stats_x = area.x + 16; // leave space after widest wordmark line

        let mut x = stats_x;

        x = put(buf, x, stats_y, "hosts: ", theme::mute());
        x = put(buf, x, stats_y, &host_count.to_string(), theme::text());
        x = put(buf, x, stats_y, "  \u{00b7}  ", theme::dim());
        x = put(buf, x, stats_y, &online.to_string(), theme::green());
        x = put(buf, x, stats_y, " online", theme::green());
        x = put(buf, x, stats_y, "  \u{00b7}  ", theme::dim());
        x = put(buf, x, stats_y, &slow.to_string(), theme::amber());
        x = put(buf, x, stats_y, " slow", theme::amber());
        x = put(buf, x, stats_y, "  \u{00b7}  ", theme::dim());
        x = put(buf, x, stats_y, &down.to_string(), theme::red());
        let _ = put(buf, x, stats_y, " unreachable", theme::red());

        // Clock — far right of row 1
        let clock_len = clock.len() as u16;
        if area.width > clock_len + 2 {
            let clock_x = area.x + area.width - clock_len - 1;
            buf.set_string(clock_x, stats_y, clock, theme::mute());
        }
    }
}

/// Lifecycle marker for an open session, used to color its status dot.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SessionDot {
    Connecting,
    Running,
    Exited,
}

/// One open embedded session, as rendered in the dashboard session strip.
pub struct SessionChip {
    pub name: String,
    pub dot: SessionDot,
    pub active: bool,
}

/// Render the "open sessions" strip on the top header row, to the right of the
/// wordmark. Makes background SSH sessions visible from the dashboard instead
/// of hiding them behind a footer hint. Renders nothing when `chips` is empty.
///
/// Each chip is `● name`; the dot is colored by lifecycle (green running,
/// amber connecting, red exited) and the active session is reversed. Overflow
/// past the available width collapses into a `+N` counter.
pub fn render_session_strip(frame: &mut Frame, area: Rect, chips: &[SessionChip]) {
    if area.height == 0 || area.width == 0 || chips.is_empty() {
        return;
    }

    let buf = frame.buffer_mut();
    let y = area.y; // top row, alongside the wordmark
    let start_x = area.x + 16; // clear of the widest wordmark line
    let end_x = area.x + area.width; // right edge (exclusive)
    if start_x + 6 >= end_x {
        return; // too narrow to say anything useful
    }

    let mut x = put(buf, start_x, y, "open ", theme::mute());

    let mut rendered = 0usize;
    for (i, chip) in chips.iter().enumerate() {
        let dot_style = match chip.dot {
            SessionDot::Connecting => theme::amber(),
            SessionDot::Running => theme::green(),
            SessionDot::Exited => theme::red(),
        };
        let name_style = if chip.active {
            theme::inv()
        } else {
            theme::text()
        };

        // Width this chip needs: "● " + name + a trailing separator space.
        let chip_w = 2 + unicode_width(&chip.name) + 1;
        let remaining = chips.len() - i;
        // Reserve room for a "+N" overflow marker unless this is the last chip.
        let reserve = if remaining > 1 { 4 } else { 0 };
        if x + chip_w as u16 + reserve > end_x {
            let more = chips.len() - rendered;
            let _ = put(buf, x, y, &format!("+{more}"), theme::mute());
            return;
        }

        x = put(buf, x, y, "\u{25cf} ", dot_style);
        x = put(buf, x, y, &chip.name, name_style);
        x = put(buf, x, y, " ", theme::dim());
        rendered += 1;
    }
}

/// Write `text` at (x, y) and return x + width.
fn put(buf: &mut ratatui::buffer::Buffer, x: u16, y: u16, text: &str, style: Style) -> u16 {
    buf.set_string(x, y, text, style);
    x + unicode_width(text) as u16
}

/// Simple Unicode display-width approximation (ASCII-safe).
fn unicode_width(s: &str) -> usize {
    // We only use ASCII + block-element chars (all single-width).
    s.chars().count()
}
