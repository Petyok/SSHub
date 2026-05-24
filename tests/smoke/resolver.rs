use std::path::PathBuf;

use sshub::ssh::{HostResolver, SshConfigResolver};

fn fixture_ssh_config() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ssh_config")
}

#[test]
fn list_hosts_reads_fixture_via_config_path() {
    let resolver = SshConfigResolver::with_config_path(fixture_ssh_config());
    let hosts = resolver
        .list_hosts()
        .expect("list_hosts from fixture config");

    assert_eq!(
        hosts,
        vec![
            "dev-local".to_string(),
            "staging-app".to_string(),
            "prod-db-01".to_string(),
        ]
    );
}

#[test]
fn list_hosts_reads_fixture_via_ssh_launcher_ssh_config_env() {
    let config_path = fixture_ssh_config();
    std::env::set_var("SSH_LAUNCHER_SSH_CONFIG", &config_path);

    let resolver = SshConfigResolver::new();
    assert_eq!(resolver.config_path(), config_path.as_path());

    let hosts = resolver.list_hosts().expect("list_hosts via env override");
    assert_eq!(hosts.len(), 3);
    assert!(hosts.contains(&"dev-local".to_string()));
}
