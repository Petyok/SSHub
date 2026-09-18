use super::*;

fn profile_app(names: &[&str]) -> (tempfile::TempDir, App, crate::profile::RootDirs) {
    let dir = tempfile::tempdir().unwrap();
    let roots = crate::profile::RootDirs {
        data_root: dir.path().join("data"),
        config_root: dir.path().join("config"),
        compat: false,
    };
    let mut state = crate::profile::ProfileState::default();
    for name in names {
        crate::profile::create_profile(&roots, &mut state, name).unwrap();
    }
    let active = state.profiles[0].clone();
    state.last_used = Some(active.id.clone());
    state.save(&roots.data_root).unwrap();
    let mut app = test_app(vec![]);
    app.profile = Some(crate::profile::profile_paths(
        &roots,
        &active,
        dir.path().join("ssh_config"),
    ));
    (dir, app, roots)
}

#[test]
fn settings_profile_action_creates_and_esc_returns_to_settings() {
    let (_dir, mut app, roots) = profile_app(&["default"]);
    app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL))
        .unwrap();
    let index = SETTINGS_ITEMS
        .iter()
        .position(|d| d.item == SettingItem::Profiles)
        .unwrap();
    for _ in 0..index {
        app.handle_key(key(KeyCode::Down)).unwrap();
    }
    app.handle_key(key_char(' ')).unwrap();
    assert_eq!(app.mode, AppMode::Settings);
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.mode, AppMode::ProfilePicker);
    for c in "work".chars() {
        app.handle_key(key_char(c)).unwrap();
    }
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.profile_count, 2);
    assert_eq!(app.profile_action_label(), "Manage profiles");
    assert!(crate::profile::ProfileState::load(&roots.data_root)
        .unwrap()
        .unwrap()
        .by_name("work")
        .is_some());
    assert!(app.pending_profile.is_none());
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.mode, AppMode::Settings);
    assert_eq!(app.settings_selected, index);
}

#[test]
fn initial_create_escape_returns_directly_to_dashboard() {
    let (_dir, mut app, _) = profile_app(&["default"]);
    app.open_profile_manager();
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.mode, AppMode::Normal);
    assert!(app.profile_picker.is_none());
    assert!(app.pending_profile.is_none());
}

#[test]
fn manager_queues_other_profile_without_mutating_active_or_last_used() {
    let (_dir, mut app, roots) = profile_app(&["default", "work"]);
    let active = app.profile.clone().unwrap();
    app.open_profile_manager();
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();
    let pending = app.pending_profile.as_ref().unwrap();
    assert_eq!(pending.name, "work");
    assert_eq!(pending.root, roots.data_root.join("profiles/work"));
    assert_eq!(app.profile.as_ref(), Some(&active));
    assert_eq!(app.mode, AppMode::ProfilePicker);
    assert_eq!(
        crate::profile::ProfileState::load(&roots.data_root)
            .unwrap()
            .unwrap()
            .last_used
            .as_deref(),
        Some(active.id.as_str())
    );
}

#[test]
fn selecting_current_profile_closes_without_rebuild_even_while_busy() {
    let (_dir, mut app, _) = profile_app(&["default", "work"]);
    let (tx, _rx) = std::sync::mpsc::channel();
    app.sftp_tx = Some(tx);
    app.open_profile_manager();
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert!(app.pending_profile.is_none());
    assert_eq!(app.mode, AppMode::Normal);
}

#[test]
fn connecting_sftp_and_waiting_tunnel_reject_profile_switch() {
    let (_dir, mut app, _) = profile_app(&["default", "work"]);
    let (tx, _rx) = std::sync::mpsc::channel();
    app.sftp_tx = Some(tx);
    app.open_profile_manager();
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert!(app.pending_profile.is_none());
    assert!(app.profile_switch_blocker().unwrap().contains("SFTP"));
    app.sftp_tx = None;
    app.tunnel_manager.on_auto_start_failed(
        9,
        "offline",
        &crate::config::TunnelReconnectConfig::default(),
    );
    assert!(app.tunnel_manager.is_reconnecting(9));
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert!(app.pending_profile.is_none());
    assert!(app.profile_switch_blocker().unwrap().contains("tunnels"));
    assert_eq!(app.mode, AppMode::ProfilePicker);
}

#[test]
fn exhausted_tunnel_reconnect_allows_profile_switch() {
    let (_dir, mut app, _) = profile_app(&["default", "work"]);
    let cfg = crate::config::TunnelReconnectConfig {
        max_attempts: 2,
        ..Default::default()
    };
    app.open_profile_manager();
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.tunnel_manager.on_auto_start_failed(9, "offline", &cfg);
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert!(app.pending_profile.is_none());
    assert!(app.profile_switch_blocker().unwrap().contains("tunnels"));

    app.tunnel_manager.on_auto_start_failed(9, "offline", &cfg);
    app.tunnel_manager.on_auto_start_failed(9, "offline", &cfg);
    assert!(app.tunnel_manager.is_gave_up(9));
    assert!(!app.tunnel_manager.has_child(9));
    assert!(app.profile_switch_blocker().is_none());
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.pending_profile.as_ref().unwrap().name, "work");
}

#[test]
fn profile_create_paste_inserts_at_cursor_without_submitting() {
    let (_dir, mut app, roots) = profile_app(&["default"]);
    app.open_profile_manager();
    for ch in "wrk".chars() {
        app.handle_key(key_char(ch)).unwrap();
    }
    app.handle_key(key(KeyCode::Left)).unwrap();
    app.handle_key(key(KeyCode::Left)).unwrap();
    app.handle_paste("o\n\r\t\0\u{7f}\u{85}").unwrap();
    assert_eq!(app.mode, AppMode::ProfilePicker);
    assert_eq!(app.profile_picker.as_ref().unwrap().profile_count(), 1);
    assert_eq!(
        crate::profile::ProfileState::load(&roots.data_root)
            .unwrap()
            .unwrap()
            .profiles
            .len(),
        1
    );
    assert!(app.pending_profile.is_none());

    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert!(crate::profile::ProfileState::load(&roots.data_root)
        .unwrap()
        .unwrap()
        .by_name("work")
        .is_some());
    assert_eq!(app.profile_count, 2);
}

#[test]
fn profile_rename_paste_waits_for_explicit_submit() {
    let (_dir, mut app, roots) = profile_app(&["default", "work"]);
    app.open_profile_manager();
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key_char('r')).unwrap();
    app.handle_key(key(KeyCode::Home)).unwrap();
    app.handle_paste("new-\r\n\t\0").unwrap();
    assert!(crate::profile::ProfileState::load(&roots.data_root)
        .unwrap()
        .unwrap()
        .by_name("work")
        .is_some());
    assert!(app.pending_profile.is_none());

    app.handle_key(key(KeyCode::Enter)).unwrap();
    let state = crate::profile::ProfileState::load(&roots.data_root)
        .unwrap()
        .unwrap();
    assert!(state.by_name("work").is_none());
    assert!(state.by_name("new-work").is_some());
    assert_eq!(app.active_profile_name(), "default");
}

#[test]
fn profile_list_and_delete_confirmation_ignore_pasted_commands() {
    let (_dir, mut app, roots) = profile_app(&["default", "work"]);
    app.open_profile_manager();
    app.handle_paste("jndy\r\n2\u{1b}").unwrap();
    assert!(app.pending_profile.is_none());
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key_char('d')).unwrap();
    app.handle_paste("yY\r\nnr\u{1b}").unwrap();
    assert!(crate::profile::ProfileState::load(&roots.data_root)
        .unwrap()
        .unwrap()
        .by_name("work")
        .is_some());
    assert!(app.pending_profile.is_none());
    assert_eq!(app.mode, AppMode::ProfilePicker);

    app.handle_key(key_char('y')).unwrap();
    assert!(crate::profile::ProfileState::load(&roots.data_root)
        .unwrap()
        .unwrap()
        .by_name("work")
        .is_none());
    assert_eq!(app.profile_count, 1);
}

#[test]
fn finished_unaudited_broadcast_blocks_profile_switch() {
    let (_dir, mut app, _) = profile_app(&["default", "work"]);
    let (_tx, rx) = std::sync::mpsc::channel();
    app.broadcast = Some(BroadcastState {
        target_label: "test".into(),
        command: "true".into(),
        results: vec![],
        rx,
        cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        concurrency: 1,
        phase: BroadcastPhase::Paused,
        anim: None,
        audit_written: false,
    });
    app.open_profile_manager();
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert!(app.pending_profile.is_none());
    assert!(app.profile_switch_blocker().unwrap().contains("broadcast"));
}

#[test]
fn compatibility_mode_explains_unavailable_management_without_state_writes() {
    let (dir, mut app, roots) = profile_app(&["default"]);
    app.profile = Some(crate::profile::compat_paths(
        &crate::profile::RootDirs {
            compat: true,
            ..roots
        },
        dir.path().join("ssh_config"),
    ));
    app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::ALT))
        .unwrap();
    assert_eq!(app.mode, AppMode::Normal);
    assert!(app.profile_picker.is_none());
    assert!(app.pending_profile.is_none());
    assert!(app
        .host_notice
        .as_deref()
        .unwrap()
        .contains("compatibility mode"));
}

#[test]
fn remapped_profile_action_roundtrips_and_opens_manager() {
    let (_dir, mut app, _) = profile_app(&["default"]);
    app.config
        .keybinds
        .set(KeyAction::ProfilesManage, vec!["F6".into()]);
    let encoded = toml::to_string(&app.config.keybinds).unwrap();
    app.config.keybinds = toml::from_str(&encoded).unwrap();
    assert_eq!(app.config.keybinds.primary(KeyAction::ProfilesManage), "F6");
    app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::ALT))
        .unwrap();
    assert_eq!(app.mode, AppMode::Normal);
    app.handle_key(key(KeyCode::F(6))).unwrap();
    assert_eq!(app.mode, AppMode::ProfilePicker);
    app.config.keybinds.reset_action(KeyAction::ProfilesManage);
    assert_eq!(
        app.config.keybinds.primary(KeyAction::ProfilesManage),
        "Alt+P"
    );
}

#[test]
fn pending_profile_load_failure_keeps_workspace_and_last_used() {
    let (_dir, mut app, roots) = profile_app(&["default", "work"]);
    let active = app.profile.clone().unwrap();
    app.search_query = "keep my search".into();
    let store = Arc::clone(&app.store);
    let config_path = active.root.join("watched_ssh_config");
    std::fs::write(&config_path, "").unwrap();
    app.set_watcher_rx(crate::watcher::spawn_config_watcher(&config_path).unwrap());
    app.open_profile_manager();
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();
    let target = app.pending_profile.clone().unwrap();
    std::fs::write(&target.config_file, "invalid = [").unwrap();

    assert!(!crate::apply_pending_profile(
        &mut app,
        crate::load_profile_app
    ));
    assert_eq!(app.profile.as_ref(), Some(&active));
    assert!(Arc::ptr_eq(&store, &app.store));
    assert_eq!(app.search_query, "keep my search");
    assert_eq!(app.mode, AppMode::ProfilePicker);
    assert!(app.profile_picker.is_some());
    assert!(app.pending_profile.is_none());
    assert!(
        app.watcher_rx.is_some(),
        "failed load must retain the old watcher"
    );
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
    terminal
        .draw(|frame| crate::tui::render(frame, &app))
        .unwrap();
    let rendered: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(rendered.contains("Cannot switch profiles"));
    assert_eq!(
        crate::profile::ProfileState::load(&roots.data_root)
            .unwrap()
            .unwrap()
            .last_used
            .as_deref(),
        Some(active.id.as_str())
    );
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.mode, AppMode::Normal);
}

#[test]
fn pending_profile_success_replaces_workspace_and_records_last_used() {
    let (_dir, mut app, roots) = profile_app(&["default", "work"]);
    app.search_query = "old search".into();
    app.terminal_area = ratatui::layout::Rect::new(0, 0, 120, 40);
    let old_store = Arc::clone(&app.store);
    app.open_profile_manager();
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();
    let target = app.pending_profile.clone().unwrap();

    assert!(crate::apply_pending_profile(&mut app, |paths| {
        assert_eq!(paths, &target);
        let mut candidate = test_app(vec![("new-work-host", host("new-work-host"))]);
        candidate.profile = Some(paths.clone());
        Ok(candidate)
    }));
    assert_eq!(app.profile.as_ref(), Some(&target));
    assert!(!Arc::ptr_eq(&old_store, &app.store));
    assert!(app.search_query.is_empty());
    assert_eq!(app.hosts[0].name(), "new-work-host");
    assert_eq!(app.terminal_area.width, 120);
    assert!(app.profile_picker.is_none());
    assert_eq!(
        crate::profile::ProfileState::load(&roots.data_root)
            .unwrap()
            .unwrap()
            .last_used
            .as_deref(),
        Some(target.id.as_str())
    );
}

#[test]
fn pending_profile_rechecks_resources_before_loading() {
    let (_dir, mut app, _) = profile_app(&["default", "work"]);
    let active = app.profile.clone();
    app.open_profile_manager();
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();
    let (tx, _rx) = std::sync::mpsc::channel();
    app.sftp_tx2 = Some(tx);
    assert!(!crate::apply_pending_profile(&mut app, |_| panic!(
        "busy App must not load another profile"
    )));
    assert_eq!(app.profile, active);
    assert!(app.sftp_tx2.is_some());
    assert!(app.pending_profile.is_none());
    assert_eq!(app.mode, AppMode::ProfilePicker);
}

#[test]
fn detached_local_session_blocks_switch_until_closed() {
    let (_dir, mut app, _) = profile_app(&["default", "work"]);
    let session = crate::session::Session::spawn(
        crate::session::SessionConfig {
            argv: vec!["sleep".into(), "30".into()],
            display_name: "local".into(),
            meta: crate::session::SessionMeta::default(),
            pending_secret: None,
            key_push_identity: None,
            host_name: "local".into(),
        },
        24,
        80,
        None,
    )
    .unwrap();
    app.sessions.push(session);
    app.open_profile_manager();
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert!(app.pending_profile.is_none());
    assert!(app.profile_switch_blocker().unwrap().contains("sessions"));
    app.shutdown_all();
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert!(app.pending_profile.is_some());
}

#[test]
fn pending_profile_database_failure_keeps_old_store_usable() {
    let (_dir, mut app, roots) = profile_app(&["default", "work"]);
    let active = app.profile.clone();
    let old_store = Arc::clone(&app.store);
    app.open_profile_manager();
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();
    let target = app.pending_profile.clone().unwrap();
    std::fs::write(target.launcher_db(), b"not a sqlite database").unwrap();
    assert!(!crate::apply_pending_profile(
        &mut app,
        crate::load_profile_app
    ));
    assert_eq!(app.profile, active);
    assert!(Arc::ptr_eq(&old_store, &app.store));
    app.reload_hosts().unwrap();
    assert_eq!(app.mode, AppMode::ProfilePicker);
    assert_eq!(
        crate::profile::ProfileState::load(&roots.data_root)
            .unwrap()
            .unwrap()
            .last_used,
        Some(active.unwrap().id)
    );
}
