use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use sshub::app::{App, AppDeps, AppMode};
use sshub::config::AppConfig;
use sshub::metadata::MetadataDb;
use sshub::ssh::{HostResolver, SshHost};
use sshub::store::LauncherStore;
use tempfile::NamedTempFile;

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

fn key_shift_char(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT)
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

/// Fill the snippet form (already open, focused on Name) and save with F2.
fn fill_and_save(app: &mut App, name: &str, command: &str) {
    type_text(app, name);
    app.handle_key(key(KeyCode::Tab)).unwrap(); // → Command
    type_text(app, command);
    app.handle_key(key(KeyCode::F(2))).unwrap(); // save (Enter would just advance)
}

#[test]
fn create_snippet_via_manager_persists() {
    let file = NamedTempFile::new().unwrap();
    let mut app = app_with_store(file.path());

    // Shift+S opens the snippet manager, 'a' opens the new-snippet form.
    app.handle_key(key_shift_char('S')).unwrap();
    assert_eq!(app.mode, AppMode::SnippetManage);
    app.handle_key(key_char('a')).unwrap();
    assert_eq!(app.mode, AppMode::SnippetForm);

    fill_and_save(&mut app, "restart nginx", "sudo systemctl restart nginx");
    // Saving returns to the manager and the snippet is listed.
    assert_eq!(app.mode, AppMode::SnippetManage);
    assert!(app.snippets.iter().any(|s| s.name == "restart nginx"));

    // Persisted to the launcher DB.
    let store = LauncherStore::open(file.path()).unwrap();
    let listed = store.list_snippets().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "restart nginx");
    assert_eq!(listed[0].command, "sudo systemctl restart nginx");
}

#[test]
fn empty_name_or_command_is_rejected() {
    let file = NamedTempFile::new().unwrap();
    let mut app = app_with_store(file.path());

    app.handle_key(key_shift_char('S')).unwrap();
    app.handle_key(key_char('a')).unwrap();
    // Name only, no command → save (F2) is refused, form stays open with notice.
    type_text(&mut app, "incomplete");
    app.handle_key(key(KeyCode::F(2))).unwrap();
    assert_eq!(app.mode, AppMode::SnippetForm);
    assert!(app.snippet_notice.is_some());
    assert!(app.snippets.is_empty());
}

#[test]
fn edit_snippet_updates_command() {
    let file = NamedTempFile::new().unwrap();
    let mut app = app_with_store(file.path());

    app.handle_key(key_shift_char('S')).unwrap();
    app.handle_key(key_char('a')).unwrap();
    fill_and_save(&mut app, "disk", "df -h");
    assert_eq!(app.mode, AppMode::SnippetManage);

    // 'e' edits the selected snippet; clear the Command field and retype.
    app.handle_key(key_char('e')).unwrap();
    assert_eq!(app.mode, AppMode::SnippetForm);
    app.handle_key(key(KeyCode::Tab)).unwrap(); // Name → Command
    app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL))
        .unwrap(); // clear field
    type_text(&mut app, "df -h /");
    app.handle_key(key(KeyCode::F(2))).unwrap(); // save
    assert_eq!(app.mode, AppMode::SnippetManage);

    let store = LauncherStore::open(file.path()).unwrap();
    let listed = store.list_snippets().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].command, "df -h /");
}

#[test]
fn delete_snippet_confirm_and_cancel() {
    let file = NamedTempFile::new().unwrap();
    let mut app = app_with_store(file.path());

    app.handle_key(key_shift_char('S')).unwrap();
    app.handle_key(key_char('a')).unwrap();
    fill_and_save(&mut app, "keep", "echo keep");
    assert_eq!(app.mode, AppMode::SnippetManage);

    // Cancel a delete: snippet survives.
    app.handle_key(key_char('d')).unwrap();
    assert_eq!(app.mode, AppMode::ConfirmDelete);
    app.handle_key(key_char('n')).unwrap();
    assert_eq!(app.mode, AppMode::SnippetManage);
    assert!(app.snippets.iter().any(|s| s.name == "keep"));

    // Confirm a delete: snippet is gone.
    app.handle_key(key_char('d')).unwrap();
    assert_eq!(app.mode, AppMode::ConfirmDelete);
    app.handle_key(key_char('y')).unwrap();
    assert_eq!(app.mode, AppMode::SnippetManage);
    assert!(app.snippets.is_empty());

    let store = LauncherStore::open(file.path()).unwrap();
    assert!(store.list_snippets().unwrap().is_empty());
}

#[test]
fn manager_navigation_clamps() {
    let file = NamedTempFile::new().unwrap();
    let mut app = app_with_store(file.path());

    for (name, cmd) in [("alpha", "a"), ("bravo", "b")] {
        app.handle_key(key_shift_char('S')).unwrap();
        app.handle_key(key_char('a')).unwrap();
        fill_and_save(&mut app, name, cmd);
        app.handle_key(key(KeyCode::Esc)).unwrap(); // back to Normal between adds
    }

    app.handle_key(key_shift_char('S')).unwrap();
    assert_eq!(app.mode, AppMode::SnippetManage);
    // The manager remembers the last-selected snippet; normalize to the top.
    app.handle_key(key_char('k')).unwrap();
    assert_eq!(app.snippet_manage_selected, 0);
    app.handle_key(key_char('j')).unwrap();
    assert_eq!(app.snippet_manage_selected, 1);
    app.handle_key(key_char('j')).unwrap();
    assert_eq!(app.snippet_manage_selected, 1); // clamped at the end
    app.handle_key(key_char('k')).unwrap();
    assert_eq!(app.snippet_manage_selected, 0);
}

#[test]
fn enter_advances_fields_and_saves_on_last() {
    let file = NamedTempFile::new().unwrap();
    let mut app = app_with_store(file.path());

    app.handle_key(key_shift_char('S')).unwrap();
    app.handle_key(key_char('a')).unwrap();
    // Enter walks Name → Command → Description → Tags without saving...
    type_text(&mut app, "tail log");
    app.handle_key(key(KeyCode::Enter)).unwrap(); // → Command
    type_text(&mut app, "tail -f /var/log/syslog");
    app.handle_key(key(KeyCode::Enter)).unwrap(); // → Description
    app.handle_key(key(KeyCode::Enter)).unwrap(); // → Tags
    assert_eq!(
        app.mode,
        AppMode::SnippetForm,
        "still editing, not saved yet"
    );
    // ...and Enter on the last field (Tags) saves.
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.mode, AppMode::SnippetManage);
    assert!(app.snippets.iter().any(|s| s.name == "tail log"));
}

#[test]
fn esc_with_edits_prompts_discard_then_can_save_or_drop() {
    let file = NamedTempFile::new().unwrap();
    let mut app = app_with_store(file.path());

    // Dirty form + Esc → ConfirmDiscard; 'n' (No) drops the edits.
    app.handle_key(key_shift_char('S')).unwrap();
    app.handle_key(key_char('a')).unwrap();
    type_text(&mut app, "throwaway");
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.mode, AppMode::ConfirmDiscard);
    app.handle_key(key_char('n')).unwrap();
    assert_eq!(app.mode, AppMode::SnippetManage);
    assert!(app.snippets.is_empty());

    // Dirty form + Esc → ConfirmDiscard; 'y' (Yes) saves (when valid).
    app.handle_key(key_char('a')).unwrap();
    type_text(&mut app, "keeper");
    app.handle_key(key(KeyCode::Tab)).unwrap(); // → Command
    type_text(&mut app, "echo hi");
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.mode, AppMode::ConfirmDiscard);
    app.handle_key(key_char('y')).unwrap();
    assert_eq!(app.mode, AppMode::SnippetManage);
    assert!(app.snippets.iter().any(|s| s.name == "keeper"));

    // A pristine form (no edits) closes straight back, no prompt.
    app.handle_key(key_char('e')).unwrap();
    assert_eq!(app.mode, AppMode::SnippetForm);
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.mode, AppMode::SnippetManage);
}

#[test]
fn delete_sets_a_visible_notice() {
    let file = NamedTempFile::new().unwrap();
    let mut app = app_with_store(file.path());

    app.handle_key(key_shift_char('S')).unwrap();
    app.handle_key(key_char('a')).unwrap();
    fill_and_save(&mut app, "gone", "echo gone");
    assert_eq!(app.mode, AppMode::SnippetManage);

    app.handle_key(key_char('d')).unwrap();
    app.handle_key(key_char('y')).unwrap();
    assert_eq!(app.mode, AppMode::SnippetManage);
    // The notice survives the return to the manager (set after re-entry).
    assert_eq!(
        app.snippet_notice.as_deref(),
        Some("Snippet 'gone' deleted")
    );
}
