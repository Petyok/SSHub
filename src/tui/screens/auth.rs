use crate::{
    app::App,
    session::auth_prompt::{HostKeyDetails, PromptClass},
    theme::catalog::StyleRole,
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph, Wrap},
};

pub fn render(frame: &mut Frame, app: &App) {
    let Some(modal) = app.auth_modal.as_ref() else {
        return;
    };
    let area = frame.area();
    let w = crate::tui::fit_popup(84, 20, area.width);
    let h = crate::tui::fit_popup(20, 6, area.height);
    if w == 0 || h == 0 {
        return;
    }
    let popup = crate::tui::popup_open_rect(
        Rect::new(
            area.x + (area.width - w) / 2,
            area.y + (area.height - h) / 2,
            w,
            h,
        ),
        app,
    );
    let theme = app.theme();
    crate::tui::open_popup(frame, popup, theme);
    let title = if modal.changed_key {
        " DANGER: host key changed "
    } else if modal.class == PromptClass::HostKey {
        " Verify host identity "
    } else {
        " SSH authentication "
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(title, theme.style(StyleRole::PopupTitle)));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if inner.is_empty() {
        return;
    }
    let mut lines = Vec::new();
    if let Some(session) = app.sessions.get(modal.session) {
        lines.push(Line::raw(format!("Session: {}", session.display_name)));
    }
    if modal.class == PromptClass::HostKey {
        let details = HostKeyDetails::parse(&modal.prompt);
        if let Some(host) = details.host {
            lines.push(Line::raw(format!("Host: {host}")));
        }
        if let Some(kind) = details.key_type {
            lines.push(Line::raw(format!("Key type: {kind}")));
        }
        if let Some(fp) = details.fingerprint {
            lines.push(Line::raw(format!("Fingerprint: {fp}")));
        }
        if modal.changed_key {
            if let Some(fp) = app
                .sessions
                .get(modal.session)
                .and_then(|s| s.host_key_fingerprint())
            {
                lines.push(Line::raw(format!("New fingerprint: {fp}")));
            }
        }
    } else {
        lines.push(Line::raw(format!("Attempt {} of 3", modal.attempt)));
    }
    lines.extend(modal.prompt.lines().map(|line| Line::raw(line.to_owned())));
    let fixed_rows = if modal.class == PromptClass::HostKey {
        2
    } else {
        4
    };
    let body_h = inner.height.saturating_sub(fixed_rows);
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    // Exact wrapped height: the width estimate undercounts when word wrap
    // leaves a remainder narrower than the next word, hiding the final
    // prompt/fingerprint line on narrow terminals.
    let rows: usize = if inner.width == 0 {
        0
    } else {
        paragraph.line_count(inner.width)
    };
    let max_scroll = rows.saturating_sub(body_h as usize).min(u16::MAX as usize) as u16;
    frame.render_widget(
        paragraph.scroll((modal.scroll.min(max_scroll), 0)),
        Rect::new(inner.x, inner.y, inner.width, body_h),
    );
    let footer = Rect::new(
        inner.x,
        inner.y + body_h,
        inner.width,
        inner.height - body_h,
    );
    let mut controls = Vec::new();
    if modal.class == PromptClass::HostKey {
        controls.push(Line::raw(if modal.changed_key {
            "A: remove old key and reconnect   n/Esc: refuse"
        } else {
            "y: accept and continue   n/Esc: refuse"
        }));
    } else {
        let masked = "*".repeat(modal.value.chars().count());
        let offset = modal
            .cursor
            .saturating_sub(inner.width.saturating_sub(2) as usize);
        controls.push(Line::raw(format!(
            "> {}",
            masked.chars().skip(offset).collect::<String>()
        )));
        controls.push(Line::raw(if modal.can_remember {
            format!(
                "{} [{}] Remember after successful authentication",
                if modal.checkbox_focused { ">" } else { " " },
                if modal.remember { "x" } else { " " }
            )
        } else if modal.class == PromptClass::Generic {
            "One-time response: never saved".into()
        } else {
            "Not saved: no managed host/identity destination".into()
        }));
        controls.push(Line::raw(if modal.can_remember {
            "Enter: submit   Tab/Space: remember   Esc: cancel"
        } else {
            "Enter: submit   Esc: cancel"
        }));
        if !modal.checkbox_focused && footer.height > 0 && footer.width > 2 {
            frame.set_cursor_position((
                footer.x
                    + 2
                    + (modal.cursor - offset).min(footer.width.saturating_sub(3) as usize) as u16,
                footer.y,
            ));
        }
    }
    controls.push(Line::raw("PgUp/PgDn: scroll prompt"));
    frame.render_widget(Paragraph::new(controls), footer);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AppMode;
    use crate::session::{
        askpass_channel, interactive_auth::InteractiveAuth, Session, SessionConfig, SessionMeta,
    };
    use crate::test_support::{frame_at, resolved_default, themed_app};
    use ratatui::layout::Rect;
    use std::time::{Duration, Instant};

    /// Word wrap needs more rows than `width.div_ceil` when a word does not
    /// fit the leftover of a visual line: three 14-wide words need three
    /// rows at width 28, not two. The scroll range must use the exact
    /// wrapped count, or the final prompt line stays out of reach on narrow
    /// terminals even with scroll pinned at max.
    #[test]
    fn narrow_terminal_max_scroll_keeps_final_prompt_line_visible() {
        const PROMPT: &str = "Password: aaaaaaaaaaaaaa bbbbbbbbbbbbbb cccccccccccccc\ndddddddddddddd eeeeeeeeeeeeee ffffffffffffff gggggggggggggg\nhhhhhhhhhhhhhh iiiiiiiiiiiiii jjjjjjjjjjjjjj\nFINAL-TOKEN-VISIBLE";
        let mut app = themed_app(resolved_default());
        let config = SessionConfig {
            argv: vec!["sh".into(), "-c".into(), "sleep 30".into()],
            display_name: "offline".into(),
            meta: SessionMeta::default(),
            pending_secret: None,
            key_push_identity: None,
            host_name: "offline".into(),
        };
        let mut session = Session::spawn(config, 24, 100, None).unwrap();
        let auth = InteractiveAuth::new().unwrap();
        let env = auth.channel_env(std::path::Path::new("unused"));
        let get = |name: &str| env.iter().find(|(key, _)| key == name).unwrap().1.clone();
        let path = std::path::PathBuf::from(get(askpass_channel::SOCKET_ENV));
        let token = get(askpass_channel::TOKEN_ENV);
        session.auth = Some(auth);
        app.sessions.push(session);
        app.active_session = Some(0);
        app.mode = AppMode::Connecting;
        let helper = std::thread::spawn(move || {
            askpass_channel::request_answer(&path, &token, PROMPT, Duration::from_secs(3))
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            for session in &mut app.sessions {
                session.drain();
            }
            app.poll_authentication();
            if app.auth_modal.is_some() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "authentication modal never appeared"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        app.auth_modal.as_mut().unwrap().scroll = u16::MAX;
        let area = Rect::new(0, 0, 30, 12);
        let buf = frame_at(area, |f| render(f, &app));
        let body: String = (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf.cell((x, y)).unwrap().symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        drop(helper);
        assert!(
            body.contains("FINAL-TOKEN-VISIBLE"),
            "final prompt line scrolled out of view at max scroll:\n{body}"
        );
    }
}
