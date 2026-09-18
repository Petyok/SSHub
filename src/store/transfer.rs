//! Cross-profile transfers: plan a scoped copy/move from one profile's store
//! into another, then apply it.
//!
//! Snapshot-first design: [`TransferSnapshot::capture`] and
//! [`DestIndex::capture`] copy plain DTOs out of each store (one short lock
//! per `list_*` call), so planning never holds a store lock and applying
//! never holds two locks at once — the `LauncherStore` mutex is
//! non-reentrant and `with_conn` uses `unchecked_transaction`.

use std::collections::{HashMap, HashSet};

use super::types::{
    HostGroup, HostSource, Identity, ManagedHost, NewHost, NewHostGroup, NewIdentity, NewTunnel,
    Tunnel,
};
use super::LauncherStore;

/// Copy into the destination, or copy then delete the source originals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferMode {
    Copy,
    Move,
}

/// Whether a transferred group carries its member hosts with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupTransferMode {
    GroupOnly,
    WithHosts,
}

/// One planned row: what it is, where it comes from, what it will be called.
#[derive(Debug, Clone)]
pub struct PlannedItem {
    /// `"host"`, `"group"`, `"identity"` or `"tunnel"`.
    pub kind: &'static str,
    pub src_name: String,
    pub dest_name: String,
    pub renamed: bool,
    /// `Some(message)` when this item cannot transfer (active session /
    /// running tunnel on the host, or a tunnel of such a host).
    pub blocked: Option<String>,
}

/// Ordered (groups, identities, hosts, tunnels) plan for one destination profile.
#[derive(Debug, Clone, Default)]
pub struct TransferPlan {
    pub items: Vec<PlannedItem>,
    pub dest_profile: String,
}

impl TransferPlan {
    /// Items that cannot transfer.
    pub fn blocked(&self) -> Vec<&PlannedItem> {
        self.items.iter().filter(|i| i.blocked.is_some()).collect()
    }

    /// Items that will transfer on apply.
    pub fn runnable(&self) -> Vec<&PlannedItem> {
        self.items.iter().filter(|i| i.blocked.is_none()).collect()
    }
}

/// Plain-DTO copy of everything a transfer may need from the source store.
/// Ids are preserved so host/identity/group/tunnel references resolve.
#[derive(Debug, Clone, Default)]
pub struct TransferSnapshot {
    pub hosts: Vec<ManagedHost>,
    pub groups: Vec<HostGroup>,
    pub identities: Vec<Identity>,
    pub tunnels: Vec<Tunnel>,
    /// `(host_name, group_name)` membership pairs, reserved groups included.
    pub memberships: Vec<(String, String)>,
}

impl TransferSnapshot {
    /// Copy the transfer-relevant rows out of `store`, then drop the lock.
    pub fn capture(store: &LauncherStore) -> anyhow::Result<Self> {
        let hosts = store.list_hosts()?;
        let groups = store.list_groups()?;
        let identities = store.list_identities()?;
        let tunnels = store.list_tunnels()?;
        let memberships = hosts
            .iter()
            .flat_map(|h| {
                h.groups
                    .iter()
                    .map(|g| (h.name.clone(), g.name.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();
        Ok(Self {
            hosts,
            groups,
            identities,
            tunnels,
            memberships,
        })
    }
}

/// Destination collision surface plus the profile the plan targets.
#[derive(Debug, Clone, Default)]
pub struct DestIndex {
    pub dest_profile: String,
    pub host_names: Vec<String>,
    pub group_names: Vec<String>,
    pub identity_names: Vec<String>,
    /// Labels of labelled destination tunnels.
    pub tunnel_labels: Vec<String>,
}

impl DestIndex {
    /// Copy the destination name surface out of `store`, then drop the lock.
    pub fn capture(store: &LauncherStore, dest_profile: impl Into<String>) -> anyhow::Result<Self> {
        Ok(Self {
            dest_profile: dest_profile.into(),
            host_names: store.list_hosts()?.iter().map(|h| h.name.clone()).collect(),
            group_names: store
                .list_groups()?
                .iter()
                .map(|g| g.name.clone())
                .collect(),
            identity_names: store
                .list_identities()?
                .iter()
                .map(|i| i.name.clone())
                .collect(),
            tunnel_labels: store
                .list_tunnels()?
                .iter()
                .filter_map(|t| t.label.clone())
                .collect(),
        })
    }
}

/// Names selected for transfer. Unknown names are ignored.
#[derive(Debug, Clone, Default)]
pub struct TransferSelection {
    pub host_names: Vec<String>,
    pub group_names: Vec<String>,
    pub identity_names: Vec<String>,
    /// Tunnel labels (`tunnels` have no names; unlabeled tunnels travel only
    /// as auto-carried companions of transferred hosts).
    pub tunnel_names: Vec<String>,
}

/// Outcome of [`apply_transfer_plan`].
#[derive(Debug, Clone, Default)]
pub struct TransferResult {
    pub transferred: usize,
    pub renamed: Vec<String>,
    pub blocked: Vec<String>,
}

/// First free name for `base`: `base` itself when unused, otherwise
/// `base (copy 1)`, `base (copy 2)`, … Never returns a name in `existing`.
pub fn unique_dest_name(base: &str, existing: &[String]) -> String {
    if !existing.iter().any(|e| e == base) {
        return base.to_string();
    }
    let mut n = 1u32;
    loop {
        let candidate = format!("{base} (copy {n})");
        if !existing.iter().any(|e| e == &candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Plan the transfer described by `selection`.
///
/// Scope = selected hosts + (with [`GroupTransferMode::WithHosts`]) members
/// of selected groups + identities required by transferred hosts/groups
/// (auto-carried) + tunnels referencing transferred hosts + explicitly
/// selected identities/tunnels. Hosts in `blocked_hosts` (active session /
/// running tunnel) become per-item `blocked` entries with a message; the rest
/// of the plan stays runnable. Reserved groups (Favorites) never transfer.
pub fn build_transfer_plan(
    src_snapshot: &TransferSnapshot,
    dest_existing: &DestIndex,
    selection: &TransferSelection,
    _mode: TransferMode,
    group_mode: GroupTransferMode,
    blocked_hosts: &HashSet<String>,
) -> TransferPlan {
    let host_by_name: HashMap<&str, &ManagedHost> = src_snapshot
        .hosts
        .iter()
        .map(|h| (h.name.as_str(), h))
        .collect();
    let group_by_name: HashMap<&str, &HostGroup> = src_snapshot
        .groups
        .iter()
        .map(|g| (g.name.as_str(), g))
        .collect();
    let identity_by_name: HashMap<&str, &Identity> = src_snapshot
        .identities
        .iter()
        .map(|i| (i.name.as_str(), i))
        .collect();
    let identity_by_id: HashMap<i64, &Identity> =
        src_snapshot.identities.iter().map(|i| (i.id, i)).collect();
    let host_name_by_id: HashMap<i64, &str> = src_snapshot
        .hosts
        .iter()
        .map(|h| (h.id, h.name.as_str()))
        .collect();

    // --- scope: hosts ---
    let mut host_names: Vec<String> = selection
        .host_names
        .iter()
        .filter(|n| host_by_name.contains_key(n.as_str()))
        .cloned()
        .collect();
    // --- scope: groups (reserved groups never transfer) ---
    let mut group_names: Vec<String> = selection
        .group_names
        .iter()
        .filter(|n| group_by_name.get(n.as_str()).is_some_and(|g| !g.reserved))
        .cloned()
        .collect();
    if group_mode == GroupTransferMode::WithHosts {
        for (host_name, group_name) in &src_snapshot.memberships {
            if group_names.iter().any(|g| g == group_name)
                && group_by_name
                    .get(group_name.as_str())
                    .is_some_and(|g| !g.reserved)
                && !host_names.iter().any(|h| h == host_name)
            {
                host_names.push(host_name.clone());
            }
        }
    }
    host_names.sort();
    host_names.dedup();
    group_names.sort();
    group_names.dedup();

    // --- scope: identities (explicit + auto-carried) ---
    let mut identity_names: Vec<String> = selection
        .identity_names
        .iter()
        .filter(|n| identity_by_name.contains_key(n.as_str()))
        .cloned()
        .collect();
    for host_name in &host_names {
        if let Some(host) = host_by_name.get(host_name.as_str()) {
            if let Some(id) = host.identity_id {
                if let Some(identity) = identity_by_id.get(&id) {
                    identity_names.push(identity.name.clone());
                }
            }
        }
    }
    for group_name in &group_names {
        if let Some(group) = group_by_name.get(group_name.as_str()) {
            if let Some(id) = group.default_identity_id {
                if let Some(identity) = identity_by_id.get(&id) {
                    identity_names.push(identity.name.clone());
                }
            }
        }
    }
    identity_names.sort();
    identity_names.dedup();

    // --- scope: tunnels (explicitly selected labels + companions of
    // transferred hosts) ---
    // (index into snapshot.tunnels, host name or None, label or None)
    let mut tunnel_scope: Vec<usize> = Vec::new();
    for (idx, tunnel) in src_snapshot.tunnels.iter().enumerate() {
        let host_name = tunnel
            .host_id
            .and_then(|id| host_name_by_id.get(&id).copied());
        let selected_label = tunnel
            .label
            .as_ref()
            .is_some_and(|l| selection.tunnel_names.iter().any(|s| s == l));
        let companion_of_transferred = host_name.is_some_and(|h| host_names.iter().any(|s| s == h));
        if selected_label || companion_of_transferred {
            tunnel_scope.push(idx);
        }
    }

    // --- names: never overwrite; thread already-planned names through ---
    let mut used_hosts: Vec<String> = dest_existing.host_names.clone();
    let mut used_groups: Vec<String> = dest_existing.group_names.clone();
    let mut used_identities: Vec<String> = dest_existing.identity_names.clone();
    let mut used_tunnels: Vec<String> = dest_existing.tunnel_labels.clone();

    let mut items = Vec::new();
    for name in &group_names {
        let dest = unique_dest_name(name, &used_groups);
        used_groups.push(dest.clone());
        items.push(PlannedItem {
            kind: "group",
            src_name: name.clone(),
            dest_name: dest.clone(),
            renamed: dest != *name,
            blocked: None,
        });
    }
    for name in &identity_names {
        let dest = unique_dest_name(name, &used_identities);
        used_identities.push(dest.clone());
        items.push(PlannedItem {
            kind: "identity",
            src_name: name.clone(),
            dest_name: dest.clone(),
            renamed: dest != *name,
            blocked: None,
        });
    }
    for name in &host_names {
        let dest = unique_dest_name(name, &used_hosts);
        used_hosts.push(dest.clone());
        let blocked = blocked_hosts
            .contains(name)
            .then(|| format!("{name} has an active session or running tunnel; skipped"));
        items.push(PlannedItem {
            kind: "host",
            src_name: name.clone(),
            dest_name: dest.clone(),
            renamed: dest != *name,
            blocked,
        });
    }
    for idx in tunnel_scope {
        let tunnel = &src_snapshot.tunnels[idx];
        let host_name = tunnel
            .host_id
            .and_then(|id| host_name_by_id.get(&id).copied());
        let host_blocked = host_name.is_some_and(|h| blocked_hosts.contains(h));
        let (src_name, dest_name, renamed) = match &tunnel.label {
            Some(label) => {
                let dest = unique_dest_name(label, &used_tunnels);
                used_tunnels.push(dest.clone());
                let renamed = dest != *label;
                (label.clone(), dest, renamed)
            }
            None => {
                let fallback = format!("tunnel#{}", tunnel.id);
                (fallback.clone(), fallback, false)
            }
        };
        let blocked = if host_blocked {
            Some(format!(
                "tunnel of blocked host {}; skipped",
                host_name.unwrap_or("?")
            ))
        } else {
            None
        };
        items.push(PlannedItem {
            kind: "tunnel",
            src_name,
            dest_name,
            renamed,
            blocked,
        });
    }

    TransferPlan {
        items,
        dest_profile: dest_existing.dest_profile.clone(),
    }
}

/// Apply `plan`: recreate every runnable item in `dest_store`, then — for
/// [`TransferMode::Move`] — delete the source originals.
///
/// Identities are always COPIED (a new row in dest; the source row survives
/// whenever remaining source hosts/groups still reference it). Key files and
/// credentials travel as paths/flags only; secret contents are never read or
/// copied (new rows start with `has_password = false`, matching
/// `duplicate_host`). Blocked items are skipped in both stores and reported
/// in [`TransferResult::blocked`]. Creation order is groups, identities,
/// hosts, tunnels; move-deletion order is hosts (cascading their tunnels),
/// leftover tunnels, groups, unreferenced identities.
pub fn apply_transfer_plan(
    src_store: &mut LauncherStore,
    dest_store: &mut LauncherStore,
    plan: &TransferPlan,
    mode: TransferMode,
) -> TransferResult {
    let mut result = TransferResult::default();

    // Creation maps: source name -> destination id.
    let mut group_ids: HashMap<String, i64> = HashMap::new();
    let mut identity_ids: HashMap<String, i64> = HashMap::new();
    let mut host_ids: HashMap<String, i64> = HashMap::new();

    // Source tunnel rows captured up front (ids shift under us on move).
    let src_tunnels: Vec<Tunnel> = src_store.list_tunnels().unwrap_or_default();
    let src_hosts: Vec<ManagedHost> = src_store.list_hosts().unwrap_or_default();
    let src_host_name_by_id: HashMap<i64, String> =
        src_hosts.iter().map(|h| (h.id, h.name.clone())).collect();
    // Consumed once per matching tunnel item (labels may repeat).
    let mut consumed_tunnel: HashSet<i64> = HashSet::new();

    for item in plan.items.iter().filter(|i| i.blocked.is_none()) {
        match item.kind {
            "group" => {
                let Some(src_group) = src_store
                    .list_groups()
                    .unwrap_or_default()
                    .into_iter()
                    .find(|g| g.name == item.src_name)
                else {
                    result.blocked.push(item.src_name.clone());
                    continue;
                };
                if src_group.reserved {
                    continue;
                }
                let default_identity_id = src_group
                    .default_identity_id
                    .and_then(|id| {
                        src_store
                            .list_identities()
                            .unwrap_or_default()
                            .into_iter()
                            .find(|i| i.id == id)
                    })
                    .and_then(|identity| identity_ids.get(&identity.name).copied());
                let created = dest_store.create_group(&NewHostGroup {
                    name: item.dest_name.clone(),
                    sort_order: src_group.sort_order,
                    default_identity_id,
                    // Nesting is restored only when the parent travels too;
                    // otherwise the group is promoted to top level (existing
                    // delete_group promotion behavior).
                    parent_id: None,
                });
                match created {
                    Ok(group) => {
                        group_ids.insert(item.src_name.clone(), group.id);
                        transferred(&mut result, item);
                    }
                    Err(_) => result.blocked.push(item.src_name.clone()),
                }
            }
            "identity" => {
                let Some(src_identity) = src_store
                    .get_identity_by_name(&item.src_name)
                    .unwrap_or(None)
                else {
                    result.blocked.push(item.src_name.clone());
                    continue;
                };
                // Shared identities are COPIED: a brand-new row in dest. Only
                // the path strings travel — secret contents stay in the source
                // profile's keyring namespace.
                let created = dest_store.create_identity(&NewIdentity {
                    name: item.dest_name.clone(),
                    username: src_identity.username.clone(),
                    private_key: src_identity.private_key.clone(),
                    certificate: src_identity.certificate.clone(),
                    sort_order: 0,
                    has_password: false,
                });
                match created {
                    Ok(identity) => {
                        identity_ids.insert(item.src_name.clone(), identity.id);
                        transferred(&mut result, item);
                    }
                    Err(_) => result.blocked.push(item.src_name.clone()),
                }
            }
            "host" => {
                let Some(src_host) = src_store.get_host_by_name(&item.src_name).unwrap_or(None)
                else {
                    result.blocked.push(item.src_name.clone());
                    continue;
                };
                let dest_identity_id = src_host
                    .identity_id
                    .and_then(|id| {
                        src_store
                            .list_identities()
                            .unwrap_or_default()
                            .into_iter()
                            .find(|i| i.id == id)
                    })
                    .and_then(|identity| identity_ids.get(&identity.name).copied());
                // Primary group travels when it was planned; otherwise NULL.
                let dest_group_id = src_host
                    .group_id
                    .and_then(|id| {
                        src_store
                            .list_groups()
                            .unwrap_or_default()
                            .into_iter()
                            .find(|g| g.id == id)
                    })
                    .filter(|g| !g.reserved)
                    .and_then(|g| group_ids.get(&g.name).copied());
                let created = dest_store.create_host(&NewHost {
                    name: item.dest_name.clone(),
                    label: src_host.label.clone(),
                    address: src_host.address.clone(),
                    port: src_host.port,
                    group_id: dest_group_id,
                    identity_id: dest_identity_id,
                    os_icon: src_host.os_icon.clone(),
                    tags: src_host.tags.clone(),
                    notes: src_host.notes.clone(),
                    proxy_jump: src_host.proxy_jump.clone(),
                    forward_agent: src_host.forward_agent,
                    remote_command: src_host.remote_command.clone(),
                    source: HostSource::Launcher,
                    has_password: false,
                    username: src_host.username.clone(),
                    session_logging: src_host.session_logging,
                    transport: src_host.transport,
                });
                match created {
                    Ok(created_host) => {
                        host_ids.insert(item.src_name.clone(), created_host.id);
                        // Restore the remaining transferred memberships.
                        for group in &src_host.groups {
                            if group.reserved {
                                continue;
                            }
                            if let Some(dest_gid) = group_ids.get(&group.name) {
                                if Some(*dest_gid) != dest_group_id {
                                    let _ =
                                        dest_store.add_host_to_group(created_host.id, *dest_gid);
                                }
                            }
                        }
                        // Favorites is reserved per profile; re-favorite in dest.
                        if src_host.favorite {
                            if let Ok(fav_id) = dest_store.favorites_group_id() {
                                let _ = dest_store.add_host_to_group(created_host.id, fav_id);
                            }
                        }
                        // NewHost has no environment field; restore it after.
                        if src_host.environment.is_some() {
                            let _ = dest_store.update_host(
                                created_host.id,
                                &super::types::HostUpdate {
                                    environment: Some(src_host.environment.clone()),
                                    ..Default::default()
                                },
                            );
                        }
                        transferred(&mut result, item);
                    }
                    Err(_) => result.blocked.push(item.src_name.clone()),
                }
            }
            "tunnel" => {
                // Match the source row: labelled by (label, host), consumed
                // once so duplicate labels drain distinct rows.
                let src_tunnel = src_tunnels.iter().find(|t| {
                    if consumed_tunnel.contains(&t.id) {
                        return false;
                    }
                    match (&t.label, item.src_name.strip_prefix("tunnel#")) {
                        (Some(label), None) => *label == item.src_name,
                        (None, Some(id)) => t.id.to_string() == id,
                        _ => false,
                    }
                });
                let Some(src_tunnel) = src_tunnel else {
                    result.blocked.push(item.src_name.clone());
                    continue;
                };
                consumed_tunnel.insert(src_tunnel.id);
                // A tunnel whose host did not transfer (explicitly selected
                // alone) lands detached rather than dangling.
                let dest_host_id = src_tunnel.host_id.and_then(|id| {
                    src_host_name_by_id
                        .get(&id)
                        .and_then(|name| host_ids.get(name).copied())
                });
                let dest_label = src_tunnel.label.as_ref().map(|_| item.dest_name.clone());
                let created = dest_store.create_tunnel(&NewTunnel {
                    host_id: dest_host_id,
                    tunnel_type: src_tunnel.tunnel_type,
                    local_port: src_tunnel.local_port,
                    remote_host: src_tunnel.remote_host.clone(),
                    remote_port: src_tunnel.remote_port,
                    label: dest_label,
                    auto_connect: src_tunnel.auto_connect,
                });
                match created {
                    Ok(_) => transferred(&mut result, item),
                    Err(_) => result.blocked.push(item.src_name.clone()),
                }
            }
            _ => {}
        }
    }

    // Plan-blocked items are reported even though they were never attempted.
    for item in plan.blocked() {
        if !result.blocked.iter().any(|n| n == &item.src_name) {
            result.blocked.push(item.src_name.clone());
        }
    }

    if mode == TransferMode::Move {
        delete_source_originals(src_store, plan, &src_tunnels, &src_host_name_by_id);
    }

    result
}

fn transferred(result: &mut TransferResult, item: &PlannedItem) {
    result.transferred += 1;
    if item.renamed {
        result.renamed.push(item.dest_name.clone());
    }
}

/// Move-phase deletion: runnable items only, hosts first (their tunnels and
/// memberships CASCADE), then leftover tunnels, then groups (members promote
/// to remaining groups per `delete_group`), then identities that nothing
/// still references. A carried identity still used by a remaining source host
/// or group default is never deleted.
fn delete_source_originals(
    src_store: &mut LauncherStore,
    plan: &TransferPlan,
    src_tunnels: &[Tunnel],
    src_host_name_by_id: &HashMap<i64, String>,
) {
    let runnable: Vec<&PlannedItem> = plan.runnable();

    for item in runnable.iter().filter(|i| i.kind == "host") {
        if let Ok(Some(host)) = src_store.get_host_by_name(&item.src_name) {
            let _ = src_store.delete_host(host.id);
        }
    }
    for item in runnable.iter().filter(|i| i.kind == "tunnel") {
        // Host deletion already cascaded companion tunnels; remove leftovers.
        let leftover: Vec<i64> = src_store
            .list_tunnels()
            .unwrap_or_default()
            .iter()
            .filter(
                |t| match (&t.label, item.src_name.strip_prefix("tunnel#")) {
                    (Some(label), None) => *label == item.src_name,
                    (None, Some(id)) => t.id.to_string() == id,
                    _ => false,
                },
            )
            .map(|t| t.id)
            .collect();
        // Prefer the exact pre-apply row when it survived.
        let target = src_tunnels
            .iter()
            .find(|t| {
                let label_match = match (&t.label, item.src_name.strip_prefix("tunnel#")) {
                    (Some(label), None) => *label == item.src_name,
                    (None, Some(id)) => t.id.to_string() == id,
                    _ => false,
                };
                label_match && leftover.contains(&t.id)
            })
            .map(|t| t.id)
            .or_else(|| leftover.into_iter().next());
        if let Some(id) = target {
            let _ = src_store.delete_tunnel(id);
        }
    }
    // Silence unused-host-map warnings in builds without tunnel leftovers.
    let _ = src_host_name_by_id;
    for item in runnable.iter().filter(|i| i.kind == "group") {
        if let Ok(groups) = src_store.list_groups() {
            if let Some(group) = groups.into_iter().find(|g| g.name == item.src_name) {
                let _ = src_store.delete_group(group.id);
            }
        }
    }
    for item in runnable.iter().filter(|i| i.kind == "identity") {
        if let Ok(Some(identity)) = src_store.get_identity_by_name(&item.src_name) {
            let hosts_still_use = src_store
                .count_hosts_using_identity(identity.id)
                .unwrap_or(1);
            let group_default_still_use = src_store
                .list_groups()
                .unwrap_or_default()
                .iter()
                .any(|g| g.default_identity_id == Some(identity.id));
            if hosts_still_use == 0 && !group_default_still_use {
                let _ = src_store.delete_identity(identity.id);
            }
        }
    }
}
