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
    sshub host list    [--tag TAG]... [--group GROUP] [--sort MODE] [--format plain|json]
    sshub host show    <name> [--format plain|json]
    sshub host connect <name> [-v|--verbose]
    sshub host resolve <name> [--format plain|json]
    sshub host search  <query> [--format plain|json]
    sshub host add     --name NAME --address ADDR [--port N] [--username U] [--identity NAME]
                       [--group NAME] [--tag TAG]... [--proxy-jump SPEC] [--transport ssh|mosh] ...
    sshub host edit    --name NAME [--set-FIELD ... | --clear-FIELD ...]
    sshub host rename  --name NAME --new-name NEW [--strict]
    sshub host delete  --name NAME --yes
    sshub host duplicate <name>

--sort MODE: label|last-connected|favorite|group|manual. Run `man sshub` for the
full add/edit flag list."#
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
    sshub identity list                  [--format plain|json]
    sshub identity show   <name>         [--format plain|json]
    sshub identity add    --name NAME [--username U] [--private-key PATH]
                          [--certificate PATH] [--password-stdin]
    sshub identity edit   --name NAME [--set-name ...] [--set-username ...] [--clear-username]
                          [--set-private-key ...] [--clear-private-key]
                          [--set-certificate ...] [--clear-certificate]
                          [--password-stdin] [--clear-password]
    sshub identity delete --name NAME --yes
    sshub identity agent-remove --name NAME"#
    );
}

fn print_tunnel() {
    println!(
        r#"sshub tunnel - manage SSH tunnels

USAGE:
    sshub tunnel list                [--format plain|json]
    sshub tunnel show   <id>         [--format plain|json]
    sshub tunnel create --host NAME --type local|remote|dynamic --local-port P
                        [--remote-host H] [--remote-port P] [--label L] [--keep-alive]
    sshub tunnel start  <id>         [--foreground]
    sshub tunnel stop   <id>
    sshub tunnel delete <id> --yes

<id> accepts a tunnel id, label, or local port. Detached tunnels record a PID
file and are not visible to the TUI tunnel manager (and vice versa)."#
    );
}

fn print_sftp() {
    println!(
        r#"sshub sftp - one-shot SFTP file operations (direct hosts only; ProxyJump unsupported)

USAGE:
    sshub sftp ls     <host> [remote-path] [--format plain|json]
    sshub sftp get    <host> <remote-path> [local-path] [--recursive]
    sshub sftp put    <host> <local-path> [remote-path] [--recursive]
    sshub sftp rm     <host> <remote-path> [--recursive] --yes
    sshub sftp mkdir  <host> <remote-path>
    sshub sftp rename <host> <from> <to>
    sshub sftp chmod  <host> <octal-mode> <remote-path>

<host> is a saved host name. rm is destructive and requires --yes; --recursive
descends into directories."#
    );
}

fn print_audit() {
    println!(
        r#"sshub audit - inspect the connection audit log

USAGE:
    sshub audit list  [--status all|ok|fail|retry] [--via all|connect|tunnel|agent]
                      [--host NAME] [--limit N] [--days N] [--format plain|json]
    sshub audit stats [--days N] [--via all|connect|tunnel|agent] [--include-retry]
                      [--format plain|json]"#
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
        r#"sshub import - import hosts into the launcher

USAGE:
    sshub import [--from ssh|termius|putty|mremoteng] [--dry-run] [PATH]

SOURCES:
    ssh        (default) import ~/.ssh/config; PATH ignored, --dry-run unsupported
    termius    PATH = export directory containing L00t.csv (default: auto-detected)
    putty      [PATH] = a .reg file or a sessions dir (default: ~/.putty/sessions)
    mremoteng  PATH = confCons.xml

--dry-run previews the hosts that would be imported without writing anything.
Only SSH sessions are imported; encrypted mRemoteNG passwords are not decrypted."#
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
