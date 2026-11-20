use super::*;
use crate::store::NewSnippet;

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn ch(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty())
}

/// Build an app with one live session (running `cat`, which stays alive to
/// receive writes) and two stored snippets, positioned in `Session` mode.
fn app_with_session_and_snippets() -> App {
    let mut app = test_app(vec![("edge", host("edge"))]);
    app.terminal_area = ratatui::layout::Rect::new(0, 0, 80, 24);

    app.store()
        .create_snippet(&NewSnippet {
            name: "restart nginx".into(),
            command: "sudo systemctl restart nginx".into(),
            description: None,
            tags: vec!["web".into()],
        })
        .unwrap();
    app.store()
        .create_snippet(&NewSnippet {
            name: "disk usage".into(),
            command: "df -h".into(),
            description: None,
            tags: vec![],
        })
        .unwrap();

    let config = crate::session::SessionConfig {
        hops: Vec::new(),
        argv: vec!["cat".into()],
        display_name: "edge".into(),
        meta: crate::session::SessionMeta::default(),
        pending_secret: None,
        key_push_identity: None,
        host_name: "edge".into(),
    };
    let mut session = crate::session::Session::spawn(config, (24, 80), None).unwrap();
    // The picker only opens over a *running* session; a bare `cat` never emits
    // output to trip the reveal, so mark it running explicitly.
    session.phase = crate::session::SessionPhase::Running {
        started_at: std::time::Instant::now(),
    };
    session.local_shell = true;
    app.sessions.push(session);
    app.active_session = Some(0);
    app.mode = AppMode::Session;
    app
}

#[test]
fn session_snippets_key_opens_picker_over_session() {
    let mut app = app_with_session_and_snippets();

    // Ctrl+N (default bind) opens the picker while the session keeps rendering.
    app.handle_key(ctrl('n')).unwrap();
    assert_eq!(app.mode, AppMode::SnippetPicker);
    assert!(app.snippet_picker_over_session());
    assert!(app.session_is_rendered());

    let state = app.snippet_picker.as_ref().unwrap();
    assert_eq!(state.results.len(), 2, "both snippets shown initially");
    assert_eq!(state.return_mode, AppMode::Session);
}

#[test]
fn picker_fuzzy_filters_by_typing() {
    let mut app = app_with_session_and_snippets();
    app.handle_key(ctrl('n')).unwrap();

    // Type "nginx": only the first snippet matches.
    for c in "nginx".chars() {
        app.handle_key(ch(c)).unwrap();
    }
    let state = app.snippet_picker.as_ref().unwrap();
    assert_eq!(state.results.len(), 1);
    assert_eq!(app.snippets[state.results[0]].name, "restart nginx");
}

#[test]
fn enter_runs_snippet_and_returns_to_session() {
    let mut app = app_with_session_and_snippets();
    app.handle_key(ctrl('n')).unwrap();

    // Enter injects the selected command into the PTY and closes the picker.
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
        .unwrap();
    assert_eq!(app.mode, AppMode::Session);
    assert!(app.snippet_picker.is_none());
    // Injection into a live `cat` session must not have raised an error notice.
    assert!(
        app.host_notice.is_none(),
        "unexpected notice: {:?}",
        app.host_notice
    );
}

#[test]
fn tab_inserts_snippet_without_running() {
    let mut app = app_with_session_and_snippets();
    app.handle_key(ctrl('n')).unwrap();

    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()))
        .unwrap();
    assert_eq!(app.mode, AppMode::Session);
    assert!(app.snippet_picker.is_none());
    assert!(app.host_notice.is_none());
}

#[test]
fn esc_cancels_picker_back_to_session() {
    let mut app = app_with_session_and_snippets();
    app.handle_key(ctrl('n')).unwrap();
    assert_eq!(app.mode, AppMode::SnippetPicker);

    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()))
        .unwrap();
    assert_eq!(app.mode, AppMode::Session);
    assert!(app.snippet_picker.is_none());
}

#[test]
fn injecting_with_no_session_reports_a_notice() {
    // Open the picker with a session return mode but no live session; running a
    // snippet then has nowhere to go and must surface a notice, not panic.
    let mut app = test_app(vec![]);
    app.store()
        .create_snippet(&NewSnippet {
            name: "noop".into(),
            command: ":".into(),
            description: None,
            tags: vec![],
        })
        .unwrap();
    app.open_snippet_picker(AppMode::Session).unwrap();
    assert_eq!(app.mode, AppMode::SnippetPicker);

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
        .unwrap();
    // Failure surfaces as a modal Notice (visible over the session view),
    // not a dashboard-only host_notice.
    assert_eq!(app.mode, AppMode::Notice);
    assert!(app.notice_popup.is_some());
}

/// The Running-phase gate (the round-one security fix): Ctrl+N must not open the
/// picker while the session is still connecting, so Enter can't type a snippet
/// into ssh's password or host-key prompt.
#[test]
fn ctrl_n_is_inert_over_a_connecting_session() {
    let mut app = test_app(vec![("edge", host("edge"))]);
    app.terminal_area = ratatui::layout::Rect::new(0, 0, 80, 24);
    app.store()
        .create_snippet(&NewSnippet {
            name: "noop".into(),
            command: ":".into(),
            description: None,
            tags: vec![],
        })
        .unwrap();
    let config = crate::session::SessionConfig {
        hops: Vec::new(),
        argv: vec!["cat".into()],
        display_name: "edge".into(),
        meta: crate::session::SessionMeta::default(),
        pending_secret: None,
        key_push_identity: None,
        host_name: "edge".into(),
    };
    // Session::spawn leaves the phase in `Connecting` until output reveals it.
    let session = crate::session::Session::spawn(config, (24, 80), None).unwrap();
    assert!(matches!(
        session.phase,
        crate::session::SessionPhase::Connecting { .. }
    ));
    app.sessions.push(session);
    app.active_session = Some(0);
    app.mode = AppMode::Connecting;

    app.handle_key(ctrl('n')).unwrap();
    assert_ne!(app.mode, AppMode::SnippetPicker);
    assert!(app.snippet_picker.is_none());
}

/// Delete removes a character, so it must mark the form dirty: an Esc after it
/// then routes through the discard prompt instead of dropping the edit silently.
#[test]
fn delete_marks_the_form_dirty() {
    let mut app = test_app(vec![]);
    app.handle_key(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT))
        .unwrap();
    app.handle_key(ch('a')).unwrap();
    assert_eq!(app.mode, AppMode::SnippetForm);
    for c in "abc".chars() {
        app.handle_key(ch(c)).unwrap();
    }
    // Move to the start and press Delete to remove a character.
    app.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::empty()))
        .unwrap();
    app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::empty()))
        .unwrap();
    assert!(app.snippet_form.as_ref().unwrap().dirty);
    // Esc now prompts to discard rather than dropping the edit.
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()))
        .unwrap();
    assert_eq!(app.mode, AppMode::ConfirmDiscard);
}

/// A bracketed paste into the snippet form lands in the focused field and marks
/// it dirty (the two new modes were missing from the paste whitelist).
#[test]
fn paste_reaches_the_snippet_form() {
    let mut app = test_app(vec![]);
    app.handle_key(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT))
        .unwrap();
    app.handle_key(ch('a')).unwrap();
    assert_eq!(app.mode, AppMode::SnippetForm);
    app.handle_paste("echo hi").unwrap();
    let form = app.snippet_form.as_ref().unwrap();
    assert_eq!(form.name, "echo hi");
    assert!(form.dirty);
}

#[test]
fn snippet_manager_slash_filter_types_letters_and_opens_the_match() {
    let mut app = test_app(vec![]);
    for (name, command) in [
        ("disk usage", "df -h"),
        ("restart nginx", "sudo nginx -s reload"),
    ] {
        app.store()
            .create_snippet(&NewSnippet {
                name: name.into(),
                command: command.into(),
                description: None,
                tags: vec![],
            })
            .unwrap();
    }
    app.enter_snippet_manage().unwrap();

    app.handle_key(ch('/')).unwrap();
    // `e`, `a` and `d` are manager binds; while filtering they are text.
    for c in "nginx".chars() {
        app.handle_key(ch(c)).unwrap();
    }
    assert_eq!(app.mode, AppMode::SnippetManage);
    let view = app.snippet_manage_view();
    assert_eq!(view.len(), 1);
    assert_eq!(app.snippets[view[0]].name, "restart nginx");

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
        .unwrap(); // keep the filter
    assert!(!app.snippet_manage_filtering);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
        .unwrap(); // edit the match
    assert_eq!(app.mode, AppMode::SnippetForm);
    assert_eq!(app.snippet_form.as_ref().unwrap().name, "restart nginx");
}

#[test]
fn snippet_without_a_name_is_named_after_its_command() {
    let mut app = test_app(vec![]);
    app.enter_snippet_manage().unwrap();
    app.handle_key(ch('a')).unwrap();
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::empty()))
        .unwrap(); // Name -> Command
    for c in "uptime && df -h /srv/data/backups".chars() {
        app.handle_key(ch(c)).unwrap();
    }
    app.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::empty()))
        .unwrap();
    assert_eq!(app.mode, AppMode::SnippetManage);
    assert_eq!(app.snippets.len(), 1);
    assert_eq!(app.snippets[0].name, "uptime && df -h /srv/data/back");
}
