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
    let list_rows = area.height.saturating_sub(8).clamp(8, 20);
    let popup_h = (list_rows + 6).min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_h)) / 2;
    let popup = Rect::new(x, y, popup_w, popup_h);

    let popup = crate::tui::popup_open_rect(popup, app);

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(" Keybindings ", theme::heading()))
            .border_style(theme::popup_border()),
        popup,
    );

    let buf = frame.buffer_mut();
    let row_x = popup.x + 2;
    let val_x = popup.x + 33;
    let visible = popup.height.saturating_sub(4) as usize;
    let total = KeyAction::ALL.len();
    let scroll = editor.scroll.min(total.saturating_sub(visible));

    for (row, i) in (scroll..total).take(visible).enumerate() {
        let action = KeyAction::ALL[i];
        let ry = popup.y + 1 + row as u16;
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
        // Keep the label from bleeding into the value column at `val_x`.
        let label_avail = (val_x.saturating_sub(row_x + 1)) as usize;
        buf.set_string(
            row_x,
            ry,
            crate::tui::text::ellipsize(&format!("{marker}{}", action.label()), label_avail),
            label_style,
        );

        let binds = app.config.keybinds.binds(action).join(", ");
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
        let avail = popup
            .x
            .saturating_add(popup.width)
            .saturating_sub(val_x + 1) as usize;
        buf.set_string(
            val_x,
            ry,
            crate::tui::text::ellipsize(&value, avail),
            val_style,
        );
    }

    let hint_y = popup.y + popup.height.saturating_sub(2);
    let scroll_hint = if total > visible {
        format!(" ({}/{})", editor.selected + 1, total)
    } else {
        String::new()
    };
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
        crate::tui::text::ellipsize(
            &format!("{hint}{scroll_hint}"),
            popup.width.saturating_sub(4) as usize,
        ),
        theme::dim(),
    );
}
