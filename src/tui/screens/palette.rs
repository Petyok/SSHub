use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders};

use crate::app::{App, HostEntry};
use crate::theme::catalog::{ColorRole, PaintRole, StyleRole};
use crate::theme::model::{ResolvedPaint, ResolvedTheme};

/// Maximum number of result rows visible in the palette list.
const MAX_VISIBLE_ROWS: usize = 12;

/// Render the fuzzy palette popup as a centred overlay.
///
/// * `query` – current search text typed by the user.
/// * `hosts` – full host list from `App::hosts`.
/// * `filtered` – indices into `hosts` for the current fuzzy match set.
/// * `selected` – which row inside `filtered` is highlighted (0-based). When an
///   ad-hoc target is offered, `selected == filtered.len()` highlights it.
/// * `adhoc` – optional "connect without saving" target rendered as one extra
///   row (index `filtered.len()`, one past the last result) beneath the list.
pub fn render_palette(
    frame: &mut Frame,
    app: &App,
    query: &str,
    hosts: &[HostEntry],
    filtered: &[usize],
    selected: usize,
    adhoc: Option<&crate::app::adhoc::AdhocTarget>,
) {
    let area = frame.area();

    // ── popup geometry ──────────────────────────────────────
    let popup_width = crate::tui::fit_popup(area.width * 80 / 100, 50, 96.min(area.width));
    // 1 border-top + 1 prompt + 1 separator + MAX_VISIBLE_ROWS + 1 separator
    // + 4 detail rows + 1 border-bottom = MAX_VISIBLE_ROWS + 9
    let popup_height = crate::tui::fit_popup((MAX_VISIBLE_ROWS as u16) + 9, 1, area.height);
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);
    let popup_area = crate::tui::popup_open_rect(popup_area, app);

    let theme = app.theme();
    // Clears the area and lays down `components.popup.background`, exactly as
    // every other overlay does.
    crate::tui::open_popup(frame, popup_area, theme);
    // The palette is then the one overlay that has always been *opaque*.
    // `popup.background` is transparent in `default`, as every surface role is,
    // so where it resolves to the terminal's own ground the opaque companion
    // `semantic.canvas` stands in — the same substitution the fade pass makes,
    // and under `default` literally the colour this used to hard-code. A theme
    // that names its own popup background keeps it untouched.
    if *theme.paint(PaintRole::PopupBackground) == ResolvedPaint::Solid(Color::Reset) {
        frame.render_widget(
            Block::default().style(Style::default().bg(theme.semantic().canvas)),
            popup_area,
        );
    }

    // ── outer border ────────────────────────────────────────
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(crate::tui::popup_border_style(theme, popup_area))
        .title(Span::styled(
            " quick connect ",
            theme.style(StyleRole::PopupTitle),
        ));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);
    // Below this the palette writes into the buffer directly. `set_string`
    // clips horizontally, but an out-of-range *row* panics — and on a terminal
    // narrower or shorter than the popup's own minimum the frame alone can eat
    // every inner cell.
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let prompt = Style::default().fg(theme.color(ColorRole::StatusSuccess));
    let row_selected = theme.style(StyleRole::CommandPaletteRowSelected);
    let separator = Style::default().fg(crate::tui::blit::line_color(
        theme,
        PaintRole::SeparatorPrimary,
        inner,
    ));
    let legend = theme.style(StyleRole::PopupLegend);
    let hint = theme.style(StyleRole::PopupHint);
    let theme_query = theme.style(StyleRole::PickerQuery);
    let detail = DetailStyles::of(theme);

    // We'll write directly into the buffer for fine-grained control.
    let buf = frame.buffer_mut();
    let w = inner.width as usize;

    // ── prompt line (row 0 of inner) ────────────────────────
    {
        let row_y = inner.y;
        let mut col = inner.x;

        // prompt marker
        buf.set_string(col, row_y, " \u{276f} ", prompt);
        col += 4;

        // query text
        buf.set_string(col, row_y, query, theme_query);
        col += query.len() as u16;

        // blinking caret
        buf.set_string(col, row_y, "\u{2588}", prompt);

        // right-aligned match count: "<matches>/<total>"
        let counter = format!("{}/{}", filtered.len(), hosts.len());
        let counter_x = inner.x + inner.width.saturating_sub(counter.len() as u16 + 1);
        buf.set_string(counter_x, row_y, &counter, legend);
    }

    // ── separator line (row 1) ──────────────────────────────
    {
        let sep_y = inner.y + 1;
        if sep_y < inner.bottom() {
            let line = "\u{2500}".repeat(w);
            buf.set_string(inner.x, sep_y, &line, separator);
        }
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
            row_selected
        } else {
            Style::default()
        };

        // Fill entire row with background style
        let blank = " ".repeat(w);
        buf.set_string(inner.x, row_y, &blank, row_style);

        let mut col = inner.x;

        // selection marker
        if is_selected {
            // Foreground only: the row fill above already set the background.
            buf.set_string(col, row_y, " \u{25b8} ", prompt);
        } else {
            buf.set_string(col, row_y, "   ", row_style);
        }
        col += 3;

        // host name — up to 30 chars
        let name_width = 30.min(w.saturating_sub(3));
        let name_display = crate::tui::text::pad_ellipsize(name, name_width);

        let name_style = if is_selected {
            row_selected
        } else {
            theme.style(StyleRole::TextBright)
        };
        buf.set_string(col, row_y, &name_display, name_style);
        col += name_width as u16 + 1;

        // group label — up to 14 chars
        if col < inner.x + inner.width {
            let group_width = 14.min((inner.x + inner.width - col) as usize);
            let group_display = crate::tui::text::pad_ellipsize(group_name, group_width);
            buf.set_string(col, row_y, &group_display, legend);
            col += group_width as u16 + 1;
        }

        // user — up to 14 chars
        if col < inner.x + inner.width {
            let user_width = 14.min((inner.x + inner.width - col) as usize);
            let user_display = crate::tui::text::pad_ellipsize(user, user_width);
            buf.set_string(col, row_y, &user_display, hint);
        }
    }

    // ── ad-hoc "connect without saving" row ─────────────────
    // Rendered as a virtual result at index `filtered.len()` (one past the last
    // real match), so it sits directly beneath the results and shares the same
    // scroll window / selection convention.
    if let Some(adhoc) = adhoc {
        let virtual_idx = filtered.len();
        if virtual_idx >= scroll_offset {
            let vis = virtual_idx - scroll_offset;
            if vis < MAX_VISIBLE_ROWS {
                let row_y = list_start_y + vis as u16;
                if row_y < inner.y + inner.height {
                    let is_selected = selected == virtual_idx;
                    let row_style = if is_selected {
                        row_selected
                    } else {
                        Style::default()
                    };

                    let blank = " ".repeat(w);
                    buf.set_string(inner.x, row_y, &blank, row_style);

                    let mut col = inner.x;
                    if is_selected {
                        buf.set_string(
                            col,
                            row_y,
                            " \u{25b8} ",
                            prompt.bg(row_selected.bg.unwrap_or(theme.semantic().selection_bg)),
                        );
                    } else {
                        buf.set_string(col, row_y, "   ", row_style);
                    }
                    col += 3;

                    let text = format!("connect without saving  {}", adhoc.label());
                    let text_style = if is_selected { row_selected } else { prompt };
                    let avail = w.saturating_sub(3);
                    let disp = crate::tui::text::pad_ellipsize(&text, avail);
                    buf.set_string(col, row_y, &disp, text_style);
                }
            }
        }
    }

    // ── separator before detail block ───────────────────────
    let detail_sep_y = list_start_y.saturating_add(MAX_VISIBLE_ROWS as u16);
    if detail_sep_y < inner.bottom() {
        let line = "\u{2500}".repeat(w);
        buf.set_string(inner.x, detail_sep_y, &line, separator);
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
        render_detail_kv(buf, inner.x, detail_y, half, "host", &host_addr, detail);
        render_detail_kv(
            buf,
            inner.x + half,
            detail_y,
            half,
            "user",
            &user_addr,
            detail,
        );

        // Row 1: identity + jump
        if detail_y + 1 < inner.y + inner.height {
            render_detail_kv(
                buf,
                inner.x,
                detail_y + 1,
                half,
                "identity",
                &identity_path,
                detail,
            );
            render_detail_kv(
                buf,
                inner.x + half,
                detail_y + 1,
                half,
                "jump",
                jump_host,
                detail,
            );
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
                detail,
            );
        }
    }

    // ── hint line below the box ─────────────────────────────
    let hint_y = popup_area.y + popup_area.height;
    if hint_y < area.height {
        let hint = " \u{21b5} connect   esc cancel";
        let hint_x = popup_area.x + (popup_area.width.saturating_sub(hint.len() as u16)) / 2;
        buf.set_string(hint_x, hint_y, hint, legend);
    }
}

/// The two roles one `key  value` detail pair is drawn from.
#[derive(Clone, Copy)]
struct DetailStyles {
    key: Style,
    value: Style,
}

impl DetailStyles {
    fn of(theme: &ResolvedTheme) -> Self {
        Self {
            key: theme.style(StyleRole::PopupLegend),
            value: theme.style(StyleRole::TextPrimary),
        }
    }
}

/// Render a "key  value" pair at the given position.
fn render_detail_kv(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    max_width: u16,
    key: &str,
    value: &str,
    styles: DetailStyles,
) {
    let label = format!(" {:<10}", key);
    buf.set_string(x, y, &label, styles.key);
    let val_x = x + label.len() as u16;
    let avail = max_width.saturating_sub(label.len() as u16) as usize;
    let truncated = crate::tui::text::ellipsize(value, avail);
    buf.set_string(val_x, y, &truncated, styles.value);
}
