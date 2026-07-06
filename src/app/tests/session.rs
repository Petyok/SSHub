use super::*;

#[test]
pub(crate) fn enter_starts_embedded_session() {
    // Pressing Enter no longer shells out to an external terminal; it
    // spawns a PTY in-process and flips into Connecting mode. We use
    // /bin/true as the program so the child exits immediately — the
    // session itself stays in App until Drop tears it down.
    let metadata: Arc<dyn MetadataStore> = Arc::new(MetadataDb::default());
    let (launcher, _launched) = RecordingLauncher::new();
    let resolver = MockResolver::new(vec![("edge", host("edge"))]);
    let mut app = App::new_with_deps(
        AppConfig::default(),
        AppDeps {
            resolver: Box::new(resolver),
            metadata: Arc::clone(&metadata),
            store: test_store(),
            launcher: Box::new(launcher),
            password_store: Box::new(crate::credentials::NoopPasswordStore),
        },
    );
    app.reload_hosts().unwrap();
    // Pretend the terminal is 80x24 so the session has a sensible PTY size.
    app.terminal_area = ratatui::layout::Rect::new(0, 0, 80, 24);

    // Directly inject the session args we want (avoid spawning real ssh in
    // a unit test). This mirrors what connect_selected does after building
    // ssh_argv.
    let config = crate::session::SessionConfig {
        argv: vec!["true".into()],
        display_name: "edge".into(),
        meta: crate::session::SessionMeta::default(),
        pending_secret: None,
    };
    let session = crate::session::Session::spawn(config, 24, 80).unwrap();
    app.sessions.push(session);
    app.active_session = Some(0);
    app.mode = AppMode::Connecting;

    // Sanity: app has one tab.
    assert_eq!(app.sessions.len(), 1);
    assert_eq!(app.mode, AppMode::Connecting);

    // Ctrl+D closes the last tab and returns to Normal.
    app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.sessions.is_empty());
    assert!(app.active_session.is_none());
    assert_eq!(app.mode, AppMode::Normal);
}

#[test]
pub(crate) fn ctrl_t_duplicates_and_ctrl_w_closes_tab() {
    // Three tabs, all running `true`. Verify Ctrl+T appends, Ctrl+PgUp/Dn
    // cycle, Ctrl+W removes the active tab.
    let metadata: Arc<dyn MetadataStore> = Arc::new(MetadataDb::default());
    let (launcher, _launched) = RecordingLauncher::new();
    let resolver = MockResolver::new(vec![("edge", host("edge"))]);
    let mut app = App::new_with_deps(
        AppConfig::default(),
        AppDeps {
            resolver: Box::new(resolver),
            metadata,
            store: test_store(),
            launcher: Box::new(launcher),
            password_store: Box::new(crate::credentials::NoopPasswordStore),
        },
    );
    app.reload_hosts().unwrap();
    app.terminal_area = ratatui::layout::Rect::new(0, 0, 80, 24);

    let cfg = crate::session::SessionConfig {
        argv: vec!["true".into()],
        display_name: "edge".into(),
        meta: crate::session::SessionMeta::default(),
        pending_secret: None,
    };
    app.sessions
        .push(crate::session::Session::spawn(cfg, 24, 80).unwrap());
    app.active_session = Some(0);
    app.mode = AppMode::Connecting;

    // Ctrl+T: duplicate to a second tab.
    app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.sessions.len(), 2);
    assert_eq!(app.active_session, Some(1));

    // Ctrl+T again: third tab.
    app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.sessions.len(), 3);
    assert_eq!(app.active_session, Some(2));

    // Ctrl+PageUp: cycle backward to tab 1.
    app.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.active_session, Some(1));

    // Ctrl+PageDown: cycle forward to tab 2.
    app.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.active_session, Some(2));

    // Ctrl+W: close active (last tab); should stay at the new last.
    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.sessions.len(), 2);
    assert_eq!(app.active_session, Some(1));

    // Ctrl+W twice more: empty + return to dashboard.
    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL))
        .unwrap();
    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.sessions.is_empty());
    assert_eq!(app.mode, AppMode::Normal);
}

#[test]
pub(crate) fn tab_toggles_detail_focus() {
    let mut app = test_app(vec![("web", host("web"))]);
    assert!(!app.detail_focus);
    app.handle_key(key(KeyCode::Tab)).unwrap();
    assert!(app.detail_focus);
    app.handle_key(key(KeyCode::Tab)).unwrap();
    assert!(!app.detail_focus);
}
