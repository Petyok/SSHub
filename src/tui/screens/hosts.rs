use ratatui::prelude::{Modifier, Style};
use ratatui::style::Color;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem};

use crate::app::{App, UNGROUPED_LABEL};

fn format_host_row(app: &App, list_idx: usize, host_idx: usize) -> ListItem<'static> {
    let entry = &app.hosts[host_idx];
    let selected = list_idx == app.selected;
    let marker = if selected { "▶ " } else { "  " };
    let star_str = if entry.favorite() { "★ " } else { "" };
    let tags: String = entry
        .tags()
        .iter()
        .map(|t| format!("[{t}]"))
        .collect::<Vec<_>>()
        .join("");
    let pad = if entry.favorite() { "" } else { "  " };

    let mut spans = vec![Span::raw(format!("  {marker}"))];
    if entry.favorite() {
        spans.push(Span::styled(star_str, Style::default().fg(Color::Yellow)));
    } else {
        spans.push(Span::raw(pad));
    }
    spans.push(Span::raw(entry.display_name().to_string()));
    if !tags.is_empty() {
        spans.push(Span::styled(tags, Style::default().fg(Color::DarkGray)));
    }

    let base_style = if selected {
        Style::default().bg(Color::DarkGray).fg(Color::White)
    } else {
        Style::default()
    };
    ListItem::new(Line::from(spans)).style(base_style)
}

fn format_group_header(label: &str) -> ListItem<'static> {
    let display = if label == UNGROUPED_LABEL {
        "Ungrouped".to_string()
    } else {
        format!("▼ {label}")
    };
    ListItem::new(display).style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )
}

pub fn render_group_tree(app: &App) -> List<'static> {
    let mut items: Vec<ListItem<'static>> = Vec::new();
    let mut list_idx = 0usize;

    for section in &app.group_sections {
        items.push(format_group_header(&section.label));
        for &host_idx in &section.host_indices {
            items.push(format_host_row(app, list_idx, host_idx));
            list_idx += 1;
        }
    }

    if items.is_empty() {
        items.push(ListItem::new("  (no hosts)"));
    }

    List::new(items)
}
