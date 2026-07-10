use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use sshub::app::{App, AppDeps, AppMode};
use sshub::config::AppConfig;
use sshub::metadata::MetadataDb;
use sshub::ssh::{HostResolver, SshHost};
use sshub::store::{LauncherStore, NewHost};
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

fn key_char(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty())
}

fn app_with_hosts(path: &std::path::Path) -> App {
    let store = Arc::new(LauncherStore::open(path).unwrap());
    if store.get_host_by_name("alpha").unwrap().is_none() {
        for (name, addr) in [
            ("alpha", "10.0.0.1"),
            ("bravo", "10.0.0.2"),
            ("charlie", "10.0.0.3"),
        ] {
            store.create_host(&NewHost::launcher(name, addr)).unwrap();
        }
    }

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

#[test]
fn tunnel_host_picker_filters_and_selects() {
    let file = NamedTempFile::new().unwrap();
    let mut app = app_with_hosts(file.path());

    // Go to the tunnels tab and open a new tunnel form.
    app.handle_key(key_char('3')).unwrap();
    app.handle_key(key_char('a')).unwrap();
    assert_eq!(app.mode, AppMode::TunnelForm);

    // The form starts on the SSH server field; Enter opens the picker.
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.mode, AppMode::TunnelHostPicker);
    assert_eq!(app.tunnel_host_matches().len(), 3);

    // Type to filter down to a single host.
    app.handle_key(key_char('c')).unwrap();
    app.handle_key(key_char('h')).unwrap();
    let matches = app.tunnel_host_matches();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].1, "charlie");

    // Enter selects it and returns to the form.
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.mode, AppMode::TunnelForm);
    let charlie_id = app.store().get_host_by_name("charlie").unwrap().unwrap().id;
    assert_eq!(app.tunnel_form.as_ref().unwrap().host_id, Some(charlie_id));
}

#[test]
fn tunnel_host_picker_esc_keeps_previous_selection() {
    let file = NamedTempFile::new().unwrap();
    let mut app = app_with_hosts(file.path());

    app.handle_key(key_char('3')).unwrap();
    app.handle_key(key_char('a')).unwrap();

    // Pick bravo (arrow down once from alpha).
    app.handle_key(key(KeyCode::Enter)).unwrap();
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();
    let bravo_id = app.store().get_host_by_name("bravo").unwrap().unwrap().id;
    assert_eq!(app.tunnel_form.as_ref().unwrap().host_id, Some(bravo_id));

    // Re-open, type a filter, then Esc: selection is unchanged.
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.mode, AppMode::TunnelHostPicker);
    app.handle_key(key_char('a')).unwrap();
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.mode, AppMode::TunnelForm);
    assert_eq!(app.tunnel_form.as_ref().unwrap().host_id, Some(bravo_id));
}
