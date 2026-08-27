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
        argv: vec!["cat".into()],
        display_name: "edge".into(),
        meta: crate::session::SessionMeta::default(),
        pending_secret: None,
        key_push_identity: None,
        host_name: "edge".into(),
    };
    let mut session = crate::session::Session::spawn(config, 24, 80, None).unwrap();
    // The picker only opens over a *running* session; a bare `cat` never emits
    // output to trip the reveal, so mark it running explicitly.
    session.phase = crate::session::SessionPhase::Running {
        started_at: std::time::Instant::now(),
    };
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
