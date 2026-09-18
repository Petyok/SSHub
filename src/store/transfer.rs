//! Cross-profile transfers: plan a scoped copy/move from one profile's store
//! into another, then apply it.
//!
//! Snapshot-first design: [`TransferSnapshot::capture`] and
//! [`DestIndex::capture`] copy plain DTOs out of each store (one short lock
//! per `list_*` call), so planning never holds a store lock and applying
//! never holds two locks at once — the `LauncherStore` mutex is
//! non-reentrant and `with_conn` uses `unchecked_transaction`.
//!
//! Tunnel selection speaks one canonical grammar ([`tunnel_token_matches`]):
//! a label, a bare id, `tunnel#<id>`, or `:<port>`. The TUI scope picker and
//! the CLI both feed this grammar, so an unlabeled tunnel selected either way
//! resolves to the same row.

use std::collections::{HashMap, HashSet};

use anyhow::Context;

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
    /// Tunnel tokens in the canonical [`tunnel_token_matches`] grammar
    /// (label, bare id, `tunnel#<id>`, or `:<port>`).
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
/// Canonical tunnel token grammar, shared by the engine, the TUI scope picker
/// (which yields `:<port>` for unlabeled tunnels), and the CLI (which passes
/// the resolved tunnel's label or `tunnel#<id>`).
///
/// A token selects a tunnel when it equals the label, the bare id (`42`), the
/// stored fallback name (`tunnel#42`), or the port token (`:8080`). A bare
/// port never matches: a bare number is always an id, mirroring the other
/// `tunnel` subcommands' resolver.
pub fn tunnel_token_matches(tunnel: &Tunnel, token: &str) -> bool {
    if tunnel.label.as_ref().is_some_and(|label| label == token) {
        return true;
    }
    let id = tunnel.id.to_string();
    if token == id {
        return true;
    }
    if token
        .strip_prefix("tunnel#")
        .is_some_and(|rest| rest == id.as_str())
    {
        return true;
    }
    token
        .strip_prefix(':')
        .is_some_and(|port| port == tunnel.local_port.to_string())
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
        // Only runnable hosts carry their identity: a blocked host stays home,
        // so its identity must not ride along without it.
        if blocked_hosts.contains(host_name) {
            continue;
        }
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

    // --- scope: tunnels (explicitly selected tokens + companions of
    // transferred hosts) ---
    // (index into snapshot.tunnels, host name or None, label or None)
    let mut tunnel_scope: Vec<usize> = Vec::new();
    for (idx, tunnel) in src_snapshot.tunnels.iter().enumerate() {
        let host_name = tunnel
            .host_id
            .and_then(|id| host_name_by_id.get(&id).copied());
        let explicitly_selected = selection
            .tunnel_names
            .iter()
            .any(|token| tunnel_token_matches(tunnel, token));
        let companion_of_transferred = host_name.is_some_and(|h| host_names.iter().any(|s| s == h));
        if explicitly_selected || companion_of_transferred {
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
/// All-or-nothing per store: destination rows are created inside a single
/// SQLite transaction (one `unchecked_transaction` on the dest connection; a
/// separate one guards the source move-deletion, since the two profiles are
/// different database files and cannot share a transaction). The first
/// creation failure rolls the destination back, skips the move-deletion, and
/// is returned as `Err` — partial rows never survive. Items the plan already
/// marks `blocked` (active session / running tunnel) are expected skips, not
/// errors: they are reported in [`TransferResult::blocked`] and apply still
/// returns `Ok`.
///
/// Dependencies are hard: an identity or group whose creation failed BLOCKS
/// every host (or group default) that references it instead of silently
/// wiring `None`. Creation order is identities, groups, hosts, tunnels, so a
/// failure is always observed before its dependents are attempted.
///
/// Identities are always COPIED (a new row in dest; the source row survives
/// whenever remaining source hosts/groups still reference it). Key files and
/// credentials travel as paths/flags only; secret contents are never read or
/// copied (new rows start with `has_password = false`, matching
/// `duplicate_host`). Group nesting is flattened: transferred groups land
/// with `parent_id = None` (existing delete-group promotion behavior), and
/// transferred hosts land with [`HostSource::Launcher`]. Move-deletion order
/// is hosts (cascading their tunnels), leftover tunnels, groups,
/// unreferenced identities; deletion errors roll the source back and are
/// returned as `Err`, leaving the source untouched.
pub fn apply_transfer_plan(
    src_store: &mut LauncherStore,
    dest_store: &mut LauncherStore,
    plan: &TransferPlan,
    mode: TransferMode,
) -> anyhow::Result<TransferResult> {
    // Source rows captured up front (ids shift under us on move).
    let src_hosts: Vec<ManagedHost> = src_store.list_hosts().unwrap_or_default();
    let src_groups: Vec<HostGroup> = src_store.list_groups().unwrap_or_default();
    let src_identities: Vec<Identity> = src_store.list_identities().unwrap_or_default();
    let src_tunnels: Vec<Tunnel> = src_store.list_tunnels().unwrap_or_default();
    let src_host_by_name: HashMap<&str, &ManagedHost> =
        src_hosts.iter().map(|h| (h.name.as_str(), h)).collect();
    let src_group_by_name: HashMap<&str, &HostGroup> =
        src_groups.iter().map(|g| (g.name.as_str(), g)).collect();
    let src_group_by_id: HashMap<i64, &HostGroup> = src_groups.iter().map(|g| (g.id, g)).collect();
    let src_identity_by_name: HashMap<&str, &Identity> = src_identities
        .iter()
        .map(|i| (i.name.as_str(), i))
        .collect();
    let identity_name_by_id: HashMap<i64, &str> = src_identities
        .iter()
        .map(|i| (i.id, i.name.as_str()))
        .collect();
    let src_host_name_by_id: HashMap<i64, String> =
        src_hosts.iter().map(|h| (h.id, h.name.clone())).collect();

    let mut result = TransferResult::default();
    // Creation maps: source name -> destination id.
    let mut group_ids: HashMap<String, i64> = HashMap::new();
    let mut identity_ids: HashMap<String, i64> = HashMap::new();
    let mut host_ids: HashMap<String, i64> = HashMap::new();
    // Source names whose creation failed: dependents block on these instead
    // of wiring `None`.
    let mut failed_groups: HashSet<String> = HashSet::new();
    let mut failed_identities: HashSet<String> = HashSet::new();
    let mut failed_names: Vec<String> = Vec::new();
    let mut first_error: Option<anyhow::Error> = None;
    // Consumed once per matching tunnel item (labels may repeat).
    let mut consumed_tunnel: HashSet<i64> = HashSet::new();

    // Record one creation failure: the item is reported blocked, its name is
    // remembered for the error below, and dependents block on it. The open
    // transaction rolls back when the loop below sees `first_error`.
    macro_rules! record_failure {
        ($item:expr, $failed:expr, $error:expr) => {{
            result.blocked.push($item.src_name.clone());
            $failed.insert($item.src_name.clone());
            failed_names.push($item.src_name.clone());
            if first_error.is_none() {
                first_error = Some($error);
            }
        }};
    }

    dest_store
        .with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            // Dependency order, not plan order: a failure must be observed
            // before its dependents are attempted.
            for kind in ["identity", "group", "host", "tunnel"] {
                for item in plan
                    .items
                    .iter()
                    .filter(|i| i.blocked.is_none() && i.kind == kind)
                {
                    match item.kind {
                        "group" => {
                            let Some(src_group) = src_group_by_name.get(item.src_name.as_str())
                            else {
                                result.blocked.push(item.src_name.clone());
                                continue;
                            };
                            if src_group.reserved {
                                continue;
                            }
                            let default_identity_id = match src_group
                                .default_identity_id
                                .and_then(|id| identity_name_by_id.get(&id).copied())
                            {
                                None => None,
                                Some(name) => {
                                    if failed_identities.contains(name) {
                                        record_failure!(
                                            item,
                                            failed_groups,
                                            anyhow::anyhow!("identity '{name}' failed to transfer")
                                        );
                                        continue;
                                    }
                                    identity_ids.get(name).copied()
                                }
                            };
                            match super::hosts::create_group_on(
                                conn,
                                &NewHostGroup {
                                    name: item.dest_name.clone(),
                                    sort_order: src_group.sort_order,
                                    default_identity_id,
                                    // Group-level connection defaults travel
                                    // with the group: they are plain values,
                                    // unlike the identity, which is remapped.
                                    default_username: src_group.default_username.clone(),
                                    default_port: src_group.default_port,
                                    default_proxy_jump: src_group.default_proxy_jump.clone(),
                                    default_transport: src_group.default_transport,
                                    default_forward_agent: src_group.default_forward_agent,
                                    // Nesting is restored only when the parent
                                    // travels too; otherwise the group is
                                    // promoted to top level (existing
                                    // delete_group promotion behavior).
                                    parent_id: None,
                                },
                            ) {
                                Ok(group) => {
                                    group_ids.insert(item.src_name.clone(), group.id);
                                    transferred(&mut result, item);
                                }
                                Err(e) => record_failure!(
                                    item,
                                    failed_groups,
                                    e.context(format!("create group '{}'", item.src_name))
                                ),
                            }
                        }
                        "identity" => {
                            let Some(src_identity) =
                                src_identity_by_name.get(item.src_name.as_str())
                            else {
                                result.blocked.push(item.src_name.clone());
                                continue;
                            };
                            // Shared identities are COPIED: a brand-new row in
                            // dest. Only the path strings travel — secret
                            // contents stay in the source profile's keyring
                            // namespace.
                            match super::identities::create_identity_on(
                                conn,
                                &NewIdentity {
                                    name: item.dest_name.clone(),
                                    username: src_identity.username.clone(),
                                    private_key: src_identity.private_key.clone(),
                                    certificate: src_identity.certificate.clone(),
                                    sort_order: 0,
                                    has_password: false,
                                },
                            ) {
                                Ok(identity) => {
                                    identity_ids.insert(item.src_name.clone(), identity.id);
                                    transferred(&mut result, item);
                                }
                                Err(e) => record_failure!(
                                    item,
                                    failed_identities,
                                    e.context(format!("create identity '{}'", item.src_name))
                                ),
                            }
                        }
                        "host" => {
                            let Some(src_host) = src_host_by_name.get(item.src_name.as_str())
                            else {
                                result.blocked.push(item.src_name.clone());
                                continue;
                            };
                            let dest_identity_id = match src_host
                                .identity_id
                                .and_then(|id| identity_name_by_id.get(&id).copied())
                            {
                                None => None,
                                Some(name) => {
                                    if failed_identities.contains(name) {
                                        record_failure!(
                                            item,
                                            failed_groups,
                                            anyhow::anyhow!("identity '{name}' failed to transfer")
                                        );
                                        continue;
                                    }
                                    identity_ids.get(name).copied()
                                }
                            };
                            // Primary group travels when it was planned;
                            // otherwise NULL.
                            let dest_group_id = match src_host
                                .group_id
                                .and_then(|id| src_group_by_id.get(&id).copied())
                            {
                                None => None,
                                Some(group) if group.reserved => None,
                                Some(group) => {
                                    if failed_groups.contains(group.name.as_str()) {
                                        record_failure!(
                                            item,
                                            failed_groups,
                                            anyhow::anyhow!(
                                                "group '{}' failed to transfer",
                                                group.name
                                            )
                                        );
                                        continue;
                                    }
                                    group_ids.get(group.name.as_str()).copied()
                                }
                            };
                            let created = super::hosts::create_host_on(
                                conn,
                                &NewHost {
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
                                },
                            );
                            let created_host = match created {
                                Ok(host) => host,
                                Err(e) => {
                                    record_failure!(
                                        item,
                                        failed_groups,
                                        e.context(format!("create host '{}'", item.src_name))
                                    );
                                    continue;
                                }
                            };
                            host_ids.insert(item.src_name.clone(), created_host.id);
                            // Restore the remaining transferred memberships.
                            let mut restore_error: Option<anyhow::Error> = None;
                            for group in &src_host.groups {
                                if group.reserved {
                                    continue;
                                }
                                if let Some(dest_gid) = group_ids.get(&group.name) {
                                    if Some(*dest_gid) != dest_group_id {
                                        if let Err(e) = super::hosts::add_host_to_group_on(
                                            conn,
                                            created_host.id,
                                            *dest_gid,
                                        ) {
                                            restore_error = Some(e.context(format!(
                                                "restore membership of host '{}' in group '{}'",
                                                item.src_name, group.name
                                            )));
                                            break;
                                        }
                                    }
                                }
                            }
                            // Favorites is reserved per profile; re-favorite in dest.
                            if restore_error.is_none() && src_host.favorite {
                                match super::hosts::favorites_group_id_on(conn) {
                                    Ok(fav_id) => {
                                        if let Err(e) = super::hosts::add_host_to_group_on(
                                            conn,
                                            created_host.id,
                                            fav_id,
                                        ) {
                                            restore_error = Some(e.context(format!(
                                                "re-favorite host '{}'",
                                                item.src_name
                                            )));
                                        }
                                    }
                                    Err(e) => {
                                        restore_error = Some(e.context(format!(
                                            "resolve Favorites group for host '{}'",
                                            item.src_name
                                        )));
                                    }
                                }
                            }
                            // NewHost has no environment field; restore it after.
                            if restore_error.is_none() && src_host.environment.is_some() {
                                if let Err(e) = super::hosts::set_host_environment_on(
                                    conn,
                                    created_host.id,
                                    &src_host.environment,
                                ) {
                                    restore_error = Some(e.context(format!(
                                        "restore environment of host '{}'",
                                        item.src_name
                                    )));
                                }
                            }
                            if let Some(e) = restore_error {
                                record_failure!(item, failed_groups, e);
                                continue;
                            }
                            transferred(&mut result, item);
                        }
                        "tunnel" => {
                            // Match the source row with the canonical grammar:
                            // labelled by label, unlabeled by bare id,
                            // `tunnel#<id>`, or `:<port>`. Consumed once so
                            // duplicate labels drain distinct rows.
                            let src_tunnel = src_tunnels.iter().find(|t| {
                                if consumed_tunnel.contains(&t.id) {
                                    return false;
                                }
                                tunnel_token_matches(t, &item.src_name)
                            });
                            let Some(src_tunnel) = src_tunnel else {
                                result.blocked.push(item.src_name.clone());
                                continue;
                            };
                            consumed_tunnel.insert(src_tunnel.id);
                            // A tunnel whose host did not transfer (explicitly
                            // selected alone) lands detached rather than
                            // dangling.
                            let dest_host_id = src_tunnel.host_id.and_then(|id| {
                                src_host_name_by_id
                                    .get(&id)
                                    .and_then(|name| host_ids.get(name).copied())
                            });
                            let dest_label =
                                src_tunnel.label.as_ref().map(|_| item.dest_name.clone());
                            match super::tunnels::create_tunnel_on(
                                conn,
                                &NewTunnel {
                                    host_id: dest_host_id,
                                    tunnel_type: src_tunnel.tunnel_type,
                                    local_port: src_tunnel.local_port,
                                    remote_host: src_tunnel.remote_host.clone(),
                                    remote_port: src_tunnel.remote_port,
                                    label: dest_label,
                                    auto_connect: src_tunnel.auto_connect,
                                },
                            ) {
                                Ok(_) => transferred(&mut result, item),
                                Err(e) => record_failure!(
                                    item,
                                    failed_groups,
                                    e.context(format!("create tunnel '{}'", item.src_name))
                                ),
                            }
                        }
                        _ => {}
                    }
                }
            }

            // Plan-blocked items are reported even though they were never
            // attempted.
            for item in plan.blocked() {
                if !result.blocked.iter().any(|n| n == &item.src_name) {
                    result.blocked.push(item.src_name.clone());
                }
            }

            // Commit only a clean build. Any recorded failure drops the
            // transaction uncommitted here, so no partial rows survive.
            if first_error.is_none() {
                tx.commit()?;
            }
            Ok(())
        })
        .context("apply transfer to destination")?;

    if let Some(error) = first_error {
        anyhow::bail!(
            "transfer to '{}' failed and was rolled back (failed: {}): {error:#}",
            plan.dest_profile,
            failed_names.join(", ")
        );
    }

    if mode == TransferMode::Move {
        let owned_hosts: HashMap<String, ManagedHost> =
            src_hosts.into_iter().map(|h| (h.name.clone(), h)).collect();
        let owned_groups: HashMap<String, HostGroup> = src_groups
            .into_iter()
            .map(|g| (g.name.clone(), g))
            .collect();
        let owned_identities: HashMap<String, Identity> = src_identities
            .into_iter()
            .map(|i| (i.name.clone(), i))
            .collect();
        delete_source_originals(
            src_store,
            plan,
            &owned_hosts,
            &owned_groups,
            &owned_identities,
        )?;
    }

    Ok(result)
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
///
/// Runs inside a single source transaction and only ever runs after a clean
/// destination build, so a move either fully lands or leaves the source
/// untouched. Deletion errors roll back and propagate as `Err`.
fn delete_source_originals(
    src_store: &mut LauncherStore,
    plan: &TransferPlan,
    src_hosts: &HashMap<String, ManagedHost>,
    src_groups: &HashMap<String, HostGroup>,
    src_identities: &HashMap<String, Identity>,
) -> anyhow::Result<()> {
    let runnable: Vec<&PlannedItem> = plan.runnable();
    src_store
        .with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            for item in runnable.iter().filter(|i| i.kind == "host") {
                let Some(host) = src_hosts.get(item.src_name.as_str()) else {
                    continue;
                };
                if host.source != HostSource::Launcher {
                    continue;
                }
                super::hosts::delete_host_on(conn, host.id)?;
            }
            // Host deletion cascaded companion tunnels; remove leftovers by
            // re-reading inside the transaction, matched with the same
            // canonical grammar as planning.
            let remaining = super::tunnels::list_tunnels_on(conn)?;
            let mut consumed: HashSet<i64> = HashSet::new();
            for item in runnable.iter().filter(|i| i.kind == "tunnel") {
                let target = remaining
                    .iter()
                    .filter(|t| !consumed.contains(&t.id))
                    .find(|t| tunnel_token_matches(t, &item.src_name))
                    .map(|t| t.id);
                if let Some(id) = target {
                    consumed.insert(id);
                    super::tunnels::delete_tunnel_on(conn, id)?;
                }
            }
            for item in runnable.iter().filter(|i| i.kind == "group") {
                let Some(group) = src_groups.get(item.src_name.as_str()) else {
                    continue;
                };
                if group.reserved {
                    continue;
                }
                super::hosts::delete_group_on(conn, group.id)?;
            }
            for item in runnable.iter().filter(|i| i.kind == "identity") {
                let Some(identity) = src_identities.get(item.src_name.as_str()) else {
                    continue;
                };
                let hosts_still_use =
                    super::identities::count_hosts_using_identity_on(conn, identity.id)?;
                let group_default_still_use =
                    super::hosts::group_default_uses_identity_on(conn, identity.id)?;
                if hosts_still_use == 0 && !group_default_still_use {
                    super::identities::delete_identity_on(conn, identity.id)?;
                }
            }
            tx.commit()?;
            Ok(())
        })
        .context("delete moved source originals")
}
