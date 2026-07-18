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
pub(crate) fn render_host_panel(buf: &mut Buffer, area: Rect, app: &App) {
    let entry = app.selected_entry();
    let title = match entry.as_ref() {
        Some(e) => format!("host · {}", e.name()),
        None => "host".to_string(),
    };
    render_panel_box(
        buf,
        area,
        &title,
        None,
        app.focused_panel == crate::app::PanelId::Detail,
    );

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

    // Left: OS logo (when enabled in Settings and the os_icon resolves to a
    // vendored distro logo). The OS name still shows in the fact sheet either way.
    let os_id = entry.managed().and_then(|m| m.os_icon.as_deref());
    let logo = if app.config.appearance.os_logo {
        os_id.and_then(crate::osinfo::logo_for)
    } else {
        None
    };
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
    render_panel_box(
        buf,
        area,
        &title,
        None,
        app.focused_panel == crate::app::PanelId::SshLog,
    );
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

    // Flatten entries into wrapped visual rows so long command lines stay fully
    // readable (word-wrapped) instead of truncated. The timestamp prints on the
    // first row of an entry; continuation rows indent under the message column.
    struct VRow {
        time: Option<String>,
        text: String,
        style: ratatui::style::Style,
    }
    const TIME_W: usize = 9; // "HH:MM:SS " — fixed width
    let wrap_w = inner_w.saturating_sub(TIME_W).max(1);
    let mut vrows: Vec<VRow> = Vec::new();
    for entry in &filtered {
        let style = match entry.level {
            crate::ssh::probe::LogLevel::Error => theme::red(),
            crate::ssh::probe::LogLevel::Success => theme::green(),
            crate::ssh::probe::LogLevel::Info => theme::dim(),
        };
        let time_str = format!("{} ", crate::tui::format_local_time(entry.timestamp));
        for (j, chunk) in wrap_line(&entry.line, wrap_w).into_iter().enumerate() {
            vrows.push(VRow {
                time: if j == 0 { Some(time_str.clone()) } else { None },
                text: chunk,
                style,
            });
        }
    }

    // Scrollable tail view over visual rows: scroll=0 shows the latest.
    let total = vrows.len();
    let scroll = app.ssh_log_scroll.min(total.saturating_sub(max_rows));
    let end = total.saturating_sub(scroll);
    let start = end.saturating_sub(max_rows);

    if scroll > 0 {
        let badge = format!("↑{scroll}");
        let bx = area.x + area.width.saturating_sub(badge.len() as u16 + 3);
        if bx > area.x + 2 {
            buf.set_string(bx, area.y, &badge, theme::mute());
        }
    }

    for (i, vr) in vrows[start..end].iter().enumerate() {
        let row_y = area.y + 1 + i as u16;
        if row_y >= area.y + area.height - 1 {
            break;
        }
        if let Some(t) = &vr.time {
            buf.set_string(inner_x, row_y, t, theme::mute());
        }
        buf.set_string(inner_x + TIME_W as u16, row_y, &vr.text, vr.style);
    }
}

/// Greedy word-wrap `s` to `width` columns (char count == display width for the
/// ASCII log lines here). Words longer than `width` are hard-split so a long
/// path/flag never overflows. Never returns an empty vec.
fn wrap_line(s: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![s.to_string()];
    }
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_len = 0usize;
    for word in s.split(' ') {
        let wlen = word.chars().count();
        if wlen > width {
            if cur_len > 0 {
                lines.push(std::mem::take(&mut cur));
                cur_len = 0;
            }
            let chars: Vec<char> = word.chars().collect();
            for chunk in chars.chunks(width) {
                lines.push(chunk.iter().collect());
            }
            continue;
        }
        let projected = if cur_len == 0 {
            wlen
        } else {
            cur_len + 1 + wlen
        };
        if projected > width {
            lines.push(std::mem::take(&mut cur));
            cur = word.to_string();
            cur_len = wlen;
        } else {
            if cur_len > 0 {
                cur.push(' ');
                cur_len += 1;
            }
            cur.push_str(word);
            cur_len += wlen;
        }
    }
    if cur_len > 0 || lines.is_empty() {
        lines.push(cur);
    }
    lines
}

// ── Agent panel ─────────────────────────────────────────

pub(crate) fn render_agent_panel(buf: &mut Buffer, area: Rect, app: &App) {
    render_panel_box(
        buf,
        area,
        "agent",
        None,
        app.focused_panel == crate::app::PanelId::Agent,
    );

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
    render_panel_box(buf, area, "tunnels", badge.as_deref(), false);

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

        let running =
            app.tunnel_manager.is_running(tunnel.id) || app.tunnel_manager.has_child(tunnel.id);
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

pub(crate) fn render_latency_panel(buf: &mut Buffer, area: Rect, app: &App) {
    // Per-host latency: the ping timeline of the currently selected host.
    let selected = app.selected_entry().map(|e| e.name().to_string());
    let title = match selected.as_deref() {
        Some(n) => format!("latency \u{b7} {n}"),
        None => "latency p50".to_string(),
    };
    render_panel_box(
        buf,
        area,
        &title,
        None,
        app.focused_panel == crate::app::PanelId::Latency,
    );

    let inner_x = area.x + 2;
    let inner_w = area.width.saturating_sub(4) as usize;

    let samples: Vec<u32> = selected
        .as_deref()
        .and_then(|n| app.ping_data.get(n))
        .into_iter()
        .flat_map(|v| {
            v.iter()
                .copied()
                .filter(|ms| !crate::ping::is_unreachable(*ms))
        })
        .collect();

    if samples.is_empty() {
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

    // Compute stats over this host's samples.
    let mut sorted = samples.clone();
    sorted.sort_unstable();
    let p50 = sorted[sorted.len() / 2];
    let peak = *sorted.last().unwrap_or(&0);
    let now_val = *samples.last().unwrap_or(&0);

    // Sparkline from the last ~30 samples of this host.
    let spark_y = area.y + 1;
    if spark_y < area.y + area.height - 1 {
        let spark_len = samples.len().min(inner_w).min(30);
        let start = samples.len().saturating_sub(spark_len);
        let window = &samples[start..];
        let max_val = (*window.iter().max().unwrap_or(&1)).max(1);
        let sparkline: String = window
            .iter()
            .map(|&v| {
                let idx = ((v as u64 * 7) / max_val as u64).min(7) as usize;
                SPARK_CHARS[idx]
            })
            .collect();
        buf.set_string(inner_x, spark_y, &sparkline, theme::green());
    }

    // Stats row (avg = p50 median).
    let info_y = area.y + 2;
    if info_y < area.y + area.height - 1 {
        let stats = format!("now {}ms  avg {}ms  peak {}ms", now_val, p50, peak);
        put_clamped(buf, inner_x, info_y, &stats, theme::dim(), inner_w);
    }
}

#[cfg(test)]
mod tests {
    use super::wrap_line;

    #[test]
    fn wraps_on_word_boundaries() {
        let out = wrap_line("alpha beta gamma", 11);
        assert_eq!(out, vec!["alpha beta".to_string(), "gamma".to_string()]);
    }

    #[test]
    fn hard_splits_overlong_words() {
        let out = wrap_line("aaaaaaaa", 3);
        assert_eq!(out, vec!["aaa", "aaa", "aa"]);
    }

    #[test]
    fn never_empty_and_short_fits() {
        assert_eq!(wrap_line("", 10), vec!["".to_string()]);
        assert_eq!(wrap_line("hi", 10), vec!["hi".to_string()]);
    }
}
