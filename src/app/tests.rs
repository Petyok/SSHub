
use super::*;
use crate::store::{LauncherStore, NewHost};
use std::collections::HashMap;

pub(crate) fn test_store() -> Arc<LauncherStore> {
    Arc::new(LauncherStore::open_in_memory().unwrap())
}

struct MockResolver {
    hosts: HashMap<String, SshHost>,
    order: Vec<String>,
}

impl MockResolver {
    fn new(entries: Vec<(&str, SshHost)>) -> Self {
        let mut hosts = HashMap::new();
        let mut order = Vec::new();
        for (name, host) in entries {
            order.push(name.to_string());
            hosts.insert(name.to_string(), host);
        }
        Self { hosts, order }
    }
}

impl HostResolver for MockResolver {
    fn list_hosts(&self) -> Result<Vec<String>> {
        Ok(self.order.clone())
    }

    fn resolve_host(&self, name: &str) -> Result<SshHost> {
        self.hosts
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("unknown host {name}"))
    }
}

struct RecordingLauncher {
    last: Arc<std::sync::Mutex<Option<String>>>,
}

impl RecordingLauncher {
    fn new() -> (Self, Arc<std::sync::Mutex<Option<String>>>) {
        let last = Arc::new(std::sync::Mutex::new(None));
        (
            Self {
                last: Arc::clone(&last),
            },
            last,
        )
    }

    fn take(last: &Arc<std::sync::Mutex<Option<String>>>) -> Option<String> {
        last.lock().ok()?.take()
    }
}

impl TerminalLauncher for RecordingLauncher {
    fn launch(&self, host: &SshHost) -> Result<()> {
        if let Ok(mut guard) = self.last.lock() {
            *guard = Some(host.name.clone());
        }
        Ok(())
    }

    fn launch_ssh_argv(&self, ssh_argv: &[String]) -> Result<()> {
        // Record last argument (the hostname/alias) for test assertions
        if let Ok(mut guard) = self.last.lock() {
            *guard = ssh_argv.last().cloned();
        }
        Ok(())
    }
}

pub(crate) fn test_app(hosts: Vec<(&str, SshHost)>) -> App {
    let resolver = MockResolver::new(hosts);
    let metadata: Arc<dyn MetadataStore> = Arc::new(MetadataDb::default());
    let (launcher, _launched) = RecordingLauncher::new();
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
    app
}

pub(crate) fn host(name: &str) -> SshHost {
    let mut h = SshHost::new(name);
    h.hostname = Some(format!("{name}.example.com"));
    h
}

#[test]
pub(crate) fn keyevent_to_spec_roundtrips() {
    let f2 = KeyEvent::new(KeyCode::F(2), KeyModifiers::empty());
    assert_eq!(keyevent_to_spec(&f2).as_deref(), Some("F2"));
    let ctrl_s = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
    assert_eq!(keyevent_to_spec(&ctrl_s).as_deref(), Some("Ctrl+S"));
    // Round-trips through parse_keyspec back to a matching event.
    let spec = keyevent_to_spec(&ctrl_s).unwrap();
    let (code, mods) = parse_keyspec(&spec).unwrap();
    assert!(keyspec_matches(code, mods, &ctrl_s));
}

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
pub(crate) fn multiline_paste_into_form_stays_in_field() {
    let mut app = test_app(vec![("web", host("web"))]);
    app.active_tab = 2; // keys tab
    app.enter_identity_form(None).unwrap();
    assert_eq!(app.mode, AppMode::IdentityForm);

    // Navigate to the Private key path field.
    while app.identity_form.as_ref().unwrap().field != IdentityFormField::PrivateKey {
        app.handle_key(key(KeyCode::Down)).unwrap();
    }

    // Paste a multi-line PEM blob. Previously the newlines fired
    // Enter/save and the rest ran as commands; now it must all stay put.
    let key_blob =
        "-----BEGIN OPENSSH PRIVATE KEY-----\nabc123\ndef456\n-----END OPENSSH PRIVATE KEY-----\n";
    app.handle_paste(key_blob).unwrap();

    // Still in the form, on the same field, no host connection triggered.
    assert_eq!(app.mode, AppMode::IdentityForm);
    let form = app.identity_form.as_ref().unwrap();
    assert_eq!(form.field, IdentityFormField::PrivateKey);
    // Key material captured as a blob (written to a file on save).
    assert_eq!(form.pasted_key.as_deref(), Some(key_blob));
    assert!(form.private_key.contains("pasted key"));
}

#[test]
pub(crate) fn pasted_key_material_is_written_to_a_file_on_save() {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", dir.path());

    let mut app = test_app(vec![("web", host("web"))]);
    app.active_tab = 2;
    app.enter_identity_form(None).unwrap();

    // Name the identity, then paste key material into the key field.
    for c in "pasted-id".chars() {
        app.handle_key(key_char(c)).unwrap();
    }
    while app.identity_form.as_ref().unwrap().field != IdentityFormField::PrivateKey {
        app.handle_key(key(KeyCode::Down)).unwrap();
    }
    let blob = "-----BEGIN OPENSSH PRIVATE KEY-----\nabc123\n-----END OPENSSH PRIVATE KEY-----";
    app.handle_paste(blob).unwrap();
    assert!(app.identity_form.as_ref().unwrap().pasted_key.is_some());

    app.handle_key(key(KeyCode::F(2))).unwrap(); // save
    assert_eq!(app.mode, AppMode::Normal);

    let created = app
        .store
        .get_identity_by_name("pasted-id")
        .unwrap()
        .expect("identity created");
    let path = created.private_key.expect("key path set");
    assert!(path.to_string_lossy().contains("sshub_pasted-id"));
    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(contents.contains("BEGIN OPENSSH PRIVATE KEY"));
    assert!(contents.ends_with('\n'));

    std::env::remove_var("HOME");
}

#[test]
pub(crate) fn backspace_discards_pasted_key_blob() {
    let mut app = test_app(vec![("web", host("web"))]);
    app.active_tab = 2;
    app.enter_identity_form(None).unwrap();
    while app.identity_form.as_ref().unwrap().field != IdentityFormField::PrivateKey {
        app.handle_key(key(KeyCode::Down)).unwrap();
    }
    app.handle_paste("-----BEGIN OPENSSH PRIVATE KEY-----\nx\n-----END OPENSSH PRIVATE KEY-----")
        .unwrap();
    assert!(app.identity_form.as_ref().unwrap().pasted_key.is_some());

    app.handle_key(key(KeyCode::Backspace)).unwrap();
    let form = app.identity_form.as_ref().unwrap();
    assert!(form.pasted_key.is_none());
    assert!(form.private_key.is_empty());
}

#[test]
pub(crate) fn identity_grid_navigation_moves_by_row_and_column() {
    let mut app = test_app(vec![("web", host("web"))]);
    app.terminal_area = ratatui::layout::Rect::new(0, 0, 140, 40); // wide → 2 cols
    app.identities = (0..5)
        .map(|i| crate::store::Identity {
            id: i,
            name: format!("id{i}"),
            username: None,
            private_key: None,
            certificate: None,
            has_password: false,
        })
        .collect();
    // Grid: [0,1] [2,3] [4]
    app.identity_selected = 0;
    app.move_identity_grid(0, 1);
    assert_eq!(app.identity_selected, 1, "right");
    app.move_identity_grid(0, 1);
    assert_eq!(app.identity_selected, 1, "right at edge stays");
    app.move_identity_grid(1, 0);
    assert_eq!(app.identity_selected, 3, "down a row, same column");
    app.move_identity_grid(0, -1);
    assert_eq!(app.identity_selected, 2, "left");
    app.move_identity_grid(1, 0);
    assert_eq!(app.identity_selected, 4, "down into last row");
    app.move_identity_grid(1, 0);
    assert_eq!(app.identity_selected, 4, "no row below, stays");
    app.identity_selected = 3;
    app.move_identity_grid(1, 0);
    assert_eq!(
        app.identity_selected, 4,
        "down from col1 drops onto shorter last row"
    );
}

#[test]
pub(crate) fn keyless_identity_secret_is_a_login_password() {
    use std::collections::HashMap;
    use std::sync::Mutex;
    struct MapStore(Mutex<HashMap<String, String>>);
    impl crate::credentials::PasswordStore for MapStore {
        fn get(&self, k: &str) -> anyhow::Result<Option<String>> {
            Ok(self.0.lock().unwrap().get(k).cloned())
        }
        fn set(&self, k: &str, v: &str) -> anyhow::Result<()> {
            self.0.lock().unwrap().insert(k.into(), v.into());
            Ok(())
        }
        fn delete(&self, k: &str) -> anyhow::Result<()> {
            self.0.lock().unwrap().remove(k);
            Ok(())
        }
    }

    let store = test_store();
    // Identity with username + password, no key file.
    let id = store
        .create_identity(&crate::store::NewIdentity {
            name: "team".into(),
            username: Some("ops".into()),
            private_key: None,
            certificate: None,
            sort_order: 0,
            has_password: true,
        })
        .unwrap()
        .id;
    let mut nh = NewHost::launcher("h1", "10.0.0.1");
    nh.identity_id = Some(id);
    let host_id = store.create_host(&nh).unwrap().id;

    let pw = MapStore(Mutex::new(HashMap::new()));
    crate::credentials::PasswordStore::set(&pw, &crate::credentials::identity_key(id), "s3cret")
        .unwrap();

    let entry = HostEntry::Managed(store.get_host(host_id).unwrap().unwrap());
    let (secret, diag) = resolve_pending_secret(&entry, &pw);
    assert!(
        matches!(secret, Some(crate::session::PendingSecret::Password(ref p)) if p == "s3cret"),
        "keyless identity should yield a login password, got {secret:?} / {diag}"
    );
}

#[test]
pub(crate) fn paste_in_normal_mode_is_ignored() {
    let mut app = test_app(vec![("web", host("web"))]);
    // A stray paste in Normal must not run commands or change mode.
    app.handle_paste("adq#/").unwrap();
    assert_eq!(app.mode, AppMode::Normal);
    assert!(app.host_form.is_none());
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
pub(crate) fn base64_encode_known_vectors() {
    // Test the standard test vectors plus a few padding cases.
    assert_eq!(base64_encode(b""), "");
    assert_eq!(base64_encode(b"f"), "Zg==");
    assert_eq!(base64_encode(b"fo"), "Zm8=");
    assert_eq!(base64_encode(b"foo"), "Zm9v");
    assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
    assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
    assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    assert_eq!(
        base64_encode(b"Many hands make light work."),
        "TWFueSBoYW5kcyBtYWtlIGxpZ2h0IHdvcmsu"
    );
}

pub(crate) fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
}

pub(crate) fn key_char(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty())
}

pub(crate) fn legacy_meta(entry: &mut HostEntry) -> &mut crate::metadata::HostMetadata {
    entry.legacy_mut().expect("legacy host").1
}

#[test]
pub(crate) fn reload_hosts_builds_entries_with_metadata_defaults() {
    let app = test_app(vec![("alpha", host("alpha")), ("beta", host("beta"))]);
    assert_eq!(app.hosts.len(), 2);
    assert_eq!(app.filtered_indices, vec![0, 1]);
    assert_eq!(app.hosts[0].name(), "alpha");
    if let HostEntry::Legacy { meta, .. } = &app.hosts[0] {
        assert_eq!(meta.host_name, "alpha");
    }
}

#[test]
pub(crate) fn slash_opens_palette_mode() {
    let mut app = test_app(vec![
        ("web-prod", host("web-prod")),
        ("db-staging", host("db-staging")),
    ]);
    legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
    legacy_meta(&mut app.hosts[1]).tags = vec!["staging".into()];
    app.rebuild_filter();

    app.handle_key(key_char('/')).unwrap();
    assert_eq!(app.mode, AppMode::Palette);
    assert_eq!(app.palette_results.len(), 2);

    app.handle_key(key_char('w')).unwrap();
    assert_eq!(app.palette_query, "w");
    assert_eq!(app.palette_results.len(), 1);
}

#[test]
pub(crate) fn typing_char_opens_palette() {
    let mut app = test_app(vec![
        ("web-prod", host("web-prod")),
        ("db-staging", host("db-staging")),
    ]);
    legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
    legacy_meta(&mut app.hosts[1]).tags = vec!["staging".into()];
    app.rebuild_filter();

    // Typing a character in Normal mode opens the palette
    app.handle_key(key_char('w')).unwrap();
    assert_eq!(app.mode, AppMode::Palette);
    assert_eq!(app.palette_query, "w");
    assert_eq!(app.palette_results.len(), 1);
}

#[test]
pub(crate) fn esc_exits_search_and_clears_query_and_tag_filter() {
    let mut app = test_app(vec![("alpha", host("alpha"))]);
    app.tag_filters = vec!["prod".into()];
    app.mode = AppMode::Search;
    app.search_query = "al".into();

    app.handle_key(key(KeyCode::Esc)).unwrap();

    assert_eq!(app.mode, AppMode::Normal);
    assert!(app.search_query.is_empty());
    assert!(app.tag_filters.is_empty());
}

#[test]
pub(crate) fn navigation_wraps_around() {
    let mut app = test_app(vec![("a", host("a")), ("b", host("b"))]);
    assert_eq!(app.selected, 0);

    // Up from first wraps to last
    app.handle_key(key(KeyCode::Up)).unwrap();
    assert_eq!(app.selected, 1);

    // Down from last wraps to first
    app.handle_key(key(KeyCode::Down)).unwrap();
    assert_eq!(app.selected, 0);

    // Normal forward navigation
    app.handle_key(key(KeyCode::Down)).unwrap();
    assert_eq!(app.selected, 1);
}

#[test]
pub(crate) fn j_k_move_selection_in_search_mode() {
    let mut app = test_app(vec![("a", host("a")), ("b", host("b"))]);
    app.mode = AppMode::Search;

    app.handle_key(key_char('j')).unwrap();
    assert_eq!(app.selected, 1);

    app.handle_key(key_char('k')).unwrap();
    assert_eq!(app.selected, 0);
}

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
pub(crate) fn favourite_toggle_updates_metadata() {
    let mut app = test_app(vec![("web", host("web"))]);
    assert!(!app.hosts[0].favorite());

    app.handle_key(key_char('f')).unwrap();
    assert!(app.hosts[0].favorite());

    app.handle_key(key_char('f')).unwrap();
    assert!(!app.hosts[0].favorite());
}

#[test]
pub(crate) fn e_enters_host_detail_with_edit_buffers() {
    let mut app = test_app(vec![("web", host("web"))]);
    legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
    legacy_meta(&mut app.hosts[0]).description = Some("Primary".into());
    legacy_meta(&mut app.hosts[0]).environment = Some("staging".into());

    // HostDetail is the fallback metadata editor (used when an ssh_config
    // alias can't be materialized). Drive it directly.
    app.enter_host_detail().unwrap();
    assert_eq!(app.mode, AppMode::HostDetail);
    let edit = app.detail_edit.as_ref().unwrap();
    assert_eq!(edit.tags, "prod");
    assert_eq!(edit.description, "Primary");
    assert_eq!(edit.environment, "staging");

    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.mode, AppMode::Normal);
    assert!(app.detail_edit.is_none());
}

#[test]
pub(crate) fn host_detail_save_persists_metadata() {
    let metadata: Arc<dyn MetadataStore> = Arc::new(MetadataDb::default());
    let resolver = MockResolver::new(vec![("web", host("web"))]);
    let (launcher, _launched) = RecordingLauncher::new();
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

    app.enter_host_detail().unwrap();
    app.handle_key(key_char('p')).unwrap();
    app.handle_key(key_char('r')).unwrap();
    app.handle_key(key_char('o')).unwrap();
    app.handle_key(key_char('d')).unwrap();
    app.handle_key(key(KeyCode::Tab)).unwrap();
    app.handle_key(key_char('n')).unwrap();
    app.handle_key(key_char('o')).unwrap();
    app.handle_key(key_char('t')).unwrap();
    app.handle_key(key_char('e')).unwrap();
    app.handle_key(key(KeyCode::Tab)).unwrap();
    app.handle_key(key_char('d')).unwrap();
    app.handle_key(key_char('e')).unwrap();
    app.handle_key(key_char('v')).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();

    assert_eq!(app.mode, AppMode::Normal);
    assert_eq!(app.hosts[0].tags(), &["prod".to_string()]);
    assert_eq!(app.hosts[0].description(), Some("note"));
    assert_eq!(app.hosts[0].environment(), Some("dev"));

    let stored = metadata.get("web").unwrap().unwrap();
    assert_eq!(stored.tags, vec!["prod".to_string()]);
    assert_eq!(stored.description.as_deref(), Some("note"));
    assert_eq!(stored.environment.as_deref(), Some("dev"));
}

#[test]
pub(crate) fn host_detail_esc_discards_unsaved_edits() {
    let metadata: Arc<dyn MetadataStore> = Arc::new(MetadataDb::default());
    let resolver = MockResolver::new(vec![("web", host("web"))]);
    let (launcher, _launched) = RecordingLauncher::new();
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
    legacy_meta(&mut app.hosts[0]).description = Some("saved".into());
    metadata.upsert(legacy_meta(&mut app.hosts[0])).unwrap();

    app.enter_host_detail().unwrap();
    app.handle_key(key_char('x')).unwrap();
    app.handle_key(key(KeyCode::Esc)).unwrap();

    assert_eq!(app.mode, AppMode::Normal);
    assert_eq!(app.hosts[0].description(), Some("saved"));
}

#[test]
pub(crate) fn favourite_toggle_works_in_host_detail() {
    let mut app = test_app(vec![("web", host("web"))]);
    app.enter_host_detail().unwrap();
    app.handle_key(key_char('f')).unwrap();
    assert!(app.hosts[0].favorite());
}

#[test]
pub(crate) fn parse_tags_splits_and_trims() {
    assert_eq!(
        parse_tags(" prod , db , , staging "),
        vec!["prod", "db", "staging"]
    );
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

#[test]
pub(crate) fn keychain_create_edit_delete_flow() {
    let store = test_store();
    let mut app = App::new_with_deps(
        AppConfig::default(),
        AppDeps {
            resolver: Box::new(MockResolver::new(vec![])),
            metadata: Arc::new(MetadataDb::default()),
            store: Arc::clone(&store),
            launcher: Box::new(RecordingLauncher::new().0),
            password_store: Box::new(crate::credentials::NoopPasswordStore),
        },
    );
    app.active_tab = 2;
    app.reload_identities().unwrap();
    app.handle_key(key_char('a')).unwrap();

    // Single-step model: type straight into the active field, ↓ advances.
    for c in "work-laptop".chars() {
        app.handle_key(key_char(c)).unwrap();
    }
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Username
    for c in "deploy".chars() {
        app.handle_key(key_char(c)).unwrap();
    }
    app.handle_key(key(KeyCode::Down)).unwrap(); // → PrivateKey
    for c in "~/.ssh/id_ed25519".chars() {
        app.handle_key(key_char(c)).unwrap();
    }
    // F2 to save
    app.handle_key(key(KeyCode::F(2))).unwrap();

    assert_eq!(app.mode, AppMode::Normal);
    let created = store
        .get_identity_by_name("work-laptop")
        .unwrap()
        .expect("created in store");
    assert_eq!(created.username.as_deref(), Some("deploy"));
}

#[test]
pub(crate) fn tag_filter_narrows_candidates_before_search() {
    let mut app = test_app(vec![("web", host("web")), ("db", host("db"))]);
    legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
    legacy_meta(&mut app.hosts[1]).tags = vec!["staging".into()];
    app.tag_filters = vec!["prod".into()];
    app.rebuild_filter();

    assert_eq!(app.filtered_indices, vec![0]);
}

#[test]
pub(crate) fn tag_filter_picker_arrow_selects_and_applies_tag() {
    let mut app = test_app(vec![("web", host("web")), ("db", host("db"))]);
    legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
    legacy_meta(&mut app.hosts[1]).tags = vec!["staging".into()];
    app.rebuild_filter();

    app.handle_key(key_char('#')).unwrap();
    // Rows are ["(all)", "prod", "staging"]; row 0 selected by default.
    assert_eq!(app.tag_filter_rows(), vec!["(all)", "prod", "staging"]);
    assert_eq!(app.tag_filter_selected, 0);

    // Arrow down twice lands on "staging" and Enter toggles + applies it.
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();

    assert_eq!(app.mode, AppMode::Normal);
    assert_eq!(app.tag_filters, vec!["staging".to_string()]);
    assert_eq!(app.filtered_indices, vec![1]);
}

#[test]
pub(crate) fn tag_filter_picker_space_toggles_multiple_tags_and_ands_them() {
    let mut app = test_app(vec![
        ("web", host("web")),
        ("db", host("db")),
        ("both", host("both")),
    ]);
    legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
    legacy_meta(&mut app.hosts[1]).tags = vec!["eu".into()];
    legacy_meta(&mut app.hosts[2]).tags = vec!["prod".into(), "eu".into()];
    app.rebuild_filter();

    app.handle_key(key_char('#')).unwrap();
    // Rows: ["(all)", "eu", "prod"]. Space toggles a tag and stays open.
    app.handle_key(key(KeyCode::Down)).unwrap(); // → "eu"
    app.handle_key(key_char(' ')).unwrap();
    assert_eq!(app.mode, AppMode::TagFilter, "stays open after Space");
    assert_eq!(app.tag_filters, vec!["eu".to_string()]);

    app.handle_key(key(KeyCode::Down)).unwrap(); // → "prod"
    app.handle_key(key_char(' ')).unwrap();
    assert_eq!(app.tag_filters, vec!["eu".to_string(), "prod".to_string()]);

    // AND semantics: only the host carrying both tags survives.
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.mode, AppMode::Normal);
    assert_eq!(app.filtered_indices, vec![2]);
}

#[test]
pub(crate) fn tag_filter_picker_enter_after_multiselect_keeps_all_tags() {
    // Regression: Enter must confirm the built-up set, never remove the
    // last-highlighted tag.
    let mut app = test_app(vec![
        ("web", host("web")),
        ("db", host("db")),
        ("both", host("both")),
    ]);
    legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
    legacy_meta(&mut app.hosts[1]).tags = vec!["eu".into()];
    legacy_meta(&mut app.hosts[2]).tags = vec!["prod".into(), "eu".into()];
    app.rebuild_filter();

    app.handle_key(key_char('#')).unwrap();
    app.handle_key(key(KeyCode::Down)).unwrap(); // → "eu"
    app.handle_key(key_char(' ')).unwrap(); // toggle eu on
    app.handle_key(key(KeyCode::Down)).unwrap(); // → "prod"
    app.handle_key(key_char(' ')).unwrap(); // toggle prod on
                                            // Cursor still on "prod" (active). Enter must NOT toggle it off.
    app.handle_key(key(KeyCode::Enter)).unwrap();

    assert_eq!(app.mode, AppMode::Normal);
    assert_eq!(app.tag_filters, vec!["eu".to_string(), "prod".to_string()]);
    assert_eq!(app.filtered_indices, vec![2]);
}

#[test]
pub(crate) fn tag_filter_picker_space_toggles_tag_off() {
    let mut app = test_app(vec![("web", host("web")), ("db", host("db"))]);
    legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
    legacy_meta(&mut app.hosts[1]).tags = vec!["staging".into()];
    app.tag_filters = vec!["prod".into()];
    app.rebuild_filter();

    app.handle_key(key_char('#')).unwrap();
    app.handle_key(key(KeyCode::Down)).unwrap(); // → "prod" (already active)
    app.handle_key(key_char(' ')).unwrap(); // toggle off
    assert!(app.tag_filters.is_empty());
    assert_eq!(app.filtered_indices.len(), 2);
}

#[test]
pub(crate) fn tag_filter_picker_all_row_clears_filter() {
    let mut app = test_app(vec![("web", host("web")), ("db", host("db"))]);
    legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
    legacy_meta(&mut app.hosts[1]).tags = vec!["staging".into()];
    app.tag_filters = vec!["prod".into()];
    app.rebuild_filter();

    app.handle_key(key_char('#')).unwrap();
    // Cursor opens on the "(all)" row.
    assert_eq!(app.tag_filter_selected, 0);

    // Enter on "(all)" clears every active filter and closes.
    app.handle_key(key(KeyCode::Enter)).unwrap();

    assert!(app.tag_filters.is_empty());
    assert_eq!(app.filtered_indices.len(), 2);
}

#[test]
pub(crate) fn tag_filter_picker_esc_keeps_active_filter() {
    let mut app = test_app(vec![("web", host("web")), ("db", host("db"))]);
    legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
    legacy_meta(&mut app.hosts[1]).tags = vec!["staging".into()];
    app.tag_filters = vec!["prod".into()];
    app.rebuild_filter();

    app.handle_key(key_char('#')).unwrap();
    // Esc closes the picker without touching the active filter.
    app.handle_key(key(KeyCode::Esc)).unwrap();

    assert_eq!(app.mode, AppMode::Normal);
    assert_eq!(app.tag_filters, vec!["prod".to_string()]);
    assert_eq!(app.filtered_indices, vec![0]);
}

#[test]
pub(crate) fn hash_enters_tag_filter_and_enter_applies() {
    let mut app = test_app(vec![("web", host("web")), ("db", host("db"))]);
    legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
    legacy_meta(&mut app.hosts[1]).tags = vec!["staging".into()];
    app.rebuild_filter();

    app.handle_key(key_char('#')).unwrap();
    assert_eq!(app.mode, AppMode::TagFilter);

    app.handle_key(key_char('p')).unwrap();
    app.handle_key(key_char('r')).unwrap();
    app.handle_key(key_char('o')).unwrap();
    app.handle_key(key_char('d')).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();

    // Enter toggles the highlighted match, applies it and returns to Normal
    // so the list can be navigated while filtered.
    assert_eq!(app.mode, AppMode::Normal);
    assert_eq!(app.tag_filters, vec!["prod".to_string()]);
    assert_eq!(app.filtered_indices, vec![0]);

    // Esc in Normal clears the active tag filter.
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.mode, AppMode::Normal);
    assert!(app.tag_filters.is_empty());
    assert_eq!(app.filtered_indices.len(), 2);
}

#[test]
pub(crate) fn sort_mode_label_orders_by_display_name() {
    let store = test_store();
    let default_id = store.get_identity_by_name("Default").unwrap().unwrap().id;
    store
        .create_host(&NewHost {
            name: "z-host".into(),
            label: Some("Zulu".into()),
            address: "10.0.0.1".into(),
            port: 22,
            group_id: None,
            identity_id: Some(default_id),
            tags: vec![],
            notes: None,
            ..Default::default()
        })
        .unwrap();
    store
        .create_host(&NewHost {
            name: "a-host".into(),
            label: Some("Alpha".into()),
            address: "10.0.0.2".into(),
            port: 22,
            group_id: None,
            identity_id: Some(default_id),
            tags: vec![],
            notes: None,
            ..Default::default()
        })
        .unwrap();

    let mut app = App::new_with_deps(
        AppConfig::default(),
        AppDeps {
            resolver: Box::new(MockResolver::new(vec![])),
            metadata: Arc::new(MetadataDb::default()),
            store,
            launcher: Box::new(RecordingLauncher::new().0),
            password_store: Box::new(crate::credentials::NoopPasswordStore),
        },
    );
    app.reload_hosts().unwrap();
    assert_eq!(
        app.filtered_indices
            .iter()
            .map(|&i| app.hosts[i].name().to_string())
            .collect::<Vec<_>>(),
        vec!["a-host", "z-host"]
    );
}

#[test]
pub(crate) fn reload_hosts_skips_unresolved_and_preserves_selection() {
    struct PartialResolver {
        order: Vec<String>,
    }

    impl HostResolver for PartialResolver {
        fn list_hosts(&self) -> Result<Vec<String>> {
            Ok(self.order.clone())
        }

        fn resolve_host(&self, name: &str) -> Result<SshHost> {
            if name == "bad" {
                anyhow::bail!("simulated resolve failure");
            }
            Ok(host(name))
        }
    }

    let metadata: Arc<dyn MetadataStore> = Arc::new(MetadataDb::default());
    let (launcher, _launched) = RecordingLauncher::new();
    let mut app = App::new_with_deps(
        AppConfig::default(),
        AppDeps {
            resolver: Box::new(PartialResolver {
                order: vec!["good".into(), "bad".into(), "also".into()],
            }),
            metadata,
            store: test_store(),
            launcher: Box::new(launcher),
            password_store: Box::new(crate::credentials::NoopPasswordStore),
        },
    );
    app.reload_hosts().unwrap();
    assert_eq!(app.hosts.len(), 2);
    assert!(app.hosts.iter().all(|e| e.name() != "bad"));

    app.handle_key(key(KeyCode::Down)).unwrap();
    assert_eq!(app.selected_entry().unwrap().name(), "good");

    app.reload_hosts().unwrap();
    assert_eq!(app.selected_entry().unwrap().name(), "good");
}

#[test]
pub(crate) fn host_form_up_down_navigate_fields_in_both_directions() {
    let mut app = test_app(vec![]);
    app.enter_host_form(None, false).unwrap();
    assert_eq!(
        app.host_form.as_ref().unwrap().field,
        HostFormField::Address
    );

    app.handle_key(key(KeyCode::Down)).unwrap();
    assert_eq!(
        app.host_form.as_ref().unwrap().field,
        HostFormField::Password
    );

    app.handle_key(key(KeyCode::Down)).unwrap();
    assert_eq!(
        app.host_form.as_ref().unwrap().field,
        HostFormField::Username
    );

    app.handle_key(key(KeyCode::Down)).unwrap();
    assert_eq!(app.host_form.as_ref().unwrap().field, HostFormField::Label);

    app.handle_key(key(KeyCode::Up)).unwrap();
    assert_eq!(
        app.host_form.as_ref().unwrap().field,
        HostFormField::Username
    );

    app.handle_key(key(KeyCode::Up)).unwrap();
    assert_eq!(
        app.host_form.as_ref().unwrap().field,
        HostFormField::Password
    );

    app.handle_key(key(KeyCode::Up)).unwrap();
    assert_eq!(
        app.host_form.as_ref().unwrap().field,
        HostFormField::Address
    );

    // Navigate to the end (12 downs from Address)
    for _ in 0..12 {
        app.handle_key(key(KeyCode::Down)).unwrap();
    }
    assert_eq!(app.host_form.as_ref().unwrap().field, HostFormField::OsIcon);

    app.handle_key(key(KeyCode::Up)).unwrap();
    assert_eq!(
        app.host_form.as_ref().unwrap().field,
        HostFormField::RemoteCommand
    );
}

#[test]
pub(crate) fn host_form_picker_at_boundary_moves_to_adjacent_field() {
    let mut app = test_app(vec![]);
    app.enter_host_form(None, false).unwrap();
    for _ in 0..6 {
        app.handle_key(key(KeyCode::Down)).unwrap();
    }
    assert_eq!(app.host_form.as_ref().unwrap().field, HostFormField::Group);

    app.handle_key(key(KeyCode::Up)).unwrap();
    assert_eq!(app.host_form.as_ref().unwrap().field, HostFormField::Port);

    app.handle_key(key(KeyCode::Down)).unwrap();
    assert_eq!(app.host_form.as_ref().unwrap().field, HostFormField::Group);

    app.handle_key(key(KeyCode::Down)).unwrap();
    assert_eq!(
        app.host_form.as_ref().unwrap().field,
        HostFormField::Identity
    );
}
