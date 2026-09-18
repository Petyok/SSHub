use super::*;
use crate::session::auth_prompt::PromptClass;

/// No Debug or Clone: the input must never reach diagnostic formatting.
pub struct AuthModal {
    pub session: usize,
    pub prompt: String,
    pub class: PromptClass,
    pub attempt: u8,
    pub can_remember: bool,
    pub remember: bool,
    pub value: String,
    pub cursor: usize,
    pub checkbox_focused: bool,
    pub changed_key: bool,
    pub scroll: u16,
}

impl App {
    /// Called after every session drain, including background sessions.
    pub(crate) fn poll_authentication(&mut self) {
        let mut ready = Vec::new();
        for (index, session) in self.sessions.iter_mut().enumerate() {
            let connected = session.is_connected();
            if let Some(auth) = session.auth.as_mut() {
                ready.extend(
                    auth.take_ready_secrets(connected)
                        .into_iter()
                        .map(|secret| (index, secret)),
                );
            }
        }
        for (index, secret) in ready {
            if let Err(e) = self.put_secret(&secret.target.key(), &secret.value) {
                let notice =
                    "Connected, but saving the credential failed; check your credential store";
                self.host_notice = Some(notice.into());
                self.sessions[index].set_copy_notice(notice.into());
                // The modal notice is transient: after the fact a silent
                // persistence failure is undebuggable. Leave one audit error
                // row with the cause. Secret kind + ids only, never the value.
                let session = &self.sessions[index];
                let _ = self.store.log_auth_event(
                    &session.host_name,
                    session.meta.user.as_deref(),
                    session.meta.proxy_jump.as_deref().unwrap_or("direct"),
                    "fail",
                    &format!(
                        "remember-me save failed for credential {}: {e:#}",
                        secret.target.key()
                    ),
                    None,
                );
            }
        }
        if let Some(modal) = &self.auth_modal {
            let alive = self.sessions.get(modal.session).is_some_and(|s| {
                modal.changed_key || s.auth.as_ref().is_some_and(|a| a.request.is_some())
            });
            if alive {
                return;
            }
            self.auth_modal = None;
            self.mode = AppMode::Session;
        }
        // Do not replace another editor; pending requests keep their deadlines.
        if !matches!(
            self.mode,
            AppMode::Normal | AppMode::Connecting | AppMode::Session
        ) {
            return;
        }
        let candidate = self.sessions.iter().position(|s| {
            s.auth.as_ref().is_some_and(|a| {
                a.request.is_some()
                    || (s.phase.is_terminal() && s.host_key_changed() && !a.changed_key_shown)
            })
        });
        let Some(index) = candidate else {
            return;
        };
        let session = &mut self.sessions[index];
        let changed_key = session.phase.is_terminal() && session.host_key_changed();
        let auth = session
            .auth
            .as_mut()
            .expect("candidate has authentication state");
        let prompt = if changed_key {
            auth.changed_key_shown = true;
            "WARNING: the server identity changed. This may be an attack. Verify the new fingerprint independently before removing the old key and reconnecting.".into()
        } else {
            auth.request
                .as_ref()
                .expect("candidate has request")
                .prompt
                .clone()
        };
        let class = if changed_key {
            PromptClass::HostKey
        } else {
            PromptClass::classify(&prompt)
        };
        // Offered, never pre-ticked: the prompt class alone decides *where* a
        // remembered answer lands (`Password:` -> the host row, whatever the
        // user actually typed), so persisting a credential must be a
        // deliberate act. A pre-checked box stored key passphrases as host
        // passwords for people who never asked for either.
        let can_remember = auth.save_target(class).is_some();
        self.auth_modal = Some(AuthModal {
            session: index,
            prompt,
            class,
            attempt: auth.attempts,
            can_remember,
            remember: false,
            value: String::new(),
            cursor: 0,
            checkbox_focused: false,
            changed_key,
            scroll: 0,
        });
        self.active_session = Some(index);
        self.mode = AppMode::AuthPrompt;
    }

    pub(crate) fn paste_auth(&mut self, text: &str) {
        let Some(modal) = self.auth_modal.as_mut() else {
            return;
        };
        if modal.class == PromptClass::HostKey || modal.checkbox_focused {
            return;
        }
        // Reject a multiline paste rather than silently changing a credential.
        if text.chars().any(char::is_control)
            || modal.value.len().saturating_add(text.len())
                > crate::session::askpass_channel::MAX_ANSWER_BYTES
        {
            return;
        }
        let byte = text_input::byte_index(&modal.value, modal.cursor);
        modal.value.insert_str(byte, text);
        modal.cursor += text_input::char_len(text);
    }

    pub(crate) fn handle_key_auth(&mut self, key: KeyEvent) -> Result<()> {
        let Some(modal) = self.auth_modal.as_mut() else {
            return Ok(());
        };
        if key.code == KeyCode::Esc
            || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
        {
            let modal = self.auth_modal.take().unwrap();
            if !modal.changed_key {
                self.sessions[modal.session].cancel_authentication();
            }
            self.mode = AppMode::Session;
            return Ok(());
        }
        if key.code == KeyCode::PageDown {
            modal.scroll = modal.scroll.saturating_add(3);
            return Ok(());
        }
        if key.code == KeyCode::PageUp {
            modal.scroll = modal.scroll.saturating_sub(3);
            return Ok(());
        }
        if modal.class == PromptClass::HostKey {
            // Neither Enter nor a default selection can change host trust.
            if key.code == KeyCode::Char('n') {
                let modal = self.auth_modal.take().unwrap();
                if !modal.changed_key {
                    self.sessions[modal.session].cancel_authentication();
                }
                self.mode = AppMode::Session;
            } else if key.code == KeyCode::Char(if modal.changed_key { 'A' } else { 'y' }) {
                let modal = self.auth_modal.take().unwrap();
                self.mode = AppMode::Session;
                if modal.changed_key {
                    self.repair_auth_host_key(modal.session)?;
                } else if let Some(auth) = self.sessions[modal.session].auth.as_mut() {
                    if auth.answer("yes".into(), false).is_err() {
                        self.sessions[modal.session].cancel_authentication();
                    }
                }
            }
            return Ok(());
        }
        if key.code == KeyCode::Tab && modal.can_remember {
            modal.checkbox_focused = !modal.checkbox_focused;
        } else if modal.checkbox_focused && key.code == KeyCode::Char(' ') {
            modal.remember = !modal.remember;
        } else if key.code == KeyCode::Enter {
            let modal = self.auth_modal.take().unwrap();
            let session = &mut self.sessions[modal.session];
            if let Some(auth) = session.auth.as_mut() {
                if auth
                    .answer(modal.value, modal.remember && modal.can_remember)
                    .is_err()
                {
                    session.cancel_authentication();
                }
            }
            self.mode = if matches!(
                session.phase,
                crate::session::SessionPhase::Connecting { .. }
            ) {
                AppMode::Connecting
            } else {
                AppMode::Session
            };
        } else if !modal.checkbox_focused {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                match key.code {
                    KeyCode::Char('u') => {
                        modal.cursor =
                            text_input::clear_before_cursor(&mut modal.value, modal.cursor)
                    }
                    KeyCode::Char('w') => {
                        modal.cursor =
                            text_input::delete_word_before(&mut modal.value, modal.cursor)
                    }
                    _ => {}
                }
            } else if key.modifiers.contains(KeyModifiers::ALT) {
                // No reveal binding and no forwarding to the PTY.
            } else if text_input::handle_cursor_key(key.code, &mut modal.value, &mut modal.cursor)
                .is_none()
            {
                match key.code {
                    KeyCode::Backspace => {
                        modal.cursor = text_input::backspace_at(&mut modal.value, modal.cursor)
                    }
                    KeyCode::Char(c)
                        if !c.is_control()
                            && modal.value.len() + c.len_utf8()
                                <= crate::session::askpass_channel::MAX_ANSWER_BYTES =>
                    {
                        modal.cursor = text_input::insert_at(&mut modal.value, modal.cursor, c)
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn repair_auth_host_key(&mut self, index: usize) -> Result<()> {
        let session = &self.sessions[index];
        let target = session.changed_key_target();
        let config = session.config.clone();
        let entry = self
            .hosts
            .iter()
            .find(|h| match session.meta.host_id {
                Some(id) => h.managed_id() == Some(id),
                None => h.managed_id().is_none() && h.name() == session.host_name,
            })
            .cloned();
        let Some((spec, path)) = target else {
            self.sessions[index].set_copy_notice(
                "Cannot safely identify the changed known_hosts entry; repair it manually".into(),
            );
            return Ok(());
        };
        if crate::known_hosts::remove_host(&spec, &path).is_err() {
            self.sessions[index].set_copy_notice(
                "Could not remove the changed known_hosts entry; repair it manually".into(),
            );
            return Ok(());
        }
        self.active_session = Some(index);
        self.close_active_session();
        if let Some(entry) = entry {
            self.connect_host_entry(entry)
        } else {
            self.spawn_embedded_session(
                config.argv,
                config.display_name,
                config.meta,
                None,
                &config.host_name,
                false,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{
        askpass_channel, interactive_auth::InteractiveAuth, Session, SessionConfig, SessionMeta,
    };
    use std::time::{Duration, Instant};

    fn tick_until(app: &mut App, predicate: impl Fn(&App) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            for session in &mut app.sessions {
                session.drain();
            }
            app.poll_authentication();
            if predicate(app) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "authentication transition timed out"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn channel_modal_masks_paste_reopens_and_caps_attempts() {
        let mut app = crate::app::tests::test_app(vec![]);
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
        for attempt in 0..4 {
            let path = path.clone();
            let token = token.clone();
            let helper = std::thread::spawn(move || {
                askpass_channel::request_answer(&path, &token, "Password:", Duration::from_secs(3))
            });
            if attempt == 3 {
                tick_until(&mut app, |app| app.sessions[0].phase.is_terminal());
                assert!(helper.join().unwrap().is_err());
                assert!(app.auth_modal.is_none());
                break;
            }
            tick_until(&mut app, |app| app.mode == AppMode::AuthPrompt);
            assert_eq!(app.auth_modal.as_ref().unwrap().attempt, attempt + 1);
            assert!(!app.auth_modal.as_ref().unwrap().can_remember);
            app.handle_paste("synthetic-secret").unwrap();
            assert!(!helper.is_finished());
            let backend = ratatui::backend::TestBackend::new(100, 24);
            let mut terminal = ratatui::Terminal::new(backend).unwrap();
            terminal
                .draw(|frame| crate::tui::render(frame, &app))
                .unwrap();
            let rendered: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect();
            assert!(!rendered.contains("synthetic-secret"));
            assert!(rendered.contains("Password:"));
            app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
                .unwrap();
            assert_eq!(helper.join().unwrap().unwrap(), "synthetic-secret");
            assert!(app.auth_modal.is_none());
            assert!(!app.sessions[0].debug_log().contains("synthetic-secret"));
            assert!(!app.sessions[0]
                .take_diagnostics()
                .join(" ")
                .contains("synthetic-secret"));
            assert!(!app.sessions[0]
                .screen_tail_snippet()
                .contains("synthetic-secret"));
        }
    }
    #[test]
    fn concurrent_auth_requests_serialize_in_session_order() {
        let mut app = crate::app::tests::test_app(vec![]);
        let mut channels = Vec::new();
        for name in ["alpha", "beta"] {
            let config = SessionConfig {
                argv: vec!["sh".into(), "-c".into(), "sleep 30".into()],
                display_name: name.into(),
                meta: SessionMeta::default(),
                pending_secret: None,
                key_push_identity: None,
                host_name: name.into(),
            };
            let mut session = Session::spawn(config, 24, 100, None).unwrap();
            let auth = InteractiveAuth::new().unwrap();
            let env = auth.channel_env(std::path::Path::new("unused"));
            let get = |name: &str| env.iter().find(|(key, _)| key == name).unwrap().1.clone();
            channels.push((
                std::path::PathBuf::from(get(askpass_channel::SOCKET_ENV)),
                get(askpass_channel::TOKEN_ENV),
            ));
            session.auth = Some(auth);
            app.sessions.push(session);
        }
        app.active_session = Some(0);
        app.mode = AppMode::Connecting;
        let prompts = ["Password for alpha:", "Password for beta:"];
        let mut helpers: Vec<_> = channels
            .into_iter()
            .zip(prompts)
            .map(|((path, token), prompt)| {
                std::thread::spawn(move || {
                    askpass_channel::request_answer(&path, &token, prompt, Duration::from_secs(5))
                })
            })
            .collect();
        // Both helpers blocked in `request_answer`; the first poll must raise
        // the modal for session 0 only, even though session 1 is waiting too.
        tick_until(&mut app, |app| {
            app.mode == AppMode::AuthPrompt
                && app.auth_modal.as_ref().is_some_and(|m| m.session == 0)
                && app.sessions[1]
                    .auth
                    .as_ref()
                    .is_some_and(|a| a.request.is_some())
        });
        assert_eq!(app.auth_modal.as_ref().unwrap().prompt, prompts[0]);
        // Extra polls must not let the queued request steal the open modal.
        for _ in 0..5 {
            for session in &mut app.sessions {
                session.drain();
            }
            app.poll_authentication();
            assert_eq!(app.auth_modal.as_ref().unwrap().session, 0);
        }
        assert!(app.sessions[1]
            .auth
            .as_ref()
            .is_some_and(|a| a.request.is_some()));
        // Answering session 0 releases session 1's modal with its own prompt.
        app.handle_paste("first-secret").unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
            .unwrap();
        assert_eq!(helpers.remove(0).join().unwrap().unwrap(), "first-secret");
        // Serialization oracle: session 1's helper must still be blocked in
        // `request_answer` — its answer cannot be delivered until its own
        // modal opens below. If requests were answered out of order or
        // broadcast, this handle would already be finished here.
        assert!(!helpers[0].is_finished());
        tick_until(&mut app, |app| {
            app.mode == AppMode::AuthPrompt
                && app.auth_modal.as_ref().is_some_and(|m| m.session == 1)
        });
        assert_eq!(app.auth_modal.as_ref().unwrap().prompt, prompts[1]);
        app.handle_paste("second-secret").unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
            .unwrap();
        assert_eq!(helpers.remove(0).join().unwrap().unwrap(), "second-secret");
        assert!(app.auth_modal.is_none());
    }

    #[test]
    fn remember_only_eligible_targets_and_only_after_connected() {
        use crate::credentials::PasswordStore;

        fn wait_for(app: &mut App, stage: &str, predicate: impl Fn(&App) -> bool) {
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                for session in &mut app.sessions {
                    session.drain();
                }
                app.poll_authentication();
                if predicate(app) {
                    return;
                }
                assert!(Instant::now() < deadline,
                    "authentication fixture timed out at {stage}: terminal={:?}, connected={}, terminal_phase={}, modal={}",
                    app.sessions[0].parser.screen().size(), app.sessions[0].is_connected(),
                    app.sessions[0].phase.is_terminal(), app.auth_modal.is_some());
                std::thread::sleep(Duration::from_millis(1));
            }
        }

        for (prompt, managed, remember, succeeds, expected) in [
            (
                "Password:",
                true,
                true,
                true,
                Some(crate::credentials::host_key(7)),
            ),
            (
                "Enter passphrase for key:",
                true,
                true,
                true,
                Some(crate::credentials::identity_key(9)),
            ),
            ("Verification code:", true, true, true, None),
            ("Password:", false, true, true, None),
            ("Password:", true, false, true, None),
            ("Password:", true, true, false, None),
        ] {
            let (mut app, store) = crate::app::tests::test_app_with_secrets(vec![]);
            // The embedded launch sizes its PTY from the last rendered terminal area.
            app.terminal_area = ratatui::layout::Rect::new(0, 0, 100, 24);
            let root = tempfile::tempdir().unwrap();
            let gate = root.path().join("release-child");
            // The child cannot emit an authenticated marker until after the
            // helper received the modal answer and the pre-success assertions.
            let config = SessionConfig {
                argv: vec![
                    "sh".into(), "-c".into(),
                    "while [ ! -f \"$1\" ]; do sleep 0.01; done; if [ \"$2\" = yes ]; then printf 'Authenticated to offline using password.\\n' >&2; exec cat; else exit 1; fi".into(),
                    "auth-fixture".into(), gate.to_string_lossy().into_owned(),
                    if succeeds { "yes" } else { "no" }.into(),
                ],
                display_name: "offline".into(), meta: SessionMeta::default(),
                pending_secret: None, key_push_identity: None, host_name: "offline".into(),
            };
            app.spawn_embedded_session(
                config.argv,
                config.display_name,
                config.meta,
                None,
                &config.host_name,
                false,
            )
            .unwrap();
            let mut session = app.sessions.pop().unwrap();
            let log = crate::session_log::SessionLogWriter::open(
                root.path(),
                "offline",
                None,
                1024 * 1024,
                2,
            )
            .unwrap();
            let log_path = log.path().to_owned();
            session.set_log(log);
            let mut auth = InteractiveAuth::new().unwrap();
            auth.host_id = managed.then_some(7);
            auth.identity_id = Some(9);
            let env = auth.channel_env(std::path::Path::new("unused"));
            let get = |name: &str| env.iter().find(|(key, _)| key == name).unwrap().1.clone();
            let path = std::path::PathBuf::from(get(askpass_channel::SOCKET_ENV));
            let token = get(askpass_channel::TOKEN_ENV);
            session.auth = Some(auth);
            app.sessions.push(session);
            app.active_session = Some(0);
            app.sessions[0].write(b"before-first-prompt\n").unwrap();
            app.sessions[0].write_paste(b"initial-paste\n").unwrap();
            app.mode = AppMode::Connecting;
            let helper = std::thread::spawn(move || {
                askpass_channel::request_answer(&path, &token, prompt, Duration::from_secs(3))
            });
            wait_for(&mut app, "opening authentication modal", |app| {
                app.auth_modal.is_some()
            });
            let eligible = managed && prompt != "Verification code:";
            assert_eq!(app.auth_modal.as_ref().unwrap().can_remember, eligible);
            assert!(
                !app.auth_modal.as_ref().unwrap().remember,
                "remembering is opt-in: the box starts clear even when eligible"
            );
            if eligible && remember {
                app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()))
                    .unwrap();
                app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::empty()))
                    .unwrap();
                app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()))
                    .unwrap();
            }
            app.sessions[0].write(b"during-prompt\n").unwrap();
            app.handle_paste("synthetic-secret").unwrap();
            app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
                .unwrap();
            assert_eq!(helper.join().unwrap().unwrap(), "synthetic-secret");
            app.sessions[0].write(b"between-prompts\n").unwrap();
            app.sessions[0].write_paste(b"between-paste\n").unwrap();
            app.poll_authentication();
            assert!(!app.sessions[0].is_connected());
            for key in [
                crate::credentials::host_key(7),
                crate::credentials::identity_key(9),
            ] {
                assert_eq!(store.get(&key).unwrap(), None);
            }
            std::fs::write(&gate, b"release").unwrap();
            wait_for(
                &mut app,
                if succeeds {
                    "connected marker after gate release"
                } else {
                    "child exit after gate release"
                },
                |app| {
                    if succeeds {
                        app.sessions[0].is_connected()
                    } else {
                        app.sessions[0].phase.is_terminal()
                    }
                },
            );
            assert_eq!(app.sessions[0].is_connected(), succeeds);
            for key in [
                crate::credentials::host_key(7),
                crate::credentials::identity_key(9),
            ] {
                assert_eq!(
                    store.get(&key).unwrap(),
                    expected
                        .as_ref()
                        .filter(|target| *target == &key)
                        .map(|_| "synthetic-secret".to_owned())
                );
            }
            if succeeds {
                app.sessions[0].write(b"post-auth-input\n").unwrap();
                app.sessions[0].write_paste(b"post-auth-paste\n").unwrap();
                wait_for(&mut app, "post-auth paste visible in screen tail", |app| {
                    app.sessions[0]
                        .screen_tail_snippet()
                        .contains("post-auth-paste")
                });
                let screen = app.sessions[0].parser.screen().contents();
                assert!(screen.contains("post-auth-input"));
                for forbidden in [
                    "before-first-prompt",
                    "initial-paste",
                    "during-prompt",
                    "between-prompts",
                    "between-paste",
                    "synthetic-secret",
                ] {
                    assert!(
                        !screen.contains(forbidden),
                        "pre-auth input leaked into PTY: {forbidden}"
                    );
                }
            }
            assert!(!app.sessions[0].debug_log().contains("synthetic-secret"));
            for line in app.sessions[0].take_diagnostics() {
                assert!(!line.contains("synthetic-secret"));
                app.push_ssh_log(crate::ssh::probe::SshLogEntry {
                    host_name: "offline".into(),
                    line,
                    level: crate::ssh::probe::LogLevel::Success,
                    timestamp: 0,
                });
            }
            assert!(app
                .ssh_log
                .iter()
                .all(|entry| !entry.line.contains("synthetic-secret")));
            let audit = app.store.list_auth_events(100).unwrap();
            assert!(
                !audit.is_empty(),
                "exercise the real session launch audit path"
            );
            assert!(audit.iter().all(|event| !event
                .note
                .as_ref()
                .is_some_and(|note| note.contains("synthetic-secret"))));
            assert!(
                !audit.iter().any(|event| event.status == "fail"),
                "successful saves stay silent: failures alone earn an audit row"
            );
            app.sessions.clear(); // Flush the real transcript writer on drop.
            let transcript = std::fs::read_to_string(log_path).unwrap();
            assert!(!transcript.contains("synthetic-secret"));
            if succeeds {
                assert!(transcript.contains("post-auth-input"));
            }
        }
    }

    /// Oracle: the remember-me-only row — secret saved via the modal (same
    /// `put_secret` helper the host form uses), row flag untouched, exactly as
    /// `poll_authentication` leaves it. Intended UX (README: stored secrets
    /// are reused; the pre-channel flow auto-answers via SSH_ASKPASS so the
    /// prompt never shows): the next connect must find the secret and answer
    /// with no modal left standing.
    #[test]
    fn restore_finds_remembered_password_without_row_flag() {
        use crate::session::PendingSecret;

        let (mut app, _secrets) = crate::app::tests::test_app_with_secrets(vec![]);
        let created = app
            .store
            .create_host(&crate::store::NewHost::launcher("restored", "10.99.0.1"))
            .unwrap();
        assert!(
            !created.has_password,
            "remember-me never touches the row flag"
        );
        // Same helper the host form uses to persist a password.
        app.put_secret(&crate::credentials::host_key(created.id), "saved-pw")
            .unwrap();
        let entry = crate::app::HostEntry::Managed(created);
        // Lookup leg: the saved secret must resolve to a pending secret.
        let (pending, _diag) =
            crate::app::resolve_pending_secret(&entry, None, app.password_store.as_ref());
        assert!(
            matches!(&pending, Some(PendingSecret::Password(pw)) if pw == "saved-pw"),
            "remember-me-saved password must resolve on the next connect"
        );
        // Delivery leg: the helper receives the saved value, no modal remains.
        app.terminal_area = ratatui::layout::Rect::new(0, 0, 100, 24);
        let config = SessionConfig {
            argv: vec!["sh".into(), "-c".into(), "sleep 30".into()],
            display_name: "restored".into(),
            meta: SessionMeta::default(),
            pending_secret: pending.clone(),
            key_push_identity: None,
            host_name: "restored".into(),
        };
        let mut session = Session::spawn(config, 24, 100, None).unwrap();
        session.auth = Some(InteractiveAuth::new().unwrap());
        let env = session
            .auth
            .as_ref()
            .unwrap()
            .channel_env(std::path::Path::new("unused"));
        let get = |name: &str| env.iter().find(|(key, _)| key == name).unwrap().1.clone();
        let path = std::path::PathBuf::from(get(askpass_channel::SOCKET_ENV));
        let token = get(askpass_channel::TOKEN_ENV);
        app.sessions.push(session);
        app.active_session = Some(0);
        app.mode = AppMode::Connecting;
        let helper = std::thread::spawn(move || {
            askpass_channel::request_answer(&path, &token, "Password:", Duration::from_secs(3))
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            for session in &mut app.sessions {
                session.drain();
            }
            app.poll_authentication();
            if helper.is_finished() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "stored secret was never delivered to the helper"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(helper.join().unwrap().unwrap(), "saved-pw");
        app.poll_authentication();
        assert!(
            app.auth_modal.is_none(),
            "a stored secret auto-answers; no modal remains"
        );
    }

    /// Oracle: the `env -i` Secret Service failure from the keyring probe
    /// (dbus-daemon autolaunch disabled, no session bus). A store that fails
    /// every operation exactly like `OsKeyring` does there.
    struct UnwritablePasswordStore;

    impl crate::credentials::PasswordStore for UnwritablePasswordStore {
        fn get(&self, _key: &str) -> anyhow::Result<Option<String>> {
            Err(anyhow::anyhow!(
                "keyring: Platform secure storage failure: DBus error: Using X11 for dbus-daemon autolaunch was disabled at compile time, set your DBUS_SESSION_BUS_ADDRESS instead"
            ))
        }
        fn set(&self, _key: &str, _password: &str) -> anyhow::Result<()> {
            Err(anyhow::anyhow!(
                "Platform secure storage failure: DBus error: Using X11 for dbus-daemon autolaunch was disabled at compile time, set your DBUS_SESSION_BUS_ADDRESS instead"
            ))
        }
        fn delete(&self, _key: &str) -> anyhow::Result<()> {
            Err(anyhow::anyhow!(
                "Platform secure storage failure: DBus error: Using X11 for dbus-daemon autolaunch was disabled at compile time, set your DBUS_SESSION_BUS_ADDRESS instead"
            ))
        }
    }

    /// A remember-me persistence failure only flashed a transient modal
    /// notice, so after the fact there was nothing to debug. It must leave
    /// exactly one audit error row carrying the cause — never the secret.
    #[test]
    fn failed_remember_save_leaves_one_audit_error_row() {
        fn wait_for(app: &mut App, stage: &str, predicate: impl Fn(&App) -> bool) {
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                for session in &mut app.sessions {
                    session.drain();
                }
                app.poll_authentication();
                if predicate(app) {
                    return;
                }
                assert!(
                    Instant::now() < deadline,
                    "authentication fixture timed out at {stage}"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
        }

        let mut app =
            crate::app::tests::test_app_with_store(vec![], Box::new(UnwritablePasswordStore));
        app.terminal_area = ratatui::layout::Rect::new(0, 0, 100, 24);
        let root = tempfile::tempdir().unwrap();
        let gate = root.path().join("release-child");
        let config = SessionConfig {
            argv: vec![
                "sh".into(),
                "-c".into(),
                "while [ ! -f \"$1\" ]; do sleep 0.01; done; if [ \"$2\" = yes ]; then printf 'Authenticated to offline using password.\\n' >&2; exec cat; else exit 1; fi".into(),
                "auth-fixture".into(),
                gate.to_string_lossy().into_owned(),
                "yes".into(),
            ],
            display_name: "offline".into(),
            meta: SessionMeta::default(),
            pending_secret: None,
            key_push_identity: None,
            host_name: "offline".into(),
        };
        app.spawn_embedded_session(
            config.argv,
            config.display_name,
            config.meta,
            None,
            &config.host_name,
            false,
        )
        .unwrap();
        let mut session = app.sessions.pop().unwrap();
        let mut auth = InteractiveAuth::new().unwrap();
        auth.host_id = Some(7);
        auth.identity_id = Some(9);
        let env = auth.channel_env(std::path::Path::new("unused"));
        let get = |name: &str| env.iter().find(|(key, _)| key == name).unwrap().1.clone();
        let path = std::path::PathBuf::from(get(askpass_channel::SOCKET_ENV));
        let token = get(askpass_channel::TOKEN_ENV);
        session.auth = Some(auth);
        app.sessions.push(session);
        app.active_session = Some(0);
        app.mode = AppMode::Connecting;
        let helper = std::thread::spawn(move || {
            askpass_channel::request_answer(&path, &token, "Password:", Duration::from_secs(3))
        });
        wait_for(&mut app, "opening authentication modal", |app| {
            app.auth_modal.is_some()
        });
        assert!(app.auth_modal.as_ref().unwrap().can_remember);
        // Remembering is opt-in, so tick the box the way a user would.
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()))
            .unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::empty()))
            .unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()))
            .unwrap();
        assert!(app.auth_modal.as_ref().unwrap().remember);
        app.handle_paste("synthetic-secret").unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
            .unwrap();
        assert_eq!(helper.join().unwrap().unwrap(), "synthetic-secret");
        std::fs::write(&gate, b"release").unwrap();
        wait_for(&mut app, "connected marker after gate release", |app| {
            app.sessions[0].is_connected()
        });
        assert!(app.sessions[0].is_connected());
        let audit = app.store.list_auth_events(100).unwrap();
        let failures: Vec<_> = audit
            .iter()
            .filter(|event| event.status == "fail")
            .collect();
        assert_eq!(
            failures.len(),
            1,
            "a failed remember-me save must leave one auditable row"
        );
        let note = failures[0].note.as_deref().unwrap_or_default();
        assert!(
            note.contains("remember"),
            "row names the failed operation: {note}"
        );
        assert!(
            note.contains("autolaunch"),
            "row carries the failure cause: {note}"
        );
        assert!(
            !note.contains("synthetic-secret"),
            "row must never carry the secret"
        );
        assert_eq!(failures[0].host_name, "offline");
    }

    #[test]
    fn cancellation_closes_helper_then_child_and_fails_open_screen() {
        let mut app = crate::app::tests::test_app(vec![]);
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
        let path = {
            let env = auth.channel_env(std::path::Path::new("unused"));
            let get = |name: &str| env.iter().find(|(key, _)| key == name).unwrap().1.clone();
            std::path::PathBuf::from(get(askpass_channel::SOCKET_ENV))
        };
        session.auth = Some(auth);
        app.sessions.push(session);
        app.active_session = Some(0);
        app.mode = AppMode::Connecting;
        let token = {
            let env = app.sessions[0]
                .auth
                .as_ref()
                .unwrap()
                .channel_env(std::path::Path::new("unused"));
            env.into_iter()
                .find(|(key, _)| key == askpass_channel::TOKEN_ENV)
                .unwrap()
                .1
        };
        let helper = std::thread::spawn(move || {
            askpass_channel::request_answer(&path, &token, "Password:", Duration::from_secs(3))
        });
        tick_until(&mut app, |app| app.mode == AppMode::AuthPrompt);
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()))
            .unwrap();
        assert!(helper.join().unwrap().is_err());
        tick_until(&mut app, |app| app.sessions[0].phase.is_terminal());
        assert!(!matches!(app.mode, AppMode::AuthPrompt));
        let env = app.sessions[0]
            .auth
            .as_ref()
            .unwrap()
            .channel_env(std::path::Path::new("unused"));
        assert!(env.is_empty(), "socket must be removed with the session");
    }

    #[test]
    fn host_key_modal_answer_is_never_a_credential() {
        let prompt = "The authenticity of host '[127.0.0.1]:2222 ([127.0.0.1]:2222)' can't be established.\nED25519 key fingerprint is: SHA256:MXUTTgmDROalky/NE8jlcGeW+758bjK3+hOz3jbY4l4\nThis key is not known by any other names.\nAre you sure you want to continue connecting (yes/no/[fingerprint])?";
        let mut app = crate::app::tests::test_app(vec![]);
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
            askpass_channel::request_answer(&path, &token, prompt, Duration::from_secs(3))
        });
        tick_until(&mut app, |app| app.mode == AppMode::AuthPrompt);
        let modal = app.auth_modal.as_ref().unwrap();
        assert!(!modal.can_remember);
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
            .unwrap();
        assert!(
            !helper.is_finished(),
            "Enter must not act on a trust question"
        );
        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::empty()))
            .unwrap();
        assert_eq!(helper.join().unwrap().unwrap(), "yes");
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()))
            .unwrap();
    }

    #[test]
    fn changed_key_repair_requires_uppercase_and_reconnects_adhoc_config() {
        let root = tempfile::tempdir().unwrap();
        let keys = root.path().join("custom known_hosts");
        let original = "[alias]:2222 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBbSwmRXm0WEQzC3oHnJkV0tBk3kCQh8mFjWz3nLx9oK\n# keep this comment\n";
        std::fs::write(&keys, original).unwrap();
        let mut app = crate::app::tests::test_app(vec![]);
        let config = SessionConfig {
            argv: vec!["sh".into(), "-c".into(), "printf 'WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!\\nOffending ED25519 key in %s:1\\nHost key for [alias]:2222 has changed and you have requested strict checking.\\n' \"$1\" >&2; sleep 30".into(), "fixture".into(), keys.to_string_lossy().into_owned()],
            display_name: "Ad-hoc label".into(), meta: SessionMeta::default(),
            pending_secret: None, key_push_identity: None, host_name: "original-adhoc-target".into(),
        };
        let mut session = Session::spawn(config, 24, 100, None).unwrap();
        session.auth = Some(InteractiveAuth::new().unwrap());
        app.sessions.push(session);
        app.active_session = Some(0);
        app.mode = AppMode::Connecting;
        tick_until(&mut app, |app| app.sessions[0].host_key_changed());
        app.sessions[0].cancel_authentication();
        // The retained reconnect configuration runs a local child, never ssh.
        app.sessions[0].config.argv = vec![
            "sh".into(),
            "-c".into(),
            "printf reconnected; sleep 30".into(),
        ];
        app.poll_authentication();
        assert!(app.auth_modal.as_ref().unwrap().changed_key);
        for code in [KeyCode::Enter, KeyCode::Char('a')] {
            app.handle_key(KeyEvent::new(code, KeyModifiers::empty()))
                .unwrap();
            assert_eq!(std::fs::read_to_string(&keys).unwrap(), original);
            assert!(app.auth_modal.is_some());
        }
        app.handle_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::empty()))
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(&keys).unwrap(),
            "# keep this comment\n"
        );
        tick_until(&mut app, |app| {
            app.sessions[0]
                .screen_tail_snippet()
                .contains("reconnected")
        });
        assert_eq!(app.sessions[0].host_name, "original-adhoc-target");
    }
}
