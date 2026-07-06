use ratatui::layout::Rect;
use ratatui::prelude::*;

use crate::app::App;
use crate::ssh::agent::AgentInfo;
use crate::store::Identity;
use crate::tui::theme;

const CARD_W: u16 = 42;
const CARD_H: u16 = 6;
/// Narrowest a card may shrink to before content becomes unreadable.
const MIN_CARD_W: u16 = 26;
const CARD_GAP: u16 = 2;

/// Inner content width of the identities body for a given total width
/// (mirrors the margin logic in [`render_keys`]).
pub fn inner_width(total_width: u16) -> u16 {
    let margin = if total_width >= 132 {
        2
    } else if total_width >= 80 {
        1
    } else {
        0
    };
    total_width.saturating_sub(margin * 2)
}

/// How many columns of at least [`MIN_CARD_W`] fit into `inner_w`.
pub fn max_columns(inner_w: u16) -> usize {
    (((inner_w + CARD_GAP) / (MIN_CARD_W + CARD_GAP)) as usize).max(1)
}

/// Resolve the column count for the identities grid. `pref == 0` means auto
/// (the original 1-or-2 heuristic); otherwise the user's choice, clamped to
/// what fits.
pub fn resolve_columns(inner_w: u16, pref: usize) -> usize {
    if pref == 0 {
        if inner_w >= CARD_W * 2 + CARD_GAP {
            2
        } else {
            1
        }
    } else {
        pref.clamp(1, max_columns(inner_w))
    }
}

pub fn render_keys(frame: &mut Frame, area: Rect, app: &App) {
    if area.height < 4 || area.width < 20 {
        return;
    }

    let buf = frame.buffer_mut();
    let margin = if area.width >= 132 {
        2
    } else if area.width >= 80 {
        1
    } else {
        0
    };
    let inner_x = area.x + margin;
    let inner_w = area.width.saturating_sub(margin * 2);

    let agent = app.agent_info.as_ref();

    // Cards per row — user preference (0 = auto), clamped to what fits.
    let cards_per_row = resolve_columns(inner_w, app.config.appearance.identity_columns);
    let cpr_u16 = cards_per_row as u16;
    let card_w = (inner_w.saturating_sub((cpr_u16 - 1) * CARD_GAP)) / cpr_u16;

    if app.identities.is_empty() {
        let msg = "No identities — press 'a' (key or user+password)";
        let x = inner_x + (inner_w.saturating_sub(msg.len() as u16)) / 2;
        buf.set_string(x, area.y + 2, msg, theme::dim());
    }

    // Cards are laid out in rows of `cards_per_row`. Once there are more rows
    // than fit, scroll by whole card-rows to keep the selected card on screen
    // (roughly centered).
    let row_stride = CARD_H + 1;
    let cpr = (cards_per_row as usize).max(1);
    let total_rows = app.identities.len().div_ceil(cpr);
    let visible_rows = ((area.height / row_stride) as usize).max(1);
    let row_offset = app.keys_scroll_row_offset(area.height, cpr, row_stride);
    let window_end_row = row_offset + visible_rows;

    for (i, identity) in app.identities.iter().enumerate() {
        let row = i / cpr;
        if row < row_offset {
            continue;
        }
        if row >= window_end_row {
            break;
        }

        let col = i % cpr;
        let card_x = inner_x + (col as u16) * (card_w + CARD_GAP);
        let y = area.y + ((row - row_offset) as u16) * row_stride;

        let is_selected = i == app.identity_selected;
        render_card(buf, card_x, y, card_w, identity, is_selected, agent);
    }

    // Agent info below the visible cards (only when there is room left).
    let drawn_rows = window_end_row.min(total_rows).saturating_sub(row_offset);
    let mut y = area.y + (drawn_rows as u16) * row_stride;
    if y + 3 <= area.y + area.height {
        y += 1;
        render_agent_info(buf, inner_x, y, inner_w, agent);
    }

    // Notice
    if let Some(notice) = app.identity_notice.as_deref() {
        let notice_y = area.y + area.height.saturating_sub(2);
        buf.set_string(
            inner_x,
            notice_y,
            truncate(notice, inner_w as usize),
            theme::amber(),
        );
    }
}

fn render_card(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    w: u16,
    identity: &Identity,
    selected: bool,
    agent: Option<&AgentInfo>,
) {
    let border_style = if selected {
        Style::default().fg(theme::ACCENT)
    } else {
        theme::border()
    };

    // Top border
    let top = format!("┌{}┐", "─".repeat((w - 2) as usize));
    buf.set_string(x, y, &top, border_style);

    // Bottom border
    let bottom = format!("└{}┘", "─".repeat((w - 2) as usize));
    buf.set_string(x, y + CARD_H - 1, &bottom, border_style);

    // Side borders
    for row in 1..CARD_H - 1 {
        buf.set_string(x, y + row, "│", border_style);
        buf.set_string(x + w - 1, y + row, "│", border_style);
        // Clear interior
        for cx in x + 1..x + w - 1 {
            if let Some(cell) = buf.cell_mut((cx, y + row)) {
                cell.set_symbol(" ");
                if selected {
                    cell.set_style(theme::selected());
                }
            }
        }
    }

    let inner_x = x + 2;
    let inner_w = w.saturating_sub(4);
    let text_style = if selected {
        theme::selected()
    } else {
        theme::text()
    };

    // Row 1: Name + key type
    let name_style = if selected {
        Style::default()
            .fg(theme::BRIGHT)
            .bg(theme::SEL_BG)
            .add_modifier(Modifier::BOLD)
    } else {
        theme::heading()
    };
    buf.set_string(
        inner_x,
        y + 1,
        truncate(&identity.name, inner_w as usize / 2),
        name_style,
    );

    let key_type = detect_key_type(identity);
    let type_x = x + w - 2 - key_type.len() as u16;
    let type_style = if selected {
        Style::default().fg(theme::MUTE).bg(theme::SEL_BG)
    } else {
        theme::mute()
    };
    buf.set_string(type_x, y + 1, &key_type, type_style);

    // Row 2: Username + fingerprint
    let username = identity.username.as_deref().unwrap_or("-");
    buf.set_string(
        inner_x,
        y + 2,
        truncate(username, inner_w as usize / 2),
        text_style,
    );

    if let Some(fp) = find_fingerprint(identity, agent) {
        let fp_x = inner_x + (inner_w / 2);
        let fp_style = if selected {
            Style::default().fg(theme::DIM).bg(theme::SEL_BG)
        } else {
            theme::dim()
        };
        buf.set_string(fp_x, y + 2, truncate(&fp, (inner_w / 2) as usize), fp_style);
    }

    // Row 3: Key path (or a note for a keyless password credential)
    let path_str = identity
        .private_key
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "password login (no key)".into());
    let path_style = if selected {
        Style::default().fg(theme::DIM).bg(theme::SEL_BG)
    } else {
        theme::dim()
    };
    buf.set_string(
        inner_x,
        y + 3,
        truncate(&path_str, inner_w as usize),
        path_style,
    );

    // Row 4: for a key, show agent load status (+ passphrase indicator);
    // for a keyless password credential, just the password status.
    if identity.private_key.is_some() {
        let loaded = is_loaded_in_agent(identity, agent);
        let (dot, dot_color, label) = if loaded {
            ("●", theme::GREEN, " loaded")
        } else {
            ("○", theme::DIM, " not loaded")
        };
        let dot_style = if selected {
            Style::default().fg(dot_color).bg(theme::SEL_BG)
        } else {
            Style::default().fg(dot_color)
        };
        buf.set_string(inner_x, y + 4, dot, dot_style);
        let label_style = if selected {
            Style::default()
                .fg(if loaded { theme::GREEN } else { theme::DIM })
                .bg(theme::SEL_BG)
        } else if loaded {
            theme::green()
        } else {
            theme::dim()
        };
        buf.set_string(inner_x + 1, y + 4, label, label_style);

        if identity.has_password {
            // Passphrase indicator, placed after the status label (whose width
            // varies: " loaded" vs " not loaded") with a 2-col gap, if it fits.
            let pw_x = inner_x + 1 + label.chars().count() as u16 + 2;
            let pw_text = "● passphrase";
            if pw_x + pw_text.chars().count() as u16 <= inner_x + inner_w {
                let pw_style = if selected {
                    Style::default().fg(theme::AMBER).bg(theme::SEL_BG)
                } else {
                    theme::amber()
                };
                buf.set_string(pw_x, y + 4, pw_text, pw_style);
            }
        }
    } else {
        let (dot, color, text) = if identity.has_password {
            ("●", theme::AMBER, " password set")
        } else {
            ("○", theme::DIM, " no password")
        };
        let base = if selected {
            Style::default().bg(theme::SEL_BG)
        } else {
            Style::default()
        };
        buf.set_string(inner_x, y + 4, dot, base.fg(color));
        buf.set_string(inner_x + 1, y + 4, text, base.fg(color));
    }
}

fn render_agent_info(buf: &mut Buffer, x: u16, y: u16, w: u16, agent: Option<&AgentInfo>) {
    let line: String = std::iter::repeat_n('─', w as usize).collect();
    buf.set_string(x, y, &line, theme::dim());

    let info_y = y + 1;
    match agent {
        Some(info) => {
            let socket = info.socket_path.as_deref().unwrap_or("(not set)");
            buf.set_string(x, info_y, "agent socket  ", theme::mute());
            buf.set_string(
                x + 14,
                info_y,
                truncate(socket, (w - 14) as usize),
                theme::text(),
            );

            let key_count = info.keys.len();
            buf.set_string(x, info_y + 1, "loaded keys   ", theme::mute());
            buf.set_string(x + 14, info_y + 1, key_count.to_string(), theme::bright());
        }
        None => {
            buf.set_string(x, info_y, "SSH agent not detected", theme::dim());
        }
    }
}

fn detect_key_type(identity: &Identity) -> String {
    let path = identity
        .private_key
        .as_ref()
        .map(|p| p.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    if identity.private_key.is_none() {
        "password".into()
    } else if path.contains("ed25519") {
        "ed25519".into()
    } else if path.contains("ecdsa") {
        "ecdsa".into()
    } else if path.contains("rsa") {
        "rsa".into()
    } else if path.contains("dsa") {
        "dsa".into()
    } else {
        "key".into()
    }
}

fn find_fingerprint(identity: &Identity, agent: Option<&AgentInfo>) -> Option<String> {
    let agent = agent?;
    let key_path = identity.private_key.as_ref()?.to_string_lossy();
    agent
        .keys
        .iter()
        .find(|k| k.comment.contains(key_path.as_ref()))
        .map(|k| k.fingerprint.clone())
}

fn is_loaded_in_agent(identity: &Identity, agent: Option<&AgentInfo>) -> bool {
    let Some(agent) = agent else { return false };
    let Some(ref key_path) = identity.private_key else {
        return false;
    };
    let path_str = key_path.to_string_lossy();
    agent
        .keys
        .iter()
        .any(|k| k.comment.contains(path_str.as_ref()))
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        let end = s
            .char_indices()
            .take(max)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0);
        &s[..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use std::path::PathBuf;

    fn row_text(buf: &Buffer, y: u16, w: u16) -> String {
        (0..w)
            .map(|x| buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "))
            .collect()
    }

    fn identity(private_key: Option<&str>, has_password: bool) -> Identity {
        Identity {
            id: 1,
            name: "selectel-core".into(),
            username: Some("root".into()),
            private_key: private_key.map(PathBuf::from),
            certificate: None,
            has_password,
        }
    }

    #[test]
    fn resolve_columns_auto_and_manual() {
        // Auto (pref 0): 2 when two full cards fit, else 1.
        assert_eq!(resolve_columns(CARD_W * 2 + CARD_GAP, 0), 2);
        assert_eq!(resolve_columns(CARD_W, 0), 1);
        // Manual pref clamps to what fits.
        assert_eq!(resolve_columns(200, 3), 3);
        assert_eq!(resolve_columns(60, 4), max_columns(60));
        assert!(resolve_columns(60, 4) >= 1);
        // pref of 1 always honoured.
        assert_eq!(resolve_columns(500, 1), 1);
    }

    #[test]
    fn key_card_status_row_does_not_overlap() {
        let mut buf = Buffer::empty(Rect::new(0, 0, CARD_W, CARD_H));
        let id = identity(Some("/home/u/.ssh/sshub_selectel-core"), true);
        render_card(&mut buf, 0, 0, CARD_W, &id, false, None);

        let row = row_text(&buf, 4, CARD_W);
        // Both labels present, and "passphrase" isn't glued onto "loaded".
        assert!(row.contains("not loaded"), "row: {row:?}");
        assert!(row.contains("passphrase"), "row: {row:?}");
        assert!(!row.contains("loaded● passphrase") && !row.contains("loaded●passphrase"),
            "labels overlap: {row:?}");
        assert!(row.contains("loaded  ● passphrase") || row.contains("loaded ● passphrase"),
            "expected a gap before the passphrase marker: {row:?}");
    }

    #[test]
    fn keyless_card_shows_password_credential() {
        let mut buf = Buffer::empty(Rect::new(0, 0, CARD_W, CARD_H));
        let id = identity(None, true);
        render_card(&mut buf, 0, 0, CARD_W, &id, false, None);

        assert!(row_text(&buf, 1, CARD_W).contains("password"), "badge missing");
        assert!(row_text(&buf, 3, CARD_W).contains("no key"), "row3: expected keyless note");
        assert!(row_text(&buf, 4, CARD_W).contains("password set"), "row4: expected password status");
    }
}
