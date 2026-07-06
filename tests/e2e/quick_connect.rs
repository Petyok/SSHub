use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use sshub::app::{App, AppDeps};
use sshub::config::AppConfig;
use sshub::metadata::MetadataDb;
use sshub::ssh::{HostResolver, SshHost};
use sshub::store::{LauncherStore, NewHost, NewHostGroup};

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

/// Two groups, each with two managed hosts. Returns the app plus the host id of
/// "delta" (the second host of the second group).
fn app_with_grouped_hosts() -> App {
    let store = Arc::new(LauncherStore::open_in_memory().unwrap());
    let g1 = store
        .create_group(&NewHostGroup {
            name: "alpha-grp".into(),
            sort_order: 0,
            ..Default::default()
        })
        .unwrap()
        .id;
    let g2 = store
        .create_group(&NewHostGroup {
            name: "bravo-grp".into(),
            sort_order: 1,
            ..Default::default()
        })
        .unwrap()
        .id;
    for (name, gid) in [
        ("aa-one", g1),
        ("aa-two", g1),
        ("bb-one", g2),
        ("delta", g2),
    ] {
        let mut nh = NewHost::launcher(name, "10.0.0.9");
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

/// Regression: in tree mode (groups present), `nav_rows` interleaves group
/// headers with hosts, so a `filtered_indices` position is NOT a valid
/// `nav_rows` index. quick-connect used to land on the wrong row; `reveal_host`
/// must select the exact host chosen.
#[test]
fn reveal_host_selects_correct_host_across_groups() {
    let mut app = app_with_grouped_hosts();

    let target = app.hosts.iter().position(|h| h.name() == "delta").unwrap();
    assert!(app.reveal_host(target));
    assert_eq!(app.selected_host_index(), Some(target));

    // And a host in the first group resolves too (guards the interleave bug).
    let other = app.hosts.iter().position(|h| h.name() == "aa-one").unwrap();
    assert!(app.reveal_host(other));
    assert_eq!(app.selected_host_index(), Some(other));
}

/// quick-connect must be able to reach a host hidden inside a collapsed group:
/// `reveal_host` expands the group and selects the host.
#[test]
fn reveal_host_expands_collapsed_group() {
    let mut app = app_with_grouped_hosts();

    // Collapse every group (Shift+Z toggles collapse-all).
    app.handle_key(KeyEvent::new(KeyCode::Char('Z'), KeyModifiers::SHIFT))
        .unwrap();
    let target = app.hosts.iter().position(|h| h.name() == "delta").unwrap();
    // While collapsed, the host has no navigable row.
    assert_eq!(app.selected_host_index(), None);

    assert!(app.reveal_host(target));
    assert_eq!(app.selected_host_index(), Some(target));
}
