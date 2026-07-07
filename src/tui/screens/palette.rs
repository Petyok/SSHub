use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear};

use crate::app::HostEntry;
use crate::tui::theme;

/// Maximum number of result rows visible in the palette list.
const MAX_VISIBLE_ROWS: usize = 12;

/// Render the fuzzy palette popup as a centred overlay.
///
/// * `query` – current search text typed by the user.
/// * `hosts` – full host list from `App::hosts`.
/// * `filtered` – indices into `hosts` for the current fuzzy match set.
/// * `selected` – which row inside `filtered` is highlighted (0-based).
pub fn render_palette(
    frame: &mut Frame,
    query: &str,
    hosts: &[HostEntry],
    filtered: &[usize],
    selected: usize,
) {
    let area = frame.area();

    // ── popup geometry ──────────────────────────────────────
    let popup_width = (area.width * 80 / 100).clamp(50, 96.min(area.width));
    // 1 border-top + 1 prompt + 1 separator + MAX_VISIBLE_ROWS + 1 separator
    // + 4 detail rows + 1 border-bottom = MAX_VISIBLE_ROWS + 9
    let popup_height = ((MAX_VISIBLE_ROWS as u16) + 9).clamp(1, area.height);
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);

    // ── outer border ────────────────────────────────────────
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border())
        .style(Style::default().bg(theme::BG))
        .title(Span::styled(" quick connect ", theme::heading()));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    // We'll write directly into the buffer for fine-grained control.
    let buf = frame.buffer_mut();
    let w = inner.width as usize;

    // ── prompt line (row 0 of inner) ────────────────────────
    {
        let row_y = inner.y;
        let mut col = inner.x;

        // prompt marker
        buf.set_string(col, row_y, " \u{276f} ", theme::green());
        col += 4;

        // query text
        buf.set_string(col, row_y, query, theme::white());
        col += query.len() as u16;

        // blinking caret
        buf.set_string(col, row_y, "\u{2588}", theme::green());

        // right-aligned match count: "<matches>/<total>"
        let counter = format!("{}/{}", filtered.len(), hosts.len());
        let counter_x = inner.x + inner.width.saturating_sub(counter.len() as u16 + 1);
        buf.set_string(counter_x, row_y, &counter, theme::mute());
    }

    // ── separator line (row 1) ──────────────────────────────
    {
        let sep_y = inner.y + 1;
        let line = "\u{2500}".repeat(w);
        buf.set_string(inner.x, sep_y, &line, theme::border());
    }

    // ── result rows (rows 2 .. 2+MAX_VISIBLE_ROWS) ─────────
    let list_start_y = inner.y + 2;
    let _visible_count = filtered.len().min(MAX_VISIBLE_ROWS);

    // Scroll window: keep `selected` visible.
    let scroll_offset = if selected >= MAX_VISIBLE_ROWS {
        selected - MAX_VISIBLE_ROWS + 1
    } else {
        0
    };

    for i in 0..MAX_VISIBLE_ROWS {
        let row_y = list_start_y + i as u16;
        if row_y >= inner.y + inner.height {
            break;
        }

        let idx_in_filtered = scroll_offset + i;
        if idx_in_filtered >= filtered.len() {
            // empty row
            continue;
        }

        let host_idx = filtered[idx_in_filtered];
        let is_selected = idx_in_filtered == selected;

        // CONTRACT: HostEntry interface
        let entry = &hosts[host_idx];
        let name = entry.display_name();
        let _tags_str = entry.tags().join(" \u{00b7} ");

        // group name (from managed host's group, if any)
        // CONTRACT: HostEntry interface — managed().group
        let group_name = entry
            .managed()
            .and_then(|m| m.group.as_ref())
            .map(|g| g.name.as_str())
            .unwrap_or("");

        // user string (from identity username or "")
        // CONTRACT: HostEntry interface — managed().identity
        let user = entry
            .managed()
            .and_then(|m| m.identity.as_ref())
            .and_then(|id| id.username.as_deref())
            .unwrap_or("");

        let row_style = if is_selected {
            theme::selected()
        } else {
            Style::default()
        };

        // Fill entire row with background style
        let blank = " ".repeat(w);
        buf.set_string(inner.x, row_y, &blank, row_style);

        let mut col = inner.x;

        // selection marker
        if is_selected {
            buf.set_string(col, row_y, " \u{25b8} ", theme::green().bg(theme::SEL_BG));
        } else {
            buf.set_string(col, row_y, "   ", row_style);
        }
        col += 3;

        // host name — up to 30 chars
        let name_width = 30.min(w.saturating_sub(3));
        let name_display = crate::tui::text::pad_ellipsize(name, name_width);

        let name_style = if is_selected {
            theme::white().bg(theme::SEL_BG)
        } else {
            theme::bright()
        };
        buf.set_string(col, row_y, &name_display, name_style);
        col += name_width as u16 + 1;

        // group label — up to 14 chars
        if col < inner.x + inner.width {
            let group_width = 14.min((inner.x + inner.width - col) as usize);
            let group_display = crate::tui::text::pad_ellipsize(group_name, group_width);
            buf.set_string(
                col,
                row_y,
                &group_display,
                theme::mute().bg(row_style.bg.unwrap_or(theme::BG)),
            );
            col += group_width as u16 + 1;
        }

        // user — up to 14 chars
        if col < inner.x + inner.width {
            let user_width = 14.min((inner.x + inner.width - col) as usize);
            let user_display = crate::tui::text::pad_ellipsize(user, user_width);
            buf.set_string(
                col,
                row_y,
                &user_display,
                theme::dim().bg(row_style.bg.unwrap_or(theme::BG)),
            );
        }
    }

    // ── separator before detail block ───────────────────────
    let detail_sep_y = list_start_y + MAX_VISIBLE_ROWS as u16;
    if detail_sep_y < inner.y + inner.height {
        let line = "\u{2500}".repeat(w);
        buf.set_string(inner.x, detail_sep_y, &line, theme::border());
    }

    // ── detail block (4 rows) ───────────────────────────────
    let detail_y = detail_sep_y + 1;
    if !filtered.is_empty() && detail_y + 1 < inner.y + inner.height {
        let sel_idx = filtered[selected.min(filtered.len() - 1)];
        let entry = &hosts[sel_idx];

        // CONTRACT: HostEntry interface
        let host_addr = entry
            .managed()
            .map(|m| {
                let port = m.port;
                if port == 22 {
                    m.address.clone()
                } else {
                    format!("{}:{}", m.address, port)
                }
            })
            .unwrap_or_else(|| entry.name().to_string());

        // Prefer the host's own username, then fall back to its identity's.
        let user_full = entry
            .managed()
            .and_then(|m| {
                m.username
                    .as_deref()
                    .or_else(|| m.identity.as_ref().and_then(|id| id.username.as_deref()))
            })
            .unwrap_or("");

        // Show the connection target as `user@host` only when a user is known;
        // otherwise leave the "user" field empty rather than echoing the address.
        let user_addr = if user_full.is_empty() {
            String::new()
        } else {
            format!("{}@{}", user_full, host_addr)
        };

        let identity_path = entry
            .managed()
            .and_then(|m| m.identity.as_ref())
            .and_then(|id| id.private_key.as_ref())
            .map(|p| p.display().to_string())
            .unwrap_or_default();

        let jump_host = entry
            .managed()
            .and_then(|m| m.proxy_jump.as_deref())
            .unwrap_or("");

        let tags_display = entry.tags().join(" \u{00b7} ");

        // Row 0: host + user
        let half = (w / 2) as u16;
        render_detail_kv(buf, inner.x, detail_y, half, "host", &host_addr);
        render_detail_kv(buf, inner.x + half, detail_y, half, "user", &user_addr);

        // Row 1: identity + jump
        if detail_y + 1 < inner.y + inner.height {
            render_detail_kv(buf, inner.x, detail_y + 1, half, "identity", &identity_path);
            render_detail_kv(buf, inner.x + half, detail_y + 1, half, "jump", jump_host);
        }

        // Row 2: tags
        if detail_y + 2 < inner.y + inner.height {
            render_detail_kv(
                buf,
                inner.x,
                detail_y + 2,
                inner.width,
                "tags",
                &tags_display,
            );
        }
    }

    // ── hint line below the box ─────────────────────────────
    let hint_y = popup_area.y + popup_area.height;
    if hint_y < area.height {
        let hint = " \u{21b5} connect   esc cancel";
        let hint_x = popup_area.x + (popup_area.width.saturating_sub(hint.len() as u16)) / 2;
        buf.set_string(hint_x, hint_y, hint, theme::mute());
    }
}

/// Render a "key  value" pair at the given position, with the key in MUTE and value in TEXT.
fn render_detail_kv(buf: &mut Buffer, x: u16, y: u16, max_width: u16, key: &str, value: &str) {
    let label = format!(" {:<10}", key);
    buf.set_string(x, y, &label, theme::mute());
    let val_x = x + label.len() as u16;
    let avail = max_width.saturating_sub(label.len() as u16) as usize;
    let truncated = crate::tui::text::ellipsize(value, avail);
    buf.set_string(val_x, y, &truncated, theme::text());
}
