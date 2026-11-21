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

use super::apply::{effective_view, hops_to_wire, render_hops, rendered_name, validate_body, View};
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

const NOT_RECORDED: &str = "the change is saved here but could not be recorded for sync";

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
/// not be recorded when it was made (a locked key, a full disk, a field the op validator
/// refused). State based, so an entity that already matches its view costs queries and no op;
/// a sync round calls it first. BEST EFFORT: an entity that cannot be published is skipped
/// with a warning ([`LauncherStore::take_sync_warning`]) and the round never depends on it.
pub fn publish_pending(store: &LauncherStore, id: &Arc<DeviceIdentity>) -> Result<()> {
    if !store.with_conn(in_ring)? {
        return Ok(());
    }
    let sink = EntityCapture {
        source: None,
        identity: Mutex::new(Some(id.clone())),
    };
    if let Err(e) = store.with_conn(|conn| sink.changed(conn, ChangeScope::Everything)) {
        store.record_sync_warning(format!("{e:#}"));
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
            // A profile in a ring always has its device key. A missing file means the profile was
            // moved without its `sync/` directory: a NEW identity here would sign as an unknown
            // author and register itself as a holder, so refuse instead.
            anyhow::ensure!(
                super::identity_store::key_path(paths).exists(),
                "this profile is in a ring but its device key ({}) is missing: restore it or join the ring again",
                super::identity_store::key_path(paths).display()
            );
            let secrets = open_secrets(paths);
            *slot = Some(Arc::new(load_or_init_identity(paths, secrets.as_ref())?));
        }
        Ok(slot.clone().expect("set above"))
    }

    fn append(&self, conn: &Connection, body: OpBody) -> Result<()> {
        let id = self.identity().context(NOT_RECORDED)?;
        append_local_on(conn, &id, body, crate::store::now_ts()).context(NOT_RECORDED)?;
        Ok(())
    }
}

impl EntityCapture {
    /// Append `make(fields)`, leaving out any field the op validator refuses on its own (a note
    /// over the limit, a control character in a label): the rest goes through, and the refused
    /// ones come back as ONE error AFTER it, so the user is told which field is not in the ring
    /// instead of every later edit of the entity failing with it.
    fn append_valid(
        &self,
        conn: &Connection,
        diff: BTreeSet<&'static str>,
        make: impl Fn(&BTreeSet<&'static str>) -> OpBody,
    ) -> Result<()> {
        let mut keep = diff.clone();
        let mut refused: Vec<String> = Vec::new();
        if let Err(whole) = validate_body(&make(&keep)) {
            for f in &diff {
                let one = BTreeSet::from([*f]);
                if let Err(why) = validate_body(&make(&one)) {
                    refused.push(format!("{f}: {why}"));
                    keep.remove(f);
                }
            }
            if refused.is_empty() {
                // Not attributable to one field: say why and send nothing.
                refused.push(whole);
                keep.clear();
            }
        }
        if !keep.is_empty() {
            self.append(conn, make(&keep))?;
        }
        if refused.is_empty() {
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "saved here, but not synced because the ring would refuse it ({})",
                refused.join("; ")
            ))
        }
    }
}

impl LocalChangeSink for EntityCapture {
    fn prepare(&self) -> Result<()> {
        self.identity().map(|_| ()).context(NOT_RECORDED)
    }

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
                // One entity that cannot be recorded never stops the others: keep the first
                // problem as the warning and go on.
                let mut first: Option<anyhow::Error> = None;
                let mut note = |r: Result<()>| {
                    if let Err(e) = r {
                        first.get_or_insert(e);
                    }
                };
                for id in column(conn, "SELECT id FROM identities ORDER BY id")? {
                    note(self.identity_row(conn, id));
                }
                for id in column(
                    conn,
                    "SELECT id FROM host_groups WHERE reserved = 0 ORDER BY id",
                )? {
                    note(self.group(conn, id));
                }
                for id in column(
                    conn,
                    "SELECT id FROM hosts WHERE source = 'launcher' ORDER BY id",
                )? {
                    note(self.host(conn, id));
                }
                first.map_or(Ok(()), Err)
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

/// What the projection renders for a view value: a reference to an entity this device has no
/// row for is dropped (the row cannot hold it), so the row's "none" IS the view's value.
fn rendered(conn: &Connection, field: &str, cur: &Value) -> Result<Value> {
    let known = |kind: EntityKind, v: &Value| -> Result<bool> {
        let Some(uid) = v.as_str().and_then(crate::store::uid_from_hex) else {
            return Ok(false);
        };
        Ok(match kind {
            EntityKind::Identity => crate::store::identity_id_by_uid_on(conn, &uid)?,
            EntityKind::Group => crate::store::group_id_by_uid_on(conn, &uid)?,
            EntityKind::Host => crate::store::host_id_by_uid_on(conn, &uid)?,
        }
        .is_some())
    };
    Ok(match (field, cur) {
        ("identity_uid" | "default_identity_uid", v @ Value::String(_)) => {
            if known(EntityKind::Identity, v)? {
                v.clone()
            } else {
                Value::Null
            }
        }
        ("parent_uid", v @ Value::String(_)) => {
            if known(EntityKind::Group, v)? {
                v.clone()
            } else {
                Value::Null
            }
        }
        ("group_uids", Value::Array(list)) => {
            let mut kept = Vec::new();
            for v in list {
                if known(EntityKind::Group, v)? {
                    kept.push(v.clone());
                }
            }
            Value::Array(kept)
        }
        _ => cur.clone(),
    })
}

/// A field the view does not name is "inherit" for the row too. The projection stores NULL for
/// a port or transport the ring did not give, so on an entity the ring knows a value that is
/// there (even 22 or ssh) is the user's explicit choice and is published. A host the ring has
/// not seen yet is created by the form with the row defaults (22, ssh): those are not a choice.
fn is_inherit(field: &str, v: &Value, alive: bool) -> bool {
    match field {
        "port" if !alive => v == &Value::from(22),
        "transport" if !alive => v == &Value::from("ssh"),
        _ => is_default(v),
    }
}

/// Names of the fields whose local value is not what the view renders to.
fn differing(
    conn: &Connection,
    view: &View,
    fields: Vec<(&'static str, Value)>,
    required: &[&str],
) -> Result<BTreeSet<&'static str>> {
    let mut out = BTreeSet::new();
    for (k, v) in fields {
        let differs = match view.fields.get(k) {
            Some(cur) => rendered(conn, k, cur)? != v,
            None => (!view.alive && required.contains(&k)) || !is_inherit(k, &v, view.alive),
        };
        if differs {
            out.insert(k);
        }
    }
    Ok(out)
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
        let mut diff = differing(conn, &view, identity_field_values(&full), &["name"])?;
        keep_name(conn, EntityKind::Identity, &uid, &view, &name, &mut diff)?;
        if diff.is_empty() {
            return Ok(());
        }
        self.append_valid(conn, diff, |keep| {
            let pick = |k: &str| keep.contains(k);
            OpBody::IdentityUpsert {
                identity_uid: uid,
                fields: IdentityFields {
                    name: if pick("name") {
                        full.name.clone()
                    } else {
                        None
                    },
                    username: if pick("username") {
                        full.username.clone()
                    } else {
                        None
                    },
                    has_password: if pick("has_password") {
                        full.has_password
                    } else {
                        None
                    },
                },
            }
        })
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
        let mut diff = differing(conn, &view, group_field_values(&full), &["name"])?;
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
        self.append_valid(conn, diff, |keep| {
            macro_rules! pick {
                ($f:ident) => {
                    if keep.contains(stringify!($f)) {
                        full.$f.clone()
                    } else {
                        Default::default()
                    }
                };
            }
            OpBody::GroupUpsert {
                group_uid: uid,
                fields: GroupFields {
                    name: pick!(name),
                    default_identity_uid: pick!(default_identity_uid),
                    default_username: pick!(default_username),
                    default_port: pick!(default_port),
                    default_proxy_jump: pick!(default_proxy_jump),
                    default_transport: pick!(default_transport),
                    default_forward_agent: pick!(default_forward_agent),
                    parent_uid: pick!(parent_uid),
                },
            }
        })
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
        let mut diff = differing(conn, &view, host_field_values(&full), &["name", "address"])?;
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
        let was_new = !view.alive;
        let published = self.append_valid(conn, diff, |keep| {
            macro_rules! pick {
                ($f:ident) => {
                    if keep.contains(stringify!($f)) {
                        full.$f.clone()
                    } else {
                        Default::default()
                    }
                };
            }
            OpBody::HostUpsert {
                host_uid: uid,
                fields: HostFields {
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
                },
            }
        });
        if was_new {
            // The form's defaults (22, ssh) were not a choice and were not published: the row
            // now inherits them like a host the ring gave, so a later explicit choice shows.
            conn.execute(
                "UPDATE hosts SET port = NULL WHERE id = ?1 AND port = 22",
                params![id],
            )?;
            conn.execute(
                "UPDATE hosts SET transport = NULL WHERE id = ?1 AND transport = 'ssh'",
                params![id],
            )?;
        }
        published
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
