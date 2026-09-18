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

/// `inner_width` is the popup's inner width (borders excluded): the two path
/// rows are cut to it with an ellipsis (see below), so the caller passes the
/// popup width minus its borders.
pub fn render_identity_form(
    form: &IdentityFormEdit,
    save_hint: &str,
    secret_hints: &str,
    cert: Option<&crate::ssh::cert::CertStatus>,
    theme: &ResolvedTheme,
    border: Style,
    inner_width: u16,
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
                let raw = if editing {
                    text_input::with_cursor(value, form.cursor)
                } else if value.is_empty() {
                    "(empty)".to_string()
                } else {
                    value.clone()
                };
                match field {
                    // A long absolute path used to paint past the popup border
                    // with its underline trailing off the form. Both path rows
                    // share this display path and overflowed the same way, so
                    // both are cut to what fits after the marker and label,
                    // with the shared `…` helper (as in the SFTP browser and
                    // the tunnel footer). The stored value is untouched — only
                    // its display is shortened.
                    IdentityFormField::PrivateKey | IdentityFormField::Certificate => {
                        let used = prefix.chars().count()
                            + field.label().chars().count()
                            + ": ".chars().count();
                        let budget = (inner_width as usize).saturating_sub(used);
                        if budget == 0 {
                            String::new()
                        } else {
                            crate::tui::text::ellipsize(&raw, budget)
                        }
                    }
                    _ => raw,
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
        // Certificate detail directly under its row: key id, principals and
        // validity from `ssh-keygen -L`, or the reason nothing could be shown.
        // Keyed to the typed path via the shared snapshot map, so a renamed
        // path never shows the previous file's detail.
        if field == IdentityFormField::Certificate {
            if let Some(status) = cert {
                // Same bound as the path rows above: many/long principals
                // make this line arbitrarily long, re-creating the overflow
                // one row down. Leading indent plus the shared `…` helper.
                let budget = (inner_width as usize).saturating_sub(4);
                let detail = if budget == 0 {
                    String::new()
                } else {
                    format!(
                        "    {}",
                        crate::tui::text::ellipsize(&status.detail_line(), budget)
                    )
                };
                lines.push(ratatui::text::Line::from(vec![
                    ratatui::text::Span::styled(detail, theme.style(StyleRole::FormHelp)),
                ]));
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    fn form_with_cert() -> IdentityFormEdit {
        IdentityFormEdit {
            id: None,
            name: "deploy".into(),
            username: "alice".into(),
            private_key: "/home/u/.ssh/id_ed25519".into(),
            certificate: "/home/u/.ssh/id-cert.pub".into(),
            password: String::new(),
            password_original: String::new(),
            has_password: false,
            password_revealed: false,
            pasted_key: None,
            field: IdentityFormField::Certificate,
            cursor: 0,
            editing: true,
            edit_snapshot: String::new(),
            dirty: false,
        }
    }

    fn render_text(form: &IdentityFormEdit, cert: Option<&crate::ssh::cert::CertStatus>) -> String {
        let theme = crate::test_support::resolved_default();
        let widget = render_identity_form(form, "F2", "", cert, &theme, Style::default(), 98);
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 12));
        ratatui::widgets::Widget::render(widget, buf.area, &mut buf);
        let mut text = String::new();
        for y in 0..12 {
            for x in 0..100 {
                text.push_str(buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "));
            }
        }
        text
    }

    #[test]
    fn form_shows_cert_detail_line() {
        // Oracle: a hand-built snapshot — the form must surface the key id,
        // principals and window it is given, verbatim.
        let status = crate::ssh::cert::CertStatus {
            badge: crate::ssh::cert::CertBadge::Valid,
            key_id: "test-key-id".into(),
            principals: vec!["alice".into(), "root".into()],
            valid_from: Some("2024-01-01T00:00:00".into()),
            valid_to: Some("2030-01-01T00:00:00".into()),
            always_valid: false,
        };
        let text = render_text(&form_with_cert(), Some(&status));
        assert!(text.contains("test-key-id"), "form text: {text:?}");
        assert!(text.contains("alice, root"), "form text: {text:?}");
        assert!(text.contains("2030-01-01"), "form text: {text:?}");
    }

    #[test]
    fn form_without_cert_status_shows_no_detail_line() {
        let text = render_text(&form_with_cert(), None);
        assert!(!text.contains("principals:"), "form text: {text:?}");
    }
    fn row_text(buf: &Buffer, y: u16, w: u16) -> String {
        (0..w)
            .map(|x| buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "))
            .collect()
    }

    #[test]
    fn long_path_rows_fit_the_form_width() {
        // Oracle: a 300-wide buffer, so ratatui's own LineTruncator never
        // clips — the width each row paints at is the widget's logical
        // width, measured back out of real cells, never re-derived from the
        // truncation math. Both path rows must fit the popup inner width
        // they were given, cut with an ellipsis: a user screenshot showed
        // `/home/petruha/sshub-dev/tmp/manual/smoke-certs/id-cert.pub`
        // escaping the field with its editing underline trailing off the
        // form. The neighbouring private-key row shares the same display
        // path and overflowed the same way, so both are asserted here, in
        // both the idle and the editing states.
        const INNER_W: u16 = 68; // a 70-wide popup minus its borders
        for edited in [
            IdentityFormField::PrivateKey,
            IdentityFormField::Certificate,
        ] {
            for editing in [false, true] {
                let mut form = form_with_cert();
                form.private_key =
                    "/home/petruha/sshub-dev/tmp/manual/smoke-certs/id_ed25519".into();
                form.certificate =
                    "/home/petruha/sshub-dev/tmp/manual/smoke-certs/id-cert.pub".into();
                form.field = edited;
                form.editing = editing;
                form.cursor = match edited {
                    IdentityFormField::PrivateKey => form.private_key.chars().count(),
                    IdentityFormField::Certificate => form.certificate.chars().count(),
                    _ => 0,
                };
                let theme = crate::test_support::resolved_default();
                let widget =
                    render_identity_form(&form, "F2", "", None, &theme, Style::default(), INNER_W);
                let mut buf = Buffer::empty(Rect::new(0, 0, 300, 12));
                ratatui::widgets::Widget::render(widget, buf.area, &mut buf);
                for (label, full) in [
                    ("Private key path:", form.private_key.clone()),
                    ("Certificate path:", form.certificate.clone()),
                ] {
                    let row = (0..12)
                        .map(|y| row_text(&buf, y, 300))
                        .find(|r| r.contains(label))
                        .unwrap_or_else(|| panic!("{label} row missing (editing={editing})"));
                    // Strip the popup borders, then trailing fill; what remains
                    // is the row itself.
                    let painted = row.trim_end();
                    let cells = painted.chars().count().saturating_sub(2);
                    let content: String = painted
                        .chars()
                        .skip(1)
                        .take(cells)
                        .collect::<String>()
                        .trim_end()
                        .to_string();
                    let width = content.chars().count();
                    assert!(
                        width <= INNER_W as usize,
                        "{label} paints {width} cells wide, form fits {INNER_W} (editing={editing}): {content:?}"
                    );
                    assert!(
                        content.contains('\u{2026}'),
                        "{label} must be cut with an ellipsis, not run whole (editing={editing}): {content:?}"
                    );
                    assert!(
                        !content.contains(&full),
                        "{label} shows the full path uncut (editing={editing}): {content:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn long_cert_detail_line_fits_the_form_width() {
        // Oracle: same 300-wide-buffer arbiter as the path-row test — widths
        // come from real rendered cells, never from the truncation math. A
        // cert with many/long principals makes `detail_line()` arbitrarily
        // long; that row must still fit the popup inner width, cut with an
        // ellipsis like the path row directly above it.
        const INNER_W: u16 = 68; // a 70-wide popup minus its borders
        let status = crate::ssh::cert::CertStatus {
            badge: crate::ssh::cert::CertBadge::Valid,
            key_id: "deploy-key-with-a-long-name".into(),
            principals: vec![
                "alice".into(),
                "bob-the-builder".into(),
                "deploy-service-account".into(),
                "root".into(),
                "backup-operator".into(),
                "ci-runner-07".into(),
            ],
            valid_from: Some("2024-01-01T00:00:00".into()),
            valid_to: Some("2030-01-01T00:00:00".into()),
            always_valid: false,
        };
        let mut form = form_with_cert();
        form.editing = false;
        let theme = crate::test_support::resolved_default();
        let widget = render_identity_form(
            &form,
            "F2",
            "",
            Some(&status),
            &theme,
            Style::default(),
            INNER_W,
        );
        let mut buf = Buffer::empty(Rect::new(0, 0, 300, 14));
        ratatui::widgets::Widget::render(widget, buf.area, &mut buf);
        let row = (0..14)
            .map(|y| row_text(&buf, y, 300))
            .find(|r| r.contains("principals:"))
            .expect("cert detail row missing");
        let painted = row.trim_end();
        let cells = painted.chars().count().saturating_sub(2);
        let content: String = painted
            .chars()
            .skip(1)
            .take(cells)
            .collect::<String>()
            .trim_end()
            .to_string();
        let width = content.chars().count();
        assert!(
            width <= INNER_W as usize,
            "detail paints {width} cells wide, form fits {INNER_W}: {content:?}"
        );
        assert!(
            content.contains('\u{2026}'),
            "detail must be cut with an ellipsis, not run whole: {content:?}"
        );
    }
}
