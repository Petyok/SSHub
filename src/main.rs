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

    // Subcommands must be handled before the TUI launch path.
    if args.first().map(String::as_str) == Some("db") {
        return run_db(&args[1..]);
    }

    if args.iter().any(|a| a == "--dry-run") {
        return Ok(());
    }
    sshub::run()
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
    sshub [OPTIONS]
    sshub db purge [{CONFIRM_FLAG}]

OPTIONS:
    -h, --help              Print help
        --dry-run           Exit immediately (smoke / CI)
        {CONFIRM_FLAG}   Confirm a destructive command (e.g. db purge)

COMMANDS:
    db purge                Delete the launcher database (managed hosts, groups,
                            identities, tunnels, audit log). Irreversible -
                            requires {CONFIRM_FLAG}. Leaves ~/.ssh/config alone.

ENVIRONMENT:
    SSHUB_CONFIG_DIR          Override config directory (fallback: SSH_LAUNCHER_CONFIG_DIR)
    SSHUB_DATA_DIR            Override data directory (fallback: SSH_LAUNCHER_DATA_DIR)
    SSHUB_SSH_CONFIG          Override SSH config path (fallback: SSH_LAUNCHER_SSH_CONFIG)
    SSHUB_DRY_RUN             Exit immediately (fallback: SSH_LAUNCHER_DRY_RUN)
    SSHUB_AUTO_QUIT           Headless smoke (fallback: SSH_LAUNCHER_AUTO_QUIT): 1 = quit after first draw, q = quit via q key
"#
    );
}
