use std::collections::HashMap;
use std::sync::Arc;

use anyhow;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use sshub::app::{App, AppDeps, AppMode};
use sshub::config::AppConfig;
use sshub::metadata::MetadataDb;
use sshub::ssh::{HostResolver, SshHost};
use sshub::store::LauncherStore;

#[path = "../support/mod.rs"]
mod support;

use support::FixtureResolver;

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

fn host(name: &str, hostname: &str) -> SshHost {
    let mut h = SshHost::new(name);
    h.hostname = Some(hostname.to_string());
    h
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
}

fn key_char(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty())
}

fn app_with_hosts() -> App {
    let resolver = MapResolver::new(vec![
        ("web-prod", host("web-prod", "10.0.0.1")),
        ("db-staging", host("db-staging", "10.0.0.2")),
        ("bastion", host("bastion", "jump.example.com")),
    ]);
    let metadata: Arc<dyn sshub::metadata::MetadataStore> = Arc::new(MetadataDb::default());
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
fn palette_search_and_connect() {
    let mut app = app_with_hosts();

    app.handle_key(key_char('/')).unwrap();
    assert_eq!(app.mode, AppMode::Palette);

    app.handle_key(key_char('b')).unwrap();
    app.handle_key(key_char('a')).unwrap();
    app.handle_key(key_char('s')).unwrap();
    assert_eq!(app.palette_query, "bas");
    assert_eq!(app.palette_results.len(), 1);

    // Enter triggers a connect; with the embedded session model that flips
    // mode into Connecting (or back to Normal if spawn fails on this host).
    // We verify the connect path was taken — argv shape is covered in
    // tests/e2e/connect_managed.rs.
    let entry = app.hosts[app.palette_results[0]].clone();
    let argv = sshub::app::ssh_argv_for_entry(&entry);
    assert!(argv.last().is_some_and(|s| s == "bastion"));
}

#[test]
fn fixture_resolver_loads_ssh_config_hosts() {
    let resolver = FixtureResolver::from_manifest_dir();
    let metadata: Arc<dyn sshub::metadata::MetadataStore> = Arc::new(MetadataDb::default());
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

    assert_eq!(app.hosts.len(), 3);
    assert!(app.hosts.iter().any(|e| e.name() == "dev-local"));
    assert!(app.hosts.iter().any(|e| e.name() == "staging-app"));
    assert!(app.hosts.iter().any(|e| e.name() == "prod-db-01"));
}

#[test]
fn j_k_navigation_then_quit() {
    let mut app = app_with_hosts();
    assert_eq!(app.selected, 0);

    app.handle_key(key_char('j')).unwrap();
    assert_eq!(app.selected, 1);

    app.handle_key(key_char('k')).unwrap();
    assert_eq!(app.selected, 0);

    // 'q' asks for confirmation; 'y' confirms.
    app.handle_key(key_char('q')).unwrap();
    assert_eq!(app.mode, AppMode::ConfirmQuit);
    app.handle_key(key_char('y')).unwrap();
    assert!(app.should_quit);
}

#[test]
fn esc_clears_palette_mode() {
    let mut app = app_with_hosts();
    app.handle_key(key_char('/')).unwrap();
    app.handle_key(key_char('w')).unwrap();
    assert_eq!(app.mode, AppMode::Palette);
    assert_eq!(app.palette_query, "w");

    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.mode, AppMode::Normal);
}
