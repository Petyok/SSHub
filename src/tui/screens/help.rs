use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::tui::theme;

/// Fixed footer hint shown below the scrollable help body.
pub const HELP_FOOTER: &str = "\u{2191}\u{2193}/PgUp/PgDn scroll  ·  ? / Esc / Enter to close";

fn entry(key: &'static str, desc: &'static str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{:<16}", key), theme::bright()),
        Span::styled(desc, theme::text()),
    ])
}

fn section(title: &'static str) -> Line<'static> {
    Line::from(Span::styled(title, theme::heading()))
}

/// Total number of lines in the help content (for scroll clamping).
pub fn help_line_count() -> u16 {
    help_lines().len() as u16
}

fn help_lines() -> Vec<Line<'static>> {
    vec![
        section("navigate"),
        entry("\u{2191}\u{2193} / j k", "Move up / down"),
        entry(
            "1..5 / h i",
            "Switch tab (hosts, sftp, tunnels, identities, audit)",
        ),
        entry("Tab", "Toggle detail panel (hosts)"),
        entry("Enter", "Connect / start tunnel"),
        entry("Esc", "Back / close overlay"),
        Line::from(""),
        section("hosts (tab 1)"),
        entry("a", "Add new host"),
        entry("e", "Edit host, or set group default identity on a header"),
        entry("d", "Delete selected host"),
        entry("Shift+D", "Duplicate selected host"),
        entry("f", "Toggle favorite"),
        entry("+ / -", "Zoom: widen / narrow the hosts column"),
        entry("Shift+\u{2191}\u{2193}", "Jump between group headers"),
        entry("s", "Cycle sort mode"),
        entry("Ctrl+\u{2191}\u{2193}", "Move host up / down (manual sort)"),
        entry("c", "Clear SSH log"),
        entry("y", "Copy SSH log for selected host (clipboard)"),
        Line::from(""),
        section("tunnels (tab 3)"),
        entry("a", "Add new tunnel"),
        entry("e", "Edit selected tunnel"),
        entry("d", "Delete tunnel"),
        entry("Enter", "Start / stop tunnel (cancels reconnect while retrying)"),
        entry("x", "Kill tunnel process"),
        entry("R", "Keep-alive reconnect settings (backoff, max retries)"),
        entry(
            "",
            "Keep alive (tunnel form): auto-start on launch + reconnect with backoff after unexpected exit.",
        ),
        entry(
            "Enter/Space",
            "In form on SSH server: pick host (searchable)",
        ),
        Line::from(""),
        section("identities (tab 4)"),
        entry("←→ / l", "Move between columns (grid)"),
        entry("[ / ]", "Fewer / more columns (saved)"),
        entry("a", "Add identity (key or user+password)"),
        entry("e", "Edit identity"),
        entry("d", "Delete identity"),
        entry("p", "Add key to agent"),
        entry("r", "Remove key from agent"),
        Line::from(""),
        section("audit (tab 5)"),
        entry("f", "Cycle filter (all/ok/fail)"),
        entry("r", "Cycle range (all/24h/week/month)"),
        Line::from(""),
        section("search & tags"),
        entry("/", "Fuzzy palette (type to search, Enter connects)"),
        entry("#", "Filter hosts by tag (type to narrow the list)"),
        entry(
            "Space",
            "In the tag list: toggle a tag (combine several, AND)",
        ),
        entry("Enter", "In the tag list: toggle highlighted tag and close"),
        entry("", "In the tag list: (all) removes every filter"),
        entry("Esc", "In Normal mode: clear the active tag filter"),
        entry("", "Tags are comma-separated, e.g.  prod, db, eu-west"),
        Line::from(""),
        section("groups"),
        entry("Space / ←→", "Collapse / expand selected group"),
        entry(
            "Shift+\u{2191}\u{2193}",
            "Jump between group headers (from any row in the group)",
        ),
        entry(
            "Enter",
            "On a group header: collapse/expand; on a host: connect",
        ),
        entry("Shift+Z", "Collapse / expand all groups"),
        entry(
            "Enter",
            "In host form on Group: open dropdown (+ create new)",
        ),
        entry("Shift+G", "Manage groups"),
        entry("Ctrl+G", "Edit selected group (name + default identity)"),
        entry("e", "On a group header: pick its default identity"),
        entry("←/→", "In group form: cycle default identity"),
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
        entry("Ctrl+H", "Settings (session logging, opaque background, …)"),
        entry(
            "Ctrl+K",
            "Edit all keybindings (navigation, tabs, session, …)",
        ),
        entry(
            "",
            "Defaults listed below; rebind any action in the editor.",
        ),
        entry("[session]", ""),
        entry("Ctrl+T", "New session tab (pick host)"),
        entry("Ctrl+W", "Close session tab"),
        entry("Ctrl+D", "Detach to dashboard (session keeps running)"),
        entry(
            "Ctrl+Shift+F",
            "Open SFTP for this host (session keeps running)",
        ),
        entry("Ctrl+[ / Ctrl+]", "Previous / next session tab"),
        entry("Ctrl+PgUp/PgDn", "Previous / next session tab (alternate)"),
        entry("Ctrl+Shift+S", "Focus session from dashboard"),
        entry("PgUp/PgDn", "Scroll session history"),
        entry(
            "",
            "Session logs (opt-in): ~/.local/share/sshub/logs/<host-dir>/ — managed hosts use {name}-{id}; pure ssh_config aliases may share a dir when names sanitize the same. Captures all PTY output including secrets echoed on screen.",
        ),
        entry("", ""),
        entry("[sftp]", ""),
        entry("2", "Open the SFTP tab"),
        entry("Enter", "Connect to host · descend into dir · fold group"),
        entry("Tab", "Switch focus between local and remote pane"),
        entry("Backspace", "Up one directory"),
        entry(
            "\u{2190} / \u{2192}",
            "Stage download / upload (files or whole folders)",
        ),
        entry("c / u", "Run queue / remove last queued transfer"),
        entry("d", "Delete selected file/folder (recursive)"),
        entry("n / R", "New folder / rename in the focused pane"),
        entry("M", "Change permissions (chmod, octal)"),
        entry("r", "Refresh both panes"),
        entry(
            "/",
            "Filter files in the focused pane / search hosts in the picker",
        ),
        entry("s", "Open SSH session for this host (SFTP stays live)"),
        entry("Esc", "Disconnect · back to picker"),
        entry("", ""),
        entry("? / Shift+H", "Toggle this help screen"),
        entry("F2 / Ctrl+S", "Save form (rebindable)"),
        entry("Ctrl+K", "Edit keybindings (all actions)"),
        entry(
            "q / Ctrl+C",
            "Quit (asks to confirm; disable via appearance.confirm_quit)",
        ),
    ]
}

/// The scrollable help body (no border/footer — the caller frames it).
pub fn render_help(scroll: u16) -> Paragraph<'static> {
    Paragraph::new(help_lines()).scroll((scroll, 0))
}
