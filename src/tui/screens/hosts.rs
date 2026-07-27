//! The flat group tree list.
//!
//! Note for the theme migration: nothing in the running app calls
//! [`render_group_tree`] any more — the dashboard host list lives in
//! `widgets/hosts_panel.rs`. The roles below are therefore wired for
//! correctness, but no rendered cell can prove them; retiring this module is
//! left to the pass that removes the legacy palette.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem};

use crate::app::{App, UNGROUPED_LABEL};
use crate::theme::catalog::{ColorRole, StyleRole};
use crate::theme::model::ResolvedTheme;

fn format_host_row(
    app: &App,
    list_idx: usize,
    host_idx: usize,
    theme: &ResolvedTheme,
) -> ListItem<'static> {
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
        spans.push(Span::styled(
            star_str,
            Style::default().fg(theme.color(ColorRole::StatusWarning)),
        ));
    } else {
        spans.push(Span::raw(pad));
    }
    spans.push(Span::raw(entry.display_name().to_string()));
    if !tags.is_empty() {
        spans.push(Span::styled(tags, theme.style(StyleRole::PopupHint)));
    }

    let base_style = if selected {
        theme.style(StyleRole::TableRowSelected)
    } else {
        theme.style(StyleRole::TableRow)
    };
    ListItem::new(Line::from(spans)).style(base_style)
}

fn format_group_header(label: &str, theme: &ResolvedTheme) -> ListItem<'static> {
    let display = if label == UNGROUPED_LABEL {
        "Ungrouped".to_string()
    } else {
        format!("▼ {label}")
    };
    ListItem::new(display).style(theme.style(StyleRole::TableHeader))
}

pub fn render_group_tree(app: &App, theme: &ResolvedTheme) -> List<'static> {
    let mut items: Vec<ListItem<'static>> = Vec::new();
    let mut list_idx = 0usize;

    for section in &app.group_sections {
        items.push(format_group_header(&section.label, theme));
        for &host_idx in &section.host_indices {
            items.push(format_host_row(app, list_idx, host_idx, theme));
            list_idx += 1;
        }
    }

    if items.is_empty() {
        items.push(ListItem::new("  (no hosts)").style(theme.style(StyleRole::TableRow)));
    }

    List::new(items)
}
