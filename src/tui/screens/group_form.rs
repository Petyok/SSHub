use ratatui::layout::Rect;
use ratatui::prelude::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::app::{App, GroupFormEdit, GroupFormField};
use crate::tui::theme;

/// Dropdown list picker for the group form's Parent / Identity field.
pub fn render_group_field_picker(frame: &mut Frame, app: &App) {
    let Some(picker) = app.group_field_picker.as_ref() else {
        return;
    };
    let (none_label, options) = app.group_field_picker_options();
    let title = match picker.kind {
        GroupFormField::Parent => " Parent group ",
        GroupFormField::Identity => " Default identity ",
        GroupFormField::Name => " Select ",
    };

    let mut items: Vec<ListItem> = Vec::with_capacity(options.len() + 1);
    items.push(ListItem::new(Span::styled(format!(" {none_label}"), theme::mute())));
    items.extend(
        options
            .iter()
            .map(|(_, name)| ListItem::new(Span::styled(format!(" {name}"), theme::text()))),
    );

    let area = frame.area();
    let width = (area.width * 40 / 100).clamp(24, area.width.saturating_sub(2));
    let height = ((items.len() as u16) + 2).clamp(4, area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let popup = Rect::new(x, y, width, height);

    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border())
        .title(Span::styled(title, theme::heading()));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let mut state = ListState::default();
    state.select(Some(picker.selected.min(items.len().saturating_sub(1))));
    frame.render_stateful_widget(
        List::new(items).highlight_style(theme::selected()),
        inner,
        &mut state,
    );
}

pub fn render_group_form(
    form: &GroupFormEdit,
    default_identity: Option<&str>,
    parent_group: Option<&str>,
) -> Paragraph<'static> {
    let title = if form.id.is_some() {
        "Edit group"
    } else {
        "New group"
    };
    let display = if form.name.is_empty() {
        "(empty)".to_string()
    } else {
        form.name.clone()
    };
    let identity_display = default_identity.unwrap_or("(none)").to_string();
    let parent_display = parent_group.unwrap_or("(top level)").to_string();

    // One labelled row per field; the focused one gets a `▸` marker and a
    // highlighted label so ↑/↓ navigation is obvious.
    let field_row = |field: GroupFormField, label: &str, value: String, is_picker: bool| {
        let focused = form.field == field;
        let marker = if focused { "\u{25b8} " } else { "  " };
        let label_style = if focused {
            theme::heading()
        } else {
            theme::mute()
        };
        let value_style = if focused {
            theme::bright().add_modifier(Modifier::BOLD)
        } else {
            theme::text()
        };
        let mut spans = vec![
            Span::styled(format!("{marker}{label}: "), label_style),
            Span::styled(value, value_style),
        ];
        if focused && is_picker {
            spans.push(Span::styled("   Enter to choose", theme::dim()));
        }
        Line::from(spans)
    };

    let lines = vec![
        Line::from(""),
        field_row(GroupFormField::Name, "Name", display, false),
        Line::from(""),
        field_row(GroupFormField::Parent, "Parent group", parent_display, true),
        Line::from(""),
        field_row(
            GroupFormField::Identity,
            "Default identity",
            identity_display,
            true,
        ),
        Line::from(""),
        Line::from(Span::styled(
            "\u{2191}\u{2193} move field  ·  Enter save/choose  ·  Esc cancel",
            theme::dim(),
        )),
    ];
    Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(theme::border())
            .title(Span::styled(format!(" {title} "), theme::heading())),
    )
}
