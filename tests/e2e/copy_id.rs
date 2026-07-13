//! E2E: `ssh-copy-id` from the hosts tab.
//!
//! Drives a headless [`App`]. The pure argv-shape assertions live in the
//! colocated unit tests in `src/app/copyid.rs`; here we verify the hosts-tab
//! wiring end-to-end: a host with a bound identity key builds a copy-id session
//! (resilient to `ssh-copy-id` spawning or failing to spawn in CI), and a host
//! without an identity key surfaces the no-key notice instead.

use std::path::PathBuf;
use std::sync::Arc;

use sshub::app::{App, AppDeps};
use sshub::config::AppConfig;
use sshub::metadata::MetadataDb;
use sshub::ssh::{HostResolver, SshHost};
use sshub::store::{LauncherStore, NewHost, NewIdentity};

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

/// Build an app around a store, with a single managed host that optionally has
/// a bound identity carrying a private key.
fn app_with_host(with_key: bool) -> App {
    let store = Arc::new(LauncherStore::open_in_memory().unwrap());

    let identity_id = if with_key {
        let identity = store
            .create_identity(&NewIdentity {
                name: "deploy-key".into(),
                username: Some("deploy".into()),
                private_key: Some(PathBuf::from("/home/me/.ssh/id_ed25519")),
                certificate: None,
                sort_order: 1,
                has_password: false,
            })
            .unwrap();
        Some(identity.id)
    } else {
        None
    };

    store
        .create_host(&NewHost {
            name: "prod-api".into(),
            label: Some("Production API".into()),
            address: "10.20.30.40".into(),
            port: 2222,
            identity_id,
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
    assert_eq!(app.hosts.len(), 1);
    // Move the selection off the group header onto the sole host row.
    for i in 0..32 {
        app.selected = i;
        if app.selected_host_index() == Some(0) {
            break;
        }
    }
    assert_eq!(app.selected_host_index(), Some(0));
    app
}

/// A host with a bound identity key builds a copy-id session: whether or not
/// `ssh-copy-id` is installed on the CI runner, the "no identity key" notice is
/// never raised — the key was found and a safe argv was built.
#[test]
fn copy_id_with_key_does_not_raise_no_key_notice() {
    let mut app = app_with_host(true);

    app.copy_id_selected_host().unwrap();

    assert_ne!(
        app.host_notice.as_deref(),
        Some("host has no identity key to copy"),
    );
    // Either a session tab was spawned (ssh-copy-id present) or a spawn/which
    // failure notice was set — but never the no-key path.
    if let Some(session) = app.sessions.first() {
        assert!(session.display_name.starts_with("copy-id "));
    }
}

/// A managed host without a bound identity has no key to copy, so the action
/// surfaces a notice and spawns nothing.
#[test]
fn copy_id_without_key_sets_notice() {
    let mut app = app_with_host(false);

    app.copy_id_selected_host().unwrap();

    assert_eq!(
        app.host_notice.as_deref(),
        Some("host has no identity key to copy"),
    );
    assert!(app.sessions.is_empty());
    assert!(app.active_session.is_none());
}
