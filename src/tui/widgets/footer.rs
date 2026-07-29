//! Dashboard footer — keybind hint bar.

use ratatui::layout::Rect;
use ratatui::Frame;

use crate::tui::theme;

/// Gap between two hint pairs.
const GAP: u16 = 3;

/// Cells one `(key, label)` pair occupies: key, a space, label.
fn pair_width<K: AsRef<str>, L: AsRef<str>>((key, label): &(K, L)) -> u16 {
    key.as_ref().chars().count() as u16 + 1 + label.as_ref().chars().count() as u16
}

/// Draw one pair and return the column after it.
fn put_pair<K: AsRef<str>, L: AsRef<str>>(
    buf: &mut ratatui::buffer::Buffer,
    x: u16,
    y: u16,
    (key, label): &(K, L),
) -> u16 {
    let key = key.as_ref();
    let label = label.as_ref();
    buf.set_string(x, y, key, theme::footer_key());
    let mut x = x + key.chars().count() as u16;
    buf.set_string(x, y, " ", theme::footer_label());
    x += 1;
    buf.set_string(x, y, label, theme::footer_label());
    x + label.chars().count() as u16
}

/// Render the footer keybind bar.
///
/// `keybinds` is a slice of `(key, label)` pairs, e.g.
/// `&[("↑↓", "select"), ("↵", "connect"), ("/", "search"), …]`.
///
/// Keys are rendered in BRIGHT, labels in MUTE, with 3 spaces between pairs.
///
/// A list that does not fit loses pairs from the *middle*, marked with `…`, not
/// from the end. The SFTP tab needs 220 columns to show its full row, so plain
/// tail truncation dropped `? help` and `q quit` on any normal terminal: the two
/// hints telling you how to get out were the first to go.
///
/// `pinned` is how many trailing pairs must survive that truncation. The caller
/// decides, because which pairs those are depends on what is running: with a
/// session in the background the way back into it is as essential as the way out
/// of the app.
pub fn render_footer<K, L>(frame: &mut Frame, area: Rect, keybinds: &[(K, L)], pinned: usize)
where
    K: AsRef<str>,
    L: AsRef<str>,
{
    if area.height == 0 || area.width == 0 || keybinds.is_empty() {
        return;
    }

    let y = area.y;
    let left = area.x + 1; // 1-char left margin
    let max_x = area.x + area.width;

    let widths: u16 = keybinds.iter().map(pair_width).sum();
    let total = widths + GAP * (keybinds.len() as u16 - 1);

    let buf = frame.buffer_mut();

    // Everything fits: lay it out left to right, nothing to decide.
    if left + total <= max_x {
        let mut x = left;
        for pair in keybinds {
            x = put_pair(buf, x, y, pair) + GAP;
        }
        return;
    }

    let pinned = keybinds.len().min(pinned.max(1));
    let (head, tail) = keybinds.split_at(keybinds.len() - pinned);
    let tail_width: u16 =
        tail.iter().map(pair_width).sum::<u16>() + GAP * (pinned as u16).saturating_sub(1);

    // Room for the pinned block, the ellipsis, and the gap on either side of it.
    let head_limit = max_x.saturating_sub(tail_width + 1 + GAP * 2);
    let mut x = left;
    let mut dropped = false;
    for pair in head {
        if x + pair_width(pair) > head_limit {
            dropped = true;
            break;
        }
        x = put_pair(buf, x, y, pair) + GAP;
    }
    if dropped {
        buf.set_string(x, y, "\u{2026}", theme::footer_label());
        x += 1 + GAP;
    }
    for pair in tail {
        if x + pair_width(pair) > max_x {
            return;
        }
        x = put_pair(buf, x, y, pair) + GAP;
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
