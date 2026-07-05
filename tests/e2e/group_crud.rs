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

fn key_shift_char(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT)
}

fn key_ctrl_char(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn key_ctrl(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::CONTROL)
}

fn key_ctrl_shift_char(c: char) -> KeyEvent {
    KeyEvent::new(
        KeyCode::Char(c),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    )
}

/// Open field edit, type text, confirm with Enter.
fn edit_field(app: &mut App, text: &str) {
    // Single-step form model: typing goes straight into the active field.
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
            launcher: Box::new(MockLauncher::new()),
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
fn create_group_assign_host_visible_in_tree() {
    let file = NamedTempFile::new().unwrap();
    let mut app = app_with_store(file.path());

    // Shift+G opens group manage, then 'a' opens new group form
    app.handle_key(key_shift_char('G')).unwrap();
    assert_eq!(app.mode, AppMode::GroupManage);
    app.handle_key(key_char('a')).unwrap();
    assert_eq!(app.mode, AppMode::GroupForm);
    type_text(&mut app, "dev-vcenter");
    app.handle_key(key(KeyCode::Enter)).unwrap();
    // After save, returns to GroupManage
    assert_eq!(app.mode, AppMode::GroupManage);
    assert!(app.groups.iter().any(|g| g.name == "dev-vcenter"));

    // Go back to hosts to create a host
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.mode, AppMode::Normal);

    app.handle_key(key_char('a')).unwrap();
    edit_field(&mut app, "10.0.0.10");
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Password (skip)
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Username (skip)
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Label
    edit_field(&mut app, "VC Host");
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Name
    edit_field(&mut app, "vc-host");
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Port (skip)
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Group
    // Pickers scroll in place with ←/→ — no edit mode needed.
    app.handle_key(key(KeyCode::Right)).unwrap();
    app.handle_key(key(KeyCode::F(2))).unwrap(); // save form

    assert_eq!(app.mode, AppMode::Normal);
    assert_eq!(app.hosts.len(), 1);
    assert!(
        app.group_sections
            .iter()
            .any(|s| s.label == "dev-vcenter" && !s.host_indices.is_empty()),
        "host should appear under dev-vcenter section"
    );

    let store = LauncherStore::open(file.path()).unwrap();
    let host = store
        .get_host_by_name("vc-host")
        .unwrap()
        .expect("persisted");
    let group = store
        .list_groups()
        .unwrap()
        .into_iter()
        .find(|g| g.name == "dev-vcenter")
        .expect("group persisted");
    assert_eq!(host.group_id, Some(group.id));
}

#[test]
fn rename_and_delete_group_via_shortcuts() {
    let file = NamedTempFile::new().unwrap();
    let mut app = app_with_store(file.path());

    // Create group via GroupManage
    app.handle_key(key_shift_char('G')).unwrap();
    app.handle_key(key_char('a')).unwrap();
    type_text(&mut app, "old-name");
    app.handle_key(key(KeyCode::Enter)).unwrap();
    app.handle_key(key(KeyCode::Esc)).unwrap(); // back to Normal

    // Create a host in that group
    app.handle_key(key_char('a')).unwrap();
    edit_field(&mut app, "10.0.0.11");
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Password (skip)
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Username (skip)
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Label (skip)
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Name
    edit_field(&mut app, "g-host");
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Port (skip)
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Group
    app.handle_key(key(KeyCode::Right)).unwrap();
    app.handle_key(key(KeyCode::F(2))).unwrap(); // save form

    // Ctrl+G renames group of selected host
    app.handle_key(key_ctrl_char('G')).unwrap();
    assert_eq!(app.mode, AppMode::GroupForm);
    let len = app.group_form.as_ref().unwrap().name.len();
    for _ in 0..len {
        app.handle_key(key(KeyCode::Backspace)).unwrap();
    }
    type_text(&mut app, "new-name");
    app.handle_key(key(KeyCode::Enter)).unwrap();

    assert!(app.groups.iter().any(|g| g.name == "new-name"));

    // Ctrl+Shift+G deletes group of selected host (with confirm)
    app.handle_key(key_ctrl_shift_char('G')).unwrap();
    assert_eq!(app.mode, AppMode::ConfirmDelete);
    assert!(app.pending_delete.is_some());

    app.handle_key(key_char('y')).unwrap();
    // After confirm, returns to GroupManage
    assert_eq!(app.mode, AppMode::GroupManage);
    assert!(!app.groups.iter().any(|g| g.name == "new-name"));

    // Back to normal to check host
    app.handle_key(key(KeyCode::Esc)).unwrap();
    let host = app.hosts[0].managed().expect("managed");
    assert!(host.group_id.is_none());
}

#[test]
fn delete_group_cancel_preserves_group() {
    let file = NamedTempFile::new().unwrap();
    let mut app = app_with_store(file.path());

    // Create group via GroupManage
    app.handle_key(key_shift_char('G')).unwrap();
    app.handle_key(key_char('a')).unwrap();
    type_text(&mut app, "keep-me");
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert!(app.groups.iter().any(|g| g.name == "keep-me"));
    app.handle_key(key(KeyCode::Esc)).unwrap(); // back to Normal

    // Create a host in that group
    app.handle_key(key_char('a')).unwrap();
    edit_field(&mut app, "10.0.0.20");
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Password (skip)
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Username (skip)
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Label
    edit_field(&mut app, "Test Host");
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Name
    edit_field(&mut app, "t-host");
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Port (skip)
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Group
    app.handle_key(key(KeyCode::Right)).unwrap();
    app.handle_key(key(KeyCode::F(2))).unwrap(); // save form
    assert_eq!(app.mode, AppMode::Normal);

    // Delete via GroupManage screen
    app.handle_key(key_shift_char('G')).unwrap();
    assert_eq!(app.mode, AppMode::GroupManage);
    app.handle_key(key_char('d')).unwrap();
    assert_eq!(app.mode, AppMode::ConfirmDelete);
    app.handle_key(key_char('n')).unwrap();
    assert_eq!(app.mode, AppMode::GroupManage);
    assert!(app.groups.iter().any(|g| g.name == "keep-me"));

    // Cancel with Esc
    app.handle_key(key_char('d')).unwrap();
    assert_eq!(app.mode, AppMode::ConfirmDelete);
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.mode, AppMode::GroupManage);
    assert!(app.groups.iter().any(|g| g.name == "keep-me"));

    // Now actually delete
    app.handle_key(key_char('d')).unwrap();
    assert_eq!(app.mode, AppMode::ConfirmDelete);
    app.handle_key(key_char('y')).unwrap();
    assert_eq!(app.mode, AppMode::GroupManage);
    assert!(!app.groups.iter().any(|g| g.name == "keep-me"));
}

#[test]
fn group_manage_edit_renames_group() {
    let file = NamedTempFile::new().unwrap();
    let mut app = app_with_store(file.path());

    // Create group
    app.handle_key(key_shift_char('G')).unwrap();
    app.handle_key(key_char('a')).unwrap();
    type_text(&mut app, "alpha");
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.mode, AppMode::GroupManage);

    // Edit it
    app.handle_key(key_char('e')).unwrap();
    assert_eq!(app.mode, AppMode::GroupForm);
    let len = app.group_form.as_ref().unwrap().name.len();
    for _ in 0..len {
        app.handle_key(key(KeyCode::Backspace)).unwrap();
    }
    type_text(&mut app, "beta");
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.mode, AppMode::GroupManage);
    assert!(app.groups.iter().any(|g| g.name == "beta"));
    assert!(!app.groups.iter().any(|g| g.name == "alpha"));
}

#[test]
fn group_manage_navigation() {
    let file = NamedTempFile::new().unwrap();
    let mut app = app_with_store(file.path());

    // Create two groups
    app.handle_key(key_shift_char('G')).unwrap();
    app.handle_key(key_char('a')).unwrap();
    type_text(&mut app, "first");
    app.handle_key(key(KeyCode::Enter)).unwrap();
    app.handle_key(key_char('a')).unwrap();
    type_text(&mut app, "second");
    app.handle_key(key(KeyCode::Enter)).unwrap();

    assert_eq!(app.group_manage_selected, 0);
    app.handle_key(key_char('j')).unwrap();
    assert_eq!(app.group_manage_selected, 1);
    app.handle_key(key_char('j')).unwrap();
    assert_eq!(app.group_manage_selected, 1); // clamped
    app.handle_key(key_char('k')).unwrap();
    assert_eq!(app.group_manage_selected, 0);
}
