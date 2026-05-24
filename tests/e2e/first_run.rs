use std::sync::Arc;

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

/// When launcher.db does not exist yet and there are no hosts,
/// App::new() should automatically enter Help mode.
#[test]
fn first_run_empty_shows_help() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().to_path_buf();

    // Ensure launcher.db does NOT exist yet
    assert!(!data_dir.join("launcher.db").exists());

    // Use env vars to point App::new() at the temp directory
    std::env::set_var("SSH_LAUNCHER_DATA_DIR", &data_dir);
    std::env::set_var("SSH_LAUNCHER_SSH_CONFIG", "/dev/null");

    let app = App::new(AppConfig::default()).unwrap();

    // Clean up env vars
    std::env::remove_var("SSH_LAUNCHER_DATA_DIR");
    std::env::remove_var("SSH_LAUNCHER_SSH_CONFIG");

    assert_eq!(
        app.mode,
        AppMode::Help,
        "first run with no hosts should show Help"
    );
}

/// When launcher.db already exists, the app should start in Normal mode
/// even if there are no hosts.
#[test]
fn subsequent_run_empty_stays_normal() {
    let tmp = tempfile::tempdir().unwrap();
    let store_path = tmp.path().join("launcher.db");

    // Pre-create the store so it's not a first run
    let _store = LauncherStore::open(&store_path).unwrap();
    drop(_store);
    assert!(store_path.exists());

    let app = App::new_with_deps(
        AppConfig::default(),
        AppDeps {
            resolver: Box::new(EmptyResolver),
            metadata: Arc::new(MetadataDb::default()),
            store: Arc::new(LauncherStore::open(&store_path).unwrap()),
            launcher: Box::new(MockLauncher::new()),
            password_store: Box::new(sshub::credentials::NoopPasswordStore),
        },
    );

    assert_eq!(
        app.mode,
        AppMode::Normal,
        "subsequent run should stay Normal"
    );
}
