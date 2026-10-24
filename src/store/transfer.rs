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
    AuthEvent, HostGroup, HostSource, HostUpdate, Identity, IdentityUpdate, ManagedHost, NewHost,
    NewHostGroup, NewIdentity, NewTunnel, Tunnel,
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
    /// `"host"`, `"group"`, `"identity"`, `"tunnel"` or `"audit"` (the audit
    /// events of host `src_name`, landing under the host's `dest_name`).
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
    /// A move keeps a planned group in the source while a host that is not
    /// being moved still belongs to it (set by [`TransferSelection::no_auto_carry`]:
    /// the guided migration moves chosen hosts, not whole groups). Without it
    /// a moved group is always deleted and its remaining members promoted.
    pub leave_shared_groups: bool,
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
    /// Carry only what is listed: no identities pulled in by transferred
    /// hosts/groups, no tunnels of transferred hosts. The guided migration
    /// lists every dependent explicitly, so an unchecked one stays behind.
    pub no_auto_carry: bool,
    /// Hosts whose audit events travel with them. A name that is not a
    /// transferred host is ignored.
    pub audit_hosts: Vec<String>,
}

/// Outcome of [`apply_transfer_plan`].
#[derive(Debug, Clone, Default)]
pub struct TransferResult {
    pub transferred: usize,
    pub renamed: Vec<String>,
    pub blocked: Vec<String>,
    /// Source host ids deleted by a move; their `host:{id}` secrets are
    /// orphaned until [`TransferResult::purge_source_secrets`] runs.
    pub deleted_host_ids: Vec<i64>,
    /// Source identity ids deleted by a move (`identity:{id}` secrets).
    pub deleted_identity_ids: Vec<i64>,
    /// Audit events written to the destination.
    pub audit_events: usize,
    /// Every host created in the destination, with the ids that tie its
    /// secret to the source's (see [`SecretStores`]).
    pub created_hosts: Vec<CreatedRow>,
    /// Every identity created in the destination.
    pub created_identities: Vec<CreatedRow>,
    /// Every group created in the destination.
    pub created_groups: Vec<CreatedRow>,
    /// One message per secret a move could not carry (never a secret value).
    pub carry_failures: Vec<String>,
    /// Names of the hosts and identities a move left in the SOURCE because
    /// their secret did not carry: still there, still holding their password.
    /// A host's destination copy is removed again, so repeating the move does
    /// not duplicate it; an identity's destination copy stays (moved hosts
    /// point at it).
    pub kept_in_source: Vec<String>,
}

/// A host or identity created in the destination.
#[derive(Debug, Clone)]
pub struct CreatedRow {
    /// Source name, for messages.
    pub name: String,
    /// Name of the created destination row.
    pub dest_name: String,
    /// Hosts: id of the source identity the host points at.
    pub identity_src_id: Option<i64>,
    pub src_id: i64,
    pub dest_id: i64,
    /// The source row flags a saved secret (`has_password`).
    pub had_secret: bool,
}

/// The credential stores of a move's two profiles. With these, a move carries
/// every saved secret across before it deletes anything from the source.
pub struct SecretStores<'a> {
    pub src: &'a dyn crate::credentials::PasswordStore,
    pub dest: &'a dyn crate::credentials::PasswordStore,
}

impl TransferResult {
    /// Delete the saved secrets of the source hosts/identities a move removed
    /// from `src_secrets` (the SOURCE profile's credential store). Returns one
    /// message per key that could not be deleted; callers MUST surface them.
    /// Copy results carry no deleted ids, so this is a no-op there.
    pub fn purge_source_secrets(
        &self,
        src_secrets: &dyn crate::credentials::PasswordStore,
    ) -> Vec<String> {
        let keys = self
            .deleted_host_ids
            .iter()
            .map(|id| crate::credentials::host_key(*id))
            .chain(
                self.deleted_identity_ids
                    .iter()
                    .map(|id| crate::credentials::identity_key(*id)),
            );
        keys.filter_map(|key| {
            src_secrets
                .delete(&key)
                .err()
                .map(|e| format!("{key}: {e:#}"))
        })
        .collect()
    }
}

/// What carrying the secrets of a move left undone.
struct CarryOutcome {
    failed_hosts: HashSet<i64>,
    failed_identities: HashSet<i64>,
    messages: Vec<String>,
    kept: Vec<String>,
}

/// Copy the saved secret of every created host and identity from the source
/// store into the destination one (under the new id) and flag the new row
/// `has_password`. Rows whose source row has no saved secret are skipped. A
/// row whose secret is flagged but cannot be read, written or flagged is
/// reported in `failed_*`: the caller keeps its source original.
fn carry_secrets(
    result: &TransferResult,
    stores: &SecretStores<'_>,
    dest_store: &LauncherStore,
) -> CarryOutcome {
    use crate::credentials::{host_key, identity_key};
    let mut out = CarryOutcome {
        failed_hosts: HashSet::new(),
        failed_identities: HashSet::new(),
        messages: Vec::new(),
        kept: Vec::new(),
    };
    for row in &result.created_hosts {
        let carried = carry_one(
            stores,
            row,
            &host_key(row.src_id),
            &host_key(row.dest_id),
            || {
                dest_store
                    .update_host(
                        row.dest_id,
                        &HostUpdate {
                            has_password: Some(true),
                            ..Default::default()
                        },
                    )
                    .map(|_| ())
            },
        );
        if let Err(message) = carried {
            out.failed_hosts.insert(row.src_id);
            out.kept.push(row.name.clone());
            out.messages.push(format!("host '{}': {message}", row.name));
        }
    }
    for row in &result.created_identities {
        let carried = carry_one(
            stores,
            row,
            &identity_key(row.src_id),
            &identity_key(row.dest_id),
            || {
                dest_store
                    .update_identity(
                        row.dest_id,
                        &IdentityUpdate {
                            has_password: Some(true),
                            ..Default::default()
                        },
                    )
                    .map(|_| ())
            },
        );
        if let Err(message) = carried {
            out.failed_identities.insert(row.src_id);
            out.kept.push(row.name.clone());
            out.messages
                .push(format!("identity '{}': {message}", row.name));
        }
    }
    // A host whose identity stays behind must stay too: moved, it would point
    // at a destination identity that has lost its passphrase.
    for row in &result.created_hosts {
        let Some(identity) = row.identity_src_id else {
            continue;
        };
        if out.failed_identities.contains(&identity) && out.failed_hosts.insert(row.src_id) {
            out.kept.push(row.name.clone());
            out.messages.push(format!(
                "host '{}': its identity could not be carried",
                row.name
            ));
        }
    }
    out
}

/// Carry one secret `from` -> `to`. A half-done carry (secret written, row
/// flag not) is rolled back so the destination never keeps a secret nothing
/// points at.
fn carry_one(
    stores: &SecretStores<'_>,
    row: &CreatedRow,
    from: &str,
    to: &str,
    flag_row: impl FnOnce() -> anyhow::Result<()>,
) -> Result<(), String> {
    // The row flag is only a hint: a password saved through "remember" lives
    // in the store with the flag unset, and presence decides everywhere else
    // (`resolve_pending_secret`). Skipping unflagged rows would purge it.
    let secret = match stores.src.get(from) {
        Ok(Some(secret)) => secret,
        Ok(None) if !row.had_secret => return Ok(()),
        // Flagged but absent: a different store than the one that holds it
        // (keyring vs credentials.json), not "nothing to carry".
        Ok(None) => {
            return Err(
                "saved secret not found in the source credential store (re-enter the password or clear it on the row, then move again)"
                    .into(),
            )
        }
        Err(e) => return Err(format!("cannot read the saved secret: {e:#}")),
    };
    stores
        .dest
        .set(to, &secret)
        .map_err(|e| format!("cannot write the secret to the destination: {e:#}"))?;
    if let Err(e) = flag_row() {
        let _ = stores.dest.delete(to);
        return Err(format!("destination row not updated: {e:#}"));
    }
    Ok(())
}

/// Warning for the secrets a move could not carry: one headline naming at
/// most five of the rows left behind, then at most five failure lines. The
/// named rows were NOT moved (the source originals stay and their destination
/// copies were removed), so the move can be repeated once the cause is fixed.
pub fn secret_carry_warning(failures: &[String], kept: &[String]) -> String {
    const SHOWN: usize = 5;
    let mut names = kept
        .iter()
        .take(SHOWN)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if kept.len() > SHOWN {
        names.push_str(&format!(" and {} more", kept.len() - SHOWN));
    }
    let mut out = format!(
        "{} problem(s) carrying secrets; these stay in the source profile: {names}",
        failures.len()
    );
    for failure in failures.iter().take(SHOWN) {
        out.push_str(&format!("\n  - {failure}"));
    }
    if failures.len() > SHOWN {
        out.push_str(&format!("\n  ... and {} more", failures.len() - SHOWN));
    }
    out
}

/// One-line warning for the failures from [`TransferResult::purge_source_secrets`].
pub fn secret_purge_warning(failures: &[String]) -> String {
    format!(
        "{} moved secret(s) could not be removed from the source keyring: {}",
        failures.len(),
        failures.join("; ")
    )
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
    for host_name in host_names.iter().filter(|_| !selection.no_auto_carry) {
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
    for group_name in group_names.iter().filter(|_| !selection.no_auto_carry) {
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
        let companion_of_transferred = !selection.no_auto_carry
            && host_name.is_some_and(|h| host_names.iter().any(|s| s == h));
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
                // The label is the plan's handle on the row only while it picks
                // exactly this tunnel; a shared label (or one that reads as
                // another tunnel's id or port) would apply to, and a move
                // would delete, the wrong rows.
                let unique = src_snapshot
                    .tunnels
                    .iter()
                    .filter(|t| tunnel_token_matches(t, label))
                    .count()
                    == 1;
                let handle = if unique {
                    label.clone()
                } else {
                    format!("tunnel#{}", tunnel.id)
                };
                (handle, dest, renamed)
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
    // Audit events ride with their host: runnable only when the host is.
    let audit_items: Vec<PlannedItem> = items
        .iter()
        .filter(|item| item.kind == "host")
        .filter(|item| selection.audit_hosts.contains(&item.src_name))
        .map(|host| PlannedItem {
            kind: "audit",
            src_name: host.src_name.clone(),
            dest_name: host.dest_name.clone(),
            renamed: false,
            blocked: host.blocked.clone(),
        })
        .collect();
    items.extend(audit_items);

    TransferPlan {
        items,
        dest_profile: dest_existing.dest_profile.clone(),
        leave_shared_groups: selection.no_auto_carry,
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
/// credentials travel as paths/flags only; this entry point never reads or
/// copies secret contents (new rows start with `has_password = false`,
/// matching `duplicate_host`); a move that should keep them goes through
/// [`apply_transfer_plan_with_secrets`]. Group nesting is flattened: transferred groups land
/// with `parent_id = None` (existing delete-group promotion behavior), and
/// transferred hosts land with [`HostSource::Launcher`]. Move-deletion order
/// is hosts (their tunnels are detached first, then the plan's tunnels are
/// deleted by handle), leftover tunnels, groups,
/// unreferenced identities; deletion errors roll the source back and are
/// returned as `Err`, leaving the source untouched.
pub fn apply_transfer_plan(
    src_store: &mut LauncherStore,
    dest_store: &mut LauncherStore,
    plan: &TransferPlan,
    mode: TransferMode,
) -> anyhow::Result<TransferResult> {
    apply_transfer_plan_with_secrets(src_store, dest_store, plan, mode, None)
}

/// [`apply_transfer_plan`] plus the credential stores of both profiles. For a
/// move, `secrets` makes it lossless: between the destination commit and the
/// source deletion every saved secret is carried across, and a host or
/// identity whose secret fails to carry is NOT deleted from the source (see
/// [`TransferResult::kept_in_source`] and [`TransferResult::carry_failures`]).
/// Without `secrets` nothing is carried, so the caller must not purge the
/// source credential store afterwards. A copy never carries secrets.
pub fn apply_transfer_plan_with_secrets(
    src_store: &mut LauncherStore,
    dest_store: &mut LauncherStore,
    plan: &TransferPlan,
    mode: TransferMode,
    secrets: Option<SecretStores<'_>>,
) -> anyhow::Result<TransferResult> {
    // Source rows captured up front (ids shift under us on move).
    let src_hosts: Vec<ManagedHost> = src_store.list_hosts()?;
    let src_groups: Vec<HostGroup> = src_store.list_groups()?;
    let src_identities: Vec<Identity> = src_store.list_identities()?;
    let src_tunnels: Vec<Tunnel> = src_store.list_tunnels()?;
    // Audit events of runnable audit items, read before any write.
    let src_audit: HashMap<String, Vec<AuthEvent>> = plan
        .runnable()
        .iter()
        .filter(|item| item.kind == "audit")
        .map(|item| {
            Ok((
                item.src_name.clone(),
                src_store.list_auth_events_for_host(&item.src_name)?,
            ))
        })
        .collect::<anyhow::Result<_>>()?;
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

    // Audit rows above this id are the ones this run writes.
    let audit_floor: i64 = dest_store
        .with_conn(|conn| {
            Ok(
                conn.query_row("SELECT COALESCE(MAX(id), 0) FROM auth_events", [], |r| {
                    r.get(0)
                })?,
            )
        })
        .unwrap_or(0);
    dest_store
        .with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            // Dependency order, not plan order: a failure must be observed
            // before its dependents are attempted.
            for kind in ["identity", "group", "host", "tunnel", "audit"] {
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
                                    result.created_groups.push(CreatedRow {
                                        name: item.src_name.clone(),
                                        dest_name: item.dest_name.clone(),
                                        identity_src_id: None,
                                        src_id: src_group.id,
                                        dest_id: group.id,
                                        had_secret: false,
                                    });
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
                                    result.created_identities.push(CreatedRow {
                                        name: item.src_name.clone(),
                                        src_id: src_identity.id,
                                        dest_id: identity.id,
                                        dest_name: item.dest_name.clone(),
                                        identity_src_id: None,
                                        had_secret: src_identity.has_password,
                                    });
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
                                    secret_lookup: src_host.secret_lookup.clone(),
                                    username: src_host.username.clone(),
                                    session_logging: src_host.session_logging,
                                    ping: src_host.ping,
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
                            result.created_hosts.push(CreatedRow {
                                name: item.src_name.clone(),
                                src_id: src_host.id,
                                dest_id: created_host.id,
                                dest_name: item.dest_name.clone(),
                                identity_src_id: src_host.identity_id,
                                had_secret: src_host.has_password,
                            });
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
                                    // Host-less, the keep-alive loop could only fail on it.
                                    auto_connect: src_tunnel.auto_connect && dest_host_id.is_some(),
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
                        "audit" => {
                            // The host row exists by now (kind order); a failed
                            // host already rolled everything back.
                            for event in src_audit.get(&item.src_name).into_iter().flatten() {
                                if let Err(e) =
                                    super::hosts::insert_auth_event_on(conn, &item.dest_name, event)
                                {
                                    record_failure!(
                                        item,
                                        failed_groups,
                                        e.context(format!(
                                            "copy audit events of '{}'",
                                            item.src_name
                                        ))
                                    );
                                    break;
                                }
                                result.audit_events += 1;
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
        let mut keep_hosts: HashSet<i64> = HashSet::new();
        let mut keep_identities: HashSet<i64> = HashSet::new();
        if let Some(stores) = &secrets {
            let carried = carry_secrets(&result, stores, dest_store);
            keep_hosts = carried.failed_hosts;
            keep_identities = carried.failed_identities;
            result.carry_failures = carried.messages;
            result.kept_in_source = carried.kept;
            remove_kept_from_destination(
                &mut result,
                plan,
                stores,
                dest_store,
                &keep_hosts,
                &keep_identities,
                audit_floor,
            );
        }
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
        match delete_source_originals(
            src_store,
            plan,
            &owned_hosts,
            &owned_groups,
            &owned_identities,
            &Keep {
                hosts: &keep_hosts,
                identities: &keep_identities,
            },
        ) {
            Ok((hosts, identities)) => {
                result.deleted_host_ids = hosts;
                result.deleted_identity_ids = identities;
            }
            // The destination is committed: an `Err` here would invite a retry
            // that duplicates every row. The source stays whole, so nothing is
            // purged, and the caller reports it like a failed carry.
            Err(e) => result.carry_failures.push(format!(
                "the originals could not be removed from the source profile, the destination already holds the copies: {e:#}"
            )),
        }
    }

    Ok(result)
}

/// A host that stays in the source must not also stay in the destination, or
/// a repeated move would add `(copy 1)`: remove the destination copies of the
/// kept hosts (tunnels cascade; the audit events this run inserted; the saved
/// secret written for it), then the groups and identities this run created
/// that nothing in the destination uses any more. Failures are appended to
/// `carry_failures`.
fn remove_kept_from_destination(
    result: &mut TransferResult,
    plan: &TransferPlan,
    stores: &SecretStores<'_>,
    dest_store: &LauncherStore,
    keep_hosts: &HashSet<i64>,
    keep_identities: &HashSet<i64>,
    audit_floor: i64,
) {
    use crate::credentials::{host_key, identity_key};
    let mut problems = Vec::new();
    let mut removed_any = false;
    let kept_rows: Vec<CreatedRow> = result
        .created_hosts
        .iter()
        .filter(|r| keep_hosts.contains(&r.src_id))
        .cloned()
        .collect();
    for row in &kept_rows {
        let with_audit = plan
            .runnable()
            .iter()
            .any(|i| i.kind == "audit" && i.src_name == row.name);
        let cleaned = dest_store.with_conn(|conn| {
            // The host's tunnels go with it (cascade): count them out.
            let tunnels = super::tunnels::list_tunnels_on(conn)?
                .iter()
                .filter(|t| t.host_id == Some(row.dest_id))
                .count();
            super::hosts::delete_host_on(conn, row.dest_id)?;
            let events = if with_audit {
                super::hosts::delete_auth_events_since_on(conn, &row.dest_name, audit_floor)?
            } else {
                0
            };
            Ok((1 + tunnels, events))
        });
        match cleaned {
            Ok((gone, events)) => {
                removed_any = true;
                result.transferred = result.transferred.saturating_sub(gone);
                result.audit_events = result.audit_events.saturating_sub(events);
                result.renamed.retain(|n| n != &row.dest_name);
                // Best effort: a secret written before the row was kept.
                let _ = stores.dest.delete(&host_key(row.dest_id));
            }
            Err(e) => problems.push(format!(
                "host '{}': the destination copy could not be removed: {e:#}",
                row.name
            )),
        }
    }
    if removed_any || !keep_identities.is_empty() {
        let swept = dest_store.with_conn(|conn| {
            let mut gone = 0usize;
            for group in result.created_groups.iter().filter(|_| removed_any) {
                let used: i64 = conn.query_row(
                    "SELECT (SELECT COUNT(*) FROM hosts WHERE group_id = ?1)
                          + (SELECT COUNT(*) FROM host_group_memberships WHERE group_id = ?1)",
                    [group.dest_id],
                    |r| r.get(0),
                )?;
                if used == 0 && super::hosts::delete_group_on(conn, group.dest_id)? {
                    gone += 1;
                    result.renamed.retain(|n| n != &group.dest_name);
                }
            }
            let mut dropped_identities = Vec::new();
            for identity in &result.created_identities {
                let unused =
                    super::identities::count_hosts_using_identity_on(conn, identity.dest_id)? == 0
                        && !super::hosts::group_default_uses_identity_on(conn, identity.dest_id)?;
                // An identity kept in the source (its secret did not carry) must
                // not stay in the destination either; others only when a removed
                // host left them unused.
                let sweep = removed_any || keep_identities.contains(&identity.src_id);
                if sweep && unused && super::identities::delete_identity_on(conn, identity.dest_id)?
                {
                    gone += 1;
                    dropped_identities.push(identity.clone());
                }
            }
            Ok((gone, dropped_identities))
        });
        match swept {
            Ok((gone, dropped)) => {
                result.transferred = result.transferred.saturating_sub(gone);
                for identity in dropped {
                    result.renamed.retain(|n| n != &identity.dest_name);
                    let _ = stores.dest.delete(&identity_key(identity.dest_id));
                }
            }
            Err(e) => problems.push(format!(
                "the destination copies of groups and identities could not be tidied: {e:#}"
            )),
        }
    }
    result.carry_failures.extend(problems);
}

fn transferred(result: &mut TransferResult, item: &PlannedItem) {
    result.transferred += 1;
    if item.renamed {
        result.renamed.push(item.dest_name.clone());
    }
}

/// Source rows a move must not delete: their secret did not carry.
struct Keep<'a> {
    hosts: &'a HashSet<i64>,
    identities: &'a HashSet<i64>,
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
    keep: &Keep<'_>,
) -> anyhow::Result<(Vec<i64>, Vec<i64>)> {
    let runnable: Vec<&PlannedItem> = plan.runnable();
    src_store
        .with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            let mut deleted_hosts: Vec<i64> = Vec::new();
            let mut deleted_identities: Vec<i64> = Vec::new();
            for item in runnable.iter().filter(|i| i.kind == "host") {
                let Some(host) = src_hosts.get(item.src_name.as_str()) else {
                    continue;
                };
                if host.source != HostSource::Launcher || keep.hosts.contains(&host.id) {
                    continue;
                }
                super::tunnels::detach_tunnels_of_host_on(conn, host.id)?;
                super::hosts::delete_host_on(conn, host.id)?;
                deleted_hosts.push(host.id);
                if runnable
                    .iter()
                    .any(|i| i.kind == "audit" && i.src_name == item.src_name)
                {
                    super::hosts::delete_auth_events_for_host_on(conn, &host.name)?;
                }
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
                    // A host kept in the source keeps its tunnels too.
                    .filter(|t| !t.host_id.is_some_and(|h| keep.hosts.contains(&h)))
                    .find(|t| tunnel_token_matches(t, &item.src_name))
                    .map(|t| t.id);
                if let Some(id) = target {
                    consumed.insert(id);
                    super::tunnels::delete_tunnel_on(conn, id)?;
                }
            }
            // Which planned groups really go. A host kept in the source keeps
            // its groups (deleting one would un-group it and drop the group's
            // defaults). A guided move also keeps a group that still has a
            // member staying behind, and a group with a child group that stays:
            // the child would be orphaned and lose the inherited defaults.
            let kept_groups: HashSet<i64> = src_hosts
                .values()
                .filter(|h| keep.hosts.contains(&h.id))
                .flat_map(|h| h.groups.iter().map(|g| g.id).chain(h.group_id))
                .collect();
            let mut going: HashSet<i64> = runnable
                .iter()
                .filter(|i| i.kind == "group")
                .filter_map(|i| src_groups.get(i.src_name.as_str()))
                .filter(|g| !g.reserved && !kept_groups.contains(&g.id))
                .filter(|g| {
                    !plan.leave_shared_groups
                        || !src_hosts.values().any(|h| {
                            h.groups.iter().any(|m| m.id == g.id) && !deleted_hosts.contains(&h.id)
                        })
                })
                .map(|g| g.id)
                .collect();
            if plan.leave_shared_groups {
                loop {
                    let snapshot = going.clone();
                    going.retain(|id| {
                        !src_groups
                            .values()
                            .any(|c| c.parent_id == Some(*id) && !snapshot.contains(&c.id))
                    });
                    if going.len() == snapshot.len() {
                        break;
                    }
                }
            }
            for item in runnable.iter().filter(|i| i.kind == "group") {
                let Some(group) = src_groups.get(item.src_name.as_str()) else {
                    continue;
                };
                if !going.contains(&group.id) {
                    continue;
                }
                super::hosts::delete_group_on(conn, group.id)?;
            }
            for item in runnable.iter().filter(|i| i.kind == "identity") {
                let Some(identity) = src_identities.get(item.src_name.as_str()) else {
                    continue;
                };
                if keep.identities.contains(&identity.id) {
                    continue;
                }
                let hosts_still_use =
                    super::identities::count_hosts_using_identity_on(conn, identity.id)?;
                let group_default_still_use =
                    super::hosts::group_default_uses_identity_on(conn, identity.id)?;
                if hosts_still_use == 0 && !group_default_still_use {
                    super::identities::delete_identity_on(conn, identity.id)?;
                    deleted_identities.push(identity.id);
                }
            }
            tx.commit()?;
            Ok((deleted_hosts, deleted_identities))
        })
        .context("delete moved source originals")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::NewHost;

    /// The destination is committed before the source originals are deleted: a
    /// failing delete must come back as a warning, not as an `Err` that arms a
    /// retry which can only duplicate the rows or fail on their names.
    #[test]
    fn a_failed_source_delete_after_the_commit_is_a_warning_not_an_error() {
        let mut src = LauncherStore::open_in_memory().unwrap();
        src.create_host(&NewHost::launcher("web", "web.example.com"))
            .unwrap();
        src.with_conn(|c| {
            c.execute_batch(
                "CREATE TRIGGER no_delete BEFORE DELETE ON hosts
                 BEGIN SELECT RAISE(ABORT, 'locked'); END;",
            )?;
            Ok(())
        })
        .unwrap();
        let mut dest = LauncherStore::open_in_memory().unwrap();
        let plan = build_transfer_plan(
            &TransferSnapshot::capture(&src).unwrap(),
            &DestIndex::capture(&dest, "dest").unwrap(),
            &TransferSelection {
                host_names: vec!["web".into()],
                ..Default::default()
            },
            TransferMode::Move,
            GroupTransferMode::GroupOnly,
            &HashSet::new(),
        );
        let result =
            apply_transfer_plan_with_secrets(&mut src, &mut dest, &plan, TransferMode::Move, None)
                .unwrap();
        assert!(result.deleted_host_ids.is_empty());
        assert!(result.carry_failures[0].contains("originals could not be removed"));
        assert_eq!(dest.list_hosts().unwrap().len(), 1);
        assert_eq!(src.list_hosts().unwrap().len(), 1);
    }
}
