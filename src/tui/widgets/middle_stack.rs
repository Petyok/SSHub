//! Middle column dashboard stack: selected-host card, Agent info, Latency.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;
use ratatui::Frame;

use crate::app::App;
use crate::osinfo::widget::{logo_dimensions, OsLogoWidget};
use crate::tui::theme;
use crate::tui::widgets::panel_box::{put_clamped, render_panel_box};

// ── Panel heights (sum = 19 to align with the right column) ─
const HOST_H: u16 = 9;
const AGENT_H: u16 = 6;
#[allow(dead_code)]
const TUNNELS_H: u16 = 8;
const LATENCY_H: u16 = 4;

/// Render the three middle-column panels stacked vertically.
pub fn render_middle_stack(frame: &mut Frame, area: Rect, app: &App) {
    let buf = frame.buffer_mut();

    let mut y = area.y;
    let w = area.width;

    // ── Panel 1: Selected-host card (OS logo + connection) ─
    let host_area = Rect::new(area.x, y, w, HOST_H.min(area.height));
    render_host_panel(buf, host_area, app);
    y += host_area.height;

    if y >= area.y + area.height {
        return;
    }

    // ── Panel 2: Agent info ─────────────────────────────
    let remaining = area.y + area.height - y;
    let agent_area = Rect::new(area.x, y, w, AGENT_H.min(remaining));
    render_agent_panel(buf, agent_area, app);
    y += agent_area.height;

    if y >= area.y + area.height {
        return;
    }

    // ── Panel 3: Latency sparkline ──────────────────────
    let remaining = area.y + area.height - y;
    let latency_area = Rect::new(area.x, y, w, LATENCY_H.min(remaining));
    render_latency_panel(buf, latency_area, app);
}

// ── Selected-host card ──────────────────────────────────

/// Render the selected host's card: its colored OS logo on the left and the
/// name / address / detected OS on the right. The logo is drawn only when the
/// host's `os_icon` resolves to a known distro (auto-detected on first connect
/// or set manually in the form); otherwise the card shows just the text.
fn render_host_panel(buf: &mut Buffer, area: Rect, app: &App) {
    let entry = app.selected_entry();
    let title = match entry.as_ref() {
        Some(e) => format!("host · {}", e.name()),
        None => "host".to_string(),
    };
    render_panel_box(buf, area, &title, None);

    if area.height < 3 || area.width < 6 {
        return;
    }
    let Some(entry) = entry else {
        return;
    };

    let inner_x = area.x + 2;
    let inner_top = area.y + 1;
    let inner_w = area.width.saturating_sub(4);
    let inner_h = area.height.saturating_sub(2);

    // Left: OS logo (when the os_icon resolves to a vendored distro logo).
    let os_id = entry.managed().and_then(|m| m.os_icon.as_deref());
    let logo = os_id.and_then(crate::osinfo::logo_for);
    let mut text_x = inner_x;
    if let Some(logo) = logo {
        let (lw, lh) = logo_dimensions(logo);
        let logo_w = lw.min(inner_w.saturating_sub(1));
        let logo_h = lh.min(inner_h);
        // Vertically center the logo within the card body.
        let pad = (inner_h.saturating_sub(logo_h)) / 2;
        let logo_area = Rect::new(inner_x, inner_top + pad, logo_w, logo_h);
        OsLogoWidget::new(logo).render(logo_area, buf);
        text_x = inner_x + logo_w + 2;
    }

    // Right: a compact fact sheet for the selected host. Guard against the
    // panel height and the right inner edge; skip fields the host doesn't carry.
    if text_x >= inner_x + inner_w {
        return;
    }
    let text_w = (inner_x + inner_w).saturating_sub(text_x) as usize;
    let ssh = entry.ssh_host();
    let addr = ssh
        .hostname
        .clone()
        .unwrap_or_else(|| entry.name().to_string());
    let port = ssh.port.unwrap_or(22);
    let managed = entry.managed();

    let mut rows: Vec<(String, ratatui::style::Style)> = Vec::new();

    // Name (+ favourite star).
    let name = if entry.favorite() {
        format!("{} \u{2605}", entry.name())
    } else {
        entry.name().to_string()
    };
    rows.push((name, theme::bright()));

    // user@host:port (user omitted when unknown).
    let hostport = match ssh.user.as_deref() {
        Some(u) if !u.is_empty() => format!("{}@{}:{}", u, addr, port),
        _ => format!("{}:{}", addr, port),
    };
    rows.push((hostport, theme::text()));

    // OS  ·  latest ping latency (when we have a live sample).
    let latency = app
        .ping_data
        .get(entry.name())
        .and_then(|v| v.last().copied())
        .filter(|&v| v > 0 && !crate::ping::is_unreachable(v));
    let os_line = match (os_id, latency) {
        (Some(os), Some(ms)) => format!("{os}  \u{b7}  {ms}ms"),
        (Some(os), None) => os.to_string(),
        (None, Some(ms)) => format!("\u{b7} {ms}ms"),
        (None, None) => "unknown os".to_string(),
    };
    rows.push((os_line, theme::cyan()));

    // Group / identity / proxy — managed hosts only.
    if let Some(m) = managed {
        if let Some(g) = m.group.as_ref() {
            rows.push((format!("group: {}", g.name), theme::mute()));
        }
        if let Some(id) = m.identity.as_ref() {
            rows.push((format!("key: {}", id.name), theme::mute()));
        }
        if let Some(pj) = m.proxy_jump.as_deref().filter(|s| !s.is_empty()) {
            rows.push((format!("via {pj}"), theme::mute()));
        }
    }

    // Tags.
    if !entry.tags().is_empty() {
        let tags = entry
            .tags()
            .iter()
            .map(|t| format!("#{t}"))
            .collect::<Vec<_>>()
            .join(" ");
        rows.push((tags, theme::dim()));
    }

    // Last connected (relative).
    if let Some(ts) = entry.last_connected() {
        let ago = crate::tui::widgets::right_stack::format_relative_time(ts);
        rows.push((format!("last: {ago}"), theme::dim()));
    }

    // Render as many rows as fit, one per line.
    for (i, (s, style)) in rows.iter().enumerate() {
        let y = inner_top + i as u16;
        if y >= area.y + area.height - 1 {
            break;
        }
        put_clamped(buf, text_x, y, s, *style, text_w);
    }
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
        let time_w = time_str.chars().count();
        buf.set_string(inner_x, row_y, &time_str, theme::mute());

        // Clamp with `…` (same helper as the rest of the dashboard) so a long
        // command line stays inside the panel border instead of a hard cut that
        // reads as overflowing the box.
        let remaining_w = inner_w.saturating_sub(time_w);
        put_clamped(
            buf,
            inner_x + time_w as u16,
            row_y,
            &entry.line,
            style,
            remaining_w,
        );
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

// Retained (not currently stacked) so the tunnels summary is easy to restore;
// the dedicated tunnels tab covers the same data.
#[allow(dead_code)]
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
