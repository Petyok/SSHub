mod resolver;
mod run_app_quit;

use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn help_exits_zero() {
    Command::cargo_bin("sshub")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("sshub"));
}

#[test]
fn dry_run_exits_zero() {
    Command::cargo_bin("sshub")
        .unwrap()
        .arg("--dry-run")
        .assert()
        .success();
}

#[test]
fn default_run_without_tty_fails_with_helpful_message() {
    let config_dir = tempfile::tempdir().unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    Command::cargo_bin("sshub")
        .unwrap()
        .env("SSH_LAUNCHER_CONFIG_DIR", config_dir.path())
        .env("SSH_LAUNCHER_DATA_DIR", data_dir.path())
        .env(
            "SSH_LAUNCHER_SSH_CONFIG",
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ssh_config"),
        )
        .assert()
        .failure()
        .stderr(predicate::str::contains("interactive terminal"));
}
