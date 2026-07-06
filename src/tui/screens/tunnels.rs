use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::widgets::Clear;

use crate::app::App;
use crate::store::TunnelType;
use crate::tui::theme;

pub fn render_tunnels(frame: &mut Frame, area: Rect, app: &App) {
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

    // Row 0: Summary strip
    let summary_y = area.y;
    let active = app.tunnel_manager.active_count();
    let total = app.tunnels.len();
    let summary = format!("{total} tunnels  {active} active");
    buf.set_string(inner_x, summary_y, &summary, theme::mute());

    if let Some(ref notice) = app.tunnel_notice {
        let nx = inner_x + summary.len() as u16 + 3;
        if nx + notice.len() as u16 <= inner_x + inner_w {
            buf.set_string(nx, summary_y, notice, theme::amber());
        }
    }

    // Row 1: blank separator
    // Row 2: Table header
    let header_y = area.y + 2;
    if header_y >= area.y + area.height {
        return;
    }
    render_table_header(buf, inner_x, header_y, inner_w);

    // Row 3: separator line
    let sep_y = header_y + 1;
    if sep_y < area.y + area.height {
        let line: String = std::iter::repeat_n('─', inner_w as usize).collect();
        buf.set_string(inner_x, sep_y, &line, theme::dim());
    }

    // Row 4+: Data rows
    let data_y = header_y + 2;
    let max_rows = (area.y + area.height).saturating_sub(data_y) as usize;

    let visible_rows = max_rows.min(app.tunnels.len());
    let scroll = if app.tunnel_selected >= max_rows {
        app.tunnel_selected - max_rows + 1
    } else {
        0
    };

    for (i, tunnel) in app
        .tunnels
        .iter()
        .skip(scroll)
        .take(visible_rows)
        .enumerate()
    {
        let y = data_y + i as u16;
        let row_idx = scroll + i;
        let is_selected = row_idx == app.tunnel_selected;
        let status = app.tunnel_manager.status(tunnel.id);
        let uptime = app.tunnel_manager.uptime_secs(tunnel.id);

        let host_name = tunnel
            .host_id
            .and_then(|hid| app.store().get_host(hid).ok().flatten())
            .map(|h| h.name);

        render_tunnel_row(
            buf,
            inner_x,
            y,
            inner_w,
            tunnel,
            is_selected,
            status,
            uptime,
            host_name.as_deref(),
        );
    }

    // Empty state
    if app.tunnels.is_empty() {
        let msg = "No tunnels — press 'a' to add one";
        let x = inner_x + (inner_w.saturating_sub(msg.len() as u16)) / 2;
        let y = data_y + 2.min(max_rows.saturating_sub(1) as u16);
        buf.set_string(x, y, msg, theme::dim());
    }
}

fn render_table_header(buf: &mut Buffer, x: u16, y: u16, w: u16) {
    let cols = table_columns(w);
    let mut cx = x;
    for (label, width) in &cols {
        buf.set_string(cx, y, label, theme::bright().add_modifier(Modifier::BOLD));
        cx += width;
    }
}

fn render_tunnel_row(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    w: u16,
    tunnel: &crate::store::Tunnel,
    selected: bool,
    status: &str,
    uptime: Option<u64>,
    host_name: Option<&str>,
) {
    let running = status == "up";
    let base_style = if selected {
        theme::selected()
    } else {
        theme::text()
    };

    if selected {
        for cx in x..x + w {
            if let Some(cell) = buf.cell_mut((cx, y)) {
                cell.set_style(base_style);
                cell.set_symbol(" ");
            }
        }
    }

    let cols = table_columns(w);
    let mut cx = x;

    // STATUS dot
    let status_w = cols[0].1;
    let (dot, dot_color) = match status {
        "up" => ("●", theme::GREEN),
        "error" => ("●", theme::RED),
        _ => ("○", theme::DIM),
    };
    let dot_style = if selected {
        Style::default().fg(dot_color).bg(theme::SEL_BG)
    } else {
        Style::default().fg(dot_color)
    };
    buf.set_string(cx, y, dot, dot_style);
    if running {
        let label = "up";
        buf.set_string(
            cx + 2,
            y,
            label,
            if selected { base_style } else { theme::green() },
        );
    } else if status == "error" {
        buf.set_string(
            cx + 2,
            y,
            "err",
            if selected { base_style } else { theme::red() },
        );
    } else {
        buf.set_string(
            cx + 2,
            y,
            "off",
            if selected { base_style } else { theme::dim() },
        );
    }
    cx += status_w;

    // DIR
    let dir_w = cols[1].1;
    let dir_label = match tunnel.tunnel_type {
        TunnelType::Local => "L",
        TunnelType::Remote => "R",
        TunnelType::Dynamic => "D",
    };
    buf.set_string(
        cx,
        y,
        dir_label,
        if selected { base_style } else { theme::cyan() },
    );
    cx += dir_w;

    // LOCAL
    let local_w = cols[2].1;
    let local_str = format!(":{}", tunnel.local_port);
    buf.set_string(cx, y, truncate(&local_str, local_w as usize), base_style);
    cx += local_w;

    // REMOTE
    let remote_w = cols[3].1;
    let remote_str = if tunnel.tunnel_type == TunnelType::Dynamic {
        "SOCKS".to_string()
    } else {
        format!("{}:{}", tunnel.remote_host, tunnel.remote_port)
    };
    buf.set_string(
        cx,
        y,
        truncate(&remote_str, remote_w as usize),
        if selected { base_style } else { theme::mute() },
    );
    cx += remote_w;

    // HOST
    let host_w = cols[4].1;
    let host_label = host_name.unwrap_or("-");
    buf.set_string(
        cx,
        y,
        truncate(host_label, host_w as usize),
        if selected { base_style } else { theme::dim() },
    );
    cx += host_w;

    // LABEL / UPTIME
    let remaining = (x + w).saturating_sub(cx) as usize;
    let label_str = if let Some(secs) = uptime {
        let label_part = tunnel.label.as_deref().unwrap_or("");
        if label_part.is_empty() {
            format_uptime(secs)
        } else {
            format!("{}  {}", label_part, format_uptime(secs))
        }
    } else {
        tunnel.label.as_deref().unwrap_or("").to_string()
    };
    buf.set_string(
        cx,
        y,
        truncate(&label_str, remaining),
        if selected { base_style } else { theme::dim() },
    );
}

pub fn render_tunnel_form(frame: &mut Frame, app: &App) {
    let Some(form) = app.tunnel_form.as_ref() else {
        return;
    };

    let area = frame.area();
    let popup_width = 52u16.min(area.width.saturating_sub(4)).max(40);
    let popup_height = 14u16.min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);

    // Border
    let title = if form.editing_id.is_some() {
        "Edit Tunnel"
    } else {
        "New Tunnel"
    };
    let border = ratatui::widgets::Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(theme::ACCENT));
    let inner = border.inner(popup_area);
    frame.render_widget(border, popup_area);

    let buf = frame.buffer_mut();

    use crate::app::TunnelFormField;

    let fields: Vec<(TunnelFormField, &str, String)> = vec![
        (TunnelFormField::Host, "SSH server", {
            form.host_id
                .and_then(|hid| app.store().get_host(hid).ok().flatten())
                .map(|h| h.name)
                .unwrap_or_else(|| "(none)".to_string())
        }),
        (
            TunnelFormField::Type,
            "Type",
            form.tunnel_type.label().to_string(),
        ),
        (
            TunnelFormField::LocalPort,
            "Local port",
            form.local_port.clone(),
        ),
        (
            TunnelFormField::RemoteHost,
            "Destination",
            form.remote_host.clone(),
        ),
        (
            TunnelFormField::RemotePort,
            "Dest port",
            form.remote_port.clone(),
        ),
        (TunnelFormField::Label, "Label", form.label.clone()),
    ];

    for (i, (field, name, value)) in fields.iter().enumerate() {
        let row_y = inner.y + i as u16;
        if row_y >= inner.y + inner.height {
            break;
        }

        let is_active = form.active_field == *field;

        // Field label
        let label_style = if is_active {
            theme::bright()
        } else {
            theme::mute()
        };
        buf.set_string(inner.x + 1, row_y, name, label_style);

        // Value
        let val_x = inner.x + 15;
        let val_w = inner.width.saturating_sub(16) as usize;
        let val_style = if is_active && form.editing {
            Style::default()
                .fg(theme::WHITE)
                .add_modifier(Modifier::UNDERLINED)
        } else if is_active {
            theme::bright()
        } else {
            theme::text()
        };

        let display = if value.is_empty() && !form.editing {
            "─"
        } else {
            value.as_str()
        };
        buf.set_string(val_x, row_y, truncate(display, val_w), val_style);

        // Arrow indicator for active field
        if is_active {
            buf.set_string(inner.x, row_y, "›", theme::green());
        }

        // Navigation hints for Type/Host fields
        if is_active && matches!(field, TunnelFormField::Type | TunnelFormField::Host) {
            let hint = "←/→";
            let hx = val_x + display.len() as u16 + 1;
            if hx + hint.len() as u16 <= inner.x + inner.width {
                buf.set_string(hx, row_y, hint, theme::dim());
            }
        }
    }

    // Footer hints
    let hint_y = inner.y + inner.height.saturating_sub(1);
    if hint_y > inner.y + fields.len() as u16 {
        let hint = format!("type to edit  Tab/\u{2193}: next  {}: save  Esc: close", app.save_key_label());
        buf.set_string(inner.x + 1, hint_y, hint, theme::dim());
    }
}

fn table_columns(total_w: u16) -> Vec<(&'static str, u16)> {
    if total_w >= 100 {
        vec![
            ("STATUS", 8),
            ("DIR", 4),
            ("LOCAL", 10),
            ("DEST", 22),
            ("SERVER", 20),
            ("LABEL", total_w.saturating_sub(64)),
        ]
    } else {
        vec![
            ("", 6),
            ("DIR", 4),
            ("LOCAL", 8),
            ("DEST", 18),
            ("SERVER", 14),
            ("LABEL", total_w.saturating_sub(50)),
        ]
    }
}

fn format_uptime(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    }
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
