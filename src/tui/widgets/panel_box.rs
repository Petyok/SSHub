//! Reusable bordered panel box for dashboard columns.
//!
//! Draws box-drawing borders using theme::border() style, with an optional
//! title (in BRIGHT) and count badge (in DIM) embedded in the top border.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::tui::text::ellipsize;
use crate::tui::theme;

/// Write `s` at (`x`,`y`), truncated with `…` so it never exceeds `max_w`
/// display columns — keeps dashboard text inside its panel border even when
/// the column is narrow (e.g. after a zoom). Returns the columns written.
pub fn put_clamped(buf: &mut Buffer, x: u16, y: u16, s: &str, style: Style, max_w: usize) -> u16 {
    if max_w == 0 {
        return 0;
    }
    let text = ellipsize(s, max_w);
    buf.set_string(x, y, &text, style);
    text.chars().count() as u16
}

/// Draw a bordered panel box into `buf`.
///
/// Top line: `┌── title ── count ──...──┐`
/// Sides:    `│`
/// Bottom:   `└──...──┘`
///
/// If `count` is `None`, the title fills the top bar alone.
pub fn render_panel_box(buf: &mut Buffer, area: Rect, title: &str, count: Option<&str>) {
    if area.width < 4 || area.height < 2 {
        return;
    }

    let x = area.x;
    let y = area.y;
    let w = area.width as usize;
    let bottom = area.y + area.height - 1;
    let bstyle = theme::border();

    // ── Top border ──────────────────────────────────────
    // Build: ┌── title ── count ──...──┐
    buf.set_string(x, y, "┌── ", bstyle);
    let mut col = x + 4;

    // Title in BRIGHT — clamp so a long title (or badge) never runs past the
    // right border. Reserve room for " ── <badge> " + the closing "┐".
    let right_edge = x + area.width - 1;
    let reserved = 1 + count.map(|c| c.len() + 4).unwrap_or(0); // "┐" + "── c "
    let title_budget = (right_edge.saturating_sub(col) as usize).saturating_sub(reserved);
    let written = put_clamped(buf, col, y, title, theme::bright(), title_budget);
    col += written;
    buf.set_string(col, y, " ", bstyle);
    col += 1;

    if let Some(c) = count {
        buf.set_string(col, y, "── ", bstyle);
        col += 3;
        buf.set_string(col, y, c, theme::dim());
        col += c.len() as u16;
        buf.set_string(col, y, " ", bstyle);
        col += 1;
    }

    // Fill remaining top with ─ and close with ┐
    while col < right_edge {
        buf.set_string(col, y, "─", bstyle);
        col += 1;
    }
    buf.set_string(right_edge, y, "┐", bstyle);

    // ── Side borders ────────────────────────────────────
    for row in (y + 1)..bottom {
        buf.set_string(x, row, "│", bstyle);
        buf.set_string(right_edge, row, "│", bstyle);
    }

    // ── Bottom border ───────────────────────────────────
    buf.set_string(x, bottom, "└", bstyle);
    for col in 1..(w - 1) {
        buf.set_string(x + col as u16, bottom, "─", bstyle);
    }
    buf.set_string(right_edge, bottom, "┘", bstyle);
}
