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
fn host_form_group_dropdown_creates_group_inline() {
    let file = NamedTempFile::new().unwrap();
    let mut app = app_with_store(file.path());

    // New host, fill the required fields.
    app.handle_key(key_char('a')).unwrap();
    edit_field(&mut app, "10.0.0.20"); // Address
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Password
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Username
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Label
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Name
    edit_field(&mut app, "inline-host");
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Port
    app.handle_key(key(KeyCode::Down)).unwrap(); // → Group

    // Enter opens the dropdown; no groups exist yet, so the only rows are
    // "(none)" and "+ New group…".
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.mode, AppMode::FieldPicker);

    // Move to the "+ New group…" row (index groups.len()+1 == 1 here).
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap(); // enter inline create
    type_text(&mut app, "prod");
    app.handle_key(key(KeyCode::Enter)).unwrap(); // create + select

    // Back in the form, group is now selected.
    assert_eq!(app.mode, AppMode::HostForm);
    assert!(app.groups.iter().any(|g| g.name == "prod"));

    app.handle_key(key(KeyCode::F(2))).unwrap(); // save
    assert_eq!(app.mode, AppMode::Normal);

    let store = LauncherStore::open(file.path()).unwrap();
    let host = store
        .get_host_by_name("inline-host")
        .unwrap()
        .expect("persisted");
    let group = store
        .list_groups()
        .unwrap()
        .into_iter()
        .find(|g| g.name == "prod")
        .expect("group created inline");
    assert_eq!(host.group_id, Some(group.id));
}

#[test]
fn tag_filter_hides_groups_with_no_matches() {
    use sshub::store::{NewHost, NewHostGroup};

    let file = NamedTempFile::new().unwrap();
    let store = Arc::new(LauncherStore::open(file.path()).unwrap());

    let prod = store
        .create_group(&NewHostGroup {
            name: "prod".into(),
            sort_order: 0,
            ..Default::default()
        })
        .unwrap();
    let dev = store
        .create_group(&NewHostGroup {
            name: "dev".into(),
            sort_order: 1,
            ..Default::default()
        })
        .unwrap();

    store
        .create_host(&NewHost {
            name: "web1".into(),
            address: "10.0.0.1".into(),
            port: 22,
            group_id: Some(prod.id),
            tags: vec!["eu".into()],
            ..Default::default()
        })
        .unwrap();
    store
        .create_host(&NewHost {
            name: "db1".into(),
            address: "10.0.0.2".into(),
            port: 22,
            group_id: Some(dev.id),
            tags: vec!["us".into()],
            ..Default::default()
        })
        .unwrap();

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

    // No filter: both groups are visible.
    let labels: Vec<&str> = app
        .group_sections
        .iter()
        .map(|s| s.label.as_str())
        .collect();
    assert!(
        labels.contains(&"prod") && labels.contains(&"dev"),
        "both groups shown, got {labels:?}"
    );

    // Filter by "eu": only "prod" contains a match, so "dev" is hidden.
    app.handle_key(key_char('#')).unwrap();
    type_text(&mut app, "eu");
    app.handle_key(key(KeyCode::Enter)).unwrap();

    assert_eq!(app.tag_filters, vec!["eu".to_string()]);
    let labels: Vec<&str> = app
        .group_sections
        .iter()
        .map(|s| s.label.as_str())
        .collect();
    assert!(labels.contains(&"prod"), "prod kept, got {labels:?}");
    assert!(!labels.contains(&"dev"), "dev hidden, got {labels:?}");
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

use sshub::store::{NewHost, NewHostGroup};

/// Create a group with one host, return the app positioned in Normal mode.
fn app_with_grouped_host(path: &std::path::Path) -> App {
    let store = Arc::new(LauncherStore::open(path).unwrap());
    // Idempotent: the test opens the same DB twice to check persistence.
    if store.get_host_by_name("host-a").unwrap().is_none() {
        let gid = store
            .create_group(&NewHostGroup {
                name: "servers".into(),
                sort_order: 0,
                ..Default::default()
            })
            .unwrap()
            .id;
        let mut nh = NewHost::launcher("host-a", "10.0.0.1");
        nh.group_id = Some(gid);
        store.create_host(&nh).unwrap();
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
fn collapsing_a_group_hides_its_hosts_and_persists() {
    let file = NamedTempFile::new().unwrap();
    let mut app = app_with_grouped_host(file.path());

    // Move selection onto the "servers" header.
    while app.selected_nav_header().is_none() {
        app.handle_key(key_char('k')).unwrap();
    }
    assert!(app.selected_host_index().is_none(), "on a header");

    // Collapse with Space: the host row disappears from navigation.
    let before = app.nav_rows.len();
    app.handle_key(key_char(' ')).unwrap();
    assert!(app.nav_rows.len() < before, "host hidden after collapse");
    assert!(
        app.group_sections
            .iter()
            .any(|s| s.label == "servers" && s.collapsed),
        "section marked collapsed"
    );

    // Persisted to launcher.db ui_state.
    let raw = app
        .store()
        .get_ui_state("collapsed_groups")
        .unwrap()
        .expect("persisted");
    assert!(raw.contains(&app.groups[0].id.to_string()));

    // A fresh app over the same DB restores the collapsed state.
    let app2 = app_with_grouped_host(file.path());
    assert!(
        app2.group_sections
            .iter()
            .any(|s| s.label == "servers" && s.collapsed),
        "collapse restored on reload"
    );

    // Expanding with Enter on the header brings the host back.
    let mut app = app;
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert!(
        app.group_sections.iter().all(|s| !s.collapsed),
        "expanded again"
    );
}

#[test]
fn e_on_group_header_edits_group_and_picks_identity_via_dropdown() {
    use crossterm::event::KeyModifiers;
    let save = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);

    let file = NamedTempFile::new().unwrap();
    let mut app = app_with_grouped_host(file.path());

    // Move selection onto the "servers" header.
    while app.selected_nav_header().is_none() {
        app.handle_key(key_char('k')).unwrap();
    }

    // `e` on a header opens the full group edit form.
    app.handle_key(key_char('e')).unwrap();
    assert_eq!(app.mode, AppMode::GroupForm);
    assert!(app.group_form.is_some());

    // Move focus Name → Parent → Identity, then Enter opens the dropdown.
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.mode, AppMode::GroupFieldPicker);

    // Row 0 is "(none)"; pick the first real identity and confirm.
    app.handle_key(key_char('j')).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.mode, AppMode::GroupForm);

    // Save the form (Ctrl+S).
    app.handle_key(save).unwrap();
    assert_eq!(app.mode, AppMode::Normal);

    let first_identity = app.identities[0].id;
    let group = app.groups.iter().find(|g| g.name == "servers").unwrap();
    assert_eq!(group.default_identity_id, Some(first_identity));

    // Persisted: a fresh app over the same DB sees the default identity.
    let app2 = app_with_grouped_host(file.path());
    let group2 = app2.groups.iter().find(|g| g.name == "servers").unwrap();
    assert_eq!(group2.default_identity_id, Some(first_identity));
}
