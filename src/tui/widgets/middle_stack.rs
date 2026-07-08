//! Middle column dashboard stack: Agent info, Tunnels summary, Latency sparkline.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::Frame;

use crate::app::App;
use crate::tui::theme;
use crate::tui::widgets::panel_box::{put_clamped, render_panel_box};

// ── Panel heights ───────────────────────────────────────
const AGENT_H: u16 = 6;
const TUNNELS_H: u16 = 8;
const LATENCY_H: u16 = 5;

/// Render the three middle-column panels stacked vertically.
pub fn render_middle_stack(frame: &mut Frame, area: Rect, app: &App) {
    let buf = frame.buffer_mut();

    let mut y = area.y;
    let w = area.width;

    // ── Panel 1: Agent info ─────────────────────────────
    let agent_area = Rect::new(area.x, y, w, AGENT_H.min(area.height));
    render_agent_panel(buf, agent_area, app);
    y += agent_area.height;

    if y >= area.y + area.height {
        return;
    }

    // ── Panel 2: Tunnels ────────────────────────────────
    let remaining = area.y + area.height - y;
    let tunnels_area = Rect::new(area.x, y, w, TUNNELS_H.min(remaining));
    render_tunnels_panel(buf, tunnels_area, app);
    y += tunnels_area.height;

    if y >= area.y + area.height {
        return;
    }

    // ── Panel 3: Latency sparkline ──────────────────────
    let remaining = area.y + area.height - y;
    let latency_area = Rect::new(area.x, y, w, LATENCY_H.min(remaining));
    render_latency_panel(buf, latency_area, app);
}

/// Render the SSH log panel (meant to span both middle + right columns).
pub fn render_ssh_log_panel(frame: &mut Frame, area: Rect, app: &App) {
    let buf = frame.buffer_mut();
    render_ssh_log(buf, area, app);
}

fn render_ssh_log(buf: &mut Buffer, area: Rect, app: &App) {
    // Title reflects the host we're filtering by so it's not just "ssh log".
    let selected_name = app.selected_entry().map(|e| e.name().to_string());
    let title = match selected_name.as_deref() {
        Some(name) => format!("ssh log · {name}"),
        None => "ssh log".to_string(),
    };
    render_panel_box(buf, area, &title, None);
    let inner_x = area.x + 2;
    let inner_w = area.width.saturating_sub(4) as usize;
    let max_rows = area.height.saturating_sub(2) as usize;

    // Show only entries for the currently selected host. Per-host context
    // beats firehose noise.
    let filtered: Vec<&crate::ssh::probe::SshLogEntry> = match selected_name.as_deref() {
        Some(name) => app.ssh_log.iter().filter(|e| e.host_name == name).collect(),
        None => Vec::new(),
    };

    if filtered.is_empty() {
        let placeholder_y = area.y + 1;
        if placeholder_y < area.y + area.height - 1 {
            let msg = match selected_name.as_deref() {
                Some(name) => format!("no events for {name} yet — Enter to connect"),
                None => "select a host to see its log".to_string(),
            };
            put_clamped(buf, inner_x, placeholder_y, &msg, theme::dim(), inner_w);
        }
        return;
    }

    // Scrollable tail view: ssh_log_scroll=0 means show latest, higher = scroll up
    let total = filtered.len();
    let scroll = app.ssh_log_scroll.min(total.saturating_sub(max_rows));
    let end = total.saturating_sub(scroll);
    let start = end.saturating_sub(max_rows);

    // Title badge: scroll position
    if scroll > 0 {
        let badge = format!("↑{scroll}");
        let bx = area.x + area.width.saturating_sub(badge.len() as u16 + 3);
        if bx > area.x + 2 {
            buf.set_string(bx, area.y, &badge, theme::mute());
        }
    }

    for (i, entry) in filtered[start..end].iter().enumerate() {
        let row_y = area.y + 1 + i as u16;
        if row_y >= area.y + area.height - 1 {
            break;
        }

        let style = match entry.level {
            crate::ssh::probe::LogLevel::Error => theme::red(),
            crate::ssh::probe::LogLevel::Success => theme::green(),
            crate::ssh::probe::LogLevel::Info => theme::dim(),
        };

        // Timestamp HH:MM:SS (local timezone) — host prefix omitted since
        // the panel title already names the host we're filtering on.
        let time_str = format!("{} ", crate::tui::format_local_time(entry.timestamp));
        let time_w = time_str.len();
        buf.set_string(inner_x, row_y, &time_str, theme::mute());

        let remaining_w = inner_w.saturating_sub(time_w);
        let display: String = entry.line.chars().take(remaining_w).collect();
        buf.set_string(inner_x + time_w as u16, row_y, &display, style);
    }
}

// ── Agent panel ─────────────────────────────────────────

fn render_agent_panel(buf: &mut Buffer, area: Rect, app: &App) {
    render_panel_box(buf, area, "agent", None);

    let inner_x = area.x + 2;
    let inner_w = area.width.saturating_sub(4) as usize;

    let agent = crate::ssh::agent::detect_agent();

    // Row 1: socket path
    let row1_y = area.y + 1;
    if row1_y < area.y + area.height - 1 {
        buf.set_string(inner_x, row1_y, "socket  ", theme::dim());
        let label_w = 8; // "socket  ".len()
        let val_x = inner_x + label_w as u16;
        let max_path = inner_w.saturating_sub(label_w);
        match &agent.socket_path {
            Some(path) => {
                let display: String = path.chars().take(max_path).collect();
                buf.set_string(val_x, row1_y, &display, theme::text());
            }
            None => {
                buf.set_string(val_x, row1_y, "not found", theme::red());
            }
        }
    }

    // Row 2: keys loaded
    let row2_y = area.y + 2;
    if row2_y < area.y + area.height - 1 {
        buf.set_string(inner_x, row2_y, "keys    ", theme::dim());
        let val_x = inner_x + 8;
        let key_str = format!("{} loaded", agent.keys.len());
        put_clamped(
            buf,
            val_x,
            row2_y,
            &key_str,
            theme::bright(),
            inner_w.saturating_sub(8),
        );
    }

    // Row 3: forward agent hosts count
    let row3_y = area.y + 3;
    if row3_y < area.y + area.height - 1 {
        buf.set_string(inner_x, row3_y, "forward ", theme::dim());
        let val_x = inner_x + 8;
        let fwd_count = app
            .hosts
            .iter()
            .filter(|h| match h {
                crate::app::HostEntry::Managed(m) => m.forward_agent,
                crate::app::HostEntry::Legacy { host, .. } => host.forward_agent.unwrap_or(false),
            })
            .count();
        let fwd_str = format!("{} hosts", fwd_count);
        put_clamped(
            buf,
            val_x,
            row3_y,
            &fwd_str,
            theme::bright(),
            inner_w.saturating_sub(8),
        );
    }

    // Row 4: config path
    let row4_y = area.y + 4;
    if row4_y < area.y + area.height - 1 {
        buf.set_string(inner_x, row4_y, "config  ", theme::dim());
        let val_x = inner_x + 8;
        put_clamped(
            buf,
            val_x,
            row4_y,
            "~/.ssh/config",
            theme::text(),
            inner_w.saturating_sub(8),
        );
    }
}

// ── Tunnels panel ───────────────────────────────────────

fn render_tunnels_panel(buf: &mut Buffer, area: Rect, app: &App) {
    let active = app.tunnel_manager.active_count();
    let total = app.tunnels.len();
    let badge = if total > 0 {
        Some(format!("{active}/{total}"))
    } else {
        None
    };
    render_panel_box(buf, area, "tunnels", badge.as_deref());

    let inner_x = area.x + 2;
    let inner_w = area.width.saturating_sub(4) as usize;
    let max_rows = area.height.saturating_sub(2) as usize;

    if app.tunnels.is_empty() {
        let y = area.y + 1;
        if y < area.y + area.height - 1 {
            put_clamped(
                buf,
                inner_x,
                y,
                "press 2 for tunnels tab",
                theme::dim(),
                inner_w,
            );
        }
        return;
    }

    for (i, tunnel) in app.tunnels.iter().take(max_rows).enumerate() {
        let y = area.y + 1 + i as u16;
        if y >= area.y + area.height - 1 {
            break;
        }

        let running = app.tunnel_manager.is_running(tunnel.id);
        let (dot, dot_color) = if running {
            ("\u{25cf}", theme::GREEN)
        } else {
            ("\u{25cb}", theme::DIM)
        };
        buf.set_string(
            inner_x,
            y,
            dot,
            ratatui::style::Style::default().fg(dot_color),
        );

        let dir = match tunnel.tunnel_type {
            crate::store::TunnelType::Local => "L",
            crate::store::TunnelType::Remote => "R",
            crate::store::TunnelType::Dynamic => "D",
        };
        buf.set_string(inner_x + 2, y, dir, theme::cyan());

        let label = tunnel.label.as_deref().unwrap_or("");
        let port_str = format!(":{}", tunnel.local_port);
        let desc = if label.is_empty() {
            port_str.clone()
        } else {
            format!("{} {}", port_str, label)
        };
        let max_desc = inner_w.saturating_sub(4);
        let truncated: String = desc.chars().take(max_desc).collect();
        buf.set_string(
            inner_x + 4,
            y,
            &truncated,
            if running { theme::text() } else { theme::dim() },
        );
    }
}

// ── Latency sparkline panel ─────────────────────────────

/// Sparkline block characters, ordered lowest to highest.
const SPARK_CHARS: [char; 8] = [
    '\u{2581}', '\u{2582}', '\u{2583}', '\u{2584}', '\u{2585}', '\u{2586}', '\u{2587}', '\u{2588}',
];

fn render_latency_panel(buf: &mut Buffer, area: Rect, app: &App) {
    render_panel_box(buf, area, "latency p50", None);

    let inner_x = area.x + 2;
    let inner_w = area.width.saturating_sub(4) as usize;

    // Collect all ping samples into a combined timeline
    let all_samples: Vec<u32> = app
        .ping_data
        .values()
        .flat_map(|v| {
            v.iter()
                .copied()
                .filter(|ms| !crate::ping::is_unreachable(*ms))
        })
        .collect();

    if all_samples.is_empty() {
        // Empty sparkline — flat baseline
        let spark_y = area.y + 1;
        if spark_y < area.y + area.height - 1 {
            let baseline: String = "\u{2581}".repeat(inner_w.min(20));
            buf.set_string(inner_x, spark_y, &baseline, theme::dim());
        }
        let info_y = area.y + 2;
        if info_y < area.y + area.height - 1 {
            put_clamped(
                buf,
                inner_x,
                info_y,
                "no latency data",
                theme::dim(),
                inner_w,
            );
        }
        return;
    }

    // Compute stats
    let mut sorted = all_samples.clone();
    sorted.sort_unstable();
    let p50 = sorted[sorted.len() / 2];
    let peak = *sorted.last().unwrap_or(&0);
    let now_val = *all_samples.last().unwrap_or(&0);

    // Build sparkline from last 30 combined samples (take tail from each host interleaved)
    let spark_data: Vec<u32> = {
        let mut combined: Vec<u32> = Vec::new();
        for samples in app.ping_data.values() {
            let start = samples.len().saturating_sub(30);
            for &v in &samples[start..] {
                if !crate::ping::is_unreachable(v) {
                    combined.push(v);
                }
            }
        }
        let start = combined.len().saturating_sub(30);
        combined[start..].to_vec()
    };

    let spark_y = area.y + 1;
    if spark_y < area.y + area.height - 1 && !spark_data.is_empty() {
        let max_val = *spark_data.iter().max().unwrap_or(&1);
        let max_val = max_val.max(1);
        let spark_len = spark_data.len().min(inner_w);
        let start = spark_data.len().saturating_sub(spark_len);
        let sparkline: String = spark_data[start..]
            .iter()
            .map(|&v| {
                let idx = ((v as u64 * 7) / max_val as u64).min(7) as usize;
                SPARK_CHARS[idx]
            })
            .collect();
        buf.set_string(inner_x, spark_y, &sparkline, theme::green());
    }

    // Stats row
    let info_y = area.y + 2;
    if info_y < area.y + area.height - 1 {
        let stats = format!("now {}ms  avg {}ms  peak {}ms", now_val, p50, peak);
        put_clamped(buf, inner_x, info_y, &stats, theme::dim(), inner_w);
    }
}
