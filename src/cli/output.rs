//! JSON DTOs and plain-text formatters for CLI output.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::app::{
    prepare_cli_connect_argv, resolve_pending_secret, session_argv_for_entry, HostEntry,
};
use crate::credentials::PasswordStore;
use crate::session_log::SessionLoggingOverride;
use crate::session_transport::SessionTransport;
use crate::store::{AuthEvent, HostSource, LauncherStore, Tunnel, TunnelType};

#[derive(Debug, Serialize)]
pub struct HostRecordJson {
    pub name: String,
    pub label: Option<String>,
    pub address: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub group: Option<String>,
    pub groups: Vec<String>,
    pub identity: Option<String>,
    pub proxy_jump: Option<String>,
    pub source: String,
    pub tags: Vec<String>,
    pub environment: Option<String>,
    pub description: Option<String>,
    pub favorite: bool,
    pub last_connected: Option<i64>,
    pub session_logging: String,
    pub transport: String,
    pub forward_agent: Option<bool>,
    pub remote_command: Option<String>,
    pub os_icon: Option<String>,
    pub has_password: bool,
    pub managed_id: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct HostResolveJson {
    #[serde(flatten)]
    pub host: HostRecordJson,
    pub argv: Vec<String>,
    pub has_stored_secret: bool,
}

#[derive(Debug, Serialize)]
pub struct AuditEventJson {
    pub id: i64,
    pub host_name: String,
    pub username: Option<String>,
    pub via: Option<String>,
    pub status: String,
    pub note: Option<String>,
    pub log_path: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Serialize)]
pub struct TunnelJson {
    pub id: i64,
    pub host_id: Option<i64>,
    pub host_name: Option<String>,
    pub tunnel_type: String,
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
    pub label: Option<String>,
    pub auto_connect: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

pub fn host_record_json(entry: &HostEntry, store: &LauncherStore) -> HostRecordJson {
    let ssh = entry.ssh_host();
    let groups = group_names(entry, store);
    let (group, identity, source, has_password, managed_id, os_icon, forward_agent) =
        match entry.managed() {
            Some(m) => (
                m.group.as_ref().map(|g| g.name.clone()),
                m.identity
                    .as_ref()
                    .map(|i| i.name.clone())
                    .or_else(|| ssh.identity_file.clone()),
                m.source.as_str().to_string(),
                m.has_password,
                Some(m.id),
                m.os_icon.clone(),
                Some(m.forward_agent),
            ),
            None => (
                None,
                ssh.identity_file.clone(),
                HostSource::SshConfig.as_str().to_string(),
                false,
                None,
                None,
                ssh.forward_agent,
            ),
        };

    HostRecordJson {
        name: entry.name().to_string(),
        label: entry
            .managed()
            .and_then(|m| m.label.clone())
            .filter(|l| !l.is_empty())
            .or_else(|| {
                if entry.display_name() != entry.name() {
                    Some(entry.display_name().to_string())
                } else {
                    None
                }
            }),
        address: ssh.hostname.clone(),
        user: ssh.user.clone(),
        port: ssh.port,
        group,
        groups,
        identity,
        proxy_jump: ssh.proxy_jump.clone(),
        source,
        tags: entry.tags().to_vec(),
        environment: entry.environment().map(str::to_string),
        description: entry.description().map(str::to_string),
        favorite: entry.favorite(),
        last_connected: entry.last_connected(),
        session_logging: entry.session_logging_override().label().to_string(),
        transport: entry.session_transport().label().to_string(),
        forward_agent,
        remote_command: ssh.remote_command.clone(),
        os_icon,
        has_password,
        managed_id,
    }
}

pub fn host_resolve_json(
    entry: &HostEntry,
    store: &LauncherStore,
    password_store: &dyn PasswordStore,
    verbose: bool,
) -> HostResolveJson {
    let (pending_secret, _) = resolve_pending_secret(entry, password_store);
    let argv = prepare_cli_connect_argv(
        session_argv_for_entry(entry),
        pending_secret.is_some(),
        verbose,
    );
    HostResolveJson {
        host: host_record_json(entry, store),
        argv,
        has_stored_secret: pending_secret.is_some(),
    }
}

pub fn audit_event_json(event: &AuthEvent) -> AuditEventJson {
    AuditEventJson {
        id: event.id,
        host_name: event.host_name.clone(),
        username: event.username.clone(),
        via: event.via.clone(),
        status: event.status.clone(),
        note: event.note.clone(),
        log_path: event.log_path.clone(),
        created_at: event.created_at,
    }
}

pub fn tunnel_json(tunnel: &Tunnel, store: &LauncherStore) -> TunnelJson {
    let host_name = tunnel
        .host_id
        .and_then(|id| store.get_host(id).ok().flatten())
        .map(|h| h.name);
    TunnelJson {
        id: tunnel.id,
        host_id: tunnel.host_id,
        host_name,
        tunnel_type: tunnel_type_cli(tunnel.tunnel_type),
        local_port: tunnel.local_port,
        remote_host: tunnel.remote_host.clone(),
        remote_port: tunnel.remote_port,
        label: tunnel.label.clone(),
        auto_connect: tunnel.auto_connect,
        created_at: tunnel.created_at,
        updated_at: tunnel.updated_at,
    }
}

fn tunnel_type_cli(t: TunnelType) -> String {
    match t {
        TunnelType::Local => "local".into(),
        TunnelType::Remote => "remote".into(),
        TunnelType::Dynamic => "dynamic".into(),
    }
}

fn group_names(entry: &HostEntry, store: &LauncherStore) -> Vec<String> {
    if let Some(m) = entry.managed() {
        return m.groups.iter().map(|g| g.name.clone()).collect();
    }
    store
        .list_groups()
        .ok()
        .map(|groups| {
            groups
                .into_iter()
                .filter(|g| entry.group_ids().contains(&g.id))
                .map(|g| g.name)
                .collect()
        })
        .unwrap_or_default()
}

pub fn format_host_plain(record: &HostRecordJson) -> String {
    let mut lines = vec![
        format!("Host: {}", record.name),
        format!(
            "Label: {}",
            record.label.as_deref().unwrap_or(record.name.as_str())
        ),
        format!("Address: {}", opt_dash(&record.address)),
        format!("User: {}", opt_dash(&record.user)),
        format!(
            "Port: {}",
            record
                .port
                .map(|p| p.to_string())
                .unwrap_or_else(|| "—".into())
        ),
        format!("Group: {}", opt_dash(&record.group)),
        format!(
            "Groups: {}",
            if record.groups.is_empty() {
                "—".into()
            } else {
                record.groups.join(", ")
            }
        ),
        format!("Identity: {}", opt_dash(&record.identity)),
        format!("ProxyJump: {}", opt_dash(&record.proxy_jump)),
        format!("Source: {}", record.source),
        String::new(),
        format!(
            "Tags: {}",
            if record.tags.is_empty() {
                "—".into()
            } else {
                record.tags.join(", ")
            }
        ),
        format!("Environment: {}", opt_dash(&record.environment)),
        format!("Description: {}", opt_dash(&record.description)),
        format!("Favorite: {}", if record.favorite { "yes" } else { "no" }),
        format!(
            "Last connected: {}",
            record
                .last_connected
                .map(|ts| ts.to_string())
                .unwrap_or_else(|| "—".into())
        ),
        format!("Session log: {}", record.session_logging),
        format!("Transport: {}", record.transport),
    ];
    if let Some(fa) = record.forward_agent {
        lines.push(format!("ForwardAgent: {}", fa));
    }
    if let Some(rc) = &record.remote_command {
        if !rc.is_empty() {
            lines.push(format!("RemoteCommand: {rc}"));
        }
    }
    if let Some(icon) = &record.os_icon {
        if !icon.is_empty() {
            lines.push(format!("OS icon: {icon}"));
        }
    }
    lines.join("\n")
}

pub fn format_host_list_plain(names: &[&str]) -> String {
    names.join("\n")
}

pub fn format_resolve_plain(resolve: &HostResolveJson) -> String {
    let mut out = format_host_plain(&resolve.host);
    out.push_str("\n\nCommand:\n");
    out.push_str(&format!("  {}", resolve.argv.join(" ")));
    out.push_str("\n\nHas stored secret: ");
    out.push_str(if resolve.has_stored_secret {
        "yes"
    } else {
        "no"
    });
    out
}

pub fn format_audit_plain(event: &AuditEventJson) -> String {
    format!(
        "{} {} {} via={} status={}{}",
        event.created_at,
        event.host_name,
        event.username.as_deref().unwrap_or("-"),
        event.via.as_deref().unwrap_or("-"),
        event.status,
        event
            .note
            .as_ref()
            .map(|n| format!(" ({n})"))
            .unwrap_or_default()
    )
}

pub fn format_tunnel_plain(tunnel: &TunnelJson) -> String {
    format!(
        "#{} {} {} localhost:{} -> {}:{}{}",
        tunnel.id,
        tunnel.tunnel_type,
        tunnel.host_name.as_deref().unwrap_or("-"),
        tunnel.local_port,
        tunnel.remote_host,
        tunnel.remote_port,
        tunnel
            .label
            .as_ref()
            .map(|l| format!(" [{l}]"))
            .unwrap_or_default()
    )
}

fn opt_dash(value: &Option<String>) -> String {
    value
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("—")
        .to_string()
}

pub fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub fn parse_session_logging(s: &str) -> Option<SessionLoggingOverride> {
    match s {
        "inherit" => Some(SessionLoggingOverride::Inherit),
        "on" => Some(SessionLoggingOverride::On),
        "off" => Some(SessionLoggingOverride::Off),
        _ => None,
    }
}

pub fn parse_transport(s: &str) -> Option<SessionTransport> {
    match s {
        "ssh" => Some(SessionTransport::Ssh),
        "mosh" => Some(SessionTransport::Mosh),
        _ => None,
    }
}
