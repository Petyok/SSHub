pub mod audit;
pub mod completions;
pub mod context;
pub mod filter;
pub mod group;
pub mod help;
pub mod host;
pub mod identity;
pub mod inventory;
pub mod output;
pub mod parse;
pub mod sftp;
pub mod tunnel;

pub use context::CliContext;

use anyhow::Result;

/// True if `cmd` is a recognized CLI subcommand or alias. Called by main.rs
/// BEFORE bootstrap (cheap string check). Must NOT bootstrap a CliContext.
pub fn is_subcommand(cmd: &str) -> bool {
    matches!(
        cmd,
        "host"
            | "connect"
            | "list"
            | "tunnel"
            | "group"
            | "groups"
            | "identity"
            | "tags"
            | "sync"
            | "import"
            | "export"
            | "completions"
            | "sftp"
            | "audit"
    )
}

/// Dispatch a subcommand. `rest` is argv AFTER the subcommand token. main.rs
/// owns bootstrap and passes `ctx` in; this fn must NOT call CliContext::bootstrap.
pub fn run_subcommand(ctx: &mut CliContext, cmd: &str, rest: &[String]) -> Result<i32> {
    if rest.iter().any(|a| a == "--help" || a == "-h") {
        help::print_command_help(cmd);
        return Ok(0);
    }
    match cmd {
        "host" => host::run_host(ctx, rest),
        // aliases:
        "connect" => host::run_host(ctx, &prepend("connect", rest)),
        "list" => host::run_host(ctx, &prepend("list", rest)),
        "groups" => group::run(ctx, &prepend("list", rest)),

        "tunnel" => tunnel::run(ctx, rest),
        "group" => group::run(ctx, rest),
        "identity" => identity::run(ctx, rest),
        "tags" => inventory::run_tags(ctx, rest),
        "sync" => inventory::run_sync(ctx, rest),
        "import" => inventory::run_import(ctx, rest),
        "export" => inventory::run_export(ctx, rest),
        "completions" => completions::run(ctx, rest),
        "sftp" => sftp::run(ctx, rest),
        "audit" => audit::run(ctx, rest),
        other => anyhow::bail!("unknown subcommand: {other}"),
    }
}

fn prepend(head: &str, rest: &[String]) -> Vec<String> {
    let mut v = Vec::with_capacity(rest.len() + 1);
    v.push(head.to_string());
    v.extend_from_slice(rest);
    v
}
