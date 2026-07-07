use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::tui::theme;

/// Centered, themed popup listing the groups as a tree. Rendered as an overlay
/// over the dashboard so it matches the rest of the app's chrome.
pub fn render_group_manage_popup(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let width = (area.width * 60 / 100).clamp(40, area.width.saturating_sub(2));
    // Rows for groups (or the empty hint) + borders + a hint row.
    let rows = app.groups.len().max(1) as u16;
    let height = (rows + 4).clamp(6, area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let popup = Rect::new(x, y, width, height);

    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border())
        .title(Span::styled(" Groups ", theme::heading()));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    if inner.height == 0 {
        return;
    }

    // Reserve the last inner row for the action hint.
    let list_area = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(1),
    );
    let hint_area = Rect::new(inner.x, inner.y + inner.height - 1, inner.width, 1);

    if app.groups.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "No groups yet — press 'a' to add one.",
                theme::mute(),
            )),
            list_area,
        );
    } else {
        let items: Vec<ListItem> = app
            .groups
            .iter()
            .map(|group| {
                let depth = group_depth(app, group.id);
                let count = app
                    .hosts
                    .iter()
                    .filter(|h| h.group_id() == Some(group.id))
                    .count();
                let indent = "  ".repeat(depth);
                let arrow = if depth > 0 { "\u{2514} " } else { "" }; // └
                ListItem::new(Line::from(vec![
                    Span::styled(format!(" {indent}{arrow}"), theme::mute()),
                    Span::styled(group.name.clone(), theme::text()),
                    Span::styled(format!("  ({count})"), theme::mute()),
                ]))
            })
            .collect();

        let mut state = ListState::default();
        state.select(Some(app.group_manage_selected.min(app.groups.len() - 1)));
        let list = List::new(items).highlight_style(theme::selected());
        frame.render_stateful_widget(list, list_area, &mut state);
    }

    frame.render_widget(
        Paragraph::new(Span::styled(
            " a add · e edit · d delete · Esc back",
            theme::dim(),
        )),
        hint_area,
    );
}

/// Nesting depth of a group = number of ancestors (0 for top-level). Bounded by
/// the group count so a stray parent cycle can't loop forever.
fn group_depth(app: &App, mut id: i64) -> usize {
    let mut depth = 0;
    let max = app.groups.len();
    while let Some(parent) = app
        .groups
        .iter()
        .find(|g| g.id == id)
        .and_then(|g| g.parent_id)
    {
        depth += 1;
        id = parent;
        if depth > max {
            break;
        }
    }
    depth
}
