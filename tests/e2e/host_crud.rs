use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use sshub::app::{App, AppDeps, AppMode};
use sshub::config::AppConfig;
use sshub::metadata::MetadataDb;
use sshub::ssh::{HostResolver, SshHost};
use sshub::store::LauncherStore;
use tempfile::NamedTempFile;

#[path = "../support/mod.rs"]
mod support;

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

fn key_shift_char(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT)
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
fn host_create_edit_delete_roundtrip() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path();
    let mut app = app_with_store(path);
    assert!(app.hosts.is_empty());

    app.handle_key(key_char('a')).unwrap();
    assert_eq!(app.mode, AppMode::HostForm);

    // Address field (already selected)
    edit_field(&mut app, "10.0.0.50");
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Password (skip)
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Username (skip)
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Label
    edit_field(&mut app, "Dev Server");
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Name
    edit_field(&mut app, "dev-server");
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Port
    edit_field_replace(&mut app, 2, "2222");
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Group (skip)
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Identity (skip)
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Tags
    edit_field(&mut app, "dev, staging");
    app.handle_key(key(KeyCode::F(2))).unwrap(); // save form

    assert_eq!(app.mode, AppMode::Normal);
    assert_eq!(app.hosts.len(), 1);
    assert_eq!(app.hosts[0].name(), "dev-server");
    assert_eq!(app.hosts[0].tags(), &["dev", "staging"]);

    let store = LauncherStore::open(path).unwrap();
    let created = store
        .get_host_by_name("dev-server")
        .unwrap()
        .expect("persisted");
    assert_eq!(created.address, "10.0.0.50");
    assert_eq!(created.port, 2222);
    assert_eq!(created.label.as_deref(), Some("Dev Server"));

    app.handle_key(key_char('e')).unwrap();
    assert_eq!(app.mode, AppMode::HostForm);

    app.handle_key(key(KeyCode::Down)).unwrap(); // → Password
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Username
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Label
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Name
    edit_field_replace(&mut app, 10, "dev-server-v2");
    app.handle_key(key(KeyCode::F(2))).unwrap(); // save form

    assert_eq!(app.mode, AppMode::Normal);
    let updated = store
        .get_host_by_name("dev-server-v2")
        .unwrap()
        .expect("updated");
    assert_eq!(updated.address, "10.0.0.50");

    app.handle_key(key_char('d')).unwrap();
    assert_eq!(app.mode, AppMode::ConfirmDelete);
    app.handle_key(key_char('y')).unwrap();
    assert!(store.get_host_by_name("dev-server-v2").unwrap().is_none());
    assert!(app.hosts.is_empty());
}

#[test]
fn host_duplicate_creates_copy() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path();
    let mut app = app_with_store(path);

    app.handle_key(key_char('a')).unwrap();
    edit_field(&mut app, "192.168.1.10");
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Password (skip)
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Username (skip)
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Label (skip)
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Name
    edit_field(&mut app, "app-host");
    app.handle_key(key(KeyCode::F(2))).unwrap(); // save form

    assert_eq!(app.hosts.len(), 1);

    app.handle_key(key_shift_char('D')).unwrap();
    assert_eq!(app.hosts.len(), 2);
    assert!(app.hosts.iter().any(|h| h.name() == "app-host-copy"));

    let store = LauncherStore::open(path).unwrap();
    assert!(store.get_host_by_name("app-host-copy").unwrap().is_some());
}

#[test]
fn adding_host_with_duplicate_name_renames_instead_of_crashing() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path();
    let mut app = app_with_store(path);

    // Add a host named "web".
    let add_web = |app: &mut App| {
        app.handle_key(key_char('a')).unwrap();
        edit_field(app, "10.0.0.1");
        app.handle_key(key(KeyCode::Down)).unwrap(); // → Password
        app.handle_key(key(KeyCode::Down)).unwrap(); // → Username
        app.handle_key(key(KeyCode::Down)).unwrap(); // → Label
        app.handle_key(key(KeyCode::Down)).unwrap(); // → Name
        edit_field(app, "web");
        app.handle_key(key(KeyCode::F(2))).unwrap(); // save
    };

    add_web(&mut app);
    assert_eq!(app.hosts.len(), 1);
    assert!(app.host_notice.is_none());

    // Adding "web" again must NOT bubble a UNIQUE-constraint error (which used
    // to abort the app); it should auto-rename to "web-2" and notify the user.
    add_web(&mut app);
    assert_eq!(app.mode, AppMode::Normal);
    assert_eq!(app.hosts.len(), 2);
    assert!(app.hosts.iter().any(|h| h.name() == "web-2"));
    assert!(app.host_notice.as_deref().unwrap_or("").contains("web-2"));

    let store = LauncherStore::open(path).unwrap();
    assert!(store.get_host_by_name("web-2").unwrap().is_some());
}

#[test]
fn enter_on_last_field_saves_the_form() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path();
    let mut app = app_with_store(path);

    app.handle_key(key_char('a')).unwrap();
    assert_eq!(app.mode, AppMode::HostForm);
    edit_field(&mut app, "10.0.0.7"); // Address
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Password
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Username
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Label
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Name
    edit_field(&mut app, "edge-1");

    // Walk Up to the last field (OS icon): Name → Label → Username → Password
    // → Address → (wrap) OS icon. Enter there should save, not open an editor.
    for _ in 0..5 {
        app.handle_key(key(KeyCode::Up)).unwrap();
    }
    app.handle_key(key(KeyCode::Enter)).unwrap();

    assert_eq!(
        app.mode,
        AppMode::Normal,
        "Enter on last field saves & closes"
    );
    let store = LauncherStore::open(path).unwrap();
    assert!(store.get_host_by_name("edge-1").unwrap().is_some());
}

#[test]
fn esc_reverted_field_edit_leaves_form_clean() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path();
    let mut app = app_with_store(path);

    app.handle_key(key_char('a')).unwrap();
    // Navigate around without typing anything: the form stays clean, so Esc
    // closes silently — no "Save changes?" trap after zero edits.
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Up)).unwrap();
    assert_eq!(app.mode, AppMode::HostForm, "still in the form");

    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.mode, AppMode::Normal);
    assert!(app.hosts.is_empty());
}

#[test]
fn esc_after_committed_field_edit_prompts_for_discard() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path();
    let mut app = app_with_store(path);

    app.handle_key(key_char('a')).unwrap();
    // Typing straight into the active field makes the form dirty.
    type_text(&mut app, "10.9.9.9");

    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.mode, AppMode::ConfirmDiscard);

    // 'n' discards and closes.
    app.handle_key(key_char('n')).unwrap();
    assert_eq!(app.mode, AppMode::Normal);
    assert!(app.hosts.is_empty());
}

#[test]
fn delete_ssh_config_host_shows_notice() {
    use std::collections::HashMap;

    struct MapResolver {
        hosts: HashMap<String, SshHost>,
    }

    impl HostResolver for MapResolver {
        fn list_hosts(&self) -> anyhow::Result<Vec<String>> {
            Ok(self.hosts.keys().cloned().collect())
        }

        fn resolve_host(&self, name: &str) -> anyhow::Result<SshHost> {
            self.hosts
                .get(name)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("unknown {name}"))
        }
    }

    let store = Arc::new(LauncherStore::open_in_memory().unwrap());
    let mut app = App::new_with_deps(
        AppConfig::default(),
        AppDeps {
            resolver: Box::new(MapResolver {
                hosts: HashMap::from([(String::from("legacy"), SshHost::new("legacy"))]),
            }),
            metadata: Arc::new(MetadataDb::default()),
            store,
            password_store: Box::new(sshub::credentials::NoopPasswordStore),
        },
    );
    app.reload_hosts().unwrap();
    assert_eq!(app.hosts.len(), 1);
    assert!(!app.hosts[0].is_launcher());

    app.handle_key(key_char('d')).unwrap();
    assert!(app
        .host_notice
        .as_deref()
        .unwrap_or("")
        .contains("launcher"));
    assert_eq!(app.hosts.len(), 1);
}
