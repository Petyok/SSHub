//! The op state machine and deterministic merge rules.
//!
//! Every stored op has a state: `applied` (Green, automatic), `pending`
//! (waiting for review), `approved` (review accepted, or the op is ours),
//! `rejected` (review refused; kept so the chain stays whole) or `void` (its
//! author was revoked at or below its lamport). `applied` and `approved` ops are
//! *effective*: only they feed the registers. The state of an op is a function
//! of the op, the review decision and the other ops in the log, never of arrival
//! order, so honest peers holding the same ops (and the same decisions) hold the
//! same effective set and, through [`crate::sync::apply`], the same state.

use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::{BTreeSet, HashSet, VecDeque};

use super::canonical::{body_name, group_field_values, host_field_values, identity_field_values};
use super::op::*;
use super::tier::EntityKind;
use crate::store::{uid_hex, Uid};
use crate::sync::apply::Touched;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum State {
    Applied,
    Pending,
    Approved,
    Rejected,
    Void,
}

impl State {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            State::Applied => "applied",
            State::Pending => "pending",
            State::Approved => "approved",
            State::Rejected => "rejected",
            State::Void => "void",
        }
    }

    pub(crate) fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "applied" => State::Applied,
            "pending" => State::Pending,
            "approved" => State::Approved,
            "rejected" => State::Rejected,
            "void" => State::Void,
            other => return Err(anyhow!("unknown op state {other}")),
        })
    }

    pub(crate) fn effective(self) -> bool {
        matches!(self, State::Applied | State::Approved)
    }
}

pub(crate) struct OpRow {
    pub op: Op,
    pub hash: [u8; 32],
    pub state: State,
    pub reason: Option<String>,
    pub local: bool,
}

/// Entity an op writes to, and whether it deletes it.
pub(crate) fn entity_of(body: &OpBody) -> Option<(EntityKind, Uid, bool)> {
    Some(match body {
        OpBody::HostUpsert { host_uid, .. } => (EntityKind::Host, *host_uid, false),
        OpBody::HostDelete { host_uid } => (EntityKind::Host, *host_uid, true),
        OpBody::GroupUpsert { group_uid, .. } => (EntityKind::Group, *group_uid, false),
        OpBody::GroupDelete { group_uid } => (EntityKind::Group, *group_uid, true),
        OpBody::IdentityUpsert { identity_uid, .. } => (EntityKind::Identity, *identity_uid, false),
        OpBody::IdentityDelete { identity_uid } => (EntityKind::Identity, *identity_uid, true),
        _ => return None,
    })
}

fn subject_of(body: &OpBody) -> Option<String> {
    match entity_of(body) {
        Some((_, uid, _)) => Some(uid_hex(&uid)),
        None => match body {
            OpBody::DeviceRevoke { device_id } => Some(device_id.clone()),
            OpBody::CredUpdate { host_uid, .. } => Some(uid_hex(host_uid)),
            _ => None,
        },
    }
}

fn row_of(conn: &Connection, where_sql: &str, p: impl rusqlite::Params) -> Result<Vec<OpRow>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT op_hash, canonical, signature, state, reason, local FROM sync_ops WHERE {where_sql}"
    ))?;
    let rows = stmt.query_map(p, |r| {
        Ok((
            r.get::<_, Vec<u8>>(0)?,
            r.get::<_, Vec<u8>>(1)?,
            r.get::<_, Vec<u8>>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, bool>(5)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (hash, canonical, sig, state, reason, local) = row?;
        let sig: [u8; 64] = sig
            .try_into()
            .map_err(|_| anyhow!("stored signature length"))?;
        out.push(OpRow {
            op: Op::from_canonical(&canonical, sig)?,
            hash: hash.try_into().map_err(|_| anyhow!("stored hash length"))?,
            state: State::parse(&state)?,
            reason,
            local,
        });
    }
    Ok(out)
}

pub(crate) fn load_op(conn: &Connection, hash: &[u8; 32]) -> Result<Option<OpRow>> {
    Ok(row_of(conn, "op_hash = ?1", params![&hash[..]])?.pop())
}

pub(crate) fn op_at(conn: &Connection, device: &str, seq: u64) -> Result<Option<OpRow>> {
    Ok(row_of(
        conn,
        "device_id = ?1 AND seq = ?2",
        params![device, seq as i64],
    )?
    .pop())
}

/// Store a verified op, its register writes and its tombstone. The state is
/// decided by the caller.
pub(crate) fn insert_op(
    conn: &Connection,
    op: &Op,
    state: State,
    reason: Option<&str>,
    local: bool,
    now: i64,
) -> Result<[u8; 32]> {
    let canonical = op.canonical_bytes();
    let hash = crate::sync::crypto::chain_hash(&canonical);
    conn.execute(
        "INSERT INTO sync_ops (op_hash, device_id, seq, lamport, prev_hash, epoch, canonical,
             signature, kind, subject, state, reason, local, received_at, timestamp)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            &hash[..],
            op.device_id,
            op.seq as i64,
            op.lamport as i64,
            &op.prev_hash[..],
            op.epoch as i64,
            canonical,
            &op.signature[..],
            body_name(&op.body),
            subject_of(&op.body),
            state.as_str(),
            reason,
            local,
            now,
            op.timestamp,
        ],
    )?;
    conn.execute(
        "INSERT INTO sync_chain_heads (device_id, seq, op_hash, lamport) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(device_id) DO UPDATE SET seq = excluded.seq, op_hash = excluded.op_hash,
             lamport = excluded.lamport",
        params![op.device_id, op.seq as i64, &hash[..], op.lamport as i64],
    )?;
    conn.execute(
        "INSERT INTO sync_epochs (device_id, epoch) VALUES (?1, ?2)
         ON CONFLICT(device_id) DO UPDATE SET epoch = MAX(epoch, excluded.epoch)",
        params![op.device_id, op.epoch as i64],
    )?;
    if let Some((kind, uid, is_delete)) = entity_of(&op.body) {
        if is_delete {
            conn.execute(
                "INSERT OR IGNORE INTO sync_tombstones (kind, uid, lamport, device_id, op_hash, ts)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    kind.as_str(),
                    &uid[..],
                    op.lamport as i64,
                    op.device_id,
                    &hash[..],
                    op.timestamp
                ],
            )?;
        } else {
            let mut fields = match &op.body {
                OpBody::HostUpsert { fields, .. } => host_field_values(fields),
                OpBody::GroupUpsert { fields, .. } => group_field_values(fields),
                OpBody::IdentityUpsert { fields, .. } => identity_field_values(fields),
                _ => Vec::new(),
            };
            // Liveness marker: an upsert with an empty patch still keeps it alive.
            fields.push(("_", serde_json::Value::Null));
            for (name, value) in fields {
                conn.execute(
                    "INSERT OR IGNORE INTO sync_fields (kind, uid, field, op_hash, lamport, device_id, seq, value)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        kind.as_str(),
                        &uid[..],
                        name,
                        &hash[..],
                        op.lamport as i64,
                        op.device_id,
                        op.seq as i64,
                        serde_json::to_string(&value)?,
                    ],
                )?;
            }
        }
    }
    super::checkpoint::record_writes(conn, op, &hash)?;
    sync_pending_row(conn, &hash, op, state, reason, now)?;
    Ok(hash)
}

fn sync_pending_row(
    conn: &Connection,
    hash: &[u8; 32],
    op: &Op,
    state: State,
    reason: Option<&str>,
    now: i64,
) -> Result<()> {
    if state == State::Pending {
        let (kind, uid) = match entity_of(&op.body) {
            Some((k, u, _)) => (Some(k.as_str()), Some(u.to_vec())),
            None => (None, None),
        };
        conn.execute(
            "INSERT OR REPLACE INTO sync_pending_ops (op_hash, device_id, entity_kind, entity_uid, reason, received_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![&hash[..], op.device_id, kind, uid, reason.unwrap_or("tier"), now],
        )?;
    } else {
        conn.execute(
            "DELETE FROM sync_pending_ops WHERE op_hash = ?1",
            params![&hash[..]],
        )?;
    }
    Ok(())
}

fn set_state(
    conn: &Connection,
    row: &OpRow,
    state: State,
    reason: Option<&str>,
    now: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE sync_ops SET state = ?1, reason = ?2 WHERE op_hash = ?3",
        params![state.as_str(), reason, &row.hash[..]],
    )?;
    sync_pending_row(conn, &row.hash, &row.op, state, reason, now)
}

// ------------------------------------------------------------ state rules

fn revoke_exists(conn: &Connection, op: &Op, hash: &[u8; 32], state: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sync_ops WHERE kind = 'DeviceRevoke' AND subject = ?1
             AND state = ?2 AND lamport <= ?3 AND op_hash != ?4)",
        params![op.device_id, state, op.lamport as i64, &hash[..]],
        |r| r.get(0),
    )?)
}

fn resurrects(conn: &Connection, op: &Op) -> Result<bool> {
    let Some((kind, uid, false)) = entity_of(&op.body) else {
        return Ok(false);
    };
    let tomb: Option<i64> = conn.query_row(
        "SELECT MAX(t.lamport) FROM sync_tombstones t LEFT JOIN sync_ops o ON o.op_hash = t.op_hash
         WHERE t.kind = ?1 AND t.uid = ?2
           AND (o.state IN ('applied','approved') OR t.op_hash = zeroblob(32))",
        params![kind.as_str(), &uid[..]],
        |r| r.get(0),
    )?;
    Ok(tomb.is_some_and(|t| op.lamport > t as u64))
}

/// The tier of an op in this store: a revoke is Yellow when the target is a
/// mirror, Red when it is a holder or unknown.
pub(crate) fn effective_tier(conn: &Connection, op: &Op) -> Result<OpTier> {
    if let OpBody::DeviceRevoke { device_id } = &op.body {
        let role: Option<String> = conn
            .query_row(
                "SELECT role FROM sync_devices WHERE device_id = ?1",
                params![device_id],
                |r| r.get(0),
            )
            .optional()?;
        if role.as_deref() == Some("mirror") {
            return Ok(OpTier::Yellow);
        }
    }
    Ok(op.tier())
}

/// The state an op should be in given everything else in the log.
pub(crate) fn desired_state(
    conn: &Connection,
    row: &OpRow,
) -> Result<(State, Option<&'static str>)> {
    let op = &row.op;
    if row.state == State::Rejected {
        return Ok((State::Rejected, None));
    }
    if revoke_exists(conn, op, &row.hash, "approved")? {
        return Ok((State::Void, None));
    }
    if row.state == State::Approved || row.local {
        return Ok((State::Approved, None));
    }
    if effective_tier(conn, op)? != OpTier::Green {
        return Ok((State::Pending, Some("tier")));
    }
    if super::checkpoint::stale_above_cut(conn, op)? {
        // A device that was stale at an accepted checkpoint: its later ops wait
        // for a human whatever their fields (a safe write to a forgotten host
        // would resurrect it).
        return Ok((State::Pending, Some("stale")));
    }
    if revoke_exists(conn, op, &row.hash, "pending")? {
        return Ok((State::Pending, Some("revoke-conflict")));
    }
    if resurrects(conn, op)? {
        return Ok((State::Pending, Some("resurrection")));
    }
    Ok((State::Applied, None))
}

// ----------------------------------------------------------- transitions

#[derive(Hash, PartialEq, Eq, Clone)]
enum Work {
    Author(String),
    Entity(EntityKind, Uid),
}

/// Applies state transitions to a fixpoint and records what to reproject.
pub(crate) struct Settler<'a> {
    conn: &'a Connection,
    now: i64,
    pub touched: Touched,
    queue: VecDeque<Work>,
    done: HashSet<Work>,
}

impl<'a> Settler<'a> {
    pub(crate) fn new(conn: &'a Connection, now: i64) -> Self {
        Self {
            conn,
            now,
            touched: BTreeSet::new(),
            queue: VecDeque::new(),
            done: HashSet::new(),
        }
    }

    /// Move one op to `state` (the caller has decided), run its effects and
    /// queue everything its change can affect.
    pub(crate) fn transition(
        &mut self,
        row: &OpRow,
        state: State,
        reason: Option<&str>,
    ) -> Result<()> {
        if row.state == state && row.reason.as_deref() == reason {
            return Ok(());
        }
        set_state(self.conn, row, state, reason, self.now)?;
        if let Some((kind, uid, is_delete)) = entity_of(&row.op.body) {
            self.touched.insert((kind, uid));
            if is_delete && row.state.effective() != state.effective() {
                // Tombstone effectiveness changed: ops that resurrect re-settle.
                self.queue.push_back(Work::Entity(kind, uid));
            }
        }
        if let OpBody::DeviceRevoke { device_id } = &row.op.body {
            if state == State::Approved && !row.state.effective() {
                self.conn.execute(
                    "UPDATE sync_devices SET status = 'revoked' WHERE device_id = ?1 AND status != 'compromised'",
                    params![device_id],
                )?;
            }
            self.queue.push_back(Work::Author(device_id.clone()));
        }
        if state.effective() && !row.state.effective() {
            effects(self.conn, &row.op, &row.hash)?;
        }
        Ok(())
    }

    /// Drain the queue. Each unit of work is re-run only when an earlier
    /// transition re-queued it.
    pub(crate) fn run(&mut self) -> Result<()> {
        let mut guard = 0usize;
        while let Some(work) = self.queue.pop_front() {
            guard += 1;
            anyhow::ensure!(guard < 100_000, "op state settling did not converge");
            let rows = match &work {
                Work::Author(device) => row_of(
                    self.conn,
                    "device_id = ?1 AND state IN ('applied','pending','approved')",
                    params![device],
                )?,
                Work::Entity(kind, uid) => row_of(
                    self.conn,
                    "kind = ?1 AND subject = ?2 AND state IN ('applied','pending')
                     AND (reason IS NULL OR reason != 'tier')",
                    params![upsert_name(*kind), uid_hex(uid)],
                )?,
            };
            for row in rows {
                let (state, reason) = desired_state(self.conn, &row)?;
                self.transition(&row, state, reason)?;
            }
            self.done.insert(work);
        }
        Ok(())
    }
}

fn upsert_name(kind: EntityKind) -> &'static str {
    match kind {
        EntityKind::Host => "HostUpsert",
        EntityKind::Group => "GroupUpsert",
        EntityKind::Identity => "IdentityUpsert",
    }
}

// --------------------------------------------------------------- effects

fn lww_key(lamport: u64, device: &str) -> String {
    format!("{lamport:020}:{device}")
}

/// Table effects of ops that are not entity upserts, run once when an op
/// becomes effective. Idempotent. Entity ops are projected by `reproject`.
fn effects(conn: &Connection, op: &Op, hash: &[u8; 32]) -> Result<()> {
    match &op.body {
        OpBody::DeviceAdd {
            device,
            name,
            role,
            attendance,
            kind,
            verified_by,
        } => {
            if device.validate().is_ok()
                && device.device_id() == device_id_of_pub(&device.ed25519_pub)
            {
                conn.execute(
                    "INSERT OR IGNORE INTO sync_devices
                       (device_id, ed25519, x25519, mlkem, name, role, attendance, kind, verified_by, last_seen, added_by)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10)",
                    params![
                        device.device_id(),
                        &device.ed25519_pub[..],
                        &device.x25519_pub[..],
                        device.mlkem_pub,
                        name,
                        role_db(*role),
                        attendance_db(*attendance),
                        kind_db(*kind),
                        verified_db(*verified_by),
                        &hash[..],
                    ],
                )?;
            }
        }
        OpBody::DeviceRoleChange { device_id, to } => {
            conn.execute(
                "UPDATE sync_devices SET role = ?2 WHERE device_id = ?1",
                params![device_id, role_db(*to)],
            )?;
        }
        OpBody::CredUpdate {
            host_uid,
            ciphertext,
            wrapped_shares,
        } => {
            let incoming = lww_key(op.lamport, &op.device_id);
            let current: Option<(i64, String)> = conn
                .query_row(
                    "SELECT lamport, device_id FROM sync_cred_state WHERE host_uid = ?1",
                    params![&host_uid[..]],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if current.is_none_or(|(l, d)| lww_key(l as u64, &d) < incoming) {
                conn.execute(
                    "INSERT OR REPLACE INTO sync_cred_state
                       (host_uid, ciphertext, wrapped_shares, lamport, device_id, op_hash)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        &host_uid[..],
                        ciphertext,
                        shares_json(wrapped_shares)?,
                        op.lamport as i64,
                        op.device_id,
                        &hash[..],
                    ],
                )?;
            }
        }
        OpBody::CertIssue {
            device_id,
            scope,
            expiry,
            max_new_hosts_per_day,
            epoch_lo,
            epoch_hi,
        } => {
            conn.execute(
                "INSERT OR IGNORE INTO sync_delegation_certs
                   (cert_id, device_id, scope, expiry, max_new_hosts_per_day, epoch_lo, epoch_hi, issuer)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    &hash[..],
                    device_id,
                    scope_json(scope),
                    expiry,
                    max_new_hosts_per_day,
                    *epoch_lo as i64,
                    *epoch_hi as i64,
                    op.device_id,
                ],
            )?;
        }
        OpBody::CertRevoke { cert_id } => {
            conn.execute(
                "INSERT OR REPLACE INTO sync_meta (key, value) VALUES (?1, x'01')",
                params![format!("cert-revoked:{}", hex(cert_id))],
            )?;
            conn.execute(
                "UPDATE sync_delegation_certs SET revoked = 1 WHERE cert_id = ?1",
                params![&cert_id[..]],
            )?;
        }
        OpBody::CertHostOpened { cert_id, host_uid } => {
            conn.execute(
                "INSERT OR IGNORE INTO sync_cert_hosts_opened (cert_id, host_uid, opened_at)
                 VALUES (?1, ?2, ?3)",
                params![&cert_id[..], &host_uid[..], op.timestamp],
            )?;
        }
        OpBody::ModeChange { to, .. } => {
            let incoming = lww_key(op.lamport, &op.device_id);
            let cur: Option<Vec<u8>> = conn
                .query_row(
                    "SELECT value FROM sync_meta WHERE key = 'cred_mode_key'",
                    [],
                    |r| r.get(0),
                )
                .optional()?;
            if cur.is_none_or(|c| String::from_utf8_lossy(&c).as_ref() < incoming.as_str()) {
                conn.execute(
                    "INSERT OR REPLACE INTO sync_meta (key, value) VALUES ('cred_mode_key', ?1)",
                    params![incoming.as_bytes()],
                )?;
                conn.execute(
                    "INSERT OR REPLACE INTO sync_meta (key, value) VALUES ('cred_mode', ?1)",
                    params![match to {
                        CredMode::Vault => &b"vault"[..],
                        CredMode::Quorum => &b"quorum"[..],
                    }],
                )?;
            }
        }
        OpBody::WrapKeyRotate {
            device_id,
            wrap_epoch,
            x25519_pub,
            mlkem_pub,
        } if *device_id == op.device_id && *wrap_epoch > 0 => {
            type DeviceKeys = (Vec<u8>, Vec<u8>, Vec<u8>, i64);
            let cur: Option<DeviceKeys> = conn
                .query_row(
                    "SELECT ed25519, x25519, mlkem, wrap_epoch FROM sync_devices WHERE device_id = ?1",
                    params![device_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()?;
            let Some((ed, x, ml, cur_epoch)) = cur else {
                return Ok(());
            };
            let Ok(ed25519_pub) = <[u8; 32]>::try_from(ed) else {
                return Ok(());
            };
            let next = crate::sync::crypto::DevicePublic {
                ed25519_pub,
                x25519_pub: *x25519_pub,
                mlkem_pub: mlkem_pub.clone(),
            };
            if next.validate().is_err() {
                return Ok(());
            }
            // History first: the generation being replaced stays addressable.
            conn.execute(
                "INSERT OR IGNORE INTO sync_wrap_history (device_id, wrap_epoch, x25519, mlkem)
                 VALUES (?1, ?2, ?3, ?4)",
                params![device_id, cur_epoch, x, ml],
            )?;
            conn.execute(
                "INSERT OR IGNORE INTO sync_wrap_history (device_id, wrap_epoch, x25519, mlkem)
                 VALUES (?1, ?2, ?3, ?4)",
                params![device_id, *wrap_epoch as i64, &x25519_pub[..], mlkem_pub],
            )?;
            if *wrap_epoch as i64 > cur_epoch {
                conn.execute(
                    "UPDATE sync_devices SET x25519 = ?2, mlkem = ?3, wrap_epoch = ?4 WHERE device_id = ?1",
                    params![device_id, &x25519_pub[..], mlkem_pub, *wrap_epoch as i64],
                )?;
            }
        }
        OpBody::HeadAttest {
            device_id,
            seq,
            chain_hash,
        } => {
            conn.execute(
                "INSERT OR IGNORE INTO sync_head_attests (device_id, seq, chain_hash, signature)
                 VALUES (?1, ?2, ?3, ?4)",
                params![device_id, *seq as i64, &chain_hash[..], &op.signature[..]],
            )?;
        }
        _ => {}
    }
    Ok(())
}

fn device_id_of_pub(p: &[u8; 32]) -> String {
    crate::sync::crypto::device_id_of(p)
}

pub(crate) fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub(crate) fn role_db(r: DeviceRole) -> &'static str {
    match r {
        DeviceRole::Holder => "holder",
        DeviceRole::Mirror => "mirror",
    }
}

pub(crate) fn verified_db(v: VerifiedBy) -> &'static str {
    match v {
        VerifiedBy::Sas => "sas",
        VerifiedBy::SshHostKey => "ssh_host_key",
    }
}

pub(crate) fn kind_db(k: DeviceKind) -> &'static str {
    match k {
        DeviceKind::Terminal => "terminal",
        DeviceKind::Phone => "phone",
    }
}

pub(crate) fn attendance_db(a: Attendance) -> &'static str {
    match a {
        Attendance::Attended => "attended",
        Attendance::Unattended => "unattended",
    }
}

fn scope_json(s: &CertScope) -> String {
    match s {
        CertScope::Group(g) => format!("{{\"Group\":\"{}\"}}", uid_hex(g)),
        CertScope::Hosts(h) => format!(
            "{{\"Hosts\":[{}]}}",
            h.iter()
                .map(|u| format!("\"{}\"", uid_hex(u)))
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}

pub(crate) fn shares_json(shares: &[WrappedShare]) -> Result<String> {
    let v: Vec<serde_json::Value> = shares
        .iter()
        .map(|s| {
            serde_json::json!({
                "recipient": s.recipient,
                "alg": s.alg.to_bytes(),
                "sealed": super::canonical::b64_encode(&s.sealed),
            })
        })
        .collect();
    Ok(serde_json::to_string(&v)?)
}

/// Which of two conflicting ops (same entity) wins an automatic tie-break.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TieBreak {
    First,
    Second,
    ManualReview,
}

/// Deterministic `(lamport, device_id)` order. A revoke never loses an
/// automatic tie-break: any conflict that involves one goes to manual review.
pub fn tie_break(a: &Op, b: &Op) -> TieBreak {
    let revoke = |o: &Op| matches!(o.body, OpBody::DeviceRevoke { .. });
    if revoke(a) || revoke(b) {
        return TieBreak::ManualReview;
    }
    if (a.lamport, &a.device_id) >= (b.lamport, &b.device_id) {
        TieBreak::First
    } else {
        TieBreak::Second
    }
}
