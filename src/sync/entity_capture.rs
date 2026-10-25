//! Local edits of hosts, groups and identities become ops.
//!
//! The sink is installed on the [`LauncherStore`] of a profile; every local write (TUI form,
//! CLI verb, importer, transfer) tells it which row changed, and while the profile is in a
//! ring it appends ONE op per changed entity: the fields that differ between the row and the
//! entity's effective view (a new entity: everything that is not a default). Applying a synced
//! op writes through the connection-level helpers and never reaches the sink; and because the
//! comparison is state based, a row that already equals the view yields no op at all. A
//! profile without a ring appends nothing.
//!
//! `ssh_config` hosts are the machine's own, never replicated.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use super::apply::{effective_view, hops_to_wire, render_hops, rendered_name, View};
use super::crypto::DeviceIdentity;
use super::identity_store::load_or_init_identity;
use super::oplog::canonical::{group_field_values, host_field_values, identity_field_values};
use super::oplog::op::{GroupFields, HostFields, IdentityFields, OpBody};
use super::oplog::store_impl::append_local_on;
use super::oplog::tier::EntityKind;
use crate::credentials::PasswordStore;
use crate::profile::ProfilePaths;
use crate::store::{
    group_uid_on, host_uid_on, identity_uid_on, load_group_by_id, load_host_by_id, ChangeScope,
    HostSource, LauncherStore, LocalChangeSink, Uid, FAVORITES_GROUP_UID,
};

/// Where the device key lives and how to open the profile's credential store.
type KeySource = (ProfilePaths, fn(&ProfilePaths) -> Box<dyn PasswordStore>);

pub struct EntityCapture {
    /// Where the device key lives; `None` when the identity was handed in (tests).
    source: Option<KeySource>,
    identity: Mutex<Option<Arc<DeviceIdentity>>>,
}

/// Install the sink on `store` for the profile `paths`. Nothing is read or prompted until the
/// first change in a ring: the device key loads lazily.
pub fn install(
    store: &LauncherStore,
    paths: &ProfilePaths,
    open_secrets: fn(&ProfilePaths) -> Box<dyn PasswordStore>,
) -> Result<()> {
    store.set_local_change_sink(Box::new(EntityCapture {
        source: Some((paths.clone(), open_secrets)),
        identity: Mutex::new(None),
    }));
    Ok(())
}

/// Install a sink whose device key cannot be loaded (a locked identity).
#[cfg(test)]
pub(crate) fn install_locked(store: &LauncherStore) {
    store.set_local_change_sink(Box::new(EntityCapture {
        source: None,
        identity: Mutex::new(None),
    }));
}

/// Install with an identity at hand.
#[cfg(test)]
pub(crate) fn install_with_identity(store: &LauncherStore, id: Arc<DeviceIdentity>) -> Result<()> {
    store.set_local_change_sink(Box::new(EntityCapture {
        source: None,
        identity: Mutex::new(Some(id)),
    }));
    Ok(())
}

/// Publish what the ring does not have yet: hosts that predate the ring, a change that could
/// not be recorded when it was made. Cheap when there is nothing (one query); a sync round
/// calls it first.
pub fn publish_pending(store: &LauncherStore, id: &Arc<DeviceIdentity>) -> Result<()> {
    let unpublished = store.with_conn(|conn| {
        if !in_ring(conn)? {
            return Ok(false);
        }
        Ok(conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM hosts h WHERE h.source = 'launcher' AND NOT EXISTS (
                 SELECT 1 FROM sync_entities e WHERE e.kind = 'host' AND e.uid = h.uid AND e.alive = 1))",
            [],
            |r| r.get(0),
        )?)
    })?;
    if unpublished {
        let sink = EntityCapture {
            source: None,
            identity: Mutex::new(Some(id.clone())),
        };
        store.with_conn(|conn| sink.changed(conn, ChangeScope::Everything))?;
    }
    Ok(())
}

/// [`install`] for a store opened from a database path (the other side of a transfer).
pub fn install_for_db(
    store: &LauncherStore,
    db: &Path,
    open_secrets: fn(&ProfilePaths) -> Box<dyn PasswordStore>,
) -> Result<()> {
    install(store, &paths_for_db(db)?, open_secrets)
}

fn paths_for_db(db: &Path) -> Result<ProfilePaths> {
    let roots = crate::profile::resolve_roots()?;
    let root = db.parent().context("database path has no directory")?;
    if let (Some(state), Some(name)) = (
        crate::profile::ProfileState::load(&roots.data_root)?,
        root.file_name().and_then(|n| n.to_str()),
    ) {
        if let Some(record) = state.by_name(name) {
            if crate::profile::ProfilePaths::profile_dir(&roots.data_root, &record.name) == root {
                return Ok(crate::profile::profile_paths(
                    &roots,
                    record,
                    Default::default(),
                ));
            }
        }
    }
    Ok(crate::profile::compat_paths(&roots, Default::default()))
}

fn in_ring(conn: &Connection) -> Result<bool> {
    Ok(
        conn.query_row("SELECT EXISTS(SELECT 1 FROM sync_devices)", [], |r| {
            r.get(0)
        })?,
    )
}

impl EntityCapture {
    fn identity(&self) -> Result<Arc<DeviceIdentity>> {
        let mut slot = self.identity.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            let (paths, open_secrets) = self.source.as_ref().context("no device identity")?;
            let secrets = open_secrets(paths);
            *slot = Some(Arc::new(load_or_init_identity(paths, secrets.as_ref())?));
        }
        Ok(slot.clone().expect("set above"))
    }

    fn append(&self, conn: &Connection, body: OpBody) -> Result<()> {
        let note = "the change is saved here but could not be recorded for sync";
        let id = self.identity().context(note)?;
        append_local_on(conn, &id, body, crate::store::now_ts()).context(note)?;
        Ok(())
    }
}

impl LocalChangeSink for EntityCapture {
    fn changed(&self, conn: &Connection, scope: ChangeScope) -> Result<()> {
        if !in_ring(conn)? {
            return Ok(());
        }
        match scope {
            ChangeScope::Host(id) => self.host(conn, id),
            ChangeScope::Group(id) => self.group(conn, id),
            ChangeScope::Identity(id) => self.identity_row(conn, id),
            ChangeScope::Deleted(kind, uid) => self.deleted(conn, kind, &uid),
            ChangeScope::Everything => {
                for id in column(conn, "SELECT id FROM identities ORDER BY id")? {
                    self.identity_row(conn, id)?;
                }
                for id in column(
                    conn,
                    "SELECT id FROM host_groups WHERE reserved = 0 ORDER BY id",
                )? {
                    self.group(conn, id)?;
                }
                for id in column(
                    conn,
                    "SELECT id FROM hosts WHERE source = 'launcher' ORDER BY id",
                )? {
                    self.host(conn, id)?;
                }
                Ok(())
            }
        }
    }
}

fn column(conn: &Connection, sql: &str) -> Result<Vec<i64>> {
    let mut st = conn.prepare(sql)?;
    let rows = st.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// Absent in the view and equal to this: nothing to say.
fn is_default(v: &Value) -> bool {
    match v {
        Value::Null | Value::Bool(false) => true,
        Value::Array(a) => a.is_empty(),
        _ => false,
    }
}

/// Names of the fields whose local value is not what the view says.
fn differing(
    view: &View,
    fields: Vec<(&'static str, Value)>,
    required: &[&str],
) -> BTreeSet<&'static str> {
    fields
        .into_iter()
        .filter(|(k, v)| match view.fields.get(*k) {
            Some(cur) => cur != v,
            None => (!view.alive && required.contains(k)) || !is_default(v),
        })
        .map(|(k, _)| k)
        .collect()
}

fn s(v: &Option<String>) -> Option<Option<String>> {
    Some(v.clone())
}

impl EntityCapture {
    fn deleted(&self, conn: &Connection, kind: EntityKind, uid: &Uid) -> Result<()> {
        if !effective_view(conn, kind, uid)?.alive {
            return Ok(());
        }
        self.append(
            conn,
            match kind {
                EntityKind::Host => OpBody::HostDelete { host_uid: *uid },
                EntityKind::Group => OpBody::GroupDelete { group_uid: *uid },
                EntityKind::Identity => OpBody::IdentityDelete { identity_uid: *uid },
            },
        )
    }

    fn identity_row(&self, conn: &Connection, id: i64) -> Result<()> {
        let Some(uid) = identity_uid_on(conn, id)? else {
            return Ok(());
        };
        let row: Option<(String, Option<String>, bool)> = conn
            .query_row(
                "SELECT name, username, has_password FROM identities WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((name, username, has_password)) = row else {
            return Ok(());
        };
        let view = effective_view(conn, EntityKind::Identity, &uid)?;
        let full = IdentityFields {
            name: Some(name.clone()),
            username: s(&username),
            has_password: Some(has_password),
        };
        let mut diff = differing(&view, identity_field_values(&full), &["name"]);
        keep_name(conn, EntityKind::Identity, &uid, &view, &name, &mut diff)?;
        if diff.is_empty() {
            return Ok(());
        }
        let keep = |k: &str| diff.contains(k);
        let fields = IdentityFields {
            name: if keep("name") { full.name } else { None },
            username: if keep("username") {
                full.username
            } else {
                None
            },
            has_password: if keep("has_password") {
                full.has_password
            } else {
                None
            },
        };
        self.append(
            conn,
            OpBody::IdentityUpsert {
                identity_uid: uid,
                fields,
            },
        )
    }

    fn group(&self, conn: &Connection, id: i64) -> Result<()> {
        let Some(g) = load_group_by_id(conn, id)? else {
            return Ok(());
        };
        let Some(uid) = group_uid_on(conn, id)? else {
            return Ok(());
        };
        if g.reserved || uid == FAVORITES_GROUP_UID {
            return Ok(());
        }
        let view = effective_view(conn, EntityKind::Group, &uid)?;
        let full = GroupFields {
            name: Some(g.name.clone()),
            default_identity_uid: Some(match g.default_identity_id {
                Some(i) => identity_uid_on(conn, i)?,
                None => None,
            }),
            default_username: s(&g.default_username),
            default_port: Some(g.default_port),
            default_proxy_jump: Some(match &g.default_proxy_jump {
                Some(j) => Some(hops_to_wire(conn, j)?),
                None => None,
            }),
            default_transport: Some(g.default_transport.map(|t| t.to_db().to_string())),
            default_forward_agent: Some(g.default_forward_agent),
            parent_uid: Some(match g.parent_id {
                Some(p) => group_uid_on(conn, p)?,
                None => None,
            }),
        };
        let mut diff = differing(&view, group_field_values(&full), &["name"]);
        jump_equal(
            conn,
            &view,
            "default_proxy_jump",
            g.default_proxy_jump.as_deref(),
            &mut diff,
        )?;
        keep_name(conn, EntityKind::Group, &uid, &view, &g.name, &mut diff)?;
        if diff.is_empty() {
            return Ok(());
        }
        let keep = |k: &str| diff.contains(k);
        macro_rules! pick {
            ($f:ident) => {
                if keep(stringify!($f)) {
                    full.$f
                } else {
                    Default::default()
                }
            };
        }
        let fields = GroupFields {
            name: pick!(name),
            default_identity_uid: pick!(default_identity_uid),
            default_username: pick!(default_username),
            default_port: pick!(default_port),
            default_proxy_jump: pick!(default_proxy_jump),
            default_transport: pick!(default_transport),
            default_forward_agent: pick!(default_forward_agent),
            parent_uid: pick!(parent_uid),
        };
        self.append(
            conn,
            OpBody::GroupUpsert {
                group_uid: uid,
                fields,
            },
        )
    }

    fn host(&self, conn: &Connection, id: i64) -> Result<()> {
        let Some(h) = load_host_by_id(conn, id)? else {
            return Ok(());
        };
        if h.source != HostSource::Launcher {
            return Ok(());
        }
        let Some(uid) = host_uid_on(conn, id)? else {
            return Ok(());
        };
        let view = effective_view(conn, EntityKind::Host, &uid)?;
        let mut group_uids = Vec::new();
        for g in h.groups.iter().filter(|g| !g.reserved) {
            if let Some(u) = group_uid_on(conn, g.id)? {
                group_uids.push(u);
            }
        }
        group_uids.sort();
        let full = HostFields {
            name: Some(h.name.clone()),
            label: s(&h.label),
            address: Some(h.address.clone()),
            port: Some(h.port),
            username: s(&h.username),
            proxy_jump: Some(match &h.proxy_jump {
                Some(j) => Some(hops_to_wire(conn, j)?),
                None => None,
            }),
            forward_agent: Some(h.forward_agent),
            remote_command: s(&h.remote_command),
            transport: Some(h.transport.map(|t| t.to_db().to_string())),
            environment: s(&h.environment),
            os_icon: s(&h.os_icon),
            tags: Some(h.tags.clone()),
            notes: s(&h.notes),
            favorite: Some(h.favorite),
            secret_lookup: s(&h.secret_lookup),
            has_password: Some(h.has_password),
            group_uids: Some(group_uids),
            identity_uid: Some(match h.identity_id {
                Some(i) => identity_uid_on(conn, i)?,
                None => None,
            }),
        };
        let mut diff = differing(&view, host_field_values(&full), &["name", "address"]);
        jump_equal(
            conn,
            &view,
            "proxy_jump",
            h.proxy_jump.as_deref(),
            &mut diff,
        )?;
        keep_name(conn, EntityKind::Host, &uid, &view, &h.name, &mut diff)?;
        if diff.is_empty() {
            return Ok(());
        }
        let keep = |k: &str| diff.contains(k);
        macro_rules! pick {
            ($f:ident) => {
                if keep(stringify!($f)) {
                    full.$f
                } else {
                    Default::default()
                }
            };
        }
        let fields = HostFields {
            name: pick!(name),
            label: pick!(label),
            address: pick!(address),
            port: pick!(port),
            username: pick!(username),
            proxy_jump: pick!(proxy_jump),
            forward_agent: pick!(forward_agent),
            remote_command: pick!(remote_command),
            transport: pick!(transport),
            environment: pick!(environment),
            os_icon: pick!(os_icon),
            tags: pick!(tags),
            notes: pick!(notes),
            favorite: pick!(favorite),
            secret_lookup: pick!(secret_lookup),
            has_password: pick!(has_password),
            group_uids: pick!(group_uids),
            identity_uid: pick!(identity_uid),
        };
        self.append(
            conn,
            OpBody::HostUpsert {
                host_uid: uid,
                fields,
            },
        )
    }
}

/// The row's name is the view's name when it is exactly what the projection renders for it
/// (a suffix `name (uid8)` shown for a name another entity holds is not a rename).
fn keep_name(
    conn: &Connection,
    kind: EntityKind,
    uid: &Uid,
    view: &View,
    row_name: &str,
    diff: &mut BTreeSet<&'static str>,
) -> Result<()> {
    if view.base_name().is_some()
        && rendered_name(conn, kind, uid, view)?.as_deref() == Some(row_name)
    {
        diff.remove("name");
    }
    Ok(())
}

/// A jump column the projection cleared (a hop it cannot render) is not an edit either.
fn jump_equal(
    conn: &Connection,
    view: &View,
    field: &'static str,
    local: Option<&str>,
    diff: &mut BTreeSet<&'static str>,
) -> Result<()> {
    if let Some(Value::String(wire)) = view.fields.get(field) {
        if render_hops(conn, wire)?.as_deref() == local {
            diff.remove(field);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
