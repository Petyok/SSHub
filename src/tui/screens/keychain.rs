use ratatui::prelude::{Modifier, Style};
use ratatui::style::Color;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::{App, AppMode, IdentityFormEdit, IdentityFormField};
use crate::text_input;

pub fn render_keychain(app: &App) -> Paragraph<'static> {
    let title = if app.mode == AppMode::IdentityForm {
        if app.identity_form.as_ref().and_then(|f| f.id).is_some() {
            "Edit identity"
        } else {
            "New identity"
        }
    } else {
        "Keychain"
    };
    Paragraph::new(title).style(Style::default().add_modifier(Modifier::BOLD))
}

pub fn render_identity_list(app: &App) -> List<'static> {
    let items: Vec<ListItem> = app
        .identities
        .iter()
        .enumerate()
        .map(|(idx, identity)| {
            let selected = idx == app.identity_selected;
            let marker = if selected { "▶ " } else { "  " };
            let user = identity
                .username
                .as_deref()
                .map(|u| format!(" ({u})"))
                .unwrap_or_default();
            let label = format!("{marker}{}{user}", identity.name);
            let style = if selected {
                Style::default().bg(Color::DarkGray).fg(Color::White)
            } else {
                Style::default()
            };
            ListItem::new(label).style(style)
        })
        .collect();
    List::new(items)
}

pub fn render_identity_form(form: &IdentityFormEdit) -> Paragraph<'static> {
    let mut lines = Vec::with_capacity(IdentityFormField::ALL.len() + 2);
    for field in IdentityFormField::ALL {
        let active = form.field == field;
        let editing = active && form.editing;
        let prefix = if editing {
            "▸ "
        } else if active {
            "> "
        } else {
            "  "
        };
        let display = match field {
            IdentityFormField::Password => {
                if editing {
                    text_input::with_cursor(&form.password, form.cursor)
                } else if !form.password.is_empty() {
                    "\u{25CF}".repeat(form.password.chars().count())
                } else if form.has_password {
                    "(set)".to_string()
                } else {
                    "(empty)".to_string()
                }
            }
            _ => {
                let value = match field {
                    IdentityFormField::Name => &form.name,
                    IdentityFormField::Username => &form.username,
                    IdentityFormField::PrivateKey => &form.private_key,
                    IdentityFormField::Certificate => &form.certificate,
                    IdentityFormField::Password => unreachable!(),
                };
                if editing {
                    text_input::with_cursor(value, form.cursor)
                } else if value.is_empty() {
                    "(empty)".to_string()
                } else {
                    value.clone()
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
            ratatui::text::Span::styled(format!("{prefix}{}: ", field.label()), label_style),
            ratatui::text::Span::styled(display, value_style),
        ]));
    }
    lines.push(ratatui::text::Line::from(""));
    lines.push(ratatui::text::Line::from(ratatui::text::Span::styled(
        "Enter: edit field │ F2/Ctrl+S: save │ Up/Down: navigate │ Esc: cancel",
        Style::default().add_modifier(Modifier::DIM),
    )));
    Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title("Identity"))
}

pub fn render_notice(notice: &str) -> Paragraph<'static> {
    let color = if notice.starts_with("Error") || notice.starts_with("error") {
        Color::Red
    } else {
        Color::Green
    };
    Paragraph::new(notice.to_string())
        .style(Style::default().fg(color).add_modifier(Modifier::ITALIC))
}
