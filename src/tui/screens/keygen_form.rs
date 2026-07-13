//! Centered popup for generating a new ed25519 key on the identities tab.

use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::widgets::Clear;

use crate::app::App;
use crate::tui::theme;

pub fn render(frame: &mut Frame, app: &App) {
    let Some(form) = app.keygen_form.as_ref() else {
        return;
    };

    let area = frame.area();
    let popup_w = 54u16.min(area.width.saturating_sub(4)).max(32);
    let popup_h = 9u16.min(area.height.saturating_sub(2)).max(7);
    let x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_h)) / 2;
    let popup = Rect::new(x, y, popup_w, popup_h);

    frame.render_widget(Clear, popup);
    frame.render_widget(
        ratatui::widgets::Block::default()
            .borders(ratatui::widgets::Borders::ALL)
            .title(Span::styled(" generate ed25519 key ", theme::heading()))
            .border_style(Style::default().fg(theme::ACCENT)),
        popup,
    );

    let buf = frame.buffer_mut();
    let row_x = popup.x + 2;
    let inner_w = popup.width.saturating_sub(3) as usize;
    let val_w = inner_w.saturating_sub(12);

    // Name field.
    let name_active = form.name_active();
    let name_val = if name_active {
        crate::text_input::with_cursor(&form.name, form.cursor)
    } else if form.name.is_empty() {
        "(required)".to_string()
    } else {
        form.name.clone()
    };
    buf.set_string(row_x, popup.y + 1, "Name:", label_style(name_active));
    buf.set_string(
        row_x + 12,
        popup.y + 1,
        crate::tui::text::ellipsize(&name_val, val_w),
        value_style(name_active),
    );

    // Passphrase field (masked).
    let pass_active = form.passphrase_active();
    let masked: String = "\u{25CF}".repeat(form.passphrase.chars().count());
    let pass_val = if pass_active {
        format!("{masked}\u{2588}")
    } else if form.passphrase.is_empty() {
        "(optional)".to_string()
    } else {
        masked
    };
    buf.set_string(row_x, popup.y + 2, "Passphrase:", label_style(pass_active));
    buf.set_string(
        row_x + 12,
        popup.y + 2,
        crate::tui::text::ellipsize(&pass_val, val_w),
        value_style(pass_active),
    );

    buf.set_string(
        row_x,
        popup.y + 4,
        crate::tui::text::ellipsize("type: ed25519 (fixed)", inner_w),
        theme::mute(),
    );
    buf.set_string(
        row_x,
        popup.y + 5,
        crate::tui::text::ellipsize("Saved under the app data directory (keys/).", inner_w),
        theme::mute(),
    );

    let hint_y = popup.y + popup.height.saturating_sub(2);
    buf.set_string(
        row_x,
        hint_y,
        crate::tui::text::ellipsize(
            "Tab/\u{2193} next \u{00b7} Enter saves \u{00b7} Esc cancel",
            inner_w,
        ),
        theme::mute(),
    );
}

fn label_style(active: bool) -> Style {
    if active {
        theme::cyan().add_modifier(Modifier::BOLD)
    } else {
        theme::dim()
    }
}

fn value_style(active: bool) -> Style {
    if active {
        theme::bright()
    } else {
        theme::text()
    }
}
