use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use ratatui::style::Style;

use crate::app::App;
use crate::theme::catalog::{ColorRole, StyleRole};

/// Centered, themed popup listing the command snippets. Rendered as an overlay
/// over the dashboard so it matches the rest of the app's chrome.
pub fn render_snippet_manage_popup(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let width = crate::tui::fit_popup(area.width * 66 / 100, 46, area.width.saturating_sub(2));
    let rows = app.snippets.len().max(1) as u16;
    // list rows + borders + a hint row (+ an optional notice row).
    let notice_rows = u16::from(app.snippet_notice.is_some());
    let height = crate::tui::fit_popup(rows + 4 + notice_rows, 6, area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let popup = Rect::new(x, y, width, height);

    let popup = crate::tui::popup_open_rect(popup, app);

    let theme = app.theme();
    let legend = theme.style(StyleRole::PopupLegend);

    crate::tui::open_popup(frame, popup, theme);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(crate::tui::popup_border_style(theme, popup))
        .title(Span::styled(
            " Command snippets ",
            theme.style(StyleRole::PopupTitle),
        ));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    crate::tui::paint_popup_border(frame, popup, theme);

    if inner.height == 0 {
        return;
    }

    // Reserve the last inner row for the action hint, and the row above it for a
    // transient notice when present.
    let notice_h = if app.snippet_notice.is_some() { 1 } else { 0 };
    let list_h = inner.height.saturating_sub(1 + notice_h);
    let list_area = Rect::new(inner.x, inner.y, inner.width, list_h);

    if app.snippets.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "No snippets yet — press 'a' to add one.",
                legend,
            )),
            list_area,
        );
    } else if app.snippet_manage_view().is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("No snippets match the filter.", legend)),
            list_area,
        );
    } else {
        let view = app.snippet_manage_view();
        let items: Vec<ListItem> = view
            .iter()
            .map(|&i| {
                let snippet = &app.snippets[i];
                let tags = if snippet.tags.is_empty() {
                    String::new()
                } else {
                    format!("  #{}", snippet.tags.join(" #"))
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!(" {}", snippet.name),
                        theme.style(StyleRole::TableRow),
                    ),
                    Span::styled(format!("  {}", snippet.command), legend),
                    Span::styled(tags, theme.style(StyleRole::PopupHint)),
                ]))
            })
            .collect();

        let mut state = ListState::default();
        state.select(view.iter().position(|&i| i == app.snippet_manage_selected));
        let list = List::new(items).highlight_style(theme.style(StyleRole::TableRowSelected));
        frame.render_stateful_widget(list, list_area, &mut state);
    }

    if let Some(notice) = &app.snippet_notice {
        let notice_area = Rect::new(inner.x, inner.y + list_h, inner.width, 1);
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(" {notice}"),
                Style::default().fg(theme.color(ColorRole::StatusInfo)),
            )),
            notice_area,
        );
    }

    let hint_area = Rect::new(inner.x, inner.y + inner.height - 1, inner.width, 1);
    let query = &app.snippet_manage_query;
    let hint = if app.snippet_manage_filtering {
        format!(" / {query}\u{2581} · Enter keep · Esc clear")
    } else if !query.is_empty() {
        format!(" filter: {query} · / edit · a add · e edit · d delete · Esc clear")
    } else {
        " / filter · a add · e edit · d delete · Enter edit · Esc back".to_string()
    };
    frame.render_widget(
        Paragraph::new(Span::styled(hint, theme.style(StyleRole::PopupHint))),
        hint_area,
    );
}
