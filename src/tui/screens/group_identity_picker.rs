//! Dedicated popup for choosing a group's default identity (`e` on a group).

use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear};

use crate::app::App;
use crate::tui::theme;

/// Render the group default-identity picker overlay.
pub fn render(frame: &mut Frame, app: &App) {
    let Some(picker) = app.group_identity_picker.as_ref() else {
        return;
    };

    // Row 0 is the "(none)" option; the rest map to identities in order.
    let mut rows: Vec<String> = vec!["(none)".to_string()];
    rows.extend(app.identities.iter().map(|i| {
        match &i.username {
            Some(u) if !u.is_empty() => format!("{}  ({u})", i.name),
            _ => i.name.clone(),
        }
    }));

    let title = format!(" default identity · {} ", picker.group_name);
    let hint = "↑/↓ move · Enter select · Esc cancel";

    let area = frame.area();
    let inner_w = rows
        .iter()
        .map(|r| r.chars().count())
        .max()
        .unwrap_or(10)
        .max(title.chars().count())
        .max(hint.chars().count())
        .max(24) as u16;
    let popup_w = (inner_w + 4).min(area.width.saturating_sub(2));
    // rows + hint line + blank separator + borders.
    let popup_h = (rows.len() as u16 + 4).min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_h)) / 2;
    let popup = Rect::new(x, y, popup_w, popup_h);

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(title, theme::heading()))
            .border_style(theme::border()),
        popup,
    );

    let buf = frame.buffer_mut();
    let row_x = popup.x + 2;
    let max_rows = popup.height.saturating_sub(3) as usize;
    for (i, label) in rows.iter().enumerate().take(max_rows) {
        let ry = popup.y + 1 + i as u16;
        let is_sel = i == picker.selected;
        let style = if is_sel { theme::selected() } else { theme::text() };
        if is_sel {
            let blank = " ".repeat(popup.width.saturating_sub(2) as usize);
            buf.set_string(popup.x + 1, ry, &blank, theme::selected());
        }
        let marker = if is_sel { "› " } else { "  " };
        buf.set_string(
            row_x,
            ry,
            crate::tui::text::ellipsize(&format!("{marker}{label}"), (popup.width - 3) as usize),
            style,
        );
    }

    // Hint line at the bottom.
    let hint_y = popup.y + popup.height.saturating_sub(2);
    buf.set_string(
        row_x,
        hint_y,
        crate::tui::text::ellipsize(hint, (popup.width - 3) as usize),
        theme::mute(),
    );
}
