use std::path::PathBuf;

use assert_cmd::Command;
use tempfile::tempdir;

fn fixture_ssh_config() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ssh_config")
}

fn smoke_command(auto_quit: &str) -> Command {
    let config_dir = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let mut cmd = Command::cargo_bin("sshub").unwrap();
    cmd.env("SSH_LAUNCHER_AUTO_QUIT", auto_quit)
        .env("SSH_LAUNCHER_CONFIG_DIR", config_dir.path())
        .env("SSH_LAUNCHER_DATA_DIR", data_dir.path())
        .env("SSH_LAUNCHER_SSH_CONFIG", fixture_ssh_config());
    cmd
}

#[test]
fn run_app_auto_quit_exits_zero() {
    smoke_command("1").assert().success();
}

#[test]
fn run_app_quit_with_q_exits_zero() {
    smoke_command("q").assert().success();
}
