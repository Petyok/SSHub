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
fn db_purge_without_flag_refuses_and_keeps_db() {
    let data_dir = tempfile::tempdir().unwrap();
    let db = data_dir.path().join("launcher.db");
    std::fs::write(&db, b"pretend-db").unwrap();

    Command::cargo_bin("sshub")
        .unwrap()
        .env("SSHUB_DATA_DIR", data_dir.path())
        .args(["db", "purge"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--yes-i-am-stupid"));

    assert!(db.exists(), "db must survive a refused purge");
}

#[test]
fn db_purge_with_flag_removes_db_and_sidecars() {
    let data_dir = tempfile::tempdir().unwrap();
    let db = data_dir.path().join("launcher.db");
    let wal = data_dir.path().join("launcher.db-wal");
    std::fs::write(&db, b"pretend-db").unwrap();
    std::fs::write(&wal, b"pretend-wal").unwrap();

    Command::cargo_bin("sshub")
        .unwrap()
        .env("SSHUB_DATA_DIR", data_dir.path())
        .args(["db", "purge", "--yes-i-am-stupid"])
        .assert()
        .success()
        .stdout(predicate::str::contains("purged"));

    assert!(!db.exists(), "db must be removed");
    assert!(!wal.exists(), "wal sidecar must be removed");
}

#[test]
fn db_purge_with_no_db_is_a_clean_noop() {
    let data_dir = tempfile::tempdir().unwrap();
    Command::cargo_bin("sshub")
        .unwrap()
        .env("SSHUB_DATA_DIR", data_dir.path())
        .args(["db", "purge", "--yes-i-am-stupid"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Nothing to purge"));
}

#[test]
fn unknown_db_subcommand_exits_two() {
    Command::cargo_bin("sshub")
        .unwrap()
        .args(["db", "frobnicate"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unknown db subcommand"));
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
