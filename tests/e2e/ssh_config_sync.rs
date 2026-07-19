use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use sshub::app::{App, AppDeps};
use sshub::config::AppConfig;
use sshub::metadata::MetadataDb;
use sshub::store::{HostSource, HostUpdate, LauncherStore};
use tempfile::tempdir;

#[path = "../support/mod.rs"]
mod support;

use support::FixtureResolver;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn copy_fixture_tree(dest: &std::path::Path) -> (PathBuf, PathBuf) {
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

#[test]
fn reload_updates_ssh_config_address_without_wiping_tags() {
    let temp = tempdir().unwrap();
    let (config_path, ssh_g_dir) = copy_fixture_tree(temp.path());

    let resolver = FixtureResolver::with_paths(&config_path, &ssh_g_dir);
    let store = Arc::new(LauncherStore::open_in_memory().unwrap());
    sshub::ssh::import_ssh_config(&resolver, &store, &MetadataDb::default()).unwrap();

    let id = store
        .get_host_by_name("dev-local")
        .unwrap()
        .expect("imported")
        .id;
    store
        .update_host(
            id,
            &HostUpdate {
                tags: Some(vec!["keep-me".into()]),
                favorite: Some(true),
                ..Default::default()
            },
        )
        .unwrap();

    fs::write(
        ssh_g_dir.join("dev-local.txt"),
        "host dev-local\nhostname 10.99.88.77\nuser dev\nport 22\n",
    )
    .unwrap();

    let mut app = App::new_with_deps(
        AppConfig::default(),
        AppDeps {
            resolver: Box::new(FixtureResolver::with_paths(&config_path, &ssh_g_dir)),
            metadata: Arc::new(MetadataDb::default()),
            store: store.clone(),
            password_store: Box::new(sshub::credentials::NoopPasswordStore),
        },
    );
    app.reload_hosts().unwrap();

    let host = store.get_host_by_name("dev-local").unwrap().unwrap();
    assert_eq!(host.source, HostSource::SshConfig);
    assert_eq!(host.address, "10.99.88.77");
    assert_eq!(host.tags, vec!["keep-me"]);
    assert!(host.favorite);
}
