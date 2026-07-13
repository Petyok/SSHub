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

fn ssh_keygen_available() -> bool {
    std::process::Command::new("ssh-keygen")
        .arg("-?")
        .output()
        .map(|o| o.status.code().is_some())
        .unwrap_or(false)
}

fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        app.handle_key(key_char(c)).unwrap();
    }
}

#[test]
fn keygen_generates_and_registers_identity() {
    if !ssh_keygen_available() {
        eprintln!("skipping: ssh-keygen not available");
        return;
    }

    // Isolate the generated key files under a throwaway data directory.
    let data = tempfile::tempdir().unwrap();
    std::env::set_var("SSHUB_DATA_DIR", data.path());

    let file = NamedTempFile::new().unwrap();
    let mut app = app_with_store(file.path());

    // Go to the identities (keys) tab and open the keygen form.
    app.handle_key(key_char('i')).unwrap();
    assert_eq!(app.active_tab, 3);
    app.handle_key(key_char('g')).unwrap();
    assert_eq!(app.mode, AppMode::KeygenForm);

    // Type a name (no passphrase) and save.
    type_text(&mut app, "e2e-generated");
    app.handle_key(key(KeyCode::F(2))).unwrap();

    assert_eq!(app.mode, AppMode::Normal);
    let created = app
        .identities
        .iter()
        .find(|i| i.name == "e2e-generated")
        .expect("generated identity registered");
    let key_path = created
        .private_key
        .as_ref()
        .expect("generated identity has a private key path");
    assert!(
        key_path.exists(),
        "private key file should exist at {}",
        key_path.display()
    );
    assert!(key_path.with_extension("pub").exists(), "public key present");

    std::env::remove_var("SSHUB_DATA_DIR");
}
