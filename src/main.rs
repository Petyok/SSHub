use anyhow::Result;

/// Confirmation flag required for destructive subcommands (e.g. `db purge`).
const CONFIRM_FLAG: &str = "--yes-i-am-stupid";

fn main() -> Result<()> {
    // If ssh re-executed us as its SSH_ASKPASS helper, emit the staged secret
    // and exit before touching argv or the TUI.
    if sshub::session::askpass::maybe_run_askpass() {
        return Ok(());
    }

    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return Ok(());
    }

    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("sshub {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    // Subcommands must be handled before the TUI launch path.
    if args.first().map(String::as_str) == Some("db") {
        return run_db(&args[1..]);
    }

    if let Some(cmd) = args.first() {
        if sshub::cli::is_subcommand(cmd) {
            let code = run_cli(&args)?;
            std::process::exit(code);
        }
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
    sshub::run()
}

fn run_cli(args: &[String]) -> Result<i32> {
    let cmd = args[0].as_str();
    let rest = &args[1..];
    let mut ctx = sshub::cli::CliContext::bootstrap()?;
    sshub::cli::run_subcommand(&mut ctx, cmd, rest)
}

/// Handle `sshub db <subcommand>`.
fn run_db(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("purge") => run_db_purge(args.iter().any(|a| a == CONFIRM_FLAG)),
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

/// `sshub db purge` — wipe the launcher database. Refuses without the
/// confirmation flag because it is irreversible.
fn run_db_purge(confirmed: bool) -> Result<()> {
    if !confirmed {
        eprintln!("This permanently deletes your SSHub database:");
        eprintln!("  - all managed hosts, groups, identities, and tunnels");
        eprintln!("  - the entire audit log");
        eprintln!("It does NOT touch ~/.ssh/config or the hosts imported from it.");
        eprintln!();
        eprintln!("If you really mean it, re-run:");
        eprintln!("    sshub db purge {CONFIRM_FLAG}");
        std::process::exit(1);
    }

    let removed = sshub::purge_database()?;
    if removed.is_empty() {
        println!("Nothing to purge - no database found.");
    } else {
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
        r#"sshub — SSHub TUI SSH host launcher

USAGE:
    sshub [OPTIONS]                         Launch TUI (default)
    sshub <command> [args]                  Headless CLI subcommands

OPTIONS:
    -h, --help              Print help
    -V, --version           Print version
        --dry-run           Exit immediately (smoke / CI)

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
    sshub import                            Import ~/.ssh/config hosts
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
"#
    );
}
