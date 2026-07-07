//! Fullscreen session view: 1-row header, body, 1-row footer.
//!
//! The body is the live PTY grid via `tui_term` — including while connecting,
//! so the real ssh handshake (`ssh -v`) is shown verbatim with nothing
//! fabricated. Header / footer match the design tokens in `src/tui/theme.rs`.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use tui_term::widget::PseudoTerminal;

use crate::app::App;
use crate::session::{Session, SessionMeta, SessionPhase};
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
            SessionPhase::Exited { .. } => ("● exited", theme::red()),
            // Only claim "connected" once ssh has genuinely reached the remote.
            // The connect screen may be shown live before that (or revealed by
            // the timeout fail-open), in which case we're still "connecting".
            _ if session.is_connected() => ("● connected", theme::green()),
            _ => ("● connecting", theme::amber()),
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

    // Right-side hints for tab management.
    let hint_text: String = if app.sessions.len() > 1 {
        "Ctrl+T new  Ctrl+W close  Ctrl+[/] tabs  Ctrl+D detach".into()
    } else {
        "Ctrl+T new tab  Ctrl+D detach".into()
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
    // While connecting, ssh's verbose `-v` handshake is siphoned off the PTY
    // (via a side FIFO) into `debug_log`, so the grid stays clean. Show a
    // spinner + a dim tail of that log instead of the raw firehose; the user
    // can expand the full log on demand. Once the shell reveals we switch to
    // the live PTY grid, which now carries only the post-auth banner + prompt.
    if let SessionPhase::Connecting { started_at } = &session.phase {
        render_connecting(frame, area, session, started_at.elapsed());
        return;
    }
    // Exited before ever reaching a shell (e.g. unreachable host, auth refused):
    // the PTY grid is blank, so show a failure marker + plain-language reason
    // (derived from the `-v` log) with the debug tail underneath.
    if matches!(session.phase, SessionPhase::Exited { .. }) && !session.is_connected() {
        render_failure(frame, area, session);
        return;
    }
    let term = PseudoTerminal::new(session.parser.screen());
    frame.render_widget(term, area);
}

/// Shared connect-screen layout: `center` lines centered in the upper band,
/// the dim `-v` debug tail filling a bottom band. Used by both the connecting
/// spinner and the failure screen so they line up visually.
fn render_centered_and_tail(
    frame: &mut Frame,
    area: Rect,
    session: &Session,
    center: Vec<Line<'static>>,
) {
    let dim = Style::default().fg(theme::DIM);
    let tail_h = area.height.saturating_sub(1).min(8);
    let top_h = area.height - tail_h;
    let top_area = Rect::new(area.x, area.y, area.width, top_h);
    let tail_area = Rect::new(area.x, area.y + top_h, area.width, tail_h);

    if top_h >= 1 {
        let pad_top = top_h.saturating_sub(center.len() as u16) / 2;
        let centered = Rect::new(top_area.x, top_area.y + pad_top, top_area.width, top_h - pad_top);
        frame.render_widget(
            Paragraph::new(center).alignment(ratatui::layout::Alignment::Center),
            centered,
        );
    }

    if tail_h >= 1 {
        let all: Vec<&str> = session.debug_log().lines().collect();
        let start = all.len().saturating_sub(tail_h as usize);
        let lines: Vec<Line> = all[start..]
            .iter()
            .map(|l| Line::from(Span::styled(truncate(l, area.width as usize), dim)))
            .collect();
        frame.render_widget(Paragraph::new(lines), tail_area);
    }
}

/// Failure screen: a red ✗, the plain-language reason, and a dismiss hint.
fn render_failure(frame: &mut Frame, area: Rect, session: &Session) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let red = Style::default().fg(theme::RED);
    let mute = Style::default().fg(theme::MUTE);
    let dim = Style::default().fg(theme::DIM);
    let host = session
        .meta
        .address
        .clone()
        .unwrap_or_else(|| session.display_name.clone());

    let center = vec![
        Line::from(Span::styled(
            "\u{2717}",
            red.add_modifier(ratatui::style::Modifier::BOLD),
        )),
        Line::raw(""),
        Line::from(vec![
            Span::styled("couldn't connect to ", mute),
            Span::styled(host, Style::default().fg(theme::TEXT)),
        ]),
        Line::raw(""),
        Line::from(Span::styled(session.failure_reason(), Style::default().fg(theme::TEXT))),
        Line::raw(""),
        Line::from(Span::styled("press any key to close", dim)),
    ];
    render_centered_and_tail(frame, area, session, center);
}

/// Render the whole captured `-v` debug log, bottom-anchored and dimmed.
fn render_full_debug_log(frame: &mut Frame, area: Rect, session: &Session) {
    let dim = Style::default().fg(theme::DIM);
    let lines: Vec<Line> = session
        .debug_log()
        .lines()
        .map(|l| Line::from(Span::styled(l.to_string(), dim)))
        .collect();
    let total = lines.len() as u16;
    let scroll = total.saturating_sub(area.height);
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), area);
}

/// Braille spinner frames, advanced by wall-clock so it animates while idle.
const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn render_connecting(
    frame: &mut Frame,
    area: Rect,
    session: &Session,
    elapsed: std::time::Duration,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let dim = Style::default().fg(theme::DIM);
    let mute = Style::default().fg(theme::MUTE);

    // Expanded: hand the whole body to the raw debug log, bottom-anchored.
    if session.debug_expanded() {
        render_full_debug_log(frame, area, session);
        return;
    }

    let frame_idx = (elapsed.as_millis() / 90) as usize % SPINNER_FRAMES.len();
    let host = session
        .meta
        .address
        .clone()
        .unwrap_or_else(|| session.display_name.clone());
    let secs = elapsed.as_secs();
    let center = vec![
        Line::from(vec![
            Span::styled(SPINNER_FRAMES[frame_idx], Style::default().fg(theme::GREEN)),
            Span::raw("  "),
            Span::styled("connecting to ", mute),
            Span::styled(host, Style::default().fg(theme::TEXT)),
        ]),
        Line::raw(""),
        Line::from(Span::styled(
            format!("elapsed {secs}s  ·  Ctrl+O expand log  ·  Esc cancel"),
            dim,
        )),
    ];
    render_centered_and_tail(frame, area, session, center);
}

/// Clip a line to `max` display columns (byte-safe for ASCII debug output).
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
    }
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

    // Real host:port from the session meta — no fabricated cipher/keepalive.
    let target = {
        let host = session
            .meta
            .address
            .clone()
            .unwrap_or_else(|| session.display_name.clone());
        match session.meta.port {
            Some(p) => format!("{host}:{p}"),
            None => host,
        }
    };
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(elapsed_str, mute),
        bullet.clone(),
        Span::styled(target, mute),
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
