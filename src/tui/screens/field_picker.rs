use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders};

use crate::app::{App, PickerKind};
use crate::theme::catalog::{ColorRole, StyleRole};

/// Dropdown overlay for the host form's Group / Identity field.
pub fn render_field_picker(frame: &mut Frame, app: &App) {
    let Some(picker) = app.field_picker.as_ref() else {
        return;
    };

    let (title, mut rows): (&str, Vec<String>) = match picker.kind {
        PickerKind::Group => {
            // Checkbox list: each group shows [x]/[ ] for its membership in the
            // form's selected set; the final row creates a new group inline.
            let selected = app.host_form.as_ref().map(|f| &f.group_ids);
            let mut rows: Vec<String> = app
                .groups
                .iter()
                .map(|g| {
                    let checked = selected.is_some_and(|s| s.contains(&g.id));
                    let mark = if checked { "[x] " } else { "[ ] " };
                    format!("{mark}{}", g.name)
                })
                .collect();
            rows.push("+ New group…".to_string());
            ("Select groups (Space toggles)", rows)
        }
        PickerKind::Identity => (
            "Select identity",
            app.identities.iter().map(|i| i.name.clone()).collect(),
        ),
    };
    if rows.is_empty() {
        rows.push("(no identities)".to_string());
    }

    // The last Group row is the create affordance.
    let create_index = if picker.kind == PickerKind::Group {
        Some(rows.len() - 1)
    } else {
        None
    };

    let area = frame.area();
    let inner_w = rows
        .iter()
        .map(|r| r.chars().count())
        .max()
        .unwrap_or(10)
        .max(title.len())
        .max(20) as u16;
    let popup_w = (inner_w + 4).min(area.width.saturating_sub(2));
    // rows + optional inline input line + borders.
    let extra = if picker.creating.is_some() { 1 } else { 0 };
    let popup_h = (rows.len() as u16 + 2 + extra).min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_h)) / 2;
    let popup = Rect::new(x, y, popup_w, popup_h);

    let popup = crate::tui::popup_open_rect(popup, app);

    let theme = app.theme();
    let selection = theme.style(StyleRole::PickerRowSelected);
    // The row marker used to be part of the styled label and therefore wore
    // the row's own selection style. It keeps that appearance under `default`
    // through its family's marker role, while staying separately themeable.
    let focus = theme.style(StyleRole::PickerMarker);

    crate::tui::open_popup(frame, popup, theme);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
                format!(" {title} "),
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
    let max_rows = popup.height.saturating_sub(2) as usize;
    for (i, label) in rows.iter().enumerate().take(max_rows) {
        let ry = popup.y + 1 + i as u16;
        let is_sel = i == picker.selected && picker.creating.is_none();
        let is_create = Some(i) == create_index;
        let style = if is_sel {
            selection
        } else if is_create {
            Style::default().fg(theme.color(ColorRole::StatusSuccess))
        } else {
            theme.style(StyleRole::PickerRow)
        };
        if is_sel {
            let blank = " ".repeat(popup.width.saturating_sub(2) as usize);
            buf.set_string(popup.x + 1, ry, &blank, selection);
        }
        // The marker is the focus indicator; it lands on the selection bar
        // painted just above, so it stays foreground-only.
        let avail = popup.width.saturating_sub(3) as usize;
        buf.set_string(row_x, ry, if is_sel { "› " } else { "  " }, {
            if is_sel {
                focus
            } else {
                style
            }
        });
        buf.set_string(
            row_x + 2,
            ry,
            crate::tui::text::ellipsize(label, avail.saturating_sub(2)),
            style,
        );
    }

    // Inline "new group name" entry.
    if let Some(name) = picker.creating.as_ref() {
        let iy = popup.y + popup.height.saturating_sub(1);
        let text = format!(
            "name: {}",
            crate::text_input::with_cursor(name, picker.cursor)
        );
        buf.set_string(
            row_x,
            iy.saturating_sub(1),
            crate::tui::text::ellipsize(&text, popup.width.saturating_sub(3) as usize),
            theme.style(StyleRole::FormInput),
        );
    }
}
