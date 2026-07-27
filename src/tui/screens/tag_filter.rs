//! Popup for filtering the host list by a single tag (`#`).
//!
//! Type to narrow the list, `↑/↓` to move, `Enter` to apply, `Esc` to clear.

use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear};

use crate::app::App;
use crate::theme::catalog::StyleRole;

/// Render the tag-filter overlay.
pub fn render(frame: &mut Frame, app: &App) {
    let rows = app.tag_filter_rows();
    let has_tags = rows.len() > 1;

    let active = app.tag_filters.len();
    let title = if active > 0 {
        format!(" filter by tag · {active} active ")
    } else {
        " filter by tag ".to_string()
    };
    let query_line = format!("› {}\u{2588}", app.search_query);
    let hint = "↑/↓ move · Space toggle (multi) · Enter apply · Esc close";

    let empty_note = "no tags yet — add comma-separated tags on a host";

    let area = frame.area();
    let widest_row = rows.iter().map(|r| r.chars().count()).max().unwrap_or(0);
    let inner_w = widest_row
        .max(title.chars().count())
        .max(hint.chars().count())
        .max(query_line.chars().count())
        .max(if has_tags {
            0
        } else {
            empty_note.chars().count()
        })
        .max(28) as u16;
    let popup_w = (inner_w + 4).min(area.width.saturating_sub(2));
    // query line + separator + rows + hint + borders.
    let body_rows = if has_tags { rows.len() as u16 } else { 1 };
    let popup_h = (body_rows + 5).min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_h)) / 2;
    let popup = Rect::new(x, y, popup_w, popup_h);

    let popup = crate::tui::popup_open_rect(popup, app);

    let theme = app.theme();
    let legend = theme.style(StyleRole::PopupLegend);
    let selection = theme.style(StyleRole::PickerRowSelected);
    let row = theme.style(StyleRole::PickerRow);
    let focus = theme.style(StyleRole::FocusIndicator);

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(title, theme.style(StyleRole::PopupTitle)))
            .border_style(crate::tui::popup_border_style(theme, popup)),
        popup,
    );

    // Everything below writes into the buffer directly. `set_string` clips
    // columns on its own, but an out-of-range *row* panics — and `fit_popup`
    // only keeps the outer box legal, not its inner rows.
    if popup.width < 4 || popup.height < 4 {
        return;
    }

    let content_w = popup.width.saturating_sub(3) as usize;
    let row_x = popup.x + 2;
    let buf = frame.buffer_mut();

    // Query line at the top.
    buf.set_string(
        row_x,
        popup.y + 1,
        crate::tui::text::ellipsize(&query_line, content_w),
        theme.style(StyleRole::PickerQuery),
    );

    if !has_tags {
        buf.set_string(
            row_x,
            popup.y + 3,
            crate::tui::text::ellipsize(empty_note, content_w),
            legend,
        );
    } else {
        let list_top = popup.y + 3;
        let max_rows = popup.height.saturating_sub(5) as usize;
        for (i, label) in rows.iter().enumerate().take(max_rows) {
            let ry = list_top + i as u16;
            let is_sel = i == app.tag_filter_selected;
            let is_active = app.is_tag_active(label);
            let style = if is_sel { selection } else { row };
            if is_sel {
                let blank = " ".repeat(popup.width.saturating_sub(2) as usize);
                buf.set_string(popup.x + 1, ry, &blank, selection);
            }
            let suffix = if is_active { "  ✓" } else { "" };
            // Foreground-only marker over the selection bar drawn above.
            buf.set_string(
                row_x,
                ry,
                if is_sel { "› " } else { "  " },
                if is_sel { focus } else { style },
            );
            buf.set_string(
                row_x + 2,
                ry,
                crate::tui::text::ellipsize(
                    &format!("{label}{suffix}"),
                    content_w.saturating_sub(2),
                ),
                style,
            );
        }
    }

    let hint_y = popup.y + popup.height.saturating_sub(2);
    buf.set_string(
        row_x,
        hint_y,
        crate::tui::text::ellipsize(hint, content_w),
        legend,
    );
}
