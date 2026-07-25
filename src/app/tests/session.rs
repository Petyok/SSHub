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
    app.sessions[0].phase = crate::session::SessionPhase::Running {
        started_at: std::time::Instant::now(),
    };
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
    app.active_tab = 1;
    app.sftp = Some(crate::sftp::model::SftpState::new("/srv", "/home/me"));

    app.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::empty()))
        .unwrap();
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
    app.sftp = None;
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

/// App with `names.len()` spawned sessions, ready for picker tests.
fn app_with_sessions(names: &[&str]) -> App {
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
    for name in names {
        let cfg = crate::session::SessionConfig {
            argv: vec!["true".into()],
            display_name: (*name).into(),
            meta: crate::session::SessionMeta::default(),
            pending_secret: None,
        };
        app.sessions
            .push(crate::session::Session::spawn(cfg, 24, 80, None).unwrap());
    }
    app.active_session = if names.is_empty() { None } else { Some(0) };
    app.mode = AppMode::Normal;
    app
}

#[test]
pub(crate) fn session_switcher_open_guard_matrix() {
    use crate::app::SessionPickerPurpose::SwitchSession;

    const ORIGINS: [AppMode; 3] = [AppMode::Normal, AppMode::Session, AppMode::Connecting];

    // No sessions: refuses to open, whatever the origin.
    for origin in ORIGINS {
        let mut app = app_with_sessions(&[]);
        app.mode = origin;
        app.open_session_picker(SwitchSession);
        assert!(app.session_picker.is_none(), "origin {origin:?}");
        assert_eq!(app.mode, origin, "origin {origin:?}");
    }

    // One session: opens from every origin and pre-selects it.
    for origin in ORIGINS {
        let mut app = app_with_sessions(&["only"]);
        app.mode = origin;
        app.open_session_picker(SwitchSession);
        assert_eq!(app.mode, AppMode::SessionPicker, "origin {origin:?}");
        let p = app.session_picker.as_ref().unwrap();
        assert_eq!(p.purpose, SwitchSession, "origin {origin:?}");
        assert_eq!(p.return_mode, origin, "origin {origin:?}");
        assert_eq!(p.selected, 0, "origin {origin:?}");
        assert_eq!(app.session_picker_rows().len(), 1);
    }

    // Several sessions: from every origin the active one is pre-selected and
    // marked current, and the ordinals count from one.
    for origin in ORIGINS {
        let mut app = app_with_sessions(&["a", "b", "c"]);
        app.active_session = Some(2);
        app.mode = origin;
        app.open_session_picker(SwitchSession);
        let p = app.session_picker.as_ref().unwrap();
        assert_eq!(p.purpose, SwitchSession, "origin {origin:?}");
        assert_eq!(p.return_mode, origin, "origin {origin:?}");
        assert_eq!(p.selected, 2, "origin {origin:?}");
        let rows = app.session_picker_rows();
        assert_eq!(rows.iter().filter(|r| r.current).count(), 1);
        assert!(rows[2].current);
        assert_eq!(rows[0].ordinal, Some(1));
        assert_eq!(rows[2].ordinal, Some(3));
    }
}

#[test]
pub(crate) fn session_switcher_matching_and_navigation() {
    use crate::app::SessionPickerPurpose::SwitchSession;

    let mut app = app_with_sessions(&["web-PROD", "dev-box", "db"]);
    app.sessions[1].meta.user = Some("Deploy".into());
    app.sessions[2].meta.address = Some("10.0.0.42".into());
    app.open_session_picker(SwitchSession);

    // Name, user and address all match, case-insensitively.
    for (query, expected) in [("prod", 0usize), ("deploy", 1), ("0.0.42", 2)] {
        if let Some(p) = app.session_picker.as_mut() {
            p.query = query.into();
            p.selected = 0;
        }
        let rows = app.session_picker_rows();
        assert_eq!(rows.len(), 1, "query {query}");
        assert_eq!(rows[0].index, expected, "query {query}");
    }

    // Identical names remain separate choices because their source indices and
    // displayed tab ordinals are different.
    let mut duplicates = app_with_sessions(&["same", "same"]);
    duplicates.open_session_picker(SwitchSession);
    let rows = duplicates.session_picker_rows();
    assert_eq!(rows.iter().map(|r| r.index).collect::<Vec<_>>(), vec![0, 1]);
    assert_eq!(
        rows.iter().map(|r| r.ordinal).collect::<Vec<_>>(),
        vec![Some(1), Some(2)]
    );

    // Typing resets the selection.
    if let Some(p) = app.session_picker.as_mut() {
        p.query.clear();
        p.selected = 2;
    }
    app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::empty()))
        .unwrap();
    assert_eq!(app.session_picker.as_ref().unwrap().selected, 0);

    // Up/Down wrap around, and an empty list neither panics nor moves.
    if let Some(p) = app.session_picker.as_mut() {
        p.query.clear();
        p.selected = 0;
    }
    app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::empty()))
        .unwrap();
    assert_eq!(app.session_picker.as_ref().unwrap().selected, 2);
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::empty()))
        .unwrap();
    assert_eq!(app.session_picker.as_ref().unwrap().selected, 0);

    if let Some(p) = app.session_picker.as_mut() {
        p.query = "zzzz".into();
    }
    assert!(app.session_picker_rows().is_empty());
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::empty()))
        .unwrap();
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
        .unwrap();
    assert_eq!(
        app.mode,
        AppMode::SessionPicker,
        "enter without a match is a no-op"
    );
    assert!(app.session_picker.is_some());
}

#[test]
pub(crate) fn session_switcher_enter_derives_mode_from_target_phase() {
    use crate::app::SessionPickerPurpose::SwitchSession;
    use crate::session::SessionPhase;
    use std::time::Instant;

    let mut app = app_with_sessions(&["one", "two", "three"]);
    let cases = [
        (
            0usize,
            SessionPhase::Connecting {
                started_at: Instant::now(),
            },
            AppMode::Connecting,
        ),
        (
            1,
            SessionPhase::Running {
                started_at: Instant::now(),
            },
            AppMode::Session,
        ),
        (
            2,
            SessionPhase::Exited {
                status: "exit 0".into(),
                at: Instant::now(),
            },
            AppMode::Session,
        ),
    ];

    for (target, phase, expected) in cases {
        app.sessions[target].phase = phase;
        app.active_session = Some(0);
        app.mode = AppMode::Normal;
        app.open_session_picker(SwitchSession);
        if let Some(p) = app.session_picker.as_mut() {
            p.selected = target;
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
            .unwrap();
        assert_eq!(app.active_session, Some(target), "target {target}");
        assert_eq!(app.mode, expected, "target {target}");
        assert!(app.session_picker.is_none());
    }
}

#[test]
pub(crate) fn session_switcher_phase_change_while_open() {
    use crate::app::SessionPickerPurpose::SwitchSession;
    use crate::session::SessionPhase;
    use std::time::Instant;

    let mut app = app_with_sessions(&["aaa", "bbb"]);
    app.sessions[0].phase = SessionPhase::Running {
        started_at: Instant::now(),
    };
    app.active_session = Some(0);
    app.mode = AppMode::Session;
    app.open_session_picker(SwitchSession);
    assert_eq!(
        app.session_picker.as_ref().unwrap().return_mode,
        AppMode::Session
    );

    // A session dying under the open overlay changes its badge but must not
    // renumber the list, or the cursor would silently point somewhere else.
    let before: Vec<usize> = app.session_picker_rows().iter().map(|r| r.index).collect();
    app.sessions[1].phase = SessionPhase::Exited {
        status: "exit 1".into(),
        at: Instant::now(),
    };
    let after = app.session_picker_rows();
    assert_eq!(
        before,
        after.iter().map(|r| r.index).collect::<Vec<_>>(),
        "a dying session must not renumber the list under the cursor"
    );
    assert_eq!(after[1].badge, Some(crate::app::PickerBadge::Exited));

    // Now move the *active* session backwards to Connecting. The stored
    // return_mode is still Session, so an implementation that restores it
    // verbatim would land on Session — only re-deriving from the current phase
    // yields Connecting. Changing an inactive session here would make this test
    // pass either way, which is exactly the trap to avoid.
    app.sessions[0].phase = SessionPhase::Connecting {
        started_at: Instant::now(),
    };
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()))
        .unwrap();
    assert_eq!(
        app.mode,
        AppMode::Connecting,
        "escape must re-derive the mode from the active session's current phase"
    );
}

#[test]
pub(crate) fn session_picker_opener_refuses_to_replace_an_open_picker() {
    use crate::app::SessionPickerPurpose::{NewSession, SwitchSession};

    let mut app = app_with_sessions(&["one", "two"]);

    // The opener's mode guard already refuses while a picker is up, in both
    // directions. The keyboard-level counterpart lives in task 4, because it
    // needs the hotkey to exist.
    app.open_session_picker(SwitchSession);
    app.open_session_picker(NewSession);
    assert_eq!(app.session_picker.as_ref().unwrap().purpose, SwitchSession);

    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()))
        .unwrap();
    app.open_session_picker(NewSession);
    app.open_session_picker(SwitchSession);
    assert_eq!(app.session_picker.as_ref().unwrap().purpose, NewSession);
}

#[test]
pub(crate) fn session_picker_paste_lands_in_the_query() {
    use crate::app::SessionPickerPurpose::SwitchSession;

    let mut app = app_with_sessions(&["dev-box", "web-prod"]);
    app.open_session_picker(SwitchSession);
    app.handle_paste("dev").unwrap();
    assert_eq!(app.session_picker.as_ref().unwrap().query, "dev");
    let rows = app.session_picker_rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].index, 0);
}

#[test]
pub(crate) fn session_switcher_hotkey_from_dashboard_and_session() {
    use crate::app::SessionPickerPurpose::{NewSession, SwitchSession};

    let alt_s = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::ALT);

    // Without sessions the hotkey does nothing at all.
    let mut app = app_with_sessions(&[]);
    app.handle_key(alt_s).unwrap();
    assert_eq!(app.mode, AppMode::Normal);

    let mut app = app_with_sessions(&["edge", "other"]);

    // From the dashboard.
    app.handle_key(alt_s).unwrap();
    assert_eq!(app.mode, AppMode::SessionPicker);
    assert_eq!(app.session_picker.as_ref().unwrap().purpose, SwitchSession);
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()))
        .unwrap();

    // From inside a live session — the 's' must not reach the PTY.
    app.mode = AppMode::Session;
    app.handle_key(alt_s).unwrap();
    assert_eq!(app.mode, AppMode::SessionPicker);
    assert_eq!(app.session_picker.as_ref().unwrap().purpose, SwitchSession);

    // Ctrl+T inside the switcher is swallowed as ordinary modal input.
    app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.session_picker.as_ref().unwrap().purpose, SwitchSession);
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()))
        .unwrap();

    // And the other direction, through the real key dispatch rather than the
    // opener: Alt+S inside the new-tab picker must leave it untouched.
    app.mode = AppMode::Normal;
    app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.session_picker.as_ref().unwrap().purpose, NewSession);
    app.handle_key(alt_s).unwrap();
    assert_eq!(
        app.session_picker.as_ref().unwrap().purpose,
        NewSession,
        "Alt+S must not hijack an open new-tab picker"
    );
    assert_eq!(app.mode, AppMode::SessionPicker);
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()))
        .unwrap();

    // A rebind takes effect and the old default stops working.
    app.config
        .keybinds
        .set(KeyAction::SessionSwitcher, vec!["F7".into()]);
    app.mode = AppMode::Normal;
    app.handle_key(alt_s).unwrap();
    assert_ne!(app.mode, AppMode::SessionPicker);
    app.handle_key(KeyEvent::new(KeyCode::F(7), KeyModifiers::empty()))
        .unwrap();
    assert_eq!(app.mode, AppMode::SessionPicker);
}

#[test]
pub(crate) fn session_switcher_hotkey_works_from_every_dashboard_tab() {
    use crate::app::SessionPickerPurpose::SwitchSession;

    let alt_s = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::ALT);

    for active_tab in 0..=4 {
        let mut app = app_with_sessions(&["edge", "other"]);
        app.active_tab = active_tab;
        app.handle_key(alt_s).unwrap();

        assert_eq!(
            app.mode,
            AppMode::SessionPicker,
            "dashboard tab {active_tab}"
        );
        assert_eq!(
            app.session_picker.as_ref().unwrap().purpose,
            SwitchSession,
            "dashboard tab {active_tab}"
        );
    }
}
