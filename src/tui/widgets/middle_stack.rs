//! Middle column dashboard stack: selected-host card, Agent info, Latency.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Widget;
use ratatui::Frame;

use crate::app::App;
use crate::osinfo::widget::{logo_dimensions, OsLogoWidget};
use crate::theme::catalog::{ColorRole, PaintRole, StyleRole};
use crate::tui::widgets::panel_box::{
    put_clamped, render_panel_box, AGENT_PANEL, DETAILS_PANEL, LATENCY_PANEL, SSH_LOG_PANEL,
};

// ── Panel heights (sum = 19 to align with the right column) ─
pub const HOST_H: u16 = 9;
pub const AGENT_H: u16 = 6;
pub const LATENCY_H: u16 = 4;

/// Render the three middle-column panels stacked vertically.
pub fn render_middle_stack(frame: &mut Frame, area: Rect, app: &App) {
    let agent = crate::ssh::agent::detect_agent();
    render_middle_stack_with_info(frame, area, app, &agent);
}

pub(crate) fn render_middle_stack_with_info(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    agent: &crate::ssh::agent::AgentInfo,
) {
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
    render_agent_panel_with_info(buf, agent_area, app, agent);
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
    // Everything below the title belongs to whichever host is selected, so a
    // moved cursor swaps the lot. Fade it up instead of flicking it over (#35).
    let fade = content_fade(app.selection_at, app.motion_enabled());
    let theme = app.theme();
    let entry = app.selected_entry();
    let title = match entry.as_ref() {
        Some(e) => format!("host · {}", e.name()),
        None => "host".to_string(),
    };
    render_panel_box(
        buf,
        area,
        &title,
        DETAILS_PANEL.plain(),
        app.focused_panel == crate::app::PanelId::Detail,
        theme,
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
    let zoomed = area.height >= crate::tui::widgets::panel_box::ZOOM_CONTENT_MIN;
    let os_id = entry.managed().and_then(|m| m.os_icon.as_deref());
    // When zoomed, prefer the large full-colour logo (fastfetch art); fall back
    // to the small Braille one otherwise.
    let logo = if app.config.appearance.os_logo {
        let large = zoomed
            .then(|| os_id.and_then(crate::osinfo::large_logo_for))
            .flatten();
        large.or_else(|| os_id.and_then(crate::osinfo::logo_for))
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
        OsLogoWidget::new(logo, crate::osinfo::logos::os_logo_tint(theme)).render(logo_area, buf);
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

    let metadata = theme.style(StyleRole::DashboardDetailsMetadata);
    let dim = theme.style(StyleRole::TextDim);
    let mut rows: Vec<(String, ratatui::style::Style)> = Vec::new();

    // Name (+ favourite star).
    let name = if entry.favorite() {
        format!("{} \u{2605}", entry.name())
    } else {
        entry.name().to_string()
    };
    rows.push((name, theme.style(StyleRole::TextBright)));

    // user@host:port (user omitted when unknown).
    let hostport = match ssh.user.as_deref() {
        Some(u) if !u.is_empty() => format!("{}@{}:{}", u, addr, port),
        _ => format!("{}:{}", addr, port),
    };
    rows.push((hostport, theme.style(StyleRole::DashboardDetailsValue)));

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
    rows.push((os_line, theme.style(StyleRole::DashboardDetailsLabel)));

    // Group / identity / proxy — managed hosts only.
    if let Some(m) = managed {
        if let Some(g) = m.group.as_ref() {
            rows.push((format!("group: {}", g.name), metadata));
        }
        if let Some(id) = m.identity.as_ref() {
            rows.push((format!("key: {}", id.name), metadata));
        }
        if let Some(pj) = m.proxy_jump.as_deref().filter(|s| !s.is_empty()) {
            rows.push((format!("via {pj}"), metadata));
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
        rows.push((tags, dim));
    }

    // Last connected (relative).
    if let Some(ts) = entry.last_connected() {
        let ago = crate::tui::widgets::right_stack::format_relative_time(ts);
        rows.push((format!("last: {ago}"), dim));
    }

    // Zoomed: the card owns the whole dashboard body, so surface the full fact
    // sheet instead of the compact summary.
    if zoomed {
        rows.push((
            format!("transport: {}", entry.session_transport().label()),
            metadata,
        ));
        rows.push((
            format!("session log: {}", entry.session_logging_override().label()),
            metadata,
        ));
        if ssh.forward_agent == Some(true) {
            rows.push(("forward agent: yes".to_string(), metadata));
        }
        if let Some(rc) = ssh.remote_command.as_deref().filter(|s| !s.is_empty()) {
            rows.push((format!("command: {rc}"), metadata));
        }
        if let Some(m) = managed {
            if let Some(id) = m.identity.as_ref() {
                if let Some(u) = id.username.as_deref().filter(|s| !s.is_empty()) {
                    rows.push((format!("login: {u}"), metadata));
                }
                if let Some(pk) = id.private_key.as_ref() {
                    rows.push((format!("key file: {}", pk.display()), dim));
                }
            }
            if m.has_password {
                rows.push(("password: stored".to_string(), dim));
            }
        }
        rows.push((format!("source: {}", entry.source().as_str()), dim));
        if let Some(env) = entry.environment().filter(|s| !s.is_empty()) {
            rows.push((format!("env: {env}"), dim));
        }
        if let Some(notes) = entry.description().filter(|s| !s.is_empty()) {
            rows.push((format!("notes: {notes}"), dim));
        }
    }

    // Render as many rows as fit, one per line.
    for (i, (s, style)) in rows.iter().enumerate() {
        let y = inner_top + i as u16;
        if y >= area.y + area.height - 1 {
            break;
        }
        put_clamped(buf, text_x, y, s, *style, text_w);
    }

    // Fade the body only: the box and its title frame the panel and shouldn't
    // blink along with what they hold.
    if area.width > 2 && area.height > 2 {
        let body = Rect::new(area.x + 1, area.y + 1, area.width - 2, area.height - 2);
        crate::tui::blit::fade(
            buf,
            body,
            fade,
            crate::tui::blit::FadeGround {
                theme,
                role: PaintRole::DashboardDetailsBackground,
                // The details background's component rect is *this panel*, not
                // the frame and not the faded slice: a gradient must restart
                // where the panel starts and nowhere else.
                paint_area: area,
                exclusions: &[],
            },
        );
    }
}

/// Render the SSH log panel (meant to span both middle + right columns).
pub fn render_ssh_log_panel(frame: &mut Frame, area: Rect, app: &App) {
    let buf = frame.buffer_mut();
    render_ssh_log(buf, area, app);
}

fn render_ssh_log(buf: &mut Buffer, area: Rect, app: &App) {
    let theme = app.theme();
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
        SSH_LOG_PANEL.plain(),
        app.focused_panel == crate::app::PanelId::SshLog,
        theme,
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
            put_clamped(
                buf,
                inner_x,
                placeholder_y,
                &msg,
                theme.style(StyleRole::TextDim),
                inner_w,
            );
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
            crate::ssh::probe::LogLevel::Error => {
                Style::default().fg(theme.color(ColorRole::StatusError))
            }
            crate::ssh::probe::LogLevel::Success => {
                Style::default().fg(theme.color(ColorRole::StatusSuccess))
            }
            crate::ssh::probe::LogLevel::Info => theme.style(StyleRole::TextDim),
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
            buf.set_string(bx, area.y, &badge, theme.style(StyleRole::TextMuted));
        }
    }

    for (i, vr) in vrows[start..end].iter().enumerate() {
        let row_y = area.y + 1 + i as u16;
        if row_y >= area.y + area.height - 1 {
            break;
        }
        if let Some(t) = &vr.time {
            buf.set_string(inner_x, row_y, t, theme.style(StyleRole::TextMuted));
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

/// How far swapped-out panel content has faded in, `0.0` to `1.0` (#35).
/// `1.0` at rest and under reduced motion, where content simply appears.
pub(crate) fn content_fade(at: Option<std::time::Instant>, motion: bool) -> f32 {
    if !motion {
        return 1.0;
    }
    match at {
        Some(at) => crate::tui::tween::ease_out(crate::tui::tween::progress(
            at,
            crate::tui::CONTENT_FADE,
            std::time::Instant::now(),
        )),
        None => 1.0,
    }
}

pub(crate) fn render_agent_panel(buf: &mut Buffer, area: Rect, app: &App) {
    let agent = crate::ssh::agent::detect_agent();
    render_agent_panel_with_info(buf, area, app, &agent);
}

pub(crate) fn render_agent_panel_with_info(
    buf: &mut Buffer,
    area: Rect,
    app: &App,
    agent: &crate::ssh::agent::AgentInfo,
) {
    let theme = app.theme();
    let text = theme.style(StyleRole::TextPrimary);
    let bright = theme.style(StyleRole::TextBright);
    let dim = theme.style(StyleRole::TextDim);
    let error = Style::default().fg(theme.color(ColorRole::StatusError));
    render_panel_box(
        buf,
        area,
        "agent",
        AGENT_PANEL.plain(),
        app.focused_panel == crate::app::PanelId::Agent,
        theme,
    );

    let inner_x = area.x + 2;
    let inner_w = area.width.saturating_sub(4) as usize;

    // Zoomed: keep the socket/forward/config header, then list every loaded key
    // (type, bits, full fingerprint, comment) filling the panel height.
    if area.height >= crate::tui::widgets::panel_box::ZOOM_CONTENT_MIN {
        let bottom_guard = area.y + area.height - 1;
        let label_style = dim;
        let mut y = area.y + 1;

        // Header row: socket path.
        if y < bottom_guard {
            buf.set_string(inner_x, y, "socket  ", label_style);
            let val_x = inner_x + 8;
            let _ = match &agent.socket_path {
                Some(path) => put_clamped(buf, val_x, y, path, text, inner_w.saturating_sub(8)),
                None => put_clamped(buf, val_x, y, "not found", error, inner_w.saturating_sub(8)),
            };
            y += 1;
        }

        // Header row: forward-agent host count.
        if y < bottom_guard {
            let fwd_count = app
                .hosts
                .iter()
                .filter(|h| match h {
                    crate::app::HostEntry::Managed(m) => m.forward_agent,
                    crate::app::HostEntry::Legacy { host, .. } => {
                        host.forward_agent.unwrap_or(false)
                    }
                })
                .count();
            buf.set_string(inner_x, y, "forward ", label_style);
            let fwd_str = format!("{fwd_count} hosts");
            put_clamped(
                buf,
                inner_x + 8,
                y,
                &fwd_str,
                bright,
                inner_w.saturating_sub(8),
            );
            y += 1;
        }

        // Header row: config path.
        if y < bottom_guard {
            buf.set_string(inner_x, y, "config  ", label_style);
            put_clamped(
                buf,
                inner_x + 8,
                y,
                "~/.ssh/config",
                text,
                inner_w.saturating_sub(8),
            );
            y += 1;
        }

        // Blank spacer before the key list.
        if y < bottom_guard {
            y += 1;
        }

        // Key-list header.
        if y < bottom_guard {
            let hdr = format!("keys ({}):", agent.keys.len());
            put_clamped(buf, inner_x, y, &hdr, bright, inner_w);
            y += 1;
        }

        if agent.keys.is_empty() {
            if y < bottom_guard {
                put_clamped(
                    buf,
                    inner_x + 2,
                    y,
                    "no keys loaded",
                    dim,
                    inner_w.saturating_sub(2),
                );
            }
            return;
        }

        // One indented row per key: type, bits, full fingerprint, comment.
        // Selectable (issue #18): the highlighted key is removable with `d`.
        let key_w = inner_w.saturating_sub(2);
        let visible = bottom_guard.saturating_sub(y) as usize;
        let (first, sel) =
            crate::tui::widgets::panel_box::zoom_window(app, agent.keys.len(), visible);
        for (di, key) in agent.keys.iter().enumerate().skip(first) {
            if y >= bottom_guard {
                break;
            }
            let mut line = format!("{} {}", key.key_type, key.bits);
            if !key.fingerprint.is_empty() {
                line.push_str("  ");
                line.push_str(&key.fingerprint);
            }
            if !key.comment.is_empty() {
                line.push_str("  ");
                line.push_str(&key.comment);
            }
            let style = if di == sel {
                text.add_modifier(ratatui::style::Modifier::REVERSED)
            } else {
                text
            };
            put_clamped(buf, inner_x + 2, y, &line, style, key_w);
            y += 1;
        }
        return;
    }

    // Row 1: socket path
    let row1_y = area.y + 1;
    if row1_y < area.y + area.height - 1 {
        buf.set_string(inner_x, row1_y, "socket  ", dim);
        let label_w = 8; // "socket  ".len()
        let val_x = inner_x + label_w as u16;
        let max_path = inner_w.saturating_sub(label_w);
        match &agent.socket_path {
            Some(path) => {
                let display: String = path.chars().take(max_path).collect();
                buf.set_string(val_x, row1_y, &display, text);
            }
            None => {
                buf.set_string(val_x, row1_y, "not found", error);
            }
        }
    }

    // Row 2: keys loaded
    let row2_y = area.y + 2;
    if row2_y < area.y + area.height - 1 {
        buf.set_string(inner_x, row2_y, "keys    ", dim);
        let val_x = inner_x + 8;
        let key_str = format!("{} loaded", agent.keys.len());
        put_clamped(
            buf,
            val_x,
            row2_y,
            &key_str,
            bright,
            inner_w.saturating_sub(8),
        );
    }

    // Row 3: forward agent hosts count
    let row3_y = area.y + 3;
    if row3_y < area.y + area.height - 1 {
        buf.set_string(inner_x, row3_y, "forward ", dim);
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
            bright,
            inner_w.saturating_sub(8),
        );
    }

    // Row 4: config path
    let row4_y = area.y + 4;
    if row4_y < area.y + area.height - 1 {
        buf.set_string(inner_x, row4_y, "config  ", dim);
        let val_x = inner_x + 8;
        put_clamped(
            buf,
            val_x,
            row4_y,
            "~/.ssh/config",
            text,
            inner_w.saturating_sub(8),
        );
    }
}

// ── Latency sparkline panel ─────────────────────────────

/// The ping timeline of the selected host: a bar graph when the panel is zoomed
/// tall enough for one, a single-row sparkline otherwise.
///
/// Both draw from `theme::SPARK` and band their columns by the *window peak*
/// rather than an absolute latency, so a fast link and a slow one are both
/// legible instead of one of them being a flat line.
pub(crate) fn render_latency_panel(buf: &mut Buffer, area: Rect, app: &App) {
    let theme = app.theme();
    let dim = theme.style(StyleRole::TextDim);
    // The bar/sparkline ramp is the metrics family's own three colours.
    let spark_low = Style::default().fg(theme.color(ColorRole::DashboardMetricsSparklineLow));
    let spark_mid = Style::default().fg(theme.color(ColorRole::DashboardMetricsSparklineMedium));
    let spark_high = Style::default().fg(theme.color(ColorRole::DashboardMetricsSparklineHigh));
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
        LATENCY_PANEL.plain(),
        app.focused_panel == crate::app::PanelId::Latency,
        theme,
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
            buf.set_string(inner_x, spark_y, &baseline, dim);
        }
        let info_y = area.y + 2;
        if info_y < area.y + area.height - 1 {
            put_clamped(buf, inner_x, info_y, "no latency data", dim, inner_w);
        }
        return;
    }

    // Compute stats over this host's samples.
    let mut sorted = samples.clone();
    sorted.sort_unstable();
    let p50 = sorted[sorted.len() / 2];
    let peak = *sorted.last().unwrap_or(&0);
    let now_val = *samples.last().unwrap_or(&0);

    // Zoomed: a numeric stat row plus a tall, full-height bar graph of the
    // samples (one bottom-anchored column per sample, coloured by latency).
    if area.height >= crate::tui::widgets::panel_box::ZOOM_CONTENT_MIN {
        let min = sorted[0];
        let bottom_guard = area.y + area.height - 1;

        // Numeric stat row across the top.
        let stat_y = area.y + 1;
        if stat_y < bottom_guard {
            let stats = format!("min {min}  p50 {p50}  max {peak}  last {now_val} (ms)");
            put_clamped(
                buf,
                inner_x,
                stat_y,
                &stats,
                theme.style(StyleRole::TextBright),
                inner_w,
            );
        }

        // Bar graph fills the rest of the body below the stat row.
        let graph_top = area.y + 2;
        let grow = selected.as_deref().map(|n| app.ping_grow(n)).unwrap_or(1.0);
        let graph_h = area.height.saturating_sub(3);
        if graph_h >= 1 && inner_w >= 1 {
            let cols = samples.len().min(inner_w);
            let start = samples.len().saturating_sub(cols);
            let window = &samples[start..];
            let max_val = (*window.iter().max().unwrap_or(&1)).max(1) as u64;
            let bottom = graph_top + graph_h - 1;
            let units = graph_h as u64 * 8; // 8 sub-cell levels per row
            for (i, &v) in window.iter().enumerate() {
                let x = inner_x + i as u16;
                if x >= inner_x + inner_w as u16 {
                    break;
                }
                // Colour by latency relative to the window peak.
                let style = if (v as u64) * 3 < max_val {
                    spark_low
                } else if (v as u64) * 3 < max_val * 2 {
                    spark_mid
                } else {
                    spark_high
                };
                let mut level = (((v as u64) * units) / max_val).clamp(1, units);
                // The newest column grows in rather than appearing full height.
                if i + 1 == window.len() {
                    level = ((level as f32 * grow).round() as u64).clamp(1, units);
                }
                let full = (level / 8) as u16;
                let rem = (level % 8) as usize;
                // Full block cells from the bottom up.
                for c in 0..full {
                    let y = bottom - c;
                    if y < graph_top {
                        break;
                    }
                    buf.set_string(x, y, "\u{2588}", style);
                }
                // Partial cap above the full cells.
                if rem > 0 && full < graph_h {
                    let y = bottom - full;
                    if y >= graph_top {
                        buf.set_string(
                            x,
                            y,
                            crate::tui::theme::SPARK[rem - 1].to_string().as_str(),
                            style,
                        );
                    }
                }
            }
        }
        return;
    }

    // Sparkline from the last ~30 samples of this host.
    let spark_y = area.y + 1;
    if spark_y < area.y + area.height - 1 {
        let spark_len = samples.len().min(inner_w).min(30);
        let start = samples.len().saturating_sub(spark_len);
        let window = &samples[start..];
        let max_val = (*window.iter().max().unwrap_or(&1)).max(1);
        let grow = selected.as_deref().map(|n| app.ping_grow(n)).unwrap_or(1.0);
        let last = window.len().saturating_sub(1);
        let sparkline: String = window
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                let mut idx = ((v as u64 * 7) / max_val as u64).min(7) as usize;
                if i == last {
                    idx = (idx as f32 * grow).round() as usize;
                }
                crate::tui::theme::SPARK[idx]
            })
            .collect();
        buf.set_string(inner_x, spark_y, &sparkline, spark_low);
    }

    // Stats row (avg = p50 median).
    let info_y = area.y + 2;
    if info_y < area.y + area.height - 1 {
        let stats = format!("now {}ms  avg {}ms  peak {}ms", now_val, p50, peak);
        put_clamped(buf, inner_x, info_y, &stats, dim, inner_w);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        assert_panel_wears, buffer_at, find_text, find_text_from, frame_at, panel_marker_theme,
        resolved_source, themed_app, PanelFamily, PanelProof,
    };
    use ratatui::style::Color;

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

    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        width: 60,
        height: 14,
    };

    /// The details, metrics and status roles the middle column writes with,
    /// each on a colour nobody else uses.
    fn middle_marker_theme() -> crate::theme::model::ResolvedTheme {
        resolved_source(
            "markers",
            "schema_version = 1\nname = \"Markers\"\nextends = \"default\"\n\n\
             [components.dashboard.details]\n\
             label = { foreground = \"#ff0000\" }\n\
             value = { foreground = \"#00ff00\" }\n\
             metadata = { foreground = \"#0000ff\" }\n\n\
             [components.dashboard.metrics]\n\
             sparkline_low = \"#111100\"\n\
             sparkline_medium = \"#222200\"\n\
             sparkline_high = \"#333300\"\n\n\
             [components.text]\n\
             bright = { foreground = \"#ffff00\" }\n\
             dim = { foreground = \"#00ffff\" }\n",
        )
    }

    /// `find_text`, but never on the frame's top row — a panel title often
    /// repeats the very word the body is being checked for.
    fn find_in_body(buf: &ratatui::buffer::Buffer, needle: &str) -> (u16, u16) {
        find_text_from(buf, needle, buf.area.top() + 1)
    }

    /// The host details card writes value, label and metadata from their own
    /// three roles rather than from one shared text style.
    #[test]
    fn the_details_card_separates_value_label_and_metadata() {
        let mut app = themed_app(middle_marker_theme());
        // `metadata` only reaches a cell on a managed host: the group, key and
        // proxy rows are the only thing that carries it. Without one, the
        // marker would resolve and never be drawn. `proxy_jump` is used rather
        // than a group so the host stays the first navigable row.
        app.hosts = vec![crate::app::HostEntry::from_managed(
            crate::store::ManagedHost {
                id: 1,
                name: "web-prod".into(),
                label: None,
                address: "10.0.0.1".into(),
                port: 22,
                group_id: None,
                identity_id: None,
                group: None,
                groups: Vec::new(),
                identity: None,
                os_icon: None,
                tags: Vec::new(),
                notes: None,
                proxy_jump: Some("bastion".into()),
                forward_agent: false,
                remote_command: None,
                environment: None,
                sort_order: 0,
                favorite: false,
                last_connected: None,
                source: crate::store::HostSource::Launcher,
                ssh_config_hash: None,
                has_password: false,
                username: None,
                session_logging: crate::session_log::SessionLoggingOverride::Inherit,
                transport: Default::default(),
                created_at: 0,
                updated_at: 0,
            },
        )];
        app.rebuild_filter();
        let buf = buffer_at(AREA, |buf| render_host_panel(buf, AREA, &app));

        // `user@host:port` is the value row.
        let (x, y) = find_text(&buf, "10.0.0.1:22");
        assert_eq!(
            buf.cell((x, y)).unwrap().fg,
            Color::Rgb(0x00, 0xff, 0x00),
            "the address row takes `details.value`"
        );

        // The name row above it is the card's bright headline. Searched in the
        // body: the panel *title* is `host \u{b7} web-prod` and carries
        // `details.title` instead.
        let (nx, ny) = find_in_body(&buf, "web-prod");
        assert_eq!(
            buf.cell((nx, ny)).unwrap().fg,
            Color::Rgb(0xff, 0xff, 0x00),
            "the host name takes `text.bright`"
        );

        // The os/latency row is the label role.
        let (lx, ly) = find_text(&buf, "unknown os");
        assert_eq!(
            buf.cell((lx, ly)).unwrap().fg,
            Color::Rgb(0xff, 0x00, 0x00),
            "the os row takes `details.label`"
        );

        // The `via <jump>` row is the metadata role — the marker's only surface.
        let (mx, my) = find_text(&buf, "via bastion");
        assert_eq!(
            buf.cell((mx, my)).unwrap().fg,
            Color::Rgb(0x00, 0x00, 0xff),
            "the group row takes `details.metadata`"
        );
    }

    /// The four middle-column panels each wear their own family, in **both**
    /// focus states, and none of them draws a badge.
    #[test]
    fn the_middle_panels_wear_their_own_roles_in_both_focus_states() {
        use crate::app::PanelId;

        let mut app = themed_app(panel_marker_theme());
        let body = (2, AREA.height - 2);

        for focused in [false, true] {
            let elsewhere = PanelId::Hosts;

            app.focused_panel = if focused { PanelId::Detail } else { elsewhere };
            let buf = buffer_at(AREA, |buf| render_host_panel(buf, AREA, &app));
            assert_panel_wears(
                &buf,
                AREA,
                PanelProof {
                    family: PanelFamily::Details,
                    focused,
                    title: "host \u{b7}",
                    count: None,
                    body,
                },
            );

            app.focused_panel = if focused { PanelId::SshLog } else { elsewhere };
            let buf = frame_at(AREA, |frame| render_ssh_log_panel(frame, AREA, &app));
            assert_panel_wears(
                &buf,
                AREA,
                PanelProof {
                    family: PanelFamily::SshLog,
                    focused,
                    title: "ssh log",
                    count: None,
                    body,
                },
            );

            app.focused_panel = if focused { PanelId::Agent } else { elsewhere };
            let buf = buffer_at(AREA, |buf| render_agent_panel(buf, AREA, &app));
            assert_panel_wears(
                &buf,
                AREA,
                PanelProof {
                    family: PanelFamily::Agent,
                    focused,
                    title: "agent",
                    count: None,
                    body,
                },
            );

            app.focused_panel = if focused { PanelId::Latency } else { elsewhere };
            let buf = buffer_at(AREA, |buf| render_latency_panel(buf, AREA, &app));
            assert_panel_wears(
                &buf,
                AREA,
                PanelProof {
                    family: PanelFamily::Latency,
                    focused,
                    title: "latency",
                    count: None,
                    body,
                },
            );
        }
    }

    /// The latency bars ramp through the three `metrics.sparkline_*` colours,
    /// which is the only place those roles reach a cell.
    #[test]
    fn the_latency_bars_take_the_three_metrics_sparkline_colours() {
        let mut app = themed_app(middle_marker_theme());
        // A spread wide enough that the ramp uses all three bands: the panel
        // colours each column by its value against the window peak.
        app.ping_data
            .insert("web-prod".into(), vec![10, 20, 30, 200, 400, 600, 800, 900]);

        let buf = buffer_at(AREA, |buf| render_latency_panel(buf, AREA, &app));

        let mut seen = std::collections::HashSet::new();
        for y in AREA.top()..AREA.bottom() {
            for x in AREA.left()..AREA.right() {
                seen.insert(buf.cell((x, y)).unwrap().fg);
            }
        }
        for (name, want) in [
            ("sparkline_low", Color::Rgb(0x11, 0x11, 0x00)),
            ("sparkline_medium", Color::Rgb(0x22, 0x22, 0x00)),
            ("sparkline_high", Color::Rgb(0x33, 0x33, 0x00)),
        ] {
            assert!(
                seen.contains(&want),
                "`metrics.{name}` never reached a cell; saw {seen:?}"
            );
        }
    }

    /// The SSH log's empty state is the dim text role, and the agent panel's
    /// labels and values are separate roles.
    #[test]
    fn the_ssh_log_and_agent_panels_take_their_text_roles() {
        let app = themed_app(middle_marker_theme());

        let buf = frame_at(AREA, |frame| render_ssh_log_panel(frame, AREA, &app));
        let (x, y) = find_text(&buf, "no events for");
        assert_eq!(
            buf.cell((x, y)).unwrap().fg,
            Color::Rgb(0x00, 0xff, 0xff),
            "the ssh-log placeholder takes `text.dim`"
        );

        // Below `ZOOM_CONTENT_MIN`, so the compact `label  value` rows render.
        let compact = Rect::new(0, 0, 60, 8);
        let buf = buffer_at(compact, |buf| render_agent_panel(buf, compact, &app));
        let (lx, ly) = find_text(&buf, "socket");
        assert_eq!(
            buf.cell((lx, ly)).unwrap().fg,
            Color::Rgb(0x00, 0xff, 0xff),
            "the agent panel's labels take `text.dim`"
        );
        let (kx, ky) = find_text(&buf, "loaded");
        assert_eq!(
            buf.cell((kx, ky)).unwrap().fg,
            Color::Rgb(0xff, 0xff, 0x00),
            "the agent panel's key count takes `text.bright`"
        );
    }

    #[test]
    fn the_agent_panel_renders_the_supplied_agent_snapshot() {
        let app = themed_app(middle_marker_theme());
        let compact = Rect::new(0, 0, 60, 8);
        let connected = crate::ssh::agent::AgentInfo {
            socket_path: Some("/tmp/fixed-agent.sock".into()),
            keys: vec![crate::ssh::agent::AgentKey {
                bits: "256".into(),
                fingerprint: "SHA256:fixed".into(),
                comment: "ci key".into(),
                key_type: "ED25519".into(),
            }],
            forwarding_hosts: 0,
        };

        let connected_buf = buffer_at(compact, |buf| {
            render_agent_panel_with_info(buf, compact, &app, &connected)
        });
        assert!(find_text(&connected_buf, "/tmp/fixed-agent.sock").0 > 0);
        assert!(find_text(&connected_buf, "1 loaded").0 > 0);

        let disconnected_buf = buffer_at(compact, |buf| {
            render_agent_panel_with_info(
                buf,
                compact,
                &app,
                &crate::ssh::agent::AgentInfo::default(),
            )
        });
        let (x, y) = find_text(&disconnected_buf, "not found");
        assert_eq!(
            disconnected_buf.cell((x, y)).unwrap().fg,
            app.theme().color(ColorRole::StatusError)
        );
        assert!(find_text(&disconnected_buf, "0 loaded").0 > 0);
    }
}
