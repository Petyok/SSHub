use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use sshub::app::{App, AppDeps, AppMode};
use sshub::config::AppConfig;
use sshub::metadata::MetadataDb;
use sshub::ssh::{HostResolver, SshHost};
use sshub::store::LauncherStore;

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

fn new_app(store: Arc<LauncherStore>) -> App {
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

fn type_path(app: &mut App, path: &str) {
    for c in path.chars() {
        app.handle_key(key_char(c)).unwrap();
    }
}

#[test]
fn shift_t_opens_prompt_and_imports_csv_export() {
    let export = tempfile::tempdir().unwrap();
    std::fs::write(
        export.path().join("L00t.csv"),
        "Label,Host,Port,Username,Password,SSH_Key,OS\n\
         web,10.0.0.1,22,admin,,,ubuntu\n\
         db,10.0.0.2,5432,root,,,\n",
    )
    .unwrap();

    let store = Arc::new(LauncherStore::open_in_memory().unwrap());
    let mut app = new_app(Arc::clone(&store));

    // Shift+T opens the import prompt.
    app.handle_key(KeyEvent::new(KeyCode::Char('T'), KeyModifiers::SHIFT))
        .unwrap();
    assert_eq!(app.mode, AppMode::ImportPrompt);

    // The prompt may prefill a guessed path; clear it before typing ours.
    while app
        .import_prompt
        .as_ref()
        .is_some_and(|p| !p.path.is_empty())
    {
        app.handle_key(key(KeyCode::Backspace)).unwrap();
    }
    type_path(&mut app, &export.path().display().to_string());

    // Enter runs the import and returns to Normal.
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.mode, AppMode::Normal);
    assert!(app.import_prompt.is_none());

    let web = store.get_host_by_name("web").unwrap().unwrap();
    assert_eq!(web.address, "10.0.0.1");
    assert_eq!(web.username.as_deref(), Some("admin"));
    let db = store.get_host_by_name("db").unwrap().unwrap();
    assert_eq!(db.port, 5432);

    assert!(app.hosts.iter().any(|h| h.name() == "web"));
    assert!(app.hosts.iter().any(|h| h.name() == "db"));
}

#[test]
fn import_accepts_path_to_loot_csv_directly() {
    let export = tempfile::tempdir().unwrap();
    let csv = export.path().join("L00t.csv");
    std::fs::write(
        &csv,
        "Label,Host,Port,Username,Password,SSH_Key,OS\nweb,10.0.0.1,22,admin,,,\n",
    )
    .unwrap();

    let store = Arc::new(LauncherStore::open_in_memory().unwrap());
    let mut app = new_app(Arc::clone(&store));

    app.handle_key(KeyEvent::new(KeyCode::Char('T'), KeyModifiers::SHIFT))
        .unwrap();
    while app
        .import_prompt
        .as_ref()
        .is_some_and(|p| !p.path.is_empty())
    {
        app.handle_key(key(KeyCode::Backspace)).unwrap();
    }
    // Point at the CSV file itself, not the folder.
    type_path(&mut app, &csv.display().to_string());
    app.handle_key(key(KeyCode::Enter)).unwrap();

    assert_eq!(app.mode, AppMode::Normal);
    assert!(store.get_host_by_name("web").unwrap().is_some());
}

#[test]
fn esc_cancels_import_prompt() {
    let store = Arc::new(LauncherStore::open_in_memory().unwrap());
    let mut app = new_app(store);

    app.handle_key(KeyEvent::new(KeyCode::Char('T'), KeyModifiers::SHIFT))
        .unwrap();
    assert_eq!(app.mode, AppMode::ImportPrompt);

    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.mode, AppMode::Normal);
    assert!(app.import_prompt.is_none());
}

/// In-memory password store that actually persists within the test (unlike
/// `NoopPasswordStore`), so we can assert the full import → resolve path.
#[derive(Default)]
struct MapPasswordStore {
    map: std::sync::Mutex<std::collections::HashMap<String, String>>,
}

impl sshub::credentials::PasswordStore for MapPasswordStore {
    fn get(&self, key: &str) -> anyhow::Result<Option<String>> {
        Ok(self.map.lock().unwrap().get(key).cloned())
    }
    fn set(&self, key: &str, password: &str) -> anyhow::Result<()> {
        self.map
            .lock()
            .unwrap()
            .insert(key.to_string(), password.to_string());
        Ok(())
    }
    fn delete(&self, key: &str) -> anyhow::Result<()> {
        self.map.lock().unwrap().remove(key);
        Ok(())
    }
}

#[test]
fn imported_host_password_is_resolved_at_connect_time() {
    // Mirrors a real Termius export byte-for-byte: UTF-8 BOM, CRLF line
    // endings, every field double-quoted.
    let export = tempfile::tempdir().unwrap();
    let real =
        "\u{feff}\"Label\",\"Host\",\"Port\",\"Username\",\"Password\",\"SSH_Key\",\"OS\"\r\n\
                \"dev-alumni\",\"10.100.19.83\",\"22\",\"su-adm\",\"StrongPassw0rd\",\"\",\"\"\r\n";
    std::fs::write(export.path().join("L00t.csv"), real).unwrap();

    let store = Arc::new(LauncherStore::open_in_memory().unwrap());
    let pw = MapPasswordStore::default();

    let report = sshub::import::termius_csv::import_csv_export(export.path(), &store, &pw).unwrap();
    assert_eq!(report.hosts_imported, 1);
    assert_eq!(report.passwords_stored, 1, "password must be stored");
    assert_eq!(report.keyring_failures, 0);

    // The host took its name from the Label and has_password set.
    let host = store.get_host_by_name("dev-alumni").unwrap().unwrap();
    assert_eq!(host.address, "10.100.19.83");
    assert_eq!(host.username.as_deref(), Some("su-adm"));
    assert!(
        host.has_password,
        "row must be flagged as having a password"
    );

    // The connect path must resolve the stored password (not fall back to a
    // manual ssh prompt).
    let mut app = new_app(Arc::clone(&store));
    let entry = app
        .hosts
        .iter()
        .find(|h| h.name() == "dev-alumni")
        .cloned()
        .expect("imported host present after reload");
    let (secret, diag) = sshub::app::resolve_pending_secret(&entry, &pw);
    match secret {
        Some(sshub::session::PendingSecret::Password(p)) => assert_eq!(p, "StrongPassw0rd"),
        other => panic!("expected a resolved password, got {other:?} ({diag})"),
    }
    let _ = &mut app;
}

#[test]
fn import_prompt_bad_path_keeps_prompt_open_with_notice() {
    let store = Arc::new(LauncherStore::open_in_memory().unwrap());
    let mut app = new_app(store);

    app.handle_key(KeyEvent::new(KeyCode::Char('T'), KeyModifiers::SHIFT))
        .unwrap();
    while app
        .import_prompt
        .as_ref()
        .is_some_and(|p| !p.path.is_empty())
    {
        app.handle_key(key(KeyCode::Backspace)).unwrap();
    }
    type_path(&mut app, "/no/such/termius/export");
    app.handle_key(key(KeyCode::Enter)).unwrap();

    // Stays on the prompt and shows the error inside the popup.
    assert_eq!(app.mode, AppMode::ImportPrompt);
    let prompt = app.import_prompt.as_ref().unwrap();
    assert!(prompt
        .error
        .as_deref()
        .unwrap_or("")
        .contains("L00t.csv not found"));
}
