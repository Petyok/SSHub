//! Tunnel CLI subcommands.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::app::resolve_pending_secret_for_managed;
use crate::cli::context::CliContext;
use crate::cli::parse::{fail, fail_code, parse_format, take_flag, take_opt, OutputFormat};
use crate::store::{NewTunnel, Tunnel, TunnelType};
use crate::tunnel::{
    ensure_tunnel_pid_dir, log_tunnel_reconnect_events, spawn_detached_tunnel,
    stop_detached_tunnel, tunnel_data_dir, tunnel_runtime_state, TunnelManager,
};

pub fn run(ctx: &mut CliContext, args: &[String]) -> Result<i32> {
    match args.first().map(String::as_str) {
        Some("list") => cmd_list(ctx, &args[1..])?,
        Some("show") => cmd_show(ctx, &args[1..])?,
        Some("create") => cmd_create(ctx, &args[1..])?,
        Some("delete") => cmd_delete(ctx, &args[1..])?,
        Some("start") => cmd_start(ctx, &args[1..])?,
        Some("stop") => cmd_stop(ctx, &args[1..])?,
        Some(other) => {
            fail(&format!(
                "unknown tunnel subcommand '{other}' (try: list, show, create, delete, start, stop)"
            ));
        }
        None => fail("tunnel requires a subcommand (list, show, create, delete, start, stop)"),
    }
    Ok(0)
}

fn cmd_list(ctx: &CliContext, args: &[String]) -> Result<()> {
    let fmt = parse_format(args).map_err(anyhow::Error::msg)?;
    let tunnels = ctx.store.list_tunnels()?;
    let pid_dir = ensure_tunnel_pid_dir(&tunnel_data_dir()?)?;

    let rows: Vec<TunnelListRow> = tunnels
        .iter()
        .map(|t| {
            let host_name = t
                .host_id
                .and_then(|hid| ctx.store.get_host(hid).ok().flatten())
                .map(|h| h.name)
                .unwrap_or_else(|| "?".into());
            let state = tunnel_runtime_state(t.id, t.local_port, &pid_dir);
            TunnelListRow {
                id: t.id,
                label: t.label.clone(),
                host: host_name,
                tunnel_type: t.tunnel_type.label().to_string(),
                local_port: t.local_port,
                remote_host: t.remote_host.clone(),
                remote_port: t.remote_port,
                keep_alive: t.auto_connect,
                state: state.as_str().to_string(),
            }
        })
        .collect();

    match fmt {
        OutputFormat::Plain => {
            for row in &rows {
                let label = row.label.as_deref().unwrap_or("");
                println!(
                    "{}\t{}\t:{}\t{}\t{}",
                    row.id, row.state, row.local_port, row.host, label
                );
            }
        }
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&rows)?);
        }
    }
    Ok(())
}

fn cmd_show(ctx: &CliContext, args: &[String]) -> Result<()> {
    let fmt = parse_format(args).map_err(anyhow::Error::msg)?;
    let token = positional_one(args, "show")?;
    let tunnel = ctx.resolve_tunnel(token)?;
    let pid_dir = ensure_tunnel_pid_dir(&tunnel_data_dir()?)?;
    let state = tunnel_runtime_state(tunnel.id, tunnel.local_port, &pid_dir);
    let host_name = tunnel
        .host_id
        .and_then(|hid| ctx.store.get_host(hid).ok().flatten())
        .map(|h| h.name)
        .unwrap_or_else(|| "?".into());

    let row = TunnelShowRow {
        id: tunnel.id,
        label: tunnel.label.clone(),
        host: host_name,
        tunnel_type: tunnel.tunnel_type.label().to_string(),
        local_port: tunnel.local_port,
        remote_host: tunnel.remote_host.clone(),
        remote_port: tunnel.remote_port,
        keep_alive: tunnel.auto_connect,
        state: state.as_str().to_string(),
        created_at: tunnel.created_at,
        updated_at: tunnel.updated_at,
    };

    match fmt {
        OutputFormat::Plain => {
            println!("id: {}", row.id);
            if let Some(ref l) = row.label {
                println!("label: {l}");
            }
            println!("host: {}", row.host);
            println!("type: {}", row.tunnel_type);
            println!("local_port: {}", row.local_port);
            if tunnel.tunnel_type != TunnelType::Dynamic {
                println!("remote: {}:{}", row.remote_host, row.remote_port);
            }
            println!("keep_alive: {}", row.keep_alive);
            println!("state: {}", row.state);
        }
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&row)?);
        }
    }
    Ok(())
}

fn cmd_create(ctx: &mut CliContext, args: &[String]) -> Result<()> {
    let mut a = args.to_vec();
    let host_name =
        take_opt(&mut a, "--host").ok_or_else(|| anyhow::anyhow!("--host is required"))?;
    let tunnel_type =
        parse_tunnel_type(&take_opt(&mut a, "--type").unwrap_or_else(|| "local".into()))?;
    let local_port: u16 = take_opt(&mut a, "--local-port")
        .ok_or_else(|| anyhow::anyhow!("--local-port is required"))?
        .parse()
        .context("invalid --local-port")?;
    let remote_host = take_opt(&mut a, "--remote-host").unwrap_or_else(|| "localhost".into());
    let remote_port: u16 = take_opt(&mut a, "--remote-port")
        .unwrap_or_else(|| "0".into())
        .parse()
        .unwrap_or(0);
    let label = take_opt(&mut a, "--label");
    let keep_alive = take_flag(&mut a, "--keep-alive");

    let managed = ctx.managed_host_by_name(&host_name)?;
    let new = NewTunnel {
        host_id: Some(managed.id),
        tunnel_type,
        local_port,
        remote_host,
        remote_port: if tunnel_type == TunnelType::Dynamic {
            0
        } else {
            remote_port
        },
        label,
        auto_connect: keep_alive,
    };
    let id = ctx.store.create_tunnel(&new)?;
    println!("created tunnel {id} :{local_port}");
    Ok(())
}

fn cmd_delete(ctx: &mut CliContext, args: &[String]) -> Result<()> {
    let mut a = args.to_vec();
    if !take_flag(&mut a, "--yes") {
        fail_code("tunnel delete requires --yes", 1);
    }
    let token = positional_one(&a, "delete")?;
    let tunnel = ctx.resolve_tunnel(token)?;
    let data_dir = tunnel_data_dir()?;
    let _ = stop_detached_tunnel(&data_dir, tunnel.id);
    ctx.store.delete_tunnel(tunnel.id)?;
    println!("deleted tunnel {}", tunnel.id);
    Ok(())
}

fn cmd_start(ctx: &CliContext, args: &[String]) -> Result<()> {
    let mut a = args.to_vec();
    let foreground = take_flag(&mut a, "--foreground");
    let token = positional_one(&a, "start")?;
    let tunnel = ctx.resolve_tunnel(token)?;
    let host = ctx.resolve_tunnel_host(&tunnel)?;
    let (secret, _) = ctx.resolve_tunnel_secret(&host);
    let label = tunnel.label.as_deref().unwrap_or("");
    let host_name = host.name.as_str();

    if foreground {
        return run_foreground(ctx, &tunnel, &host, secret);
    }

    let data_dir = tunnel_data_dir()?;
    let pid = spawn_detached_tunnel(&tunnel, &host, secret.as_ref(), &data_dir)?;
    let _ = ctx.store.log_auth_event(
        host_name,
        None,
        "tunnel",
        "launched",
        &format!("tunnel started (cli) :{} {label}", tunnel.local_port),
        None,
    );
    println!(
        "started tunnel {} pid {pid} :{}",
        tunnel.id, tunnel.local_port
    );
    Ok(())
}

fn run_foreground(
    ctx: &CliContext,
    tunnel: &Tunnel,
    host: &crate::store::ManagedHost,
    secret: Option<crate::session::PendingSecret>,
) -> Result<()> {
    let cfg = ctx.config.tunnel_reconnect.clone();
    let store = Arc::clone(&ctx.store);
    let password_store = ctx.password_store.as_ref();
    let host_name = host.name.clone();
    let label = tunnel.label.clone().unwrap_or_default();
    let local_port = tunnel.local_port;

    let mut mgr = TunnelManager::new();
    mgr.start(tunnel, Some(host), secret.as_ref())
        .context("tunnel start failed")?;
    let _ = store.log_auth_event(
        &host_name,
        None,
        "tunnel",
        "launched",
        &format!("tunnel started (cli foreground) :{local_port} {label}"),
        None,
    );

    loop {
        let tunnels = vec![tunnel.clone()];
        let health = mgr.check_health(&tunnels, &cfg);
        log_tunnel_reconnect_events(&store, &health, &tunnels);

        if mgr.is_gave_up(tunnel.id) {
            fail_code(
                &format!(
                    "tunnel gave up: {}",
                    mgr.error_detail(tunnel.id).unwrap_or("unknown error")
                ),
                1,
            );
        }

        let reconnect = mgr.tick_reconnect(
            &tunnels,
            &cfg,
            |host_id| store.get_host(host_id).ok().flatten(),
            |h| resolve_pending_secret_for_managed(h, password_store).0,
        );
        log_tunnel_reconnect_events(&store, &reconnect, &tunnels);

        if !mgr.has_child(tunnel.id)
            && !mgr.is_reconnecting(tunnel.id)
            && mgr.status(tunnel.id) == "stopped"
        {
            break;
        }

        std::thread::sleep(Duration::from_secs(1));
    }
    Ok(())
}

fn cmd_stop(ctx: &CliContext, args: &[String]) -> Result<()> {
    let token = positional_one(args, "stop")?;
    let tunnel = ctx.resolve_tunnel(token)?;
    let data_dir = tunnel_data_dir()?;
    let stopped = stop_detached_tunnel(&data_dir, tunnel.id)?;
    let host_name = tunnel
        .host_id
        .and_then(|hid| ctx.store.get_host(hid).ok().flatten())
        .map(|h| h.name)
        .unwrap_or_else(|| "unknown".into());
    if stopped {
        let _ = ctx.store.log_auth_event(
            &host_name,
            None,
            "tunnel",
            "ok",
            &format!("tunnel stopped (cli) :{}", tunnel.local_port),
            None,
        );
        println!("stopped tunnel {}", tunnel.id);
    } else {
        println!("tunnel {} was not running", tunnel.id);
    }
    Ok(())
}

fn parse_tunnel_type(s: &str) -> Result<TunnelType> {
    match s {
        "local" | "L" => Ok(TunnelType::Local),
        "remote" | "R" => Ok(TunnelType::Remote),
        "dynamic" | "D" => Ok(TunnelType::Dynamic),
        other => anyhow::bail!("unknown tunnel type '{other}'"),
    }
}

fn positional_one<'a>(args: &'a [String], sub: &str) -> Result<&'a str> {
    let pos: Vec<_> = args
        .iter()
        .filter(|a| !a.starts_with('-'))
        .map(String::as_str)
        .collect();
    pos.first()
        .copied()
        .with_context(|| format!("tunnel {sub} requires an id, label, or local-port"))
}

#[derive(Serialize)]
struct TunnelListRow {
    id: i64,
    label: Option<String>,
    host: String,
    tunnel_type: String,
    local_port: u16,
    remote_host: String,
    remote_port: u16,
    keep_alive: bool,
    state: String,
}

#[derive(Serialize)]
struct TunnelShowRow {
    id: i64,
    label: Option<String>,
    host: String,
    tunnel_type: String,
    local_port: u16,
    remote_host: String,
    remote_port: u16,
    keep_alive: bool,
    state: String,
    created_at: i64,
    updated_at: i64,
}
