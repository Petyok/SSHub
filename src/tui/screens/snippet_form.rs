use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::{App, SnippetFormField};
use crate::theme::catalog::StyleRole;

/// Centered popup for adding or editing a single command snippet.
pub fn render_snippet_form(frame: &mut Frame, app: &App) {
    let Some(form) = app.snippet_form.as_ref() else {
        return;
    };

    let area = frame.area();
    let width = crate::tui::fit_popup(area.width * 66 / 100, 46, area.width.saturating_sub(2));
    // 1 pad + 4 field rows + 1 pad + 1 help = 7 inner rows, + 2 borders, plus a
    // validation-notice row when one is pending.
    let notice_row = u16::from(app.snippet_notice.is_some());
    let height = crate::tui::fit_popup(9 + notice_row, 8, area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let popup = Rect::new(x, y, width, height);
    let popup = crate::tui::popup_open_rect(popup, app);

    let theme = app.theme();
    crate::tui::open_popup(frame, popup, theme);

    let title = if form.id.is_some() {
        " Edit snippet "
    } else {
        " New snippet "
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(crate::tui::popup_border_style(theme, popup))
        .title(Span::styled(title, theme.style(StyleRole::PopupTitle)));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    crate::tui::paint_popup_border(frame, popup, theme);

    let help = theme.style(StyleRole::FormHelp);
    let marker = theme.style(StyleRole::GroupFormMarker);

    let field_row = |field: SnippetFormField, value: &str| {
        let focused = form.field == field;
        let mark = if focused { "\u{25b8} " } else { "  " };
        let label_style = if focused {
            theme.style(StyleRole::GroupFormLabelFocused)
        } else {
            theme.style(StyleRole::GroupFormLabel)
        };
        let value_style = if focused {
            theme.style(StyleRole::GroupFormValueFocused)
        } else {
            theme.style(StyleRole::GroupFormValue)
        };
        // Show a block cursor on the focused field so typing has an anchor.
        let shown = if value.is_empty() {
            if focused {
                "\u{2588}".to_string()
            } else {
                "(empty)".to_string()
            }
        } else if focused {
            format!("{value}\u{2588}")
        } else {
            value.to_string()
        };
        Line::from(vec![
            Span::styled(mark, if focused { marker } else { label_style }),
            Span::styled(format!("{}: ", field.label()), label_style),
            Span::styled(shown, value_style),
        ])
    };

    let mut lines = vec![
        Line::from(""),
        field_row(SnippetFormField::Name, &form.name),
        field_row(SnippetFormField::Command, &form.command),
        field_row(SnippetFormField::Description, &form.description),
        field_row(SnippetFormField::Tags, &form.tags),
        Line::from(""),
    ];
    if let Some(notice) = app.snippet_notice.as_ref() {
        lines.push(Line::from(Span::styled(
            format!(" {notice}"),
            theme.style(StyleRole::PopupWarning),
        )));
    }
    lines.push(Line::from(Span::styled(
        "\u{2191}\u{2193}/Tab move field  ·  Enter next / save on last  ·  Esc cancel",
        help,
    )));

    frame.render_widget(Paragraph::new(lines), inner);
}
