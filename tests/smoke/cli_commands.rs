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

#[test]
fn sync_exits_zero() {
    let d = dir();
    sshub(d.path()).arg("sync").assert().success();
}

#[test]
fn tags_exits_zero() {
    let d = dir();
    sshub(d.path()).arg("tags").assert().success();
}

#[test]
fn export_stdout_exits_zero() {
    let d = dir();
    sshub(d.path())
        .args(["export", "--stdout"])
        .assert()
        .success();
}

#[test]
fn groups_exits_zero() {
    let d = dir();
    sshub(d.path()).arg("groups").assert().success();
}

#[test]
fn group_list_exits_zero() {
    let d = dir();
    sshub(d.path()).args(["group", "list"]).assert().success();
}

#[test]
fn identity_list_exits_zero() {
    let d = dir();
    sshub(d.path())
        .args(["identity", "list"])
        .assert()
        .success();
}

#[test]
fn import_from_fixture_exits_zero() {
    let d = dir();
    // Importing from the fixture ssh_config: partial or no failures still exit 0.
    sshub(d.path()).arg("import").assert().success();
}

#[test]
fn completions_bash_prints_non_empty() {
    let d = dir();
    sshub(d.path())
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty().not());
}

#[test]
fn completions_zsh_prints_non_empty() {
    let d = dir();
    sshub(d.path())
        .args(["completions", "zsh"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty().not());
}

#[test]
fn completions_fish_prints_non_empty() {
    let d = dir();
    sshub(d.path())
        .args(["completions", "fish"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty().not());
}

#[test]
fn groups_all_forwards_flag_and_exits_zero() {
    let d = dir();
    // `groups --all` aliases to `group list --all`; the reserved-group filter
    // still exits 0 on an empty database.
    sshub(d.path()).args(["groups", "--all"]).assert().success();
}

#[test]
fn host_delete_without_yes_exits_one() {
    let d = dir();
    // `--yes` is required before the host is even looked up, so a missing host
    // without confirmation still exits 1.
    sshub(d.path())
        .args(["host", "delete", "--name", "doesnotexist"])
        .assert()
        .code(1);
}

#[test]
fn identity_delete_without_yes_exits_one() {
    let d = dir();
    sshub(d.path())
        .args(["identity", "delete", "--name", "nope"])
        .assert()
        .code(1);
}

#[test]
fn import_help_lists_new_sources() {
    let d = dir();
    sshub(d.path())
        .args(["import", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--from"))
        .stdout(predicate::str::contains("putty"))
        .stdout(predicate::str::contains("mremoteng"));
}

#[test]
fn import_unknown_source_exits_two() {
    let d = dir();
    sshub(d.path())
        .args(["import", "--from", "bogus"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unknown source"));
}

#[test]
fn import_ssh_dry_run_is_rejected() {
    let d = dir();
    sshub(d.path())
        .args(["import", "--dry-run"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("not supported for --from ssh"));
}

#[test]
fn import_mremoteng_without_path_exits_one() {
    let d = dir();
    sshub(d.path())
        .args(["import", "--from", "mremoteng"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("needs a PATH"));
}

#[test]
fn import_mremoteng_dry_run_previews_without_writing() {
    let d = dir();
    let xml = d.path().join("confCons.xml");
    std::fs::write(
        &xml,
        r#"<mrng:Connections><Node Name="smoke-host" Type="Connection" Hostname="10.9.9.9" Protocol="SSH2" Port="22"/></mrng:Connections>"#,
    )
    .unwrap();

    // Preview lists the host and says nothing was written.
    sshub(d.path())
        .args(["import", "--from", "mremoteng"])
        .arg(&xml)
        .arg("--dry-run")
        .assert()
        .success()
        .stdout(predicate::str::contains("smoke-host"))
        .stdout(predicate::str::contains("dry run"));

    // The store is untouched: the host is not listed afterwards.
    sshub(d.path())
        .args(["list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("smoke-host").not());
}

#[test]
fn import_putty_reg_file_imports_host() {
    let d = dir();
    let reg = d.path().join("sessions.reg");
    std::fs::write(
        &reg,
        "Windows Registry Editor Version 5.00\r\n\r\n\
         [HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\Sessions\\smokebox]\r\n\
         \"HostName\"=\"10.8.8.8\"\r\n\
         \"Protocol\"=\"ssh\"\r\n\
         \"PortNumber\"=dword:00000016\r\n",
    )
    .unwrap();

    sshub(d.path())
        .args(["import", "--from", "putty"])
        .arg(&reg)
        .assert()
        .success()
        .stdout(predicate::str::contains("imported: 1 host"));

    // The imported host is now listed.
    sshub(d.path())
        .args(["list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("smokebox"));
}
