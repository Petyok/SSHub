use ratatui::layout::Rect;
use ratatui::prelude::*;

use crate::app::{App, AuditFilter, AuditRange};
use crate::tui::theme;

pub fn render_audit(frame: &mut Frame, area: Rect, app: &App) {
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

    // Row 0: Filter + Range strip
    let filter_y = area.y;
    render_filter_strip(
        buf,
        inner_x,
        filter_y,
        inner_w,
        app.audit_filter,
        app.audit_range,
    );

    // Row 1: blank separator
    // Row 2: Table header
    let header_y = area.y + 2;
    if header_y >= area.y + area.height {
        return;
    }
    render_table_header(buf, inner_x, header_y, inner_w);

    // Row 3+: Data rows
    let data_y = header_y + 1;
    let max_rows = (area.y + area.height).saturating_sub(data_y) as usize;
    let events = &app.auth_events_cache;

    let scroll = if app.audit_selected >= max_rows {
        app.audit_selected - max_rows + 1
    } else {
        0
    };

    for (i, event) in events.iter().skip(scroll).take(max_rows).enumerate() {
        let y = data_y + i as u16;
        let row_idx = scroll + i;
        let is_selected = row_idx == app.audit_selected;
        render_event_row(buf, inner_x, y, inner_w, event, is_selected);
    }

    // Empty state
    if events.is_empty() {
        let msg = "No audit events";
        let x = inner_x + (inner_w.saturating_sub(msg.len() as u16)) / 2;
        let y = data_y + 2.min(max_rows.saturating_sub(1) as u16);
        buf.set_string(x, y, msg, theme::dim());
    }
}

fn render_filter_strip(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    w: u16,
    filter: AuditFilter,
    range: AuditRange,
) {
    let mut cx = x;

    buf.set_string(cx, y, "filter: ", theme::dim());
    cx += 8;

    for f in [AuditFilter::All, AuditFilter::Ok, AuditFilter::Fail] {
        let label = f.label();
        let style = if f == filter {
            theme::inv()
        } else {
            theme::dim()
        };
        buf.set_string(cx, y, label, style);
        cx += label.len() as u16 + 2;
    }

    cx += 2;
    buf.set_string(cx, y, "range: ", theme::dim());
    cx += 7;

    for r in [
        AuditRange::All,
        AuditRange::Today,
        AuditRange::Week,
        AuditRange::Month,
    ] {
        let label = r.label();
        let style = if r == range {
            theme::inv()
        } else {
            theme::dim()
        };
        buf.set_string(cx, y, label, style);
        cx += label.len() as u16 + 2;
        if cx >= x + w {
            break;
        }
    }
}

fn render_table_header(buf: &mut Buffer, x: u16, y: u16, w: u16) {
    let cols = table_columns(w);
    let mut cx = x;

    for (label, width) in &cols {
        buf.set_string(cx, y, label, theme::bright().add_modifier(Modifier::BOLD));
        cx += width;
    }

    // Underline
    if y + 1 < buf.area.y + buf.area.height {
        let line: String = std::iter::repeat_n('─', w as usize).collect();
        buf.set_string(x, y + 1, &line, theme::dim());
    }
}

fn render_event_row(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    w: u16,
    event: &crate::store::AuthEvent,
    selected: bool,
) {
    let base_style = if selected {
        theme::selected()
    } else {
        theme::text()
    };

    // Clear row with selection background
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

    // TIME
    let time_str = format_timestamp(event.created_at);
    let time_w = cols[0].1;
    buf.set_string(
        cx,
        y,
        truncate(&time_str, time_w as usize),
        if selected { base_style } else { theme::mute() },
    );
    cx += time_w;

    // HOST
    let host_w = cols[1].1;
    buf.set_string(
        cx,
        y,
        truncate(&event.host_name, host_w as usize),
        base_style,
    );
    cx += host_w;

    // USER
    let user_w = cols[2].1;
    let user = event.username.as_deref().unwrap_or("-");
    buf.set_string(
        cx,
        y,
        truncate(user, user_w as usize),
        if selected { base_style } else { theme::dim() },
    );
    cx += user_w;

    // VIA
    let via_w = cols[3].1;
    let via = event.via.as_deref().unwrap_or("direct");
    buf.set_string(
        cx,
        y,
        truncate(via, via_w as usize),
        if selected { base_style } else { theme::dim() },
    );
    cx += via_w;

    // RESULT (with status dot)
    let result_w = cols[4].1;
    let dot_color = theme::status_color(&event.status);
    let dot_style = if selected {
        Style::default().fg(dot_color).bg(theme::SEL_BG)
    } else {
        Style::default().fg(dot_color)
    };
    buf.set_string(cx, y, "●", dot_style);
    cx += 2;
    let status_label = match event.status.as_str() {
        "launched" => "ok",
        other => other,
    };
    buf.set_string(
        cx,
        y,
        truncate(status_label, (result_w - 2) as usize),
        if selected { base_style } else { theme::text() },
    );
    cx += result_w - 2;

    // NOTE
    let note = audit_note(event);
    let remaining = (x + w).saturating_sub(cx) as usize;
    buf.set_string(
        cx,
        y,
        truncate(&note, remaining),
        if selected { base_style } else { theme::dim() },
    );
}

fn table_columns(total_w: u16) -> Vec<(&'static str, u16)> {
    if total_w >= 100 {
        vec![
            ("TIME", 12),
            ("HOST", 24),
            ("USER", 12),
            ("VIA", 14),
            ("RESULT", 10),
            ("NOTE", total_w.saturating_sub(72)),
        ]
    } else {
        vec![
            ("TIME", 10),
            ("HOST", 18),
            ("USER", 10),
            ("VIA", 10),
            ("RESULT", 8),
            ("NOTE", total_w.saturating_sub(56)),
        ]
    }
}

fn audit_note(event: &crate::store::AuthEvent) -> String {
    match (&event.note, &event.log_path) {
        (Some(note), Some(path)) if note.is_empty() => path.clone(),
        (Some(note), Some(path)) => format!("{note} (logs in {path})"),
        (Some(note), None) => note.clone(),
        (None, Some(path)) => path.clone(),
        (None, None) => String::new(),
    }
}

fn format_timestamp(ts: i64) -> String {
    crate::tui::format_local_time(ts)
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
