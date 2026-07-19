use sshub::config::load_config;
use tempfile::tempdir;

#[test]
fn load_config_creates_default_file_in_config_dir() {
    let dir = tempdir().unwrap();
    let config_dir = dir.path().to_path_buf();

    std::env::set_var("SSH_LAUNCHER_CONFIG_DIR", &config_dir);

    load_config().expect("load_config should succeed");

    let config_file = config_dir.join("config.toml");
    assert!(config_file.is_file(), "expected config.toml to be created");

    std::env::remove_var("SSH_LAUNCHER_CONFIG_DIR");
}
