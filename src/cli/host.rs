//! `sshub host` subcommand dispatch.

use std::io::{self, Read};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

use crate::app::{
    optional_field, parse_tags, prepare_cli_connect_argv, resolve_pending_secret,
    session_argv_for_entry, SortMode,
};
use crate::cli::context::CliContext;
use crate::cli::filter::apply_filters;
use crate::cli::output::{
    format_host_list_plain, format_host_plain, format_resolve_plain, host_record_json,
    host_resolve_json, now_ts, parse_session_logging, parse_transport,
};
use crate::cli::parse::{self, OutputFormat};
use crate::hosts::duplicate_legacy_to_launcher;
use crate::metadata::{HostMetadata, MetadataStore};
use crate::search::HostSearch;
use crate::session::askpass::AskpassSecret;
use crate::session_log::{allocate_log_path, effective_enabled, wrap_script_command};
use crate::session_transport::SessionTransport;
use crate::ssh::materialize_ssh_config_host;
use crate::store::{DeleteHostOutcome, HostSource, HostUpdate, NewHost};

pub fn run_host(ctx: &mut CliContext, args: &[String]) -> Result<i32> {
    let Some(sub) = args.first().map(String::as_str) else {
        host_usage("missing host subcommand");
    };
    let rest = &args[1..];
    match sub {
        "list" => cmd_list(ctx, rest),
        "show" => cmd_show(ctx, rest),
        "resolve" => cmd_resolve(ctx, rest),
        "search" => cmd_search(ctx, rest),
        "connect" => cmd_connect(ctx, rest),
        "add" => cmd_add(ctx, rest),
        "edit" => cmd_edit(ctx, rest),
        "rename" => cmd_rename(ctx, rest),
        "delete" => cmd_delete(ctx, rest),
        "duplicate" => cmd_duplicate(ctx, rest),
        other => {
            eprintln!("sshub: unknown host subcommand '{other}'");
            host_usage(
                "try: list, show, connect, resolve, search, add, edit, rename, delete, duplicate",
            );
        }
    }
}

fn cmd_list(ctx: &CliContext, args: &[String]) -> Result<i32> {
    let mut args = args.to_vec();
    let tags = parse::take_all(&mut args, "--tag");
    let group = parse::take_opt(&mut args, "--group");
    let sort = parse::parse_sort(args.as_slice())
        .map_err(anyhow::Error::msg)?
        .unwrap_or(SortMode::Label);
    let fmt = parse::parse_format(args.as_slice()).map_err(anyhow::Error::msg)?;

    let indices = apply_filters(&ctx.hosts, &ctx.store, &tags, group.as_deref(), sort)?;

    match fmt {
        OutputFormat::Json => {
            let records: Vec<_> = indices
                .iter()
                .map(|&i| host_record_json(&ctx.hosts[i], &ctx.store))
                .collect();
            println!("{}", serde_json::to_string_pretty(&records)?);
        }
        OutputFormat::Plain => {
            let names: Vec<&str> = indices.iter().map(|&i| ctx.hosts[i].name()).collect();
            if !names.is_empty() {
                println!("{}", format_host_list_plain(&names));
            }
        }
    }
    Ok(0)
}

fn cmd_show(ctx: &CliContext, args: &[String]) -> Result<i32> {
    let fmt = parse::parse_format(args).map_err(anyhow::Error::msg)?;
    let pos = parse::positional(args);
    let Some(name) = pos.first() else {
        host_usage("show requires <name>");
    };
    let entry = ctx.host_by_name(name)?;
    let record = host_record_json(entry, &ctx.store);
    match fmt {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&record)?),
        OutputFormat::Plain => println!("{}", format_host_plain(&record)),
    }
    Ok(0)
}

fn cmd_resolve(ctx: &CliContext, args: &[String]) -> Result<i32> {
    let mut args = args.to_vec();
    let verbose = parse::take_flag(&mut args, "--verbose") || parse::take_flag(&mut args, "-v");
    let fmt = parse::parse_format(&args).map_err(anyhow::Error::msg)?;
    let pos = parse::positional(&args);
    let Some(name) = pos.first() else {
        host_usage("resolve requires <name>");
    };
    let entry = ctx.host_by_name(name)?;
    let resolve = host_resolve_json(entry, &ctx.store, ctx.password_store.as_ref(), verbose);
    match fmt {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&resolve)?),
        OutputFormat::Plain => println!("{}", format_resolve_plain(&resolve)),
    }
    Ok(0)
}

fn cmd_search(ctx: &CliContext, args: &[String]) -> Result<i32> {
    let fmt = parse::parse_format(args).map_err(anyhow::Error::msg)?;
    let pos = parse::positional(args);
    let Some(query) = pos.first() else {
        host_usage("search requires <query>");
    };
    let mut search = HostSearch::new();
    let indices = search.update_query(&ctx.hosts, query);
    match fmt {
        OutputFormat::Json => {
            let records: Vec<_> = indices
                .iter()
                .map(|&i| host_record_json(&ctx.hosts[i], &ctx.store))
                .collect();
            println!("{}", serde_json::to_string_pretty(&records)?);
        }
        OutputFormat::Plain => {
            let names: Vec<&str> = indices.iter().map(|&i| ctx.hosts[i].name()).collect();
            if !names.is_empty() {
                println!("{}", format_host_list_plain(&names));
            }
        }
    }
    Ok(0)
}

fn cmd_connect(ctx: &mut CliContext, args: &[String]) -> Result<i32> {
    let mut args = args.to_vec();
    let verbose = parse::take_flag(&mut args, "--verbose") || parse::take_flag(&mut args, "-v");
    let pos = parse::positional(&args);
    let Some(name) = pos.first() else {
        host_usage("connect requires <name>");
    };

    let entry = ctx.host_by_name(name)?.clone();
    let host_name = entry.name().to_string();
    let (pending_secret, _) = resolve_pending_secret(&entry, ctx.password_store.as_ref());
    let mut argv = prepare_cli_connect_argv(
        session_argv_for_entry(&entry),
        pending_secret.is_some(),
        verbose,
    );

    if let Some(cmd) = argv.first() {
        if Command::new("which")
            .arg(cmd)
            .output()
            .map(|o| !o.status.success())
            .unwrap_or(true)
        {
            eprintln!("sshub: command not found: '{cmd}'");
            return Ok(1);
        }
    }

    let is_mosh = matches!(entry.session_transport(), SessionTransport::Mosh);
    let log_enabled = effective_enabled(
        ctx.config.session_logging.enabled,
        entry.session_logging_override(),
    );

    let mut log_path: Option<String> = None;
    if log_enabled && !is_mosh {
        if let Ok(data_dir) = crate::config::data_dir() {
            match allocate_log_path(&data_dir, &host_name, entry.managed_id()) {
                Ok(path) => {
                    if let Some(wrapped) = wrap_script_command(&path, &argv) {
                        argv = wrapped;
                        log_path = Some(path.parent().unwrap_or(&path).display().to_string());
                    } else {
                        eprintln!(
                            "sshub: warning: session logging requested but script(1) unavailable; continuing without transcript"
                        );
                    }
                }
                Err(e) => {
                    eprintln!("sshub: warning: session logging unavailable: {e:#}");
                }
            }
        }
    } else if log_enabled && is_mosh {
        eprintln!("sshub: warning: session logging skipped for mosh transport");
    }

    let mut askpass_guard = None;
    let mut extra_env: Vec<(String, String)> = Vec::new();
    if let Some(secret) = pending_secret.as_ref() {
        if let Ok(exe) = std::env::current_exe() {
            if let Ok(guard) = AskpassSecret::new(secret.value()) {
                extra_env = guard.env(&exe);
                askpass_guard = Some(guard);
            }
        }
    }

    // For legacy ssh_config hosts entry.managed() is None, so fall back to the
    // resolved SshHost (User / ProxyJump from ~/.ssh/config) rather than logging
    // username=None and via=direct.
    let ssh = entry.ssh_host();
    let username = entry
        .managed()
        .and_then(|m| {
            m.username
                .as_deref()
                .or_else(|| m.identity.as_ref().and_then(|i| i.username.as_deref()))
        })
        .or(ssh.user.as_deref());
    let via = entry
        .managed()
        .and_then(|m| m.proxy_jump.as_deref())
        .or(ssh.proxy_jump.as_deref())
        .unwrap_or("direct");

    let program = argv.first().context("empty connect argv")?;
    let mut cmd = Command::new(program);
    cmd.args(&argv[1..]);
    cmd.stdin(Stdio::inherit());
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());
    for (k, v) in &extra_env {
        cmd.env(k, v);
    }

    let mut child = match cmd.spawn() {
        Ok(child) => {
            let _ = ctx.store.log_auth_event(
                &host_name,
                username,
                via,
                "launched",
                "cli connect",
                log_path.as_deref(),
            );
            child
        }
        Err(e) => {
            let msg = format!("spawn failed: {e:#}");
            let _ = ctx
                .store
                .log_auth_event(&host_name, username, via, "fail", &msg, None);
            eprintln!("sshub: {msg}");
            return Ok(1);
        }
    };

    let status = child.wait().context("wait connect child")?;
    let timestamp = now_ts();

    if let Some(id) = entry.managed_id() {
        ctx.store.set_host_last_connected(id, timestamp)?;
        if let Some(idx) = ctx.hosts.iter().position(|e| e.managed_id() == Some(id)) {
            if let crate::app::HostEntry::Managed(m) = &mut ctx.hosts[idx] {
                m.last_connected = Some(timestamp);
            }
        }
    } else {
        ctx.metadata.set_last_connected(entry.name(), timestamp)?;
        if let Some(idx) = ctx.hosts.iter().position(|e| e.name() == entry.name()) {
            if let Some((_, meta)) = ctx.hosts[idx].legacy_mut() {
                meta.last_connected = Some(timestamp);
            }
        }
    }

    drop(askpass_guard);
    Ok(status.code().unwrap_or(1))
}

fn cmd_add(ctx: &mut CliContext, args: &[String]) -> Result<i32> {
    let spec = HostAddSpec::parse(args)?;
    let unique_name = ctx.store.unique_host_name(&spec.name, None)?;
    if unique_name != spec.name {
        eprintln!("sshub: name '{}' taken, using '{}'", spec.name, unique_name);
    }

    let identity_id = spec
        .identity
        .as_deref()
        .map(|n| ctx.identity_by_name(n))
        .transpose()?
        .map(|i| i.id);

    let group_ids = resolve_group_ids(ctx, &spec.groups)?;

    let created = ctx.store.create_host(&NewHost {
        name: unique_name.clone(),
        label: spec.label,
        address: spec.address,
        port: spec.port,
        group_id: group_ids.first().copied(),
        identity_id,
        os_icon: spec.os_icon,
        tags: spec.tags,
        notes: spec.notes,
        proxy_jump: spec.proxy_jump,
        forward_agent: spec.forward_agent,
        remote_command: spec.remote_command,
        source: HostSource::Launcher,
        has_password: spec.password.is_some(),
        username: spec.username,
        session_logging: spec.session_logging,
        transport: spec.transport,
    })?;

    ctx.store.set_host_groups(created.id, &group_ids)?;

    if let Some(pw) = spec.password {
        if let Err(e) = ctx
            .password_store
            .set(&crate::credentials::host_key(created.id), &pw)
        {
            eprintln!("sshub: warning: storing password failed: {e:#}");
        }
    }

    if spec.favorite {
        let fav_id = ctx.store.favorites_group_id()?;
        ctx.store.add_host_to_group(created.id, fav_id)?;
    }

    ctx.reload_hosts()?;
    println!("{}", created.name);
    Ok(0)
}

fn cmd_edit(ctx: &mut CliContext, args: &[String]) -> Result<i32> {
    let patch = HostEditPatch::parse(args)?;
    let name = patch.name.clone();

    let needs_materialize = patch.needs_materialize();
    let metadata_only = patch.metadata_only();

    if needs_materialize && ctx.host_by_name(&name)?.managed().is_none() {
        let materialized =
            materialize_ssh_config_host(&ctx.resolver, &ctx.store, ctx.metadata.as_ref(), &name)?;
        if !materialized {
            bail!("host '{name}' could not be materialized");
        }
        ctx.reload_hosts()?;
    }

    let entry = ctx.host_by_name(&name)?.clone();

    if metadata_only {
        return apply_metadata_edit(ctx, &entry, &patch);
    }

    let managed = entry
        .managed()
        .ok_or_else(|| anyhow::anyhow!("host '{name}' has no managed row for connection edits"))?
        .clone();

    if managed.source == HostSource::SshConfig && patch.changes_connection_fields() {
        bail!("connection fields are read-only for ssh_config hosts");
    }

    let mut update = HostUpdate::default();

    if let Some(v) = patch.set_label {
        update.label = Some(v);
    }
    if let Some(ref v) = patch.set_name {
        update.name = Some(v.clone());
    }
    if let Some(v) = patch.set_address {
        update.address = Some(v);
    }
    if let Some(v) = patch.set_port {
        update.port = Some(v);
    }
    if let Some(v) = patch.set_username {
        update.username = Some(v);
    }
    if patch.clear_username {
        update.username = Some(None);
    }
    if let Some(v) = patch.set_tags {
        update.tags = Some(v);
    }
    if let Some(v) = patch.set_notes {
        update.notes = Some(v);
    }
    if patch.clear_notes {
        update.notes = Some(None);
    }
    if let Some(v) = patch.set_environment {
        update.environment = Some(v);
    }
    if patch.clear_environment {
        update.environment = Some(None);
    }
    if let Some(v) = patch.set_proxy_jump {
        update.proxy_jump = Some(v);
    }
    if patch.clear_proxy_jump {
        update.proxy_jump = Some(None);
    }
    if let Some(v) = patch.set_remote_command {
        update.remote_command = Some(v);
    }
    if patch.clear_remote_command {
        update.remote_command = Some(None);
    }
    if let Some(v) = patch.set_forward_agent {
        update.forward_agent = Some(v);
    }
    if let Some(v) = patch.set_os_icon {
        update.os_icon = Some(v);
    }
    if patch.clear_os_icon {
        update.os_icon = Some(None);
    }
    if let Some(v) = patch.set_session_logging {
        update.session_logging = Some(v);
    }
    if let Some(v) = patch.set_transport {
        update.transport = Some(v);
    }
    if let Some(v) = patch.favorite {
        update.favorite = Some(v);
    }
    if let Some(v) = patch.set_identity {
        let identity = ctx.identity_by_name(&v)?;
        update.identity_id = Some(Some(identity.id));
    }
    if patch.clear_identity {
        update.identity_id = Some(None);
    }
    if let Some(v) = patch.set_has_password {
        update.has_password = Some(v);
    }

    let saved_name = patch
        .set_name
        .clone()
        .unwrap_or_else(|| managed.name.clone());

    if let Some(ref new_name) = patch.set_name {
        let unique = ctx.store.unique_host_name(new_name, Some(managed.id))?;
        if unique != *new_name {
            eprintln!("sshub: name taken, using '{unique}'");
            update.name = Some(unique);
        } else {
            update.name = Some(unique);
        }
    }

    ctx.store.update_host(managed.id, &update)?;

    if let Some(groups) = patch.set_groups {
        let group_ids = resolve_group_ids(ctx, &groups)?;
        ctx.store.set_host_groups(managed.id, &group_ids)?;
    }

    if let Some(pw) = patch.password {
        if let Err(e) = ctx
            .password_store
            .set(&crate::credentials::host_key(managed.id), &pw)
        {
            eprintln!("sshub: warning: storing password failed: {e:#}");
        }
    } else if patch.clear_password {
        let _ = ctx
            .password_store
            .delete(&crate::credentials::host_key(managed.id));
    }

    ctx.reload_hosts()?;
    let out_name = update.name.as_deref().unwrap_or(saved_name.as_str());
    println!("{out_name}");
    Ok(0)
}

fn apply_metadata_edit(
    ctx: &mut CliContext,
    entry: &crate::app::HostEntry,
    patch: &HostEditPatch,
) -> Result<i32> {
    let host_name = entry.name().to_string();

    if let Some(managed) = entry.managed() {
        let id = managed.id;
        let mut update = HostUpdate::default();
        if let Some(v) = patch.set_tags.clone() {
            update.tags = Some(v);
        }
        if let Some(v) = patch.set_notes.clone() {
            update.notes = Some(v);
        }
        if patch.clear_notes {
            update.notes = Some(None);
        }
        if let Some(v) = patch.set_environment.clone() {
            update.environment = Some(v);
        }
        if patch.clear_environment {
            update.environment = Some(None);
        }
        if let Some(v) = patch.set_session_logging {
            update.session_logging = Some(v);
        }
        if let Some(v) = patch.favorite {
            update.favorite = Some(v);
        }
        if let Some(v) = patch.set_label.clone() {
            update.label = Some(v);
        }
        if let Some(v) = patch.set_transport {
            update.transport = Some(v);
        }
        ctx.store.update_host(id, &update)?;
        if let Some(groups) = patch.set_groups.clone() {
            let group_ids = resolve_group_ids(ctx, &groups)?;
            ctx.store.set_host_groups(id, &group_ids)?;
        }
        if let Some(v) = patch.set_identity.clone() {
            let identity = ctx.identity_by_name(&v)?;
            ctx.store.update_host(
                id,
                &HostUpdate {
                    identity_id: Some(Some(identity.id)),
                    ..Default::default()
                },
            )?;
        }
        ctx.reload_hosts()?;
        println!("{host_name}");
        return Ok(0);
    }

    let favorite = patch.favorite.unwrap_or(entry.favorite());
    let meta = HostMetadata {
        host_name: host_name.clone(),
        tags: patch
            .set_tags
            .clone()
            .unwrap_or_else(|| entry.tags().to_vec()),
        description: if patch.clear_notes {
            None
        } else {
            patch
                .set_notes
                .clone()
                .flatten()
                .or_else(|| entry.description().map(str::to_string))
        },
        environment: if patch.clear_environment {
            None
        } else {
            patch
                .set_environment
                .clone()
                .flatten()
                .or_else(|| entry.environment().map(str::to_string))
        },
        favorite,
        last_connected: entry.last_connected(),
        session_logging: patch
            .set_session_logging
            .unwrap_or(entry.session_logging_override()),
        transport: patch.set_transport.unwrap_or(entry.session_transport()),
    };
    ctx.metadata.upsert(&meta)?;
    ctx.reload_hosts()?;
    println!("{host_name}");
    Ok(0)
}

fn cmd_rename(ctx: &mut CliContext, args: &[String]) -> Result<i32> {
    let mut args = args.to_vec();
    let strict = parse::take_flag(&mut args, "--strict");
    let name = parse::take_opt(&mut args, "--name")
        .ok_or_else(|| anyhow::anyhow!("rename requires --name"))?;
    let new_name = parse::take_opt(&mut args, "--new-name")
        .ok_or_else(|| anyhow::anyhow!("rename requires --new-name"))?;

    let managed = ctx.managed_host_by_name(&name)?.clone();
    if managed.source != HostSource::Launcher {
        bail!("only launcher hosts can be renamed");
    }

    let target = if strict {
        if ctx.store.get_host_by_name(&new_name)?.is_some() {
            bail!("host name '{new_name}' already exists");
        }
        new_name.clone()
    } else {
        ctx.store.unique_host_name(&new_name, Some(managed.id))?
    };

    ctx.store.update_host(
        managed.id,
        &HostUpdate {
            name: Some(target.clone()),
            ..Default::default()
        },
    )?;
    ctx.reload_hosts()?;
    println!("{target}");
    Ok(0)
}

fn cmd_delete(ctx: &mut CliContext, args: &[String]) -> Result<i32> {
    let mut args = args.to_vec();
    if !parse::take_flag(&mut args, "--yes") {
        eprintln!("sshub: delete requires --yes");
        return Ok(1);
    }
    let name = parse::take_opt(&mut args, "--name")
        .ok_or_else(|| anyhow::anyhow!("delete requires --name"))?;

    let managed = ctx.managed_host_by_name(&name)?;
    match ctx.store.delete_host(managed.id)? {
        DeleteHostOutcome::Deleted => {
            ctx.reload_hosts()?;
            Ok(0)
        }
        DeleteHostOutcome::NotFound => {
            eprintln!("sshub: host not found");
            Ok(1)
        }
        DeleteHostOutcome::NotLauncher => {
            eprintln!("sshub: only launcher-managed hosts can be deleted");
            Ok(2)
        }
    }
}

fn cmd_duplicate(ctx: &mut CliContext, args: &[String]) -> Result<i32> {
    let mut args = args.to_vec();
    let fmt = parse::parse_format(&args).map_err(anyhow::Error::msg)?;
    let name = parse::take_opt(&mut args, "--name")
        .ok_or_else(|| anyhow::anyhow!("duplicate requires --name"))?;
    let new_name = parse::take_opt(&mut args, "--new-name");

    let entry = ctx.host_by_name(&name)?.clone();
    let copy_name = match &entry {
        crate::app::HostEntry::Managed(m) => {
            if let Some(requested) = new_name {
                let unique = ctx.store.unique_host_name(&requested, None)?;
                let copy = ctx.store.duplicate_host(m.id)?.context("host not found")?;
                if unique != copy.name {
                    ctx.store.update_host(
                        copy.id,
                        &HostUpdate {
                            name: Some(unique.clone()),
                            ..Default::default()
                        },
                    )?;
                    unique
                } else {
                    copy.name
                }
            } else {
                ctx.store
                    .duplicate_host(m.id)?
                    .context("host not found")?
                    .name
            }
        }
        crate::app::HostEntry::Legacy { host, meta } => {
            if let Some(requested) = new_name {
                let mut h = host.clone();
                h.name = requested.clone();
                let mut m = meta.clone();
                m.host_name = requested.clone();
                duplicate_legacy_to_launcher(&ctx.store, &h, &m)?
            } else {
                duplicate_legacy_to_launcher(&ctx.store, host, meta)?
            }
        }
    };

    ctx.reload_hosts()?;
    let record = host_record_json(ctx.host_by_name(&copy_name)?, &ctx.store);
    match fmt {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&record)?),
        OutputFormat::Plain => println!("{}", record.name),
    }
    Ok(0)
}

fn resolve_group_ids(ctx: &CliContext, names: &[String]) -> Result<Vec<i64>> {
    names
        .iter()
        .map(|n| ctx.group_by_name(n).map(|g| g.id))
        .collect()
}

struct HostAddSpec {
    name: String,
    address: String,
    port: u16,
    label: Option<String>,
    groups: Vec<String>,
    identity: Option<String>,
    username: Option<String>,
    tags: Vec<String>,
    notes: Option<String>,
    proxy_jump: Option<String>,
    forward_agent: bool,
    remote_command: Option<String>,
    transport: SessionTransport,
    session_logging: crate::session_log::SessionLoggingOverride,
    os_icon: Option<String>,
    favorite: bool,
    password: Option<String>,
}

impl HostAddSpec {
    fn parse(args: &[String]) -> Result<Self> {
        let mut args = args.to_vec();
        let name = parse::take_opt(&mut args, "--name")
            .ok_or_else(|| anyhow::anyhow!("add requires --name"))?;
        let address = parse::take_opt(&mut args, "--address")
            .ok_or_else(|| anyhow::anyhow!("add requires --address"))?;
        let port = parse::take_opt(&mut args, "--port")
            .map(|p| p.parse())
            .transpose()
            .map_err(|_| anyhow::anyhow!("invalid --port"))?
            .unwrap_or(22);
        let label = parse::take_opt(&mut args, "--label").and_then(|s| optional_field(&s));
        let groups = parse::take_all(&mut args, "--group");
        let identity = parse::take_opt(&mut args, "--identity");
        let username = parse::take_opt(&mut args, "--username").and_then(|s| optional_field(&s));
        let tags = parse::take_opt(&mut args, "--tags")
            .map(|t| parse_tags(&t))
            .unwrap_or_default();
        let notes = parse::take_opt(&mut args, "--notes").and_then(|s| optional_field(&s));
        let proxy_jump =
            parse::take_opt(&mut args, "--proxy-jump").and_then(|s| optional_field(&s));
        let forward_agent = if parse::take_flag(&mut args, "--no-forward-agent") {
            false
        } else {
            parse::take_flag(&mut args, "--forward-agent")
        };
        let remote_command =
            parse::take_opt(&mut args, "--remote-command").and_then(|s| optional_field(&s));
        let transport = parse::take_opt(&mut args, "--transport")
            .and_then(|s| parse_transport(&s))
            .unwrap_or(SessionTransport::Ssh);
        let session_logging = parse::take_opt(&mut args, "--session-log")
            .and_then(|s| parse_session_logging(&s))
            .unwrap_or(crate::session_log::SessionLoggingOverride::Inherit);
        let os_icon = parse::take_opt(&mut args, "--os-icon").and_then(|s| optional_field(&s));
        let favorite = parse::take_flag(&mut args, "--favorite");
        let password = if parse::take_flag(&mut args, "--password-stdin") {
            Some(read_password_stdin()?)
        } else {
            None
        };
        Ok(Self {
            name,
            address,
            port,
            label,
            groups,
            identity,
            username,
            tags,
            notes,
            proxy_jump,
            forward_agent,
            remote_command,
            transport,
            session_logging,
            os_icon,
            favorite,
            password,
        })
    }
}

struct HostEditPatch {
    name: String,
    set_label: Option<Option<String>>,
    set_name: Option<String>,
    set_address: Option<String>,
    set_port: Option<u16>,
    set_username: Option<Option<String>>,
    clear_username: bool,
    set_tags: Option<Vec<String>>,
    set_notes: Option<Option<String>>,
    clear_notes: bool,
    set_environment: Option<Option<String>>,
    clear_environment: bool,
    set_proxy_jump: Option<Option<String>>,
    clear_proxy_jump: bool,
    set_remote_command: Option<Option<String>>,
    clear_remote_command: bool,
    set_forward_agent: Option<bool>,
    set_os_icon: Option<Option<String>>,
    clear_os_icon: bool,
    set_session_logging: Option<crate::session_log::SessionLoggingOverride>,
    set_transport: Option<SessionTransport>,
    favorite: Option<bool>,
    set_identity: Option<String>,
    clear_identity: bool,
    set_groups: Option<Vec<String>>,
    set_has_password: Option<bool>,
    password: Option<String>,
    clear_password: bool,
}

impl HostEditPatch {
    fn parse(args: &[String]) -> Result<Self> {
        let mut args = args.to_vec();
        let name = parse::take_opt(&mut args, "--name")
            .ok_or_else(|| anyhow::anyhow!("edit requires --name"))?;

        let mut patch = Self {
            name,
            set_label: None,
            set_name: None,
            set_address: None,
            set_port: None,
            set_username: None,
            clear_username: false,
            set_tags: None,
            set_notes: None,
            clear_notes: false,
            set_environment: None,
            clear_environment: false,
            set_proxy_jump: None,
            clear_proxy_jump: false,
            set_remote_command: None,
            clear_remote_command: false,
            set_forward_agent: None,
            set_os_icon: None,
            clear_os_icon: false,
            set_session_logging: None,
            set_transport: None,
            favorite: None,
            set_identity: None,
            clear_identity: false,
            set_groups: None,
            set_has_password: None,
            password: None,
            clear_password: false,
        };

        if let Some(v) = parse::take_opt(&mut args, "--set-label") {
            patch.set_label = Some(optional_field(&v));
        }
        if let Some(v) = parse::take_opt(&mut args, "--set-name") {
            patch.set_name = Some(v);
        }
        if let Some(v) = parse::take_opt(&mut args, "--set-address") {
            patch.set_address = Some(v);
        }
        if let Some(v) = parse::take_opt(&mut args, "--set-port") {
            patch.set_port = Some(v.parse().context("invalid --set-port")?);
        }
        if let Some(v) = parse::take_opt(&mut args, "--set-username") {
            patch.set_username = Some(optional_field(&v));
        }
        patch.clear_username = parse::take_flag(&mut args, "--clear-username");
        if let Some(v) = parse::take_opt(&mut args, "--set-tags") {
            patch.set_tags = Some(parse_tags(&v));
        }
        if let Some(v) = parse::take_opt(&mut args, "--set-notes") {
            patch.set_notes = Some(optional_field(&v));
        }
        patch.clear_notes = parse::take_flag(&mut args, "--clear-notes");
        if let Some(v) = parse::take_opt(&mut args, "--set-environment") {
            patch.set_environment = Some(optional_field(&v));
        }
        patch.clear_environment = parse::take_flag(&mut args, "--clear-environment");
        if let Some(v) = parse::take_opt(&mut args, "--set-proxy-jump") {
            patch.set_proxy_jump = Some(optional_field(&v));
        }
        patch.clear_proxy_jump = parse::take_flag(&mut args, "--clear-proxy-jump");
        if let Some(v) = parse::take_opt(&mut args, "--set-remote-command") {
            patch.set_remote_command = Some(optional_field(&v));
        }
        patch.clear_remote_command = parse::take_flag(&mut args, "--clear-remote-command");
        if parse::take_flag(&mut args, "--set-forward-agent") {
            patch.set_forward_agent = Some(true);
        }
        if parse::take_flag(&mut args, "--no-forward-agent") {
            patch.set_forward_agent = Some(false);
        }
        if let Some(v) = parse::take_opt(&mut args, "--set-os-icon") {
            patch.set_os_icon = Some(optional_field(&v));
        }
        patch.clear_os_icon = parse::take_flag(&mut args, "--clear-os-icon");
        if let Some(v) = parse::take_opt(&mut args, "--set-session-log") {
            patch.set_session_logging = Some(
                parse_session_logging(&v)
                    .ok_or_else(|| anyhow::anyhow!("invalid --set-session-log"))?,
            );
        }
        if let Some(v) = parse::take_opt(&mut args, "--set-transport") {
            patch.set_transport = Some(
                parse_transport(&v).ok_or_else(|| anyhow::anyhow!("invalid --set-transport"))?,
            );
        }
        if parse::take_flag(&mut args, "--favorite") {
            patch.favorite = Some(true);
        }
        if parse::take_flag(&mut args, "--no-favorite") {
            patch.favorite = Some(false);
        }
        if let Some(v) = parse::take_opt(&mut args, "--set-identity") {
            patch.set_identity = Some(v);
        }
        patch.clear_identity = parse::take_flag(&mut args, "--clear-identity");
        let groups = parse::take_all(&mut args, "--set-group");
        if !groups.is_empty() {
            patch.set_groups = Some(groups);
        }
        if parse::take_flag(&mut args, "--set-password-stdin") {
            patch.password = Some(read_password_stdin()?);
            patch.set_has_password = Some(true);
        }
        patch.clear_password = parse::take_flag(&mut args, "--clear-password");

        if !patch.has_any_patch() {
            bail!("edit requires at least one --set-* flag");
        }
        Ok(patch)
    }

    fn has_any_patch(&self) -> bool {
        self.set_label.is_some()
            || self.set_name.is_some()
            || self.set_address.is_some()
            || self.set_port.is_some()
            || self.set_username.is_some()
            || self.clear_username
            || self.set_tags.is_some()
            || self.set_notes.is_some()
            || self.clear_notes
            || self.set_environment.is_some()
            || self.clear_environment
            || self.set_proxy_jump.is_some()
            || self.clear_proxy_jump
            || self.set_remote_command.is_some()
            || self.clear_remote_command
            || self.set_forward_agent.is_some()
            || self.set_os_icon.is_some()
            || self.clear_os_icon
            || self.set_session_logging.is_some()
            || self.set_transport.is_some()
            || self.favorite.is_some()
            || self.set_identity.is_some()
            || self.clear_identity
            || self.set_groups.is_some()
            || self.password.is_some()
            || self.clear_password
    }

    fn needs_materialize(&self) -> bool {
        self.changes_connection_fields()
            || self.set_groups.is_some()
            || self.set_identity.is_some()
            || self.clear_identity
            || self.set_label.is_some()
            || self.touches_managed_fields()
    }

    fn metadata_only(&self) -> bool {
        !self.changes_connection_fields()
            && self.set_groups.is_none()
            && self.set_identity.is_none()
            && !self.clear_identity
            && self.password.is_none()
            && !self.clear_password
            && self.set_label.is_none()
            && !self.touches_managed_fields()
    }

    /// Managed-row fields that `apply_metadata_edit` does not handle, so an edit
    /// touching them must route through the full-edit path (and materialize a
    /// legacy host first). Kept out of `changes_connection_fields` so they do
    /// not trip the ssh_config connection-field read-only guard.
    fn touches_managed_fields(&self) -> bool {
        self.set_username.is_some()
            || self.clear_username
            || self.set_os_icon.is_some()
            || self.clear_os_icon
    }

    fn changes_connection_fields(&self) -> bool {
        self.set_name.is_some()
            || self.set_address.is_some()
            || self.set_port.is_some()
            || self.set_proxy_jump.is_some()
            || self.clear_proxy_jump
            || self.set_forward_agent.is_some()
            || self.set_remote_command.is_some()
            || self.clear_remote_command
    }
}

fn read_password_stdin() -> Result<String> {
    let mut buf = String::new();
    io::stdin()
        .read_to_string(&mut buf)
        .context("read password from stdin")?;
    Ok(buf.trim_end_matches(['\r', '\n']).to_string())
}

fn host_usage(msg: &str) -> ! {
    eprintln!("sshub: {msg}");
    std::process::exit(2);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(args: &[&str]) -> HostEditPatch {
        let v: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        HostEditPatch::parse(&v).unwrap()
    }

    #[test]
    fn username_only_edit_routes_through_full_path() {
        // Regression: --set-username alone must NOT be treated as metadata-only
        // (apply_metadata_edit drops it); it must route to the full-edit path
        // and materialize a legacy host first.
        let p = patch(&["--name", "h", "--set-username", "bob"]);
        assert!(!p.metadata_only());
        assert!(p.needs_materialize());
    }

    #[test]
    fn os_icon_only_edit_routes_through_full_path() {
        let p = patch(&["--name", "h", "--set-os-icon", "linux"]);
        assert!(!p.metadata_only());
        assert!(p.needs_materialize());
    }

    #[test]
    fn clear_username_only_edit_routes_through_full_path() {
        let p = patch(&["--name", "h", "--clear-username"]);
        assert!(!p.metadata_only());
        assert!(p.needs_materialize());
    }

    #[test]
    fn tags_only_edit_stays_metadata_only() {
        let p = patch(&["--name", "h", "--set-tags", "a,b"]);
        assert!(p.metadata_only());
    }
}
