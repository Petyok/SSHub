use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear};

use crate::app::{App, PickerKind};
use crate::tui::theme;

/// Dropdown overlay for the host form's Group / Identity field.
pub fn render_field_picker(frame: &mut Frame, app: &App) {
    let Some(picker) = app.field_picker.as_ref() else {
        return;
    };

    let (title, mut rows): (&str, Vec<String>) = match picker.kind {
        PickerKind::Group => {
            let mut rows = vec!["(none)".to_string()];
            rows.extend(app.groups.iter().map(|g| g.name.clone()));
            rows.push("+ New group…".to_string());
            ("Select group", rows)
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

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(format!(" {title} "), theme::heading()))
            .border_style(theme::popup_border()),
        popup,
    );

    let buf = frame.buffer_mut();
    let row_x = popup.x + 2;
    let max_rows = popup.height.saturating_sub(2) as usize;
    for (i, label) in rows.iter().enumerate().take(max_rows) {
        let ry = popup.y + 1 + i as u16;
        let is_sel = i == picker.selected && picker.creating.is_none();
        let is_create = Some(i) == create_index;
        let style = if is_sel {
            theme::selected()
        } else if is_create {
            theme::green()
        } else {
            theme::text()
        };
        if is_sel {
            let blank = " ".repeat(popup.width.saturating_sub(2) as usize);
            buf.set_string(popup.x + 1, ry, &blank, theme::selected());
        }
        let marker = if is_sel { "› " } else { "  " };
        buf.set_string(
            row_x,
            ry,
            crate::tui::text::ellipsize(
                &format!("{marker}{label}"),
                popup.width.saturating_sub(3) as usize,
            ),
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
            theme::bright(),
        );
    }
}
