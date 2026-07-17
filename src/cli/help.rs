//! Per-command help text. `sshub <cmd> --help` prints a USAGE block scoped to
//! just that command instead of falling through to the global `sshub --help`.
//!
//! Wording mirrors `main.rs::print_help`, narrowed to one top-level command.

/// Print a short USAGE block for `cmd`. Handles the same set of commands and
/// aliases that `is_subcommand` accepts. Unrecognized commands get a one-line
/// pointer back to the global help.
pub fn print_command_help(cmd: &str) {
    match cmd {
        "host" => print_host(),
        "connect" => print_connect(),
        "list" => print_list(),
        "groups" => print_groups(),
        "group" => print_group(),
        "identity" => print_identity(),
        "tunnel" => print_tunnel(),
        "sftp" => print_sftp(),
        "audit" => print_audit(),
        "tags" => print_tags(),
        "sync" => print_sync(),
        "import" => print_import(),
        "export" => print_export(),
        "completions" => print_completions(),
        other => {
            println!(
                "sshub: no per-command help for '{other}'; run `sshub --help` for the command list"
            );
        }
    }
}

fn print_host() {
    println!(
        r#"sshub host - manage launcher hosts (read/write)

USAGE:
    sshub host list [--tag TAG]... [--group GROUP] [--sort MODE] [--format plain|json]
    sshub host show <name> [--format plain|json]
    sshub host connect <name> [-v|--verbose]
    sshub host resolve <name> [--format plain|json]
    sshub host search <query> [--format plain|json]
    sshub host add|edit|rename|delete|duplicate ..."#
    );
}

fn print_connect() {
    println!(
        r#"sshub connect - open an SSH session to a host (alias for `host connect`)

USAGE:
    sshub connect <name> [-v|--verbose]"#
    );
}

fn print_list() {
    println!(
        r#"sshub list - list launcher hosts (alias for `host list`)

USAGE:
    sshub list [--tag TAG]... [--group GROUP] [--sort MODE] [--format plain|json]"#
    );
}

fn print_groups() {
    println!(
        r#"sshub groups - list host groups (alias for `group list`, forwards flags)

USAGE:
    sshub groups [--all] [--format plain|json]"#
    );
}

fn print_group() {
    println!(
        r#"sshub group - manage host groups

USAGE:
    sshub group list [--all] [--format plain|json]
    sshub group show <name> [--format plain|json]
    sshub group add --name NAME [--parent GROUP] [--default-identity NAME] [--sort-order N]
    sshub group edit --name NAME [--set-name ...] [--set-parent ...] [--clear-parent]
                     [--set-default-identity ...] [--clear-default-identity] [--set-sort-order N]
    sshub group delete --name NAME --yes"#
    );
}

fn print_identity() {
    println!(
        r#"sshub identity - manage SSH identities

USAGE:
    sshub identity list|show|add|edit|delete|agent-remove ...
    sshub identity agent-remove --name NAME"#
    );
}

fn print_tunnel() {
    println!(
        r#"sshub tunnel - manage SSH tunnels

USAGE:
    sshub tunnel list|show|create|delete|start|stop ..."#
    );
}

fn print_sftp() {
    println!(
        r#"sshub sftp - one-shot SFTP file operations

USAGE:
    sshub sftp ls|get|put|rm|mkdir|rename|chmod ..."#
    );
}

fn print_audit() {
    println!(
        r#"sshub audit - inspect the connection audit log

USAGE:
    sshub audit list [--status STATUS] [--via VIA] [--host HOST] [--limit N] [--days N] [--format plain|json]
    sshub audit stats [--days N] [--via VIA] [--include-retry] [--format plain|json]"#
    );
}

fn print_tags() {
    println!(
        r#"sshub tags - list all tags in the inventory

USAGE:
    sshub tags [--format plain|json]"#
    );
}

fn print_sync() {
    println!(
        r#"sshub sync - refresh ssh_config rows in the database

USAGE:
    sshub sync"#
    );
}

fn print_import() {
    println!(
        r#"sshub import - import hosts from ~/.ssh/config

USAGE:
    sshub import"#
    );
}

fn print_export() {
    println!(
        r#"sshub export - export launcher hosts to an ssh_config snippet

USAGE:
    sshub export [--stdout] [-o PATH]"#
    );
}

fn print_completions() {
    println!(
        r#"sshub completions - generate shell completion scripts

USAGE:
    sshub completions bash|zsh|fish [--cache PATH]"#
    );
}
