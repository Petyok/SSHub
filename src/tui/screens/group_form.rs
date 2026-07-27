use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::app::{App, GroupFormEdit, GroupFormField};
use crate::theme::catalog::StyleRole;
use crate::theme::model::ResolvedTheme;

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

    let theme = app.theme();
    let mut items: Vec<ListItem> = Vec::with_capacity(options.len() + 1);
    items.push(ListItem::new(Span::styled(
        format!(" {none_label}"),
        theme.style(StyleRole::PopupLegend),
    )));
    let row = theme.style(StyleRole::PickerRow);
    items.extend(
        options
            .iter()
            .map(|(_, name)| ListItem::new(Span::styled(format!(" {name}"), row))),
    );

    let area = frame.area();
    let width = crate::tui::fit_popup(area.width * 40 / 100, 24, area.width.saturating_sub(2));
    let height = crate::tui::fit_popup((items.len() as u16) + 2, 4, area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let popup = Rect::new(x, y, width, height);

    let popup = crate::tui::popup_open_rect(popup, app);

    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(crate::tui::popup_border_style(theme, popup))
        .title(Span::styled(title, theme.style(StyleRole::PopupTitle)));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let mut state = ListState::default();
    state.select(Some(picker.selected.min(items.len().saturating_sub(1))));
    frame.render_stateful_widget(
        List::new(items).highlight_style(theme.style(StyleRole::PickerRowSelected)),
        inner,
        &mut state,
    );
}

pub fn render_group_form(
    form: &GroupFormEdit,
    default_identity: Option<&str>,
    parent_group: Option<&str>,
    theme: &ResolvedTheme,
    border: Style,
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

    let help = theme.style(StyleRole::FormHelp);
    // Same split as the host form: the `▸` marker is the focus indicator and
    // stays themeable apart from the label it sits next to.
    let focus = theme.style(StyleRole::FocusIndicator);

    // One labelled row per field; the focused one gets a marker and a
    // highlighted label so ↑/↓ navigation is obvious.
    let field_row = |field: GroupFormField, label: &str, value: String, is_picker: bool| {
        let focused = form.field == field;
        let marker = if focused { "\u{25b8} " } else { "  " };
        let label_style = if focused {
            theme.style(StyleRole::FormLabelFocused)
        } else {
            theme.style(StyleRole::FormLabel)
        };
        let value_style = if focused {
            theme.style(StyleRole::FormInputFocused)
        } else {
            theme.style(StyleRole::FormValue)
        };
        let mut spans = vec![
            Span::styled(marker, if focused { focus } else { label_style }),
            Span::styled(format!("{label}: "), label_style),
            Span::styled(value, value_style),
        ];
        if focused && is_picker {
            spans.push(Span::styled("   Enter to choose", help));
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
            help,
        )),
    ];
    Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border)
            .title(Span::styled(
                format!(" {title} "),
                theme.style(StyleRole::PopupTitle),
            )),
    )
}
