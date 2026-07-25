//! The shared searchable picker overlay. Renders whatever rows the app hands
//! it; the purpose supplies only the title and the empty-state text.

use ratatui::layout::{Margin, Rect};
use ratatui::prelude::*;
use ratatui::widgets::Clear;

use crate::app::{App, PickerBadge, PickerRow};
use crate::tui::theme;

/// Widest the popup ever gets. Session rows carry more than host rows, so this
/// is roomier than the old host-only picker needed.
const DESIRED_W: u16 = 56;
const MIN_W: u16 = 24;

/// `"\u{25cf} "` plus a four-character word plus a space.
const BADGE_CELLS: u16 = 7;
/// Up to three digits plus a space.
const ORDINAL_CELLS: u16 = 4;
/// `"  current"`.
const CURRENT_CELLS: u16 = 9;

/// What fits on one row at `width` cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RowLayout {
    /// Draw the `\u{25cf} word ` badge plus the ordinal — all of it or none.
    prefix: bool,
    /// Draw the `  current` suffix — all of it or none.
    current: bool,
    /// Cells left for the name and, if it still fits, the endpoint.
    body: u16,
}

/// Decide a row's layout. Kept as a pure function so the all-or-nothing rules
/// can be tested exactly, without rendering and scanning a buffer.
///
/// Prefix and suffix are all-or-nothing because half a badge or a clipped
/// "curr" is worse than their absence. The lifecycle and tab ordinal take
/// absolute priority; `current` takes the remaining fixed-width priority.
/// Whatever remains goes to the body, and since the endpoint is appended after
/// the name it is the first thing to vanish.
fn plan_row(width: u16, has_badge: bool, current: bool) -> RowLayout {
    let prefix = has_badge && width >= BADGE_CELLS + ORDINAL_CELLS;
    let rest = width.saturating_sub(if prefix {
        BADGE_CELLS + ORDINAL_CELLS
    } else {
        0
    });
    let current = current && rest >= CURRENT_CELLS;
    let body = if current { rest - CURRENT_CELLS } else { rest };
    RowLayout {
        prefix,
        current,
        body,
    }
}

pub fn render(frame: &mut Frame, app: &App) {
    let Some(picker) = app.session_picker.as_ref() else {
        return;
    };
    let rows = app.session_picker_rows();

    let area = frame.area();
    let list_rows = rows.len().clamp(1, 8) as u16;
    let popup_w = crate::tui::fit_popup(DESIRED_W, MIN_W, area.width);
    let popup_h = crate::tui::fit_popup(list_rows + 5, 5, area.height);
    if popup_w == 0 || popup_h == 0 {
        return;
    }
    let x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_h)) / 2;
    let popup = crate::tui::popup_open_rect(Rect::new(x, y, popup_w, popup_h), app);

    frame.render_widget(Clear, popup);
    frame.render_widget(
        ratatui::widgets::Block::default()
            .borders(ratatui::widgets::Borders::ALL)
            .title(Span::styled(picker.purpose.title(), theme::heading()))
            .border_style(Style::default().fg(theme::ACCENT)),
        popup,
    );

    // Everything below writes into the buffer directly. `set_stringn` clips
    // horizontally on its own, but an out-of-range *row* panics with "index
    // outside of buffer" — so the row budget is what has to be guarded here.
    // `fit_popup` only keeps the outer size legal; it says nothing about
    // whether `popup.y + 3` is still on screen.
    let inner = popup.inner(Margin::new(2, 1));
    if inner.width == 0 || inner.height < 3 {
        return;
    }

    let inner_w = inner.width as usize;
    let buf = frame.buffer_mut();

    let query_line = format!("/ {}\u{2588}", picker.query);
    buf.set_stringn(inner.x, inner.y, &query_line, inner_w, theme::bright());

    let sep: String = std::iter::repeat_n('\u{2500}', inner_w).collect();
    buf.set_stringn(inner.x, inner.y + 1, &sep, inner_w, theme::dim());

    // Keep the last inner row for the key hint whenever there is room.
    let list_top = inner.y + 2;
    let has_hint = inner.height >= 4;
    let visible = (inner.height as usize).saturating_sub(if has_hint { 3 } else { 2 });

    if rows.is_empty() {
        buf.set_stringn(
            inner.x,
            list_top,
            picker.purpose.empty_text(),
            inner_w,
            theme::mute(),
        );
    } else {
        let scroll = picker.selected.saturating_sub(visible.saturating_sub(1));
        for (i, row) in rows.iter().skip(scroll).take(visible).enumerate() {
            draw_row(
                buf,
                inner,
                list_top + i as u16,
                row,
                scroll + i == picker.selected,
            );
        }
    }

    if has_hint {
        buf.set_stringn(
            inner.x,
            inner.y + inner.height - 1,
            "type to filter · \u{2191}/\u{2193} · Enter · Esc",
            inner_w,
            theme::mute(),
        );
    }
}

fn badge_style(badge: PickerBadge) -> Style {
    match badge {
        PickerBadge::Up => theme::green(),
        PickerBadge::Connecting => theme::amber(),
        PickerBadge::Exited => theme::red(),
    }
}

/// Draw one row.
///
/// Widths are handled entirely by `set_stringn`, which measures terminal cells
/// and returns the column it stopped at. Nothing here counts `char`s: that
/// would drift on CJK, emoji and combining marks and let one segment overlap
/// the next.
///
/// Truncation priority falls out of the layout rather than being computed:
/// name and endpoint are written as a single string, so a narrow popup eats the
/// endpoint first. The fixed prefix is all-or-nothing — half a badge is worse
/// than none — and `current` is dropped entirely rather than shortened.
fn draw_row(buf: &mut Buffer, inner: Rect, y: u16, row: &PickerRow, selected: bool) {
    if y >= inner.bottom() {
        return;
    }
    let layout = plan_row(inner.width, row.badge.is_some(), row.current);
    let body_style = if selected {
        theme::selected()
    } else {
        theme::text()
    };
    if selected {
        let blank = " ".repeat(inner.width as usize);
        buf.set_stringn(inner.x, y, &blank, inner.width as usize, theme::selected());
    }

    let end = inner.right();
    let mut x = inner.x;

    if layout.prefix {
        let badge = row
            .badge
            .expect("plan_row only sets prefix when a badge exists");
        x = put(
            buf,
            x,
            y,
            end,
            &format!("\u{25cf} {:<4} ", badge.word()),
            badge_style(badge),
        );
        x = put(
            buf,
            x,
            y,
            end,
            &format!("{:<3} ", row.ordinal.unwrap_or(0)),
            body_style,
        );
    }

    if layout.body > 0 {
        let mut body = row.name.clone();
        if !row.endpoint.is_empty() {
            body.push_str("  ");
            body.push_str(&row.endpoint);
        }
        put(buf, x, y, x + layout.body, &body, body_style);
    }

    if layout.current {
        put(buf, end - CURRENT_CELLS, y, end, "  current", theme::mute());
    }
}

/// Write `text` at `x`, never past `end`, and return the column ratatui
/// actually stopped at. `set_stringn` measures in terminal cells, so this is
/// the only correct way to advance `x`.
fn put(buf: &mut Buffer, x: u16, y: u16, end: u16, text: &str, style: Style) -> u16 {
    if x >= end {
        return x;
    }
    let (next, _) = buf.set_stringn(x, y, text, (end - x) as usize, style);
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_is_all_or_nothing() {
        // Exactly at the threshold the prefix appears; one cell less and it is
        // dropped whole rather than truncated.
        let threshold = BADGE_CELLS + ORDINAL_CELLS;
        assert!(plan_row(threshold, true, false).prefix);
        assert!(!plan_row(threshold - 1, true, false).prefix);
        assert!(!plan_row(0, true, false).prefix);
        // Host rows carry no badge, so they never get a prefix.
        assert!(!plan_row(80, false, false).prefix);
    }

    #[test]
    fn current_is_all_or_nothing_and_takes_priority_over_the_body() {
        let l = plan_row(56, true, true);
        assert!(l.current);
        assert_eq!(l.body, 56 - BADGE_CELLS - ORDINAL_CELLS - CURRENT_CELLS);

        // At exactly the fixed-segment width, status, ordinal, and `current`
        // remain complete while the name yields all of its cells.
        let exact = BADGE_CELLS + ORDINAL_CELLS + CURRENT_CELLS;
        let l = plan_row(exact, true, true);
        assert!(l.prefix);
        assert!(l.current);
        assert_eq!(l.body, 0);

        // One cell less cannot hold the suffix intact, so it is dropped whole
        // and the body receives the remainder.
        let tight = exact - 1;
        let l = plan_row(tight, true, true);
        assert!(!l.current);
        assert_eq!(l.body, tight - BADGE_CELLS - ORDINAL_CELLS);
    }

    #[test]
    fn body_never_exceeds_the_row() {
        // Whatever the combination, the segments must fit inside `width`.
        for width in 0u16..80 {
            for (badge, current) in [(false, false), (true, false), (true, true)] {
                let l = plan_row(width, badge, current);
                let used = if l.prefix {
                    BADGE_CELLS + ORDINAL_CELLS
                } else {
                    0
                } + if l.current { CURRENT_CELLS } else { 0 }
                    + l.body;
                assert!(
                    used <= width,
                    "width {width} badge {badge} current {current}"
                );
            }
        }
    }
}
