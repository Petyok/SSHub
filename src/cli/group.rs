//! `sshub group …` — host group CRUD.

use anyhow::Result;
use serde::Serialize;

use crate::app::optional_field;
use crate::store::{HostGroup, HostGroupUpdate, LauncherStore, NewHostGroup};

use super::context::{resolve_identity_id, resolve_parent_id, CliContext};
use super::output::parse_transport;
use super::parse::{fail, parse_format, take_flag, take_opt, usage, OutputFormat, CONFIRM_YES};

#[derive(Serialize)]
struct GroupRecord<'a> {
    id: i64,
    name: &'a str,
    sort_order: i32,
    default_identity_id: Option<i64>,
    default_username: &'a Option<String>,
    default_port: Option<u16>,
    default_proxy_jump: &'a Option<String>,
    default_transport: Option<&'a str>,
    default_forward_agent: Option<bool>,
    parent_id: Option<i64>,
    reserved: bool,
}

pub fn run(ctx: &mut CliContext, args: &[String]) -> Result<i32> {
    let mut rest = args.to_vec();
    match rest.first().map(String::as_str) {
        Some("list") => {
            rest.remove(0);
            cmd_list(ctx, &rest)
        }
        Some("show") => {
            rest.remove(0);
            cmd_show(ctx, &rest)
        }
        Some("add") => {
            rest.remove(0);
            cmd_add(ctx, &rest)
        }
        Some("edit") => {
            rest.remove(0);
            cmd_edit(ctx, &rest)
        }
        Some("delete") => {
            rest.remove(0);
            cmd_delete(ctx, &rest)
        }
        Some("transfer") => {
            rest.remove(0);
            super::transfer::cmd_group_transfer(ctx, &rest)
        }
        Some(other) => {
            eprintln!("sshub: unknown group subcommand '{other}'");
            eprintln!("       try: sshub group list|show|add|edit|delete|transfer");
            Ok(2)
        }
        None => {
            usage("group needs a subcommand (list|show|add|edit|delete|transfer)");
        }
    }
}

fn cmd_list(ctx: &CliContext, args: &[String]) -> Result<i32> {
    let fmt = parse_format(args).map_err(anyhow::Error::msg)?;
    let include_all = args.iter().any(|a| a == "--all");
    let groups: Vec<HostGroup> = ctx
        .store
        .list_groups()?
        .into_iter()
        .filter(|g| include_all || !g.reserved)
        .collect();

    match fmt {
        OutputFormat::Plain => {
            for g in &groups {
                let marker = if g.reserved { " (reserved)" } else { "" };
                println!("{:<6} {}{}", g.id, g.name, marker);
            }
        }
        OutputFormat::Json => {
            let records: Vec<GroupRecord<'_>> = groups.iter().map(group_record).collect();
            println!("{}", serde_json::to_string(&records)?);
        }
    }
    Ok(0)
}

fn cmd_show(ctx: &CliContext, args: &[String]) -> Result<i32> {
    let fmt = parse_format(args).map_err(anyhow::Error::msg)?;
    let name = group_name_arg(args);
    let group = ctx.group_by_name(&name)?;

    match fmt {
        OutputFormat::Plain => print_group_plain(&group, ctx.store.as_ref())?,
        OutputFormat::Json => {
            let record = group_record(&group);
            println!("{}", serde_json::to_string(&record)?);
        }
    }
    Ok(0)
}

fn cmd_add(ctx: &mut CliContext, args: &[String]) -> Result<i32> {
    let mut rest = args.to_vec();
    let name = take_opt(&mut rest, "--name").unwrap_or_else(|| usage("group add requires --name"));
    if name.trim().is_empty() {
        fail("group name cannot be empty");
    }

    let parent_id = take_opt(&mut rest, "--parent")
        .map(|p| resolve_parent_id(ctx, &p))
        .transpose()?;
    let default_identity_id = take_opt(&mut rest, "--default-identity")
        .map(|n| resolve_identity_id(ctx, &n))
        .transpose()?;
    let default_username =
        take_opt(&mut rest, "--default-username").and_then(|s| optional_field(&s));
    let default_port = take_opt(&mut rest, "--default-port")
        .map(|p| {
            p.parse::<u16>()
                .map_err(|_| anyhow::anyhow!("invalid --default-port '{p}'"))
                .and_then(|v| {
                    (v > 0)
                        .then_some(v)
                        .ok_or_else(|| anyhow::anyhow!("invalid --default-port '{p}'"))
                })
        })
        .transpose()?;
    let default_proxy_jump =
        take_opt(&mut rest, "--default-proxy-jump").and_then(|s| optional_field(&s));
    let default_transport = take_opt(&mut rest, "--default-transport")
        .map(|s| parse_transport(&s).ok_or_else(|| anyhow::anyhow!("invalid --default-transport")))
        .transpose()?;
    let default_forward_agent = if take_flag(&mut rest, "--default-fwd-agent") {
        Some(true)
    } else if take_flag(&mut rest, "--no-default-fwd-agent") {
        Some(false)
    } else {
        None
    };
    let sort_order = match take_opt(&mut rest, "--sort-order") {
        Some(s) => s
            .parse::<i32>()
            .map_err(|_| anyhow::anyhow!("invalid sort order '{s}'"))?,
        None => ctx.store.list_groups()?.len() as i32,
    };

    ctx.store.create_group(&NewHostGroup {
        name: name.trim().to_string(),
        sort_order,
        default_identity_id,
        default_username,
        default_port,
        default_proxy_jump,
        default_transport,
        default_forward_agent,
        parent_id,
    })?;
    ctx.reload_hosts()?;
    println!("created group '{}'", name.trim());
    Ok(0)
}

fn cmd_edit(ctx: &mut CliContext, args: &[String]) -> Result<i32> {
    let mut rest = args.to_vec();
    let name = take_opt(&mut rest, "--name").unwrap_or_else(|| usage("group edit requires --name"));
    let group = ctx.group_by_name(&name)?;

    let set_name = take_opt(&mut rest, "--set-name");
    if let Some(n) = &set_name {
        if n.trim().is_empty() {
            fail("group name cannot be empty");
        }
    }
    let set_sort = match take_opt(&mut rest, "--set-sort-order") {
        Some(s) => Some(
            s.parse::<i32>()
                .map_err(|_| anyhow::anyhow!("invalid sort order '{s}'"))?,
        ),
        None => None,
    };

    let mut parent_id: Option<Option<i64>> = None;
    if take_flag(&mut rest, "--clear-parent") {
        parent_id = Some(None);
    } else if let Some(p) = take_opt(&mut rest, "--set-parent") {
        parent_id = Some(Some(resolve_parent_id(ctx, &p)?));
    }

    let mut default_identity_id: Option<Option<i64>> = None;
    if take_flag(&mut rest, "--clear-default-identity") {
        default_identity_id = Some(None);
    } else if let Some(id_name) = take_opt(&mut rest, "--set-default-identity") {
        default_identity_id = Some(Some(resolve_identity_id(ctx, &id_name)?));
    }

    let mut default_username: Option<Option<String>> = None;
    if take_flag(&mut rest, "--clear-default-username") {
        default_username = Some(None);
    } else if let Some(v) = take_opt(&mut rest, "--set-default-username") {
        default_username = Some(optional_field(&v));
    }

    let mut default_port: Option<Option<u16>> = None;
    if take_flag(&mut rest, "--clear-default-port") {
        default_port = Some(None);
    } else if let Some(p) = take_opt(&mut rest, "--set-default-port") {
        let v: u16 = p
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid --set-default-port '{p}'"))?;
        if v == 0 {
            fail("invalid --set-default-port '0'");
        }
        default_port = Some(Some(v));
    }

    let mut default_proxy_jump: Option<Option<String>> = None;
    if take_flag(&mut rest, "--clear-default-proxy-jump") {
        default_proxy_jump = Some(None);
    } else if let Some(v) = take_opt(&mut rest, "--set-default-proxy-jump") {
        default_proxy_jump = Some(optional_field(&v));
    }

    let mut default_transport: Option<Option<crate::session_transport::SessionTransport>> = None;
    if take_flag(&mut rest, "--clear-default-transport") {
        default_transport = Some(None);
    } else if let Some(v) = take_opt(&mut rest, "--set-default-transport") {
        default_transport =
            Some(Some(parse_transport(&v).ok_or_else(|| {
                anyhow::anyhow!("invalid --set-default-transport")
            })?));
    }

    let mut default_forward_agent: Option<Option<bool>> = None;
    if take_flag(&mut rest, "--clear-default-fwd-agent") {
        default_forward_agent = Some(None);
    } else if take_flag(&mut rest, "--set-default-fwd-agent") {
        default_forward_agent = Some(Some(true));
    } else if take_flag(&mut rest, "--no-default-fwd-agent") {
        default_forward_agent = Some(Some(false));
    }
    if group.reserved && set_name.is_some() {
        eprintln!("sshub: reserved group '{}' cannot be renamed", group.name);
        return Ok(1);
    }

    let updated = ctx.store.update_group(
        group.id,
        &HostGroupUpdate {
            name: set_name,
            sort_order: set_sort,
            default_identity_id,
            default_username,
            default_port,
            default_proxy_jump,
            default_transport,
            default_forward_agent,
            parent_id,
        },
    )?;
    if updated.is_none() {
        fail(&format!("group '{name}' not found"));
    }
    ctx.reload_hosts()?;
    println!("updated group '{name}'");
    Ok(0)
}

fn cmd_delete(ctx: &mut CliContext, args: &[String]) -> Result<i32> {
    let mut rest = args.to_vec();
    let name =
        take_opt(&mut rest, "--name").unwrap_or_else(|| usage("group delete requires --name"));
    if !take_flag(&mut rest, CONFIRM_YES) {
        fail(&format!(
            "refusing to delete group '{name}' without {CONFIRM_YES}"
        ));
    }

    let group = ctx.group_by_name(&name)?;
    if group.reserved {
        eprintln!("sshub: reserved group '{}' cannot be deleted", group.name);
        return Ok(1);
    }

    if !ctx.store.delete_group(group.id)? {
        fail(&format!("group '{name}' not found"));
    }
    ctx.reload_hosts()?;
    println!("deleted group '{name}'");
    Ok(0)
}

fn group_name_arg(args: &[String]) -> String {
    let mut rest = args.to_vec();
    if let Some(n) = take_opt(&mut rest, "--name") {
        return n;
    }
    match super::parse::positional(args).first() {
        Some(n) => (*n).to_string(),
        None => usage("group show requires a group name"),
    }
}
fn group_record(g: &HostGroup) -> GroupRecord<'_> {
    GroupRecord {
        id: g.id,
        name: &g.name,
        sort_order: g.sort_order,
        default_identity_id: g.default_identity_id,
        default_username: &g.default_username,
        default_port: g.default_port,
        default_proxy_jump: &g.default_proxy_jump,
        default_transport: g.default_transport.map(|t| t.label()),
        default_forward_agent: g.default_forward_agent,
        parent_id: g.parent_id,
        reserved: g.reserved,
    }
}

fn print_group_plain(group: &HostGroup, store: &LauncherStore) -> Result<()> {
    println!("id:                 {}", group.id);
    println!("name:               {}", group.name);
    println!("sort_order:         {}", group.sort_order);
    println!("reserved:           {}", group.reserved);
    if let Some(pid) = group.parent_id {
        let parent = store
            .get_group(pid)?
            .map(|g| g.name)
            .unwrap_or_else(|| "?".into());
        println!("parent:             {parent} ({pid})");
    } else {
        println!("parent:             (top level)");
    }
    if let Some(iid) = group.default_identity_id {
        let ident = store
            .get_identity(iid)?
            .map(|i| i.name)
            .unwrap_or_else(|| "?".into());
        println!("default_identity:   {ident} ({iid})");
    } else {
        println!("default_identity:   (none)");
    }
    println!(
        "default_username:   {}",
        group.default_username.as_deref().unwrap_or("(none)")
    );
    println!(
        "default_port:       {}",
        group
            .default_port
            .map(|p| p.to_string())
            .unwrap_or_else(|| "(none)".into())
    );
    println!(
        "default_proxy_jump: {}",
        group.default_proxy_jump.as_deref().unwrap_or("(none)")
    );
    println!(
        "default_transport:  {}",
        group
            .default_transport
            .map(|t| t.label())
            .unwrap_or("(none)")
    );
    println!(
        "default_fwd_agent:  {}",
        group
            .default_forward_agent
            .map(|f| if f { "on" } else { "off" })
            .unwrap_or("(none)")
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::store::LauncherStore;

    #[test]
    fn list_hides_favorites_by_default() {
        let store = Arc::new(LauncherStore::open_in_memory().unwrap());
        store
            .create_group(&NewHostGroup {
                name: "prod".into(),
                ..Default::default()
            })
            .unwrap();
        let fav_id = store.favorites_group_id().unwrap();
        let ctx = test_ctx(store);
        let code = cmd_list(&ctx, &[]).unwrap();
        assert_eq!(code, 0);
        let names: Vec<String> = ctx
            .store
            .list_groups()
            .unwrap()
            .into_iter()
            .filter(|g| !g.reserved)
            .map(|g| g.name)
            .collect();
        assert!(names.contains(&"prod".to_string()));
        assert!(!names.iter().any(|n| n == "Favorites"));
        let _ = fav_id;
    }

    fn test_ctx(store: Arc<LauncherStore>) -> CliContext {
        CliContext {
            config: crate::config::AppConfig::default(),
            store,
            metadata: Arc::new(crate::metadata::MetadataDb::default()),
            resolver: crate::ssh::SshConfigResolver::default(),
            password_store: Box::new(crate::credentials::OsKeyring),
            hosts: Vec::new(),
            profile: crate::profile::ProfilePaths {
                data_root: std::path::PathBuf::new(),
                id: "test".into(),
                name: "test".into(),
                root: std::path::PathBuf::new(),
                config_file: std::path::PathBuf::new(),
                ssh_config: std::path::PathBuf::new(),
                compat: true,
            },
        }
    }
}
