//! Fullscreen session view: 1-row header, body, 1-row footer.
//!
//! The body is either the scripted ConnectScreen animation (when
//! `phase = Connecting`) or the live PTY grid via `tui_term`. Header / footer
//! match the design tokens in `src/tui/theme.rs`.

use std::time::Instant;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use tui_term::widget::PseudoTerminal;

use crate::app::App;
use crate::session::{connect, Session, SessionMeta, SessionPhase};
use crate::tui::theme;

pub fn render(frame: &mut Frame, app: &App) {
    let Some(session) = app.active_session() else {
        return;
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(frame.area());

    render_header(frame, chunks[0], session, app);
    render_body(frame, chunks[1], session);
    render_footer(frame, chunks[2], session);
}

// ── Header ───────────────────────────────────────────────────

fn render_header(frame: &mut Frame, area: Rect, session: &Session, app: &App) {
    let mut spans: Vec<Span<'static>> = vec![
        Span::raw(" "),
        Span::styled(" SSHub ", theme::inv()),
        Span::raw("  "),
    ];

    if app.sessions.len() > 1 {
        // Multi-tab header: compact tab strip in place of the verbose
        // connection summary. Active tab is reversed; others are mute.
        let active_idx = app.active_session.unwrap_or(0);
        for (i, s) in app.sessions.iter().enumerate() {
            let label = format!(" {} {} ", i + 1, s.display_name);
            if i == active_idx {
                spans.push(Span::styled(label, theme::inv()));
            } else {
                spans.push(Span::styled(label, theme::mute()));
            }
            spans.push(Span::raw(" "));
        }
    } else {
        // Single-tab header — full connection detail.
        let (status_label, status_style) = match &session.phase {
            SessionPhase::Connecting { .. } => ("● connecting", theme::amber()),
            SessionPhase::Running { .. } => ("● connected", theme::green()),
            SessionPhase::Exited { .. } => ("● exited", theme::red()),
        };
        spans.push(Span::styled(status_label, status_style));
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            connection_label(&session.meta, &session.display_name),
            theme::mute(),
        ));

        if let Some((prefix, value, is_jump)) = via_label(&session.meta) {
            spans.push(Span::raw("   "));
            spans.push(Span::styled(prefix, theme::mute()));
            spans.push(Span::styled(
                value,
                if is_jump {
                    theme::amber()
                } else {
                    theme::mute()
                },
            ));
        }
        if let Some(t) = tunnel_summary(app, &session.meta) {
            spans.push(Span::raw("   "));
            spans.push(Span::styled("tunnels: ", theme::mute()));
            spans.push(Span::styled(t, theme::white()));
        }
    }

    // Right-side hints. Always show Ctrl+D exit; Ctrl+T new is most useful
    // alongside the tab strip.
    let hint_text: String = if app.sessions.len() > 1 {
        "Ctrl+T new  Ctrl+W close  Ctrl+PgUp/Dn switch  Ctrl+D exit".into()
    } else {
        "Ctrl+T new tab  Ctrl+D exit".into()
    };
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let pad = (area.width as usize).saturating_sub(used + hint_text.chars().count() + 1);
    spans.push(Span::raw(" ".repeat(pad)));
    spans.push(Span::styled(hint_text, theme::mute()));

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn connection_label(meta: &SessionMeta, display_name: &str) -> String {
    let user = meta.user.clone().unwrap_or_default();
    let host = meta
        .address
        .clone()
        .unwrap_or_else(|| display_name.to_string());
    let port = meta.port.unwrap_or(22);
    if user.is_empty() {
        format!("{host}:{port}")
    } else {
        format!("{user}@{host}:{port}")
    }
}

/// Returns (prefix, value, is_jump) for the "via X" header segment.
fn via_label(meta: &SessionMeta) -> Option<(&'static str, String, bool)> {
    match &meta.proxy_jump {
        Some(jump) if !jump.is_empty() => Some(("via ", jump.clone(), true)),
        _ => Some(("via ", "direct".to_string(), false)),
    }
}

/// Summarise tunnels active for this host, e.g. `L 5432`. Returns None when
/// there are none.
fn tunnel_summary(app: &App, meta: &SessionMeta) -> Option<String> {
    let host_id = meta.host_id?;
    let parts: Vec<String> = app
        .tunnels
        .iter()
        .filter(|t| t.host_id == Some(host_id))
        .map(|t| {
            let dir = match t.tunnel_type {
                crate::store::TunnelType::Local => "L",
                crate::store::TunnelType::Remote => "R",
                crate::store::TunnelType::Dynamic => "D",
            };
            format!("{dir} {}", t.local_port)
        })
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    }
}

// ── Body ─────────────────────────────────────────────────────

fn render_body(frame: &mut Frame, area: Rect, session: &Session) {
    match &session.phase {
        SessionPhase::Connecting { started_at } => {
            render_connect_animation(frame, area, session, *started_at)
        }
        SessionPhase::Running { .. } | SessionPhase::Exited { .. } => {
            let term = PseudoTerminal::new(session.parser.screen());
            frame.render_widget(term, area);
        }
    }
}

fn render_connect_animation(frame: &mut Frame, area: Rect, session: &Session, started_at: Instant) {
    let elapsed = started_at.elapsed();
    let mut lines = connect::visible_lines(session, elapsed);
    if lines.is_empty() {
        return;
    }
    // Indent each line by 2 columns so the body has breathing room (matches
    // overlays.jsx::ConnectScreen).
    for line in lines.iter_mut() {
        line.spans.insert(0, Span::raw("  "));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

// ── Footer ───────────────────────────────────────────────────

fn render_footer(frame: &mut Frame, area: Rect, session: &Session) {
    // When the child has exited the footer becomes a red banner with a
    // dismiss hint. Otherwise it shows the usual session stats line.
    if let SessionPhase::Exited { status, .. } = &session.phase {
        let red = Style::default().fg(theme::RED);
        let mute = Style::default().fg(theme::MUTE);
        let line = Line::from(vec![
            Span::raw(" "),
            Span::styled(format!("● session ended — {status}"), red),
            Span::raw("    "),
            Span::styled("press any key to close", mute),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        return;
    }

    let elapsed_str = match &session.phase {
        SessionPhase::Connecting { started_at } => {
            let secs = started_at.elapsed().as_secs();
            format!("session 0:{:02}", secs.min(59))
        }
        SessionPhase::Running { started_at } => {
            format_session_timer(started_at.elapsed().as_secs())
        }
        SessionPhase::Exited { .. } => unreachable!("handled above"),
    };

    let mute = Style::default().fg(theme::MUTE);
    let bullet = Span::styled(" · ", mute);

    let mut spans = vec![
        Span::raw(" "),
        Span::styled(elapsed_str, mute),
        bullet.clone(),
        Span::styled("keepalive 30s", mute),
        bullet.clone(),
        Span::styled("cipher chacha20-poly1305", mute),
        bullet,
        Span::styled("compression off", mute),
    ];

    // When scrolled back, hint at how to return to live output. Otherwise
    // surface the one-time discovery hint that Shift+drag escapes mouse
    // capture for native selection.
    let scrollback = session.parser.scrollback();
    if scrollback > 0 {
        spans.push(Span::raw("   "));
        spans.push(Span::styled(
            format!("↑ scrolled {scrollback} (PgDn live)"),
            Style::default().fg(theme::AMBER),
        ));
    } else {
        // Pad to the right edge with the hint.
        let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        let hint = "Shift+drag select";
        let pad = (area.width as usize).saturating_sub(used + hint.chars().count() + 1);
        spans.push(Span::raw(" ".repeat(pad)));
        spans.push(Span::styled(hint, mute));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn format_session_timer(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("session {h}:{m:02}:{s:02}")
    } else {
        format!("session {m}:{s:02}")
    }
}
