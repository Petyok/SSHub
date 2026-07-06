use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use sshub::app::{App, AppDeps, SortMode};
use sshub::config::AppConfig;
use sshub::metadata::MetadataDb;
use sshub::ssh::{HostResolver, SshHost};
use sshub::store::{HostSource, HostUpdate, LauncherStore, NewHost, NewHostGroup};
use tempfile::NamedTempFile;

#[path = "../support/mod.rs"]
mod support;

use support::MockLauncher;

struct EmptyResolver;

impl HostResolver for EmptyResolver {
    fn list_hosts(&self) -> anyhow::Result<Vec<String>> {
        Ok(vec![])
    }

    fn resolve_host(&self, name: &str) -> anyhow::Result<SshHost> {
        Ok(SshHost::new(name))
    }
}

fn key_char(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty())
}

fn key_ctrl(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::CONTROL)
}

fn app_with_store(store_path: &std::path::Path) -> App {
    let store = Arc::new(LauncherStore::open(store_path).unwrap());
    let mut app = App::new_with_deps(
        AppConfig::default(),
        AppDeps {
            resolver: Box::new(EmptyResolver),
            metadata: Arc::new(MetadataDb::default()),
            store,
            launcher: Box::new(MockLauncher::new()),
            password_store: Box::new(sshub::credentials::NoopPasswordStore),
        },
    );
    app.reload_hosts().unwrap();
    app
}

fn visible_names(app: &App) -> Vec<String> {
    app.filtered_indices
        .iter()
        .map(|&idx| app.hosts[idx].name().to_string())
        .collect()
}

fn seed_sort_hosts(store: &LauncherStore) -> (i64, i64, i64) {
    let default_id = store
        .get_identity_by_name("Default")
        .unwrap()
        .expect("Default identity")
        .id;

    let alpha = store
        .create_host(&NewHost {
            name: "host-alpha".into(),
            label: Some("Alpha Box".into()),
            address: "10.0.0.1".into(),
            port: 22,
            group_id: None,
            identity_id: Some(default_id),
            tags: vec![],
            notes: None,
            ..Default::default()
        })
        .unwrap();
    let zebra = store
        .create_host(&NewHost {
            name: "host-zebra".into(),
            label: Some("Zebra Node".into()),
            address: "10.0.0.2".into(),
            port: 22,
            group_id: None,
            identity_id: Some(default_id),
            tags: vec![],
            notes: None,
            ..Default::default()
        })
        .unwrap();
    let mike = store
        .create_host(&NewHost {
            name: "host-mike".into(),
            label: Some("Mike Server".into()),
            address: "10.0.0.3".into(),
            port: 22,
            group_id: None,
            identity_id: Some(default_id),
            tags: vec![],
            notes: None,
            ..Default::default()
        })
        .unwrap();

    store
        .update_host(
            alpha.id,
            &HostUpdate {
                favorite: Some(true),
                sort_order: Some(20),
                ..Default::default()
            },
        )
        .unwrap();
    store.set_host_last_connected(alpha.id, 100).unwrap();
    store
        .update_host(
            mike.id,
            &HostUpdate {
                sort_order: Some(10),
                ..Default::default()
            },
        )
        .unwrap();
    store.set_host_last_connected(mike.id, 300).unwrap();
    store
        .update_host(
            zebra.id,
            &HostUpdate {
                sort_order: Some(30),
                ..Default::default()
            },
        )
        .unwrap();
    store.set_host_last_connected(zebra.id, 200).unwrap();

    (alpha.id, mike.id, zebra.id)
}

#[test]
fn sort_mode_cycles_and_reorders_hosts() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path();
    let store = LauncherStore::open(path).unwrap();
    seed_sort_hosts(&store);

    let mut app = app_with_store(path);
    assert_eq!(app.sort_mode, SortMode::Label);
    assert_eq!(
        visible_names(&app),
        vec!["host-alpha", "host-mike", "host-zebra"]
    );

    app.handle_key(key_char('s')).unwrap();
    assert_eq!(app.sort_mode, SortMode::LastConnected);
    assert_eq!(
        visible_names(&app),
        vec!["host-mike", "host-zebra", "host-alpha"]
    );

    app.handle_key(key_char('s')).unwrap();
    assert_eq!(app.sort_mode, SortMode::FavoriteFirst);
    assert_eq!(
        visible_names(&app),
        vec!["host-alpha", "host-mike", "host-zebra"]
    );

    app.handle_key(key_char('s')).unwrap();
    assert_eq!(app.sort_mode, SortMode::GroupThenLabel);
    assert_eq!(
        visible_names(&app),
        vec!["host-alpha", "host-mike", "host-zebra"]
    );

    app.handle_key(key_char('s')).unwrap();
    assert_eq!(app.sort_mode, SortMode::Manual);
    assert_eq!(
        visible_names(&app),
        vec!["host-mike", "host-alpha", "host-zebra"]
    );
}

#[test]
fn manual_sort_ctrl_arrows_swap_launcher_hosts() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path();
    let store = LauncherStore::open(path).unwrap();
    seed_sort_hosts(&store);

    let mut app = app_with_store(path);
    for _ in 0..4 {
        app.handle_key(key_char('s')).unwrap();
    }
    assert_eq!(app.sort_mode, SortMode::Manual);
    assert_eq!(
        visible_names(&app),
        vec!["host-mike", "host-alpha", "host-zebra"]
    );

    app.handle_key(key_ctrl(KeyCode::Down)).unwrap();
    assert_eq!(
        visible_names(&app),
        vec!["host-alpha", "host-mike", "host-zebra"]
    );

    app.handle_key(key_ctrl(KeyCode::Up)).unwrap();
    assert_eq!(
        visible_names(&app),
        vec!["host-mike", "host-alpha", "host-zebra"]
    );
}

#[test]
fn group_sections_include_ungrouped_virtual_group() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path();
    let store = LauncherStore::open(path).unwrap();
    let (alpha_id, _, _) = seed_sort_hosts(&store);

    let group = store
        .create_group(&NewHostGroup {
            name: "dev-vcenter".into(),
            sort_order: 0,
            ..Default::default()
        })
        .unwrap();
    store
        .update_host(
            alpha_id,
            &HostUpdate {
                group_id: Some(Some(group.id)),
                ..Default::default()
            },
        )
        .unwrap();

    let app = app_with_store(path);
    let labels: Vec<&str> = app
        .group_sections
        .iter()
        .map(|s| s.label.as_str())
        .collect();
    assert!(labels.contains(&"dev-vcenter"));
    assert!(labels.contains(&"_ungrouped"));

    let ungrouped = app
        .group_sections
        .iter()
        .find(|s| s.label == "_ungrouped")
        .expect("ungrouped section");
    assert_eq!(ungrouped.host_indices.len(), 2);
}

#[test]
fn sidebar_keys_switch_hosts_and_keychain() {
    let file = NamedTempFile::new().unwrap();
    let mut app = app_with_store(file.path());

    app.handle_key(key_char('2')).unwrap();
    assert_eq!(app.active_tab, 1); // tunnels tab

    app.handle_key(key_char('1')).unwrap();
    assert_eq!(app.active_tab, 0); // hosts tab

    app.handle_key(key_char('i')).unwrap();
    assert_eq!(app.active_tab, 2); // keys tab

    app.handle_key(key_char('h')).unwrap();
    assert_eq!(app.active_tab, 0); // hosts tab

    app.handle_key(key_char('3')).unwrap();
    assert_eq!(app.active_tab, 2); // keys tab

    app.handle_key(key_char('4')).unwrap();
    assert_eq!(app.active_tab, 3); // audit tab
}
