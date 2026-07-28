use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::widgets::Clear;

use crate::app::App;
use crate::store::TunnelType;
use crate::theme::catalog::{ColorRole, PaintRole, StyleRole};
use crate::theme::model::ResolvedTheme;
use crate::tui::blit;
use crate::tui::theme;

/// One breath of a reconnecting tunnel's status dot (#35).
const TUNNEL_PULSE: std::time::Duration = std::time::Duration::from_millis(1100);

pub fn render_tunnels(frame: &mut Frame, area: Rect, app: &App) {
    if area.height < 4 || area.width < 20 {
        return;
    }

    let theme = app.theme();
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
    buf.set_string(
        inner_x,
        summary_y,
        &summary,
        theme.style(StyleRole::TunnelsSummary),
    );

    let mut body_y = summary_y + 2;
    if let Some(tunnel) = app.tunnels.get(app.tunnel_selected) {
        let status = app.tunnel_manager.status(tunnel.id);
        if matches!(status, "gave_up" | "error" | "reconnecting") {
            if let Some(detail) = app.tunnel_manager.error_detail(tunnel.id) {
                if !detail.is_empty() {
                    // Still retrying is a notice, not a failure: the tunnel is
                    // working its way back on its own.
                    let style = if status == "reconnecting" {
                        theme.style(StyleRole::TunnelsNotice)
                    } else {
                        theme.style(StyleRole::TunnelsError)
                    };
                    buf.set_string(
                        inner_x,
                        body_y,
                        crate::tui::text::ellipsize(&format!("error: {detail}"), inner_w as usize),
                        style,
                    );
                    body_y += 2;
                }
            }
        }
    }

    if let Some(ref notice) = app.tunnel_notice {
        let nx = inner_x + summary.len() as u16 + 3;
        if nx + notice.len() as u16 <= inner_x + inner_w {
            buf.set_string(nx, summary_y, notice, theme.style(StyleRole::TunnelsNotice));
        }
    }

    // Optional error detail + spacer, then table header
    let header_y = body_y;
    if header_y >= area.y + area.height {
        return;
    }
    render_table_header(buf, inner_x, header_y, inner_w, theme);

    // Row 3: separator line
    let sep_y = header_y + 1;
    if sep_y < area.y + area.height {
        // Its own rect, so a gradient separator runs across the rule and
        // nothing else. The tunnels tab is never drawn over the remote PTY.
        let rule = Rect::new(inner_x, sep_y, inner_w, 1);
        let line: String = std::iter::repeat_n('─', inner_w as usize).collect();
        buf.set_string(
            inner_x,
            sep_y,
            &line,
            Style::default().fg(blit::line_color(theme, PaintRole::TunnelsSeparator, rule)),
        );
        blit::paint_line(buf, rule, theme, PaintRole::TunnelsSeparator);
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
            app.tunnel_manager.reconnect_attempt(tunnel.id),
            app.tunnel_manager.reconnect_countdown_secs(tunnel.id),
            app.config.tunnel_reconnect.max_attempts,
            app.motion_enabled(),
            theme,
        );
    }

    // Empty state
    if app.tunnels.is_empty() {
        let msg = "No tunnels — press 'a' to add one";
        let x = inner_x + (inner_w.saturating_sub(msg.len() as u16)) / 2;
        let y = data_y + 2.min(max_rows.saturating_sub(1) as u16);
        buf.set_string(x, y, msg, theme.style(StyleRole::TunnelsEmpty));
    }
}

/// How one tunnel status presents itself: glyph, state role and short word.
///
/// Glyph *and* word, because a terminal that reduces the theme's RGB to the
/// nearest ANSI colour can make two states share a swatch.
struct TunnelPresentation {
    glyph: &'static str,
    role: ColorRole,
    word: &'static str,
}

fn tunnel_presentation(status: &str, motion: bool) -> TunnelPresentation {
    let (glyph, role, word) = match status {
        "up" => ("●", ColorRole::TunnelRunning, "up"),
        "reconnecting" => ("●", ColorRole::TunnelRetrying, "retry"),
        // Coming up: the same spinner every other in-flight handshake turns.
        "starting" if motion => (
            crate::tui::tween::spinner_frame_now(),
            ColorRole::TunnelConnecting,
            "start",
        ),
        "starting" => ("○", ColorRole::TunnelConnecting, "start"),
        "gave_up" => ("●", ColorRole::TunnelStopped, "gave up"),
        "error" => ("●", ColorRole::TunnelStopped, "err"),
        _ => ("○", ColorRole::TunnelUnknown, "off"),
    };
    TunnelPresentation { glyph, role, word }
}

fn render_table_header(buf: &mut Buffer, x: u16, y: u16, w: u16, theme: &ResolvedTheme) {
    let cols = table_columns(w);
    let mut cx = x;
    for (label, width) in &cols {
        buf.set_string(cx, y, label, theme.style(StyleRole::TunnelsTableHeader));
        cx += width;
    }
}

#[allow(clippy::too_many_arguments)]
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
    reconnect_attempt: Option<u32>,
    reconnect_countdown: Option<u64>,
    max_attempts: u32,
    motion: bool,
    theme: &ResolvedTheme,
) {
    // The cardless full-screen table has always highlighted with `selection_fg`
    // and has its own role for it — routing it through the generic
    // `table.row_selected` would hand it another family's idiom.
    let base_style = theme.style(if selected {
        StyleRole::TunnelsRowSelected
    } else {
        StyleRole::TunnelsRow
    });

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

    // STATUS: one decision for glyph, role and word, so the three can never
    // disagree about which state a row is in.
    let status_w = cols[0].1;
    let TunnelPresentation { glyph, role, word } = tunnel_presentation(status, motion);
    let dot_color = if status == "reconnecting" && motion {
        // A tunnel working its way back breathes rather than parking on amber
        // (#35). Both ends are roles, so a theme moves the whole pulse.
        crate::tui::tween::color_lerp(
            theme.color(role),
            theme.color(ColorRole::TunnelUnknown),
            crate::tui::tween::pulse_now(TUNNEL_PULSE),
        )
    } else {
        theme.color(role)
    };
    // Keeping the row's own ground under the dot stops a selected row floating
    // its status marker on the wrong background.
    let dot_style = if selected {
        base_style.fg(dot_color)
    } else {
        Style::default().fg(dot_color)
    };
    buf.set_string(cx, y, glyph, dot_style);
    let word_style = if selected {
        base_style
    } else {
        Style::default().fg(theme.color(role))
    };
    // The retry counter is the one word that carries live numbers.
    let retry_label = (status == "reconnecting").then(|| {
        let attempt = reconnect_attempt.unwrap_or(0).saturating_add(1);
        let base = if max_attempts > 0 {
            format!("retry {attempt}/{max_attempts}")
        } else {
            format!("retry {attempt}")
        };
        match reconnect_countdown {
            Some(secs) => format!("{base} {secs}s"),
            None => base,
        }
    });
    let budget = status_w.saturating_sub(2) as usize;
    let text = match retry_label.as_deref() {
        // The counter is hard-truncated, the fixed words ellipsize — as each
        // always did.
        Some(label) => truncate(label, budget).to_string(),
        None => crate::tui::text::ellipsize(word, budget),
    };
    buf.set_string(cx + 2, y, &text, word_style);
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
        if selected {
            base_style
        } else {
            theme.style(StyleRole::TunnelsDirection)
        },
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
        if selected {
            base_style
        } else {
            theme.style(StyleRole::TunnelsRemote)
        },
    );
    cx += remote_w;

    // HOST
    let host_w = cols[4].1;
    let host_label = host_name.unwrap_or("-");
    buf.set_string(
        cx,
        y,
        truncate(host_label, host_w as usize),
        if selected {
            base_style
        } else {
            theme.style(StyleRole::TunnelsMetadata)
        },
    );
    cx += host_w;

    // LABEL / UPTIME
    let remaining = (x + w).saturating_sub(cx) as usize;
    let mut label_str = if let Some(secs) = uptime {
        let label_part = tunnel.label.as_deref().unwrap_or("");
        if label_part.is_empty() {
            format_uptime(secs)
        } else {
            format!("{}  {}", label_part, format_uptime(secs))
        }
    } else {
        tunnel.label.as_deref().unwrap_or("").to_string()
    };
    if tunnel.auto_connect {
        if label_str.is_empty() {
            label_str = "keep-alive".into();
        } else {
            label_str.push_str("  keep-alive");
        }
    }
    buf.set_string(
        cx,
        y,
        crate::tui::text::ellipsize(&label_str, remaining),
        if selected {
            base_style
        } else {
            theme.style(StyleRole::TunnelsMetadata)
        },
    );
}

pub fn render_tunnel_form(frame: &mut Frame, app: &App) {
    let Some(form) = app.tunnel_form.as_ref() else {
        return;
    };

    let area = frame.area();
    let popup_width = 52u16.min(area.width.saturating_sub(4)).max(40);
    let popup_height = 16u16.min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    let popup_area = crate::tui::popup_open_rect(popup_area, app);

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
        (
            TunnelFormField::AutoConnect,
            "Keep alive",
            if form.auto_connect {
                "on (auto-start + reconnect)".into()
            } else {
                "off".into()
            },
        ),
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

        // Render the edit cursor in the active text field (Type/Host aren't text).
        let cursored = if is_active
            && matches!(
                field,
                TunnelFormField::LocalPort
                    | TunnelFormField::RemoteHost
                    | TunnelFormField::RemotePort
                    | TunnelFormField::Label
            ) {
            Some(crate::text_input::with_cursor(value, form.cursor))
        } else {
            None
        };
        let display = if let Some(c) = &cursored {
            c.as_str()
        } else if value.is_empty() && !form.editing {
            "─"
        } else {
            value.as_str()
        };
        buf.set_string(val_x, row_y, truncate(display, val_w), val_style);

        // Arrow indicator for active field
        if is_active {
            buf.set_string(inner.x, row_y, "›", theme::green());
        }

        // Navigation hints: Type cycles with ←/→; Host opens a picker.
        if is_active
            && matches!(
                field,
                TunnelFormField::Type | TunnelFormField::Host | TunnelFormField::AutoConnect
            )
        {
            let hint = match field {
                TunnelFormField::Host => "Enter: pick",
                TunnelFormField::AutoConnect => "Space: toggle",
                _ => "←/→",
            };
            let hx = val_x + display.len() as u16 + 1;
            if hx + hint.len() as u16 <= inner.x + inner.width {
                buf.set_string(hx, row_y, hint, theme::dim());
            }
        }
    }

    // Footer hints (two rows so Esc stays visible with long save bindings)
    let avail = inner.width.saturating_sub(2) as usize;
    let footer_top = inner.y + inner.height.saturating_sub(2);
    let footer_bottom = inner.y + inner.height.saturating_sub(1);
    if footer_top > inner.y + fields.len() as u16 {
        buf.set_string(
            inner.x + 1,
            footer_top,
            crate::tui::text::ellipsize("type to edit  Tab/\u{2193}: next field", avail),
            theme::dim(),
        );
        let save_esc = format!("{}: save", app.save_key_label());
        let esc = "Esc: close";
        let esc_len = esc.chars().count();
        let mut line = if save_esc.chars().count() + 2 + esc_len <= avail {
            format!("{save_esc}  {esc}")
        } else {
            let prefix = crate::tui::text::ellipsize(
                &format!("{save_esc}  "),
                avail.saturating_sub(esc_len),
            );
            format!("{prefix}{esc}")
        };
        if line.chars().count() > avail {
            line = esc.to_string();
        }
        buf.set_string(inner.x + 1, footer_bottom, line, theme::dim());
    }
}

/// Searchable dropdown for choosing the tunnel form's SSH server.
pub fn render_tunnel_host_picker(frame: &mut Frame, app: &App) {
    let Some(picker) = app.tunnel_host_picker.as_ref() else {
        return;
    };
    let matches = app.tunnel_host_matches();

    let area = frame.area();
    let popup_w = 44u16.min(area.width.saturating_sub(4)).max(30);
    // query line + separator + up to 8 rows + hint + borders.
    let list_rows = matches.len().clamp(1, 8) as u16;
    let popup_h = (list_rows + 5).min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_h)) / 2;
    let popup = Rect::new(x, y, popup_w, popup_h);

    let popup = crate::tui::popup_open_rect(popup, app);

    frame.render_widget(Clear, popup);
    frame.render_widget(
        ratatui::widgets::Block::default()
            .borders(ratatui::widgets::Borders::ALL)
            .title(Span::styled(" select SSH server ", theme::heading()))
            .border_style(Style::default().fg(theme::ACCENT)),
        popup,
    );

    let buf = frame.buffer_mut();
    let row_x = popup.x + 2;
    let inner_w = popup.width.saturating_sub(3) as usize;

    // Query line with a blinking-ish cursor block.
    let query_line = format!("/ {}\u{2588}", picker.query);
    buf.set_string(
        row_x,
        popup.y + 1,
        crate::tui::text::ellipsize(&query_line, inner_w),
        theme::bright(),
    );

    // Separator.
    let sep: String = std::iter::repeat_n('\u{2500}', inner_w).collect();
    buf.set_string(row_x, popup.y + 2, &sep, theme::dim());

    let list_top = popup.y + 3;
    let visible = popup.height.saturating_sub(5) as usize;
    if matches.is_empty() {
        buf.set_string(row_x, list_top, "(no matching hosts)", theme::mute());
    } else {
        // Scroll so the selection stays visible.
        let scroll = picker.selected.saturating_sub(visible.saturating_sub(1));
        for (i, (_, name)) in matches.iter().skip(scroll).take(visible).enumerate() {
            let idx = scroll + i;
            let ry = list_top + i as u16;
            let is_sel = idx == picker.selected;
            let style = if is_sel {
                theme::selected()
            } else {
                theme::text()
            };
            if is_sel {
                let blank = " ".repeat(popup.width.saturating_sub(2) as usize);
                buf.set_string(popup.x + 1, ry, &blank, theme::selected());
            }
            let marker = if is_sel { "\u{203a} " } else { "  " };
            buf.set_string(
                row_x,
                ry,
                crate::tui::text::ellipsize(&format!("{marker}{name}"), inner_w),
                style,
            );
        }
    }

    // Hint line.
    let hint_y = popup.y + popup.height.saturating_sub(2);
    buf.set_string(
        row_x,
        hint_y,
        crate::tui::text::ellipsize("type to filter · \u{2191}/\u{2193} · Enter · Esc", inner_w),
        theme::mute(),
    );
}

fn table_columns(total_w: u16) -> Vec<(&'static str, u16)> {
    if total_w >= 100 {
        vec![
            ("STATUS", 10),
            ("DIR", 4),
            ("LOCAL", 10),
            ("DEST", 22),
            ("SERVER", 20),
            ("LABEL", total_w.saturating_sub(66)),
        ]
    } else {
        vec![
            ("", 8),
            ("DIR", 4),
            ("LOCAL", 8),
            ("DEST", 18),
            ("SERVER", 14),
            ("LABEL", total_w.saturating_sub(52)),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Tunnel;
    use crate::test_support::{
        fg, fg_at_text, fg_bg, frame_at, marker, resolved_default, role_marker_theme, themed_app,
        RoleMarker,
    };
    use crate::theme::model::ResolvedTheme;

    // One unique colour per role this screen reads. Two roles never share a
    // value, so a row that reaches for its neighbour fails on the literal.
    const SUMMARY: u32 = 0xa1_0001;
    const HEADER: u32 = 0xa1_0002;
    const SEPARATOR: u32 = 0xa1_0003;
    const ROW: u32 = 0xa1_0004;
    const ROW_SEL_FG: u32 = 0xa1_0005;
    const ROW_SEL_BG: u32 = 0xa1_0105;
    const DIRECTION: u32 = 0xa1_0006;
    const REMOTE: u32 = 0xa1_0007;
    const METADATA: u32 = 0xa1_0008;
    const NOTICE: u32 = 0xa1_0009;
    const ERROR: u32 = 0xa1_000a;
    const EMPTY: u32 = 0xa1_000b;
    const RUNNING: u32 = 0xa1_000c;
    const STOPPED: u32 = 0xa1_000d;
    const RETRYING: u32 = 0xa1_000e;
    const CONNECTING: u32 = 0xa1_000f;
    const UNKNOWN: u32 = 0xa1_0010;

    const MARKERS: &[RoleMarker] = &[
        fg("components.tunnels.summary", SUMMARY),
        fg("components.tunnels.table_header", HEADER),
        fg("components.tunnels.separator", SEPARATOR),
        fg("components.tunnels.row", ROW),
        fg_bg("components.tunnels.row_selected", ROW_SEL_FG, ROW_SEL_BG),
        fg("components.tunnels.direction", DIRECTION),
        fg("components.tunnels.remote", REMOTE),
        fg("components.tunnels.metadata", METADATA),
        fg("components.tunnels.notice", NOTICE),
        fg("components.tunnels.error", ERROR),
        fg("components.tunnels.empty", EMPTY),
        fg("components.tunnel.running", RUNNING),
        fg("components.tunnel.stopped", STOPPED),
        fg("components.tunnel.retrying", RETRYING),
        fg("components.tunnel.connecting", CONNECTING),
        fg("components.tunnel.unknown", UNKNOWN),
    ];

    fn marked() -> ResolvedTheme {
        role_marker_theme("tunnels", MARKERS)
    }

    fn tunnel() -> Tunnel {
        Tunnel {
            id: 7,
            host_id: None,
            tunnel_type: TunnelType::Local,
            local_port: 8080,
            remote_host: "db.internal".into(),
            remote_port: 5432,
            label: Some("staging".into()),
            auto_connect: false,
            created_at: 0,
            updated_at: 0,
        }
    }

    /// Draw one row through the productive row renderer at a known origin.
    fn row_buffer(theme: &ResolvedTheme, status: &str, selected: bool) -> Buffer {
        let area = Rect::new(0, 0, 110, 1);
        let mut buf = Buffer::empty(area);
        render_tunnel_row(
            &mut buf,
            area.x,
            area.y,
            area.width,
            &tunnel(),
            selected,
            status,
            Some(90),
            Some("web-prod"),
            Some(0),
            Some(4),
            3,
            // Motion off: the retrying dot otherwise breathes between two
            // roles and its colour depends on the wall clock.
            false,
            theme,
        );
        buf
    }

    /// The status column of every tunnel state comes from `components.tunnel.*`
    /// — dot **and** label — while the rest of the row is `components.tunnels.*`.
    ///
    /// Both selection states are rendered for every status: the selected branch
    /// is a different code path and used to be the only one anybody looked at.
    #[test]
    fn every_tunnel_state_wears_its_own_status_colour() {
        let theme = marked();
        for (status, label, state) in [
            ("up", "up", RUNNING),
            ("reconnecting", "retry", RETRYING),
            ("starting", "start", CONNECTING),
            ("gave_up", "gave up", STOPPED),
            ("error", "err", STOPPED),
            ("stopped", "off", UNKNOWN),
        ] {
            for selected in [false, true] {
                let buf = row_buffer(&theme, status, selected);
                let dot = buf.cell((0, 0)).unwrap();
                assert_eq!(
                    dot.fg,
                    marker(state),
                    "{status} (selected={selected}): the status dot"
                );
                // A selected row keeps its selection background under the dot,
                // so the state colour never floats on the wrong ground.
                assert_eq!(
                    dot.bg,
                    if selected {
                        marker(ROW_SEL_BG)
                    } else {
                        Color::Reset
                    },
                    "{status} (selected={selected}): the status dot's ground"
                );
                assert_eq!(
                    fg_at_text(&buf, label),
                    if selected {
                        marker(ROW_SEL_FG)
                    } else {
                        marker(state)
                    },
                    "{status} (selected={selected}): the `{label}` label"
                );
            }
        }
    }

    /// The columns beside the status are the tab's own chrome, in both
    /// selection states.
    #[test]
    fn tunnel_row_columns_wear_the_tunnels_roles() {
        let theme = marked();

        let buf = row_buffer(&theme, "stopped", false);
        assert_eq!(fg_at_text(&buf, "L"), marker(DIRECTION), "the direction");
        assert_eq!(fg_at_text(&buf, ":8080"), marker(ROW), "the local port");
        assert_eq!(
            fg_at_text(&buf, "db.internal"),
            marker(REMOTE),
            "the destination"
        );
        assert_eq!(fg_at_text(&buf, "web-prod"), marker(METADATA), "the server");
        assert_eq!(fg_at_text(&buf, "staging"), marker(METADATA), "the label");

        // Selected: every column collapses onto the one selection role, and the
        // whole row carries its background — not `table.row_selected`.
        let buf = row_buffer(&theme, "stopped", true);
        for (needle, what) in [
            ("L", "the direction"),
            (":8080", "the local port"),
            ("db.internal", "the destination"),
            ("web-prod", "the server"),
            ("staging", "the label"),
        ] {
            assert_eq!(
                fg_at_text(&buf, needle),
                marker(ROW_SEL_FG),
                "selected: {what}"
            );
        }
        assert_eq!(
            buf.cell((109, 0)).unwrap().bg,
            marker(ROW_SEL_BG),
            "the selection bar reaches the last column"
        );
    }

    /// An app carrying `count` tunnels and the marker theme.
    fn tunnel_app(count: usize) -> crate::app::App {
        let mut app = themed_app(marked());
        app.tunnels = (0..count)
            .map(|i| Tunnel {
                id: i as i64 + 1,
                ..tunnel()
            })
            .collect();
        app
    }

    fn tab(app: &crate::app::App) -> Buffer {
        let area = Rect::new(0, 0, 110, 12);
        frame_at(area, |frame| render_tunnels(frame, area, app))
    }

    #[test]
    fn the_tunnel_tab_chrome_wears_the_tunnels_roles() {
        let app = tunnel_app(0);
        let buf = tab(&app);

        assert_eq!(
            fg_at_text(&buf, "0 tunnels"),
            marker(SUMMARY),
            "the summary"
        );
        assert_eq!(
            fg_at_text(&buf, "STATUS"),
            marker(HEADER),
            "a column header"
        );
        assert_eq!(
            fg_at_text(&buf, "\u{2500}"),
            marker(SEPARATOR),
            "the header rule"
        );
        assert_eq!(
            fg_at_text(&buf, "No tunnels"),
            marker(EMPTY),
            "the empty state"
        );
    }

    #[test]
    fn the_tunnel_tab_notice_is_the_notice_role() {
        let mut app = tunnel_app(1);
        app.tunnel_notice = Some("started".into());
        let buf = tab(&app);
        assert_eq!(fg_at_text(&buf, "started"), marker(NOTICE));
    }

    /// A tunnel still working its way back reads as a notice; one that has
    /// given up reads as an error. The two used to be amber and red by hand.
    #[test]
    fn the_error_line_separates_retrying_from_given_up() {
        let cfg = crate::config::TunnelReconnectConfig {
            max_attempts: 1,
            ..Default::default()
        };

        let mut app = tunnel_app(1);
        app.tunnel_manager.on_auto_start_failed(1, "boom", &cfg);
        assert_eq!(app.tunnel_manager.status(1), "reconnecting");
        let buf = tab(&app);
        assert_eq!(
            fg_at_text(&buf, "error: boom"),
            marker(NOTICE),
            "a reconnecting tunnel's detail"
        );

        // A second failure exhausts the single allowed attempt.
        app.tunnel_manager.on_auto_start_failed(1, "boom", &cfg);
        assert_eq!(app.tunnel_manager.status(1), "gave_up");
        let buf = tab(&app);
        assert_eq!(
            fg_at_text(&buf, "error: boom"),
            marker(ERROR),
            "a tunnel that gave up"
        );
    }

    /// Legacy parity, hand-transcribed from the `crate::tui::theme` calls this
    /// screen used before the migration — never derived from `ROLE_SPECS`,
    /// which is the same source the renderer resolves from.
    #[test]
    fn the_tunnel_tab_reproduces_its_legacy_cells_under_default() {
        use crate::tui::theme as legacy;
        let theme = resolved_default();

        let app = {
            let mut app = themed_app(resolved_default());
            app.tunnels = vec![Tunnel { id: 1, ..tunnel() }];
            app.tunnel_notice = Some("started".into());
            app
        };
        let buf = tab(&app);
        assert_eq!(fg_at_text(&buf, "1 tunnels"), legacy::MUTE, "theme::mute()");
        assert_eq!(fg_at_text(&buf, "started"), legacy::AMBER, "theme::amber()");
        let (hx, hy) = crate::test_support::find_text(&buf, "STATUS");
        let head = buf.cell((hx, hy)).unwrap();
        assert_eq!(head.fg, legacy::BRIGHT, "theme::bright()");
        assert!(
            head.modifier.contains(Modifier::BOLD),
            "the column header kept `theme::heading()`'s weight"
        );
        assert_eq!(
            fg_at_text(&buf, "\u{2500}"),
            legacy::DIM,
            "theme::dim() rule"
        );

        // The row, both states.
        let unselected = row_buffer(&theme, "up", false);
        assert_eq!(unselected.cell((0, 0)).unwrap().fg, legacy::GREEN);
        assert_eq!(fg_at_text(&unselected, "up"), legacy::GREEN);
        assert_eq!(fg_at_text(&unselected, "L"), legacy::CYAN, "theme::cyan()");
        assert_eq!(fg_at_text(&unselected, ":8080"), legacy::TEXT);
        assert_eq!(fg_at_text(&unselected, "db.internal"), legacy::MUTE);
        assert_eq!(fg_at_text(&unselected, "web-prod"), legacy::DIM);

        let selected = row_buffer(&theme, "up", true);
        let cell = selected.cell((0, 0)).unwrap();
        assert_eq!(cell.fg, legacy::GREEN, "the dot keeps its state colour");
        assert_eq!(cell.bg, legacy::SEL_BG);
        assert_eq!(fg_at_text(&selected, ":8080"), legacy::SEL_FG);

        // The four remaining state colours, each on its own row.
        for (status, label, expected) in [
            ("reconnecting", "retry", legacy::AMBER),
            ("starting", "start", legacy::AMBER),
            ("gave_up", "gave up", legacy::RED),
            ("stopped", "off", legacy::DIM),
        ] {
            let buf = row_buffer(&theme, status, false);
            assert_eq!(buf.cell((0, 0)).unwrap().fg, expected, "{status}: the dot");
            assert_eq!(fg_at_text(&buf, label), expected, "{status}: the label");
        }
    }
}
