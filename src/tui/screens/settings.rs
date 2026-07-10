use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear};

use crate::app::{App, SETTINGS_ITEMS};
use crate::tui::theme;

/// Settings overlay: a checkbox list of appearance toggles. Space/Enter flips
/// the highlighted row (persisted immediately); Esc closes.
pub fn render_settings(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let popup_w = 56u16.min(area.width.saturating_sub(2));
    let popup_h = (SETTINGS_ITEMS.len() as u16 + 6).min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_h)) / 2;
    let popup = Rect::new(x, y, popup_w, popup_h);

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(" Settings ", theme::heading()))
            .border_style(theme::popup_border()),
        popup,
    );

    let buf = frame.buffer_mut();
    let row_x = popup.x + 2;
    let inner_w = popup.width.saturating_sub(4) as usize;

    for (i, (label, _hint)) in SETTINGS_ITEMS.iter().enumerate() {
        let ry = popup.y + 1 + i as u16;
        if ry >= popup.y + popup.height - 2 {
            break;
        }
        let is_sel = i == app.settings_selected;
        if is_sel {
            let blank = " ".repeat(popup.width.saturating_sub(2) as usize);
            buf.set_string(popup.x + 1, ry, &blank, theme::selected());
        }
        let on = app.setting_value(i);
        let check = if on { "[x] " } else { "[ ] " };
        let check_style = if on { theme::green() } else { theme::mute() };
        let label_style = if is_sel {
            theme::white().bg(theme::SEL_BG)
        } else {
            theme::text()
        };
        let check_style = if is_sel {
            check_style.bg(theme::SEL_BG)
        } else {
            check_style
        };
        buf.set_string(row_x, ry, check, check_style);
        buf.set_string(
            row_x + 4,
            ry,
            crate::tui::text::ellipsize(label, inner_w.saturating_sub(4)),
            label_style,
        );
    }

    // Footer: the hint for the highlighted row + key legend.
    let hint = SETTINGS_ITEMS
        .get(app.settings_selected)
        .map(|(_, h)| *h)
        .unwrap_or("");
    let hint_y = popup.y + popup.height - 3;
    buf.set_string(
        row_x,
        hint_y,
        crate::tui::text::ellipsize(hint, inner_w),
        theme::dim(),
    );
    let legend = "Space toggle \u{b7} \u{2191}\u{2193} move \u{b7} Esc close";
    let legend_y = popup.y + popup.height - 2;
    buf.set_string(
        row_x,
        legend_y,
        crate::tui::text::ellipsize(legend, inner_w),
        theme::mute(),
    );
}
