//! Right column dashboard stack: Recent sessions, Auth events sparkline, Ping all hosts.

use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::Frame;

use crate::app::App;
use crate::tui::theme;
use crate::tui::widgets::panel_box::{put_clamped, render_panel_box};

// ── Panel heights ───────────────────────────────────────
pub const RECENT_H: u16 = 8;
pub const AUTH_H: u16 = 6;
pub const PING_H: u16 = 5;

/// Render the three right-column panels stacked vertically.
pub fn render_right_stack(frame: &mut Frame, area: Rect, app: &App) {
    let buf = frame.buffer_mut();

    let mut y = area.y;
    let w = area.width;

    // ── Panel 1: Recent sessions ────────────────────────
    let recent_area = Rect::new(area.x, y, w, RECENT_H.min(area.height));
    render_recent_panel(buf, recent_area, app);
    y += recent_area.height;

    if y >= area.y + area.height {
        return;
    }

    // ── Panel 2: Auth events ────────────────────────────
    let remaining = area.y + area.height - y;
    let auth_area = Rect::new(area.x, y, w, AUTH_H.min(remaining));
    render_auth_panel(buf, auth_area, app);
    y += auth_area.height;

    if y >= area.y + area.height {
        return;
    }

    // ── Panel 3: Ping all hosts ─────────────────────────
    let remaining = area.y + area.height - y;
    let ping_area = Rect::new(area.x, y, w, PING_H.min(remaining));
    render_ping_panel(buf, ping_area, app);
}

// ── Recent sessions panel ───────────────────────────────

/// Format a unix timestamp relative to now, e.g. "just now", "3m", "2h", "5d".
pub fn format_relative_time(timestamp: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let diff = (now - timestamp).max(0);
    if diff < 60 {
        "just now".to_string()
    } else if diff < 3600 {
        format!("{}m", diff / 60)
    } else if diff < 86400 {
        format!("{}h", diff / 3600)
    } else {
        format!("{}d", diff / 86400)
    }
}

pub(crate) fn render_recent_panel(buf: &mut Buffer, area: Rect, app: &App) {
    render_panel_box(
        buf,
        area,
        "recent sessions",
        None,
        app.focused_panel == crate::app::PanelId::Recent,
    );

    let inner_x = area.x + 2;
    let inner_w = area.width.saturating_sub(4) as usize;

    // Collect hosts with last_connected, sort descending, take top entries.
    let max_rows = (area.height.saturating_sub(3)) as usize;
    // Height-driven so a zoomed panel (issue #18) can show every recent
    // session; at the default panel height this still yields ~5 rows.
    let max_display = max_rows;

    // Keep each recent's `app.hosts` index alongside its display fields so a
    // zoomed, selectable list (issue #18) can connect the highlighted row.
    let mut recents: Vec<(usize, &str, i64)> = app
        .hosts
        .iter()
        .enumerate()
        .filter_map(|(i, h)| h.last_connected().map(|ts| (i, h.display_name(), ts)))
        .collect();
    recents.sort_by_key(|&(_, _, ts)| std::cmp::Reverse(ts));

    // Zoomed (issue #18): `panel_scroll` holds the selected row index; the view
    // follows it and Enter connects the highlighted host. Compact stack: no
    // selection, no offset, and `zoomed_host_idx` is left untouched.
    let (off, sel) = if app.panel_zoomed && app.focused_panel == crate::app::PanelId::Recent {
        let (first, sel) =
            crate::tui::widgets::panel_box::zoom_window(app, recents.len(), max_display);
        *app.zoomed_host_idx.borrow_mut() = recents.iter().map(|(i, _, _)| *i).collect();
        (first, Some(sel))
    } else {
        (0usize, None)
    };
    let row_count = recents.len().saturating_sub(off).min(max_display);

    if recents.is_empty() {
        let y = area.y + 1;
        if y < area.y + area.height - 1 {
            put_clamped(buf, inner_x, y, "no sessions yet", theme::dim(), inner_w);
        }
    } else {
        let name_max = (area.width.saturating_sub(12)) as usize;
        for (di, (_i, host, ts)) in recents.iter().enumerate().skip(off).take(max_display) {
            let y = area.y + 1 + (di - off) as u16;
            if y >= area.y + area.height - 1 {
                break;
            }

            let mut col = inner_x;

            // ↺ icon in CYAN
            buf.set_string(col, y, "\u{21ba}", theme::cyan());
            col += 2;

            // host name (truncated)
            let display: String = host.chars().take(name_max).collect();
            buf.set_string(col, y, &display, theme::text());

            // age — right-aligned in MUTE; skip when the column is too narrow
            let age = format_relative_time(*ts);
            let needed = 2 + age.len() as u16;
            if area.width > needed {
                let age_col = area.x + area.width - needed;
                if age_col > col {
                    buf.set_string(age_col, y, &age, theme::mute());
                }
            }

            // Highlight the selected row when zoomed (issue #18).
            if Some(di) == sel {
                for col in inner_x..(inner_x + inner_w as u16) {
                    if let Some(cell) = buf.cell_mut((col, y)) {
                        cell.modifier.insert(ratatui::style::Modifier::REVERSED);
                    }
                }
            }
        }
    }

    // Action row — after session rows, with a blank line gap
    let action_y = area.y + 1 + row_count.max(1) as u16 + 1;
    if action_y < area.y + area.height - 1 {
        put_clamped(
            buf,
            inner_x,
            action_y,
            "show full audit log \u{2192}",
            theme::dim(),
            inner_w,
        );
    }
}

// ── Auth events sparkline panel ─────────────────────────

pub(crate) fn render_auth_panel(buf: &mut Buffer, area: Rect, app: &App) {
    render_panel_box(
        buf,
        area,
        "auth events",
        None,
        app.focused_panel == crate::app::PanelId::Auth,
    );

    let inner_x = area.x + 2;
    let inner_w = area.width.saturating_sub(4) as usize;
    let (ok, fail) = app.auth_stats_cache;
    let total = ok + fail;

    if total == 0 && app.auth_events_cache.is_empty() {
        let y = area.y + 1;
        if y < area.y + area.height - 1 {
            put_clamped(buf, inner_x, y, "no audit data", theme::dim(), inner_w);
        }
        return;
    }

    // Summary line: ● ok N  ● failed N  rate X%. Guard every write against the
    // inner right edge so a narrow (zoomed) column can't spill past the border.
    let right_lim = inner_x + inner_w as u16;
    let summary_y = area.y + 1;
    if summary_y < area.y + area.height - 1 {
        let mut col = inner_x;
        let mut put = |buf: &mut Buffer, s: &str, style| {
            if col < right_lim {
                col += put_clamped(buf, col, summary_y, s, style, (right_lim - col) as usize);
            }
        };

        put(buf, "\u{25cf} ", theme::green());
        put(buf, &format!("ok {}  ", ok), theme::text());
        put(buf, "\u{25cf} ", theme::red());
        put(buf, &format!("failed {}  ", fail), theme::text());
        let rate_str = if total > 0 {
            format!("rate {}%", ok * 100 / total)
        } else {
            "rate \u{2014}".to_string()
        };
        put(buf, &rate_str, theme::mute());
    }

    // Mini log: last 2-3 events
    // Height-driven so a zoomed panel (issue #18) can show more; at the default
    // panel height this still yields ~3 rows.
    let max_events = (area.height.saturating_sub(3)) as usize;
    let name_max = (area.width.saturating_sub(18)) as usize;
    let (off, sel) = if app.panel_zoomed && app.focused_panel == crate::app::PanelId::Auth {
        let (first, s) = crate::tui::widgets::panel_box::zoom_window(
            app,
            app.auth_events_cache.len(),
            max_events,
        );
        (first, Some(s))
    } else {
        (0, None)
    };
    for (di, ev) in app
        .auth_events_cache
        .iter()
        .enumerate()
        .skip(off)
        .take(max_events)
    {
        let y = area.y + 2 + (di - off) as u16;
        if y >= area.y + area.height - 1 {
            break;
        }
        let age = format_relative_time(ev.created_at);
        let host: String = ev.host_name.chars().take(name_max).collect();
        let status_style = ratatui::style::Style::default().fg(theme::status_color(&ev.status));
        let mut col = inner_x;
        let age_display = format!("{:>6} ", age);
        buf.set_string(col, y, &age_display, theme::mute());
        col += age_display.len() as u16;
        if col < right_lim {
            col += put_clamped(
                buf,
                col,
                y,
                &host,
                theme::text(),
                (right_lim - col) as usize,
            );
            col += 1;
        }
        if col < right_lim {
            put_clamped(
                buf,
                col,
                y,
                &ev.status,
                status_style,
                (right_lim - col) as usize,
            );
        }
        if Some(di) == sel {
            for col in inner_x..(inner_x + inner_w as u16) {
                if let Some(cell) = buf.cell_mut((col, y)) {
                    cell.modifier.insert(ratatui::style::Modifier::REVERSED);
                }
            }
        }
    }
}

// ── Ping all hosts panel ────────────────────────────────

/// Sparkline block characters from lowest to highest.
const SPARK_CHARS: [char; 8] = [
    '\u{2581}', '\u{2582}', '\u{2583}', '\u{2584}', '\u{2585}', '\u{2586}', '\u{2587}', '\u{2588}',
];

pub(crate) fn render_ping_panel(buf: &mut Buffer, area: Rect, app: &App) {
    render_panel_box(
        buf,
        area,
        "ping all hosts",
        None,
        app.focused_panel == crate::app::PanelId::Ping,
    );

    let inner_x = area.x + 2;
    let inner_w = area.width.saturating_sub(4) as usize;

    // Zoomed (issue #18): take over the whole dashboard body with a per-host
    // table instead of the single aggregate sparkline. The non-zoomed path
    // below is left byte-for-byte unchanged.
    if app.panel_zoomed && app.focused_panel == crate::app::PanelId::Ping {
        render_ping_zoomed(buf, area, app, inner_x, inner_w);
        return;
    }

    if app.ping_data.is_empty() {
        let y = area.y + 1;
        if y < area.y + area.height - 1 {
            let baseline: String = "\u{2581}".repeat(inner_w.min(20));
            buf.set_string(inner_x, y, &baseline, theme::dim());
        }
        let info_y = area.y + 2;
        if info_y < area.y + area.height - 1 {
            put_clamped(
                buf,
                inner_x,
                info_y,
                "waiting for ping data...",
                theme::dim(),
                inner_w,
            );
        }
        return;
    }

    // Aggregate: for each time slot, average all hosts' ping values.
    // Find the max number of samples across all hosts.
    let max_len = app.ping_data.values().map(|v| v.len()).max().unwrap_or(0);
    let slots = inner_w.min(20).min(max_len);

    let mut averages: Vec<u32> = Vec::with_capacity(slots);
    let mut all_min: u32 = u32::MAX;
    let mut all_max: u32 = 0;
    let mut total_samples: u64 = 0;
    let mut loss_count: u64 = 0;

    for i in 0..slots {
        let mut sum: u64 = 0;
        let mut count: u64 = 0;
        for samples in app.ping_data.values() {
            // Align from the end so latest data lines up
            let offset = if samples.len() > slots {
                samples.len() - slots
            } else {
                0
            };
            let idx = offset + i;
            if idx < samples.len() {
                let v = samples[idx];
                total_samples += 1;
                if crate::ping::is_unreachable(v) || v == 0 {
                    loss_count += 1;
                } else {
                    sum += v as u64;
                    count += 1;
                    if v < all_min {
                        all_min = v;
                    }
                    if v > all_max {
                        all_max = v;
                    }
                }
            }
        }
        let avg = sum.checked_div(count).unwrap_or(0) as u32;
        averages.push(avg);
    }

    // Render sparkline
    let spark_y = area.y + 1;
    if spark_y < area.y + area.height - 1 && !averages.is_empty() {
        let spark_max = averages.iter().copied().max().unwrap_or(1).max(1);
        let spark: String = averages
            .iter()
            .map(|&v| {
                if v == 0 {
                    SPARK_CHARS[0]
                } else {
                    let idx = ((v as u64 * 7) / u64::from(spark_max).max(1)).clamp(0, 7) as usize;
                    SPARK_CHARS[idx]
                }
            })
            .collect();
        buf.set_string(inner_x, spark_y, &spark, theme::cyan());
    }

    // Stats line: min Xms  max Xms  loss X%
    let info_y = area.y + 2;
    if info_y < area.y + area.height - 1 {
        let loss_pct = if total_samples > 0 {
            (loss_count * 100) / total_samples.max(1)
        } else {
            0
        };
        let min_str = if all_min == u32::MAX {
            "—".to_string()
        } else {
            format!("{}ms", all_min)
        };
        let max_str = if all_max == 0 {
            "—".to_string()
        } else {
            format!("{}ms", all_max)
        };
        let stats = format!("min {}  max {}  loss {}%", min_str, max_str, loss_pct);
        put_clamped(buf, inner_x, info_y, &stats, theme::mute(), inner_w);
    }
}

/// Colour a latency (ms) green/amber/red by how healthy it looks.
fn ping_latency_style(v: u32) -> ratatui::style::Style {
    if v < 80 {
        theme::green()
    } else if v < 200 {
        theme::amber()
    } else {
        theme::red()
    }
}

/// Zoomed ping view: one row per host (sorted by name), height-driven, laid out
/// as a table — name │ current latency │ per-host sparkline │ min/max/loss.
fn render_ping_zoomed(buf: &mut Buffer, area: Rect, app: &App, inner_x: u16, inner_w: usize) {
    let bottom = area.y + area.height - 1; // first row occupied by the border
    let right_lim = inner_x + inner_w as u16;

    // Empty state — mirror the non-zoomed copy so the panel never looks broken.
    if app.ping_data.is_empty() {
        let y = area.y + 1;
        if y < bottom {
            put_clamped(
                buf,
                inner_x,
                y,
                "waiting for ping data...",
                theme::dim(),
                inner_w,
            );
        }
        return;
    }

    // Sort hosts by name for a stable, scannable table.
    let mut hosts: Vec<(&String, &Vec<u32>)> = app.ping_data.iter().collect();
    hosts.sort_by(|a, b| a.0.cmp(b.0));

    // Column layout (from the right): stats, sparkline, current latency; the
    // name column takes whatever is left. Shrink gracefully on narrow widths.
    let stats_w = 24usize.min(inner_w);
    let spark_w = 16usize.min(inner_w.saturating_sub(stats_w));
    let ms_w = 9usize.min(inner_w.saturating_sub(stats_w + spark_w));
    let gaps = 3usize; // one space between each of the four columns
    let name_w = inner_w
        .saturating_sub(stats_w + spark_w + ms_w + gaps)
        .max(1);

    let name_x = inner_x;
    let ms_x = name_x + name_w as u16 + 1;
    let spark_x = ms_x + ms_w as u16 + 1;
    let stats_x = spark_x + spark_w as u16 + 1;

    // Header row.
    let header_y = area.y + 1;
    if header_y < bottom {
        put_clamped(buf, name_x, header_y, "host", theme::mute(), name_w);
        if ms_w > 0 {
            put_clamped(buf, ms_x, header_y, "now", theme::mute(), ms_w);
        }
        if spark_w > 0 {
            put_clamped(buf, spark_x, header_y, "recent", theme::mute(), spark_w);
        }
        if stats_w > 0 && stats_x < right_lim {
            put_clamped(
                buf,
                stats_x,
                header_y,
                "min / max / loss",
                theme::mute(),
                (right_lim - stats_x) as usize,
            );
        }
    }

    // One row per host, as many as fit under the header. Selectable + scrollable
    // (issue #18): `panel_scroll` holds the selected row index and the view
    // follows it.
    let visible = bottom.saturating_sub(area.y + 2) as usize;
    let (first, sel) = crate::tui::widgets::panel_box::zoom_window(app, hosts.len(), visible);
    // Map each display row (in order) back to its `app.hosts` index so Enter can
    // connect the selected row.
    *app.zoomed_host_idx.borrow_mut() = hosts
        .iter()
        .map(|(name, _)| {
            app.hosts
                .iter()
                .position(|h| h.name() == name.as_str())
                .unwrap_or(usize::MAX)
        })
        .collect();
    for (di, (name, samples)) in hosts.iter().enumerate().skip(first).take(visible) {
        let y = area.y + 2 + (di - first) as u16;
        if y >= bottom {
            break;
        }

        // Aggregate this host's samples.
        let mut h_min: u32 = u32::MAX;
        let mut h_max: u32 = 0;
        let mut loss: u64 = 0;
        let mut total: u64 = 0;
        for &v in samples.iter() {
            total += 1;
            if crate::ping::is_unreachable(v) || v == 0 {
                loss += 1;
            } else {
                if v < h_min {
                    h_min = v;
                }
                if v > h_max {
                    h_max = v;
                }
            }
        }

        // Host name.
        put_clamped(buf, name_x, y, name, theme::text(), name_w);

        // Current latency (last sample), coloured by health.
        if ms_w > 0 {
            let (cur_str, cur_style) = match samples.last().copied() {
                None => ("\u{2014}".to_string(), theme::dim()),
                Some(v) if crate::ping::is_unreachable(v) || v == 0 => {
                    ("timeout".to_string(), theme::red())
                }
                Some(v) => (format!("{}ms", v), ping_latency_style(v)),
            };
            // Right-align inside the ms column.
            let pad = ms_w.saturating_sub(cur_str.chars().count());
            let cur_col = ms_x + pad as u16;
            put_clamped(buf, cur_col, y, &cur_str, cur_style, ms_w - pad);
        }

        // Per-host sparkline over its own most recent samples.
        if spark_w > 0 && spark_x < right_lim {
            let take = spark_w.min(samples.len());
            let recent = &samples[samples.len() - take..];
            let s_max = recent
                .iter()
                .copied()
                .filter(|&v| !(crate::ping::is_unreachable(v) || v == 0))
                .max()
                .unwrap_or(1)
                .max(1);
            let spark: String = recent
                .iter()
                .map(|&v| {
                    if crate::ping::is_unreachable(v) || v == 0 {
                        SPARK_CHARS[0]
                    } else {
                        let idx = ((v as u64 * 7) / u64::from(s_max).max(1)).clamp(0, 7) as usize;
                        SPARK_CHARS[idx]
                    }
                })
                .collect();
            put_clamped(buf, spark_x, y, &spark, theme::cyan(), spark_w);
        }

        // min / max / loss stats.
        if stats_w > 0 && stats_x < right_lim {
            let min_str = if h_min == u32::MAX {
                "\u{2014}".to_string()
            } else {
                format!("{}ms", h_min)
            };
            let max_str = if h_max == 0 {
                "\u{2014}".to_string()
            } else {
                format!("{}ms", h_max)
            };
            let loss_pct = (loss * 100).checked_div(total).unwrap_or(0);
            let stats = format!("{} / {} / {}%", min_str, max_str, loss_pct);
            put_clamped(
                buf,
                stats_x,
                y,
                &stats,
                theme::mute(),
                (right_lim - stats_x) as usize,
            );
        }

        // Highlight the selected row across the panel's inner width (issue #18).
        if di == sel {
            for col in inner_x..right_lim {
                if let Some(cell) = buf.cell_mut((col, y)) {
                    cell.modifier.insert(ratatui::style::Modifier::REVERSED);
                }
            }
        }
    }
}
