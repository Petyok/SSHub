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

    // Regression: a plain (unshifted) letter must serialize lowercase and
    // round-trip so an editor-captured single-letter binding actually fires.
    // A bare uppercase spec means shift+letter, so "g" (not "G") is required.
    let plain_g = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::empty());
    assert_eq!(keyevent_to_spec(&plain_g).as_deref(), Some("g"));
    let (code, mods) = parse_keyspec("g").unwrap();
    assert!(keyspec_matches(code, mods, &plain_g));
    // And a bare uppercase spec must require shift, matching only shift+letter.
    let (code, mods) = parse_keyspec("G").unwrap();
    let shift_g = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::SHIFT);
    assert!(keyspec_matches(code, mods, &shift_g));
    assert!(!keyspec_matches(code, mods, &plain_g));
}

#[test]
pub(crate) fn pasted_key_material_is_written_to_a_file_on_save() {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", dir.path());

    let mut app = test_app(vec![("web", host("web"))]);
    app.active_tab = 3;
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
    app.active_tab = 3;
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
pub(crate) fn bare_char_does_not_open_palette() {
    // Type-ahead was removed: search is only reachable via '/'. A bare letter in
    // Normal mode must not open the palette (it stays a normal-mode shortcut / no-op).
    let mut app = test_app(vec![
        ("web-prod", host("web-prod")),
        ("db-staging", host("db-staging")),
    ]);
    app.rebuild_filter();

    app.handle_key(key_char('w')).unwrap();
    assert_ne!(app.mode, AppMode::Palette);
    assert!(app.palette_query.is_empty());
}

#[test]
pub(crate) fn palette_types_nav_letters() {
    // Regression: j/k are bound to move down/up, but inside the palette they
    // must be query text, not navigation — otherwise "jira" becomes "ira".
    let mut app = test_app(vec![("jira", host("jira")), ("kafka", host("kafka"))]);
    app.rebuild_filter();

    app.handle_key(key_char('/')).unwrap();
    assert_eq!(app.mode, AppMode::Palette);
    for c in "jira".chars() {
        app.handle_key(key_char(c)).unwrap();
    }
    assert_eq!(app.mode, AppMode::Palette);
    assert_eq!(app.palette_query, "jira");
    assert_eq!(app.palette_results.len(), 1);
}

#[test]
pub(crate) fn clear_ssh_log_keeps_command_line() {
    use crate::ssh::probe::{LogLevel, SshLogEntry};
    let mut app = test_app(vec![("web", host("web"))]);

    let mk = |line: &str| SshLogEntry {
        host_name: "web".into(),
        line: line.into(),
        level: LogLevel::Info,
        timestamp: 0,
    };
    app.push_ssh_log(mk("$ ssh web"));
    app.push_ssh_log(mk("debug1: handshake noise"));
    app.push_ssh_log(mk("debug1: more noise"));

    // On connect the handshake noise is dropped, but the command line survives
    // so the dashboard still shows how the host was connected to.
    app.clear_ssh_log_for_host("web");
    assert_eq!(app.ssh_log.len(), 1);
    assert_eq!(app.ssh_log[0].line, "$ ssh web");
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
pub(crate) fn j_k_type_into_search_query() {
    // In Search mode j/k are query text (not navigation) so host names like
    // "jira"/"kafka" are typeable; list movement is on the arrow keys.
    let mut app = test_app(vec![("a", host("a")), ("b", host("b"))]);
    app.mode = AppMode::Search;

    app.handle_key(key_char('j')).unwrap();
    app.handle_key(key_char('k')).unwrap();
    assert_eq!(app.search_query, "jk");
    assert_eq!(app.selected, 0);

    // Arrows still navigate the filtered list.
    app.search_query.clear();
    app.rebuild_filter();
    app.handle_key(key(KeyCode::Down)).unwrap();
    assert_eq!(app.selected, 1);
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

fn key_shift(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::SHIFT)
}

#[test]
pub(crate) fn nested_groups_build_tree_and_collapse_subtree() {
    use crate::store::{NewHost, NewHostGroup};

    let store = test_store();
    let parent = store
        .create_group(&NewHostGroup {
            name: "prod".into(),
            sort_order: 0,
            ..Default::default()
        })
        .unwrap();
    let child = store
        .create_group(&NewHostGroup {
            name: "eu".into(),
            sort_order: 1,
            parent_id: Some(parent.id),
            ..Default::default()
        })
        .unwrap();
    store
        .create_host(&NewHost {
            name: "p1".into(),
            address: "10.0.0.1".into(),
            port: 22,
            group_id: Some(parent.id),
            ..Default::default()
        })
        .unwrap();
    store
        .create_host(&NewHost {
            name: "e1".into(),
            address: "10.0.0.2".into(),
            port: 22,
            group_id: Some(child.id),
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

    // DFS order: parent header, its host, child header (depth 1), its host.
    assert_eq!(app.group_sections[0].depth, 0);
    assert_eq!(app.group_sections[1].depth, 1);
    assert_eq!(app.nav_rows.len(), 4);
    assert!(matches!(app.nav_rows[0], NavRow::Header(0)));
    assert!(matches!(app.nav_rows[2], NavRow::Header(1)));

    // Collapsing the parent hides the child header AND its host.
    app.toggle_group_by_section(0);
    assert_eq!(
        app.nav_rows.len(),
        1,
        "only the parent header stays visible"
    );
    assert!(matches!(app.nav_rows[0], NavRow::Header(0)));

    // A group can't parent itself or a descendant (would cycle).
    let eligible = app.eligible_parents(Some(parent.id));
    assert!(!eligible.contains(&parent.id));
    assert!(!eligible.contains(&child.id));
    // The child, however, may still be reparented under unrelated groups.
    assert!(app.eligible_parents(Some(child.id)).contains(&parent.id));
}

#[test]
pub(crate) fn shift_arrow_jumps_between_group_headers() {
    use crate::store::{NewHost, NewHostGroup};

    let store = test_store();
    let g1 = store
        .create_group(&NewHostGroup {
            name: "alpha".into(),
            sort_order: 0,
            ..Default::default()
        })
        .unwrap();
    let g2 = store
        .create_group(&NewHostGroup {
            name: "beta".into(),
            sort_order: 1,
            ..Default::default()
        })
        .unwrap();
    store
        .create_host(&NewHost {
            name: "a1".into(),
            address: "10.0.0.1".into(),
            port: 22,
            group_id: Some(g1.id),
            ..Default::default()
        })
        .unwrap();
    store
        .create_host(&NewHost {
            name: "b1".into(),
            address: "10.0.0.2".into(),
            port: 22,
            group_id: Some(g2.id),
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

    // Tree: [Header alpha, Host a1, Header beta, Host b1]
    assert_eq!(app.nav_rows.len(), 4);
    assert!(matches!(app.nav_rows[0], NavRow::Header(0)));
    assert!(matches!(app.nav_rows[2], NavRow::Header(1)));

    // On first host inside alpha — Shift+Down lands on beta header.
    app.selected = 1;
    app.handle_key(key_shift(KeyCode::Down)).unwrap();
    assert_eq!(app.selected, 2);
    assert_eq!(app.selected_nav_header(), Some(1));

    // Shift+Down wraps to alpha header.
    app.handle_key(key_shift(KeyCode::Down)).unwrap();
    assert_eq!(app.selected, 0);
    assert_eq!(app.selected_nav_header(), Some(0));

    // Shift+Up from alpha header wraps to beta.
    app.handle_key(key_shift(KeyCode::Up)).unwrap();
    assert_eq!(app.selected, 2);
}

#[test]
pub(crate) fn help_scroll_stops_at_render_ceiling() {
    // Regression: Down used to clamp at line_count-1 while the renderer
    // clamps at line_count - body_height. The gap banked invisible presses
    // that Up had to unwind before the view moved.
    let mut app = test_app(vec![("web", host("web"))]);
    app.terminal_area = ratatui::layout::Rect::new(0, 0, 80, 24);
    app.handle_key(key_char('?')).unwrap();
    assert_eq!(app.mode, AppMode::Help);

    let max = crate::tui::help_max_scroll(app.terminal_area);
    assert!(max > 0, "help content must overflow a 24-row terminal");
    for _ in 0..500 {
        app.handle_key(key(KeyCode::Down)).unwrap();
    }
    assert_eq!(app.help_scroll, max);
    // The very next Up must move the view immediately.
    app.handle_key(key(KeyCode::Up)).unwrap();
    assert_eq!(app.help_scroll, max - 1);
    // End lands on the same ceiling.
    app.handle_key(key(KeyCode::End)).unwrap();
    assert_eq!(app.help_scroll, max);
}
