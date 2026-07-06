use super::*;

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
pub(crate) fn paste_in_normal_mode_is_ignored() {
    let mut app = test_app(vec![("web", host("web"))]);
    // A stray paste in Normal must not run commands or change mode.
    app.handle_paste("adq#/").unwrap();
    assert_eq!(app.mode, AppMode::Normal);
    assert!(app.host_form.is_none());
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
