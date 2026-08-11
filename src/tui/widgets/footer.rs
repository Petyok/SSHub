//! Dashboard footer — keybind hint bar.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::Frame;

use crate::theme::catalog::{PaintRole, StyleRole};
use crate::theme::model::ResolvedTheme;

/// Gap between two hint pairs.
const GAP: u16 = 3;

fn pair_width<K: AsRef<str>, L: AsRef<str>>((key, label): &(K, L)) -> u16 {
    key.as_ref().chars().count() as u16 + 1 + label.as_ref().chars().count() as u16
}

fn put_pair<K: AsRef<str>, L: AsRef<str>>(
    buf: &mut ratatui::buffer::Buffer,
    x: u16,
    y: u16,
    (key, label): &(K, L),
    key_style: Style,
    label_style: Style,
) -> u16 {
    let key = key.as_ref();
    let label = label.as_ref();
    buf.set_string(x, y, key, key_style);
    let mut x = x + key.chars().count() as u16;
    buf.set_string(x, y, " ", label_style);
    x += 1;
    buf.set_string(x, y, label, label_style);
    x + label.chars().count() as u16
}

/// Render the footer keybind bar.
///
/// A list that does not fit loses pairs from the *middle*, marked with `…`, not
/// from the end. `pinned` is how many trailing pairs must survive truncation.
pub fn render_footer<K, L>(
    frame: &mut Frame,
    area: Rect,
    keybinds: &[(K, L)],
    pinned: usize,
    theme: &ResolvedTheme,
) where
    K: AsRef<str>,
    L: AsRef<str>,
{
    if area.height == 0 || area.width == 0 || keybinds.is_empty() {
        return;
    }

    let buf = frame.buffer_mut();
    crate::tui::blit::fill_paint(buf, area, theme, PaintRole::FooterBackground);
    let key_style = theme.style(StyleRole::FooterKey);
    let label_style = theme.style(StyleRole::FooterLabel);
    let y = area.y;
    let left = area.x + 1;
    let max_x = area.x + area.width;

    let widths: u16 = keybinds.iter().map(pair_width).sum();
    let total = widths + GAP * (keybinds.len() as u16 - 1);

    if left + total <= max_x {
        let mut x = left;
        for pair in keybinds {
            x = put_pair(buf, x, y, pair, key_style, label_style) + GAP;
        }
        return;
    }

    let pinned = keybinds.len().min(pinned.max(1));
    let (head, tail) = keybinds.split_at(keybinds.len() - pinned);
    let tail_width: u16 =
        tail.iter().map(pair_width).sum::<u16>() + GAP * (pinned as u16).saturating_sub(1);

    let head_limit = max_x.saturating_sub(tail_width + 1 + GAP * 2);
    let mut x = left;
    let mut dropped = false;
    for pair in head {
        if x + pair_width(pair) > head_limit {
            dropped = true;
            break;
        }
        x = put_pair(buf, x, y, pair, key_style, label_style) + GAP;
    }
    if dropped {
        buf.set_string(x, y, "…", label_style);
        x += 1 + GAP;
    }
    for pair in tail {
        if x + pair_width(pair) > max_x {
            return;
        }
        x = put_pair(buf, x, y, pair, key_style, label_style) + GAP;
    }
}

/// Render a horizontal rule spanning the full width of `area` (1 row).
pub fn render_hrule(
    frame: &mut Frame,
    area: Rect,
    bold: bool,
    theme: &ResolvedTheme,
    role: PaintRole,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let ch = if bold { '━' } else { '─' };
    let line: String = std::iter::repeat_n(ch, area.width as usize).collect();
    let buf = frame.buffer_mut();
    let color = crate::tui::blit::line_color(theme, role, area);
    buf.set_string(area.x, area.y, &line, Style::default().fg(color));
    crate::tui::blit::paint_line(buf, area, theme, role);
}
