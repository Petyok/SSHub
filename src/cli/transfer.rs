//! `sshub <entity> transfer …` — cross-profile copy/move.
//!
//! Thin dispatch over the engine contract in [`crate::store::transfer`]:
//! this module never duplicates plan/apply logic, it only resolves the
//! source/destination stores, prints the plan, gates on explicit
//! confirmation, and reports the per-item outcome.
//!
//! Engine constructors are the only way snapshots are built here
//! (`TransferSnapshot::capture`, `DestIndex::capture`): this module never
//! touches snapshot fields. `TransferSelection` is matched with the canonical
//! engine grammar; tunnel tokens are normalized to a label or `tunnel#<id>`
//! in `cmd_tunnel_transfer` before planning.

use std::collections::HashSet;
use std::io::{self, BufRead, Write};

use anyhow::{Context, Result};

use super::context::CliContext;
use super::parse::{take_flag, take_opt, usage, CONFIRM_YES};
use crate::store::transfer::{
    apply_transfer_plan, build_transfer_plan, DestIndex, GroupTransferMode, TransferMode,
    TransferPlan, TransferSelection, TransferSnapshot,
};
use crate::store::LauncherStore;
use crate::tunnel::{ensure_tunnel_pid_dir, tunnel_runtime_state, TunnelRuntimeState};

/// Parsed `transfer` flags shared by all four entity subcommands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferArgs {
    /// `--to <profile>` destination profile name.
    pub dest_profile: String,
    /// `--move`: delete source originals after copying.
    pub move_mode: bool,
    /// `--with-hosts` (group only): transfer member hosts too.
    pub with_hosts: bool,
    /// `--yes`: apply without the interactive prompt (plan still prints).
    pub auto_yes: bool,
}

/// Parse `--to <profile> [--move] [--with-hosts] [--yes]` from `args`.
/// Returns `Err` (plain message) when `--to` is missing so unit tests can
/// assert usage errors without process exit.
pub fn parse_transfer_args(args: &[String]) -> Result<TransferArgs, String> {
    let mut rest = args.to_vec();
    let dest_profile = take_opt(&mut rest, "--to")
        .ok_or_else(|| "transfer requires --to <profile>".to_string())?;
    if dest_profile.trim().is_empty() {
        return Err("transfer requires a non-empty --to <profile>".to_string());
    }
    Ok(TransferArgs {
        dest_profile,
        move_mode: take_flag(&mut rest, "--move"),
        with_hosts: take_flag(&mut rest, "--with-hosts"),
        auto_yes: take_flag(&mut rest, CONFIRM_YES),
    })
}

/// The confirmation gate is unconditional: a transfer plan always needs an
/// explicit yes (either `--yes` or the interactive `y/N` prompt).
pub fn needs_confirm(_plan: &TransferPlan) -> bool {
    true
}

/// One-line-per-item rendering of a [`TransferPlan`]: the dest profile,
/// `src -> dest` renames, and per-item blocked reasons.
pub fn render_plan_text(plan: &TransferPlan) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Transfer to profile '{}': {} item(s)\n",
        plan.dest_profile,
        plan.items.len()
    ));
    for item in &plan.items {
        if let Some(reason) = &item.blocked {
            out.push_str(&format!(
                "  {} '{}' blocked: {}\n",
                item.kind, item.src_name, reason
            ));
        } else if item.renamed {
            out.push_str(&format!(
                "  {} '{}' -> '{}' (renamed)\n",
                item.kind, item.src_name, item.dest_name
            ));
        } else {
            out.push_str(&format!(
                "  {} '{}' -> '{}'\n",
                item.kind, item.src_name, item.dest_name
            ));
        }
    }
    out
}

/// `host transfer --name NAME --to PROFILE [--move] [--yes]`.
pub fn cmd_host_transfer(ctx: &mut CliContext, args: &[String]) -> Result<i32> {
    let mut rest = args.to_vec();
    let name =
        take_opt(&mut rest, "--name").unwrap_or_else(|| usage("host transfer requires --name"));
    let targs = parse_transfer_args(&rest).unwrap_or_else(|msg| usage(&msg));
    // Fail fast on an unknown host before touching the dest profile.
    let managed = ctx.managed_host_by_name(&name)?;
    let selection = TransferSelection {
        host_names: vec![managed.name],
        group_names: Vec::new(),
        identity_names: Vec::new(),
        tunnel_names: Vec::new(),
    };
    run_transfer(ctx, &selection, GroupTransferMode::GroupOnly, &targs)
}

/// `group transfer --name NAME --to PROFILE [--move] [--with-hosts] [--yes]`.
pub fn cmd_group_transfer(ctx: &mut CliContext, args: &[String]) -> Result<i32> {
    let mut rest = args.to_vec();
    let name =
        take_opt(&mut rest, "--name").unwrap_or_else(|| usage("group transfer requires --name"));
    let targs = parse_transfer_args(&rest).unwrap_or_else(|msg| usage(&msg));
    let group = ctx.group_by_name(&name)?;
    if group.reserved {
        eprintln!(
            "sshub: reserved group '{}' cannot be transferred",
            group.name
        );
        return Ok(1);
    }
    let group_mode = if targs.with_hosts {
        GroupTransferMode::WithHosts
    } else {
        GroupTransferMode::GroupOnly
    };
    let selection = TransferSelection {
        host_names: Vec::new(),
        group_names: vec![group.name],
        identity_names: Vec::new(),
        tunnel_names: Vec::new(),
    };
    run_transfer(ctx, &selection, group_mode, &targs)
}

/// `identity transfer --name NAME --to PROFILE [--move] [--yes]`.
/// Only the key *path* is carried; secret material is never copied.
pub fn cmd_identity_transfer(ctx: &mut CliContext, args: &[String]) -> Result<i32> {
    let mut rest = args.to_vec();
    let name =
        take_opt(&mut rest, "--name").unwrap_or_else(|| usage("identity transfer requires --name"));
    let targs = parse_transfer_args(&rest).unwrap_or_else(|msg| usage(&msg));
    let identity = ctx.identity_by_name(&name)?;
    let selection = TransferSelection {
        host_names: Vec::new(),
        group_names: Vec::new(),
        identity_names: vec![identity.name],
        tunnel_names: Vec::new(),
    };
    run_transfer(ctx, &selection, GroupTransferMode::GroupOnly, &targs)
}

/// `tunnel transfer <id|label|port> --to PROFILE [--move] [--yes]`.
/// The token grammar matches the other `tunnel` subcommands, plus the
/// canonical `tunnel#<id>` / `:<port>` forms the engine accepts everywhere.
pub fn cmd_tunnel_transfer(ctx: &mut CliContext, args: &[String]) -> Result<i32> {
    let mut rest = args.to_vec();
    let targs = parse_transfer_args(&rest).unwrap_or_else(|msg| usage(&msg));
    // Strip transfer flags (with values) so a `--to <profile>` value is never
    // mistaken for the tunnel token regardless of flag order.
    let _ = take_opt(&mut rest, "--to");
    take_flag(&mut rest, "--move");
    take_flag(&mut rest, "--with-hosts");
    take_flag(&mut rest, CONFIRM_YES);
    let token = rest
        .iter()
        .find(|a| !a.starts_with('-'))
        .cloned()
        .unwrap_or_else(|| usage("tunnel transfer requires <id|label|port>"));
    let tunnel = resolve_transfer_tunnel(ctx, &token)?;
    // The engine matches `tunnel_names` with the canonical grammar: the label
    // when the tunnel has one, `tunnel#<id>` otherwise (never the bare id,
    // which would collide with a port token).
    let keys = match tunnel.label {
        Some(label) if !label.trim().is_empty() => vec![label],
        _ => vec![format!("tunnel#{}", tunnel.id)],
    };
    let selection = TransferSelection {
        host_names: Vec::new(),
        group_names: Vec::new(),
        identity_names: Vec::new(),
        tunnel_names: keys,
    };
    run_transfer(ctx, &selection, GroupTransferMode::GroupOnly, &targs)
}

/// Resolve a tunnel token in the canonical transfer grammar: `tunnel#<id>`
/// selects by id, `:<port>` by local port, anything else goes through the
/// shared id/label/port resolver.
fn resolve_transfer_tunnel(ctx: &CliContext, token: &str) -> Result<crate::store::Tunnel> {
    if let Some(rest) = token.strip_prefix("tunnel#") {
        let id: i64 = rest
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid tunnel token '{token}'"))?;
        return ctx
            .store
            .get_tunnel(id)?
            .with_context(|| format!("tunnel {id} not found"));
    }
    if let Some(rest) = token.strip_prefix(':') {
        let port: u16 = rest
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid tunnel token '{token}'"))?;
        let matches = ctx.store.find_tunnels_by_local_port(port)?;
        return match matches.len() {
            0 => anyhow::bail!("no tunnel found for local-port {port}"),
            1 => Ok(matches.into_iter().next().expect("single match")),
            n => anyhow::bail!("ambiguous local-port {port}: {n} tunnels match"),
        };
    }
    ctx.resolve_tunnel(token)
}

fn run_transfer(
    ctx: &mut CliContext,
    selection: &TransferSelection,
    group_mode: GroupTransferMode,
    targs: &TransferArgs,
) -> Result<i32> {
    if ctx.profile.name == targs.dest_profile {
        anyhow::bail!(
            "cannot transfer into the current profile '{}'",
            targs.dest_profile
        );
    }
    let dest_paths = dest_profile_paths(ctx, &targs.dest_profile)?;

    // Open both stores directly (fresh handles): `ctx.store` is shared via
    // `Arc` and the engine needs `&mut`, plus the two stores must never be
    // locked at once — snapshot DTOs are built before any dest write.
    let src_store = LauncherStore::open(ctx.profile.launcher_db())?;
    let mut dest_store = LauncherStore::open(dest_paths.launcher_db())?;

    let snapshot = capture_snapshot(&src_store)?;
    let dest_existing = capture_dest_index(&dest_store, &targs.dest_profile)?;
    let blocked_hosts = running_tunnel_hosts(ctx, &src_store);

    let mut plan = build_transfer_plan(
        &snapshot,
        &dest_existing,
        selection,
        transfer_mode(targs.move_mode),
        group_mode,
        &blocked_hosts,
    );
    plan.dest_profile = targs.dest_profile.clone();
    let text = render_plan_text(&plan);
    print!("{text}");
    io::stdout().flush()?;

    // Refuse an empty plan: confirming zero items would silently "succeed"
    // while transferring nothing (e.g. an unlabeled tunnel selected with a
    // token the engine cannot resolve).
    if plan.runnable().is_empty() {
        println!("nothing to transfer — selection matched no items");
        return Ok(1);
    }

    if needs_confirm(&plan) && !targs.auto_yes && !confirm_interactive()? {
        println!("cancelled");
        return Ok(1);
    }

    // Copy path resolution only happens inside the engine; secrets stay put.
    // Engine failures (rolled-back destination, untouched source) propagate
    // as an error exit instead of a partial success.
    let mut src_store = src_store;
    let result = apply_transfer_plan(
        &mut src_store,
        &mut dest_store,
        &plan,
        transfer_mode(targs.move_mode),
    )?;

    println!(
        "transferred {} item(s) to profile '{}'",
        result.transferred, targs.dest_profile
    );
    for renamed in &result.renamed {
        println!("  renamed: {renamed}");
    }
    for blocked in &result.blocked {
        println!("  blocked: {blocked}");
    }
    ctx.reload_hosts()?;
    Ok(0)
}

/// Fresh mode value per call: the contract does not promise `Copy`.
fn transfer_mode(is_move: bool) -> TransferMode {
    if is_move {
        TransferMode::Move
    } else {
        TransferMode::Copy
    }
}

/// Snapshot the source store via the engine constructor (never field literals).
fn capture_snapshot(store: &LauncherStore) -> Result<TransferSnapshot> {
    TransferSnapshot::capture(store)
}

fn capture_dest_index(store: &LauncherStore, dest_profile: &str) -> Result<DestIndex> {
    DestIndex::capture(store, dest_profile)
}

/// Hosts blocked from transfer: any host with a running detached tunnel in
/// the source profile. Headless CLI has no live-session view, so running
/// tunnels are the blocking signal available here.
fn running_tunnel_hosts(ctx: &CliContext, src: &LauncherStore) -> HashSet<String> {
    let Ok(pid_dir) = ensure_tunnel_pid_dir(ctx.profile.tunnel_base()) else {
        return HashSet::new();
    };
    let (Ok(tunnels), Ok(hosts)) = (src.list_tunnels(), src.list_hosts()) else {
        return HashSet::new();
    };
    let running: HashSet<i64> = tunnels
        .iter()
        .filter(|t| {
            tunnel_runtime_state(t.id, t.local_port, &pid_dir) == TunnelRuntimeState::Running
        })
        .filter_map(|t| t.host_id)
        .collect();
    hosts
        .into_iter()
        .filter(|h| running.contains(&h.id))
        .map(|h| h.name)
        .collect()
}

fn dest_profile_paths(ctx: &CliContext, name: &str) -> Result<crate::profile::ProfilePaths> {
    if ctx.profile.compat {
        anyhow::bail!("transfer --to requires profile mode (session bypasses profiles)");
    }
    let roots = crate::profile::resolve_roots()?;
    let state = crate::profile::ProfileState::load(&roots.data_root)?
        .with_context(|| "no profiles found".to_string())?;
    let record = state
        .by_name(name)
        .with_context(|| format!("profile '{name}' not found"))?;
    // SSH config is machine-local; reuse the current resolution for dest.
    Ok(crate::profile::profile_paths(
        &roots,
        record,
        ctx.profile.ssh_config.clone(),
    ))
}

fn confirm_interactive() -> Result<bool> {
    print!("Transfer the items above? [y/N]: ");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    Ok(matches!(line.trim().to_lowercase().as_str(), "y" | "yes"))
}
