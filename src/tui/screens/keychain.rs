//! The identity form popup (`AppMode::IdentityForm`).
//!
//! The identity *list* used to live here too, as `render_keychain`,
//! `render_identity_list` and `render_notice`. The identities tab replaced all
//! three with the card grid in `screens/keys.rs` and nothing called them any
//! more, so they were removed along with the four `components.keychain.*` roles
//! that only they could reach. This form is the surface that survived, and it
//! is styled from `components.form.*`, `components.focus.indicator` and
//! `components.popup.*` — not from a keychain family of its own.

use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::{IdentityFormEdit, IdentityFormField};
use crate::text_input;
use crate::theme::catalog::StyleRole;
use crate::theme::model::ResolvedTheme;

pub fn render_identity_form(
    form: &IdentityFormEdit,
    save_hint: &str,
    secret_hints: &str,
    theme: &ResolvedTheme,
    border: Style,
) -> Paragraph<'static> {
    // Same contract as the host form: the marker in front of the current field
    // is the focus indicator and is themed apart from the label beside it.
    let focus = theme.style(StyleRole::FocusIndicator);
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
                if editing && form.password_revealed {
                    text_input::with_cursor(&form.password, form.cursor)
                } else if editing {
                    text_input::with_cursor(
                        &"\u{25CF}".repeat(form.password.chars().count()),
                        form.cursor,
                    )
                } else if form.password_revealed {
                    form.password.clone()
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
        // The secret field is a key passphrase when a key is set, otherwise a
        // shared login password reused across hosts.
        let has_key = !form.private_key.is_empty() || form.pasted_key.is_some();
        let label = if field == IdentityFormField::Password && !has_key {
            "Password"
        } else {
            field.label()
        };
        lines.push(ratatui::text::Line::from(vec![
            ratatui::text::Span::styled(prefix, prefix_style),
            ratatui::text::Span::styled(format!("{label}: "), label_style),
            ratatui::text::Span::styled(display, value_style),
        ]));
    }
    lines.push(ratatui::text::Line::from(""));
    let base = format!("type to edit │ paste a key or its path into Private key │ Tab/↓: next │ {save_hint}: save │ Esc: cancel");
    // On the passphrase field the secret binds come first: the value is masked,
    // so that is the moment the user needs to know how to see or copy it.
    let hint = if form.field == IdentityFormField::Password && !secret_hints.is_empty() {
        format!("{secret_hints} │ {base}")
    } else {
        base
    };
    lines.push(ratatui::text::Line::from(ratatui::text::Span::styled(
        hint,
        theme.style(StyleRole::FormHelp),
    )));
    Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border)
            .title(ratatui::text::Span::styled(
                "Identity",
                theme.style(StyleRole::PopupTitle),
            )),
    )
}
