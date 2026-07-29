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
            password_store: Box::new(sshub::credentials::NoopPasswordStore),
        },
    );
    app.reload_hosts().unwrap();
    app
}

#[test]
fn editing_ssh_config_alias_materializes_and_persists_tags() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path();
    let mut app = app_with_persisted_db(path);

    // `web` is a live ssh_config alias (not in launcher.db). Editing it
    // materializes it into launcher.db and opens the metadata form, where
    // tags/group/identity are editable and persisted to the launcher store.
    app.handle_key(key_char('e')).unwrap();
    assert_eq!(app.mode, AppMode::HostForm);
    assert!(app.host_form.as_ref().unwrap().metadata_only);

    // Navigate to the Tags field and type.
    use sshub::app::HostFormField;
    while app.host_form.as_ref().unwrap().field != HostFormField::Tags {
        app.handle_key(key(KeyCode::Down)).unwrap();
    }
    for c in "prod, db".chars() {
        app.handle_key(key_char(c)).unwrap();
    }
    app.handle_key(key(KeyCode::F(2))).unwrap();

    assert_eq!(app.mode, AppMode::Normal);
    assert_eq!(app.hosts[0].tags(), &["prod", "db"]);

    let store = app.store();
    let row = store
        .get_host_by_name("web")
        .unwrap()
        .expect("materialized");
    assert_eq!(row.source, sshub::store::HostSource::SshConfig);
    assert_eq!(row.tags, vec!["prod", "db"]);
}

#[test]
fn editing_alias_preserves_existing_metadata_on_materialize() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path();
    let mut app = app_with_persisted_db(path);

    // Pre-existing legacy metadata (from the MVP metadata.db) must carry over
    // when the alias is materialized into launcher.db.
    if let sshub::app::HostEntry::Legacy { meta, .. } = &mut app.hosts[0] {
        meta.description = Some("stored".into());
        meta.environment = Some("prod".into());
        MetadataDb::open(path).unwrap().upsert(meta).unwrap();
    }
    app.reload_hosts().unwrap();

    app.handle_key(key_char('e')).unwrap(); // materialize + open form
    assert_eq!(app.mode, AppMode::HostForm);
    app.handle_key(key(KeyCode::Esc)).unwrap(); // no changes → close cleanly
    assert_eq!(app.mode, AppMode::Normal);

    let row = app
        .store()
        .get_host_by_name("web")
        .unwrap()
        .expect("materialized");
    assert_eq!(row.notes.as_deref(), Some("stored"));
    assert_eq!(row.environment.as_deref(), Some("prod"));
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
    let db = MetadataDb::open(path).unwrap();
    assert!(!db.get("web").unwrap().unwrap().favorite);
}
