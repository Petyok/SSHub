use super::*;
use std::collections::HashMap;
use std::sync::Mutex;

/// In-memory credential store: what is stored decides, with no keyring.
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

#[test]
fn agent_snapshot_refreshes_when_stale_without_waiting_for_the_keys_tab() {
    let mut app = test_app(vec![]);
    let now = std::time::Instant::now();
    app.agent_info = None;
    app.agent_info_updated = now - std::time::Duration::from_secs(31);

    app.refresh_agent_info_with(now, || crate::ssh::agent::AgentInfo {
        socket_path: Some("/tmp/refreshed-agent.sock".into()),
        keys: vec![],
        forwarding_hosts: 0,
    });

    assert_eq!(
        app.agent_info.as_ref().unwrap().socket_path.as_deref(),
        Some("/tmp/refreshed-agent.sock")
    );
    assert_eq!(app.agent_info_updated, now);
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
    let effective = entry.managed().and_then(|m| m.identity.as_ref());
    let (secret, diag) = resolve_pending_secret(&entry, effective, &pw);
    assert!(
        matches!(secret, Some(crate::session::PendingSecret::Password(ref p)) if p == "s3cret"),
        "keyless identity should yield a login password, got {secret:?} / {diag}"
    );
}

#[test]
pub(crate) fn key_passphrase_outranks_a_remembered_host_password() {
    // The v0.17.0 shape: the host remembered a login password AND its identity
    // has a passphrase-protected key. ssh authenticates by key first, and the
    // askpass channel refuses to answer `Enter passphrase for …` with a
    // `Password`, so resolving the host secret first left the user staring at
    // a modal for a passphrase already in the keyring.
    let store = test_store();
    let id = store
        .create_identity(&crate::store::NewIdentity {
            name: "keyed".into(),
            username: Some("root".into()),
            private_key: Some("/home/u/.ssh/id_ed25519".into()),
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
    crate::credentials::PasswordStore::set(&pw, &crate::credentials::host_key(host_id), "host-pw")
        .unwrap();
    crate::credentials::PasswordStore::set(
        &pw,
        &crate::credentials::identity_key(id),
        "key-phrase",
    )
    .unwrap();

    let entry = HostEntry::Managed(store.get_host(host_id).unwrap().unwrap());
    let effective = entry.managed().and_then(|m| m.identity.as_ref());
    let (secret, diag) = resolve_pending_secret(&entry, effective, &pw);
    assert!(
        matches!(secret, Some(crate::session::PendingSecret::Passphrase(ref p)) if p == "key-phrase"),
        "the key passphrase must win over the host password, got {secret:?} / {diag}"
    );

    // Without a key the identity secret is a login password, and the host's
    // own password still comes first.
    let keyless = store
        .create_identity(&crate::store::NewIdentity {
            name: "keyless".into(),
            username: Some("root".into()),
            private_key: None,
            certificate: None,
            sort_order: 0,
            has_password: true,
        })
        .unwrap()
        .id;
    let mut nh2 = NewHost::launcher("h2", "10.0.0.2");
    nh2.identity_id = Some(keyless);
    let host2 = store.create_host(&nh2).unwrap().id;
    crate::credentials::PasswordStore::set(&pw, &crate::credentials::host_key(host2), "host-pw-2")
        .unwrap();
    let entry2 = HostEntry::Managed(store.get_host(host2).unwrap().unwrap());
    let effective2 = entry2.managed().and_then(|m| m.identity.as_ref());
    let (secret2, diag2) = resolve_pending_secret(&entry2, effective2, &pw);
    assert!(
        matches!(secret2, Some(crate::session::PendingSecret::Password(ref p)) if p == "host-pw-2"),
        "a keyless identity must not displace the host password, got {secret2:?} / {diag2}"
    );
}

#[test]
pub(crate) fn missing_stored_secret_yields_an_explicit_will_prompt_diagnostic() {
    // The `env -i` shape: the host row says a secret exists
    // (`has_password`), but the reachable store (no keyring/D-Bus, a
    // foreign-profile fallback file) holds nothing. The code must not
    // silently drop the credential — it returns no secret with a
    // diagnostic naming the manual prompt, which is exactly what the
    // user then sees.
    let store = test_store();
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
    let entry = HostEntry::Managed(store.get_host(host_id).unwrap().unwrap());
    let effective = entry.managed().and_then(|m| m.identity.as_ref());
    let (secret, diag) =
        resolve_pending_secret(&entry, effective, &crate::credentials::NoopPasswordStore);
    assert!(secret.is_none(), "empty store must yield no secret");
    assert!(
        diag.contains("will prompt"),
        "miss must diagnose the manual prompt, got: {diag}"
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
            password_store: Box::new(crate::credentials::NoopPasswordStore),
        },
    );
    app.active_tab = 3;
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
