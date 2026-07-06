use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use sshub::app::{App, AppDeps, AppMode};
use sshub::config::AppConfig;
use sshub::metadata::MetadataDb;
use sshub::store::{HostSource, LauncherStore, NewHost};
use tempfile::NamedTempFile;

#[path = "../support/mod.rs"]
mod support;

use support::{FixtureResolver, MockLauncher};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
}

fn key_ctrl(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::CONTROL)
}

fn key_char(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty())
}

fn key_shift_char(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT)
}

fn hybrid_env() -> (NamedTempFile, App) {
    let file = NamedTempFile::new().unwrap();
    let store = Arc::new(LauncherStore::open(file.path()).unwrap());
    let mut app = App::new_with_deps(
        AppConfig::default(),
        AppDeps {
            resolver: Box::new(FixtureResolver::from_manifest_dir()),
            metadata: Arc::new(MetadataDb::default()),
            store,
            launcher: Box::new(MockLauncher::new()),
            password_store: Box::new(sshub::credentials::NoopPasswordStore),
        },
    );
    app.reload_hosts().unwrap();
    (file, app)
}

#[test]
fn edit_ssh_config_managed_host_opens_metadata_only_form() {
    let (_file, mut app) = hybrid_env();
    app.import_ssh_config().unwrap();
    app.reload_hosts().unwrap();

    let idx = app
        .hosts
        .iter()
        .position(|h| h.name() == "dev-local")
        .expect("imported host");
    app.selected = app
        .filtered_indices
        .iter()
        .position(|&i| i == idx)
        .unwrap_or(0);

    app.handle_key(key_char('e')).unwrap();
    assert_eq!(app.mode, AppMode::HostForm);
    let form = app.host_form.as_ref().expect("form open");
    assert!(form.metadata_only);
    assert_eq!(form.address, "localhost");
    assert_eq!(form.name, "dev-local");

    // metadata_only form starts on Label; navigate to Tags
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Name (skip, read-only)
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Port (skip, read-only)
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Group
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Identity
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Tags
    type_text(&mut app, "imported");
    app.handle_key(key(KeyCode::F(2))).unwrap(); // save form

    assert_eq!(app.mode, AppMode::Normal);
    let updated = app
        .store()
        .get_host_by_name("dev-local")
        .unwrap()
        .expect("host");
    assert_eq!(updated.source, HostSource::SshConfig);
    assert_eq!(updated.address, "localhost");
    assert!(updated.tags.iter().any(|t| t == "imported"));
}

#[test]
fn duplicate_legacy_ssh_config_host_creates_launcher_copy() {
    let (_file, mut app) = hybrid_env();
    assert_eq!(app.hosts.len(), 3);
    assert!(app.hosts.iter().all(|h| h.managed().is_none()));

    let idx = app
        .hosts
        .iter()
        .position(|h| h.name() == "staging-app")
        .unwrap();
    app.selected = app.filtered_indices.iter().position(|&i| i == idx).unwrap();

    app.handle_key(key_shift_char('D')).unwrap();
    assert_eq!(app.hosts.len(), 4);
    assert!(app.hosts.iter().any(|h| h.name() == "staging-app-copy"));

    let copy = app
        .store()
        .get_host_by_name("staging-app-copy")
        .unwrap()
        .expect("launcher copy");
    assert_eq!(copy.source, HostSource::Launcher);
    assert_eq!(copy.address, "10.0.2.10");
    let identity = copy.identity.expect("identity from resolver User");
    assert_eq!(identity.username.as_deref(), Some("deploy"));
}

#[test]
fn duplicate_imported_ssh_config_host_creates_launcher_copy() {
    let (_file, mut app) = hybrid_env();
    app.import_ssh_config().unwrap();
    app.reload_hosts().unwrap();

    let idx = app
        .hosts
        .iter()
        .position(|h| h.name() == "prod-db-01")
        .unwrap();
    app.selected = app.filtered_indices.iter().position(|&i| i == idx).unwrap();

    app.handle_key(key_shift_char('D')).unwrap();
    assert!(app.hosts.iter().any(|h| h.name() == "prod-db-01-copy"));

    let copy = app
        .store()
        .get_host_by_name("prod-db-01-copy")
        .unwrap()
        .expect("launcher copy");
    assert_eq!(copy.source, HostSource::Launcher);
    assert_eq!(copy.address, "10.0.1.5");
}

#[test]
fn reload_merges_launcher_imported_ssh_config_and_resolver_without_duplicates() {
    let (_file, mut app) = hybrid_env();
    let default_id = app
        .store()
        .get_identity_by_name("Default")
        .unwrap()
        .unwrap()
        .id;

    app.store()
        .create_host(&NewHost {
            name: "dev-local".into(),
            label: Some("Launcher override".into()),
            address: "192.168.1.1".into(),
            port: 2222,
            group_id: None,
            identity_id: Some(default_id),
            tags: vec![],
            notes: None,
            ..Default::default()
        })
        .unwrap();

    app.import_ssh_config().unwrap();
    app.reload_hosts().unwrap();

    let names: Vec<_> = app.hosts.iter().map(|h| h.name().to_string()).collect();
    assert_eq!(names.len(), 3, "expected no duplicate names: {names:?}");
    assert_eq!(
        names.iter().filter(|n| n.as_str() == "dev-local").count(),
        1
    );

    let dev = app
        .store()
        .get_host_by_name("dev-local")
        .unwrap()
        .expect("dev-local");
    assert_eq!(dev.source, HostSource::Launcher);
    assert_eq!(dev.address, "192.168.1.1");
}

fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        app.handle_key(key_char(c)).unwrap();
    }
}

#[test]
fn editing_unimported_ssh_config_alias_allows_group_assignment() {
    // Fresh env: ssh_config hosts appear as live-resolver Legacy entries
    // (never imported into launcher.db). This mirrors a `monitor`-style host.
    let (file, mut app) = hybrid_env();
    let idx = app
        .hosts
        .iter()
        .position(|h| h.name() == "dev-local")
        .expect("dev-local present");
    app.selected = app
        .filtered_indices
        .iter()
        .position(|&i| i == idx)
        .expect("in filter");

    // Editing materializes it into launcher.db and opens the full metadata
    // form (not the tags-only HostDetail), so a group can be assigned.
    app.handle_key(key_char('e')).unwrap();
    assert_eq!(app.mode, AppMode::HostForm);
    assert!(app.host_form.as_ref().unwrap().metadata_only);

    let dev = app
        .store()
        .get_host_by_name("dev-local")
        .unwrap()
        .expect("materialized into launcher.db");
    assert_eq!(dev.source, HostSource::SshConfig);

    // Navigate to Group and create one inline via the dropdown.
    use sshub::app::HostFormField;
    while app.host_form.as_ref().unwrap().field != HostFormField::Group {
        app.handle_key(key(KeyCode::Down)).unwrap();
    }
    app.handle_key(key(KeyCode::Enter)).unwrap(); // open dropdown
    assert_eq!(app.mode, AppMode::FieldPicker);
    app.handle_key(key(KeyCode::Down)).unwrap(); // → "+ New group…"
    app.handle_key(key(KeyCode::Enter)).unwrap(); // inline create
    type_text(&mut app, "infra");
    app.handle_key(key(KeyCode::Enter)).unwrap(); // create + select
    app.handle_key(key(KeyCode::F(2))).unwrap(); // save

    let store = LauncherStore::open(file.path()).unwrap();
    let dev = store.get_host_by_name("dev-local").unwrap().unwrap();
    let group = store
        .list_groups()
        .unwrap()
        .into_iter()
        .find(|g| g.name == "infra")
        .expect("group created");
    assert_eq!(dev.group_id, Some(group.id));
}
