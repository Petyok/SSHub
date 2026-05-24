use anyhow::Result;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return Ok(());
    }
    if args.iter().any(|a| a == "--dry-run") {
        return Ok(());
    }
    sshub::run()
}

fn print_help() {
    println!(
        r#"sshub — SSHub TUI SSH host launcher

USAGE:
    sshub [OPTIONS]

OPTIONS:
    -h, --help       Print help
        --dry-run    Exit immediately (smoke / CI)

ENVIRONMENT:
    SSHUB_CONFIG_DIR          Override config directory (fallback: SSH_LAUNCHER_CONFIG_DIR)
    SSHUB_DATA_DIR            Override data directory (fallback: SSH_LAUNCHER_DATA_DIR)
    SSHUB_SSH_CONFIG          Override SSH config path (fallback: SSH_LAUNCHER_SSH_CONFIG)
    SSHUB_DRY_RUN             Exit immediately (fallback: SSH_LAUNCHER_DRY_RUN)
    SSHUB_AUTO_QUIT           Headless smoke (fallback: SSH_LAUNCHER_AUTO_QUIT): 1 = quit after first draw, q = quit via q key
"#
    );
}
