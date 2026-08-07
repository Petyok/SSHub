//! Dashboard header: 3-line compact wordmark + stats + clock.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::Frame;

use crate::theme::catalog::{ColorRole, PaintRole, StyleRole};
use crate::theme::model::ResolvedTheme;

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

/// The figures the header's stats line reports, plus its clock.
///
/// Bundled rather than passed as five positional arguments: the renderer now
/// also takes the theme, and `(usize, usize, usize, usize, &str)` at a call
/// site says nothing about which count is which.
pub struct HeaderStats<'a> {
    pub host_count: usize,
    pub online: usize,
    pub slow: usize,
    pub down: usize,
    /// Pre-formatted, e.g. `"Tue 10:42:11"`.
    pub clock: &'a str,
}

/// Render the 3-line header into `area` (expected height == 3).
pub fn render_header(frame: &mut Frame, area: Rect, stats: HeaderStats<'_>, theme: &ResolvedTheme) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let buf = frame.buffer_mut();
    // The chrome band behind the wordmark. `default` leaves it at "terminal",
    // so this is a no-op there and the frozen golden is unaffected.
    crate::tui::blit::fill_paint(buf, area, theme, PaintRole::HeaderBackground);

    let muted = theme.style(StyleRole::HeaderStatsLabel);
    let value = theme.style(StyleRole::HeaderStatsValue);
    let dim = theme.style(StyleRole::TextDim);
    let color = |role| Style::default().fg(theme.color(role));

    // ── Wordmark (left side, 3 rows) ──────────────────────
    //
    // The two halves take the bright text role and the "online" green, which
    // is what the frozen `default` renders. `header.brand` is the *inverse*
    // chip style and belongs to the session view's title, not to the wordmark.
    let wordmark_left = theme.style(StyleRole::TextBright);
    let wordmark_right = color(ColorRole::HeaderSessionSuccess);
    for (row_idx, (left, right)) in WORDMARK.iter().enumerate() {
        let y = area.y + row_idx as u16;
        if y >= area.y + area.height {
            break;
        }
        buf.set_string(area.x + 1, y, left, wordmark_left);
        let right_x = area.x + 1 + unicode_width(left) as u16;
        buf.set_string(right_x, y, right, wordmark_right);
    }

    // ── Stats line (row 1 = middle row, after wordmark) ───
    if area.height >= 2 {
        let stats_y = area.y + 1;
        let stats_x = area.x + 16; // leave space after widest wordmark line

        let mut x = stats_x;
        let online_style = color(ColorRole::HeaderSessionSuccess);
        let slow_style = color(ColorRole::HeaderSessionWarning);
        let down_style = color(ColorRole::HeaderSessionError);

        x = put(buf, x, stats_y, "hosts: ", muted);
        x = put(buf, x, stats_y, &stats.host_count.to_string(), value);
        x = put(buf, x, stats_y, "  \u{00b7}  ", dim);
        x = put(buf, x, stats_y, &stats.online.to_string(), online_style);
        x = put(buf, x, stats_y, " online", online_style);
        x = put(buf, x, stats_y, "  \u{00b7}  ", dim);
        x = put(buf, x, stats_y, &stats.slow.to_string(), slow_style);
        x = put(buf, x, stats_y, " slow", slow_style);
        x = put(buf, x, stats_y, "  \u{00b7}  ", dim);
        x = put(buf, x, stats_y, &stats.down.to_string(), down_style);
        let _ = put(buf, x, stats_y, " unreachable", down_style);

        // Clock — far right of row 1
        let clock_len = stats.clock.len() as u16;
        if area.width > clock_len + 2 {
            let clock_x = area.x + area.width - clock_len - 1;
            buf.set_string(clock_x, stats_y, stats.clock, muted);
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
pub fn render_session_strip(
    frame: &mut Frame,
    area: Rect,
    chips: &[SessionChip],
    travel: Option<StripTravel>,
    theme: &ResolvedTheme,
) {
    if area.height == 0 || area.width == 0 || chips.is_empty() {
        return;
    }

    // Columns each chip's name occupies, so a travelling highlight knows where
    // to start and where to land. Only names are highlighted; the lifecycle dot
    // keeps its own colour.
    let mut name_spans: Vec<(u16, u16)> = Vec::with_capacity(chips.len());
    let buf = frame.buffer_mut();
    let y = area.y; // top row, alongside the wordmark
    let start_x = area.x + 16; // clear of the widest wordmark line
    let end_x = area.x + area.width; // right edge (exclusive)
    if start_x + 6 >= end_x {
        return; // too narrow to say anything useful
    }

    let more_style = theme.style(StyleRole::HeaderSessionMore);
    let mut x = put(buf, start_x, y, "open ", more_style);

    for (i, chip) in chips.iter().enumerate() {
        let dot_style = Style::default().fg(theme.color(match chip.dot {
            SessionDot::Connecting => ColorRole::HeaderSessionWarning,
            SessionDot::Running => ColorRole::HeaderSessionSuccess,
            SessionDot::Exited => ColorRole::HeaderSessionError,
        }));
        // While the highlight is travelling, no chip paints itself highlighted:
        // the moving bar below is the only thing wearing the active style.
        let name_style = if chip.active && travel.is_none() {
            theme.style(StyleRole::HeaderSessionActive)
        } else {
            theme.style(StyleRole::HeaderSessionInactive)
        };

        // Width this chip needs: "● " + name + a trailing separator space.
        let chip_w = 2 + unicode_width(&chip.name) + 1;
        let remaining = chips.len() - i;
        // Reserve room for a "+N" overflow marker unless this is the last chip.
        let reserve = if remaining > 1 { 4 } else { 0 };
        if x + chip_w as u16 + reserve > end_x {
            let more = chips.len() - i;
            let _ = put(buf, x, y, &format!("+{more}"), more_style);
            return;
        }

        x = put(buf, x, y, "\u{25cf} ", dot_style);
        name_spans.push((x, unicode_width(&chip.name) as u16));
        x = put(buf, x, y, &chip.name, name_style);
        x = put(buf, x, y, " ", theme.style(StyleRole::TextDim));
    }

    // Carry the highlight from the chip being left to the new one, so switching
    // tabs from the dashboard moves instead of teleporting (#35). A chip that
    // collapsed into the `+N` marker has no span, and then there is nothing to
    // travel between.
    if let Some(t) = travel {
        let (Some(&(fx, fw)), Some(&(tx, tw))) = (name_spans.get(t.from), name_spans.get(t.to))
        else {
            return;
        };
        let e = crate::tui::tween::ease_in_out(t.p);
        let bar_x = crate::session::render::lerp_u16(fx, tx, e);
        let bar_w = crate::session::render::lerp_u16(fw, tw, e);
        for cx in bar_x..bar_x.saturating_add(bar_w).min(end_x) {
            if let Some(cell) = buf.cell_mut((cx, y)) {
                cell.set_style(theme.style(StyleRole::HeaderSessionActive));
            }
        }
    }
}

/// An in-flight highlight travel across the dashboard session strip (#35):
/// `from` and `to` are chip indices, `p` is raw progress in `0..1` (the easing
/// is applied here, so callers pass what [`crate::tui::tween::progress`] gave
/// them).
#[derive(Debug, Clone, Copy)]
pub struct StripTravel {
    pub from: usize,
    pub to: usize,
    pub p: f32,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::resolved_source;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;
    use ratatui::Terminal;

    /// An inactive session chip must take `header.session_inactive`, not the
    /// generic primary-text role. Under `default` the two are different
    /// recipes already, but a marker colour proves the binding rather than a
    /// coincidence of palettes.
    #[test]
    fn an_inactive_session_chip_takes_the_session_inactive_role() {
        let theme = resolved_source(
            "markers",
            "schema_version = 1\nname = \"Markers\"\nextends = \"default\"\n\n\
             [components.header]\nsession_inactive = { foreground = \"#ff00ff\" }\n\n\
             [components.text]\nprimary = { foreground = \"#00ff00\" }\n",
        );
        let area = Rect::new(0, 0, 60, 2);
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal
            .draw(|frame| {
                render_session_strip(
                    frame,
                    area,
                    &[SessionChip {
                        name: "alpha".into(),
                        dot: SessionDot::Running,
                        active: false,
                    }],
                    None,
                    &theme,
                );
            })
            .unwrap();

        // "open " starts at x + 16, then "\u{25cf} " (2 cells), then the name.
        let name_x = 16 + 5 + 2;
        let buf = terminal.backend().buffer();
        assert_eq!(buf.cell((name_x, 0)).unwrap().symbol(), "a");
        assert_eq!(
            buf.cell((name_x, 0)).unwrap().fg,
            Color::Rgb(0xff, 0x00, 0xff),
            "the inactive chip name takes `header.session_inactive`"
        );
    }
}
