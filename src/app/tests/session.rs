use super::*;

#[test]
pub(crate) fn enter_starts_embedded_session() {
    let metadata: Arc<dyn MetadataStore> = Arc::new(MetadataDb::default());
    let resolver = MockResolver::new(vec![("edge", host("edge"))]);
    let mut app = App::new_with_deps(
        AppConfig::default(),
        AppDeps {
            resolver: Box::new(resolver),
            metadata: Arc::clone(&metadata),
            store: test_store(),
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
    };
    let session = crate::session::Session::spawn(config, 24, 80, None).unwrap();
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
    let resolver = MockResolver::new(vec![("edge", host("edge"))]);
    let mut app = App::new_with_deps(
        AppConfig::default(),
        AppDeps {
            resolver: Box::new(resolver),
            metadata,
            store: test_store(),
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
        .push(crate::session::Session::spawn(cfg, 24, 80, None).unwrap());
    app.active_session = Some(0);
    app.mode = AppMode::Session;

    app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.mode, AppMode::SessionPicker);
    assert_eq!(app.sessions.len(), 1);

    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()))
        .unwrap();
    assert_eq!(app.mode, AppMode::Session);
    assert_eq!(app.sessions.len(), 1);
}

#[test]
pub(crate) fn session_tabs_switch_detach_and_focus() {
    let metadata: Arc<dyn MetadataStore> = Arc::new(MetadataDb::default());
    let resolver = MockResolver::new(vec![("edge", host("edge"))]);
    let mut app = App::new_with_deps(
        AppConfig::default(),
        AppDeps {
            resolver: Box::new(resolver),
            metadata,
            store: test_store(),
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
    for _ in 0..3 {
        app.sessions
            .push(crate::session::Session::spawn(cfg.clone(), 24, 80, None).unwrap());
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
    let resolver = MockResolver::new(vec![("edge", host("edge"))]);
    let mut app = App::new_with_deps(
        AppConfig::default(),
        AppDeps {
            resolver: Box::new(resolver),
            metadata,
            store: test_store(),
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
    };
    app.sessions
        .push(crate::session::Session::spawn(cfg, 24, 80, None).unwrap());
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

#[test]
pub(crate) fn sftp_left_pane_picker_filters_and_dispatches() {
    let metadata: Arc<dyn MetadataStore> = Arc::new(MetadataDb::default());
    let resolver = MockResolver::new(vec![("alpha", host("alpha")), ("bravo", host("bravo"))]);
    let mut app = App::new_with_deps(
        AppConfig::default(),
        AppDeps {
            resolver: Box::new(resolver),
            metadata,
            store: test_store(),
            password_store: Box::new(crate::credentials::NoopPasswordStore),
        },
    );
    app.reload_hosts().unwrap();
    app.terminal_area = ratatui::layout::Rect::new(0, 0, 80, 24);
    app.mode = AppMode::Normal;

    app.open_session_picker(crate::app::SessionPickerPurpose::SftpLeftPane);
    assert_eq!(app.mode, AppMode::SessionPicker);
    assert_eq!(
        app.session_picker.as_ref().unwrap().purpose,
        crate::app::SessionPickerPurpose::SftpLeftPane
    );
    assert_eq!(app.session_picker_host_matches().len(), 2);

    for c in "bra".chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty()))
            .unwrap();
    }
    let matches = app.session_picker_host_matches();
    assert_eq!(matches.len(), 1);
    assert!(matches[0].1.contains("bravo"));

    // Esc returns to the dashboard without dispatching.
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()))
        .unwrap();
    assert_eq!(app.mode, AppMode::Normal);
    assert!(app.session_picker.is_none());

    // Reopen and press Enter: the pick must reach sftp_connect_left_pane. With
    // no SFTP browser connected that call parks a known notice, which proves
    // the dispatch without needing a network.
    app.host_notice = None;
    app.open_session_picker(crate::app::SessionPickerPurpose::SftpLeftPane);
    for c in "bra".chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty()))
            .unwrap();
    }
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
        .unwrap();
    assert_eq!(
        app.host_notice.as_deref(),
        Some("connect the SFTP browser first")
    );
    assert!(app.session_picker.is_none());
}
