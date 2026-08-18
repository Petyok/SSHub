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
fn profile_flag_is_rejected_in_compatibility_mode() {
    let d = dir();
    sshub(d.path())
        .args(["--profile", "work", "host", "list"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unavailable"));
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

// ---------------------------------------------------------------------------
// `sshub theme` — headless theme CLI.
//
// Every case below must work without a database: the theme commands are
// dispatched before `CliContext::bootstrap`, so a data directory that cannot
// hold a database at all is still a green run.
// ---------------------------------------------------------------------------

/// A minimal but valid user theme. It carries no `extends`, so it inherits the
/// built-in `default` implicitly — the normal shape of a hand-written theme.
const VALID_THEME: &str =
    "schema_version = 1\nname = \"Mine\"\n\n[semantic]\naccent = \"#123456\"\n";

/// Writes `themes/<id>.toml` below the isolated config directory.
fn install_theme(dir: &Path, id: &str, body: &str) -> PathBuf {
    let themes = dir.join("themes");
    std::fs::create_dir_all(&themes).unwrap();
    let path = themes.join(format!("{id}.toml"));
    std::fs::write(&path, body).unwrap();
    path
}

#[test]
fn theme_list_does_not_bootstrap_databases() {
    let dir = dir();
    let not_a_directory = dir.path().join("blocked-data");
    std::fs::write(&not_a_directory, "file").unwrap();
    sshub(dir.path())
        .env("SSHUB_DATA_DIR", &not_a_directory)
        .args(["theme", "list", "--format", "json"])
        .assert()
        .success();
    assert!(!dir.path().join("launcher.db").exists());
    assert!(!dir.path().join("metadata.db").exists());
}

/// A theme whose `name` carries terminal control characters.
///
/// The control bytes are written as TOML escapes rather than raw bytes so the
/// fixture stays a valid basic string — after parsing the name really does hold
/// an ESC, a newline and a DEL.
const HOSTILE_NAME_THEME: &str = "schema_version = 1\n\
     name = \"ev\\u001Bil\\nnext\\u007F\"\n\n[semantic]\naccent = \"#123456\"\n";

/// Plain `theme list` prints a user-controlled name, so a theme file must not be
/// able to smuggle escape sequences into the operator's terminal.
#[test]
fn theme_list_plain_escapes_control_characters_in_a_theme_name() {
    let d = dir();
    install_theme(d.path(), "hostile", HOSTILE_NAME_THEME);

    let out = sshub(d.path()).args(["theme", "list"]).assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();

    assert!(
        !stdout.contains('\u{1b}'),
        "a raw ESC reached the terminal:\n{stdout:?}"
    );
    assert!(
        !stdout.contains('\u{7f}'),
        "a raw DEL reached the terminal:\n{stdout:?}"
    );
    assert!(
        stdout.contains("ev\\u{001b}il\\u{000a}next\\u{007f}"),
        "the name was not escaped visibly:\n{stdout}"
    );
    // The name's own newline must not add a row: every theme stays one line.
    let rows = stdout.lines().filter(|l| l.contains("hostile")).count();
    assert_eq!(rows, 1, "the name broke the table across rows:\n{stdout}");
}

/// `theme list` and `theme show` only read, so they must not bring the config
/// directory into existence — nor drag a legacy `~/.config/ssh-launcher` tree
/// into it as a side effect of being asked what is installed.
///
/// The isolation is a real subprocess with its own `HOME` and no
/// `SSHUB_CONFIG_DIR`, so nothing here mutates this process's environment.
#[test]
fn theme_read_commands_never_create_the_config_directory() {
    let home = dir();
    let legacy_themes = home.path().join(".config/ssh-launcher/themes");
    std::fs::create_dir_all(&legacy_themes).unwrap();
    std::fs::write(legacy_themes.join("legacy.toml"), VALID_THEME).unwrap();
    let new_dir = home.path().join(".config/sshub");

    for args in [
        vec!["theme", "list"],
        vec!["theme", "show", "aqua"],
        vec!["theme", "show", "missing"],
    ] {
        Command::cargo_bin("sshub")
            .unwrap()
            .env("HOME", home.path())
            .env("SSHUB_DATA_DIR", home.path().join("data"))
            .env_remove("SSHUB_CONFIG_DIR")
            .env_remove("SSH_LAUNCHER_CONFIG_DIR")
            .env("SSHUB_SSH_CONFIG", fixture_ssh_config())
            // The exit code is not the point here — `show missing` is expected
            // to fail. What matters is what the run left on disk.
            .args(&args)
            .output()
            .unwrap();

        assert!(
            !new_dir.exists(),
            "`sshub {}` created {}",
            args.join(" "),
            new_dir.display()
        );
        assert!(
            !new_dir.with_extension("migrating").exists(),
            "`sshub {}` left a migration staging directory behind",
            args.join(" ")
        );
    }
    // And the legacy tree it did not migrate is still exactly as it was.
    assert!(legacy_themes.join("legacy.toml").exists());
}

#[test]
fn theme_show_unknown_id_exits_one() {
    let dir = dir();
    sshub(dir.path())
        .args(["theme", "show", "missing"])
        .assert()
        .code(1);
}

#[test]
fn theme_check_valid_file_succeeds_in_both_formats() {
    let d = dir();
    let path = d.path().join("mine.toml");
    std::fs::write(&path, VALID_THEME).unwrap();

    sshub(d.path())
        .args(["theme", "check"])
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("OK: mine"));

    sshub(d.path())
        .args(["theme", "check"])
        .arg(&path)
        .args(["--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"valid\": true"));
}

#[test]
fn theme_list_succeeds_in_both_formats() {
    let d = dir();
    sshub(d.path())
        .args(["theme", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("default"))
        .stdout(predicate::str::contains("aqua"));

    sshub(d.path())
        .args(["theme", "list", "--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"id\": \"aqua\""));
}

#[test]
fn theme_show_succeeds_in_toml_and_json() {
    let d = dir();
    sshub(d.path())
        .args(["theme", "show", "aqua"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "# copied from theme 'aqua'; change `name` before installing under a new filename",
        ))
        // The embedded source is printed verbatim, comments and all.
        .stdout(predicate::str::contains("[gradients.reef_ring]"));

    sshub(d.path())
        .args(["theme", "show", "aqua", "--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"id\": \"aqua\""));

    sshub(d.path())
        .args(["theme", "show", "aqua", "--resolved"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[semantic]"))
        .stdout(predicate::str::contains("extends").not());

    sshub(d.path())
        .args(["theme", "show", "aqua", "--resolved", "--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"semantic\""));
}

#[test]
fn theme_show_never_prints_an_invalid_user_themes_source() {
    let d = dir();
    let secret = "private-token-do-not-print";
    install_theme(
        d.path(),
        "broken",
        &format!("schema_version = 1\nname = [\"{secret}\"\n"),
    );

    for format in ["toml", "json"] {
        sshub(d.path())
            .args(["theme", "show", "broken", "--format", format])
            .assert()
            .code(1)
            .stdout(predicate::str::contains(secret).not())
            .stderr(predicate::str::contains(secret).not());
    }
}

#[test]
fn theme_show_resolved_output_passes_check() {
    let d = dir();
    let out = sshub(d.path())
        .args(["theme", "show", "aqua", "--resolved"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let copy = d.path().join("aqua-custom.toml");
    std::fs::write(&copy, out).unwrap();

    sshub(d.path())
        .args(["theme", "check"])
        .arg(&copy)
        .assert()
        .success();
}

#[test]
fn theme_check_rejects_the_toml_format() {
    let d = dir();
    let path = d.path().join("mine.toml");
    std::fs::write(&path, VALID_THEME).unwrap();
    sshub(d.path())
        .args(["theme", "check"])
        .arg(&path)
        .args(["--format", "toml"])
        .assert()
        .code(2);
}

#[test]
fn theme_list_rejects_the_toml_format() {
    let d = dir();
    sshub(d.path())
        .args(["theme", "list", "--format", "toml"])
        .assert()
        .code(2);
}

#[test]
fn theme_show_rejects_the_plain_format() {
    let d = dir();
    sshub(d.path())
        .args(["theme", "show", "aqua", "--format", "plain"])
        .assert()
        .code(2);
}

#[test]
fn theme_check_malformed_file_exits_one_with_a_position() {
    let d = dir();
    let path = d.path().join("broken.toml");
    std::fs::write(
        &path,
        "schema_version = 1\nname = \"Broken\"\n\n[semantic]\nbordr = \"#123456\"\n",
    )
    .unwrap();
    sshub(d.path())
        .args(["theme", "check"])
        .arg(&path)
        .assert()
        .code(1)
        .stdout(predicate::str::contains("broken.toml:5:1"))
        .stdout(predicate::str::contains("bordr"));
}

#[test]
fn theme_misuse_exits_two() {
    let d = dir();
    // No subcommand at all.
    sshub(d.path()).args(["theme"]).assert().code(2);
    // Unknown subcommand.
    sshub(d.path())
        .args(["theme", "frobnicate"])
        .assert()
        .code(2);
    // Missing mandatory argument.
    sshub(d.path()).args(["theme", "check"]).assert().code(2);
    sshub(d.path()).args(["theme", "show"]).assert().code(2);
    // Unknown option.
    sshub(d.path())
        .args(["theme", "list", "--verbose"])
        .assert()
        .code(2);
}

#[test]
fn theme_list_reports_invalid_entries_and_still_exits_zero() {
    let d = dir();
    install_theme(d.path(), "good", VALID_THEME);
    install_theme(
        d.path(),
        "bad",
        "schema_version = 1\nname = \"Bad\"\n\n[semantic]\nbordr = \"#123456\"\n",
    );

    sshub(d.path())
        .args(["theme", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("good"))
        .stdout(predicate::str::contains("bad"))
        .stdout(predicate::str::contains("invalid"));
}

#[test]
fn theme_list_reports_a_user_file_squatting_a_builtin_id() {
    let d = dir();
    // The one file the user is confused about: it looks installed, but the
    // reserved id keeps it from ever being canonical.
    install_theme(d.path(), "aqua", VALID_THEME);

    let out = sshub(d.path())
        .args(["theme", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let out = String::from_utf8(out).unwrap();
    assert!(
        out.contains("aqua.toml"),
        "the squatting file must be listed, not hidden behind the built-in:\n{out}"
    );
    assert!(
        out.contains("already taken") || out.contains("reserved"),
        "the collision must be explained:\n{out}"
    );
}

#[test]
fn theme_list_registry_io_error_exits_one() {
    let d = dir();
    // `themes` is a regular file, so the directory cannot be read at all.
    std::fs::write(d.path().join("themes"), "not a directory").unwrap();
    sshub(d.path()).args(["theme", "list"]).assert().code(1);
}

#[test]
fn theme_check_warns_about_a_sibling_parent_without_failing() {
    let d = dir();
    let pack = d.path().join("pack");
    std::fs::create_dir_all(&pack).unwrap();
    std::fs::write(
        pack.join("base.toml"),
        "schema_version = 1\nname = \"Base\"\n\n[semantic]\naccent = \"#101010\"\n",
    )
    .unwrap();
    let child = pack.join("child.toml");
    std::fs::write(
        &child,
        "schema_version = 1\nname = \"Child\"\nextends = \"base\"\n",
    )
    .unwrap();

    sshub(d.path())
        .args(["theme", "check"])
        .arg(&child)
        .assert()
        .success()
        .stdout(predicate::str::contains("base.toml"))
        .stdout(predicate::str::contains("warning"));
}

#[test]
fn theme_help_is_theme_specific_and_needs_no_context() {
    let d = dir();
    // A data directory that cannot hold a database proves help never bootstraps.
    let blocked = d.path().join("blocked-data");
    std::fs::write(&blocked, "file").unwrap();

    sshub(d.path())
        .env("SSHUB_DATA_DIR", &blocked)
        .args(["theme", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("sshub theme"))
        .stdout(predicate::str::contains("check"))
        .stdout(predicate::str::contains("list"))
        .stdout(predicate::str::contains("show"));

    sshub(d.path())
        .env("SSHUB_DATA_DIR", &blocked)
        .args(["theme", "check", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("sshub theme check"));
}

#[test]
fn global_help_lists_the_theme_command() {
    let d = dir();
    sshub(d.path())
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("sshub theme check"));
}

/// A target that is readable while its directory is not listable: the checker
/// cannot know whether the siblings this theme needs exist, so it must fail
/// rather than guess "no siblings" and report success.
#[cfg(unix)]
#[test]
fn theme_check_unlistable_directory_exits_one() {
    use std::os::unix::fs::PermissionsExt;

    let d = dir();
    let closed = d.path().join("closed");
    std::fs::create_dir(&closed).unwrap();
    let file = closed.join("child.toml");
    std::fs::write(&file, VALID_THEME).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444)).unwrap();
    std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o111)).unwrap();

    // Root ignores the permission bits, so the state under test cannot be built.
    if std::fs::read_dir(&closed).is_ok() {
        std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }

    let assertion = sshub(d.path()).args(["theme", "check"]).arg(&file).assert();
    std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o755)).unwrap();
    assertion.code(1);
}

// ── exec ────────────────────────────────────────────────────────────────────
// Every case here stops before a child is spawned: argument parsing, the
// unknown-host lookup and the mosh refusal all fail first, so nothing reaches
// the network. A real round-trip needs a live host and is not smoke-testable.

#[test]
fn exec_help_lists_the_flags_and_exits_zero() {
    let d = dir();
    sshub(d.path())
        .args(["exec", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("sshub exec"))
        .stdout(predicate::str::contains("--tty"))
        .stdout(predicate::str::contains("--timeout"));
}

#[test]
fn exec_without_a_host_exits_two() {
    let d = dir();
    sshub(d.path())
        .arg("exec")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("exec requires a host"));
}

#[test]
fn exec_without_a_command_exits_two() {
    let d = dir();
    sshub(d.path())
        .args(["exec", "some-host"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("exec requires a command"));
}

#[test]
fn exec_on_an_unknown_host_fails_before_spawning() {
    let d = dir();
    sshub(d.path())
        .args(["exec", "no-such-host", "--", "uptime"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn exec_refuses_a_mosh_host() {
    let d = dir();
    sshub(d.path())
        .args([
            "host",
            "add",
            "--name",
            "mosh-box",
            "--address",
            "10.0.0.9",
            "--transport",
            "mosh",
        ])
        .assert()
        .success();

    sshub(d.path())
        .args(["exec", "mosh-box", "--", "uptime"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("mosh"));
}

#[test]
fn exec_rejects_a_command_flag_that_is_not_behind_the_separator() {
    let d = dir();
    // `-l` would otherwise be dropped by positional parsing and the remote
    // command would silently run as plain `ls`.
    sshub(d.path())
        .args(["exec", "some-host", "ls", "-l"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("after `--`"));
}

#[test]
fn exec_timeout_without_a_value_is_a_usage_error() {
    let d = dir();
    sshub(d.path())
        .args(["exec", "some-host", "--timeout", "--", "uptime"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--timeout requires a value"));

    sshub(d.path())
        .args(["exec", "some-host", "--timeout", "0", "--", "uptime"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("at least 1 second"));
}

/// A remote command's own flags must not be read as sshub's: the profile
/// parser in `main.rs` runs before dispatch and used to swallow them.
#[test]
fn exec_does_not_let_the_profile_parser_eat_the_remote_commands_flags() {
    let d = dir();
    sshub(d.path())
        .args([
            "exec",
            "no-such-host",
            "--",
            "aws",
            "--profile",
            "prod",
            "s3",
            "ls",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"))
        .stderr(predicate::str::contains("profile").not());
}

#[test]
fn global_help_lists_the_exec_command() {
    let d = dir();
    sshub(d.path())
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("sshub exec"));
}

/// Issue #101: a host field ssh would read as an option is refused at the
/// write, and an import file carrying one loses that row only — not the file.
#[test]
fn import_refuses_an_option_like_address_and_keeps_the_rest() {
    let d = dir();
    let xml = d.path().join("confCons.xml");
    std::fs::write(
        &xml,
        r#"<?xml version="1.0" encoding="utf-8"?>
<mrng:Connections xmlns:mrng="http://mremoteng.org" Name="Connections" ConfVersion="2.6">
  <Node Name="good" Type="Connection" Hostname="10.0.0.1" Username="admin" Protocol="SSH2" />
  <Node Name="evil" Type="Connection" Hostname="-oProxyCommand=id" Protocol="SSH2" />
</mrng:Connections>
"#,
    )
    .unwrap();

    sshub(d.path())
        .args(["import", "--from", "mremoteng"])
        .arg(&xml)
        .assert()
        .success()
        .stdout(predicate::str::contains("1 host(s)"))
        .stdout(predicate::str::contains("refused"))
        .stderr(predicate::str::contains("skipping host 'evil'"));

    sshub(d.path())
        .args(["host", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("good"))
        .stdout(predicate::str::contains("evil").not());
}

#[test]
fn host_add_refuses_an_option_like_address() {
    let d = dir();
    sshub(d.path())
        .args([
            "host",
            "add",
            "--name",
            "evil",
            "--address",
            "-oProxyCommand=id",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("ssh reads as an option"));
}
