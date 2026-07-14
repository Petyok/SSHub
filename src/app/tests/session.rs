use super::*;

#[test]
pub(crate) fn enter_starts_embedded_session() {
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
    app.terminal_area = ratatui::layout::Rect::new(0, 0, 80, 24);

    let config = crate::session::SessionConfig {
        argv: vec!["true".into()],
        display_name: "edge".into(),
        meta: crate::session::SessionMeta::default(),
        pending_secret: None,
        key_push_identity: None,
        host_name: "edge".into(),
    };
    let session = crate::session::Session::spawn(config, 24, 80).unwrap();
    app.sessions.push(session);
    app.active_session = Some(0);
    app.mode = AppMode::Connecting;

    assert_eq!(app.sessions.len(), 1);
    assert_eq!(app.mode, AppMode::Connecting);

    // Ctrl+D detaches; session keeps running.
    app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.sessions.len(), 1);
    assert_eq!(app.active_session, Some(0));
    assert_eq!(app.mode, AppMode::Normal);

    // Ctrl+W closes the tab.
    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.sessions.is_empty());
    assert!(app.active_session.is_none());
    assert_eq!(app.mode, AppMode::Normal);
}

#[test]
pub(crate) fn ctrl_t_opens_host_picker() {
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
        key_push_identity: None,
        host_name: "edge".into(),
    };
    app.sessions
        .push(crate::session::Session::spawn(cfg, 24, 80).unwrap());
    app.active_session = Some(0);
    app.mode = AppMode::Session;

    app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.mode, AppMode::SessionHostPicker);
    assert_eq!(app.sessions.len(), 1);

    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()))
        .unwrap();
    assert_eq!(app.mode, AppMode::Session);
    assert_eq!(app.sessions.len(), 1);
}

#[test]
pub(crate) fn session_tabs_switch_detach_and_focus() {
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
        key_push_identity: None,
        host_name: "edge".into(),
    };
    for _ in 0..3 {
        app.sessions
            .push(crate::session::Session::spawn(cfg.clone(), 24, 80).unwrap());
    }
    app.active_session = Some(2);
    app.mode = AppMode::Session;

    app.handle_key(KeyEvent::new(KeyCode::Char('['), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.active_session, Some(1));

    app.handle_key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.active_session, Some(2));

    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.sessions.len(), 2);
    assert_eq!(app.active_session, Some(1));

    app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.mode, AppMode::Normal);

    app.handle_key(KeyEvent::new(
        KeyCode::Char('s'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ))
    .unwrap();
    assert!(matches!(app.mode, AppMode::Session | AppMode::Connecting));
}

#[test]
pub(crate) fn shutdown_all_kills_detached_sessions() {
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
        argv: vec!["sleep".into(), "30".into()],
        display_name: "edge".into(),
        meta: crate::session::SessionMeta::default(),
        pending_secret: None,
        key_push_identity: None,
        host_name: "edge".into(),
    };
    app.sessions
        .push(crate::session::Session::spawn(cfg, 24, 80).unwrap());
    app.active_session = Some(0);
    app.mode = AppMode::Session;

    app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.mode, AppMode::Normal);
    assert_eq!(app.sessions.len(), 1);

    app.shutdown_all();
    assert!(app.sessions.is_empty());
    assert!(app.active_session.is_none());
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
