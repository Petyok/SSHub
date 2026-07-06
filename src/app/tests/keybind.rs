use super::*;

#[test]
pub(crate) fn keybind_editor_captures_and_persists() {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("SSHUB_CONFIG_DIR", dir.path());

    let mut app = test_app(vec![("web", host("web"))]);
    // Open the editor (Ctrl+K).
    app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.mode, AppMode::KeybindEditor);

    // Row 0 is "Save". Enter starts capture; press F10 to bind it.
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert!(app.keybind_editor.unwrap().capturing);
    app.handle_key(key(KeyCode::F(10))).unwrap();
    assert!(!app.keybind_editor.unwrap().capturing);

    assert_eq!(app.config.keybinds.save, vec!["F10".to_string()]);
    assert!(app.is_save_key(&key(KeyCode::F(10))));
    assert!(!app.is_save_key(&key(KeyCode::F(2))));

    // Persisted to config.toml under the temp dir.
    let saved = crate::config::load_config().unwrap();
    assert_eq!(saved.keybinds.save, vec!["F10".to_string()]);

    // 'a' adds another binding without replacing.
    app.handle_key(key_char('a')).unwrap();
    assert!(app.keybind_editor.unwrap().append);
    app.handle_key(key(KeyCode::F(12))).unwrap();
    assert_eq!(
        app.config.keybinds.save,
        vec!["F10".to_string(), "F12".to_string()]
    );
    assert!(app.is_save_key(&key(KeyCode::F(10))));
    assert!(app.is_save_key(&key(KeyCode::F(12))));

    // 'x' unbinds the action entirely.
    app.handle_key(key_char('x')).unwrap();
    assert!(app.config.keybinds.save.is_empty());
    assert!(!app.is_save_key(&key(KeyCode::F(10))));

    // 'r' resets the selected action to defaults.
    app.handle_key(key_char('r')).unwrap();
    assert_eq!(app.config.keybinds.save, vec!["F2", "Ctrl+S"]);

    std::env::remove_var("SSHUB_CONFIG_DIR");
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
