//! Scripted "connecting" animation. Mirrors the design's
//! `Connect animation` timeline (handoff README §Connect animation).
//!
//! Lines reveal progressively based on elapsed time. Once real PTY bytes
//! arrive, the parent renderer drops the animation and shows the live shell
//! — so on a fast network the user may only see the first few lines.

use std::time::Duration;

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::session::{Session, SessionMeta};
use crate::tui::theme;

/// One scripted line: appears after `delay_ms` since session spawn.
struct Step {
    delay_ms: u64,
    line: fn(&SessionMeta, &Session) -> Line<'static>,
}

const STEPS: &[Step] = &[
    Step {
        delay_ms: 0,
        line: step_command,
    },
    Step {
        delay_ms: 350,
        line: step_resolving,
    },
    Step {
        delay_ms: 550,
        line: step_address,
    },
    Step {
        delay_ms: 750,
        line: step_connecting,
    },
    Step {
        delay_ms: 1100,
        line: step_channel_open,
    },
    Step {
        delay_ms: 1350,
        line: step_authenticating,
    },
    Step {
        delay_ms: 1700,
        line: step_publickey_accepted,
    },
    Step {
        delay_ms: 1950,
        line: step_negotiating,
    },
    Step {
        delay_ms: 2200,
        line: step_master_established,
    },
];

/// Render every line whose delay has elapsed.
pub fn visible_lines(session: &Session, elapsed: Duration) -> Vec<Line<'static>> {
    let elapsed_ms = elapsed.as_millis() as u64;
    STEPS
        .iter()
        .filter(|s| s.delay_ms <= elapsed_ms)
        .map(|s| (s.line)(&session.meta, session))
        .collect()
}

// ── Line builders ────────────────────────────────────────────

fn step_command(_: &SessionMeta, session: &Session) -> Line<'static> {
    let cmd = if session.display_argv.is_empty() {
        "ssh".to_string()
    } else {
        session.display_argv.join(" ")
    };
    Line::from(vec![
        Span::styled("$ ", theme::mute()),
        Span::styled(cmd, theme::white()),
    ])
}

fn step_resolving(meta: &SessionMeta, session: &Session) -> Line<'static> {
    let host = display_host(meta, session);
    Line::from(vec![
        Span::styled("resolving ", theme::dim()),
        Span::styled(host, theme::white()),
        Span::styled("...", theme::dim()),
    ])
}

fn step_address(meta: &SessionMeta, session: &Session) -> Line<'static> {
    let addr = meta
        .address
        .clone()
        .unwrap_or_else(|| display_host(meta, session));
    Line::from(vec![Span::raw("  "), Span::styled(addr, theme::cyan())])
}

fn step_connecting(meta: &SessionMeta, session: &Session) -> Line<'static> {
    let addr = meta
        .address
        .clone()
        .unwrap_or_else(|| display_host(meta, session));
    let port = meta.port.unwrap_or(22);
    Line::from(vec![
        Span::styled("connecting to ", theme::dim()),
        Span::styled(format!("{addr}:{port}"), theme::dim()),
        Span::styled("...", theme::dim()),
    ])
}

fn step_channel_open(_: &SessionMeta, _: &Session) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled("channel open", theme::green()),
    ])
}

fn step_authenticating(meta: &SessionMeta, _: &Session) -> Line<'static> {
    let with = meta.identity.clone().unwrap_or_else(|| "agent".to_string());
    Line::from(vec![
        Span::styled("authenticating with ", theme::dim()),
        Span::styled(with, Style::default().fg(theme::CYAN)),
    ])
}

fn step_publickey_accepted(_: &SessionMeta, _: &Session) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled("publickey accepted", theme::green()),
    ])
}

fn step_negotiating(_: &SessionMeta, _: &Session) -> Line<'static> {
    Line::from(vec![Span::styled("negotiating session...", theme::dim())])
}

fn step_master_established(_: &SessionMeta, _: &Session) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled("multiplex master established", theme::green()),
    ])
}

fn display_host(meta: &SessionMeta, session: &Session) -> String {
    if let Some(addr) = &meta.address {
        return addr.clone();
    }
    session.display_name.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{SessionConfig, SessionMeta};

    fn make_session() -> Session {
        // Use `true` so the child exits immediately and the reader thread
        // doesn't sit on a real PTY waiting for input.
        let cfg = SessionConfig {
            argv: vec!["true".into()],
            display_name: "host".into(),
            meta: SessionMeta::default(),
            pending_secret: None,
        };
        Session::spawn(cfg, 24, 80).unwrap()
    }

    #[test]
    fn no_lines_before_first_step() {
        let s = make_session();
        // STEPS[0].delay_ms == 0, so even 0ms elapsed shows it. Use a
        // negative-ish elapsed via Duration::ZERO and confirm we get exactly 1.
        let lines = visible_lines(&s, Duration::from_millis(0));
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn three_lines_visible_at_600ms() {
        let s = make_session();
        let lines = visible_lines(&s, Duration::from_millis(600));
        // 0ms, 350ms, 550ms have elapsed; 750ms has not.
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn all_steps_visible_after_3s() {
        let s = make_session();
        let lines = visible_lines(&s, Duration::from_secs(3));
        assert_eq!(lines.len(), STEPS.len());
    }
}
