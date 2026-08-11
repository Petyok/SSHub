use ratatui::prelude::Style;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::{KeygenFormEdit, KeygenFormField};
use crate::text_input;
use crate::theme::catalog::StyleRole;
use crate::theme::model::ResolvedTheme;

/// The key generator popup.
///
/// Reaching a field *is* editing it here — the form has no separate "selected
/// but idle" state — so each row is either focused or not, and the two focused
/// roles cover the marker, the label and the value together.
pub fn render_keygen_form(
    form: &KeygenFormEdit,
    save_hint: &str,
    theme: &ResolvedTheme,
    border: Style,
) -> Paragraph<'static> {
    let mut lines = Vec::with_capacity(KeygenFormField::ALL.len() + 2);
    for field in KeygenFormField::ALL {
        let editing = form.field == field;
        let prefix = if editing { "▸ " } else { "  " };
        let display = match field {
            KeygenFormField::KeyType => {
                let val = form.key_type.label();
                if editing {
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
        let label_style = theme.style(if editing {
            StyleRole::KeygenLabelFocused
        } else {
            StyleRole::KeygenLabel
        });
        let value_style = theme.style(if editing {
            StyleRole::KeygenValueFocused
        } else {
            StyleRole::KeygenValue
        });

        lines.push(ratatui::text::Line::from(vec![
            ratatui::text::Span::styled(format!("{prefix}{}: ", field.label()), label_style),
            ratatui::text::Span::styled(display, value_style),
        ]));
    }
    lines.push(ratatui::text::Line::from(""));
    lines.push(ratatui::text::Line::from(ratatui::text::Span::styled(
        format!("type to edit │ Tab/↓: next │ {save_hint}: save │ Esc: cancel"),
        theme.style(StyleRole::KeygenHelp),
    )));
    Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border)
            .title(ratatui::text::Span::styled(
                "Generate SSH Key",
                theme.style(StyleRole::KeygenTitle),
            )),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::KeygenFormEdit;
    use crate::test_support::{
        fg, fg_at_text, frame_at, marker, resolved_default, role_marker_theme, RoleMarker,
    };
    use ratatui::layout::Rect;

    const TITLE: u32 = 0xc1_0001;
    const LABEL: u32 = 0xc1_0002;
    const LABEL_FOCUSED: u32 = 0xc1_0003;
    const VALUE: u32 = 0xc1_0004;
    const VALUE_FOCUSED: u32 = 0xc1_0005;
    const HELP: u32 = 0xc1_0006;

    const MARKERS: &[RoleMarker] = &[
        fg("components.keygen.title", TITLE),
        fg("components.keygen.label", LABEL),
        fg("components.keygen.label_focused", LABEL_FOCUSED),
        fg("components.keygen.value", VALUE),
        fg("components.keygen.value_focused", VALUE_FOCUSED),
        fg("components.keygen.help", HELP),
    ];

    /// A form sitting on its first field, so one row is focused and the rest
    /// are not.
    fn form() -> KeygenFormEdit {
        KeygenFormEdit {
            key_type: crate::app::KeygenType::default(),
            passphrase: String::new(),
            comment: "workstation".into(),
            target_path: "/home/u/.ssh/id_ed25519".into(),
            field: crate::app::KeygenFormField::KeyType,
            cursor: 0,
            dirty: false,
        }
    }

    fn render(theme: &crate::theme::model::ResolvedTheme) -> ratatui::buffer::Buffer {
        let area = Rect::new(0, 0, 60, 12);
        let border = ratatui::style::Style::default();
        frame_at(area, |f| {
            f.render_widget(render_keygen_form(&form(), "Ctrl+S", theme, border), area)
        })
    }

    /// Every visible surface of the keygen form reads its own role.
    ///
    /// Before the migration this screen was the last one drawing raw
    /// `Color::Yellow` / `White` / `DarkGray`, so a theme could not touch it at
    /// all. Unique markers are the only way to prove each surface reaches for
    /// its *own* role rather than a neighbouring one.
    #[test]
    fn the_keygen_form_wears_its_six_roles() {
        let buf = render(&role_marker_theme("keygen", MARKERS));

        assert_eq!(fg_at_text(&buf, "Generate SSH Key"), marker(TITLE), "title");
        // The marker and the focused row's label share one role.
        assert_eq!(
            fg_at_text(&buf, "\u{25b8}"),
            marker(LABEL_FOCUSED),
            "marker"
        );
        assert_eq!(
            fg_at_text(&buf, "Key type"),
            marker(LABEL_FOCUSED),
            "focused label"
        );
        assert_eq!(
            fg_at_text(&buf, "[ "),
            marker(VALUE_FOCUSED),
            "focused value"
        );
        assert_eq!(fg_at_text(&buf, "Comment"), marker(LABEL), "an idle label");
        assert_eq!(
            fg_at_text(&buf, "workstation"),
            marker(VALUE),
            "an idle value"
        );
        assert_eq!(
            fg_at_text(&buf, "type to edit"),
            marker(HELP),
            "the help line"
        );
    }

    /// `default` keeps the weights the hard-coded styles carried: the focused
    /// label stayed bold, and the focused value bold *and* underlined.
    #[test]
    fn the_default_theme_keeps_the_focused_row_emphasis() {
        use ratatui::style::Modifier;
        let buf = render(&resolved_default());

        let label = crate::test_support::find_text(&buf, "Key type");
        assert!(
            buf.cell(label).unwrap().modifier.contains(Modifier::BOLD),
            "the focused label lost its weight"
        );
        let value = crate::test_support::find_text(&buf, "[ ");
        let modifier = buf.cell(value).unwrap().modifier;
        assert!(modifier.contains(Modifier::BOLD), "focused value: bold");
        assert!(
            modifier.contains(Modifier::UNDERLINED),
            "focused value: underline"
        );
    }
}
