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

    // `visible` maps each drawn row to its full-list index (see
    // `App::field_picker_rows`); `picker.selected` indexes the drawn rows.
    let visible = app.field_picker_rows();
    let (title, rows, create_index): (&str, Vec<String>, Option<usize>) = match picker.kind {
        PickerKind::Group => {
            // Checkbox list: each group shows [x]/[ ] for its membership in the
            // form's selected set; the final row creates a new group inline.
            let selected = app.host_form.as_ref().map(|f| &f.group_ids);
            let rows = visible
                .iter()
                .map(|&i| match app.groups.get(i) {
                    Some(g) => {
                        let checked = selected.is_some_and(|s| s.contains(&g.id));
                        let mark = if checked { "[x] " } else { "[ ] " };
                        format!("{mark}{}", g.name)
                    }
                    None => "+ New group…".to_string(),
                })
                .collect::<Vec<_>>();
            let create = Some(rows.len() - 1);
            ("Select groups (Space toggles)", rows, create)
        }
        PickerKind::Identity => {
            // Row 0 restores inheritance (host field cleared to None).
            let rows = visible
                .iter()
                .map(
                    |&i| match i.checked_sub(1).and_then(|i| app.identities.get(i)) {
                        Some(id) => id.name.clone(),
                        None => "(inherit from group)".to_string(),
                    },
                )
                .collect();
            ("Select identity", rows, None)
        }
    };
    let title = if picker.query.is_empty() {
        format!(" {title} ")
    } else {
        format!(" {title}  /{} ", picker.query)
    };
    // Only the pinned special row is left: say so on a non-selectable line.
    let empty_line = (!picker.query.is_empty() && rows.len() == 1)
        .then(|| format!("no match for \"{}\"", picker.query));

    let area = frame.area();
    let inner_w = rows
        .iter()
        .chain(empty_line.as_ref())
        .map(|r| r.chars().count())
        .max()
        .unwrap_or(10)
        .max(title.chars().count())
        .max(20) as u16;
    let popup_w = (inner_w + 4).min(area.width.saturating_sub(2));
    // rows + optional empty-state / inline input lines + borders.
    let extra = u16::from(picker.creating.is_some()) + u16::from(empty_line.is_some());
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
            .title(Span::styled(title, theme.style(StyleRole::PopupTitle)))
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
        app.record_click(
            Rect::new(popup.x + 1, ry, popup.width - 2, 1),
            crate::app::ClickHit::Row(i),
        );
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
    if let Some(line) = empty_line.as_ref().filter(|_| rows.len() < max_rows) {
        buf.set_string(
            row_x + 2,
            popup.y + 1 + rows.len() as u16,
            crate::tui::text::ellipsize(line, popup.width.saturating_sub(5) as usize),
            theme.style(StyleRole::PickerRow),
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
