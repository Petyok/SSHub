use std::sync::Arc;

use anyhow;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use sshub::app::{App, AppDeps, AppMode};
use sshub::config::AppConfig;
use sshub::metadata::MetadataDb;
use sshub::ssh::{HostResolver, SshHost};
use sshub::store::{HostSource, LauncherStore, NewHost};
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

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
}

fn key_ctrl(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::CONTROL)
}

fn key_char(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty())
}

/// Open field edit, type text, confirm with Enter.
fn edit_field(app: &mut App, text: &str) {
    // Single-step form model: typing goes straight into the active field.
    type_text(app, text);
}

/// Open field edit, clear existing text, type new text, confirm.
fn edit_field_replace(app: &mut App, clear_count: usize, text: &str) {
    for _ in 0..clear_count {
        app.handle_key(key(KeyCode::Backspace)).unwrap();
    }
    type_text(app, text);
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

fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        app.handle_key(key_char(c)).unwrap();
    }
}

#[test]
fn keychain_create_edit_delete_unused_identity() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path();
    let mut app = app_with_store(path);

    app.handle_key(key_char('i')).unwrap();
    assert_eq!(app.active_tab, 3); // keys tab
    assert!(app.identities.iter().any(|i| i.name == "Default"));

    app.handle_key(key_char('a')).unwrap();
    assert_eq!(app.mode, AppMode::IdentityForm);

    edit_field(&mut app, "work-laptop");
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Username
    edit_field(&mut app, "deploy");
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Password (skip)
    app.handle_key(key(KeyCode::Down)).unwrap(); // → PrivateKey
    edit_field(&mut app, "~/.ssh/id_ed25519");
    app.handle_key(key(KeyCode::F(2))).unwrap(); // save form

    assert_eq!(app.mode, AppMode::Normal);
    assert!(app
        .identities
        .iter()
        .any(|i| i.name == "work-laptop" && i.username.as_deref() == Some("deploy")));

    let store = LauncherStore::open(path).unwrap();
    let created = store
        .get_identity_by_name("work-laptop")
        .unwrap()
        .expect("persisted identity");
    assert_eq!(
        created
            .private_key
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned()),
        Some("~/.ssh/id_ed25519".to_string())
    );

    app.handle_key(key_char('e')).unwrap();
    assert_eq!(app.mode, AppMode::IdentityForm);

    app.handle_key(key(KeyCode::Down)).unwrap(); // → Username
    edit_field_replace(&mut app, 6, "admin");
    app.handle_key(key(KeyCode::F(2))).unwrap(); // save form

    assert_eq!(app.mode, AppMode::Normal);
    let updated = store
        .get_identity_by_name("work-laptop")
        .unwrap()
        .expect("updated identity");
    assert_eq!(updated.username.as_deref(), Some("admin"));

    app.handle_key(key_char('d')).unwrap();
    assert_eq!(app.mode, AppMode::ConfirmDelete);
    app.handle_key(key_char('y')).unwrap();
    assert!(store.get_identity_by_name("work-laptop").unwrap().is_none());
    assert!(!app.identities.iter().any(|i| i.name == "work-laptop"));
}

#[test]
fn keychain_delete_in_use_identity_shows_notice() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path();
    let store = LauncherStore::open(path).unwrap();
    let identity = store
        .get_identity_by_name("Default")
        .unwrap()
        .expect("default identity");

    store
        .create_host(&NewHost {
            name: "web".into(),
            label: None,
            address: "10.0.0.1".into(),
            port: 22,
            group_id: None,
            identity_id: Some(identity.id),
            tags: vec![],
            notes: None,
            ..Default::default()
        })
        .unwrap();

    let mut app = app_with_store(path);
    app.handle_key(key_char('i')).unwrap();
    app.handle_key(key_char('d')).unwrap();
    assert_eq!(app.mode, AppMode::ConfirmDelete);
    app.handle_key(key_char('y')).unwrap();

    assert!(app
        .identity_notice
        .as_deref()
        .unwrap_or("")
        .contains("used by 1 host"));
    assert!(store.get_identity_by_name("Default").unwrap().is_some());
}
