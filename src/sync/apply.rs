//! Applying replicated state to `hosts`, `host_groups` and `identities`.
//!
//! The model: the register table (`sync_fields`) plus the op states fully
//! determine an entity's *effective view* (see [`effective_view`]); the SQL rows
//! are a pure projection of that view. [`reproject`] recomputes the view of every
//! touched entity and everything that depends on it, then writes the rows
//! through the existing `*_on` store helpers, so the existing validators
//! (`reject_option_like`, `ssh::destination_problem`, `map::hops::validate_raw`)
//! guard every apply, automatic or approved. Because the projection only reads
//! the effective view, the result does not depend on arrival order.
//!
//! Local-only things are never touched: `source = ssh_config` rows, tunnels,
//! snippets, sort order, key file paths. Hop tokens naming a saved host travel as
//! `#<uid hex>` and are rendered as the local host name here.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::session_transport::SessionTransport;
use crate::store::{
    self, uid_from_hex, uid_hex, GroupUid, HostSource, HostUpdate, NewHost, NewHostGroup,
    NewIdentity, Uid, FAVORITES_GROUP_UID,
};
use crate::sync::oplog::op::{GroupFields, HostFields, IdentityFields, OpBody};
use crate::sync::oplog::tier::EntityKind;

use crate::sync::limits::RESHARE_ITEMS_PER_OP;

const MAX_STR: usize = 4096;
const MAX_LIST: usize = 1024;

pub type Touched = BTreeSet<(EntityKind, Uid)>;

// ------------------------------------------------------------- validation

/// Static checks every op passes before it is chained: pure functions of the
/// op, so every honest peer agrees on the verdict. An `Err` is the reason.
pub fn validate_body(body: &OpBody) -> std::result::Result<(), String> {
    match body {
        OpBody::HostUpsert { host_uid, fields } => {
            reserved_check(EntityKind::Host, host_uid)?;
            validate_host(fields)
        }
        OpBody::GroupUpsert { group_uid, fields } => {
            reserved_check(EntityKind::Group, group_uid)?;
            validate_group(fields)
        }
        OpBody::IdentityUpsert { fields, .. } => validate_identity(fields),
        OpBody::GroupDelete { group_uid } => reserved_check(EntityKind::Group, group_uid),
        OpBody::DeviceAdd { name, .. } => text("device name", name),
        OpBody::ShareReshareBatch {
            new_holders, items, ..
        } => {
            if items.len() > RESHARE_ITEMS_PER_OP || new_holders.len() > MAX_LIST {
                return Err("reshare batch is over the item limit".into());
            }
            Ok(())
        }
        OpBody::ShareReshareConfirmBatch { items, .. } => {
            if items.len() > RESHARE_ITEMS_PER_OP {
                return Err("confirm batch is over the item limit".into());
            }
            Ok(())
        }
        OpBody::BulkExport { window_secs, .. } => {
            if *window_secs == 0 || *window_secs > 3600 {
                return Err("an export window is 1 to 3600 seconds".into());
            }
            Ok(())
        }
        OpBody::FactorRegister { factor, sealed, .. } => {
            let max = match factor {
                crate::sync::oplog::op::ThirdFactor::Paper => 0,
                crate::sync::oplog::op::ThirdFactor::EmailCode => 128,
            };
            if sealed.len() > max {
                return Err("sealed factor material is too long".into());
            }
            Ok(())
        }
        OpBody::ShareGrant { sealed, .. } => {
            if sealed.len() > 128 {
                return Err("a share grant is at most 128 bytes".into());
            }
            Ok(())
        }
        OpBody::CheckpointPropose { cut, required, .. } => {
            if cut.len() > 4 * MAX_LIST || required.len() > 4 * MAX_LIST {
                return Err("checkpoint cut is too large".into());
            }
            if cut.windows(2).any(|w| w[0].device_id >= w[1].device_id)
                || required.windows(2).any(|w| w[0] >= w[1])
            {
                return Err("cut and required must be sorted and unique".into());
            }
            Ok(())
        }
        OpBody::WrapKeyRotate { wrap_epoch, .. } if *wrap_epoch == 0 => {
            Err("wrap_epoch starts at 1".into())
        }
        _ => Ok(()),
    }
}

fn reserved_check(kind: EntityKind, uid: &Uid) -> std::result::Result<(), String> {
    if kind == EntityKind::Group && *uid == FAVORITES_GROUP_UID {
        return Err("the Favorites group is local".into());
    }
    Ok(())
}

fn text(what: &str, s: &str) -> std::result::Result<(), String> {
    if s.len() > MAX_STR {
        return Err(format!("{what} is too long"));
    }
    if s.chars().any(|c| c == '\0') {
        return Err(format!("{what} contains NUL"));
    }
    Ok(())
}

fn name_ok(what: &str, s: &str) -> std::result::Result<(), String> {
    text(what, s)?;
    if s.trim().is_empty() || s != s.trim() {
        return Err(format!("{what} must be non-empty and trimmed"));
    }
    store::reject_option_like(what, s).map_err(|e| e.to_string())
}

fn opt_text(what: &str, v: &Option<Option<String>>) -> std::result::Result<(), String> {
    match v {
        Some(Some(s)) => text(what, s),
        _ => Ok(()),
    }
}

fn port_ok(what: &str, v: &Option<Option<u16>>) -> std::result::Result<(), String> {
    if matches!(v, Some(Some(0))) {
        return Err(format!("{what} 0 is not a port"));
    }
    Ok(())
}

fn validate_jump(what: &str, v: &Option<Option<String>>) -> std::result::Result<(), String> {
    let Some(Some(raw)) = v else {
        return Ok(());
    };
    text(what, raw)?;
    if crate::map::is_jump_none(raw) {
        return Ok(());
    }
    for token in raw.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        match token.strip_prefix('#') {
            Some(hex) => {
                uid_from_hex(hex).ok_or_else(|| format!("{what}: bad hop uid {token:?}"))?;
            }
            None => {
                crate::map::hops::validate_raw(token)
                    .map_err(|e| format!("{what}: hop {token:?}: {e}"))?;
            }
        }
    }
    Ok(())
}

fn validate_host(f: &HostFields) -> std::result::Result<(), String> {
    if let Some(n) = &f.name {
        name_ok("name", n)?;
    }
    if let Some(a) = &f.address {
        name_ok("address", a)?;
        if let Some(p) =
            crate::ssh::destination_problem(a, f.username.as_ref().and_then(|u| u.as_deref()))
        {
            return Err(p);
        }
    } else if let Some(Some(u)) = &f.username {
        store::reject_option_like("username", u).map_err(|e| e.to_string())?;
        if let Some(p) = crate::ssh::destination_problem("localhost", Some(u)) {
            return Err(p);
        }
    }
    port_ok("port", &f.port)?;
    validate_jump("proxy_jump", &f.proxy_jump)?;
    for (what, v) in [
        ("label", &f.label),
        ("remote_command", &f.remote_command),
        ("environment", &f.environment),
        ("os_icon", &f.os_icon),
        ("notes", &f.notes),
        ("secret_lookup", &f.secret_lookup),
    ] {
        opt_text(what, v)?;
    }
    if let Some(Some(t)) = &f.transport {
        if t != "ssh" && t != "mosh" {
            return Err(format!("unknown transport {t:?}"));
        }
    }
    if let Some(tags) = &f.tags {
        if tags.len() > MAX_LIST {
            return Err("too many tags".into());
        }
        for t in tags {
            text("tag", t)?;
        }
    }
    if let Some(groups) = &f.group_uids {
        if groups.len() > MAX_LIST {
            return Err("too many groups".into());
        }
        if groups.windows(2).any(|w| w[0] >= w[1]) {
            return Err("group_uids must be sorted and unique".into());
        }
        if groups.contains(&FAVORITES_GROUP_UID) {
            return Err("Favorites is the favorite flag, not a group uid".into());
        }
    }
    Ok(())
}

fn validate_group(f: &GroupFields) -> std::result::Result<(), String> {
    if let Some(n) = &f.name {
        name_ok("group name", n)?;
    }
    opt_text("default_username", &f.default_username)?;
    port_ok("default_port", &f.default_port)?;
    validate_jump("default_proxy_jump", &f.default_proxy_jump)?;
    if let Some(Some(t)) = &f.default_transport {
        if t != "ssh" && t != "mosh" {
            return Err(format!("unknown transport {t:?}"));
        }
    }
    if matches!(f.parent_uid, Some(Some(p)) if p == FAVORITES_GROUP_UID) {
        return Err("Favorites cannot be a parent".into());
    }
    Ok(())
}

fn validate_identity(f: &IdentityFields) -> std::result::Result<(), String> {
    if let Some(n) = &f.name {
        name_ok("identity name", n)?;
    }
    opt_text("username", &f.username)
}

// ----------------------------------------------------------- effective view

/// What the log says an entity looks like right now.
#[derive(Debug, Default, Clone)]
pub struct View {
    pub alive: bool,
    pub fields: BTreeMap<String, Value>,
    /// `(lamport, device)` of the write that set the name: the tie-break when
    /// two entities want the same name.
    pub name_key: (u64, String),
    pub max_write: u64,
}

impl View {
    fn present(&self, k: &str) -> Option<&Value> {
        self.fields.get(k)
    }
    fn opt_str(&self, k: &str) -> Option<Option<String>> {
        self.present(k).map(|v| v.as_str().map(String::from))
    }
    fn opt_bool(&self, k: &str) -> Option<Option<bool>> {
        self.present(k).map(Value::as_bool)
    }
    fn opt_port(&self, k: &str) -> Option<Option<u16>> {
        self.present(k)
            .map(|v| v.as_u64().and_then(|n| u16::try_from(n).ok()))
    }
    fn opt_uid(&self, k: &str) -> Option<Option<Uid>> {
        self.present(k).map(|v| v.as_str().and_then(uid_from_hex))
    }
    pub fn base_name(&self) -> Option<&str> {
        self.fields.get("name").and_then(Value::as_str)
    }
}

pub fn effective_view(conn: &Connection, kind: EntityKind, uid: &Uid) -> Result<View> {
    let mut view = View::default();
    let mut stmt = conn.prepare_cached(
        "SELECT f.field, f.value, f.lamport, f.device_id
         FROM sync_fields f LEFT JOIN sync_ops o ON o.op_hash = f.op_hash
         WHERE f.kind = ?1 AND f.uid = ?2
           AND (o.state IN ('applied','approved') OR f.op_hash = zeroblob(32))
         ORDER BY f.lamport, f.device_id, f.seq",
    )?;
    let rows = stmt.query_map(params![kind.as_str(), &uid[..]], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)? as u64,
            r.get::<_, String>(3)?,
        ))
    })?;
    let mut any = false;
    for row in rows {
        let (field, value, lamport, device) = row?;
        any = true;
        view.max_write = view.max_write.max(lamport);
        if field == "_" {
            continue;
        }
        if field == "name" {
            view.name_key = (lamport, device);
        }
        view.fields.insert(field, serde_json::from_str(&value)?);
    }
    let tomb: Option<i64> = conn.query_row(
        "SELECT MAX(t.lamport) FROM sync_tombstones t LEFT JOIN sync_ops o ON o.op_hash = t.op_hash
         WHERE t.kind = ?1 AND t.uid = ?2
           AND (o.state IN ('applied','approved') OR t.op_hash = zeroblob(32))",
        params![kind.as_str(), &uid[..]],
        |r| r.get(0),
    )?;
    view.alive = any && tomb.is_none_or(|t| view.max_write > t as u64);
    Ok(view)
}

/// Recompute the cache row of one entity; returns the previous cached base
/// name (if the entity was alive) and the new view.
fn refresh_cache(conn: &Connection, kind: EntityKind, uid: &Uid) -> Result<(Option<String>, View)> {
    let old: Option<String> = conn
        .query_row(
            "SELECT name FROM sync_entities WHERE kind = ?1 AND uid = ?2 AND alive = 1",
            params![kind.as_str(), &uid[..]],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let view = effective_view(conn, kind, uid)?;
    conn.execute(
        "INSERT INTO sync_entities (kind, uid, alive, name, name_lamport, name_device)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(kind, uid) DO UPDATE SET alive = excluded.alive, name = excluded.name,
             name_lamport = excluded.name_lamport, name_device = excluded.name_device",
        params![
            kind.as_str(),
            &uid[..],
            i64::from(view.alive),
            view.base_name(),
            view.name_key.0 as i64,
            view.name_key.1,
        ],
    )?;
    Ok((old, view))
}

// ---------------------------------------------------------------- reproject

/// Bring the SQL rows in line with the log for `touched` and everything that
/// depends on it (name peers, referrers). Call inside the ingest transaction.
pub fn reproject(conn: &Connection, touched: &Touched) -> Result<()> {
    if touched.is_empty() {
        return Ok(());
    }
    let mut views: BTreeMap<(EntityKind, Uid), View> = BTreeMap::new();
    let mut work: VecDeque<(EntityKind, Uid)> = touched.iter().copied().collect();
    let mut names: BTreeSet<(EntityKind, String)> = BTreeSet::new();
    let mut named: BTreeSet<(EntityKind, String)> = BTreeSet::new();
    let mut seen: BTreeSet<(EntityKind, Uid)> = BTreeSet::new();

    // Refresh caches and collect name peers and referrers to a fixpoint: a
    // peer pulled in by a shared name has referrers of its own (a hop that
    // renders its name), and so on.
    loop {
        while let Some((kind, uid)) = work.pop_front() {
            if !seen.insert((kind, uid)) {
                continue;
            }
            let (old, view) = refresh_cache(conn, kind, &uid)?;
            if let Some(n) = old {
                names.insert((kind, n));
            }
            if let (true, Some(n)) = (view.alive, view.base_name()) {
                names.insert((kind, n.to_string()));
            }
            views.insert((kind, uid), view);
            for referrer in referrers(conn, &uid)? {
                work.push_back(referrer);
            }
        }
        // Name peers: every live entity sharing a base name with a touched one.
        for (kind, name) in names.difference(&named).cloned().collect::<Vec<_>>() {
            named.insert((kind, name.clone()));
            let mut stmt = conn.prepare_cached(
                "SELECT uid FROM sync_entities WHERE kind = ?1 AND alive = 1 AND name = ?2",
            )?;
            let peers: Vec<Vec<u8>> = stmt
                .query_map(params![kind.as_str(), name], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?;
            for p in peers {
                if let Ok(uid) = <Uid>::try_from(p) {
                    work.push_back((kind, uid));
                }
            }
        }
        if work.is_empty() {
            break;
        }
    }

    // Deletions first: a dead row may still hold a name a live one now wants.
    for kind in [EntityKind::Host, EntityKind::Group, EntityKind::Identity] {
        for ((k, uid), view) in views
            .iter()
            .filter(|((k, _), v)| *k == kind && (!v.alive || !is_complete(kind, v)))
        {
            project_row(conn, *k, uid, view)
                .with_context(|| format!("delete {} {}", k.as_str(), uid_hex(uid)))?;
        }
    }

    // Free the names about to change, so swaps cannot trip UNIQUE(name).
    for ((kind, uid), view) in &views {
        if !view.alive {
            continue;
        }
        let want = rendered_name(conn, *kind, uid, view)?;
        if let Some(want) = want {
            let table = table_of(*kind);
            let cur: Option<(i64, String)> = conn
                .query_row(
                    &format!("SELECT id, name FROM {table} WHERE uid = ?1"),
                    params![&uid[..]],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((id, name)) = cur {
                if name != want {
                    conn.execute(
                        &format!("UPDATE {table} SET name = ?1 WHERE id = ?2"),
                        params![format!("\u{1}sync:{id}"), id],
                    )
                    .with_context(|| format!("park {} {}", kind.as_str(), uid_hex(uid)))?;
                }
            }
        }
    }

    for kind in [EntityKind::Identity, EntityKind::Group, EntityKind::Host] {
        for ((_, uid), view) in views.iter().filter(|((k, _), v)| *k == kind && v.alive) {
            project_row(conn, kind, uid, view)
                .with_context(|| format!("project {} {}", kind.as_str(), uid_hex(uid)))?;
        }
        // Groups reference groups (parent) and each other's rows: a second
        // sweep resolves references to rows created later in the first.
        if kind == EntityKind::Group {
            for ((_, uid), view) in views.iter().filter(|((k, _), v)| *k == kind && v.alive) {
                project_row(conn, kind, uid, view)?;
            }
        }
    }
    // Hop rendering needs every host name final.
    for ((kind, uid), view) in &views {
        if view.alive && matches!(kind, EntityKind::Host | EntityKind::Group) {
            project_jump(conn, *kind, uid, view)?;
        }
    }
    Ok(())
}

fn table_of(kind: EntityKind) -> &'static str {
    match kind {
        EntityKind::Host => "hosts",
        EntityKind::Group => "host_groups",
        EntityKind::Identity => "identities",
    }
}

/// Entities whose registers mention `uid`: host and group references, hop
/// tokens. A superset is fine; reprojecting an unrelated entity is a no-op.
fn referrers(conn: &Connection, uid: &Uid) -> Result<Vec<(EntityKind, Uid)>> {
    let hex = uid_hex(uid);
    let mut stmt = conn.prepare_cached(
        "SELECT DISTINCT kind, uid FROM sync_fields
         WHERE field IN ('identity_uid','group_uids','default_identity_uid','parent_uid',
                         'proxy_jump','default_proxy_jump')
           AND instr(value, ?1) > 0",
    )?;
    let rows = stmt.query_map(params![hex], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (kind, uid) = row?;
        if let (Some(kind), Ok(uid)) = (EntityKind::parse(&kind), <Uid>::try_from(uid)) {
            out.push((kind, uid));
        }
    }
    Ok(out)
}

/// The name this entity should carry locally: its base name when it wins the
/// name, `base (uid8)` when another entity (or a local-only row) holds it.
fn rendered_name(
    conn: &Connection,
    kind: EntityKind,
    uid: &Uid,
    view: &View,
) -> Result<Option<String>> {
    let Some(base) = view.base_name() else {
        return Ok(None);
    };
    let winner: Option<Vec<u8>> = conn
        .query_row(
            "SELECT uid FROM sync_entities WHERE kind = ?1 AND alive = 1 AND name = ?2
             ORDER BY name_lamport, name_device, uid LIMIT 1",
            params![kind.as_str(), base],
            |r| r.get(0),
        )
        .optional()?;
    let table = table_of(kind);
    let occupied: bool = conn.query_row(
        &format!(
            "SELECT EXISTS(SELECT 1 FROM {table} t WHERE t.name = ?1 AND t.uid != ?2
                   AND t.uid NOT IN (SELECT uid FROM sync_entities WHERE kind = ?3))"
        ),
        params![base, &uid[..], kind.as_str()],
        |r| r.get(0),
    )?;
    if winner.as_deref() == Some(&uid[..]) && !occupied {
        Ok(Some(base.to_string()))
    } else {
        Ok(Some(format!("{base} ({})", &uid_hex(uid)[..8])))
    }
}

fn local_id(conn: &Connection, kind: EntityKind, uid: &Uid) -> Result<Option<i64>> {
    match kind {
        EntityKind::Host => store::host_id_by_uid_on(conn, uid),
        EntityKind::Group => store::group_id_by_uid_on(conn, uid),
        EntityKind::Identity => store::identity_id_by_uid_on(conn, uid),
    }
}

/// Does the effective view hold what a row needs (a name, and for a host an
/// address)? Incomplete entities have no sync-made row.
fn is_complete(kind: EntityKind, view: &View) -> bool {
    view.base_name().is_some()
        && (kind != EntityKind::Host || matches!(view.opt_str("address"), Some(Some(_))))
}

fn created_by_sync(conn: &Connection, kind: EntityKind, uid: &Uid) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT created FROM sync_entities WHERE kind = ?1 AND uid = ?2",
            params![kind.as_str(), &uid[..]],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(false))
}

fn mark_created(conn: &Connection, kind: EntityKind, uid: &Uid, v: bool) -> Result<()> {
    conn.execute(
        "UPDATE sync_entities SET created = ?3 WHERE kind = ?1 AND uid = ?2",
        params![kind.as_str(), &uid[..], v],
    )?;
    Ok(())
}

fn set_uid(conn: &Connection, kind: EntityKind, id: i64, uid: &Uid) -> Result<()> {
    conn.execute(
        &format!("UPDATE {} SET uid = ?1 WHERE id = ?2", table_of(kind)),
        params![&uid[..], id],
    )?;
    mark_created(conn, kind, uid, true)?;
    Ok(())
}

fn project_row(conn: &Connection, kind: EntityKind, uid: &Uid, view: &View) -> Result<()> {
    if kind == EntityKind::Group && *uid == FAVORITES_GROUP_UID {
        return Ok(());
    }
    let existing = local_id(conn, kind, uid)?;
    if !view.alive {
        if let Some(id) = existing {
            match kind {
                EntityKind::Host => {
                    let source: Option<String> = conn
                        .query_row("SELECT source FROM hosts WHERE id = ?1", params![id], |r| {
                            r.get(0)
                        })
                        .optional()?;
                    if source.as_deref() == Some(HostSource::Launcher.as_str()) {
                        store::delete_host_on(conn, id)?;
                    }
                }
                EntityKind::Group => {
                    store::delete_group_on(conn, id)?;
                }
                EntityKind::Identity => {
                    store::delete_identity_on(conn, id)?;
                }
            }
        }
        return Ok(());
    }
    let name = rendered_name(conn, kind, uid, view)?;
    let Some(name) = name.filter(|_| is_complete(kind, view)) else {
        // An entity whose effective view lacks its required fields (an op that
        // set them became pending again) has no row: remove the one the
        // projection made, never one the user already had.
        if let Some(id) = existing {
            if created_by_sync(conn, kind, uid)? {
                match kind {
                    EntityKind::Host => store::delete_host_on(conn, id)?,
                    EntityKind::Group => store::delete_group_on(conn, id)?,
                    EntityKind::Identity => store::delete_identity_on(conn, id)?,
                };
                mark_created(conn, kind, uid, false)?;
            }
        }
        return Ok(());
    };
    match kind {
        EntityKind::Identity => project_identity(conn, uid, view, existing, &name),
        EntityKind::Group => project_group(conn, uid, view, existing, &name),
        EntityKind::Host => project_host(conn, uid, view, existing, &name),
    }
}

fn project_identity(
    conn: &Connection,
    uid: &Uid,
    view: &View,
    existing: Option<i64>,
    name: &str,
) -> Result<()> {
    let id = match existing {
        Some(id) => id,
        None => {
            let created = store::create_identity_on(
                conn,
                &NewIdentity {
                    name: name.to_string(),
                    ..Default::default()
                },
            )?;
            set_uid(conn, EntityKind::Identity, created.id, uid)?;
            created.id
        }
    };
    let mut upd = store::IdentityUpdate {
        name: Some(name.to_string()),
        ..Default::default()
    };
    if let Some(u) = view.opt_str("username") {
        upd.username = Some(u);
    }
    if let Some(Some(p)) = view.opt_bool("has_password") {
        upd.has_password = Some(p);
    }
    store::update_identity_on(conn, id, &upd)?;
    Ok(())
}

fn resolve_group(conn: &Connection, uid: Option<Uid>) -> Result<Option<i64>> {
    match uid {
        Some(u) => store::group_id_by_uid_on(conn, &u),
        None => Ok(None),
    }
}

fn resolve_identity(conn: &Connection, uid: Option<Uid>) -> Result<Option<i64>> {
    match uid {
        Some(u) => store::identity_id_by_uid_on(conn, &u),
        None => Ok(None),
    }
}

/// Does following `parent_uid` links from `start` come back to `start`?
/// Decided on effective views, so every device agrees which groups are in a
/// cycle (they all lose their parent).
fn parent_cycles(conn: &Connection, start: &Uid) -> Result<bool> {
    let mut cur = *start;
    for _ in 0..4096 {
        let v = effective_view(conn, EntityKind::Group, &cur)?;
        if !v.alive {
            return Ok(false);
        }
        match v.opt_uid("parent_uid").flatten() {
            Some(p) if p == *start => return Ok(true),
            Some(p) => cur = p,
            None => return Ok(false),
        }
    }
    Ok(true)
}

fn project_group(
    conn: &Connection,
    uid: &Uid,
    view: &View,
    existing: Option<i64>,
    name: &str,
) -> Result<()> {
    let id = match existing {
        Some(id) => id,
        None => {
            let created = store::create_group_on(
                conn,
                &NewHostGroup {
                    name: name.to_string(),
                    ..Default::default()
                },
            )?;
            set_uid(conn, EntityKind::Group, created.id, uid)?;
            created.id
        }
    };
    let mut upd = store::HostGroupUpdate {
        name: Some(name.to_string()),
        ..Default::default()
    };
    if let Some(u) = view.opt_uid("default_identity_uid") {
        upd.default_identity_id = Some(resolve_identity(conn, u)?);
    }
    if let Some(v) = view.opt_str("default_username") {
        upd.default_username = Some(v);
    }
    if let Some(v) = view.opt_port("default_port") {
        upd.default_port = Some(v);
    }
    if let Some(v) = view.opt_str("default_transport") {
        upd.default_transport = Some(v.map(|t| SessionTransport::from_db(Some(&t))));
    }
    if let Some(v) = view.opt_bool("default_forward_agent") {
        upd.default_forward_agent = Some(v);
    }
    if let Some(p) = view.opt_uid("parent_uid") {
        let parent = if p.is_some() && parent_cycles(conn, uid)? {
            None
        } else {
            resolve_group(conn, p)?
        };
        upd.parent_id = Some(parent);
    }
    store::update_group_on(conn, id, &upd)?;
    Ok(())
}

fn project_host(
    conn: &Connection,
    uid: &Uid,
    view: &View,
    existing: Option<i64>,
    name: &str,
) -> Result<()> {
    let address = match (view.opt_str("address"), existing) {
        (Some(Some(a)), _) => a,
        (_, Some(_)) => String::new(),
        _ => return Ok(()), // cannot create a host without an address
    };
    let username = view.opt_str("username").flatten();
    if !address.is_empty() {
        if let Some(problem) = crate::ssh::destination_problem(&address, username.as_deref()) {
            let _ = problem; // an invalid destination never reaches a row
            return Ok(());
        }
    }
    let id = match existing {
        Some(id) => {
            let source: Option<String> = conn
                .query_row("SELECT source FROM hosts WHERE id = ?1", params![id], |r| {
                    r.get(0)
                })
                .optional()?;
            if source.as_deref() != Some(HostSource::Launcher.as_str()) {
                return Ok(());
            }
            id
        }
        None => {
            let mut new = NewHost::launcher(name, &address);
            new.source = HostSource::Launcher;
            let created = store::create_host_on(conn, &new)?;
            set_uid(conn, EntityKind::Host, created.id, uid)?;
            created.id
        }
    };
    let mut upd = HostUpdate {
        name: Some(name.to_string()),
        ..Default::default()
    };
    if !address.is_empty() {
        upd.address = Some(address);
    }
    upd.label = view.opt_str("label");
    upd.port = view.opt_port("port");
    upd.username = view.opt_str("username");
    upd.forward_agent = view.opt_bool("forward_agent");
    upd.remote_command = view.opt_str("remote_command");
    upd.environment = view.opt_str("environment");
    upd.os_icon = view.opt_str("os_icon");
    upd.notes = view.opt_str("notes");
    upd.secret_lookup = view.opt_str("secret_lookup");
    upd.transport = view
        .opt_str("transport")
        .map(|t| t.map(|t| SessionTransport::from_db(Some(&t))));
    if let Some(Value::Array(tags)) = view.present("tags") {
        upd.tags = Some(
            tags.iter()
                .filter_map(|t| t.as_str().map(String::from))
                .collect(),
        );
    }
    if let Some(Some(f)) = view.opt_bool("favorite") {
        upd.favorite = Some(f);
    }
    if let Some(Some(p)) = view.opt_bool("has_password") {
        upd.has_password = Some(p);
    }
    if let Some(u) = view.opt_uid("identity_uid") {
        upd.identity_id = Some(resolve_identity(conn, u)?);
    }
    store::update_host_on(conn, id, &upd)?;

    if let Some(Value::Array(groups)) = view.present("group_uids") {
        conn.execute(
            "DELETE FROM host_group_memberships WHERE host_id = ?1
               AND group_id IN (SELECT id FROM host_groups WHERE reserved = 0)",
            params![id],
        )?;
        for g in groups {
            if let Some(gid) = g
                .as_str()
                .and_then(uid_from_hex)
                .map(|g: GroupUid| store::group_id_by_uid_on(conn, &g))
                .transpose()?
                .flatten()
            {
                conn.execute(
                    "INSERT OR IGNORE INTO host_group_memberships (host_id, group_id) VALUES (?1, ?2)",
                    params![id, gid],
                )?;
            }
        }
        conn.execute(
            "UPDATE hosts SET group_id = (
                 SELECT m.group_id FROM host_group_memberships m
                   JOIN host_groups g ON g.id = m.group_id
                  WHERE m.host_id = hosts.id AND g.reserved = 0
                  ORDER BY m.group_id LIMIT 1)
             WHERE id = ?1",
            params![id],
        )?;
    }
    Ok(())
}

// ---------------------------------------------------------------- hop render

/// Render a wire ProxyJump: `#uid` tokens become the local name of that host,
/// plain tokens pass through. `None` when a token cannot be rendered (unknown
/// or deleted host, name that is not a valid hop): the column is then cleared.
pub fn render_hops(conn: &Connection, wire: &str) -> Result<Option<String>> {
    if crate::map::is_jump_none(wire) {
        return Ok(Some(wire.trim().to_string()));
    }
    let mut out = Vec::new();
    for token in wire.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        let rendered = match token.strip_prefix('#') {
            Some(hex) => {
                let Some(uid) = uid_from_hex(hex) else {
                    return Ok(None);
                };
                let name: Option<String> = conn
                    .query_row(
                        "SELECT name FROM hosts WHERE uid = ?1 AND source = 'launcher'",
                        params![&uid[..]],
                        |r| r.get(0),
                    )
                    .optional()?;
                match name {
                    Some(n) if crate::map::hops::token_issue(&n).is_none() => n,
                    _ => return Ok(None),
                }
            }
            None => match crate::map::hops::validate_raw(token) {
                Ok(_) => token.to_string(),
                Err(_) => return Ok(None),
            },
        };
        out.push(rendered);
    }
    Ok(Some(crate::map::hops::join(&out)))
}

/// The inverse for authoring: hop tokens that name a saved launcher host
/// become `#uid`. Everything else stays as typed.
pub fn hops_to_wire(conn: &Connection, local: &str) -> Result<String> {
    if crate::map::is_jump_none(local) {
        return Ok(local.trim().to_string());
    }
    let mut out = Vec::new();
    for token in local.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        let uid: Option<Vec<u8>> = conn
            .query_row(
                "SELECT uid FROM hosts WHERE name = ?1 AND source = 'launcher'",
                params![token],
                |r| r.get(0),
            )
            .optional()?;
        out.push(match uid.and_then(|u| <Uid>::try_from(u).ok()) {
            Some(u) => format!("#{}", uid_hex(&u)),
            None => token.to_string(),
        });
    }
    Ok(crate::map::hops::join(&out))
}

fn project_jump(conn: &Connection, kind: EntityKind, uid: &Uid, view: &View) -> Result<()> {
    let (field, table, col) = match kind {
        EntityKind::Host => ("proxy_jump", "hosts", "proxy_jump"),
        EntityKind::Group => ("default_proxy_jump", "host_groups", "default_proxy_jump"),
        EntityKind::Identity => return Ok(()),
    };
    let Some(v) = view.opt_str(field) else {
        return Ok(());
    };
    let rendered = match v {
        Some(wire) => render_hops(conn, &wire)?,
        None => None,
    };
    let source_ok = kind != EntityKind::Host
        || conn
            .query_row(
                "SELECT source FROM hosts WHERE uid = ?1",
                params![&uid[..]],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .as_deref()
            == Some("launcher");
    if source_ok {
        conn.execute(
            &format!("UPDATE {table} SET {col} = ?1 WHERE uid = ?2"),
            params![rendered, &uid[..]],
        )?;
    }
    Ok(())
}

// ------------------------------------------------------------------- hashes

/// Hash of the logical replicated state: every live entity with its effective
/// registers, in uid order. Equal on two stores iff they hold the same
/// effective op set (up to the per-field order the registers resolve).
pub fn state_hash(conn: &Connection) -> Result<[u8; 32]> {
    let mut out = Vec::new();
    let mut stmt =
        conn.prepare("SELECT kind, uid FROM sync_entities WHERE alive = 1 ORDER BY kind, uid")?;
    let rows: Vec<(String, Vec<u8>)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    for (kind, uid) in rows {
        let (Some(k), Ok(u)) = (EntityKind::parse(&kind), <Uid>::try_from(uid)) else {
            continue;
        };
        let view = effective_view(conn, k, &u)?;
        out.extend_from_slice(
            format!(
                "{kind} {} {}\n",
                uid_hex(&u),
                serde_json::to_string(&view.fields)?
            )
            .as_bytes(),
        );
    }
    Ok(crate::sync::crypto::chain_hash(&out))
}

/// Hash of the projected rows (replicated columns only, keyed by uid) of every
/// managed entity: what the user actually sees.
pub fn projection_hash(conn: &Connection) -> Result<[u8; 32]> {
    Ok(crate::sync::crypto::chain_hash(
        projection_dump(conn)?.as_bytes(),
    ))
}

/// The text [`projection_hash`] hashes, one line per entity.
pub fn projection_dump(conn: &Connection) -> Result<String> {
    let mut out = String::new();
    let mut hosts = conn.prepare(
        "SELECT h.uid, h.name, h.label, h.address, h.port, h.username, h.proxy_jump,
                h.forward_agent, h.remote_command, h.transport, h.environment, h.os_icon,
                h.tags, h.notes, h.has_password, h.secret_lookup,
                (SELECT i.uid FROM identities i WHERE i.id = h.identity_id),
                (SELECT group_concat(hex(g.uid)) FROM (
                    SELECT g.uid FROM host_group_memberships m JOIN host_groups g ON g.id = m.group_id
                    WHERE m.host_id = h.id AND g.reserved = 0 ORDER BY g.uid) g),
                EXISTS(SELECT 1 FROM host_group_memberships m JOIN host_groups g ON g.id = m.group_id
                       WHERE m.host_id = h.id AND g.reserved = 1)
         FROM hosts h WHERE h.uid IN (SELECT uid FROM sync_entities WHERE kind = 'host' AND alive = 1)
         ORDER BY h.uid",
    )?;
    let mut rows = hosts.query([])?;
    while let Some(r) = rows.next()? {
        out.push('H');
        for i in 0..19 {
            out.push('|');
            out.push_str(&dump(r.get_ref(i)?));
        }
        out.push('\n');
    }
    let mut groups = conn.prepare(
        "SELECT g.uid, g.name, g.default_username, g.default_port, g.default_proxy_jump,
                g.default_transport, g.default_forward_agent,
                (SELECT i.uid FROM identities i WHERE i.id = g.default_identity_id),
                (SELECT p.uid FROM host_groups p WHERE p.id = g.parent_id)
         FROM host_groups g WHERE g.uid IN (SELECT uid FROM sync_entities WHERE kind = 'group' AND alive = 1)
         ORDER BY g.uid",
    )?;
    let mut rows = groups.query([])?;
    while let Some(r) = rows.next()? {
        out.push('G');
        for i in 0..9 {
            out.push('|');
            out.push_str(&dump(r.get_ref(i)?));
        }
        out.push('\n');
    }
    let mut ids = conn.prepare(
        "SELECT uid, name, username, has_password FROM identities
         WHERE uid IN (SELECT uid FROM sync_entities WHERE kind = 'identity' AND alive = 1) ORDER BY uid",
    )?;
    let mut rows = ids.query([])?;
    while let Some(r) = rows.next()? {
        out.push('I');
        for i in 0..4 {
            out.push('|');
            out.push_str(&dump(r.get_ref(i)?));
        }
        out.push('\n');
    }
    Ok(out)
}

fn dump(v: rusqlite::types::ValueRef<'_>) -> String {
    use rusqlite::types::ValueRef::*;
    match v {
        Null => "~".into(),
        Integer(i) => i.to_string(),
        Real(f) => f.to_string(),
        Text(t) => format!("{:?}", String::from_utf8_lossy(t)),
        Blob(b) => uid_hex_any(b),
    }
}

fn uid_hex_any(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
