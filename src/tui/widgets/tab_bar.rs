//! Dashboard tab bar — numbered tabs with active highlight.

use ratatui::layout::Rect;
use ratatui::Frame;

use crate::tui::theme;

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
pub fn render_tab_bar(frame: &mut Frame, area: Rect, active_tab: usize, scope_path: &str) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let buf = frame.buffer_mut();
    let y = area.y;
    let mut x = area.x + 1; // 1-char left margin

    for (i, (num, label)) in TABS.iter().enumerate() {
        let tab_num = i + 1;
        let is_active = tab_num == active_tab;

        if is_active {
            // Active: number in INV style (bright bg, dark fg)
            buf.set_string(x, y, "[", theme::dim());
            x += 1;
            buf.set_string(x, y, num, theme::inv());
            x += num.len() as u16;
            buf.set_string(x, y, "]", theme::dim());
            x += 1;
            buf.set_string(x, y, " ", theme::mute());
            x += 1;
            buf.set_string(x, y, label, theme::mute());
            x += label.len() as u16;
        } else {
            // Inactive: number in MUTE, label in DIM
            buf.set_string(x, y, " ", theme::dim());
            x += 1;
            buf.set_string(x, y, num, theme::mute());
            x += num.len() as u16;
            buf.set_string(x, y, " ", theme::dim());
            x += 1;
            buf.set_string(x, y, " ", theme::dim());
            x += 1;
            buf.set_string(x, y, label, theme::dim());
            x += label.len() as u16;
        }

        // Space between tabs
        buf.set_string(x, y, "   ", theme::dim());
        x += 3;
    }

    // Scope path — far right
    let scope_text = format!("scope: {}", scope_path);
    let scope_len = scope_text.len() as u16;
    if area.width > scope_len + 2 {
        let scope_x = area.x + area.width - scope_len - 1;
        buf.set_string(scope_x, y, "scope: ", theme::dim());
        buf.set_string(scope_x + 7, y, scope_path, theme::white());
    }
}
