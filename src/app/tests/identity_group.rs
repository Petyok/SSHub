use super::*;

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
