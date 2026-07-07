use ratatui::prelude::{Modifier, Style};
use ratatui::style::Color;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::{HostFormEdit, HostFormField, OS_ICON_OPTIONS};
use crate::store::{HostGroup, Identity};
use crate::text_input;

pub fn render_host_form(
    form: &HostFormEdit,
    groups: &[HostGroup],
    identities: &[Identity],
    save_hint: &str,
) -> Paragraph<'static> {
    let title = if form.metadata_only {
        "Edit metadata (ssh_config)"
    } else if form.id.is_some() {
        "Edit host"
    } else {
        "New host"
    };

    let mut lines = Vec::with_capacity(HostFormField::ALL.len() + 2);
    if form.metadata_only {
        lines.push(Line::from(Span::styled(
            "Connection fields are read-only (from ~/.ssh/config). Edit launcher metadata below.",
            Style::default().add_modifier(Modifier::DIM),
        )));
        lines.push(Line::from(""));
    }
    for field in HostFormField::ALL {
        let active = form.field == field;
        let editing = active && form.editing;
        let prefix = if editing {
            "▸ "
        } else if active {
            "> "
        } else {
            "  "
        };
        let read_only = form.metadata_only && field.is_connection_field();
        let (label, display) = match field {
            HostFormField::Address => (
                "Address",
                if editing {
                    text_input::with_cursor(&form.address, form.cursor)
                } else {
                    display_text(&form.address)
                },
            ),
            HostFormField::Label => (
                "Label",
                if editing {
                    text_input::with_cursor(&form.label, form.cursor)
                } else {
                    display_text(&form.label)
                },
            ),
            HostFormField::Name => (
                "Name (alias)",
                if editing {
                    text_input::with_cursor(&form.name, form.cursor)
                } else {
                    display_text(&form.name)
                },
            ),
            HostFormField::Port => (
                "Port",
                if editing {
                    text_input::with_cursor(&form.port, form.cursor)
                } else {
                    display_text(&form.port)
                },
            ),
            HostFormField::Group => ("Group", group_label(form.group_index, groups)),
            HostFormField::Identity => {
                ("Identity", identity_label(form.identity_index, identities))
            }
            HostFormField::Tags => (
                "Tags (comma-separated)",
                if editing {
                    text_input::with_cursor(&form.tags, form.cursor)
                } else {
                    display_text(&form.tags)
                },
            ),
            HostFormField::ProxyJump => (
                "ProxyJump",
                if editing {
                    text_input::with_cursor(&form.proxy_jump, form.cursor)
                } else {
                    display_text(&form.proxy_jump)
                },
            ),
            HostFormField::ForwardAgent => (
                "Agent forward",
                if form.forward_agent {
                    "enabled (Space to toggle)"
                } else {
                    "disabled (Space to toggle)"
                }
                .into(),
            ),
            HostFormField::RemoteCommand => (
                "Startup command",
                if editing {
                    text_input::with_cursor(&form.remote_command, form.cursor)
                } else {
                    display_text(&form.remote_command)
                },
            ),
            HostFormField::OsIcon => ("OS icon", os_icon_label(form.os_icon_index)),
            HostFormField::Password => (
                "Password",
                if editing {
                    text_input::with_cursor(&form.password, form.cursor)
                } else {
                    password_display(&form.password, form.has_password)
                },
            ),
            HostFormField::Username => (
                "Username",
                if editing {
                    text_input::with_cursor(&form.username, form.cursor)
                } else {
                    display_text(&form.username)
                },
            ),
        };
        let suffix = if read_only { " (read-only)" } else { "" };
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
        lines.push(Line::from(vec![
            Span::styled(prefix, label_style),
            Span::styled(format!("{label}{suffix}: "), label_style),
            Span::styled(display, value_style),
        ]));
    }

    let hint = Style::default().add_modifier(Modifier::DIM);
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Tab/↓: next field    Enter: open picker (Group/Identity)",
        hint,
    )));
    lines.push(Line::from(Span::styled(
        format!("{save_hint}: save    Esc: cancel"),
        hint,
    )));

    Paragraph::new(lines)
        .wrap(Wrap { trim: true })
        .block(Block::default().borders(Borders::ALL).title(title))
}

fn display_text(value: &str) -> String {
    if value.is_empty() {
        "(empty)".to_string()
    } else {
        value.to_string()
    }
}

fn group_label(index: usize, groups: &[HostGroup]) -> String {
    if index == 0 {
        "(none)".to_string()
    } else {
        groups
            .get(index - 1)
            .map(|g| g.name.clone())
            .unwrap_or_else(|| "(none)".to_string())
    }
}

fn os_icon_label(index: usize) -> String {
    OS_ICON_OPTIONS
        .get(index)
        .map(|s| (*s).to_string())
        .unwrap_or_else(|| "(none)".to_string())
}

fn password_display(password: &str, has_password: bool) -> String {
    if !password.is_empty() {
        "\u{25CF}".repeat(password.chars().count())
    } else if has_password {
        "(set)".to_string()
    } else {
        "(empty)".to_string()
    }
}

fn identity_label(index: usize, identities: &[Identity]) -> String {
    identities
        .get(index)
        .map(|i| i.name.clone())
        .unwrap_or_else(|| "(none)".to_string())
}
