//! Fullscreen session view: 1-row header, body, 1-row footer.
//!
//! The body is the live PTY grid via `tui_term` — including while connecting,
//! so the real ssh handshake (`ssh -v`) is shown verbatim with nothing
//! fabricated. Header / footer take their styles from the runtime theme's
//! `components.header.*` and `components.session.*` roles; the grid itself is
//! the host's own output and is never recoloured.

use std::time::Duration;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use tui_term::widget::PseudoTerminal;

use crate::app::App;
use crate::config::{KeyAction, KeybindsConfig};
use crate::session::{Session, SessionMeta, SessionPhase};
use crate::theme::catalog::{ColorRole, PaintRole, StyleRole};
use crate::theme::model::ResolvedTheme;

/// The PTY body's rect: everything between the one-row header and the one-row
/// footer. Exposed so the tab-switch slide can move the body alone and leave
/// the header fixed (#35) -- sliding the whole screen, chrome included, made
/// switching tabs unpleasant to look at.
pub fn body_rect(area: Rect) -> Rect {
    Rect::new(
        area.x,
        area.y.saturating_add(1),
        area.width,
        area.height.saturating_sub(2),
    )
}

/// The session view's three bands: 1-row header, body, 1-row footer.
fn session_chunks(frame_area: Rect) -> std::rc::Rc<[Rect]> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(frame_area)
}

/// The rectangle the remote PTY grid occupies in a full-screen session frame.
///
/// The single source of the protected geometry: this renderer draws the
/// `tui_term` widget into it, and `crate::tui`'s app-background pass excludes
/// exactly it. Two independently derived rects is how a one-cell drift ships,
/// and one drifted cell means a theme colour written over the host's own
/// output — which is why this goes through the real layout rather than through
/// [`body_rect`]'s arithmetic, which parts from it on a terminal too short for
/// all three bands.
pub(crate) fn remote_pty_rect(frame_area: Rect) -> Rect {
    session_chunks(frame_area)[1]
}

/// Whether this frame actually shows the remote grid.
///
/// The connecting spinner and the failure screen occupy the same rect but are
/// SSHub's own chrome, so a theme may paint them. Shared with the exclusion
/// pass for the same reason as [`remote_pty_rect`]: the *decision* must not be
/// duplicated either.
pub(crate) fn shows_remote_pty(session: &Session) -> bool {
    match &session.phase {
        SessionPhase::Connecting { .. } => false,
        SessionPhase::Exited { .. } => session.is_connected(),
        SessionPhase::Running { .. } => true,
    }
}

pub fn render(frame: &mut Frame, app: &App) {
    let Some(session) = app.active_session() else {
        return;
    };
    let theme = app.theme();

    let chunks = session_chunks(frame.area());

    render_header(frame, chunks[0], session, app, theme);
    render_body(frame, chunks[1], session, &app.config.keybinds, theme);
    render_footer(frame, chunks[2], session, theme);

    // Transient "copied" toast in the body's bottom-right corner.
    if let Some((msg, at)) = &session.copy_notice {
        if at.elapsed() < Duration::from_millis(3500) {
            render_copy_toast(frame, chunks[1], msg, theme);
        }
    }
}

/// Small self-dismissing toast (e.g. "✓ copied N chars") in the corner of the
/// session body. The event loop keeps redrawing, so the time check hides it.
fn render_copy_toast(frame: &mut Frame, body: Rect, msg: &str, theme: &ResolvedTheme) {
    let text = format!(" \u{2713} {msg} ");
    let w = (text.chars().count() as u16 + 2).min(body.width);
    let h = 3u16.min(body.height);
    if w == 0 || h == 0 {
        return;
    }
    let x = body.x + body.width.saturating_sub(w + 1);
    let y = body.y + body.height.saturating_sub(h);
    let rect = Rect::new(x, y, w, h);
    frame.render_widget(Clear, rect);
    // The toast sits *inside* the PTY viewport, so its frame takes the solid
    // colour of `session.border` and no gradient post-pass: the ring painter
    // takes no exclusions, and a rect overlapping the viewport is exactly what
    // it must never be handed.
    let border = theme.paint_color_at(PaintRole::SessionBorder, rect, rect.x, rect.y);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            text,
            Style::default().fg(theme.color(ColorRole::StatusSuccess)),
        )))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border)),
        ),
        rect,
    );
}

// ── Header ───────────────────────────────────────────────────

fn render_header(
    frame: &mut Frame,
    area: Rect,
    session: &Session,
    app: &App,
    theme: &ResolvedTheme,
) {
    let active = theme.style(StyleRole::HeaderSessionActive);
    let inactive = theme.style(StyleRole::HeaderSessionInactive);
    let muted = theme.style(StyleRole::TextMuted);
    let mut spans: Vec<Span<'static>> = vec![
        Span::raw(" "),
        Span::styled(" SSHub ", theme.style(StyleRole::HeaderBrand)),
        Span::raw("  "),
    ];

    // Column each tab's label starts at, filled while the strip is built, so
    // the highlight can travel between them (#35).
    let mut tab_spans: Vec<(u16, u16)> = Vec::new();
    let travel = highlight_travel(app);
    if app.sessions.len() > 1 {
        // Multi-tab header: compact tab strip in place of the verbose
        // connection summary. Active tab is reversed; others are mute.
        let active_idx = app.active_session.unwrap_or(0);
        // The highlight is painted afterwards, over the interpolated rect, so
        // every label goes down mute here and the moving bar decides which one
        // reads as active.
        let mut col: u16 = area.x + spans.iter().map(span_cols).sum::<u16>();
        for (i, s) in app.sessions.iter().enumerate() {
            let label = format!(" {} {} ", i + 1, s.display_name);
            let w = label.chars().count() as u16;
            tab_spans.push((col, w));
            col += w + 1;
            let style = if i == active_idx && travel.is_none() {
                active
            } else {
                inactive
            };
            spans.push(Span::styled(label, style));
            spans.push(Span::raw(" "));
        }
    } else {
        // Single-tab header — full connection detail.
        let color = |role| Style::default().fg(theme.color(role));
        let (status_label, status_style) = match &session.phase {
            SessionPhase::Exited { .. } => ("● exited", color(ColorRole::SessionExited)),
            // Only claim "connected" once ssh has genuinely reached the remote.
            // The connect screen may be shown live before that (or revealed by
            // the timeout fail-open), in which case we're still "connecting".
            _ if session.is_connected() => ("● connected", color(ColorRole::StatusSuccess)),
            _ => ("● connecting", color(ColorRole::SessionConnecting)),
        };
        spans.push(Span::styled(status_label, status_style));
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            connection_label(&session.meta, &session.display_name),
            muted,
        ));

        if let Some((prefix, value, is_jump)) = via_label(&session.meta) {
            spans.push(Span::raw("   "));
            spans.push(Span::styled(prefix, muted));
            spans.push(Span::styled(
                value,
                if is_jump {
                    color(ColorRole::StatusWarning)
                } else {
                    muted
                },
            ));
        }
        if let Some(t) = tunnel_summary(app, &session.meta) {
            spans.push(Span::raw("   "));
            spans.push(Span::styled("tunnels: ", muted));
            // No `Style` role resolves to the highlight slot on its own, and
            // this value has always been the brightest text in the row.
            spans.push(Span::styled(
                t,
                Style::default().fg(theme.semantic().text_highlight),
            ));
        }
    }

    // Right-side hints for tab management (from user keybind config).
    let hint_text = app
        .config
        .keybinds
        .session_header_hints(app.sessions.len() > 1);
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let pad = (area.width as usize).saturating_sub(used + hint_text.chars().count() + 1);
    spans.push(Span::raw(" ".repeat(pad)));
    spans.push(Span::styled(hint_text, muted));

    frame.render_widget(Paragraph::new(Line::from(spans)), area);

    // Slide the active-tab highlight from the tab being left to the new one, so
    // the strip stays put and only the bar under it moves (#35).
    if let Some(p) = travel {
        let from = app
            .session_tab_switch
            .map(|sw| sw.from.min(tab_spans.len().saturating_sub(1)))
            .unwrap_or(0);
        let to = app.active_session.unwrap_or(0);
        if let (Some(&(fx, fw)), Some(&(tx, tw))) = (tab_spans.get(from), tab_spans.get(to)) {
            let e = crate::tui::tween::ease_in_out(p);
            let x = lerp_u16(fx, tx, e);
            let w = lerp_u16(fw, tw, e);
            let buf = frame.buffer_mut();
            for cx in x..x.saturating_add(w).min(area.x + area.width) {
                if let Some(cell) = buf.cell_mut((cx, area.y)) {
                    cell.set_style(active);
                }
            }
        }
    }
}

/// Progress of the tab-highlight travel, or `None` when it is at rest (which
/// is also when the active label paints itself highlighted).
///
/// Shared with the dashboard session strip, which mirrors the same travel, so
/// both surfaces agree on when the highlight is moving.
pub(crate) fn highlight_travel(app: &App) -> Option<f32> {
    if !app.motion_enabled() {
        return None;
    }
    let sw = app.session_tab_switch?;
    let p = crate::tui::tween::progress(sw.at, crate::tui::TAB_ANIM, std::time::Instant::now());
    (p < 1.0).then_some(p)
}

/// Display columns a span occupies.
fn span_cols(span: &Span<'static>) -> u16 {
    span.content.chars().count() as u16
}

pub(crate) fn lerp_u16(a: u16, b: u16, t: f32) -> u16 {
    (a as f32 + (b as f32 - a as f32) * t).round().max(0.0) as u16
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

fn render_body(
    frame: &mut Frame,
    area: Rect,
    session: &Session,
    keybinds: &KeybindsConfig,
    theme: &ResolvedTheme,
) {
    // While connecting, ssh's verbose `-v` handshake is siphoned off the PTY
    // (via a side FIFO) into `debug_log`, so the grid stays clean. Show a
    // spinner + a dim tail of that log instead of the raw firehose; the user
    // can expand the full log on demand. Once the shell reveals we switch to
    // the live PTY grid, which now carries only the post-auth banner + prompt.
    if !shows_remote_pty(session) {
        if let SessionPhase::Connecting { started_at } = &session.phase {
            render_connecting(frame, area, session, keybinds, started_at.elapsed(), theme);
        } else {
            // Exited before ever reaching a shell (e.g. unreachable host, auth
            // refused): the PTY grid is blank, so show a failure marker + a
            // plain-language reason with the debug tail underneath.
            render_failure(frame, area, session, theme);
        }
        return;
    }
    let term = PseudoTerminal::new(session.parser.screen());
    frame.render_widget(term, area);

    // Overlay the in-app text selection by reversing the selected cells. Done
    // as a post-pass over the buffer so it survives every repaint (unlike the
    // outer terminal's native selection, which a redraw wipes).
    if let Some(sel) = session.selection {
        let (rows, cols) = session.parser.screen().size();
        let buf = frame.buffer_mut();
        for r in 0..rows.min(area.height) {
            for c in 0..cols.min(area.width) {
                if sel.contains(r, c) {
                    let x = area.x + c;
                    let y = area.y + r;
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.modifier.toggle(Modifier::REVERSED);
                    }
                }
            }
        }
    }
}

/// Shared connect-screen layout: `center` lines centered in the upper band,
/// the dim `-v` debug tail filling a bottom band. Used by both the connecting
/// spinner and the failure screen so they line up visually.
fn render_centered_and_tail(
    frame: &mut Frame,
    area: Rect,
    session: &Session,
    center: Vec<Line<'static>>,
    theme: &ResolvedTheme,
) {
    let dim = theme.style(StyleRole::SessionDebugTail);
    let tail_h = area.height.saturating_sub(1).min(8);
    let top_h = area.height - tail_h;
    let top_area = Rect::new(area.x, area.y, area.width, top_h);
    let tail_area = Rect::new(area.x, area.y + top_h, area.width, tail_h);

    if top_h >= 1 {
        let pad_top = top_h.saturating_sub(center.len() as u16) / 2;
        let centered = Rect::new(
            top_area.x,
            top_area.y + pad_top,
            top_area.width,
            top_h - pad_top,
        );
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
fn render_failure(frame: &mut Frame, area: Rect, session: &Session, theme: &ResolvedTheme) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let red = Style::default().fg(theme.color(ColorRole::SessionExited));
    let mute = theme.style(StyleRole::TextMuted);
    let dim = theme.style(StyleRole::SessionDebugTail);
    let text = theme.style(StyleRole::TextPrimary);
    let host = session
        .meta
        .address
        .clone()
        .unwrap_or_else(|| session.display_name.clone());

    let mut center = vec![
        Line::from(Span::styled(
            "\u{2717}",
            red.add_modifier(ratatui::style::Modifier::BOLD),
        )),
        Line::raw(""),
        Line::from(vec![
            Span::styled("couldn't connect to ", mute),
            Span::styled(host, text),
        ]),
        Line::raw(""),
        Line::from(Span::styled(session.failure_reason(), text)),
        Line::raw(""),
    ];

    // A changed host key can be accepted (removes the stale known_hosts entry
    // and reconnects). Highlight the choice; a changed key may be a MITM, so
    // make the accept explicit rather than automatic.
    if session.host_key_changed() {
        let warn = Style::default().fg(theme.color(ColorRole::StatusWarning));
        center.push(Line::from(Span::styled(
            "the server's key changed since you last connected",
            warn,
        )));
        center.push(Line::raw(""));
        center.push(Line::from(vec![
            Span::styled("[a]", warn),
            Span::styled(" accept new key & reconnect", mute),
            Span::styled("   ·   any other key to close", dim),
        ]));
    } else {
        center.push(Line::from(Span::styled("press any key to close", dim)));
    }
    render_centered_and_tail(frame, area, session, center, theme);
}

/// Render the whole captured `-v` debug log, bottom-anchored and dimmed.
fn render_full_debug_log(frame: &mut Frame, area: Rect, session: &Session, theme: &ResolvedTheme) {
    let dim = theme.style(StyleRole::SessionDebugTail);
    let lines: Vec<Line> = session
        .debug_log()
        .lines()
        .map(|l| Line::from(Span::styled(l.to_string(), dim)))
        .collect();
    let total = lines.len() as u16;
    let scroll = total.saturating_sub(area.height);
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), area);
}

fn render_connecting(
    frame: &mut Frame,
    area: Rect,
    session: &Session,
    keybinds: &KeybindsConfig,
    elapsed: std::time::Duration,
    theme: &ResolvedTheme,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let dim = theme.style(StyleRole::SessionDebugTail);
    let mute = theme.style(StyleRole::TextMuted);

    // Expanded: hand the whole body to the raw debug log, bottom-anchored.
    if session.debug_expanded() {
        render_full_debug_log(frame, area, session, theme);
        return;
    }

    let host = session
        .meta
        .address
        .clone()
        .unwrap_or_else(|| session.display_name.clone());
    let secs = elapsed.as_secs();
    let toggle = keybinds.primary(KeyAction::SessionToggleLog);
    let cancel = keybinds.primary(KeyAction::SessionCancel);
    let mut hint = format!("elapsed {secs}s");
    if !toggle.is_empty() {
        hint.push_str(&format!("  ·  {toggle} expand log"));
    }
    if !cancel.is_empty() {
        hint.push_str(&format!("  ·  {cancel} cancel"));
    }
    let center = vec![
        Line::from(vec![
            Span::styled(
                crate::tui::tween::spinner_frame(elapsed),
                Style::default().fg(theme.color(ColorRole::StatusSuccess)),
            ),
            Span::raw("  "),
            Span::styled("connecting to ", mute),
            Span::styled(host, theme.style(StyleRole::TextPrimary)),
        ]),
        Line::raw(""),
        Line::from(Span::styled(hint, dim)),
    ];
    render_centered_and_tail(frame, area, session, center, theme);
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

fn render_footer(frame: &mut Frame, area: Rect, session: &Session, theme: &ResolvedTheme) {
    // When the child has exited the footer becomes a red banner with a
    // dismiss hint. Otherwise it shows the usual session stats line.
    if let SessionPhase::Exited { status, .. } = &session.phase {
        let red = Style::default().fg(theme.color(ColorRole::SessionExited));
        let mute = theme.style(StyleRole::TextMuted);
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

    let mute = theme.style(StyleRole::TextMuted);
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
    // surface the selection hint: plain drag selects (and copies) in a shell;
    // inside a mouse app (vim/tmux) hold Shift.
    let scrollback = session.parser.scrollback();
    if scrollback > 0 {
        spans.push(Span::raw("   "));
        spans.push(Span::styled(
            format!("↑ scrolled {scrollback} (PgDn live)"),
            theme.style(StyleRole::SessionScrollback),
        ));
    } else {
        // Pad to the right edge with the hint.
        let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        let hint = "drag: select+copy · Shift+drag in mouse apps";
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SessionConfig;

    #[test]
    fn the_protected_rect_is_the_rect_the_pty_widget_gets() {
        // `render` hands `session_chunks(..)[1]` to the `tui_term` widget, so
        // the exclusion is the viewport by construction. What this pins is that
        // it stays that way — and that the older `body_rect` arithmetic, which
        // parts from the layout on a terminal too short for three bands, is not
        // quietly swapped back in.
        for (w, h) in [(80, 24), (132, 38), (40, 3), (20, 10), (1, 1), (10, 2)] {
            let area = Rect::new(0, 0, w, h);
            assert_eq!(
                remote_pty_rect(area),
                session_chunks(area)[1],
                "the exclusion drifted from the viewport at {w}x{h}"
            );
        }
        // The two-band case the arithmetic gets wrong, spelled out.
        let tiny = Rect::new(0, 0, 10, 1);
        assert_eq!(remote_pty_rect(tiny), Rect::new(0, 0, 10, 1));
        assert!(body_rect(tiny).is_empty());
    }

    fn spawned_session() -> Session {
        let cfg = SessionConfig {
            argv: vec!["true".into()],
            display_name: "web-prod".into(),
            meta: SessionMeta::default(),
            pending_secret: None,
        };
        Session::spawn(cfg, 24, 80, None).unwrap()
    }

    #[test]
    fn only_a_revealed_shell_counts_as_remote_output() {
        let mut session = spawned_session();

        // Connecting: SSHub's own spinner screen occupies the body.
        session.phase = SessionPhase::Connecting {
            started_at: std::time::Instant::now(),
        };
        assert!(!shows_remote_pty(&session));

        // Exited without ever connecting: SSHub's failure screen.
        session.phase = SessionPhase::Exited {
            status: "exit 255".into(),
            at: std::time::Instant::now(),
        };
        assert!(!shows_remote_pty(&session));

        // Running: the live grid, which no theme may recolour.
        session.phase = SessionPhase::Running {
            started_at: std::time::Instant::now(),
        };
        assert!(shows_remote_pty(&session));
    }
}
