//! Offline smoke tests for the hand-rolled CLI (`sshub <command>`).
//!
//! Every invocation runs against a fresh `tempfile::tempdir()` wired through
//! `SSHUB_DATA_DIR` / `SSHUB_CONFIG_DIR` (and a fixture `SSHUB_SSH_CONFIG`) so the
//! tests never touch the real user database, config, or `~/.ssh/config`. None of
//! these commands reach the network, a TTY, or a live SSH host: they exercise
//! argument parsing, per-command dispatch, and empty-database read paths only.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

/// A fixture ssh_config so `host`/`tunnel` listing never reads the user's real
/// `~/.ssh/config`. It is small and static, safe to resolve offline.
fn fixture_ssh_config() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ssh_config")
}

/// Build a `sshub` command isolated to `dir` for data and config, with the SSH
/// config pointed at the checked-in fixture. Callers append the subcommand args.
fn sshub(dir: &Path) -> Command {
    let mut cmd = Command::cargo_bin("sshub").unwrap();
    cmd.env("SSHUB_DATA_DIR", dir)
        .env("SSHUB_CONFIG_DIR", dir)
        .env("SSHUB_SSH_CONFIG", fixture_ssh_config());
    cmd
}

/// Fresh isolated data/config directory for one invocation.
fn dir() -> TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn unknown_command_exits_two() {
    let d = dir();
    sshub(d.path())
        .arg("frobnicate")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unknown command"));
}

#[test]
fn audit_list_empty_exits_zero() {
    let d = dir();
    sshub(d.path()).args(["audit", "list"]).assert().success();
}

#[test]
fn audit_stats_exits_zero_and_reports_ok() {
    let d = dir();
    sshub(d.path())
        .args(["audit", "stats"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ok"));
}

#[test]
fn audit_stats_include_retry_reports_retry() {
    let d = dir();
    sshub(d.path())
        .args(["audit", "stats", "--include-retry"])
        .assert()
        .success()
        .stdout(predicate::str::contains("retry"));
}

#[test]
fn audit_list_bogus_status_exits_two() {
    let d = dir();
    sshub(d.path())
        .args(["audit", "list", "--status", "bogus"])
        .assert()
        .code(2);
}

#[test]
fn host_list_exits_zero() {
    let d = dir();
    sshub(d.path()).args(["host", "list"]).assert().success();
}

#[test]
fn tunnel_list_exits_zero() {
    let d = dir();
    sshub(d.path()).args(["tunnel", "list"]).assert().success();
}

#[test]
fn sftp_without_subcommand_exits_two() {
    let d = dir();
    sshub(d.path()).arg("sftp").assert().code(2);
}

#[test]
fn sftp_ls_without_host_exits_two() {
    let d = dir();
    sshub(d.path()).args(["sftp", "ls"]).assert().code(2);
}

#[test]
fn audit_help_shows_per_command_help_not_global() {
    let d = dir();
    sshub(d.path())
        .args(["audit", "--help"])
        .assert()
        .success()
        // Unique to the per-command audit help; absent from the global `--help`.
        .stdout(predicate::str::contains("inspect the connection audit log"))
        .stdout(predicate::str::contains("stats"))
        // Must NOT fall through to the global help header.
        .stdout(predicate::str::contains("TUI SSH host launcher").not());
}

#[test]
fn host_help_shows_per_command_help() {
    let d = dir();
    sshub(d.path())
        .args(["host", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("manage launcher hosts"))
        .stdout(predicate::str::contains("TUI SSH host launcher").not());
}
