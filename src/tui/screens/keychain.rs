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
    // Same contract as the host form, and for the same reason: this form's
    // marker had no `theme.rs` cell of its own, only direct ANSI accents, so
    // the global focus role is what it takes.
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
                    // The reveal bind rides the row, not just the footer: on a
                    // short terminal the footer is what gets clipped, and a
                    // masked value with no visible way to unmask it reads as
                    // "this secret cannot be seen at all".
                    let masked = text_input::with_cursor(
                        &"\u{25CF}".repeat(form.password.chars().count()),
                        form.cursor,
                    );
                    if secret_hints.is_empty() {
                        masked
                    } else {
                        format!("{masked}    {}", reveal_hint(secret_hints))
                    }
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
    // Save/cancel stay at the end: the middle of this row is what truncation
    // eats first on a narrow terminal (see the 0.9.x footer fix).
    let base = format!("type to edit │ paste a key or its path into Private key │ Tab/↓: next │ {save_hint}: save │ Esc: cancel │ Ctrl+U/W: kill line/word");
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

/// The reveal half of the secret hints — the part that fits on the row next to a
/// masked value. The full pair (show + copy) stays in the footer.
fn reveal_hint(secret_hints: &str) -> &str {
    secret_hints
        .split('\u{2502}')
        .next()
        .unwrap_or_default()
        .trim()
}
