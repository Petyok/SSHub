use ratatui::layout::Rect;
use ratatui::prelude::*;

use crate::app::{App, AuditFilter, AuditRange};
use crate::theme::catalog::{ColorRole, PaintRole, StyleRole};
use crate::theme::model::ResolvedTheme;
use crate::tui::blit;

/// The audit colour a status string maps to.
///
/// The audit tab is the only productive reader of the string-to-status mapping,
/// so it owns it rather than the global `components.status.*` family.
fn status_role(status: &str) -> ColorRole {
    match status {
        "ok" | "launched" | "online" | "up" => ColorRole::AuditSuccess,
        "slow" | "idle" | "retry" | "warning" => ColorRole::AuditWarning,
        "down" | "fail" | "error" | "unreachable" => ColorRole::AuditError,
        _ => ColorRole::AuditUnknown,
    }
}

pub fn render_audit(frame: &mut Frame, area: Rect, app: &App) {
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

    // Row 0: Filter + Range strip
    let filter_y = area.y;
    render_filter_strip(
        buf,
        inner_x,
        filter_y,
        inner_w,
        app.audit_filter,
        app.audit_range,
        theme,
    );

    let mut body_y = filter_y + 2;
    if let Some(event) = app.auth_events_cache.get(app.audit_selected) {
        let note = audit_note(event);
        if !note.is_empty() {
            buf.set_string(
                inner_x,
                body_y,
                crate::tui::text::ellipsize(&format!("note: {note}"), inner_w as usize),
                note_detail_style(&event.status, theme),
            );
            body_y += 2;
        }
    }

    // Table header (after optional note detail + spacer)
    let header_y = body_y;
    if header_y >= area.y + area.height {
        return;
    }
    render_table_header(buf, inner_x, header_y, inner_w, theme);

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
        render_event_row(buf, inner_x, y, inner_w, event, is_selected, theme);
    }

    // Empty state
    if events.is_empty() {
        let msg = "No audit events";
        let x = inner_x + (inner_w.saturating_sub(msg.len() as u16)) / 2;
        let y = data_y + 2.min(max_rows.saturating_sub(1) as u16);
        buf.set_string(x, y, msg, theme.style(StyleRole::TextDim));
    }

    // Everything below the filter strip is the query's result, so a changed
    // filter or range swaps all of it. Fade it up, leaving the strip itself
    // (which the user just interacted with) solid (#35).
    let fade =
        crate::tui::widgets::middle_stack::content_fade(app.audit_filter_at, app.motion_enabled());
    let rows_top = filter_y + 2;
    if fade < 1.0 && area.height > rows_top - area.y {
        let rows = Rect::new(
            area.x,
            rows_top,
            area.width,
            (area.y + area.height).saturating_sub(rows_top),
        );
        crate::tui::blit::fade(
            buf,
            rows,
            fade,
            crate::tui::blit::FadeGround {
                theme: app.theme(),
                role: crate::theme::catalog::PaintRole::AppBackground,
                // Sampled against the frame the role belongs to, not against
                // the band of rows this fade happens to cover.
                paint_area: buf.area,
                exclusions: &[],
            },
        );
    }
}

fn render_filter_strip(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    w: u16,
    filter: AuditFilter,
    range: AuditRange,
    theme: &ResolvedTheme,
) {
    let mut cx = x;
    let caption = theme.style(StyleRole::TextDim);
    let chip = |on: bool| {
        theme.style(if on {
            StyleRole::AuditFilterActive
        } else {
            StyleRole::AuditFilterInactive
        })
    };

    buf.set_string(cx, y, "filter: ", caption);
    cx += 8;

    for f in [AuditFilter::All, AuditFilter::Ok, AuditFilter::Fail] {
        let label = f.label();
        buf.set_string(cx, y, label, chip(f == filter));
        cx += label.len() as u16 + 2;
    }

    cx += 2;
    buf.set_string(cx, y, "range: ", caption);
    cx += 7;

    for r in [
        AuditRange::All,
        AuditRange::Today,
        AuditRange::Week,
        AuditRange::Month,
    ] {
        let label = r.label();
        buf.set_string(cx, y, label, chip(r == range));
        cx += label.len() as u16 + 2;
        if cx >= x + w {
            break;
        }
    }
}

fn render_table_header(buf: &mut Buffer, x: u16, y: u16, w: u16, theme: &ResolvedTheme) {
    let cols = table_columns(w);
    let mut cx = x;

    for (label, width) in &cols {
        buf.set_string(cx, y, label, theme.style(StyleRole::AuditTableHeader));
        cx += width;
    }

    // Underline — the tab's inner divider, on its own rect so a gradient runs
    // across the rule alone. The audit tab is never drawn over the remote PTY.
    if y + 1 < buf.area.y + buf.area.height {
        let rule = Rect::new(x, y + 1, w, 1);
        let line: String = std::iter::repeat_n('─', w as usize).collect();
        buf.set_string(
            x,
            y + 1,
            &line,
            Style::default().fg(blit::line_color(theme, PaintRole::SeparatorSecondary, rule)),
        );
        blit::paint_line(buf, rule, theme, PaintRole::SeparatorSecondary);
    }
}

fn render_event_row(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    w: u16,
    event: &crate::store::AuthEvent,
    selected: bool,
    theme: &ResolvedTheme,
) {
    let base_style = theme.style(if selected {
        StyleRole::AuditRowSelected
    } else {
        StyleRole::AuditRow
    });

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
        if selected {
            base_style
        } else {
            theme.style(StyleRole::TextMuted)
        },
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
        if selected {
            base_style
        } else {
            theme.style(StyleRole::TextDim)
        },
    );
    cx += user_w;

    // VIA
    let via_w = cols[3].1;
    let via = event.via.as_deref().unwrap_or("direct");
    buf.set_string(
        cx,
        y,
        truncate(via, via_w as usize),
        if selected {
            base_style
        } else {
            theme.style(StyleRole::TextDim)
        },
    );
    cx += via_w;

    // RESULT (with status dot)
    let result_w = cols[4].1;
    let dot_color = theme.color(status_role(&event.status));
    // Keep the state colour on the row's own ground, so a selected row's dot
    // does not float on the wrong background.
    let dot_style = if selected {
        base_style.fg(dot_color)
    } else {
        Style::default().fg(dot_color)
    };
    buf.set_string(cx, y, "●", dot_style);
    cx += 2;
    // The word stays beside the dot: a terminal that reduces the theme's RGB
    // can make two states share a swatch, and the label still says which.
    let status_label = match event.status.as_str() {
        "launched" => "ok",
        other => other,
    };
    buf.set_string(
        cx,
        y,
        truncate(status_label, (result_w - 2) as usize),
        base_style,
    );
}

/// The note line's style: `components.audit.note` for everything the theme can
/// give it, with the foreground it has always had — the event's own status
/// colour. Composing the two is what keeps the note independently themeable
/// *and* keeps a failed note red under `default`.
fn note_detail_style(status: &str, theme: &ResolvedTheme) -> Style {
    theme
        .style(StyleRole::AuditNote)
        .fg(theme.color(status_role(status)))
}

fn table_columns(total_w: u16) -> Vec<(&'static str, u16)> {
    if total_w >= 100 {
        vec![
            ("TIME", 12),
            ("HOST", 30),
            ("USER", 14),
            ("VIA", 16),
            ("RESULT", total_w.saturating_sub(72)),
        ]
    } else {
        vec![
            ("TIME", 10),
            ("HOST", 20),
            ("USER", 10),
            ("VIA", 12),
            ("RESULT", total_w.saturating_sub(52)),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::AuthEvent;

    fn sample_event(note: Option<&str>, log_path: Option<&str>) -> AuthEvent {
        AuthEvent {
            id: 1,
            host_name: "web".into(),
            username: Some("deploy".into()),
            via: Some("direct".into()),
            status: "launched".into(),
            note: note.map(str::to_string),
            log_path: log_path.map(str::to_string),
            created_at: 0,
        }
    }

    #[test]
    fn audit_note_appends_log_dir_to_session_started() {
        let dir = "/home/user/.local/share/sshub/logs/web_prod-42";
        let event = sample_event(Some("session started"), Some(dir));
        assert_eq!(
            audit_note(&event),
            format!("session started (logs in {dir})")
        );
    }

    #[test]
    fn audit_note_uses_path_when_note_empty() {
        let dir = "/tmp/sshub/logs/web/";
        let event = sample_event(Some(""), Some(dir));
        assert_eq!(audit_note(&event), dir);
    }

    #[test]
    fn audit_note_note_only_without_log_path() {
        let event = sample_event(Some("spawn failed"), None);
        assert_eq!(audit_note(&event), "spawn failed");
    }

    // ── Role coverage ────────────────────────────────────────
    //
    // Every role below is proved with a colour no other role in this screen
    // carries, driven through `render_audit` itself: under `default` a role
    // that is never read produces the same cell as one that is, so parity
    // alone could not tell the two apart.

    use crate::test_support::{
        fg, fg_at_text, fg_bg, frame_at, marker, resolved_default, role_marker_theme, themed_app,
        RoleMarker,
    };
    use crate::theme::model::ResolvedTheme;

    const FILTER_ACTIVE: u32 = 0xa2_0001;
    const FILTER_ACTIVE_BG: u32 = 0xa2_0101;
    const FILTER_INACTIVE: u32 = 0xa2_0002;
    const NOTE: u32 = 0xa2_0003;
    const TABLE_HEADER: u32 = 0xa2_0004;
    const ROW: u32 = 0xa2_0005;
    const ROW_SEL: u32 = 0xa2_0006;
    const ROW_SEL_BG: u32 = 0xa2_0106;
    const SUCCESS: u32 = 0xa2_0007;
    const WARNING: u32 = 0xa2_0008;
    const ERROR: u32 = 0xa2_0009;
    const UNKNOWN: u32 = 0xa2_000a;
    const RULE: u32 = 0xa2_000b;
    const MUTED: u32 = 0xa2_000c;
    const DIM: u32 = 0xa2_000d;

    const MARKERS: &[RoleMarker] = &[
        fg_bg(
            "components.audit.filter_active",
            FILTER_ACTIVE,
            FILTER_ACTIVE_BG,
        ),
        fg("components.audit.filter_inactive", FILTER_INACTIVE),
        fg("components.audit.note", NOTE),
        fg("components.audit.table_header", TABLE_HEADER),
        fg("components.audit.row", ROW),
        fg_bg("components.audit.row_selected", ROW_SEL, ROW_SEL_BG),
        fg("components.audit.success", SUCCESS),
        fg("components.audit.warning", WARNING),
        fg("components.audit.error", ERROR),
        fg("components.audit.unknown", UNKNOWN),
        fg("components.separator.secondary", RULE),
        fg("components.text.muted", MUTED),
        fg("components.text.dim", DIM),
    ];

    fn marked() -> ResolvedTheme {
        role_marker_theme("audit", MARKERS)
    }

    fn event(status: &str, note: Option<&str>) -> AuthEvent {
        AuthEvent {
            id: 1,
            host_name: "web-prod".into(),
            username: Some("deploy".into()),
            via: Some("bastion".into()),
            status: status.into(),
            note: note.map(str::to_string),
            log_path: None,
            created_at: 0,
        }
    }

    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        width: 110,
        height: 14,
    };

    fn audit_app(theme: ResolvedTheme, events: Vec<AuthEvent>) -> crate::app::App {
        let mut app = themed_app(theme);
        app.auth_events_cache = events;
        app
    }

    fn audit(app: &crate::app::App) -> Buffer {
        frame_at(AREA, |frame| render_audit(frame, AREA, app))
    }

    // The strip's geometry, mirrored from `render_filter_strip` rather than
    // searched for: "all" appears twice on the row, once per group.
    const INNER_X: u16 = 1; // 110 columns → margin 1
    const FILTER_ALL_X: u16 = INNER_X + 8;
    const FILTER_OK_X: u16 = FILTER_ALL_X + 3 + 2;
    const RANGE_ALL_X: u16 = FILTER_OK_X + 2 + 2 + 4 + 2 + 2 + 7;
    const RANGE_24H_X: u16 = RANGE_ALL_X + 3 + 2;

    /// Both filter groups mark their active chip with one role and every other
    /// chip with another — in both groups, so neither can pass by accident.
    #[test]
    fn the_audit_filter_strip_wears_its_two_chip_roles() {
        let app = audit_app(marked(), vec![]);
        let buf = audit(&app);

        let active = buf.cell((FILTER_ALL_X, 0)).unwrap();
        assert_eq!(active.fg, marker(FILTER_ACTIVE), "the active filter chip");
        assert_eq!(
            active.bg,
            marker(FILTER_ACTIVE_BG),
            "the active chip's ground"
        );
        assert_eq!(
            buf.cell((FILTER_OK_X, 0)).unwrap().fg,
            marker(FILTER_INACTIVE),
            "an inactive filter chip"
        );
        assert_eq!(
            buf.cell((RANGE_ALL_X, 0)).unwrap().fg,
            marker(FILTER_ACTIVE),
            "the active range chip"
        );
        assert_eq!(
            buf.cell((RANGE_24H_X, 0)).unwrap().fg,
            marker(FILTER_INACTIVE),
            "an inactive range chip"
        );
        // The two group captions are body text, not chips.
        assert_eq!(
            buf.cell((INNER_X, 0)).unwrap().fg,
            marker(DIM),
            "the `filter:` caption"
        );
    }

    /// Column headers, the rule under them, and the empty state.
    #[test]
    fn the_audit_table_chrome_wears_its_roles() {
        let app = audit_app(marked(), vec![]);
        let buf = audit(&app);

        assert_eq!(fg_at_text(&buf, "TIME"), marker(TABLE_HEADER));
        assert_eq!(
            fg_at_text(&buf, "\u{2500}"),
            marker(RULE),
            "the rule under the headers"
        );
        assert_eq!(
            fg_at_text(&buf, "No audit events"),
            marker(DIM),
            "the empty state"
        );
    }

    /// Every column of a row, selected and not. The selection role owns the
    /// whole bar; the status dot keeps its own colour on that ground.
    #[test]
    fn audit_rows_wear_their_row_roles_in_both_states() {
        let app = audit_app(marked(), vec![event("launched", None), event("fail", None)]);
        let buf = audit(&app);

        // Row 0 is selected (`audit_selected` defaults to 0), row 1 is not.
        let (_, sel_y) = crate::test_support::find_text(&buf, "deploy");
        let row = |y: u16, x: u16| buf.cell((x, y)).unwrap();

        // TIME 12, HOST 30, USER 14, VIA 16 → the RESULT dot at +72.
        assert_eq!(
            row(sel_y, INNER_X + 12).fg,
            marker(ROW_SEL),
            "selected host"
        );
        assert_eq!(row(sel_y, INNER_X + 12).bg, marker(ROW_SEL_BG));
        assert_eq!(row(sel_y, INNER_X).fg, marker(ROW_SEL), "selected time");
        assert_eq!(
            row(sel_y, INNER_X + 42).fg,
            marker(ROW_SEL),
            "selected user"
        );
        assert_eq!(
            row(sel_y, INNER_X + 72).fg,
            marker(SUCCESS),
            "the selected row's status dot keeps its own colour"
        );
        assert_eq!(
            row(sel_y, INNER_X + 72).bg,
            marker(ROW_SEL_BG),
            "…on the selection ground"
        );

        let un_y = sel_y + 1;
        assert_eq!(row(un_y, INNER_X + 12).fg, marker(ROW), "unselected host");
        assert_eq!(row(un_y, INNER_X).fg, marker(MUTED), "unselected time");
        assert_eq!(row(un_y, INNER_X + 42).fg, marker(DIM), "unselected user");
        assert_eq!(
            row(un_y, INNER_X + 56).fg,
            marker(DIM),
            "unselected via column"
        );
        assert_eq!(
            row(un_y, INNER_X + 72).fg,
            marker(ERROR),
            "a failed row's status dot"
        );
    }

    /// The four status colours, each on the dot **and** on the note line.
    #[test]
    fn every_audit_status_wears_its_own_colour() {
        for (status, colour) in [
            ("launched", SUCCESS),
            ("retry", WARNING),
            ("fail", ERROR),
            ("bogus", UNKNOWN),
        ] {
            let app = audit_app(marked(), vec![event(status, Some("why it happened"))]);
            let buf = audit(&app);
            let (_, y) = crate::test_support::find_text(&buf, "deploy");
            assert_eq!(
                buf.cell((INNER_X + 72, y)).unwrap().fg,
                marker(colour),
                "{status}: the status dot"
            );
            // The note keeps the status colour it always had, on the note
            // role's own ground.
            let note = crate::test_support::find_text(&buf, "note: why it happened");
            assert_eq!(
                buf.cell(note).unwrap().fg,
                marker(colour),
                "{status}: the note line's colour"
            );
        }
    }

    /// The note line reads `components.audit.note` for everything except its
    /// foreground, which is the status colour it has always carried.
    #[test]
    fn the_audit_note_line_reads_its_own_role() {
        let theme = role_marker_theme(
            "audit-note",
            &[
                fg_bg("components.audit.note", NOTE, ROW_SEL_BG),
                fg("components.audit.success", SUCCESS),
            ],
        );
        let app = audit_app(theme, vec![event("launched", Some("why it happened"))]);
        let buf = audit(&app);
        let at = crate::test_support::find_text(&buf, "note: why");
        assert_eq!(buf.cell(at).unwrap().bg, marker(ROW_SEL_BG));
        assert_eq!(buf.cell(at).unwrap().fg, marker(SUCCESS));
    }

    /// Legacy parity, hand-transcribed from the `crate::tui::theme` calls this
    /// screen made before the migration.
    #[test]
    fn the_audit_tab_reproduces_its_legacy_cells_under_default() {
        use crate::tui::theme as legacy;

        let app = audit_app(
            resolved_default(),
            vec![
                event("fail", Some("host unreachable")),
                event("retry", None),
            ],
        );
        let buf = audit(&app);

        // `theme::inv()` — deep ground on bright text, not the other way round.
        let chip = buf.cell((FILTER_ALL_X, 0)).unwrap();
        assert_eq!(chip.fg, legacy::BG_DEEP);
        assert_eq!(chip.bg, legacy::BRIGHT);
        assert_eq!(buf.cell((FILTER_OK_X, 0)).unwrap().fg, legacy::DIM);

        let head = crate::test_support::find_text(&buf, "TIME");
        assert_eq!(buf.cell(head).unwrap().fg, legacy::BRIGHT);
        assert!(
            buf.cell(head).unwrap().modifier.contains(Modifier::BOLD),
            "the column headers kept `theme::heading()`'s weight"
        );

        assert_eq!(
            fg_at_text(&buf, "note: host unreachable"),
            legacy::RED,
            "a failed note was theme::red()"
        );

        let (_, sel_y) = crate::test_support::find_text(&buf, "deploy");
        assert_eq!(buf.cell((INNER_X + 12, sel_y)).unwrap().fg, legacy::SEL_FG);
        assert_eq!(buf.cell((INNER_X + 12, sel_y)).unwrap().bg, legacy::SEL_BG);
        assert_eq!(buf.cell((INNER_X + 72, sel_y)).unwrap().fg, legacy::RED);

        let un_y = sel_y + 1;
        assert_eq!(buf.cell((INNER_X, un_y)).unwrap().fg, legacy::MUTE);
        assert_eq!(buf.cell((INNER_X + 12, un_y)).unwrap().fg, legacy::TEXT);
        assert_eq!(buf.cell((INNER_X + 42, un_y)).unwrap().fg, legacy::DIM);
        assert_eq!(
            buf.cell((INNER_X + 72, un_y)).unwrap().fg,
            legacy::AMBER,
            "a retry dot was theme::amber()"
        );

        // The rule is only visible where no row covers it, so read it from a
        // tab with nothing to list.
        let empty = audit_app(resolved_default(), vec![]);
        let buf = audit(&empty);
        assert_eq!(fg_at_text(&buf, "\u{2500}"), legacy::DIM);
        assert_eq!(fg_at_text(&buf, "No audit events"), legacy::DIM);
    }
}
