//! Dashboard tab bar — numbered tabs with active highlight.

use std::sync::OnceLock;

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::Frame;

use crate::theme::catalog::{ColorRole, StyleRole};
use crate::theme::model::ResolvedTheme;

/// The version string shown at the far right, resolved once at runtime.
///
/// * `SSHUB_VERSION_LABEL` **unset** → `v{CARGO_PKG_VERSION}` (normal build).
/// * set but **empty** → `None`, the version is hidden entirely (used by the
///   demo recordings so their GIFs never advertise a stale version).
/// * set and **non-empty** → that exact string (custom label).
fn version_label() -> Option<&'static str> {
    static LABEL: OnceLock<Option<String>> = OnceLock::new();
    LABEL
        .get_or_init(|| resolve_version_label(std::env::var("SSHUB_VERSION_LABEL").ok()))
        .as_deref()
}

/// Pure resolution of the version label from the raw env value (extracted so it
/// can be unit tested without touching process-global env).
fn resolve_version_label(var: Option<String>) -> Option<String> {
    match var {
        // Explicitly hidden (demo recordings set SSHUB_VERSION_LABEL="").
        Some(s) if s.trim().is_empty() => None,
        Some(s) => Some(s),
        None => Some(concat!("v", env!("CARGO_PKG_VERSION")).to_string()),
    }
}

/// Tab definitions: (number label, display name).
const TABS: [(&str, &str); 5] = [
    ("1", "hosts"),
    ("2", "sftp"),
    ("3", "tunnels"),
    ("4", "identities"),
    ("5", "audit"),
];

/// Render the tab bar into a 1-row `area`.
///
/// * `active_tab` — 1-based index (1 = hosts, 2 = tunnels, …)
/// * `scope_path` — shown at far right, e.g. `"~/.config/sshub"`
pub fn render_tab_bar(
    frame: &mut Frame,
    area: Rect,
    active_tab: usize,
    scope_path: &str,
    theme: &ResolvedTheme,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let buf = frame.buffer_mut();
    let y = area.y;
    let mut x = area.x + 1; // 1-char left margin
    let active_style = theme.style(StyleRole::TabsActive);
    // `tabs.inactive` is the muted role the *number* of a closed tab carries;
    // its label has always sat one step further back, on `text.dim`.
    let inactive_style = theme.style(StyleRole::TabsInactive);
    let dim = theme.style(StyleRole::TextDim);
    let muted = theme.style(StyleRole::TextMuted);

    for (i, (num, label)) in TABS.iter().enumerate() {
        let tab_num = i + 1;
        let is_active = tab_num == active_tab;

        if is_active {
            // Active: number inverted (bright bg, dark fg)
            buf.set_string(x, y, "[", dim);
            x += 1;
            buf.set_string(x, y, num, active_style);
            x += num.len() as u16;
            buf.set_string(x, y, "]", dim);
            x += 1;
            buf.set_string(x, y, " ", muted);
            x += 1;
            buf.set_string(x, y, label, muted);
            x += label.len() as u16;
        } else {
            buf.set_string(x, y, " ", dim);
            x += 1;
            buf.set_string(x, y, num, inactive_style);
            x += num.len() as u16;
            buf.set_string(x, y, " ", dim);
            x += 1;
            buf.set_string(x, y, " ", dim);
            x += 1;
            buf.set_string(x, y, label, dim);
            x += label.len() as u16;
        }

        // Space between tabs
        buf.set_string(x, y, "   ", dim);
        x += 3;
    }

    // Version + scope path — far right. The version (when shown) sits at the
    // very edge; the scope path is placed to its left, or hugs the edge itself
    // when the version is hidden (SSHUB_VERSION_LABEL="").
    // No `Style` role resolves to the highlight slot alone, and the scope path
    // has always been the brightest text on this row.
    let scope_style = Style::default().fg(theme.semantic().text_highlight);
    let draw_scope = |buf: &mut ratatui::buffer::Buffer, right_x: u16| {
        let scope_len = (7 + scope_path.len()) as u16; // "scope: " + path
        if right_x > area.x + scope_len + 2 {
            let scope_x = right_x - scope_len;
            buf.set_string(scope_x, y, "scope: ", dim);
            buf.set_string(scope_x + 7, y, scope_path, scope_style);
        }
    };

    match version_label() {
        Some(version) => {
            let ver_len = version.len() as u16;
            if area.width > ver_len + 2 {
                let ver_x = area.x + area.width - ver_len - 1;
                let ok = Style::default().fg(theme.color(ColorRole::StatusSuccess));
                buf.set_string(ver_x, y, version, ok);
                // Two-space gap between the scope path and the version.
                draw_scope(buf, ver_x.saturating_sub(2));
            }
        }
        None => draw_scope(buf, area.x + area.width - 1),
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_version_label;

    #[test]
    fn version_label_resolution() {
        // Unset → the compiled version.
        assert_eq!(
            resolve_version_label(None).as_deref(),
            Some(concat!("v", env!("CARGO_PKG_VERSION")))
        );
        // Empty (or whitespace) → hidden.
        assert_eq!(resolve_version_label(Some(String::new())), None);
        assert_eq!(resolve_version_label(Some("   ".into())), None);
        // Non-empty → that exact custom label.
        assert_eq!(
            resolve_version_label(Some("demo".into())).as_deref(),
            Some("demo")
        );
    }
}
