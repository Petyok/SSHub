//! Popup over a public-key push running in the background (`Shift+P`):
//! progress, then the outcome, without opening a terminal tab.

use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::{App, KeyPushState};
use crate::theme::catalog::{ColorRole, StyleRole};

/// Key hints inside the popup and in the footer, per state.
pub fn hints(state: &KeyPushState) -> &'static [(&'static str, &'static str)] {
    match state {
        KeyPushState::Running => &[("Esc", "cancel")],
        KeyPushState::NeedsInput => &[("\u{21b5}", "open terminal"), ("Esc", "cancel")],
        _ => &[("\u{21b5}/Esc", "close")],
    }
}

pub fn render(frame: &mut Frame, app: &App) {
    let Some(push) = app.key_push.as_ref() else {
        return;
    };
    let theme = app.theme();
    let (title, style) = match push.state {
        KeyPushState::Running => (" push key ", theme.style(StyleRole::PopupTitle)),
        KeyPushState::NeedsInput => (
            " push key: input needed ",
            theme.style(StyleRole::PopupWarning),
        ),
        KeyPushState::Installed | KeyPushState::AlreadyPresent => (
            " push key: done ",
            Style::default().fg(theme.color(ColorRole::StatusSuccess)),
        ),
        KeyPushState::Failed(_) => (" push key: failed ", theme.style(StyleRole::PopupError)),
        KeyPushState::Cancelled => (
            " push key: cancelled ",
            theme.style(StyleRole::PopupWarning),
        ),
    };
    let mut message = push.message();
    if push.state == KeyPushState::Running {
        let spin = crate::tui::tween::spinner_frame(push.started.elapsed());
        message = format!("{spin} {message}");
    }
    let hint = hints(&push.state)
        .iter()
        .map(|(k, v)| format!("{k} {v}"))
        .collect::<Vec<_>>()
        .join(" · ");

    // Sized like the notice popup: grows with the wrapped message.
    let area = frame.area();
    let popup_width = 60u16.min(area.width).max(20.min(area.width));
    let inner_w = popup_width.saturating_sub(2).max(1) as usize;
    let msg_lines = message
        .split('\n')
        .map(|l| (l.chars().count() / inner_w) + 1)
        .sum::<usize>()
        .max(1);
    let popup_height = ((msg_lines as u16) + 4)
        .min(area.height)
        .max(5.min(area.height));
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = crate::tui::popup_open_rect(Rect::new(x, y, popup_width, popup_height), app);
    // `Text` splits the message on '\n'; a `Line` would not.
    let mut text = Text::styled(message, style);
    text.push_line(Line::default());
    text.push_line(Line::styled(hint, theme.style(StyleRole::PopupHint)));
    crate::tui::open_popup(frame, popup_area, theme);
    frame.render_widget(
        Paragraph::new(text).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(title, style))
                .border_style(style),
        ),
        popup_area,
    );
}
