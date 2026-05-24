use sshub::config::{load_config, TerminalKind};
use tempfile::tempdir;

#[test]
fn load_config_creates_default_file_in_config_dir() {
    let dir = tempdir().unwrap();
    let config_dir = dir.path().to_path_buf();

    std::env::set_var("SSH_LAUNCHER_CONFIG_DIR", &config_dir);

    let config = load_config().expect("load_config should succeed");
    assert_eq!(config.terminal, TerminalKind::Kitty);
    assert!(config.launch_command.is_none());

    let config_file = config_dir.join("config.toml");
    assert!(config_file.is_file(), "expected config.toml to be created");

    let content = std::fs::read_to_string(&config_file).expect("read config.toml");
    assert!(
        content.contains("terminal"),
        "default config should include terminal key"
    );

    std::env::remove_var("SSH_LAUNCHER_CONFIG_DIR");
}
