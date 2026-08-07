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
    crate::tui::paint_popup_border(frame, popup, theme);

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
        // The controls below keep their own foreground, but the selection bar
        // drawn above owns the background of a selected row.
        let over_bar = |style: Style| {
            if is_sel {
                crate::tui::inherit_background(style, selection)
            } else {
                style
            }
        };
        buf.set_string(
            row_x,
            ry,
            if is_sel { "› " } else { "  " },
            if is_sel { over_bar(focus) } else { label_style },
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
        let val_style = over_bar(if is_sel && editor.capturing {
            theme.style(StyleRole::KeybindValueCapturing)
        } else if is_sel {
            theme.style(StyleRole::KeybindValueBound)
        } else {
            theme.style(StyleRole::KeybindValue)
        });
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{fg_bg, marker, role_marker_theme, themed_app, RoleMarker};

    const SELECTION_FG: u32 = 0xb1_0001;
    const SELECTION_BG: u32 = 0xb1_0101;
    const MARKER_FG: u32 = 0xb1_0002;
    const MARKER_BG: u32 = 0xb1_0102;
    const BOUND_FG: u32 = 0xb1_0003;
    const BOUND_BG: u32 = 0xb1_0103;
    const CAPTURING_FG: u32 = 0xb1_0004;
    const CAPTURING_BG: u32 = 0xb1_0104;

    /// Every role that meets on a selected row gets both channels marked, so a
    /// cell can be checked for the *pair* it must end up with.
    const MARKERS: &[RoleMarker] = &[
        fg_bg(
            "components.keybind.row_selected",
            SELECTION_FG,
            SELECTION_BG,
        ),
        fg_bg("components.keybind.marker", MARKER_FG, MARKER_BG),
        fg_bg("components.keybind.value_bound", BOUND_FG, BOUND_BG),
        fg_bg(
            "components.keybind.value_capturing",
            CAPTURING_FG,
            CAPTURING_BG,
        ),
    ];

    fn editor_app(capturing: bool) -> App {
        let mut app = themed_app(role_marker_theme("keybind", MARKERS));
        app.mode = crate::app::AppMode::KeybindEditor;
        app.keybind_editor = Some(crate::app::KeybindEditor {
            selected: 0,
            scroll: 0,
            capturing,
            append: false,
            query: String::new(),
        });
        app
    }

    /// The selection bar's background must survive under every control drawn on
    /// the selected row, while each control keeps its own foreground.
    ///
    /// The bar is painted first and the controls are written over it. A control
    /// role that carries a background of its own therefore punched a hole in the
    /// bar; one that carries none used to be fine only by accident.
    #[test]
    fn selected_row_controls_keep_their_foreground_over_the_selection_background() {
        let area = Rect::new(0, 0, 80, 24);
        let sel_bg = marker(SELECTION_BG);

        for (capturing, value_fg, what) in [
            (false, BOUND_FG, "bound value"),
            (true, CAPTURING_FG, "capturing value"),
        ] {
            let app = editor_app(capturing);
            let buf = crate::test_support::frame_at(area, |f| render_keybind_editor(f, &app));

            // The popup is centred; the marker sits at the selected row's first
            // content column and the value column is a fixed offset from it.
            let popup = app.last_popup_rect.get().expect("the popup was laid out");
            let ry = popup.y + 3;
            let marker_cell = buf.cell((popup.x + 2, ry)).unwrap();
            assert_eq!(marker_cell.symbol(), "\u{203a}", "the focus marker");
            assert_eq!(marker_cell.fg, marker(MARKER_FG), "marker foreground");
            assert_eq!(marker_cell.bg, sel_bg, "marker background ({what})");

            let value_cell = buf.cell((popup.x + 33, ry)).unwrap();
            assert_eq!(value_cell.fg, marker(value_fg), "{what} foreground");
            assert_eq!(value_cell.bg, sel_bg, "{what} background");
        }
    }
}
