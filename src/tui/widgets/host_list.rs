use ratatui::prelude::{Modifier, Style};
use ratatui::widgets::{List, ListItem};

use crate::app::App;

fn format_host_row(app: &App, list_idx: usize, host_idx: usize) -> ListItem<'static> {
    let entry = &app.hosts[host_idx];
    let selected = list_idx == app.selected;
    let marker = if selected { "▶ " } else { "  " };
    let star = if entry.favorite() { "★ " } else { "  " };
    let tags: String = entry
        .tags()
        .iter()
        .map(|t| format!("[{t}]"))
        .collect::<Vec<_>>()
        .join("");
    let label = format!("{marker}{star}{}{tags}", entry.display_name());

    let style = if selected {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    };
    ListItem::new(label).style(style)
}

pub fn render_host_list(app: &App) -> List<'static> {
    let items: Vec<ListItem> = app
        .filtered_indices
        .iter()
        .enumerate()
        .map(|(list_idx, &host_idx)| format_host_row(app, list_idx, host_idx))
        .collect();
    List::new(items)
}
