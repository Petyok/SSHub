use ratatui::prelude::{Modifier, Style};
use ratatui::style::Color;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::{KeygenFormEdit, KeygenFormField};
use crate::text_input;

pub fn render_keygen_form(form: &KeygenFormEdit, save_hint: &str) -> Paragraph<'static> {
    let mut lines = Vec::with_capacity(KeygenFormField::ALL.len() + 2);
    for field in KeygenFormField::ALL {
        let active = form.field == field;
        let editing = active;
        let prefix = if editing {
            "▸ "
        } else if active {
            "> "
        } else {
            "  "
        };
        let display = match field {
            KeygenFormField::KeyType => {
                let val = form.key_type.label();
                if active {
                    format!("[ {} ]", val)
                } else {
                    val.to_string()
                }
            }
            KeygenFormField::Passphrase => {
                if editing {
                    text_input::with_cursor(&form.passphrase, form.cursor)
                } else if !form.passphrase.is_empty() {
                    "\u{25CF}".repeat(form.passphrase.chars().count())
                } else {
                    "(empty)".to_string()
                }
            }
            KeygenFormField::Comment => {
                if editing {
                    text_input::with_cursor(&form.comment, form.cursor)
                } else if form.comment.is_empty() {
                    "(empty)".to_string()
                } else {
                    form.comment.clone()
                }
            }
            KeygenFormField::TargetPath => {
                if editing {
                    text_input::with_cursor(&form.target_path, form.cursor)
                } else if form.target_path.is_empty() {
                    "(empty)".to_string()
                } else {
                    form.target_path.clone()
                }
            }
        };
        let label_style = if editing {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else if active {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let value_style = if editing {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else if active {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        lines.push(ratatui::text::Line::from(vec![
            ratatui::text::Span::styled(
                format!("{prefix}{}: ", field.label()),
                label_style,
            ),
            ratatui::text::Span::styled(display, value_style),
        ]));
    }
    lines.push(ratatui::text::Line::from(""));
    lines.push(ratatui::text::Line::from(ratatui::text::Span::styled(
        format!("type to edit │ Tab/↓: next │ {save_hint}: save │ Esc: cancel"),
        Style::default().add_modifier(Modifier::DIM),
    )));
    Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Generate SSH Key"),
    )
}
