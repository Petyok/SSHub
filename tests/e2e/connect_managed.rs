use std::path::PathBuf;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use sshub::app::{App, AppDeps};
use sshub::config::AppConfig;
use sshub::metadata::MetadataDb;
use sshub::ssh::{HostResolver, SshHost};
use sshub::store::{HostSource, LauncherStore, NewHost, NewIdentity, SshConfigHostImport};
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

fn app_with_managed_host(store_path: &std::path::Path) -> (App, MockLauncher) {
    let store = Arc::new(LauncherStore::open(store_path).unwrap());
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

    let host = store
        .create_host(&NewHost {
            name: "prod-api".into(),
            label: Some("Production API".into()),
            address: "10.20.30.40".into(),
            port: 2222,
            group_id: None,
            identity_id: Some(identity.id),
            tags: vec![],
            notes: None,
            ..Default::default()
        })
        .unwrap();

    {
        let conn = rusqlite::Connection::open(store_path).unwrap();
        conn.execute(
            "UPDATE hosts SET proxy_jump = ?1, forward_agent = ?2 WHERE id = ?3",
            rusqlite::params!["bastion.example.com", 1_i64, host.id],
        )
        .unwrap();
    }

    let launcher = MockLauncher::new();
    let app_launcher = launcher.clone();
    let mut app = App::new_with_deps(
        AppConfig::default(),
        AppDeps {
            resolver: Box::new(EmptyResolver),
            metadata: Arc::new(MetadataDb::default()),
            store,
            launcher: Box::new(app_launcher),
            password_store: Box::new(sshub::credentials::NoopPasswordStore),
        },
    );
    app.reload_hosts().unwrap();
    assert_eq!(app.hosts.len(), 1);
    assert!(app.hosts[0].is_launcher());
    (app, launcher)
}

#[test]
fn connect_managed_host_builds_ssh_argv_and_updates_last_connected() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path();
    let (app, _launcher) = app_with_managed_host(path);

    // Argv shape: full options inlined since this host is launcher-managed
    // (not derived from ~/.ssh/config).
    let entry = app.hosts[0].clone();
    let ssh_argv = sshub::app::ssh_argv_for_entry(&entry);
    assert_eq!(
        ssh_argv,
        vec![
            "ssh".to_string(),
            "-p".to_string(),
            "2222".to_string(),
            "-i".to_string(),
            "/home/me/.ssh/id_ed25519".to_string(),
            "-J".to_string(),
            "bastion.example.com".to_string(),
            "-o".to_string(),
            "ForwardAgent=yes".to_string(),
            "deploy@10.20.30.40".to_string(),
        ]
    );
}

#[test]
fn connect_ssh_config_host_uses_alias_mode() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path();
    let store = Arc::new(LauncherStore::open(path).unwrap());

    store
        .upsert_ssh_config_host(&SshConfigHostImport {
            name: "web-prod".into(),
            address: "10.0.0.5".into(),
            port: 22,
            proxy_jump: None,
            forward_agent: false,
            remote_command: None,
            ssh_config_hash: "abc123".into(),
            tags: vec![],
            notes: None,
            favorite: false,
            last_connected: None,
        })
        .unwrap();

    let launcher = MockLauncher::new();
    let app_launcher = launcher.clone();
    let mut app = App::new_with_deps(
        AppConfig::default(),
        AppDeps {
            resolver: Box::new(EmptyResolver),
            metadata: Arc::new(MetadataDb::default()),
            store,
            launcher: Box::new(app_launcher),
            password_store: Box::new(sshub::credentials::NoopPasswordStore),
        },
    );
    app.reload_hosts().unwrap();
    assert_eq!(app.hosts.len(), 1);

    let managed = app.hosts[0].managed().expect("should be managed");
    assert_eq!(managed.source, HostSource::SshConfig);

    // ssh_config-sourced host: only `ssh <alias>`, letting ssh inherit options
    // from ~/.ssh/config.
    let entry = app.hosts[0].clone();
    let ssh_argv = sshub::app::ssh_argv_for_entry(&entry);
    assert_eq!(ssh_argv, vec!["ssh".to_string(), "web-prod".to_string()]);
}
