use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::{HostFormEdit, HostFormField, OS_ICON_OPTIONS};
use crate::store::{HostGroup, Identity};
use crate::text_input;
use crate::theme::catalog::StyleRole;
use crate::theme::model::ResolvedTheme;

pub fn render_host_form(
    form: &HostFormEdit,
    groups: &[HostGroup],
    identities: &[Identity],
    save_hint: &str,
    secret_hints: &str,
    theme: &ResolvedTheme,
    border: Style,
) -> Paragraph<'static> {
    let title = if form.metadata_only {
        "Edit metadata (ssh_config)"
    } else if form.id.is_some() {
        "Edit host"
    } else {
        "New host"
    };

    let help = theme.style(StyleRole::FormHelp);
    // The `▸`/`>` glyph in front of the current field is the focus indicator.
    // This form marked focus with direct ANSI accents, which the spec allows to
    // be normalised, so there is no legacy cell to be faithful to and the
    // global role fits. The screens whose marker *did* carry a `theme.rs` style
    // — pickers, group form, keybind editor, tunnel reconnect — keep their own
    // family's marker role instead.
    let focus = theme.style(StyleRole::FocusIndicator);

    let mut lines = Vec::with_capacity(HostFormField::ALL.len() + 2);
    if form.metadata_only {
        lines.push(Line::from(Span::styled(
            "Connection fields are read-only (from ~/.ssh/config). Edit launcher metadata below.",
            help,
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
            HostFormField::Group => ("Group", group_summary(&form.group_ids, groups)),
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
            HostFormField::Transport => (
                "Transport",
                format!("{} (Space to toggle)", form.transport.label()),
            ),
            HostFormField::SessionLogging => (
                "Session log",
                format!(
                    "{} (Space or arrows to cycle)",
                    form.session_logging.label()
                ),
            ),
            HostFormField::OsIcon => ("OS icon", os_icon_label(form.os_icon_index)),
            HostFormField::Password => (
                "Password",
                // Masked while typing too, now that the field arrives prefilled
                // with the stored secret: walking onto it must not expose one.
                // The reveal bind is the way to see it — and it says so on the
                // row itself, because the footer that used to be its only
                // mention is the first thing a short terminal clips, which left
                // a wall of dots and no way to know a reveal exists at all.
                if editing && form.password_revealed {
                    text_input::with_cursor(&form.password, form.cursor)
                } else if editing {
                    let masked = text_input::with_cursor(
                        &"\u{25CF}".repeat(form.password.chars().count()),
                        form.cursor,
                    );
                    if secret_hints.is_empty() {
                        masked
                    } else {
                        format!("{masked}    {}", reveal_hint(secret_hints))
                    }
                } else {
                    password_display(&form.password, form.has_password, form.password_revealed)
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
            theme.style(StyleRole::FormLabelEditing)
        } else if active {
            theme.style(StyleRole::FormLabelFocused)
        } else {
            theme.style(StyleRole::FormLabel)
        };
        let value_style = if editing {
            theme.style(StyleRole::FormInputEditing)
        } else if active {
            theme.style(StyleRole::FormInputFocused)
        } else {
            theme.style(StyleRole::FormValue)
        };
        let prefix_style = if active { focus } else { label_style };
        lines.push(Line::from(vec![
            Span::styled(prefix, prefix_style),
            Span::styled(format!("{label}{suffix}: "), label_style),
            Span::styled(display, value_style),
        ]));
    }

    lines.push(Line::from(""));
    if form.field == HostFormField::Password && !secret_hints.is_empty() {
        // See the identity form: a masked value needs its binds said out loud.
        lines.push(Line::from(Span::styled(secret_hints.to_string(), help)));
    }
    lines.push(Line::from(Span::styled(
        "Tab/↓: next field    Enter: open picker (Group/Identity)    Ctrl+U/W: kill line/word",
        help,
    )));
    lines.push(Line::from(Span::styled(
        format!("{save_hint}: save    Esc: cancel"),
        help,
    )));

    Paragraph::new(lines).wrap(Wrap { trim: true }).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border)
            .title(Span::styled(title, theme.style(StyleRole::PopupTitle))),
    )
}

fn display_text(value: &str) -> String {
    if value.is_empty() {
        "(empty)".to_string()
    } else {
        value.to_string()
    }
}

/// Comma-separated names of the selected groups, in list order.
fn group_summary(selected: &std::collections::BTreeSet<i64>, groups: &[HostGroup]) -> String {
    let names: Vec<&str> = groups
        .iter()
        .filter(|g| selected.contains(&g.id))
        .map(|g| g.name.as_str())
        .collect();
    if names.is_empty() {
        "(none)".to_string()
    } else {
        names.join(", ")
    }
}

fn os_icon_label(index: usize) -> String {
    OS_ICON_OPTIONS
        .get(index)
        .map(|s| (*s).to_string())
        .unwrap_or_else(|| "(none)".to_string())
}

fn password_display(password: &str, has_password: bool, revealed: bool) -> String {
    if password.is_empty() {
        // `(set)` only survives for a secret the store would not hand back, e.g.
        // a locked keyring: otherwise the field is prefilled and this is honest.
        return if has_password {
            "(set)".to_string()
        } else {
            "(empty)".to_string()
        };
    }
    if revealed {
        password.to_string()
    } else {
        "\u{25CF}".repeat(password.chars().count())
    }
}

fn identity_label(index: usize, identities: &[Identity]) -> String {
    identities
        .get(index)
        .map(|i| i.name.clone())
        .unwrap_or_else(|| "(none)".to_string())
}

/// The reveal half of the secret hints — the part that fits on the row next to a
/// masked value. The full pair (show + copy) stays in the footer.
fn reveal_hint(secret_hints: &str) -> &str {
    secret_hints
        .split('\u{2502}')
        .next()
        .unwrap_or_default()
        .trim()
}
