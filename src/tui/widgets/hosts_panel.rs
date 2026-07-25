//! Hosts panel — grouped host tree for the dashboard left column.

use ratatui::layout::Rect;
use ratatui::Frame;

use crate::app::{App, HostEntry};
use crate::tui::theme;
use crate::tui::tween;
use crate::tui::widgets::panel_box;

/// Fraction of the selected row the highlight has filled, left to right (#35).
/// `1.0` at rest, so a settled cursor draws its full bar. The wipe runs on the
/// background only: the text under it is already styled for the selection, so a
/// moved cursor lands on the right row instantly and the bar catches up.
fn highlight_fill(app: &App) -> f32 {
    if !app.motion_enabled() {
        return 1.0;
    }
    match app.selection_at {
        Some(at) => tween::ease_out(tween::progress(
            at,
            crate::tui::SELECT_ANIM,
            std::time::Instant::now(),
        )),
        None => 1.0,
    }
}

/// Clear the background of the selected row past the point the wipe has
/// reached, leaving the bar filling in from the left edge of the row.
fn clip_highlight(buf: &mut ratatui::buffer::Buffer, x: u16, y: u16, w: usize, fill: f32) {
    if fill >= 1.0 {
        return;
    }
    let filled = (w as f32 * fill).round() as u16;
    for cx in (x + filled)..(x + w as u16) {
        if let Some(c) = buf.cell_mut((cx, y)) {
            c.bg = ratatui::style::Color::Reset;
        }
    }
}

/// Render the hosts panel into the left column of the dashboard bento grid.
///
/// Draws a panel box with title "hosts" and the total host count, then renders
/// group headers and host rows inside the bordered area.
pub fn render_hosts_panel(frame: &mut Frame, area: Rect, app: &App) {
    let buf = frame.buffer_mut();

    // Total host count for the panel badge.
    let count_str = app.filtered_indices.len().to_string();
    // Surface active tag filters in the panel title so they stay visible once
    // the picker popup closes.
    let raw_title = if app.tag_filters.is_empty() {
        "hosts".to_string()
    } else {
        // Spell out the AND operator so an empty result set is self-explaining.
        let joined = app
            .tag_filters
            .iter()
            .map(|t| format!("#{t}"))
            .collect::<Vec<_>>()
            .join(" AND ");
        format!("hosts · {joined}")
    };
    let max_title = (area.width as usize).saturating_sub(12).max(5);
    let title = crate::tui::text::ellipsize(&raw_title, max_title);
    panel_box::render_panel_box(
        buf,
        area,
        &title,
        Some(&count_str),
        app.focused_panel == crate::app::PanelId::Hosts,
    );

    // Content area inside the panel borders: x+2, y+1, width-4, height-2
    if area.width < 6 || area.height < 4 {
        return;
    }
    let cx = area.x + 2;
    let cy = area.y + 1;
    let cw = (area.width - 4) as usize;
    let ch = (area.height - 2) as usize;

    // Reserve 2 lines at the bottom for footer (divider + add action).
    let body_h = if ch > 2 { ch - 2 } else { ch };

    // The host tree is taller than the panel once enough hosts are imported,
    // so scroll to keep the selection on screen (roughly centered). `vrow` is
    // the flattened visual-row index, counting group headers and blank
    // separators alongside host rows. The offset math lives on `App` so click
    // mapping stays in sync with what is drawn.
    // Empty state: tell a first-time user how to get hosts in.
    if app.hosts.is_empty() {
        let lines = [
            "No hosts yet.",
            "",
            "a        add a host",
            "Shift+I  import ~/.ssh/config",
            "Shift+T  import Termius export",
        ];
        for (i, line) in lines.iter().enumerate() {
            let y = cy + 1 + i as u16;
            if (y as usize) < cy as usize + ch {
                buf.set_string(cx, y, crate::tui::text::ellipsize(line, cw), theme::mute());
            }
        }
        return;
    }

    let offset = app.host_scroll_advance(body_h);
    let window_end = offset + body_h;

    use crate::app::VisualRow;
    let visual = app.host_visual_rows();
    let fill = highlight_fill(app);

    for (vrow, row) in visual.iter().enumerate() {
        if vrow < offset || vrow >= window_end {
            continue;
        }
        let y = cy + (vrow - offset) as u16;
        let selected_row = matches!(
            *row,
            VisualRow::Header { selected: true, .. } | VisualRow::Host { selected: true, .. }
        );

        match *row {
            VisualRow::Blank => {}
            VisualRow::Header {
                section,
                collapsed,
                selected,
                depth,
            } => {
                let section = &app.group_sections[section];
                let arrow = if collapsed { "\u{25b8}" } else { "\u{25be}" }; // ▸ / ▾
                let host_count = section.host_indices.len();
                let count_suffix = format!("({})", host_count);
                let label = &section.label;
                // Indent nested groups by two columns per level.
                let mut col = cx + (depth as u16) * 2;

                if selected {
                    let blank = " ".repeat(cw);
                    buf.set_string(cx, y, &blank, theme::selected());
                }
                let (arrow_style, label_style) = if selected {
                    (
                        theme::white().bg(theme::SEL_BG),
                        theme::white().bg(theme::SEL_BG),
                    )
                } else {
                    (theme::mute(), theme::white())
                };
                let mute_bg = if selected {
                    theme::mute().bg(theme::SEL_BG)
                } else {
                    theme::mute()
                };

                buf.set_string(col, y, arrow, arrow_style);
                col += 2; // arrow + space

                // Right-align the count first and reserve its slot. Filling the
                // dotted leader up to (but not into) that slot means the count
                // always renders, regardless of the label length's parity.
                let count_x = cx + (cw as u16).saturating_sub(count_suffix.len() as u16);

                // Label, truncated so it can never run into the count slot
                // (leave at least one cell of gap before the count).
                let name_max = (count_x.saturating_sub(col) as usize).saturating_sub(1);
                let truncated_label: String = label.chars().take(name_max).collect();
                buf.set_string(col, y, &truncated_label, label_style);
                col += truncated_label.chars().count() as u16;

                // Dotted leader fills the gap [col, count_x).
                if count_x > col {
                    let gap = (count_x - col) as usize;
                    let dots: String = " \u{00b7}".repeat(gap.div_ceil(2));
                    let dots_trimmed: String = dots.chars().take(gap).collect();
                    buf.set_string(col, y, &dots_trimmed, mute_bg);
                }
                buf.set_string(count_x, y, &count_suffix, mute_bg);
            }
            VisualRow::Host {
                host_idx,
                selected: is_selected,
                depth,
            } => {
                let entry = &app.hosts[host_idx];

                // If selected, fill the entire row with SEL_BG.
                if is_selected {
                    let blank = " ".repeat(cw);
                    buf.set_string(cx, y, &blank, theme::selected());
                }

                // Indent hosts under their group; two cols per nesting level
                // (min one col so a flat ssh_config list still indents slightly).
                let mut col = cx + ((depth as u16) * 2).max(1);

                // Status dot — reflects ping latency.
                let host_name_for_dot = entry.name();
                let ping_samples = app.ping_data.get(host_name_for_dot).map(|v| v.as_slice());
                let (dot_char, dot_color) = match crate::ping::classify_ping(ping_samples) {
                    crate::ping::PingClass::Online => ("\u{25cf}", theme::GREEN),
                    crate::ping::PingClass::Slow => {
                        let ms = ping_samples.and_then(|s| s.last().copied()).unwrap_or(0);
                        if ms <= 200 {
                            ("\u{25cf}", theme::AMBER)
                        } else {
                            ("\u{25cf}", theme::RED)
                        }
                    }
                    crate::ping::PingClass::Unreachable => ("\u{25cf}", theme::RED),
                    crate::ping::PingClass::Unknown => ("\u{25cb}", theme::DIM),
                };
                let dot_style = if is_selected {
                    ratatui::style::Style::default()
                        .fg(dot_color)
                        .bg(theme::SEL_BG)
                } else {
                    ratatui::style::Style::default().fg(dot_color)
                };
                buf.set_string(col, y, dot_char, dot_style);
                col += 2; // dot + space

                // Base style for text on this row.
                let name_style = if is_selected {
                    theme::selected()
                } else {
                    theme::text()
                };
                let dim_style = if is_selected {
                    ratatui::style::Style::default()
                        .fg(theme::DIM)
                        .bg(theme::SEL_BG)
                } else {
                    theme::dim()
                };

                let inner_right = cx + cw as u16;

                // Name — width driven by the zoom level (+/-), clamped to the panel
                // width so narrow terminals don't bleed into the border/neighbour.
                let name = entry.display_name();
                let name_w = (inner_right.saturating_sub(col) as usize).min(app.name_col_width());
                if name_w > 0 {
                    let name_display = crate::tui::text::pad_ellipsize(name, name_w);
                    buf.set_string(col, y, &name_display, name_style);
                    col += name_w as u16 + 1; // + gap
                }

                // Favorite star. A fixed 2-col slot is reserved on every row so
                // addresses stay aligned whether or not the host is a favorite.
                if col < inner_right {
                    if entry.favorite() {
                        let star_style = if is_selected {
                            ratatui::style::Style::default()
                                .fg(theme::AMBER)
                                .bg(theme::SEL_BG)
                        } else {
                            theme::amber()
                        };
                        buf.set_string(col, y, "\u{2605}", star_style);
                    }
                    col += 2;
                }

                // Address — up to 14 chars, only if it still fits.
                let addr = host_address(entry);
                let addr_w = (inner_right.saturating_sub(col) as usize).min(14);
                if addr_w >= 4 {
                    let addr_display = crate::tui::text::pad_ellipsize(&addr, addr_w);
                    buf.set_string(col, y, &addr_display, dim_style);
                    col += addr_w as u16 + 1; // + gap
                }

                // Ping value — right-aligned in 6 chars at the right edge.
                let ping_width: u16 = 6;
                let right_edge = cx + cw as u16;
                if right_edge >= col + ping_width {
                    let ping_x = right_edge - ping_width;
                    let host_name = entry.name();
                    let ping_samples = app.ping_data.get(host_name).map(|v| v.as_slice());
                    let (ping_str, ping_style) = match crate::ping::classify_ping(ping_samples) {
                        crate::ping::PingClass::Online | crate::ping::PingClass::Slow => {
                            let latest = ping_samples.and_then(|s| s.last().copied()).unwrap_or(0);
                            let s = format!(
                                "{:>width$}",
                                format!("{}ms", latest),
                                width = ping_width as usize
                            );
                            let style = if latest < 100 {
                                dim_style
                            } else if latest <= 200 {
                                if is_selected {
                                    ratatui::style::Style::default()
                                        .fg(theme::AMBER)
                                        .bg(theme::SEL_BG)
                                } else {
                                    theme::amber()
                                }
                            } else {
                                if is_selected {
                                    ratatui::style::Style::default()
                                        .fg(theme::RED)
                                        .bg(theme::SEL_BG)
                                } else {
                                    theme::red()
                                }
                            };
                            (s, style)
                        }
                        crate::ping::PingClass::Unreachable | crate::ping::PingClass::Unknown => (
                            format!("{:>width$}", "\u{2014}", width = ping_width as usize),
                            dim_style,
                        ),
                    };
                    buf.set_string(ping_x, y, &ping_str, ping_style);
                }
            }
        }

        // The highlight bar fills in from the left under a moved cursor; done
        // after the row is drawn so it clips the row's own styling too.
        if selected_row {
            clip_highlight(buf, cx, y, cw, fill);
        }
    }

    // ── Footer ───────────────────────────────────────────
    if ch >= 2 {
        let footer_y = cy + (ch - 2) as u16;

        // Dotted divider line.
        let dots: String = "\u{00b7} ".repeat(cw / 2);
        let dots_trimmed: String = dots.chars().take(cw).collect();
        buf.set_string(cx, footer_y, &dots_trimmed, theme::mute());

        // "+ add a new host" action.
        let action = "+ add a new host";
        buf.set_string(cx, footer_y + 1, action, theme::dim());
    }
}

/// Extract a display address from a host entry.
fn host_address(entry: &HostEntry) -> String {
    match entry {
        HostEntry::Managed(m) => {
            if m.address.is_empty() {
                m.name.clone()
            } else {
                m.address.clone()
            }
        }
        HostEntry::Legacy { host, .. } => {
            host.hostname.clone().unwrap_or_else(|| host.name.clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::style::Color;

    fn filled_row(fill: f32) -> Vec<Color> {
        let area = Rect::new(0, 0, 8, 1);
        let mut buf = Buffer::empty(area);
        for x in 0..8 {
            buf.cell_mut((x, 0)).unwrap().bg = theme::SEL_BG;
        }
        clip_highlight(&mut buf, 0, 0, 8, fill);
        (0..8).map(|x| buf.cell((x, 0)).unwrap().bg).collect()
    }

    #[test]
    fn highlight_at_rest_keeps_the_whole_bar() {
        assert!(filled_row(1.0).iter().all(|c| *c == theme::SEL_BG));
    }

    #[test]
    fn highlight_fills_from_the_left() {
        let bgs = filled_row(0.5);
        assert!(
            bgs[..4].iter().all(|c| *c == theme::SEL_BG),
            "left half stays filled: {bgs:?}"
        );
        assert!(
            bgs[4..].iter().all(|c| *c == Color::Reset),
            "right half is cleared: {bgs:?}"
        );
    }

    #[test]
    fn highlight_at_zero_clears_the_row() {
        assert!(filled_row(0.0).iter().all(|c| *c == Color::Reset));
    }
}
