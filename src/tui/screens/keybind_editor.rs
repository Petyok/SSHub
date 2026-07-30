use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders};

use crate::app::App;
use crate::theme::catalog::StyleRole;

/// Keybinding editor overlay: one row per configurable action.
pub fn render_keybind_editor(frame: &mut Frame, app: &App) {
    let Some(editor) = app.keybind_editor.as_ref() else {
        return;
    };

    let actions = app.filtered_keybind_actions();
    let area = frame.area();
    let popup_w = 60u16.min(area.width.saturating_sub(2));
    let list_rows = area.height.saturating_sub(9).clamp(8, 20);
    let popup_h = (list_rows + 7).min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_h)) / 2;
    let popup = Rect::new(x, y, popup_w, popup_h);

    let popup = crate::tui::popup_open_rect(popup, app);

    let theme = app.theme();
    // The catalogue gives the keybind list `text_highlight` on the selection
    // background, deliberately unlike the keychain's `selection_fg`.
    let selection = theme.style(StyleRole::KeybindRowSelected);
    // The marker was part of the styled label, so it wore the highlighted
    // row's own colours; its family carries that as a role of its own.
    let focus = theme.style(StyleRole::KeybindMarker);

    crate::tui::open_popup(frame, popup, theme);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
                " Keybindings ",
                theme.style(StyleRole::PopupTitle),
            ))
            .border_style(crate::tui::popup_border_style(theme, popup)),
        popup,
    );

    // Everything below writes into the buffer directly. `set_string` clips
    // columns on its own, but an out-of-range *row* panics — and `fit_popup`
    // only keeps the outer box legal, not its inner rows.
    if popup.width < 4 || popup.height < 4 {
        return;
    }

    let buf = frame.buffer_mut();
    let row_x = popup.x + 2;
    let val_x = popup.x + 33;
    let content_w = popup.width.saturating_sub(4) as usize;

    let query_line = format!("› {}\u{2588}", editor.query);
    buf.set_string(
        row_x,
        popup.y + 1,
        crate::tui::text::ellipsize(&query_line, content_w),
        theme.style(StyleRole::PickerQuery),
    );

    let visible = popup.height.saturating_sub(5) as usize;
    let total = actions.len();
    let scroll = editor.scroll.min(total.saturating_sub(visible));
    let selected = if total == 0 {
        0
    } else {
        editor.selected.min(total - 1)
    };

    for (row, i) in (scroll..total).take(visible).enumerate() {
        let action = actions[i];
        let ry = popup.y + 3 + row as u16;
        let is_sel = i == selected;
        if is_sel {
            let blank = " ".repeat(popup.width.saturating_sub(2) as usize);
            buf.set_string(popup.x + 1, ry, &blank, selection);
        }
        let label_style = if is_sel {
            selection
        } else {
            theme.style(StyleRole::KeybindRow)
        };
        // Keep the label from bleeding into the value column at `val_x`.
        let label_avail = (val_x.saturating_sub(row_x + 1)) as usize;
        // Foreground-only marker over the selection bar drawn above.
        buf.set_string(
            row_x,
            ry,
            if is_sel { "› " } else { "  " },
            if is_sel { focus } else { label_style },
        );
        buf.set_string(
            row_x + 2,
            ry,
            crate::tui::text::ellipsize(action.label(), label_avail.saturating_sub(2)),
            label_style,
        );

        let binds = app.config.keybinds.binds(action).join(", ");
        let value = if is_sel && editor.capturing {
            "press a key…".to_string()
        } else {
            binds
        };
        let val_style = if is_sel && editor.capturing {
            theme.style(StyleRole::KeybindValueCapturing)
        } else if is_sel {
            theme.style(StyleRole::KeybindValueBound)
        } else {
            theme.style(StyleRole::KeybindValue)
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
        format!(" ({}/{})", selected + 1, total)
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
        "type to filter │ ↑↓ move │ Enter: set │ Ctrl+A: add │ Ctrl+R: reset │ Ctrl+X: unbind │ Esc: clear/close"
    };
    buf.set_string(
        row_x,
        hint_y,
        crate::tui::text::ellipsize(
            &format!("{hint}{scroll_hint}"),
            popup.width.saturating_sub(4) as usize,
        ),
        theme.style(StyleRole::PopupHint),
    );
}
