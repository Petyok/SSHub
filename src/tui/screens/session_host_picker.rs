//! Searchable host picker for opening a new embedded SSH session tab.

use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::widgets::Clear;

use crate::app::App;
use crate::tui::theme;

pub fn render(frame: &mut Frame, app: &App) {
    let Some(picker) = app.session_host_picker.as_ref() else {
        return;
    };
    let matches = app.session_host_matches();

    let area = frame.area();
    let popup_w = 48u16.min(area.width.saturating_sub(4)).max(30);
    let list_rows = matches.len().clamp(1, 8) as u16;
    let popup_h = (list_rows + 5).min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_h)) / 2;
    let popup = Rect::new(x, y, popup_w, popup_h);

    frame.render_widget(Clear, popup);
    frame.render_widget(
        ratatui::widgets::Block::default()
            .borders(ratatui::widgets::Borders::ALL)
            .title(Span::styled(" new session tab ", theme::heading()))
            .border_style(Style::default().fg(theme::ACCENT)),
        popup,
    );

    let buf = frame.buffer_mut();
    let row_x = popup.x + 2;
    let inner_w = popup.width.saturating_sub(3) as usize;

    let query_line = format!("/ {}\u{2588}", picker.query);
    buf.set_string(
        row_x,
        popup.y + 1,
        crate::tui::text::ellipsize(&query_line, inner_w),
        theme::bright(),
    );

    let sep: String = std::iter::repeat_n('\u{2500}', inner_w).collect();
    buf.set_string(row_x, popup.y + 2, &sep, theme::dim());

    let list_top = popup.y + 3;
    let visible = popup.height.saturating_sub(5) as usize;
    if matches.is_empty() {
        buf.set_string(row_x, list_top, "(no matching hosts)", theme::mute());
    } else {
        let scroll = picker.selected.saturating_sub(visible.saturating_sub(1));
        for (i, (_, name)) in matches.iter().skip(scroll).take(visible).enumerate() {
            let idx = scroll + i;
            let ry = list_top + i as u16;
            let is_sel = idx == picker.selected;
            let style = if is_sel {
                theme::selected()
            } else {
                theme::text()
            };
            if is_sel {
                let blank = " ".repeat(popup.width.saturating_sub(2) as usize);
                buf.set_string(popup.x + 1, ry, &blank, theme::selected());
            }
            let marker = if is_sel { "\u{203a} " } else { "  " };
            buf.set_string(
                row_x,
                ry,
                crate::tui::text::ellipsize(&format!("{marker}{name}"), inner_w),
                style,
            );
        }
    }

    let hint_y = popup.y + popup.height.saturating_sub(2);
    buf.set_string(
        row_x,
        hint_y,
        crate::tui::text::ellipsize("type to filter · \u{2191}/\u{2193} · Enter · Esc", inner_w),
        theme::mute(),
    );
}
