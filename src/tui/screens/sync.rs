//! Sync tab body and the Sync modals. Everything here draws
//! [`crate::app::sync::SyncView`]; a modal whose state is absent draws nothing.

use ratatui::layout::{Margin, Rect};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::app::sync::{DeviceStatusView, TRUST_TTL_DAYS};
use crate::app::App;
use crate::config::KeyAction;
use crate::theme::catalog::{ColorRole, StyleRole};

fn key_of(app: &App, action: KeyAction) -> String {
    super::map::key_label(app.config.keybinds.primary(action))
}

fn color(app: &App, role: ColorRole) -> Style {
    Style::default().fg(app.theme().color(role))
}

fn ago(last_seen: Option<i64>) -> String {
    let Some(at) = last_seen else {
        return "never".into();
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    match now - at {
        s if s < 60 => "just now".into(),
        s if s < 3600 => format!("{}m ago", s / 60),
        s if s < 86_400 => format!("{}h ago", s / 3600),
        s => format!("{}d ago", s / 86_400),
    }
}

/// Sync tab body: the ring control panel, or how to start / join a ring.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    if area.height < 4 || area.width < 20 {
        return;
    }
    let theme = app.theme();
    let text = theme.style(StyleRole::TextPrimary);
    let muted = theme.style(StyleRole::TextMuted);
    let sync = &app.sync;
    let inner = area.inner(Margin::new(if area.width >= 80 { 2 } else { 0 }, 0));

    let mut lines: Vec<Line> = vec![Line::styled("Sync Net", theme.style(StyleRole::TextBright))];
    if !sync.configured {
        let enroll = key_of(app, KeyAction::SyncEnroll);
        lines.extend([
            Line::raw(""),
            Line::styled("sync is not set up on this profile", text),
            Line::raw(""),
            Line::styled(
                "Sync keeps hosts and credentials the same on your own devices, \
                 encrypted end to end, with no server in the middle.",
                muted,
            ),
            Line::raw(""),
            Line::styled(
                format!(
                    "Start a ring: on the hosts tab select a machine that runs sshub \
                     and press {enroll} to make it one of your devices."
                ),
                text,
            ),
            Line::styled(
                "Join a ring: on a blank machine run `sshub sync-join <a reachable device>`.",
                text,
            ),
        ]);
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        return;
    }

    let mode = sync.cred_mode.map_or("-", |m| m.label());
    let trust = sync
        .trust
        .as_ref()
        .map_or("no trust session".to_string(), |t| {
            format!(
                "trust: {}, {}d, {} hosts opened",
                t.scope, t.term_days, t.hosts_opened
            )
        });
    lines.push(Line::styled(
        format!("credentials: {mode}   pending: {}   {trust}", sync.pending),
        muted,
    ));
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        format!(
            "  {:<18} {:<8} {:<13} {:<10} {}",
            "device", "role", "status", "contact", "head"
        ),
        muted,
    ));
    let selected = sync.selected.min(sync.devices.len().saturating_sub(1));
    for (i, d) in sync.devices.iter().enumerate() {
        let (glyph, role) = match d.status {
            DeviceStatusView::Operational => ("\u{25cf}", ColorRole::StatusSuccess),
            DeviceStatusView::Behind => ("\u{25d0}", ColorRole::StatusWarning),
            DeviceStatusView::Stale => ("\u{25cb}", ColorRole::StatusWarning),
            DeviceStatusView::Suspect => ("\u{25b2}", ColorRole::StatusWarning),
            DeviceStatusView::Compromised => ("\u{2716}", ColorRole::StatusError),
            DeviceStatusView::Revoked => ("\u{25cb}", ColorRole::StatusUnknown),
        };
        let role_label = match d.role {
            crate::app::sync::DeviceRoleView::Holder => "holder",
            crate::app::sync::DeviceRoleView::Mirror => "mirror",
        };
        let mut row = format!(
            " {:<18} {:<8} {:<13} {:<10} #{}",
            d.name,
            role_label,
            d.status.label(),
            ago(d.last_seen),
            d.head_seq
        );
        if d.unattended {
            row.push_str("  unattended");
        }
        if d.exempt {
            row.push_str("  exemption: master phrase");
        }
        let style = if i == selected {
            theme.style(StyleRole::PickerRowSelected)
        } else if d.status == DeviceStatusView::Compromised {
            color(app, ColorRole::StatusError)
        } else {
            text
        };
        // The glyph keeps its status colour even on the selected row.
        lines.push(Line::from(vec![
            Span::styled(glyph, color(app, role)),
            Span::styled(row, style),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// A centered popup with wrapped `body` and a one-line `hint` on its last row.
fn modal(frame: &mut Frame, app: &App, title: &str, width: u16, body: Vec<Line>, hint: &str) {
    let area = frame.area();
    let w = width.min(area.width.saturating_sub(2));
    // Rows after wrapping, so a long paragraph is not cut off by its popup.
    let text_w = usize::from(w.saturating_sub(4)).max(1);
    let rows: usize = 1 + body
        .iter()
        .map(|l| l.width().div_ceil(text_w).max(1))
        .sum::<usize>();
    let h = (rows as u16)
        .saturating_add(4)
        .min(area.height.saturating_sub(2));
    if w < 8 || h < 4 {
        return;
    }
    let popup = Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    );
    let popup = crate::tui::popup_open_rect(popup, app);
    let theme = app.theme();
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
                format!(" {title} "),
                theme.style(StyleRole::PopupTitle),
            ))
            .border_style(crate::tui::popup_border_style(theme, popup)),
        popup,
    );
    let inner = popup.inner(Margin::new(2, 1));
    let body_area = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(1),
    );
    frame.render_widget(Paragraph::new(body).wrap(Wrap { trim: false }), body_area);
    frame.buffer_mut().set_string(
        inner.x,
        inner.bottom().saturating_sub(1),
        crate::tui::text::ellipsize(hint, inner.width as usize),
        theme.style(StyleRole::PopupHint),
    );
    crate::tui::paint_popup_border(frame, popup, theme);
}

fn row(selected: bool, text: String, app: &App) -> Line<'static> {
    let theme = app.theme();
    if selected {
        Line::styled(
            format!("\u{203a} {text}"),
            theme.style(StyleRole::PickerRowSelected),
        )
    } else {
        Line::styled(format!("  {text}"), theme.style(StyleRole::PickerRow))
    }
}

pub fn render_review(frame: &mut Frame, app: &App) {
    let sync = &app.sync;
    if sync.review.is_empty() {
        return;
    }
    let selected = sync.review_selected.min(sync.review.len() - 1);
    let mut body: Vec<Line> = sync
        .review
        .iter()
        .enumerate()
        .map(|(i, op)| row(i == selected, format!("{}: {}", op.device, op.summary), app))
        .collect();
    body.push(Line::raw(""));
    for c in &sync.review[selected].changes {
        let style = if c.dangerous {
            color(app, ColorRole::StatusError)
        } else {
            app.theme().style(StyleRole::TextPrimary)
        };
        let mark = if c.dangerous { "! " } else { "  " };
        body.push(Line::styled(
            format!("{mark}{}: {} -> {}", c.field, c.from, c.to),
            style,
        ));
    }
    modal(
        frame,
        app,
        "Review changes",
        90,
        body,
        "\u{2191}\u{2193} select  a approve  r reject  Esc close",
    );
}

pub fn render_unlock(frame: &mut Frame, app: &App) {
    let Some(u) = app.sync.unlock.as_ref() else {
        return;
    };
    let theme = app.theme();
    let mut body = vec![
        Line::styled(
            format!("Unlocking {}", u.host),
            theme.style(StyleRole::TextBright),
        ),
        Line::styled(
            format!("{} of {} shares", u.have, u.need),
            theme.style(StyleRole::TextPrimary),
        ),
    ];
    for (path, status) in &u.paths {
        body.push(Line::styled(
            format!("  {path}: {status}"),
            theme.style(StyleRole::TextMuted),
        ));
    }
    body.push(Line::raw(""));
    body.push(Line::styled(
        format!("code \u{203a} {}\u{2588}", u.code),
        theme.style(StyleRole::PickerQuery),
    ));
    modal(frame, app, "Unlock", 70, body, "Enter submit  Esc cancel");
}

pub fn render_sas(frame: &mut Frame, app: &App) {
    let Some(s) = app.sync.sas.as_ref() else {
        return;
    };
    let theme = app.theme();
    let body = vec![
        Line::styled(
            format!("Enrolling with {}", s.peer),
            theme.style(StyleRole::TextPrimary),
        ),
        Line::raw(""),
        Line::styled(s.words.clone(), theme.style(StyleRole::TextBright)),
        Line::raw(""),
        Line::styled(
            "Both screens must show exactly these words. If they differ, someone is in the middle: answer no.",
            theme.style(StyleRole::TextMuted),
        ),
    ];
    modal(
        frame,
        app,
        "Verify device",
        70,
        body,
        "y words match  n / Esc they differ",
    );
}

/// Full-screen alert; takes the whole frame.
pub fn render_fork_alert(frame: &mut Frame, app: &App) {
    let Some(f) = app.sync.fork.as_ref() else {
        return;
    };
    let area = frame.area();
    let theme = app.theme();
    let red = color(app, ColorRole::StatusError);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(red)
            .title(Span::styled(" COMPROMISED DEVICE ", red)),
        area,
    );
    let text = theme.style(StyleRole::TextPrimary);
    let body = vec![
        Line::styled(format!("{} forked its history", f.device), red),
        Line::styled(f.detail.clone(), theme.style(StyleRole::TextMuted)),
        Line::raw(""),
        Line::styled(
            "It signed two different histories under one identity. Treat everything it has sent as untrusted.",
            text,
        ),
        Line::raw(""),
        Line::styled("What to do now:", text),
        Line::styled(
            "  1. Do not approve changes from this device or release shares to it.",
            text,
        ),
        Line::styled(
            "  2. Revoke it from a trusted device (Sync tab, device details).",
            text,
        ),
        Line::styled("  3. Change the credentials it could read.", text),
        Line::raw(""),
        Line::styled(
            "Esc closes this alert; the sync badge stays red.",
            theme.style(StyleRole::PopupHint),
        ),
    ];
    frame.render_widget(
        Paragraph::new(body).wrap(Wrap { trim: false }),
        area.inner(Margin::new(3, 2)),
    );
}

pub fn render_trust_scope(frame: &mut Frame, app: &App) {
    let sync = &app.sync;
    if sync.trust_scopes.is_empty() {
        return;
    }
    let selected = sync.trust_scope_selected.min(sync.trust_scopes.len() - 1);
    let mut body = vec![
        Line::styled(
            "Which hosts may this device open without asking? There is no \"everything\".",
            app.theme().style(StyleRole::TextMuted),
        ),
        Line::raw(""),
    ];
    body.extend(
        sync.trust_scopes
            .iter()
            .enumerate()
            .map(|(i, s)| row(i == selected, s.clone(), app)),
    );
    modal(
        frame,
        app,
        "Trust session: scope",
        72,
        body,
        "\u{2191}\u{2193} select  Enter next  Esc cancel",
    );
}

pub fn render_trust_ttl(frame: &mut Frame, app: &App) {
    let sync = &app.sync;
    let selected = sync.trust_ttl_selected.min(TRUST_TTL_DAYS.len() - 1);
    let scope = sync
        .trust_scopes
        .get(sync.trust_scope_selected)
        .map_or("-", String::as_str);
    let mut body = vec![
        Line::styled(
            format!("Scope: {scope}"),
            app.theme().style(StyleRole::TextPrimary),
        ),
        Line::raw(""),
    ];
    body.extend(TRUST_TTL_DAYS.iter().enumerate().map(|(i, days)| {
        let mark = if i == selected { "(\u{2022})" } else { "( )" };
        row(i == selected, format!("{mark} {days} days"), app)
    }));
    modal(
        frame,
        app,
        "Trust session: term",
        60,
        body,
        "\u{2191}\u{2193} select  Enter open  Esc back",
    );
}

pub fn render_release_confirm(frame: &mut Frame, app: &App) {
    let Some(r) = app.sync.release.as_ref() else {
        return;
    };
    let theme = app.theme();
    let body = vec![
        Line::styled(
            format!("{} ({}) asks for a share", r.requester, r.requester_id),
            theme.style(StyleRole::TextBright),
        ),
        Line::styled(
            format!("to open {}", r.host),
            theme.style(StyleRole::TextPrimary),
        ),
        Line::raw(""),
        Line::styled(
            "Release only if you recognise that device and expected this request.",
            theme.style(StyleRole::TextMuted),
        ),
    ];
    modal(
        frame,
        app,
        "Release share",
        72,
        body,
        "y release  n / Esc refuse",
    );
}

pub fn render_onboarding(frame: &mut Frame, app: &App) {
    let Some(o) = app.sync.onboarding.as_ref() else {
        return;
    };
    let theme = app.theme();
    let text = theme.style(StyleRole::TextPrimary);
    let body = vec![
        Line::styled("Mode: Vault", theme.style(StyleRole::TextBright)),
        Line::raw(""),
        Line::styled(o.words.join(" "), theme.style(StyleRole::TextBright)),
        Line::raw(""),
        Line::styled(
            "These six words are the credential store. There is no account to recover it from: lose them and the credentials are gone.",
            text,
        ),
        Line::styled(
            "The printed kit is a separate sheet. Do not photograph it: a photo in a phone gallery defeats what the sheet is for.",
            text,
        ),
        Line::styled(
            "Vault means a stolen machine reduces the protection to guessing the phrase. Moving to Quorum, where one device is not enough, needs a second holder.",
            theme.style(StyleRole::TextMuted),
        ),
    ];
    modal(
        frame,
        app,
        "Set up sync",
        76,
        body,
        "Enter: I have written the words down",
    );
}

#[cfg(test)]
mod tests {
    use crate::app::sync::*;
    use crate::app::tests::test_app;
    use crate::app::{App, AppMode};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    pub(crate) fn screen(app: &App, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| crate::tui::render(f, app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf.cell((x, y)).unwrap().symbol());
            }
            out.push('\n');
        }
        out
    }

    fn app_in(mode: AppMode) -> App {
        let mut app = test_app(vec![]);
        app.sync = sample_sync_view_with_modals();
        app.mode = mode;
        app
    }

    #[test]
    fn unconfigured_tab_explains_how_to_start_or_join_a_ring() {
        let mut app = test_app(vec![]);
        app.active_tab = 6;
        let s = screen(&app, 110, 30);
        assert!(s.contains("sync is not set up on this profile"), "{s}");
        assert!(s.contains("Start a ring"), "{s}");
        assert!(s.contains("sshub sync-join"), "{s}");
        assert!(s.contains("7 sync") || s.contains("[7] sync"), "{s}");
        assert!(!s.contains('\u{21c5}'), "no badge without a ring: {s}");
    }

    #[test]
    fn configured_tab_lists_devices_and_shows_the_badge() {
        let mut app = test_app(vec![]);
        app.sync = sample_sync_view();
        app.active_tab = 6;
        let s = screen(&app, 120, 30);
        for want in [
            "laptop",
            "server",
            "operational",
            "stale",
            "credentials: Vault",
            "pending: 2",
            "group staging",
            "\u{21c5}1",
            "2ops",
        ] {
            assert!(s.contains(want), "{want}: {s}");
        }
    }

    #[test]
    fn every_sync_modal_renders_its_content() {
        for (mode, wants) in [
            (
                AppMode::SyncReview,
                &["Review changes", "proxy_command", "nc evil 22"][..],
            ),
            (
                AppMode::SyncUnlock,
                &["1 of 2 shares", "audio: listening", "ab-12"][..],
            ),
            (
                AppMode::SyncSas,
                &["harbor-globe-lantern-mint", "Verify device"][..],
            ),
            (
                AppMode::TrustSessionScope,
                &["Trust session: scope", "group prod"][..],
            ),
            (
                AppMode::TrustSessionTtl,
                &["Trust session: term", "30 days", "90 days"][..],
            ),
            (
                AppMode::SyncReleaseConfirm,
                &["new laptop", "web-1", "id-9"][..],
            ),
            (
                AppMode::SyncOnboarding,
                &["Vault", "six words", "Do not photograph"][..],
            ),
        ] {
            let app = app_in(mode);
            let s = screen(&app, 110, 36);
            for want in wants {
                assert!(s.contains(want), "{mode:?} {want}: {s}");
            }
        }
    }

    #[test]
    fn fork_alert_is_full_screen_and_names_the_device() {
        let app = app_in(AppMode::SyncForkAlert);
        let s = screen(&app, 100, 30);
        assert!(s.contains("COMPROMISED DEVICE"), "{s}");
        assert!(s.contains("server forked its history"), "{s}");
        assert!(s.contains("Revoke it"), "{s}");
        // No dashboard chrome: the tab bar is gone.
        assert!(!s.contains("hosts"), "{s}");
    }

    #[test]
    fn dangerous_review_fields_are_drawn_in_the_error_colour() {
        let app = app_in(AppMode::SyncReview);
        let mut terminal = Terminal::new(TestBackend::new(110, 36)).unwrap();
        terminal.draw(|f| crate::tui::render(f, &app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let red = app
            .theme()
            .color(crate::theme::catalog::ColorRole::StatusError);
        let row_has = |y: u16, want: &str, fg| {
            let text: String = (0..buf.area.width)
                .map(|x| buf.cell((x, y)).unwrap().symbol())
                .collect();
            text.contains(want)
                && (0..buf.area.width).any(|x| {
                    buf.cell((x, y)).unwrap().symbol() != " " && buf.cell((x, y)).unwrap().fg == fg
                })
        };
        let danger = (0..buf.area.height).find(|&y| row_has(y, "proxy_command", red));
        assert!(danger.is_some(), "the dangerous field row is red");
    }
}
