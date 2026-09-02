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

/// An in-memory credential store, so tests can exercise the paths that read a
/// secret back: `NoopPasswordStore` answers `None` to everything, which makes
/// prefilling and "was it changed" untestable.
#[derive(Default)]
pub(crate) struct MemoryPasswordStore {
    entries: std::sync::Mutex<HashMap<String, String>>,
}

impl crate::credentials::PasswordStore for MemoryPasswordStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.entries.lock().unwrap().get(key).cloned())
    }
    fn set(&self, key: &str, password: &str) -> Result<()> {
        self.entries
            .lock()
            .unwrap()
            .insert(key.to_string(), password.to_string());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.entries.lock().unwrap().remove(key);
        Ok(())
    }
}

/// `test_app`, but with a credential store that actually remembers. Returns the
/// app and a handle to the same store so a test can look inside it.
pub(crate) fn test_app_with_secrets(
    hosts: Vec<(&str, SshHost)>,
) -> (App, Arc<MemoryPasswordStore>) {
    let secrets = Arc::new(MemoryPasswordStore::default());
    let resolver = MockResolver::new(hosts);
    let metadata: Arc<dyn MetadataStore> = Arc::new(MetadataDb::default());
    let mut app = App::new_with_deps(
        AppConfig::default(),
        AppDeps {
            resolver: Box::new(resolver),
            metadata,
            store: test_store(),
            password_store: Box::new(SharedStore(secrets.clone())),
        },
    );
    app.reload_hosts().unwrap();
    (app, secrets)
}

/// Lets the test keep a handle on the store the app owns.
struct SharedStore(Arc<MemoryPasswordStore>);

impl crate::credentials::PasswordStore for SharedStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        self.0.get(key)
    }
    fn set(&self, key: &str, password: &str) -> Result<()> {
        self.0.set(key, password)
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.0.delete(key)
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

mod broadcast;
mod host_crud;
mod host_detail;
mod host_form;
mod identity_group;
mod keybind;
mod log_browser;
mod misc;
mod session;
mod sftp;
mod snippets;
mod tags;
mod theme_picker;
mod transport;
