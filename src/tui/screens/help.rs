use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::tui::theme;

fn entry(key: &'static str, desc: &'static str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{:<16}", key), theme::bright()),
        Span::styled(desc, theme::text()),
    ])
}

fn section(title: &'static str) -> Line<'static> {
    Line::from(Span::styled(title, theme::heading()))
}

pub fn render_help() -> Paragraph<'static> {
    let lines = vec![
        section("navigate"),
        entry("\u{2191}\u{2193} / j k", "Move up / down"),
        entry("1..4 / h i", "Switch tab (hosts, tunnels, keys, audit)"),
        entry("Tab", "Toggle detail panel (hosts)"),
        entry("Enter", "Connect / start tunnel"),
        entry("Esc", "Back / close overlay"),
        Line::from(""),
        section("hosts (tab 1)"),
        entry("a", "Add new host"),
        entry("e", "Edit selected host"),
        entry("d", "Delete selected host"),
        entry("Shift+D", "Duplicate selected host"),
        entry("f", "Toggle favorite"),
        entry("s", "Cycle sort mode"),
        entry("Ctrl+\u{2191}\u{2193}", "Move host up / down (manual sort)"),
        entry("c", "Clear SSH log"),
        entry("y", "Copy SSH log for selected host (clipboard)"),
        Line::from(""),
        section("tunnels (tab 2)"),
        entry("a", "Add new tunnel"),
        entry("e", "Edit selected tunnel"),
        entry("d", "Delete tunnel"),
        entry("Enter", "Start / stop tunnel"),
        entry("x", "Kill tunnel process"),
        Line::from(""),
        section("keys (tab 3)"),
        entry("a", "Add new identity"),
        entry("e", "Edit identity"),
        entry("d", "Delete identity"),
        entry("p", "Add key to agent"),
        entry("r", "Remove key from agent"),
        Line::from(""),
        section("audit (tab 4)"),
        entry("f", "Cycle filter (all/ok/fail)"),
        entry("r", "Cycle range (all/24h/week/month)"),
        Line::from(""),
        section("search"),
        entry("/", "Open fuzzy palette"),
        entry("(typing)", "Any unmatched key opens palette"),
        entry("#", "Filter by tag"),
        Line::from(""),
        section("groups"),
        entry("Space / ←→", "Collapse / expand selected group"),
        entry("Enter", "On a group header: collapse/expand; on a host: connect"),
        entry("Shift+Z", "Collapse / expand all groups"),
        entry("Enter", "In host form on Group: open dropdown (+ create new)"),
        entry("Shift+G", "Manage groups"),
        entry("Ctrl+G", "Rename selected group"),
        entry("Ctrl+Shift+G", "Delete selected group"),
        Line::from(""),
        section("import / export"),
        entry("Shift+I", "Import from ssh config"),
        entry("Shift+E", "Export hosts to ssh config"),
        entry("Shift+T", "Import from Termius export folder"),
        Line::from(""),
        section("termius import (Shift+T)"),
        entry("", "Point the prompt at the export folder holding"),
        entry("", "L00t.csv (+ ssh_keys/). Imports hosts, logins,"),
        entry("", "passwords & keys; existing hosts are skipped."),
        Line::from(""),
        section("tools"),
        entry("", ""),
        entry("[session]", ""),
        entry("Ctrl+T", "Duplicate session tab"),
        entry("Ctrl+W", "Close session tab"),
        entry("Ctrl+D", "Detach back to dashboard"),
        entry("PgUp/PgDn", "Scroll session history"),
        entry("", ""),
        entry("? / Shift+H", "Toggle this help screen"),
        entry("F2 / Ctrl+S", "Save form (rebindable)"),
        entry("Ctrl+K", "Edit keybindings (save/quit/help/search/add/delete/…)"),
        entry("q", "Quit"),
        Line::from(""),
        Line::from(Span::styled("? / Esc / Enter to close", theme::dim())),
    ];

    Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(theme::border())
            .title(Span::styled(" Help ", theme::heading())),
    )
}
