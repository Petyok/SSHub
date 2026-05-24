//! Dashboard footer — keybind hint bar.

use ratatui::layout::Rect;
use ratatui::Frame;

use crate::tui::theme;

/// Render the footer keybind bar.
///
/// `keybinds` is a slice of `(key, label)` pairs, e.g.
/// `&[("↑↓", "select"), ("↵", "connect"), ("/", "search"), …]`.
///
/// Keys are rendered in BRIGHT, labels in MUTE, with 3 spaces between pairs.
pub fn render_footer(frame: &mut Frame, area: Rect, keybinds: &[(&str, &str)]) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let buf = frame.buffer_mut();
    let y = area.y;
    let mut x = area.x + 1; // 1-char left margin
    let max_x = area.x + area.width;

    for (i, (key, label)) in keybinds.iter().enumerate() {
        let key_len = key.chars().count() as u16;
        let label_len = label.chars().count() as u16;
        let pair_len = key_len + 1 + label_len; // key + space + label

        if x + pair_len > max_x {
            break; // don't overflow
        }

        buf.set_string(x, y, key, theme::footer_key());
        x += key_len;
        buf.set_string(x, y, " ", theme::footer_label());
        x += 1;
        buf.set_string(x, y, label, theme::footer_label());
        x += label_len;

        // 3 spaces between pairs (except after last)
        if i + 1 < keybinds.len() {
            x += 3;
        }
    }
}

/// Render a horizontal rule spanning the full width of `area` (1 row).
///
/// Uses `─` (thin) or `━` (bold) in DIM colour.
pub fn render_hrule(frame: &mut Frame, area: Rect, bold: bool) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let ch = if bold { '━' } else { '─' };
    let line: String = std::iter::repeat_n(ch, area.width as usize).collect();
    let buf = frame.buffer_mut();
    buf.set_string(area.x, area.y, &line, theme::dim());
}
