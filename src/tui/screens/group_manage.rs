use ratatui::prelude::Style;
use ratatui::style::Color;
use ratatui::widgets::{Block, Borders, List, ListItem};

use crate::app::App;

pub fn render_group_list(app: &App) -> List<'static> {
    if app.groups.is_empty() {
        return List::new(vec![ListItem::new("  (no groups — press 'a' to add)")]);
    }

    let host_counts: Vec<usize> = app
        .groups
        .iter()
        .map(|g| {
            app.hosts
                .iter()
                .filter(|h| h.group_id() == Some(g.id))
                .count()
        })
        .collect();

    let items: Vec<ListItem> = app
        .groups
        .iter()
        .enumerate()
        .map(|(idx, group)| {
            let selected = idx == app.group_manage_selected;
            let marker = if selected { "▶ " } else { "  " };
            let count = host_counts[idx];
            // Indent by nesting depth so subgroups read as a hierarchy.
            let depth = group_depth(app, group.id);
            let indent = "  ".repeat(depth);
            let label = format!(
                "{marker}{indent}{} ({count} host{})",
                group.name,
                if count == 1 { "" } else { "s" }
            );
            let style = if selected {
                Style::default().bg(Color::DarkGray).fg(Color::White)
            } else {
                Style::default()
            };
            ListItem::new(label).style(style)
        })
        .collect();
    List::new(items).block(Block::default().borders(Borders::ALL).title("Groups"))
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
