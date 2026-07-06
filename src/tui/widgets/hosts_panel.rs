//! Hosts panel — grouped host tree for the dashboard left column.

use ratatui::layout::Rect;
use ratatui::Frame;

use crate::app::{App, HostEntry};
use crate::tui::theme;
use crate::tui::widgets::panel_box;

/// Render the hosts panel into the left column of the dashboard bento grid.
///
/// Draws a panel box with title "hosts" and the total host count, then renders
/// group headers and host rows inside the bordered area.
pub fn render_hosts_panel(frame: &mut Frame, area: Rect, app: &App) {
    let buf = frame.buffer_mut();

    // Total host count for the panel badge.
    let count_str = app.filtered_indices.len().to_string();
    panel_box::render_panel_box(buf, area, "hosts", Some(&count_str));

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

    let offset = app.host_scroll_offset(body_h);
    let window_end = offset + body_h;

    use crate::app::VisualRow;
    let visual = app.host_visual_rows();

    for (vrow, row) in visual.iter().enumerate() {
        if vrow < offset || vrow >= window_end {
            continue;
        }
        let y = cy + (vrow - offset) as u16;

        match *row {
            VisualRow::Blank => {}
            VisualRow::Header {
                section,
                collapsed,
                selected,
            } => {
                let section = &app.group_sections[section];
                let arrow = if collapsed { "\u{25b8}" } else { "\u{25be}" }; // ▸ / ▾
                let host_count = section.host_indices.len();
                let count_suffix = format!("({})", host_count);
                let label = &section.label;
                let mut col = cx;

                if selected {
                    let blank = " ".repeat(cw);
                    buf.set_string(cx, y, &blank, theme::selected());
                }
                let (arrow_style, label_style) = if selected {
                    (theme::white().bg(theme::SEL_BG), theme::white().bg(theme::SEL_BG))
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

                let name_max = cw.saturating_sub(6 + count_suffix.len());
                let truncated_label: String = label.chars().take(name_max).collect();
                buf.set_string(col, y, &truncated_label, label_style);
                col += truncated_label.chars().count() as u16;

                let used = (col - cx) as usize;
                let remaining = cw.saturating_sub(used + 1 + count_suffix.len());
                if remaining > 2 {
                    buf.set_string(col, y, " ", mute_bg);
                    col += 1;
                    let dots: String = " \u{00b7}".repeat(remaining / 2);
                    let dots_trimmed: String = dots.chars().take(remaining).collect();
                    buf.set_string(col, y, &dots_trimmed, mute_bg);
                    col += dots_trimmed.chars().count() as u16;
                }
                let count_x = cx + (cw as u16).saturating_sub(count_suffix.len() as u16);
                if count_x > col {
                    buf.set_string(count_x, y, &count_suffix, mute_bg);
                }
            }
            VisualRow::Host {
                host_idx,
                selected: is_selected,
            } => {
                let entry = &app.hosts[host_idx];

                // If selected, fill the entire row with SEL_BG.
            if is_selected {
                let blank = " ".repeat(cw);
                buf.set_string(cx, y, &blank, theme::selected());
            }

            let mut col = cx + 1; // indent host rows by 1

            // Status dot — reflects ping latency.
            let host_name_for_dot = entry.name();
            let (dot_char, dot_color) = match app.ping_data.get(host_name_for_dot) {
                Some(samples) if !samples.is_empty() => {
                    let latest = *samples.last().unwrap();
                    if latest < 100 {
                        ("\u{25cf}", theme::GREEN) // ● green
                    } else if latest <= 200 {
                        ("\u{25cf}", theme::AMBER) // ● amber
                    } else {
                        ("\u{25cf}", theme::RED) // ● red
                    }
                }
                _ => ("\u{25cb}", theme::DIM), // ○ dim (no data)
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

            // Name — up to 14 chars, clamped to the panel width so narrow
            // terminals don't bleed into the border/neighbouring column.
            let name = entry.display_name();
            let name_w = (inner_right.saturating_sub(col) as usize).min(14);
            if name_w > 0 {
                let name_display = crate::tui::text::pad_ellipsize(name, name_w);
                buf.set_string(col, y, &name_display, name_style);
                col += name_w as u16 + 1; // + gap
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
                let (ping_str, ping_style) = match app.ping_data.get(host_name) {
                    Some(samples) if !samples.is_empty() => {
                        let latest = *samples.last().unwrap();
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
                    _ => {
                        // No ping data yet — show dash, right-aligned
                        (
                            format!("{:>width$}", "\u{2014}", width = ping_width as usize),
                            dim_style,
                        )
                    }
                };
                buf.set_string(ping_x, y, &ping_str, ping_style);
            }
            }
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

