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

/// `MapStore` plus a canned Secret Service answer (#142): never touches D-Bus.
struct LookupStore {
    map: MapStore,
    answer: Result<&'static str, &'static str>,
}

impl crate::credentials::PasswordStore for LookupStore {
    fn get(&self, k: &str) -> anyhow::Result<Option<String>> {
        self.map.get(k)
    }
    fn set(&self, k: &str, v: &str) -> anyhow::Result<()> {
        self.map.set(k, v)
    }
    fn delete(&self, k: &str) -> anyhow::Result<()> {
        self.map.delete(k)
    }
    fn lookup(&self, spec: &str) -> anyhow::Result<String> {
        assert_eq!(
            spec, "Title=db",
            "the host's spec reaches the store verbatim"
        );
        self.answer.map(str::to_string).map_err(anyhow::Error::msg)
    }
}

/// A host with a stored SSHub password and a lookup spec.
fn host_with_lookup(
    store: &crate::store::LauncherStore,
    identity_id: Option<i64>,
) -> crate::store::ManagedHost {
    let mut nh = NewHost::launcher("db", "10.0.0.5");
    nh.identity_id = identity_id;
    nh.has_password = true;
    nh.secret_lookup = Some("Title=db".into());
    let id = store.create_host(&nh).unwrap().id;
    store.get_host(id).unwrap().unwrap()
}

fn lookup_store(host_id: i64, answer: Result<&'static str, &'static str>) -> LookupStore {
    let map = MapStore(Mutex::new(HashMap::new()));
    crate::credentials::PasswordStore::set(&map, &crate::credentials::host_key(host_id), "stored")
        .unwrap();
    LookupStore { map, answer }
}

#[test]
pub(crate) fn secret_lookup_is_authoritative_for_the_host_password() {
    let store = test_store();
    let managed = host_with_lookup(&store, None);
    let entry = HostEntry::Managed(managed.clone());

    let found = lookup_store(managed.id, Ok("from-keepass"));
    for (secret, diag) in [
        resolve_pending_secret(&entry, None, &found),
        resolve_pending_secret_for_managed(&managed, &found),
    ] {
        assert!(
            matches!(&secret, Some(crate::session::PendingSecret::Password(p)) if p == "from-keepass"),
            "a found entry is the host password, got {secret:?} / {diag}"
        );
        assert_eq!(diag, "auth: using Secret Service entry Title=db");
    }

    // A failed lookup prompts; it never falls back to the stored password.
    let missing = lookup_store(managed.id, Err("no Secret Service entry has Title=db"));
    for (secret, diag) in [
        resolve_pending_secret(&entry, None, &missing),
        resolve_pending_secret_for_managed(&managed, &missing),
    ] {
        assert!(secret.is_none(), "stored password leaked: {secret:?}");
        assert!(
            diag.contains("secret lookup Title=db failed: no Secret Service entry"),
            "{diag}"
        );
    }
}

#[test]
pub(crate) fn key_passphrase_outranks_a_secret_lookup() {
    let store = test_store();
    let identity = store
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
    let managed = host_with_lookup(&store, Some(identity));
    let pw = lookup_store(managed.id, Ok("from-keepass"));
    crate::credentials::PasswordStore::set(
        &pw,
        &crate::credentials::identity_key(identity),
        "key-phrase",
    )
    .unwrap();
    let entry = HostEntry::Managed(managed);
    let effective = entry.managed().and_then(|m| m.identity.as_ref());
    let (secret, diag) = resolve_pending_secret(&entry, effective, &pw);
    assert!(
        matches!(&secret, Some(crate::session::PendingSecret::Passphrase(p)) if p == "key-phrase"),
        "the key passphrase must win over the lookup, got {secret:?} / {diag}"
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

fn ident(id: i64, name: &str, key: &str) -> crate::store::Identity {
    crate::store::Identity {
        id,
        name: name.into(),
        username: None,
        private_key: Some(key.into()),
        certificate: None,
        has_password: false,
    }
}

fn tunnel(id: i64, label: &str, local: u16) -> crate::store::Tunnel {
    crate::store::Tunnel {
        id,
        host_id: None,
        tunnel_type: crate::store::TunnelType::Local,
        local_port: local,
        remote_host: "localhost".into(),
        remote_port: 80,
        label: Some(label.into()),
        auto_connect: false,
        created_at: 0,
        updated_at: 0,
    }
}

fn type_str(app: &mut App, s: &str) {
    for c in s.chars() {
        app.handle_key(key_char(c)).unwrap();
    }
}

#[test]
pub(crate) fn identities_tab_slash_filters_and_esc_clears() {
    let mut app = test_app(vec![("web", host("web"))]);
    app.active_tab = 3;
    app.identities = vec![
        ident(1, "work", "/k/id_work"),
        ident(2, "home", "/k/id_home"),
        ident(3, "deploy", "/k/id_ci"),
    ];
    app.handle_key(key_char('/')).unwrap();
    assert!(app.list_filter_editing);
    // `d` and `e` are bound keys elsewhere; here they are query text.
    type_str(&mut app, "hom");
    assert_eq!(app.identity_filter, "hom");
    assert_eq!(app.identity_view(), vec![1]);
    assert_eq!(
        app.identity_selected, 1,
        "selection snaps onto a visible card"
    );
    assert!(app.identity_form.is_none());

    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert!(!app.list_filter_editing);
    assert_eq!(app.identity_view(), vec![1], "Enter keeps the filter");

    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert!(app.identity_filter.is_empty());
    assert_eq!(app.active_tab, 3, "first Esc only clears the filter");
    assert_eq!(app.identity_view(), vec![0, 1, 2]);
}

#[test]
pub(crate) fn identity_grid_moves_over_filtered_cards_only() {
    let mut app = test_app(vec![("web", host("web"))]);
    app.active_tab = 3;
    app.config.appearance.identity_columns = 1;
    app.identities = vec![
        ident(1, "aa-one", "/k/1"),
        ident(2, "zz", "/k/2"),
        ident(3, "aa-two", "/k/3"),
    ];
    app.identity_filter = "aa".into();
    app.snap_identity_selection();
    assert_eq!(app.identity_selected, 0);
    app.move_identity_grid(1, 0);
    assert_eq!(app.identity_selected, 2, "skips the hidden card");
}

#[test]
pub(crate) fn tunnels_tab_slash_filters_and_selection_follows_view() {
    let mut app = test_app(vec![("web", host("web"))]);
    app.active_tab = 2;
    app.tunnels = vec![
        tunnel(1, "db", 5432),
        tunnel(2, "web", 8080),
        tunnel(3, "dev web", 3000),
    ];
    app.handle_key(key_char('/')).unwrap();
    type_str(&mut app, "web");
    assert_eq!(app.tunnel_view().len(), 2);
    assert!(!app.tunnel_view().contains(&0));
    assert!(app.tunnel_view().contains(&app.tunnel_selected));
    let first = app.tunnel_selected;
    app.handle_key(key(KeyCode::Down)).unwrap();
    assert_ne!(app.tunnel_selected, first);
    assert_ne!(app.tunnel_selected, 0, "never lands on a hidden tunnel");

    // Ports are searchable too.
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert!(app.tunnel_filter.is_empty());
    type_str(&mut app, "/5432");
    assert_eq!(app.tunnel_view(), vec![0]);
}

#[test]
pub(crate) fn no_match_filter_leaves_tunnel_actions_inert() {
    let mut app = test_app(vec![("web", host("web"))]);
    app.active_tab = 2;
    app.tunnels = vec![tunnel(1, "db", 5432), tunnel(2, "web", 8080)];
    app.handle_key(key_char('/')).unwrap();
    type_str(&mut app, "zzzz");
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert!(app.tunnel_view().is_empty());
    assert!(app.selected_tunnel().is_none());

    app.handle_key(key_char('d')).unwrap();
    assert!(app.pending_delete.is_none(), "delete must not open");
    assert_eq!(app.mode, AppMode::Normal);
    app.handle_key(key_char('e')).unwrap();
    assert!(app.tunnel_form.is_none(), "edit must not open");
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert!(app.tunnel_notice.is_none(), "toggle must not act");
}

#[test]
pub(crate) fn no_match_filter_leaves_identity_actions_inert() {
    let mut app = test_app(vec![("web", host("web"))]);
    app.active_tab = 3;
    app.identities = vec![
        ident(1, "work", "/k/id_work"),
        ident(2, "home", "/k/id_home"),
    ];
    app.handle_key(key_char('/')).unwrap();
    type_str(&mut app, "zzzz");
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert!(app.selected_identity().is_none());

    app.handle_key(key_char('d')).unwrap();
    assert!(app.pending_delete.is_none(), "delete must not open");
    app.handle_key(key_char('e')).unwrap();
    assert!(app.identity_form.is_none(), "edit must not open");
    assert_eq!(app.mode, AppMode::Normal);
}

#[test]
pub(crate) fn group_form_cursor_follows_the_focused_field() {
    let mut app = test_app(vec![]);
    let group = crate::store::HostGroup {
        id: 1,
        name: "spbu-prod".into(),
        sort_order: 0,
        default_identity_id: None,
        default_username: Some("sm-adm".into()),
        default_port: None,
        default_proxy_jump: None,
        default_transport: None,
        default_forward_agent: None,
        parent_id: None,
        reserved: false,
    };
    app.enter_group_form(Some(&group)).unwrap();
    // Name → Parent → Identity → Username: the name's cursor (9) must not
    // carry over to the shorter username, where Backspace used to panic.
    while app.group_form.as_ref().unwrap().field != GroupFormField::Username {
        app.handle_key(key(KeyCode::Down)).unwrap();
    }
    for _ in 0.."sm-adm".len() + 1 {
        app.handle_key(key(KeyCode::Backspace)).unwrap();
    }
    assert_eq!(app.group_form.as_ref().unwrap().default_username, "");
}

fn grp(id: i64, name: &str) -> crate::store::HostGroup {
    crate::store::HostGroup {
        id,
        name: name.into(),
        sort_order: id as i32,
        default_identity_id: None,
        default_username: None,
        default_port: None,
        default_proxy_jump: None,
        default_transport: None,
        default_forward_agent: None,
        parent_id: None,
        reserved: false,
    }
}

#[test]
fn group_parent_picker_filters_as_you_type_and_picks_the_match() {
    let mut app = test_app(vec![]);
    app.groups = vec![grp(1, "alpha"), grp(2, "spbu-prod"), grp(3, "web")];
    app.enter_group_form(None).unwrap();
    app.open_group_field_picker(GroupFormField::Parent);
    type_str(&mut app, "spbu");
    let (none, options) = app.group_field_picker_options();
    assert_eq!(none, "(top level)", "the none slot stays pinned");
    assert_eq!(options, vec![(2, "spbu-prod".to_string())]);
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.mode, AppMode::GroupForm);
    assert_eq!(app.group_form.as_ref().unwrap().parent_id, Some(2));
}

#[test]
fn host_identity_picker_maps_the_filtered_row_to_the_right_identity() {
    let mut app = test_app(vec![]);
    app.identities = vec![
        ident(1, "work", "/k/id_work"),
        ident(2, "home", "/k/id_home"),
        ident(3, "deploy", "/k/id_ci"),
    ];
    app.enter_host_form(None, false).unwrap();
    app.open_field_picker(PickerKind::Identity);
    type_str(&mut app, "dep");
    // "(inherit from group)" stays first; the match is visible row 1, full row 3.
    assert_eq!(app.field_picker_rows(), vec![0, 3]);
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.mode, AppMode::HostForm);
    let index = app.host_form.as_ref().unwrap().identity_index;
    assert_eq!(app.identities[index - 1].id, 3);
}

#[test]
fn host_group_picker_space_toggles_the_filtered_group() {
    let mut app = test_app(vec![]);
    let mut ids = HashMap::new();
    for (i, name) in ["alpha", "spbu-prod", "web"].into_iter().enumerate() {
        let g = app
            .store
            .create_group(&crate::store::NewHostGroup {
                name: name.into(),
                sort_order: i as i32,
                ..Default::default()
            })
            .unwrap();
        ids.insert(name, g.id);
    }
    app.enter_host_form(None, false).unwrap();
    app.open_field_picker(PickerKind::Group);
    type_str(&mut app, "spbu");
    app.handle_key(key_char(' ')).unwrap();
    assert_eq!(
        app.field_picker.as_ref().unwrap().query,
        "spbu",
        "Space is not query text"
    );
    let selected: Vec<i64> = app
        .host_form
        .as_ref()
        .unwrap()
        .group_ids
        .iter()
        .copied()
        .collect();
    assert_eq!(selected, vec![ids["spbu-prod"]]);
}

fn store_group(app: &App, name: &str) -> i64 {
    app.store
        .create_group(&crate::store::NewHostGroup {
            name: name.into(),
            ..Default::default()
        })
        .unwrap()
        .id
}

/// #3: Enter on a group row adds it (it used to only close the dropdown).
#[test]
fn host_group_picker_enter_adds_the_highlighted_group_and_closes() {
    let mut app = test_app(vec![]);
    store_group(&app, "alpha");
    let prod = store_group(&app, "spbu-prod");
    app.enter_host_form(None, false).unwrap();
    app.open_field_picker(PickerKind::Group);
    type_str(&mut app, "spbu");
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.mode, AppMode::HostForm);
    let form = app.host_form.as_ref().unwrap();
    assert_eq!(
        form.group_ids.iter().copied().collect::<Vec<_>>(),
        vec![prod]
    );
    assert!(form.dirty);
}

/// #19: Space opens the Group and Identity dropdowns, like Enter.
#[test]
fn host_form_space_opens_group_and_identity_pickers() {
    let mut app = test_app(vec![]);
    app.enter_host_form(None, false).unwrap();
    for (field, kind) in [
        (HostFormField::Group, PickerKind::Group),
        (HostFormField::Identity, PickerKind::Identity),
    ] {
        app.host_form.as_mut().unwrap().field = field;
        app.handle_key(key_char(' ')).unwrap();
        assert_eq!(app.mode, AppMode::FieldPicker);
        assert_eq!(app.field_picker.as_ref().unwrap().kind, kind);
        app.handle_key(key(KeyCode::Esc)).unwrap();
    }
}

/// #5b/c: Enter moves to the next field (Name no longer saves; Port no longer
/// ignores it) and saves on the last field.
#[test]
fn group_form_enter_advances_and_saves_on_the_last_field() {
    let mut app = test_app(vec![]);
    app.enter_group_form(None).unwrap();
    type_str(&mut app, "ops");
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.mode, AppMode::GroupForm);
    assert_eq!(
        app.group_form.as_ref().unwrap().field,
        GroupFormField::Parent
    );
    app.group_form.as_mut().unwrap().field = GroupFormField::Port;
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(
        app.group_form.as_ref().unwrap().field,
        GroupFormField::ProxyJump
    );
    app.group_form.as_mut().unwrap().field = GroupFormField::ForwardAgent;
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.mode, AppMode::Normal);
    assert!(app.groups.iter().any(|g| g.name == "ops"));
}

/// #5a: Esc on an edited group form asks first; `n` drops the edits.
#[test]
fn group_form_esc_with_edits_asks_before_discarding() {
    let mut app = test_app(vec![]);
    app.enter_group_form(None).unwrap();
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(
        app.mode,
        AppMode::Normal,
        "an untouched form closes at once"
    );

    app.enter_group_form(None).unwrap();
    type_str(&mut app, "x");
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.mode, AppMode::ConfirmDiscard);
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.mode, AppMode::GroupForm, "Esc in the dialog goes back");
    app.handle_key(key(KeyCode::Esc)).unwrap();
    app.handle_key(key_char('n')).unwrap();
    assert_eq!(app.mode, AppMode::Normal);
    assert!(app.group_form.is_none());
    assert!(app.groups.is_empty());
}

/// #5d: validation errors land on the popup's own line (`group_notice`).
#[test]
fn group_form_validation_error_is_shown_in_the_popup() {
    let mut app = test_app(vec![]);
    app.enter_group_form(None).unwrap();
    app.handle_key(key(KeyCode::F(2))).unwrap();
    assert_eq!(app.mode, AppMode::GroupForm);
    assert_eq!(app.group_notice.as_deref(), Some("Group name is required"));
    assert!(app.host_notice.is_none());
}

/// Row index of the header of the section whose group satisfies `pred`.
fn header_row(app: &App, pred: impl Fn(&crate::store::HostGroup) -> bool) -> usize {
    app.nav_rows
        .iter()
        .position(|r| {
            matches!(r, NavRow::Header(si)
                if app.group_sections[*si].group.as_ref().is_some_and(&pred))
        })
        .expect("header row")
}

/// #7a/d: `d` on a group header asks to delete the group; `n` returns to the
/// host list (not the Group manager).
#[test]
fn d_on_a_group_header_deletes_the_group_and_returns_to_the_host_list() {
    let mut app = test_app(vec![]);
    let gid = store_group(&app, "web");
    let mut nh = crate::store::NewHost::launcher("w1", "10.0.0.1");
    nh.group_id = Some(gid);
    app.store.create_host(&nh).unwrap();
    app.reload_hosts().unwrap();
    app.selected = header_row(&app, |g| g.id == gid);

    app.handle_key(key_char('d')).unwrap();
    assert_eq!(app.mode, AppMode::ConfirmDelete);
    assert!(matches!(
        app.pending_delete,
        Some(PendingDelete::Group { id, return_to_manage: false, .. }) if id == gid
    ));
    app.handle_key(key_char('n')).unwrap();
    assert_eq!(app.mode, AppMode::Normal);

    app.handle_key(key_char('d')).unwrap();
    app.handle_key(key_char('y')).unwrap();
    assert_eq!(app.mode, AppMode::Normal);
    assert_eq!(app.host_notice.as_deref(), Some("Group 'web' deleted"));
    assert!(app.groups.is_empty());
}

/// #7b/c: the built-in Favorites header refuses edit and delete with a notice.
#[test]
fn favorites_header_refuses_edit_and_delete_with_a_notice() {
    let mut app = test_app(vec![]);
    let host = app
        .store
        .create_host(&crate::store::NewHost::launcher("w1", "10.0.0.1"))
        .unwrap();
    let fav = app.store.favorites_group_id().unwrap();
    app.store.add_host_to_group(host.id, fav).unwrap();
    app.reload_hosts().unwrap();
    for k in ['e', 'd'] {
        app.selected = header_row(&app, |g| g.reserved);
        app.host_notice = None;
        app.handle_key(key_char(k)).unwrap();
        assert_eq!(app.mode, AppMode::Normal, "{k}");
        assert!(app.pending_delete.is_none(), "{k}");
        assert!(
            app.host_notice
                .as_deref()
                .is_some_and(|n| n.contains("Favorites")),
            "{k}: {:?}",
            app.host_notice
        );
    }
}

/// #14: `/` filters the Group manager; letters are text while typing, and the
/// actions apply to the filtered selection.
#[test]
fn group_manager_slash_filters_and_actions_follow_the_match() {
    let mut app = test_app(vec![]);
    store_group(&app, "alpha");
    let prod = store_group(&app, "beta-prod");
    store_group(&app, "gamma");
    app.enter_group_manage().unwrap();

    app.handle_key(key_char('/')).unwrap();
    type_str(&mut app, "prod");
    assert_eq!(app.mode, AppMode::GroupManage, "letters are filter text");
    let view = app.group_manage_view();
    assert_eq!(view.len(), 1);
    assert_eq!(app.groups[app.group_manage_selected].id, prod);

    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert!(!app.group_manage_filtering);
    app.handle_key(key_char('d')).unwrap();
    assert!(matches!(
        app.pending_delete,
        Some(PendingDelete::Group { id, return_to_manage: true, .. }) if id == prod
    ));
    app.handle_key(key_char('n')).unwrap();
    assert_eq!(app.mode, AppMode::GroupManage);

    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert!(
        app.group_manage_query.is_empty(),
        "first Esc clears the filter"
    );
    assert_eq!(app.mode, AppMode::GroupManage);
    assert_eq!(app.group_manage_view().len(), 3);
}
