use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use sshub::app::{App, AppDeps};
use sshub::config::AppConfig;
use sshub::metadata::MetadataDb;
use sshub::store::LauncherStore;
use sshub::watcher::{spawn_config_watcher, WatchEvent, WATCHER_DEBOUNCE};

#[path = "../support/mod.rs"]
mod support;

use support::{FixtureResolver, MockLauncher};

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn copy_fixture_tree(dest: &Path) -> (PathBuf, PathBuf) {
    let config_path = dest.join("ssh_config");
    let ssh_g_dir = dest.join("ssh_g");
    fs::create_dir_all(&ssh_g_dir).unwrap();
    fs::copy(
        manifest_dir().join("tests/fixtures/ssh_config"),
        &config_path,
    )
    .unwrap();
    for entry in fs::read_dir(manifest_dir().join("tests/fixtures/ssh_g")).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), ssh_g_dir.join(entry.file_name())).unwrap();
    }
    (config_path, ssh_g_dir)
}

fn wait_for_config_changed(
    rx: &std::sync::mpsc::Receiver<WatchEvent>,
    timeout: Duration,
) -> WatchEvent {
    rx.recv_timeout(timeout)
        .unwrap_or_else(|err| panic!("timed out waiting for config watcher event: {err}"))
}

#[test]
fn config_file_change_triggers_reload() {
    let temp = tempfile::tempdir().unwrap();
    let (config_path, ssh_g_dir) = copy_fixture_tree(temp.path());

    let resolver = FixtureResolver::with_paths(&config_path, &ssh_g_dir);
    let metadata: Arc<dyn sshub::metadata::MetadataStore> = Arc::new(MetadataDb::default());
    let mut app = App::new_with_deps(
        AppConfig::default(),
        AppDeps {
            resolver: Box::new(resolver),
            metadata,
            store: Arc::new(LauncherStore::open_in_memory().unwrap()),
            launcher: Box::new(MockLauncher::new()),
            password_store: Box::new(sshub::credentials::NoopPasswordStore),
        },
    );
    app.reload_hosts().unwrap();
    assert_eq!(app.hosts.len(), 3);

    let rx = spawn_config_watcher(&config_path).unwrap();
    app.set_watcher_rx(rx);

    let mut config = fs::read_to_string(&config_path).unwrap();
    config.push_str("\nHost qa-box\n    HostName 10.0.3.7\n    User qa\n");
    fs::write(&config_path, config).unwrap();
    fs::write(
        ssh_g_dir.join("qa-box.txt"),
        "host qa-box\nhostname 10.0.3.7\nuser qa\nport 22\n",
    )
    .unwrap();

    let event = wait_for_config_changed(
        app.watcher_rx.as_ref().unwrap(),
        WATCHER_DEBOUNCE + Duration::from_secs(2),
    );
    assert_eq!(event, WatchEvent::ConfigChanged);

    app.reload_hosts().unwrap();
    assert_eq!(app.hosts.len(), 4);
    assert!(app.hosts.iter().any(|entry| entry.name() == "qa-box"));
}

#[test]
fn manual_reload_after_write_updates_host_list() {
    let temp = tempfile::tempdir().unwrap();
    let (config_path, ssh_g_dir) = copy_fixture_tree(temp.path());

    let resolver = FixtureResolver::with_paths(&config_path, &ssh_g_dir);
    let metadata: Arc<dyn sshub::metadata::MetadataStore> = Arc::new(MetadataDb::default());
    let mut app = App::new_with_deps(
        AppConfig::default(),
        AppDeps {
            resolver: Box::new(resolver),
            metadata,
            store: Arc::new(LauncherStore::open_in_memory().unwrap()),
            launcher: Box::new(MockLauncher::new()),
            password_store: Box::new(sshub::credentials::NoopPasswordStore),
        },
    );
    app.reload_hosts().unwrap();
    assert!(!app.hosts.iter().any(|entry| entry.name() == "ops-node"));

    let mut config = fs::read_to_string(&config_path).unwrap();
    config.push_str("\nHost ops-node\n    HostName 10.0.4.2\n    User ops\n");
    fs::write(&config_path, config).unwrap();
    fs::write(
        ssh_g_dir.join("ops-node.txt"),
        "host ops-node\nhostname 10.0.4.2\nuser ops\nport 22\n",
    )
    .unwrap();

    thread::sleep(Duration::from_millis(50));
    app.reload_hosts().unwrap();

    assert_eq!(app.hosts.len(), 4);
    assert!(app.hosts.iter().any(|entry| entry.name() == "ops-node"));
}
