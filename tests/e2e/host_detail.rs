use std::collections::HashMap;
use std::sync::Arc;

use anyhow;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use sshub::app::{App, AppDeps, AppMode};
use sshub::config::AppConfig;
use sshub::metadata::{MetadataDb, MetadataStore};
use sshub::ssh::{HostResolver, SshHost};
use sshub::store::LauncherStore;
use tempfile::NamedTempFile;

#[path = "../support/mod.rs"]
mod support;

use support::MockLauncher;

struct MapResolver {
    hosts: HashMap<String, SshHost>,
    order: Vec<String>,
}

impl MapResolver {
    fn new(entries: Vec<(&str, SshHost)>) -> Self {
        let mut hosts = HashMap::new();
        let mut order = Vec::new();
        for (name, host) in entries {
            order.push(name.to_string());
            hosts.insert(name.to_string(), host);
        }
        Self { hosts, order }
    }
}

impl HostResolver for MapResolver {
    fn list_hosts(&self) -> anyhow::Result<Vec<String>> {
        Ok(self.order.clone())
    }

    fn resolve_host(&self, name: &str) -> anyhow::Result<SshHost> {
        self.hosts
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("unknown host {name}"))
    }
}

fn host(name: &str) -> SshHost {
    let mut h = SshHost::new(name);
    h.hostname = Some(format!("{name}.example.com"));
    h
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
}

fn key_char(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty())
}

fn app_with_persisted_db(db_path: &std::path::Path) -> App {
    let resolver = MapResolver::new(vec![("web", host("web"))]);
    let metadata: Arc<dyn MetadataStore> = Arc::new(MetadataDb::open(db_path).unwrap());
    let mut app = App::new_with_deps(
        AppConfig::default(),
        AppDeps {
            resolver: Box::new(resolver),
            metadata,
            store: Arc::new(LauncherStore::open_in_memory().unwrap()),
            launcher: Box::new(MockLauncher::new()),
            password_store: Box::new(sshub::credentials::NoopPasswordStore),
        },
    );
    app.reload_hosts().unwrap();
    app
}

#[test]
fn host_detail_edit_save_persists_to_db() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path();
    let mut app = app_with_persisted_db(path);

    app.handle_key(key_char('e')).unwrap();
    assert_eq!(app.mode, AppMode::HostDetail);

    for c in "prod, db".chars() {
        app.handle_key(key_char(c)).unwrap();
    }
    app.handle_key(key(KeyCode::Tab)).unwrap();
    for c in "Primary server".chars() {
        app.handle_key(key_char(c)).unwrap();
    }
    app.handle_key(key(KeyCode::Tab)).unwrap();
    for c in "production".chars() {
        app.handle_key(key_char(c)).unwrap();
    }
    app.handle_key(key(KeyCode::Enter)).unwrap();

    assert_eq!(app.mode, AppMode::Normal);
    assert_eq!(app.hosts[0].tags(), &["prod", "db"]);
    assert_eq!(app.hosts[0].description(), Some("Primary server"));
    assert_eq!(app.hosts[0].environment(), Some("production"));

    let db = MetadataDb::open(path).unwrap();
    let loaded = db.get("web").unwrap().expect("saved row");
    assert_eq!(loaded.tags, vec!["prod", "db"]);
    assert_eq!(loaded.description.as_deref(), Some("Primary server"));
    assert_eq!(loaded.environment.as_deref(), Some("production"));
}

#[test]
fn host_detail_esc_cancel_reloads_from_db() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path();
    let mut app = app_with_persisted_db(path);

    if let sshub::app::HostEntry::Legacy { meta, .. } = &mut app.hosts[0] {
        meta.description = Some("stored".into());
        MetadataDb::open(path).unwrap().upsert(meta).unwrap();
    }

    app.handle_key(key_char('e')).unwrap();
    app.handle_key(key_char('x')).unwrap();
    app.handle_key(key(KeyCode::Esc)).unwrap();

    assert_eq!(app.mode, AppMode::Normal);
    assert_eq!(app.hosts[0].description(), Some("stored"));

    let db = MetadataDb::open(path).unwrap();
    assert_eq!(
        db.get("web").unwrap().unwrap().description.as_deref(),
        Some("stored")
    );
}

#[test]
fn favorite_toggle_in_normal_and_host_detail() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path();
    let mut app = app_with_persisted_db(path);

    assert!(!app.hosts[0].favorite());

    app.handle_key(key_char('f')).unwrap();
    assert!(app.hosts[0].favorite());
    let db = MetadataDb::open(path).unwrap();
    assert!(db.get("web").unwrap().unwrap().favorite);

    app.handle_key(key_char('f')).unwrap();
    assert!(!app.hosts[0].favorite());

    app.handle_key(key_char('e')).unwrap();
    app.handle_key(key_char('f')).unwrap();
    assert!(app.hosts[0].favorite());

    let db = MetadataDb::open(path).unwrap();
    assert!(db.get("web").unwrap().unwrap().favorite);
}
