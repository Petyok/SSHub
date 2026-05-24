use ratatui::prelude::{Modifier, Style};
use ratatui::style::Color;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::GroupFormEdit;

pub fn render_group_form(form: &GroupFormEdit) -> Paragraph<'static> {
    let title = if form.id.is_some() {
        "Rename group"
    } else {
        "New group"
    };
    let display = if form.name.is_empty() {
        "(empty)".to_string()
    } else {
        form.name.clone()
    };
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
    ];
    Paragraph::new(lines)
}
