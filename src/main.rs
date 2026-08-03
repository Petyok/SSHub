use anyhow::Result;

use sshub::profile::{self, ProfilePaths, StartupOptions};

/// Confirmation flag required for destructive subcommands (e.g. `db purge`).
const CONFIRM_FLAG: &str = "--yes-i-am-stupid";

fn main() -> Result<()> {
    // If ssh re-executed us as its SSH_ASKPASS helper, emit the staged secret
    // and exit before touching argv or the TUI.
    if sshub::session::askpass::maybe_run_askpass() {
        return Ok(());
    }

    let args: Vec<String> = std::env::args().skip(1).collect();

    // Global profile flags (`--profile NAME`, `--manage-profiles`) apply to
    // both the TUI and headless subcommands, so parse them before dispatch.
    let (startup, args) = profile::extract_startup_flags(args)?;

    // Subcommands must be handled before the global flags and the TUI launch
    // path, so that `sshub <cmd> --help` reaches the subcommand's own help
    // (via cli::run_subcommand) instead of the global `--help` below.
    if args.first().map(String::as_str) == Some("db") {
        return run_db(&startup, &args[1..]);
    }

    if let Some(cmd) = args.first() {
        if sshub::cli::is_subcommand(cmd) {
            let code = run_cli(&startup, &args)?;
            std::process::exit(code);
        }
    }

    // Global flags apply only when no subcommand was given.
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return Ok(());
    }

    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("sshub {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    if let Some(cmd) = args.first() {
        // A non-flag first arg that is neither `db` nor a known subcommand is a
        // usage error. The TUI takes no positional args, so falling through to
        // it would launch a full-screen app for a typo (and fail without a TTY).
        if !cmd.starts_with('-') {
            eprintln!("sshub: unknown command '{cmd}'");
            eprintln!("       run `sshub --help` for the command list");
            std::process::exit(2);
        }
    }

    if args.iter().any(|a| a == "--dry-run") {
        return Ok(());
    }
    sshub::run_with(startup)
}

/// Resolve the profile for a headless invocation: `--profile NAME` selects
/// directly (unknown names fail with the list of available profiles); without
/// it the last-used profile is chosen. Headless commands never show the picker.
fn resolve_cli_profile(startup: &StartupOptions) -> Result<ProfilePaths> {
    match profile::resolve_startup(startup, false)? {
        profile::Startup::Silent(paths) => Ok(paths),
        profile::Startup::Picker { .. } => {
            unreachable!("non-interactive resolution never returns Picker")
        }
    }
}

fn run_cli(startup: &StartupOptions, args: &[String]) -> Result<i32> {
    let cmd = args[0].as_str();
    let rest = &args[1..];
    let paths = resolve_cli_profile(startup)?;
    let mut ctx = sshub::cli::CliContext::bootstrap_with(paths)?;
    sshub::cli::run_subcommand(&mut ctx, cmd, rest)
}

/// Handle `sshub db <subcommand>`.
fn run_db(startup: &StartupOptions, args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("purge") => run_db_purge(startup, args.iter().any(|a| a == CONFIRM_FLAG)),
        Some(other) => {
            eprintln!("sshub: unknown db subcommand '{other}'");
            eprintln!("       try: sshub db purge {CONFIRM_FLAG}");
            std::process::exit(2);
        }
        None => {
            eprintln!("sshub: `db` needs a subcommand");
            eprintln!("       try: sshub db purge {CONFIRM_FLAG}");
            std::process::exit(2);
        }
    }
}

/// `sshub db purge` — wipe the launcher database of the selected profile.
/// Refuses without the confirmation flag because it is irreversible.
fn run_db_purge(startup: &StartupOptions, confirmed: bool) -> Result<()> {
    if !confirmed {
        eprintln!("This permanently deletes your SSHub database:");
        eprintln!("  - all managed hosts, groups, identities, and tunnels");
        eprintln!("  - the entire audit log");
        eprintln!("It does NOT touch ~/.ssh/config or the hosts imported from it.");
        eprintln!();
        eprintln!("If you really mean it, re-run:");
        eprintln!("    sshub db purge {CONFIRM_FLAG}");
        eprintln!("(add --profile NAME to target a specific profile)");
        std::process::exit(1);
    }

    let paths = resolve_cli_profile(startup)?;
    let removed = sshub::purge_profile_database(&paths)?;
    if removed.is_empty() {
        println!(
            "Nothing to purge - no database found for profile '{}'.",
            paths.name
        );
    } else {
        println!("profile: {}", paths.name);
        for path in &removed {
            println!("removed {}", path.display());
        }
        println!("Database purged. A fresh one is created on the next launch.");
        println!("(Passwords in the OS keyring are left untouched.)");
    }
    Ok(())
}

fn print_help() {
    println!(
        r#"sshub - SSHub TUI SSH host launcher

USAGE:
    sshub [OPTIONS]                         Launch TUI (default)
    sshub [OPTIONS] <command> [args]        Headless CLI subcommands

OPTIONS:
    -h, --help              Print help
    -V, --version           Print version
        --dry-run           Exit immediately (smoke / CI)
        --profile NAME      Use the named profile (skips the picker)
        --manage-profiles   Open the profile picker even with one profile

PROFILES:
    Each profile is an isolated workspace: its own hosts database, imported
    ssh_config source, and settings (theme, keybinds). With a single profile
    startup is unchanged; with several, a picker appears after the splash.
    Data lives under ~/.local/share/sshub/profiles/<name>/.

HOST (read/write):
    sshub host list [--tag TAG]... [--group GROUP] [--sort MODE] [--format plain|json]
    sshub host show <name> [--format plain|json]
    sshub host connect <name> [-v|--verbose]
    sshub host resolve <name> [--format plain|json]
    sshub host search <query> [--format plain|json]
    sshub host add|edit|rename|delete|duplicate …

ALIASES:
    sshub connect <name>                    Same as `host connect`
    sshub list …                            Same as `host list`

GROUPS:
    sshub group list [--all] [--format plain|json]
    sshub group show <name> [--format plain|json]
    sshub group add --name NAME [--parent GROUP] [--default-identity NAME] [--sort-order N]
    sshub group edit --name NAME [--set-name …] [--set-parent …] [--clear-parent]
                     [--set-default-identity …] [--clear-default-identity] [--set-sort-order N]
    sshub group delete --name NAME --yes
    sshub groups …                          Alias for `group list` (forwards flags)

IDENTITIES:
    sshub identity list|show|add|edit|delete|agent-remove …
    sshub identity agent-remove --name NAME

TUNNELS:
    sshub tunnel list|show|create|delete|start|stop …

SFTP (one-shot):
    sshub sftp ls|get|put|rm|mkdir|rename|chmod …

AUDIT:
    sshub audit list|stats …

INVENTORY / CONFIG:
    sshub tags [--format plain|json]
    sshub sync                              Refresh ssh_config rows in DB
    sshub import [--from SRC] [--dry-run]   Import hosts (ssh_config, Termius, PuTTY, mRemoteNG)
    sshub export [--stdout] [-o PATH]       Export launcher hosts to ssh_config snippet
    sshub db purge [{CONFIRM_FLAG}]

COMPLETIONS:
    sshub completions bash|zsh|fish [--cache PATH]

DESTRUCTIVE CONFIRMATION:
    Most delete commands require --yes
    db purge requires {CONFIRM_FLAG} (irreversible database wipe)

ENVIRONMENT:
    SSHUB_CONFIG_DIR          Override config directory (fallback: SSH_LAUNCHER_CONFIG_DIR)
    SSHUB_DATA_DIR            Override data directory (fallback: SSH_LAUNCHER_DATA_DIR)
    SSHUB_SSH_CONFIG          Override SSH config path (fallback: SSH_LAUNCHER_SSH_CONFIG)
    SSHUB_DRY_RUN             Exit immediately (fallback: SSH_LAUNCHER_DRY_RUN)
    SSHUB_AUTO_QUIT           Headless smoke (fallback: SSH_LAUNCHER_AUTO_QUIT): 1 = quit after first draw, q = quit via q key

    Directory overrides disable profiles (compat mode): the override directory
    is used verbatim and --profile is rejected.
"#
    );
}
