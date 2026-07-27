use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::theme::catalog::StyleRole;
use crate::theme::model::ResolvedTheme;

/// Fixed footer hint shown below the scrollable help body.
pub const HELP_FOOTER: &str =
    "type to filter  ·  \u{2191}\u{2193}/PgUp/PgDn scroll  ·  Esc clear/close  ·  Enter close";

#[derive(Clone, Copy)]
struct HelpStyles {
    section: Style,
    key: Style,
    description: Style,
}

impl HelpStyles {
    fn of(theme: &ResolvedTheme) -> Self {
        Self {
            section: theme.style(StyleRole::HelpSection),
            key: theme.style(StyleRole::HelpKey),
            description: theme.style(StyleRole::HelpDescription),
        }
    }

    #[cfg(test)]
    fn blank() -> Self {
        Self {
            section: Style::default(),
            key: Style::default(),
            description: Style::default(),
        }
    }
}

/// One row of the help overlay before it is turned into a styled [`Line`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpItem {
    Section(&'static str),
    Entry {
        key: &'static str,
        desc: &'static str,
    },
    Blank,
}

/// Canonical help content. Filtering and rendering both walk this list so an
/// empty query stays byte-identical to the pre-filter layout.
pub const HELP_ITEMS: &[HelpItem] = &[
    HelpItem::Section("navigate"),
    HelpItem::Entry {
        key: "\u{2191}\u{2193} / j k",
        desc: "Move up / down",
    },
    HelpItem::Entry {
        key: "1..5 / h i",
        desc: "Switch tab (hosts, sftp, tunnels, identities, audit)",
    },
    HelpItem::Entry {
        key: "Tab",
        desc: "Toggle detail panel (hosts)",
    },
    HelpItem::Entry {
        key: "Enter",
        desc: "Connect / start tunnel",
    },
    HelpItem::Entry {
        key: "Esc",
        desc: "Back / close overlay",
    },
    HelpItem::Blank,
    HelpItem::Section("hosts (tab 1)"),
    HelpItem::Entry {
        key: "a",
        desc: "Add new host",
    },
    HelpItem::Entry {
        key: "e",
        desc: "Edit host, or set group default identity on a header",
    },
    HelpItem::Entry {
        key: "d",
        desc: "Delete selected host",
    },
    HelpItem::Entry {
        key: "Shift+D",
        desc: "Duplicate selected host",
    },
    HelpItem::Entry {
        key: "f",
        desc: "Toggle favorite",
    },
    HelpItem::Entry {
        key: "+ / -",
        desc: "Zoom: widen / narrow the hosts column",
    },
    HelpItem::Entry {
        key: "Alt+\u{2190}\u{2192}\u{2191}\u{2193}",
        desc: "Move dashboard panel focus",
    },
    HelpItem::Entry {
        key: "z",
        desc: "Zoom the focused panel to full screen (Esc to exit)",
    },
    HelpItem::Entry {
        key: "\u{2191}\u{2193} / PgUp PgDn",
        desc: "Scroll the zoomed panel",
    },
    HelpItem::Entry {
        key: "Shift+\u{2191}\u{2193}",
        desc: "Jump between group headers",
    },
    HelpItem::Entry {
        key: "s",
        desc: "Cycle sort mode",
    },
    HelpItem::Entry {
        key: "Ctrl+\u{2191}\u{2193}",
        desc: "Move host up / down (manual sort)",
    },
    HelpItem::Entry {
        key: "c",
        desc: "Clear SSH log",
    },
    HelpItem::Entry {
        key: "y",
        desc: "Copy SSH log for selected host (clipboard)",
    },
    HelpItem::Entry {
        key: "Shift+P",
        desc: "Push public key to host",
    },
    HelpItem::Blank,
    HelpItem::Section("tunnels (tab 3)"),
    HelpItem::Entry {
        key: "a",
        desc: "Add new tunnel",
    },
    HelpItem::Entry {
        key: "e",
        desc: "Edit selected tunnel",
    },
    HelpItem::Entry {
        key: "d",
        desc: "Delete tunnel",
    },
    HelpItem::Entry {
        key: "Enter",
        desc: "Start / stop tunnel (cancels reconnect while retrying)",
    },
    HelpItem::Entry {
        key: "x",
        desc: "Kill tunnel process",
    },
    HelpItem::Entry {
        key: "R",
        desc: "Keep-alive reconnect settings (backoff, max retries)",
    },
    HelpItem::Entry {
        key: "",
        desc: "Keep alive (tunnel form): auto-start on launch + reconnect with backoff after unexpected exit.",
    },
    HelpItem::Entry {
        key: "Enter/Space",
        desc: "In form on SSH server: pick host (searchable)",
    },
    HelpItem::Blank,
    HelpItem::Section("identities (tab 4)"),
    HelpItem::Entry {
        key: "←→ / l",
        desc: "Move between columns (grid)",
    },
    HelpItem::Entry {
        key: "[ / ]",
        desc: "Fewer / more columns (saved)",
    },
    HelpItem::Entry {
        key: "a",
        desc: "Add identity (key or user+password)",
    },
    HelpItem::Entry {
        key: "e",
        desc: "Edit identity",
    },
    HelpItem::Entry {
        key: "d",
        desc: "Delete identity",
    },
    HelpItem::Entry {
        key: "g",
        desc: "Generate SSH key pair (ed25519 or rsa-4096)",
    },
    HelpItem::Entry {
        key: "p",
        desc: "Add key to agent",
    },
    HelpItem::Entry {
        key: "r",
        desc: "Remove key from agent",
    },
    HelpItem::Entry {
        key: "Shift+P",
        desc: "Push public key to a remote host",
    },
    HelpItem::Entry {
        key: "Ctrl+R",
        desc: "In a form: show and copy the stored secret",
    },
    HelpItem::Entry {
        key: "Ctrl+Y",
        desc: "In a form: copy the stored secret",
    },
    HelpItem::Blank,
    HelpItem::Section("audit (tab 5)"),
    HelpItem::Entry {
        key: "f",
        desc: "Cycle filter (all/ok/fail)",
    },
    HelpItem::Entry {
        key: "r",
        desc: "Cycle range (all/24h/week/month)",
    },
    HelpItem::Blank,
    HelpItem::Section("search & tags"),
    HelpItem::Entry {
        key: "/",
        desc: "Fuzzy palette (type to search, Enter connects)",
    },
    HelpItem::Entry {
        key: "",
        desc: "Unknown [user@]host[:port] offers ad-hoc connect (no save)",
    },
    HelpItem::Entry {
        key: "#",
        desc: "Filter hosts by tag (type to narrow the list)",
    },
    HelpItem::Entry {
        key: "Space",
        desc: "In the tag list: toggle a tag (combine several, AND)",
    },
    HelpItem::Entry {
        key: "Enter",
        desc: "In the tag list: toggle highlighted tag and close",
    },
    HelpItem::Entry {
        key: "",
        desc: "In the tag list: (all) removes every filter",
    },
    HelpItem::Entry {
        key: "Esc",
        desc: "In Normal mode: clear the active tag filter",
    },
    HelpItem::Entry {
        key: "",
        desc: "Tags are comma-separated, e.g.  prod, db, eu-west",
    },
    HelpItem::Blank,
    HelpItem::Section("groups"),
    HelpItem::Entry {
        key: "Space / ←→",
        desc: "Collapse / expand selected group",
    },
    HelpItem::Entry {
        key: "Shift+\u{2191}\u{2193}",
        desc: "Jump between group headers (from any row in the group)",
    },
    HelpItem::Entry {
        key: "Enter",
        desc: "On a group header: collapse/expand; on a host: connect",
    },
    HelpItem::Entry {
        key: "Shift+Z",
        desc: "Collapse / expand all groups",
    },
    HelpItem::Entry {
        key: "Enter",
        desc: "In host form on Group: open dropdown (+ create new)",
    },
    HelpItem::Entry {
        key: "Shift+G",
        desc: "Manage groups",
    },
    HelpItem::Entry {
        key: "Ctrl+G",
        desc: "Edit selected group (name + default identity)",
    },
    HelpItem::Entry {
        key: "e",
        desc: "On a group header: pick its default identity",
    },
    HelpItem::Entry {
        key: "←/→",
        desc: "In group form: cycle default identity",
    },
    HelpItem::Entry {
        key: "Ctrl+Shift+G",
        desc: "Delete selected group",
    },
    HelpItem::Blank,
    HelpItem::Section("import / export"),
    HelpItem::Entry {
        key: "Shift+I",
        desc: "Import from ssh config",
    },
    HelpItem::Entry {
        key: "Shift+E",
        desc: "Export hosts to ssh config",
    },
    HelpItem::Entry {
        key: "Shift+T",
        desc: "Import from Termius export folder",
    },
    HelpItem::Blank,
    HelpItem::Section("termius import (Shift+T)"),
    HelpItem::Entry {
        key: "",
        desc: "Point the prompt at the export folder holding",
    },
    HelpItem::Entry {
        key: "",
        desc: "L00t.csv (+ ssh_keys/). Imports hosts, logins,",
    },
    HelpItem::Entry {
        key: "",
        desc: "passwords & keys; existing hosts are skipped.",
    },
    HelpItem::Blank,
    HelpItem::Section("tools"),
    HelpItem::Entry { key: "", desc: "" },
    HelpItem::Entry {
        key: "Ctrl+H",
        desc: "Settings (session logging, opaque background, …)",
    },
    HelpItem::Entry {
        key: "Ctrl+K",
        desc: "Edit all keybindings (navigation, tabs, session, …)",
    },
    HelpItem::Entry {
        key: "",
        desc: "Defaults listed below; rebind any action in the editor.",
    },
    HelpItem::Entry {
        key: "[session]",
        desc: "",
    },
    HelpItem::Entry {
        key: "Ctrl+T",
        desc: "New session tab (pick host)",
    },
    HelpItem::Entry {
        key: "Ctrl+Shift+T",
        desc: "Open a local shell tab",
    },
    HelpItem::Entry {
        key: "Ctrl+W",
        desc: "Close session tab",
    },
    HelpItem::Entry {
        key: "Ctrl+D",
        desc: "Detach to dashboard (session keeps running)",
    },
    HelpItem::Entry {
        key: "Ctrl+Shift+F",
        desc: "Open SFTP for this host (session keeps running)",
    },
    HelpItem::Entry {
        key: "Ctrl+[ / Ctrl+]",
        desc: "Previous / next session tab",
    },
    HelpItem::Entry {
        key: "Ctrl+PgUp/PgDn",
        desc: "Previous / next session tab (alternate)",
    },
    HelpItem::Entry {
        key: "Ctrl+Shift+S",
        desc: "Focus session from dashboard",
    },
    HelpItem::Entry {
        key: "Alt+S",
        desc: "Switch to an open session (searchable)",
    },
    HelpItem::Entry {
        key: "PgUp/PgDn",
        desc: "Scroll session history",
    },
    HelpItem::Entry {
        key: "",
        desc: "Session logs (opt-in): ~/.local/share/sshub/logs/<host-dir>/ — managed hosts use {name}-{id}; pure ssh_config aliases may share a dir when names sanitize the same. Captures all PTY output including secrets echoed on screen.",
    },
    HelpItem::Entry { key: "", desc: "" },
    HelpItem::Entry {
        key: "[sftp]",
        desc: "",
    },
    HelpItem::Entry {
        key: "2",
        desc: "Open the SFTP tab",
    },
    HelpItem::Entry {
        key: "Enter",
        desc: "Connect to host · descend into dir · fold group",
    },
    HelpItem::Entry {
        key: "Tab",
        desc: "Switch focus between the two panes",
    },
    HelpItem::Entry {
        key: "Backspace",
        desc: "Up one directory (or select the \"..\" row)",
    },
    HelpItem::Entry {
        key: "\u{2190} / \u{2192}",
        desc: "Stage the focused pane's selection toward the other one",
    },
    HelpItem::Entry {
        key: "o / O",
        desc: "Point the left pane at a second server / back to local files",
    },
    HelpItem::Entry {
        key: ".",
        desc: "Show or hide dotfiles in both panes (remembered)",
    },
    HelpItem::Entry {
        key: "c / u",
        desc: "Run queue / remove last queued transfer",
    },
    HelpItem::Entry {
        key: "d",
        desc: "Delete selected file/folder (recursive)",
    },
    HelpItem::Entry {
        key: "n / R",
        desc: "New folder / rename in the focused pane",
    },
    HelpItem::Entry {
        key: "M",
        desc: "Change permissions (chmod, octal)",
    },
    HelpItem::Entry {
        key: "r",
        desc: "Refresh both panes",
    },
    HelpItem::Entry {
        key: "/",
        desc: "Filter files in the focused pane / search hosts in the picker",
    },
    HelpItem::Entry {
        key: "s",
        desc: "Open SSH session for this host (SFTP stays live)",
    },
    HelpItem::Entry {
        key: "Esc",
        desc: "Disconnect · back to picker",
    },
    HelpItem::Entry { key: "", desc: "" },
    HelpItem::Entry {
        key: "[cli]",
        desc: "",
    },
    HelpItem::Entry {
        key: "",
        desc: "Headless CLI available: run  sshub --help  (or  sshub <cmd> --help)",
    },
    HelpItem::Entry {
        key: "",
        desc: "for connect, tunnel, sftp, import/export from scripts (no TUI).",
    },
    HelpItem::Entry { key: "", desc: "" },
    HelpItem::Entry {
        key: "? / Shift+H",
        desc: "Toggle this help screen",
    },
    HelpItem::Entry {
        key: "F2 / Ctrl+S",
        desc: "Save form (rebindable)",
    },
    HelpItem::Entry {
        key: "Ctrl+K",
        desc: "Edit keybindings (all actions)",
    },
    HelpItem::Entry {
        key: "q / Ctrl+C",
        desc: "Quit (asks to confirm; disable via appearance.confirm_quit)",
    },
];

fn entry_line(styles: HelpStyles, key: &'static str, desc: &'static str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{:<16}", key), styles.key),
        Span::styled(desc, styles.description),
    ])
}

fn section_line(styles: HelpStyles, title: &'static str) -> Line<'static> {
    Line::from(Span::styled(title, styles.section))
}

fn item_to_line(styles: HelpStyles, item: HelpItem) -> Line<'static> {
    match item {
        HelpItem::Section(title) => section_line(styles, title),
        HelpItem::Entry { key, desc } => entry_line(styles, key, desc),
        HelpItem::Blank => Line::from(""),
    }
}

fn entry_matches(key: &str, desc: &str, query: &str) -> bool {
    key.to_lowercase().contains(query) || desc.to_lowercase().contains(query)
}

/// Case-insensitive substring filter over key specs and descriptions.
/// Section headers with no surviving entries are dropped; blank separators are
/// kept only when they sit between kept content.
pub fn filtered_help_items(query: &str) -> Vec<HelpItem> {
    if query.is_empty() {
        return HELP_ITEMS.to_vec();
    }
    let q = query.to_lowercase();
    let mut out = Vec::new();
    let mut pending_section: Option<&'static str> = None;
    let mut pending_blank = false;
    for &item in HELP_ITEMS {
        match item {
            HelpItem::Section(title) => {
                pending_section = Some(title);
                // Keep pending_blank so a kept section can still emit the
                // separator that preceded it in HELP_ITEMS.
            }
            HelpItem::Blank => {
                pending_blank = true;
            }
            HelpItem::Entry { key, desc } if entry_matches(key, desc, &q) => {
                if let Some(title) = pending_section.take() {
                    if pending_blank && !out.is_empty() {
                        out.push(HelpItem::Blank);
                    }
                    out.push(HelpItem::Section(title));
                }
                pending_blank = false;
                out.push(item);
            }
            HelpItem::Entry { .. } => {}
        }
    }
    out
}

fn help_lines(query: &str, styles: HelpStyles) -> Vec<Line<'static>> {
    filtered_help_items(query)
        .into_iter()
        .map(|item| item_to_line(styles, item))
        .collect()
}

/// Total number of lines in the (possibly filtered) help content.
pub fn help_line_count(query: &str) -> u16 {
    filtered_help_items(query).len() as u16
}

/// The scrollable help body (no border/footer — the caller frames it).
pub fn render_help(scroll: u16, query: &str, theme: &ResolvedTheme) -> Paragraph<'static> {
    Paragraph::new(help_lines(query, HelpStyles::of(theme))).scroll((scroll, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_keeps_full_item_list() {
        assert_eq!(filtered_help_items(""), HELP_ITEMS.to_vec());
    }

    #[test]
    fn query_matches_key_spec_and_description() {
        let by_key = filtered_help_items("Ctrl+K");
        assert!(by_key.iter().any(|i| matches!(
            i,
            HelpItem::Entry {
                key: "Ctrl+K",
                desc
            } if desc.contains("keybinding")
        )));
        assert!(by_key
            .iter()
            .any(|i| matches!(i, HelpItem::Section("tools"))));

        let by_desc = filtered_help_items("favorite");
        assert!(by_desc.iter().any(|i| matches!(
            i,
            HelpItem::Entry {
                key: "f",
                desc: "Toggle favorite"
            }
        )));
        assert!(by_desc
            .iter()
            .any(|i| matches!(i, HelpItem::Section("hosts (tab 1)"))));
        assert!(!by_desc
            .iter()
            .any(|i| matches!(i, HelpItem::Section("audit (tab 5)"))));
    }

    #[test]
    fn stranded_section_headers_are_omitted() {
        // Body rows under "termius import" mention L00t.csv, not the word
        // "termius" — so a "termius" query must keep "import / export" (Shift+T
        // row) and drop the empty "termius import" section header.
        let by_name = filtered_help_items("termius");
        assert!(by_name
            .iter()
            .any(|i| matches!(i, HelpItem::Section("import / export"))));
        assert!(!by_name
            .iter()
            .any(|i| matches!(i, HelpItem::Section("termius import (Shift+T)"))));
        assert!(!by_name
            .iter()
            .any(|i| matches!(i, HelpItem::Section("navigate"))));

        let by_body = filtered_help_items("L00t");
        assert!(by_body
            .iter()
            .any(|i| matches!(i, HelpItem::Section("termius import (Shift+T)"))));
        assert!(!by_body
            .iter()
            .any(|i| matches!(i, HelpItem::Section("navigate"))));
        assert!(!by_body
            .iter()
            .any(|i| matches!(i, HelpItem::Section("audit (tab 5)"))));
    }

    #[test]
    fn empty_query_lines_match_pre_filter_shape() {
        let styles = HelpStyles::blank();
        let lines = help_lines("", styles);
        assert_eq!(lines.len(), HELP_ITEMS.len());
        // Spot-check first section + entry styling payload.
        assert_eq!(lines[0], section_line(styles, "navigate"));
        assert_eq!(
            lines[1],
            entry_line(styles, "\u{2191}\u{2193} / j k", "Move up / down")
        );
        assert_eq!(lines[6], Line::from(""));
    }
}
