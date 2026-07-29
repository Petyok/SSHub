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
        key_push_identity: None,
        host_name: "edge".into(),
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
        key_push_identity: None,
        host_name: "edge".into(),
    };
    app.sessions
        .push(crate::session::Session::spawn(cfg, 24, 80, None).unwrap());
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
        key_push_identity: None,
        host_name: "edge".into(),
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
        key_push_identity: None,
        host_name: "edge".into(),
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

fn app_with_background_session(hosts: Vec<(&str, SshHost)>) -> App {
    let metadata: Arc<dyn MetadataStore> = Arc::new(MetadataDb::default());
    let resolver = MockResolver::new(hosts);
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

    let name = app.hosts[0].name().to_string();
    let cfg = crate::session::SessionConfig {
        argv: vec!["true".into()],
        display_name: name.clone(),
        meta: crate::session::SessionMeta::default(),
        pending_secret: None,
        key_push_identity: None,
        host_name: name,
    };
    app.sessions
        .push(crate::session::Session::spawn(cfg, 24, 80, None).unwrap());
    app.active_session = Some(0);
    app.mode = AppMode::Normal;
    app
}

#[test]
pub(crate) fn session_strip_binds_work_on_every_dashboard_tab() {
    // Footer advertises resume / new tab on every tab once a session exists;
    // those used to only fire on hosts (active_tab == 0).
    for tab in 0..=4u8 {
        let mut app = app_with_background_session(vec![("edge", host("edge"))]);
        app.active_tab = tab as usize;

        app.handle_key(KeyEvent::new(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ))
        .unwrap();
        assert!(
            matches!(app.mode, AppMode::Session | AppMode::Connecting),
            "tab {tab}: resume (Ctrl+Shift+S) must focus the session"
        );

        app.mode = AppMode::Normal;
        app.active_tab = tab as usize;
        app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL))
            .unwrap();
        assert_eq!(
            app.mode,
            AppMode::SessionHostPicker,
            "tab {tab}: Ctrl+T must open the new-session host picker"
        );
        assert_eq!(
            app.session_host_picker.as_ref().map(|p| p.target),
            Some(PickerTarget::NewSession),
            "tab {tab}: Ctrl+T target must be NewSession, not SftpLeftPane"
        );
    }
}

#[test]
pub(crate) fn session_strip_cycle_and_sftp_from_non_hosts_tab() {
    let mut app = app_with_background_session(vec![("edge", host("edge"))]);
    // Second session so tab cycling is observable.
    let cfg = crate::session::SessionConfig {
        argv: vec!["true".into()],
        display_name: "edge".into(),
        meta: crate::session::SessionMeta::default(),
        pending_secret: None,
        key_push_identity: None,
        host_name: "edge".into(),
    };
    app.sessions
        .push(crate::session::Session::spawn(cfg, 24, 80, None).unwrap());
    app.active_session = Some(0);
    app.active_tab = 2; // tunnels

    app.handle_key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.active_session, Some(1));
    assert_eq!(app.mode, AppMode::Normal);
    assert_eq!(app.active_tab, 2);

    app.handle_key(KeyEvent::new(
        KeyCode::Char('f'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ))
    .unwrap();
    assert_eq!(
        app.active_tab, 1,
        "Ctrl+Shift+F must switch to the SFTP tab"
    );
    assert!(
        app.sftp.is_some(),
        "Ctrl+Shift+F must open SFTP for the session host"
    );
    assert_eq!(app.mode, AppMode::Normal);
}

#[test]
pub(crate) fn sftp_o_still_opens_left_pane_picker_with_background_session() {
    // Ctrl+T is the new-session strip bind; plain `o` must keep opening the
    // SFTP left-pane host picker when a browser is live.
    let mut app = app_with_background_session(vec![("edge", host("edge"))]);
    app.active_tab = 1;
    app.sftp = Some(crate::sftp::model::SftpState::new("/srv", "/home/me"));

    app.handle_key(key_char('o')).unwrap();
    assert_eq!(app.mode, AppMode::SessionHostPicker);
    assert_eq!(
        app.session_host_picker.as_ref().map(|p| p.target),
        Some(PickerTarget::SftpLeftPane)
    );
}
