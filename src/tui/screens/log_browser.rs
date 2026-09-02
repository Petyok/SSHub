use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::app::{App, LogBrowserView};
use crate::theme::catalog::{ColorRole, StyleRole};

/// Render the session-log browser overlay for whichever pane is active.
pub fn render(frame: &mut Frame, app: &App) {
    let Some(state) = app.log_browser.as_ref() else {
        return;
    };

    let area = frame.area();
    let width = crate::tui::fit_popup(area.width * 80 / 100, 60, area.width.saturating_sub(2));
    let height = crate::tui::fit_popup(area.height * 80 / 100, 12, area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let popup = crate::tui::popup_open_rect(Rect::new(x, y, width, height), app);

    let theme = app.theme();
    crate::tui::open_popup(frame, popup, theme);

    let title = match state.view {
        LogBrowserView::Hosts => " Session logs ".to_string(),
        LogBrowserView::Segments => {
            format!(" Logs · {} ", state.current_host.as_deref().unwrap_or(""))
        }
        LogBrowserView::Viewer => format!(
            " Logs · {} / {} ",
            state.current_host.as_deref().unwrap_or(""),
            state.current_seg.as_deref().unwrap_or("")
        ),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(crate::tui::popup_border_style(theme, popup))
        .title(Span::styled(title, theme.style(StyleRole::PopupTitle)));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    crate::tui::paint_popup_border(frame, popup, theme);

    if inner.height < 2 || inner.width == 0 {
        return;
    }

    match state.view {
        LogBrowserView::Hosts => render_hosts(frame, app, inner),
        LogBrowserView::Segments => render_segments(frame, app, inner),
        LogBrowserView::Viewer => render_viewer(frame, app, inner),
    }
}

fn footer(frame: &mut Frame, app: &App, inner: Rect, text: &str) {
    let theme = app.theme();
    let area = Rect::new(inner.x, inner.y + inner.height - 1, inner.width, 1);
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" {text}"),
            theme.style(StyleRole::PopupHint),
        )),
        area,
    );
}

fn notice_line(frame: &mut Frame, app: &App, inner: Rect, row: u16) {
    if let Some(notice) = app.log_browser.as_ref().and_then(|s| s.notice.as_ref()) {
        let theme = app.theme();
        let area = Rect::new(inner.x, inner.y + row, inner.width, 1);
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(" {notice}"),
                Style::default().fg(theme.color(ColorRole::StatusInfo)),
            )),
            area,
        );
    }
}

fn render_hosts(frame: &mut Frame, app: &App, inner: Rect) {
    let theme = app.theme();
    let s = app.log_browser.as_ref().unwrap();
    let list_area = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(1),
    );

    if s.hosts.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " No session logs yet. Enable logging in Settings (Ctrl+H), connect, and they land here.",
                theme.style(StyleRole::PopupLegend),
            )),
            list_area,
        );
    } else {
        let legend = theme.style(StyleRole::PopupLegend);
        let items: Vec<ListItem> = s
            .hosts
            .iter()
            .map(|h| {
                ListItem::new(Line::from(vec![
                    Span::styled(format!(" {}", h.dir_name), theme.style(StyleRole::TableRow)),
                    Span::styled(
                        format!(
                            "  ({} segment{}, {})",
                            h.segment_count,
                            if h.segment_count == 1 { "" } else { "s" },
                            human_bytes(h.total_bytes)
                        ),
                        legend,
                    ),
                ]))
            })
            .collect();
        let mut st = ListState::default();
        st.select(Some(s.host_sel.min(s.hosts.len() - 1)));
        frame.render_stateful_widget(
            List::new(items).highlight_style(theme.style(StyleRole::TableRowSelected)),
            list_area,
            &mut st,
        );
    }
    footer(
        frame,
        app,
        inner,
        "\u{2191}\u{2193} move · Enter open · Esc close",
    );
}

fn render_segments(frame: &mut Frame, app: &App, inner: Rect) {
    let theme = app.theme();
    let s = app.log_browser.as_ref().unwrap();
    // Reserve the footer row, plus a notice row above it when one is pending
    // (e.g. opening a segment that turned out to be gone).
    let notice_h = u16::from(s.notice.is_some());
    let list_area = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(1 + notice_h),
    );
    if notice_h == 1 && inner.height >= 2 {
        notice_line(frame, app, inner, inner.height - 2);
    }

    if s.segments.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " No segments.",
                theme.style(StyleRole::PopupLegend),
            )),
            list_area,
        );
    } else {
        let legend = theme.style(StyleRole::PopupLegend);
        let items: Vec<ListItem> = s
            .segments
            .iter()
            .map(|seg| {
                ListItem::new(Line::from(vec![
                    Span::styled(format!(" {:>9}  ", human_bytes(seg.bytes)), legend),
                    Span::styled(seg.file_name.clone(), theme.style(StyleRole::TableRow)),
                ]))
            })
            .collect();
        let mut st = ListState::default();
        st.select(Some(s.seg_sel.min(s.segments.len() - 1)));
        frame.render_stateful_widget(
            List::new(items).highlight_style(theme.style(StyleRole::TableRowSelected)),
            list_area,
            &mut st,
        );
    }
    footer(
        frame,
        app,
        inner,
        "\u{2191}\u{2193} move · Enter view · Esc back",
    );
}

fn render_viewer(frame: &mut Frame, app: &App, inner: Rect) {
    let theme = app.theme();
    let s = app.log_browser.as_ref().unwrap();
    let legend = theme.style(StyleRole::PopupLegend);

    // Row 0: search / status line.
    let status_area = Rect::new(inner.x, inner.y, inner.width, 1);
    let status = if s.searching {
        Line::from(vec![
            Span::styled(" search: ", theme.style(StyleRole::PopupHint)),
            Span::styled(s.query.clone(), theme.style(StyleRole::CommandPaletteQuery)),
            Span::styled(
                "\u{2588}",
                Style::default().fg(theme.color(ColorRole::StatusSuccess)),
            ),
        ])
    } else if let Some(name) = &s.naming {
        Line::from(vec![
            Span::styled(" bookmark name: ", theme.style(StyleRole::PopupHint)),
            Span::styled(name.clone(), theme.style(StyleRole::CommandPaletteQuery)),
            Span::styled(
                "\u{2588}",
                Style::default().fg(theme.color(ColorRole::StatusSuccess)),
            ),
        ])
    } else {
        let mut spans = vec![Span::styled(
            format!(" line {}/{}", s.scroll + 1, s.lines.len().max(1)),
            legend,
        )];
        if !s.query.is_empty() {
            spans.push(Span::styled(
                format!("   /{}  ({} match)", s.query, s.matches.len()),
                legend,
            ));
        }
        if s.truncated {
            spans.push(Span::styled(
                "   [truncated]",
                Style::default().fg(theme.color(ColorRole::StatusInfo)),
            ));
        }
        Line::from(spans)
    };
    frame.render_widget(Paragraph::new(status), status_area);

    // Content window between the status row and the footer.
    let content_h = inner.height.saturating_sub(2);
    let content_area = Rect::new(inner.x, inner.y + 1, inner.width, content_h);
    let start = s.scroll.min(s.lines.len().saturating_sub(1));
    let needle = s.query.trim().to_lowercase();
    let rows: Vec<Line> = s
        .lines
        .iter()
        .enumerate()
        .skip(start)
        .take(content_h as usize)
        .map(|(i, line)| {
            let is_match = !needle.is_empty() && line.to_lowercase().contains(&needle);
            let num_style = legend;
            let text_style = if i == s.scroll {
                theme.style(StyleRole::TableRowSelected)
            } else if is_match {
                Style::default()
                    .fg(theme.color(ColorRole::StatusSuccess))
                    .add_modifier(Modifier::BOLD)
            } else {
                theme.style(StyleRole::TableRow)
            };
            Line::from(vec![
                Span::styled(format!(" {:>5} ", i + 1), num_style),
                Span::styled(line.clone(), text_style),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(rows), content_area);

    // Bookmarks list floats over the content when open.
    if s.show_bookmarks {
        render_bookmarks_overlay(frame, app, content_area);
    }

    let hint = if s.naming.is_some() {
        "type a name · Enter save · Esc cancel"
    } else if s.searching {
        "type to search · Enter run · Esc cancel"
    } else if s.show_bookmarks {
        "\u{2191}\u{2193} move · Enter jump · d delete · Esc close"
    } else {
        "\u{2191}\u{2193}/PgUp/PgDn scroll · / search · n/N next · b bookmark · m list · Esc back"
    };
    footer(frame, app, inner, hint);
    // A transient notice sits just above the footer.
    if app.log_browser.as_ref().is_some_and(|s| s.notice.is_some()) && inner.height >= 3 {
        notice_line(frame, app, inner, inner.height - 2);
    }
}

fn render_bookmarks_overlay(frame: &mut Frame, app: &App, over: Rect) {
    let theme = app.theme();
    let s = app.log_browser.as_ref().unwrap();
    let marks: Vec<(String, i64)> = s
        .bookmarks
        .iter()
        .filter(|b| Some(&b.host_dir) == s.current_host.as_ref())
        .map(|b| (b.name.clone(), b.line + 1))
        .collect();

    // Nothing sensible fits in a sliver, and clamping with a min above the
    // available size would trip u16::clamp's `min <= max` assert and abort the
    // whole TUI, so bail on a too-small area and keep every minimum subordinate.
    if over.width < 4 || over.height < 3 {
        return;
    }
    let w = (over.width * 70 / 100).clamp(20.min(over.width), over.width);
    let h = (marks.len() as u16 + 2).clamp(3.min(over.height), over.height);
    let x = over.x + (over.width.saturating_sub(w)) / 2;
    let y = over.y + (over.height.saturating_sub(h)) / 2;
    let popup = Rect::new(x, y, w, h);
    crate::tui::open_popup(frame, popup, theme);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(crate::tui::popup_border_style(theme, popup))
        .title(Span::styled(
            " Bookmarks ",
            theme.style(StyleRole::PopupTitle),
        ));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    crate::tui::paint_popup_border(frame, popup, theme);

    if marks.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " None for this host yet (press b in the viewer).",
                theme.style(StyleRole::PopupLegend),
            )),
            inner,
        );
        return;
    }
    let legend = theme.style(StyleRole::PopupLegend);
    let items: Vec<ListItem> = marks
        .iter()
        .map(|(name, line)| {
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {name}"), theme.style(StyleRole::PickerRow)),
                Span::styled(format!("  (line {line})"), legend),
            ]))
        })
        .collect();
    let mut st = ListState::default();
    st.select(Some(s.bookmark_sel.min(marks.len() - 1)));
    frame.render_stateful_widget(
        List::new(items).highlight_style(theme.style(StyleRole::PickerRowSelected)),
        inner,
        &mut st,
    );
}

/// Compact human-readable byte size (e.g. `12.3 KB`).
fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut val = bytes as f64;
    let mut unit = 0;
    while val >= 1024.0 && unit < UNITS.len() - 1 {
        val /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{val:.1} {}", UNITS[unit])
    }
}
