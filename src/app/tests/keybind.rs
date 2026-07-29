use super::*;

#[test]
pub(crate) fn keybind_editor_captures_and_persists() {
    let dir = tempfile::tempdir().unwrap();
    crate::config::with_test_config_dir(dir.path(), || {
        let mut app = test_app(vec![("web", host("web"))]);
        // Open the editor (Ctrl+K).
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL))
            .unwrap();
        assert_eq!(app.mode, AppMode::KeybindEditor);

        // Row 0 is "Save". Enter starts capture; press F10 to bind it.
        app.handle_key(key(KeyCode::Enter)).unwrap();
        assert!(app.keybind_editor.as_ref().unwrap().capturing);
        app.handle_key(key(KeyCode::F(10))).unwrap();
        assert!(!app.keybind_editor.as_ref().unwrap().capturing);

        assert_eq!(app.config.keybinds.save, vec!["F10".to_string()]);
        assert!(app.is_save_key(&key(KeyCode::F(10))));
        assert!(!app.is_save_key(&key(KeyCode::F(2))));

        // Persisted to config.toml under the temp dir.
        let saved = crate::config::load_config().unwrap();
        assert_eq!(saved.keybinds.save, vec!["F10".to_string()]);

        // Ctrl+A adds another binding without replacing.
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL))
            .unwrap();
        assert!(app.keybind_editor.as_ref().unwrap().append);
        app.handle_key(key(KeyCode::F(12))).unwrap();
        assert_eq!(
            app.config.keybinds.save,
            vec!["F10".to_string(), "F12".to_string()]
        );
        assert!(app.is_save_key(&key(KeyCode::F(10))));
        assert!(app.is_save_key(&key(KeyCode::F(12))));

        // Ctrl+X unbinds the action entirely.
        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL))
            .unwrap();
        assert!(app.config.keybinds.save.is_empty());
        assert!(!app.is_save_key(&key(KeyCode::F(10))));

        // Ctrl+R resets the selected action to defaults.
        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL))
            .unwrap();
        assert_eq!(app.config.keybinds.save, vec!["F2", "Ctrl+S"]);
    });
}

#[test]
pub(crate) fn keybind_editor_filter_rebinds_filtered_action() {
    let dir = tempfile::tempdir().unwrap();
    crate::config::with_test_config_dir(dir.path(), || {
        let mut app = test_app(vec![("web", host("web"))]);
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL))
            .unwrap();

        // Narrow to "Help" — filtered[0] is Help, not Save (ALL[0]).
        for c in ['h', 'e', 'l', 'p'] {
            app.handle_key(key_char(c)).unwrap();
        }
        let actions = app.filtered_keybind_actions();
        assert!(
            actions.contains(&KeyAction::Help),
            "filter should include Help"
        );
        assert_eq!(app.keybind_editor.as_ref().unwrap().selected, 0);
        assert_eq!(actions[0], KeyAction::Help);

        app.handle_key(key(KeyCode::Enter)).unwrap();
        assert!(app.keybind_editor.as_ref().unwrap().capturing);
        // While capturing, a letter must bind — not extend the query.
        app.handle_key(key_char('z')).unwrap();
        assert!(!app.keybind_editor.as_ref().unwrap().capturing);
        assert_eq!(app.keybind_editor.as_ref().unwrap().query, "help");
        assert_eq!(app.config.keybinds.help, vec!["z".to_string()]);
        // Save (ALL[0]) must be untouched.
        assert_eq!(app.config.keybinds.save, vec!["F2", "Ctrl+S"]);
    });
}

#[test]
pub(crate) fn keybind_editor_esc_clears_query_then_closes() {
    let mut app = test_app(vec![("web", host("web"))]);
    app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL))
        .unwrap();
    app.handle_key(key_char('q')).unwrap();
    assert_eq!(app.keybind_editor.as_ref().unwrap().query, "q");
    assert_eq!(app.mode, AppMode::KeybindEditor);

    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert!(app.keybind_editor.as_ref().unwrap().query.is_empty());
    assert_eq!(app.mode, AppMode::KeybindEditor);

    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert!(app.keybind_editor.is_none());
    assert_eq!(app.mode, AppMode::Normal);
}

#[test]
pub(crate) fn keybind_editor_selection_resets_on_keystroke() {
    let mut app = test_app(vec![("web", host("web"))]);
    app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL))
        .unwrap();
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Down)).unwrap();
    assert_eq!(app.keybind_editor.as_ref().unwrap().selected, 2);

    app.handle_key(key_char('m')).unwrap();
    assert_eq!(app.keybind_editor.as_ref().unwrap().selected, 0);
    let len = app.filtered_keybind_actions().len();
    assert!(len < KeyAction::ALL.len());
    assert!(len > 0);
}

#[test]
pub(crate) fn help_filter_matches_and_esc_clears() {
    let mut app = test_app(vec![("web", host("web"))]);
    app.handle_key(key_char('?')).unwrap();
    assert_eq!(app.mode, AppMode::Help);

    for c in ['f', 'a', 'v'] {
        app.handle_key(key_char(c)).unwrap();
    }
    assert_eq!(app.help_query, "fav");
    assert_eq!(app.help_scroll, 0);
    let n = crate::tui::screens::help::help_line_count(&app.help_query);
    assert!(n < crate::tui::screens::help::help_line_count(""));
    assert!(n > 0);

    // j/k are query input, not scroll.
    app.handle_key(key_char('j')).unwrap();
    assert_eq!(app.help_query, "favj");
    assert_eq!(app.help_scroll, 0);

    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert!(app.help_query.is_empty());
    assert_eq!(app.mode, AppMode::Help);

    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.mode, AppMode::Normal);
}

#[test]
pub(crate) fn keybind_editor_letters_go_to_query_not_row_actions() {
    let dir = tempfile::tempdir().unwrap();
    crate::config::with_test_config_dir(dir.path(), || {
        let mut app = test_app(vec![("web", host("web"))]);
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL))
            .unwrap();
        let save_before = app.config.keybinds.save.clone();

        // Typing "agent" must filter, not append-capture / reset / unbind.
        for c in ['a', 'g', 'e', 'n', 't'] {
            app.handle_key(key_char(c)).unwrap();
        }
        assert_eq!(app.keybind_editor.as_ref().unwrap().query, "agent");
        assert!(!app.keybind_editor.as_ref().unwrap().capturing);
        assert_eq!(app.config.keybinds.save, save_before);
    });
}

#[test]
pub(crate) fn keybind_editor_clamps_selection_after_bind_leaves_filter() {
    let dir = tempfile::tempdir().unwrap();
    crate::config::with_test_config_dir(dir.path(), || {
        let mut app = test_app(vec![("web", host("web"))]);
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL))
            .unwrap();
        for c in "ctrl+".chars() {
            app.handle_key(key_char(c)).unwrap();
        }
        let len = app.filtered_keybind_actions().len();
        assert!(len > 1, "expected multiple Ctrl+ binds");
        if let Some(e) = app.keybind_editor.as_mut() {
            e.selected = len - 1;
        }
        let target = app.filtered_keybind_actions()[len - 1];
        app.handle_key(key(KeyCode::Enter)).unwrap();
        app.handle_key(key(KeyCode::F(9))).unwrap();
        // Row dropped out of the "ctrl+" filter; selection must stay in range.
        let after = app.filtered_keybind_actions().len();
        assert!(after < len);
        assert!(!app
            .config
            .keybinds
            .binds(target)
            .iter()
            .any(|b| b.to_lowercase().contains("ctrl+")));
        let selected = app.keybind_editor.as_ref().unwrap().selected;
        assert!(selected < after || after == 0);
    });
}

#[test]
pub(crate) fn quit_asks_for_confirmation_by_default() {
    let mut app = test_app(vec![("web", host("web"))]);
    // 'q' opens the confirm dialog instead of quitting.
    app.handle_key(key_char('q')).unwrap();
    assert_eq!(app.mode, AppMode::ConfirmQuit);
    assert!(!app.should_quit);

    // 'n' cancels back to Normal.
    app.handle_key(key_char('n')).unwrap();
    assert_eq!(app.mode, AppMode::Normal);
    assert!(!app.should_quit);

    // 'q' then 'y' quits.
    app.handle_key(key_char('q')).unwrap();
    app.handle_key(key_char('y')).unwrap();
    assert!(app.should_quit);
}

#[test]
pub(crate) fn ctrl_c_confirms_then_forces() {
    let mut app = test_app(vec![("web", host("web"))]);
    let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
    // First Ctrl+C asks.
    app.handle_key(ctrl_c).unwrap();
    assert_eq!(app.mode, AppMode::ConfirmQuit);
    assert!(!app.should_quit);
    // Second Ctrl+C forces quit.
    app.handle_key(ctrl_c).unwrap();
    assert!(app.should_quit);
}

#[test]
pub(crate) fn quit_confirmation_can_be_disabled() {
    let mut app = test_app(vec![("web", host("web"))]);
    app.config.appearance.confirm_quit = false;
    app.handle_key(key_char('q')).unwrap();
    assert!(app.should_quit);
}

#[test]
pub(crate) fn rebinding_add_host_action_takes_effect() {
    let mut app = test_app(vec![("web", host("web"))]);
    // Default: 'a' opens the new-host form.
    app.handle_key(key_char('a')).unwrap();
    assert_eq!(app.mode, AppMode::HostForm);
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.mode, AppMode::Normal);

    // Rebind add-host to 'n'; now 'a' no longer opens the form, 'n' does.
    app.config
        .keybinds
        .set(KeyAction::AddHost, vec!["n".to_string()]);
    app.handle_key(key_char('a')).unwrap();
    assert_ne!(app.mode, AppMode::HostForm);
    // 'a' fell through to the palette (type-to-search).
    app.mode = AppMode::Normal;
    app.handle_key(key_char('n')).unwrap();
    assert_eq!(app.mode, AppMode::HostForm);
}

#[test]
pub(crate) fn parse_keyspec_handles_common_forms() {
    assert_eq!(
        parse_keyspec("F2"),
        Some((KeyCode::F(2), KeyModifiers::empty()))
    );
    assert_eq!(
        parse_keyspec("F10"),
        Some((KeyCode::F(10), KeyModifiers::empty()))
    );
    assert_eq!(
        parse_keyspec("Ctrl+S"),
        Some((KeyCode::Char('s'), KeyModifiers::CONTROL))
    );
    assert_eq!(
        parse_keyspec("Alt+Enter"),
        Some((KeyCode::Enter, KeyModifiers::ALT))
    );
    assert_eq!(parse_keyspec(""), None);
    assert_eq!(parse_keyspec("Meta+X"), None);
}

#[test]
pub(crate) fn is_save_key_respects_config() {
    let mut app = test_app(vec![("web", host("web"))]);
    // Defaults: F2 and Ctrl+S.
    assert!(app.is_save_key(&key(KeyCode::F(2))));
    assert!(app.is_save_key(&KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)));
    assert!(!app.is_save_key(&key(KeyCode::F(4))));

    // Remap to Ctrl+Enter only.
    app.config.keybinds.save = vec!["Ctrl+Enter".to_string()];
    assert!(app.is_save_key(&KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL)));
    assert!(!app.is_save_key(&key(KeyCode::F(2))));
}

#[test]
pub(crate) fn q_and_ctrl_c_quit() {
    // With confirmation disabled, q and Ctrl+C quit immediately.
    let mut app = test_app(vec![("web", host("web"))]);
    app.config.appearance.confirm_quit = false;

    app.handle_key(key_char('q')).unwrap();
    assert!(app.should_quit);

    app.should_quit = false;
    app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.should_quit);
}
