use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear};

use crate::app::App;
use crate::config::KeyAction;
use crate::tui::theme;

/// Keybinding editor overlay: one row per configurable action.
pub fn render_keybind_editor(frame: &mut Frame, app: &App) {
    let Some(editor) = app.keybind_editor else {
        return;
    };

    let area = frame.area();
    let popup_w = 60u16.min(area.width.saturating_sub(2));
    let popup_h = (KeyAction::ALL.len() as u16 + 6).min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_h)) / 2;
    let popup = Rect::new(x, y, popup_w, popup_h);

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(" Keybindings ", theme::heading()))
            .border_style(theme::border()),
        popup,
    );

    let buf = frame.buffer_mut();
    let row_x = popup.x + 2;
    let val_x = popup.x + 22;

    for (i, action) in KeyAction::ALL.iter().enumerate() {
        let ry = popup.y + 1 + i as u16;
        let is_sel = i == editor.selected;
        if is_sel {
            let blank = " ".repeat(popup.width.saturating_sub(2) as usize);
            buf.set_string(popup.x + 1, ry, &blank, theme::selected());
        }
        let label_style = if is_sel {
            theme::white().bg(theme::SEL_BG)
        } else {
            theme::text()
        };
        let marker = if is_sel { "› " } else { "  " };
        buf.set_string(row_x, ry, format!("{marker}{}", action.label()), label_style);

        let binds = app.config.keybinds.binds(*action).join(", ");
        let value = if is_sel && editor.capturing {
            "press a key…".to_string()
        } else {
            binds
        };
        let val_style = if is_sel && editor.capturing {
            theme::amber().bg(theme::SEL_BG)
        } else if is_sel {
            theme::green().bg(theme::SEL_BG)
        } else {
            theme::mute()
        };
        let avail = popup.width.saturating_sub(24) as usize;
        buf.set_string(
            val_x,
            ry,
            crate::tui::text::ellipsize(&value, avail),
            val_style,
        );
    }

    // Hint line at the bottom.
    let hint_y = popup.y + popup.height.saturating_sub(2);
    let hint = if editor.capturing {
        if editor.append {
            "press a key to add  │  Esc: cancel"
        } else {
            "press a key to bind  │  Esc: cancel"
        }
    } else {
        "↑↓ move │ Enter: set │ a: add │ r: reset │ x: unbind │ Esc: close"
    };
    buf.set_string(
        row_x,
        hint_y,
        crate::tui::text::ellipsize(hint, popup.width.saturating_sub(4) as usize),
        theme::dim(),
    );
}
