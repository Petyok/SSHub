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
        "exec" => print_exec(),
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
        "theme" => print_theme_help(None),
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

fn print_exec() {
    println!(
        r#"sshub exec - run one command on a saved host and return its exit code

USAGE:
    sshub exec <host> [OPTIONS] -- <command> [args...]
    sshub exec <host> [OPTIONS] "<command>"

OPTIONS:
    --tty                 Force a PTY (-tt) for commands that insist on one
    --timeout SECS        Kill the command after SECS and exit 124, like timeout(1)
    --format plain|json   json buffers the run as {{host, command, exit_code,
                          stdout, stderr, duration_ms}}; plain (default) streams
    -v, --verbose         Verbose ssh logging on stderr

Shell operators belong to whichever shell reads them: `sshub exec web -- ls &&
uptime` runs `ls` on the host and `uptime` at home, exactly as `ssh` does. Quote
the whole thing to send it all remotely: `sshub exec web -- 'ls && uptime'`. A
full-screen command (vim, top, less) needs `--tty`; without it there is no
terminal on the far side and it will complain or misdraw.

stdout/stderr pass through, stdin is inherited so pipes work, and the exit code
is the remote command's (ssh's own failures keep ssh's 255). Never prompts: with
no stored credential exec runs ssh in BatchMode, so an unknown host key fails
instead of waiting for a human. A per-host or ssh_config remote command is
overridden by the command given here. Session transcripts are skipped for exec —
script(1) wrapping fights redirection; use `sshub connect` when you want one.
Mosh hosts are refused: mosh has no one-shot command mode. Runs show up in
`sshub audit list --via exec`."#
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
    sshub audit list  [--status all|ok|fail|retry] [--via all|connect|tunnel|agent|exec]
                      [--host NAME] [--limit N] [--days N] [--format plain|json]
    sshub audit stats [--days N] [--via all|connect|tunnel|agent|exec] [--include-retry]
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

/// Help for `sshub theme`, optionally narrowed to one of its subcommands.
///
/// `theme` is dispatched before the database-backed CLI, so this is printed
/// without ever constructing a `CliContext`.
pub fn print_theme_help(sub: Option<&str>) {
    match sub {
        Some("check") => println!(
            r#"sshub theme check - validate a theme file without installing it

USAGE:
    sshub theme check <file> [--format plain|json]

The file's own directory stands in for the themes directory, so a portable
package whose child inherits from a sibling can be checked as the package it
is; a bare file name means the working directory. Unrelated *.toml files there
are read as candidate themes. A parent that came from a sibling is reported as
a warning, because installing the child alone would leave it unresolvable.

EXIT CODES:
    0  valid, possibly with warnings
    1  validation or file error
    2  wrong usage"#
        ),
        Some("list") => println!(
            r#"sshub theme list - list built-in and installed themes

USAGE:
    sshub theme list [--format plain|json]

Lists every theme the app knows, including invalid ones and user files that
collide with a reserved built-in id, each with its state and reason. A readable
themes directory always exits 0; an unreadable one exits 1."#
        ),
        Some("show") => println!(
            r#"sshub theme show - print a theme's source or its resolved form

USAGE:
    sshub theme show <id> [--resolved] [--format toml|json]

Without --resolved the theme file is printed verbatim, comments included, which
is the documented copy workflow:

    sshub theme show aqua > ~/.config/sshub/themes/aqua-custom.toml
    sshub theme check ~/.config/sshub/themes/aqua-custom.toml

--resolved writes a standalone document instead: no inheritance, no references,
every value spelled out. An unknown id exits 1."#
        ),
        _ => println!(
            r#"sshub theme - inspect and validate themes (headless; no database)

USAGE:
    sshub theme check <file> [--format plain|json]
    sshub theme list         [--format plain|json]
    sshub theme show  <id>   [--resolved] [--format toml|json]

User themes live in ~/.config/sshub/themes/*.toml; the file stem is the theme
id. Run `sshub theme <subcommand> --help` for details."#
        ),
    }
}

fn print_completions() {
    println!(
        r#"sshub completions - generate shell completion scripts

USAGE:
    sshub completions bash|zsh|fish [--cache PATH]"#
    );
}
