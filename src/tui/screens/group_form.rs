use ratatui::prelude::{Modifier, Style};
use ratatui::style::Color;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::GroupFormEdit;

pub fn render_group_form(
    form: &GroupFormEdit,
    default_identity: Option<&str>,
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
    let lines = vec![
        Line::from(Span::styled(
            title,
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "> Name: ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                display,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Default identity: ", Style::default().fg(Color::Cyan)),
            Span::styled(
                identity_display,
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled("  (←/→)", Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "New hosts added to this group inherit its identity.",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    Paragraph::new(lines)
}
