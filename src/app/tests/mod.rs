use super::*;
use crate::store::{LauncherStore, NewHost};
use std::collections::HashMap;

pub(crate) fn test_store() -> Arc<LauncherStore> {
    Arc::new(LauncherStore::open_in_memory().unwrap())
}

struct MockResolver {
    hosts: HashMap<String, SshHost>,
    order: Vec<String>,
}

impl MockResolver {
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

impl HostResolver for MockResolver {
    fn list_hosts(&self) -> Result<Vec<String>> {
        Ok(self.order.clone())
    }

    fn resolve_host(&self, name: &str) -> Result<SshHost> {
        self.hosts
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("unknown host {name}"))
    }
}

pub(crate) fn test_app(hosts: Vec<(&str, SshHost)>) -> App {
    let resolver = MockResolver::new(hosts);
    let metadata: Arc<dyn MetadataStore> = Arc::new(MetadataDb::default());
    let mut app = App::new_with_deps(
        AppConfig::default(),
        AppDeps {
            resolver: Box::new(resolver),
            metadata,
            store: test_store(),
            password_store: Box::new(crate::credentials::NoopPasswordStore),
        },
    );
    app.reload_hosts().unwrap();
    app
}

pub(crate) fn host(name: &str) -> SshHost {
    let mut h = SshHost::new(name);
    h.hostname = Some(format!("{name}.example.com"));
    h
}

pub(crate) fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
}

pub(crate) fn key_char(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty())
}

pub(crate) fn legacy_meta(entry: &mut HostEntry) -> &mut crate::metadata::HostMetadata {
    entry.legacy_mut().expect("legacy host").1
}

mod host_crud;
mod host_detail;
mod host_form;
mod identity_group;
mod keybind;
mod misc;
mod session;
mod sftp;
mod tags;
mod transport;
