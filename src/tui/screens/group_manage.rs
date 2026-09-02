use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::theme::catalog::{ColorRole, StyleRole};

/// Centered, themed popup listing the groups as a tree. Rendered as an overlay
/// over the dashboard so it matches the rest of the app's chrome.
pub fn render_group_manage_popup(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let width = crate::tui::fit_popup(area.width * 60 / 100, 40, area.width.saturating_sub(2));
    // Rows for groups (or the empty hint) + borders + a hint row, plus a
    // transient notice row when one is pending.
    let rows = app.groups.len().max(1) as u16;
    let notice_rows = u16::from(app.group_notice.is_some());
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
        .title(Span::styled(" Groups ", theme.style(StyleRole::PopupTitle)));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    crate::tui::paint_popup_border(frame, popup, theme);

    if inner.height == 0 {
        return;
    }

    // Reserve the last inner row for the action hint, and the row above it for a
    // transient notice when present.
    let notice_h = u16::from(app.group_notice.is_some());
    let list_h = inner.height.saturating_sub(1 + notice_h);
    let list_area = Rect::new(inner.x, inner.y, inner.width, list_h);
    let hint_area = Rect::new(inner.x, inner.y + inner.height - 1, inner.width, 1);

    if app.groups.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "No groups yet — press 'a' to add one.",
                legend,
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
                    Span::styled(format!(" {indent}{arrow}"), legend),
                    Span::styled(group.name.clone(), theme.style(StyleRole::TableRow)),
                    Span::styled(format!("  ({count})"), legend),
                ]))
            })
            .collect();

        let mut state = ListState::default();
        state.select(Some(app.group_manage_selected.min(app.groups.len() - 1)));
        let list = List::new(items).highlight_style(theme.style(StyleRole::TableRowSelected));
        frame.render_stateful_widget(list, list_area, &mut state);
    }

    if let Some(notice) = &app.group_notice {
        let notice_area = Rect::new(inner.x, inner.y + list_h, inner.width, 1);
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(" {notice}"),
                Style::default().fg(theme.color(ColorRole::StatusInfo)),
            )),
            notice_area,
        );
    }

    frame.render_widget(
        Paragraph::new(Span::styled(
            " a add · e edit · d delete · Esc back",
            theme.style(StyleRole::PopupHint),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{frame_at, resolved_default, themed_app};
    use ratatui::layout::Rect;

    fn frame_contains(buf: &ratatui::buffer::Buffer, area: Rect, needle: &str) -> bool {
        (0..area.height).any(|y| {
            let row: String = (0..area.width)
                .map(|x| buf.cell((x, y)).unwrap().symbol())
                .collect();
            row.contains(needle)
        })
    }

    /// The delete confirmation is only useful if it reaches the screen: the
    /// manager popup must draw `group_notice`, not just hold it in a field.
    #[test]
    fn group_notice_is_painted_in_the_manager_popup() {
        let mut app = themed_app(resolved_default());
        app.group_notice = Some("Group 'keep-me' deleted".to_string());
        let area = Rect::new(0, 0, 100, 30);
        let buf = frame_at(area, |f| render_group_manage_popup(f, &app));
        assert!(
            frame_contains(&buf, area, "Group 'keep-me' deleted"),
            "the group_notice row was not rendered"
        );
    }
}
