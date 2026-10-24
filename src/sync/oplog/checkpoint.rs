//! Checkpoints and pruning (contract section 2b, C15).
//!
//! The replicated state is a deterministic function of the effective ops. A
//! checkpoint is the *fold* of that state at a cut of the log: per entity field
//! the winning write, per `sync_writes` key the winning write, the device table,
//! tombstones inside the horizon, the fork proofs and the chain anchors. It is
//! canonical JSON (keys sorted, C2 number rules), hashed with BLAKE3; the proposal
//! op carries only the hash. A device never trusts a blob: it rebuilds the fold
//! from its own store and compares.
//!
//! Pruning turns the winners of the ops below an older cut into *base writes*
//! (`op_hash` = 32 zero bytes, always effective) and deletes the ops. Bootstrap
//! writes the same base rows from a blob, so a pruned store and a bootstrapped one
//! have the same shape.

use anyhow::{anyhow, bail, ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

use super::canonical::{b64_decode, b64_encode};
use super::fork;
use super::merge::{hex, insert_op, load_op, op_at, State};
use super::op::*;
use super::tier::EntityKind;
use crate::store::{uid_from_hex, uid_hex, Uid};
use crate::sync::apply::reproject;
use crate::sync::crypto::chain_hash;
use crate::sync::limits::*;

/// `sync_chain_tail` below the prune floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pruned {
    pub floor_seq: u64,
}

impl std::fmt::Display for Pruned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "history below seq {} was pruned; adopt a checkpoint",
            self.floor_seq
        )
    }
}

impl std::error::Error for Pruned {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CheckpointStatus {
    Proposed,
    Verified,
    Disputed,
    Accepted,
    Pruned,
    Void,
}

impl CheckpointStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Verified => "verified",
            Self::Disputed => "disputed",
            Self::Accepted => "accepted",
            Self::Pruned => "pruned",
            Self::Void => "void",
        }
    }

    fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "proposed" => Self::Proposed,
            "verified" => Self::Verified,
            "disputed" => Self::Disputed,
            "accepted" => Self::Accepted,
            "pruned" => Self::Pruned,
            "void" => Self::Void,
            other => bail!("unknown checkpoint status {other}"),
        })
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CheckpointVerdict {
    Matches,
    Differs {
        section: String,
        detail: String,
    },
    /// Ops up to the cut this device does not have yet: `(device, cut seq)`.
    MissingOps(Vec<(DeviceId, u64)>),
    Blocked(String),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CheckpointBundle {
    pub proposal: Op,
    pub signs: Vec<Op>,
    pub heads: Vec<Op>,
    pub state: Vec<u8>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BootstrapTrust {
    /// The 8 words shown by an attended device of the ring.
    RingFingerprint(String),
    /// A device whose key the user verified (SAS) at enrollment.
    Anchor(DeviceId),
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct PruneReport {
    pub ops_deleted: u64,
    pub field_writes_deleted: u64,
    pub wrapped_share_blobs_dropped: u64,
    pub bytes_freed: u64,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CheckpointRow {
    pub proposal_hash: [u8; 32],
    pub device_id: DeviceId,
    pub prev: [u8; 32],
    pub state_hash: [u8; 32],
    pub state_len: u64,
    pub cut: Vec<CutEntry>,
    pub required: Vec<DeviceId>,
    pub status: CheckpointStatus,
    pub accepted_at: Option<i64>,
    pub signers: Vec<DeviceId>,
    pub has_state: bool,
}

const ZERO: [u8; 32] = [0; 32];

// ------------------------------------------------------------------ writes

type RawWrite = (&'static str, String, u64, Value);

fn scope_value(s: &CertScope) -> Value {
    match s {
        CertScope::Group(g) => json!({ "Group": uid_hex(g) }),
        CertScope::Hosts(h) => json!({ "Hosts": h.iter().map(uid_hex).collect::<Vec<_>>() }),
    }
}

fn writes_of(op: &Op, hash: &[u8; 32]) -> Vec<RawWrite> {
    match &op.body {
        OpBody::CredUpdate {
            host_uid,
            ciphertext,
            wrapped_shares,
        } => vec![(
            "cred",
            uid_hex(host_uid),
            0,
            json!({
                "ciphertext": b64_encode(ciphertext),
                "shares": wrapped_shares.iter().map(|s| json!({
                    "recipient": s.recipient,
                    "alg": s.alg.to_bytes(),
                    "sealed": b64_encode(&s.sealed),
                })).collect::<Vec<_>>(),
            }),
        )],
        OpBody::ShareReshareConfirmBatch { epoch, items } => items
            .iter()
            .map(|it| {
                (
                    "share",
                    format!("{}:{}", uid_hex(&it.host_uid), op.device_id),
                    *epoch,
                    json!({
                        "epoch": epoch,
                        "host_uid": uid_hex(&it.host_uid),
                        "holder": op.device_id,
                        "sealed": b64_encode(&it.sealed_share),
                    }),
                )
            })
            .collect(),
        OpBody::ShareStorageKey { epoch, sealed } => vec![(
            "storage",
            op.device_id.clone(),
            *epoch,
            json!({ "epoch": epoch, "sealed": b64_encode(sealed) }),
        )],
        OpBody::DeviceAdd {
            device,
            name,
            role,
            attendance,
            kind,
            verified_by,
        } if device.validate().is_ok() => vec![(
            "device",
            device.device_id(),
            0,
            json!({
                "id": device.device_id(),
                "ed25519": hex(&device.ed25519_pub),
                "x25519": hex(&device.x25519_pub),
                "mlkem": b64_encode(&device.mlkem_pub),
                "name": name,
                "role": super::merge::role_db(*role),
                "attendance": super::merge::attendance_db(*attendance),
                "kind": super::merge::kind_db(*kind),
                "verified_by": super::merge::verified_db(*verified_by),
                "added_by": hex(hash),
            }),
        )],
        OpBody::DeviceRoleChange { device_id, to } => vec![(
            "role",
            device_id.clone(),
            0,
            json!({ "role": super::merge::role_db(*to) }),
        )],
        OpBody::DeviceRevoke { device_id } => {
            vec![("revoke", device_id.clone(), 0, json!({ "by": hex(hash) }))]
        }
        OpBody::WrapKeyRotate {
            device_id,
            wrap_epoch,
            x25519_pub,
            mlkem_pub,
        } if *device_id == op.device_id && *wrap_epoch > 0 => vec![(
            "wrap",
            device_id.clone(),
            *wrap_epoch,
            json!({ "epoch": wrap_epoch, "x25519": hex(x25519_pub), "mlkem": b64_encode(mlkem_pub) }),
        )],
        OpBody::ModeChange { to, .. } => vec![(
            "mode",
            "mode".into(),
            0,
            json!({ "to": match to { CredMode::Vault => "vault", CredMode::Quorum => "quorum" } }),
        )],
        OpBody::CertIssue {
            device_id,
            scope,
            expiry,
            max_new_hosts_per_day,
            epoch_lo,
            epoch_hi,
        } => vec![(
            "cert",
            hex(hash),
            0,
            json!({
                "device_id": device_id,
                "scope": scope_value(scope),
                "expiry": expiry,
                "max_new_hosts_per_day": max_new_hosts_per_day,
                "epoch_lo": epoch_lo,
                "epoch_hi": epoch_hi,
                "issuer": op.device_id,
            }),
        )],
        OpBody::CertRevoke { cert_id } => vec![("cert_revoked", hex(cert_id), 0, json!({}))],
        OpBody::CertHostOpened { cert_id, host_uid } => vec![(
            "cert_open",
            format!("{}:{}", hex(cert_id), uid_hex(host_uid)),
            0,
            json!({ "opened_at": op.timestamp }),
        )],
        OpBody::HeadAttest {
            device_id,
            seq,
            chain_hash,
        } if *device_id == op.device_id => vec![(
            "head",
            device_id.clone(),
            *seq,
            json!({ "seq": seq, "hash": hex(chain_hash) }),
        )],
        _ => Vec::new(),
    }
}

/// Record the per-key writes of `op` (state independent; effectiveness is the
/// op's state).
pub(crate) fn record_writes(conn: &Connection, op: &Op, hash: &[u8; 32]) -> Result<()> {
    for (section, key, ord, value) in writes_of(op, hash) {
        conn.execute(
            "INSERT OR IGNORE INTO sync_writes (section, key, op_hash, ord, lamport, device_id, seq, value)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                section,
                key,
                &hash[..],
                ord as i64,
                op.lamport as i64,
                op.device_id,
                op.seq as i64,
                serde_json::to_string(&value)?
            ],
        )?;
    }
    Ok(())
}

/// A trusted device registered without an op (enrollment ceremony, first entry
/// of a ring) is a base write: effective everywhere, identical on every device
/// that registered it.
pub(crate) fn record_anchor_device(
    conn: &Connection,
    public: &crate::sync::crypto::DevicePublic,
    name: &str,
    role: DeviceRole,
    attendance: Attendance,
    kind: DeviceKind,
    verified_by: VerifiedBy,
) -> Result<()> {
    let value = json!({
        "id": public.device_id(),
        "ed25519": hex(&public.ed25519_pub),
        "x25519": hex(&public.x25519_pub),
        "mlkem": b64_encode(&public.mlkem_pub),
        "name": name,
        "role": super::merge::role_db(role),
        "attendance": super::merge::attendance_db(attendance),
        "kind": super::merge::kind_db(kind),
        "verified_by": super::merge::verified_db(verified_by),
        "added_by": Value::Null,
    });
    conn.execute(
        "INSERT OR IGNORE INTO sync_writes (section, key, op_hash, ord, lamport, device_id, seq, value)
         VALUES ('device', ?1, zeroblob(32), 0, 0, '', 0, ?2)",
        params![public.device_id(), serde_json::to_string(&value)?],
    )?;
    Ok(())
}

// -------------------------------------------------------------------- fold

type Cut = BTreeMap<DeviceId, u64>;

fn cut_of(entries: &[CutEntry]) -> Cut {
    entries
        .iter()
        .map(|c| (c.device_id.clone(), c.seq))
        .collect()
}

fn in_cut(cut: Option<&Cut>, device: &str, seq: u64) -> bool {
    cut.is_none_or(|c| c.get(device).is_some_and(|s| seq <= *s))
}

#[derive(Clone)]
struct Win {
    ord: u64,
    lamport: u64,
    device: String,
    seq: u64,
    value: Value,
}

fn fold_writes(conn: &Connection, cut: Option<&Cut>) -> Result<BTreeMap<(String, String), Win>> {
    let mut stmt = conn.prepare(
        "SELECT w.section, w.key, w.ord, w.lamport, w.device_id, w.seq, w.value,
                w.op_hash = zeroblob(32), o.state
         FROM sync_writes w LEFT JOIN sync_ops o ON o.op_hash = w.op_hash
         ORDER BY w.ord, w.lamport, w.device_id, w.seq",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)? as u64,
            r.get::<_, i64>(3)? as u64,
            r.get::<_, String>(4)?,
            r.get::<_, i64>(5)? as u64,
            r.get::<_, String>(6)?,
            r.get::<_, bool>(7)?,
            r.get::<_, Option<String>>(8)?,
        ))
    })?;
    let mut out = BTreeMap::new();
    for row in rows {
        let (section, key, ord, lamport, device, seq, value, base, state) = row?;
        let effective = base
            || (state
                .as_deref()
                .is_some_and(|s| s == "applied" || s == "approved")
                && in_cut(cut, &device, seq));
        if !effective {
            continue;
        }
        out.insert(
            (section, key),
            Win {
                ord,
                lamport,
                device,
                seq,
                value: serde_json::from_str(&value)?,
            },
        );
    }
    Ok(out)
}

struct EntityFold {
    fields: BTreeMap<String, Win>,
    tomb: Option<(u64, String, String, i64)>, // lamport, device, op_hash hex, ts
}

fn fold_entities(
    conn: &Connection,
    cut: Option<&Cut>,
) -> Result<BTreeMap<(String, Uid), EntityFold>> {
    let mut out: BTreeMap<(String, Uid), EntityFold> = BTreeMap::new();
    let mut stmt = conn.prepare(
        "SELECT f.kind, f.uid, f.field, f.lamport, f.device_id, f.seq, f.value,
                f.op_hash = zeroblob(32), o.state
         FROM sync_fields f LEFT JOIN sync_ops o ON o.op_hash = f.op_hash
         ORDER BY f.lamport, f.device_id, f.seq",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Vec<u8>>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)? as u64,
            r.get::<_, String>(4)?,
            r.get::<_, i64>(5)? as u64,
            r.get::<_, String>(6)?,
            r.get::<_, bool>(7)?,
            r.get::<_, Option<String>>(8)?,
        ))
    })?;
    for row in rows {
        let (kind, uid, field, lamport, device, seq, value, base, state) = row?;
        let effective = base
            || (state
                .as_deref()
                .is_some_and(|s| s == "applied" || s == "approved")
                && in_cut(cut, &device, seq));
        if !effective {
            continue;
        }
        let Ok(uid) = <Uid>::try_from(uid) else {
            continue;
        };
        let e = out.entry((kind, uid)).or_insert_with(|| EntityFold {
            fields: BTreeMap::new(),
            tomb: None,
        });
        e.fields.insert(
            field,
            Win {
                ord: 0,
                lamport,
                device,
                seq,
                value: serde_json::from_str(&value)?,
            },
        );
    }
    let mut stmt = conn.prepare(
        "SELECT t.kind, t.uid, t.lamport, t.device_id, t.op_hash, t.ts, o.seq, o.state
         FROM sync_tombstones t LEFT JOIN sync_ops o ON o.op_hash = t.op_hash",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Vec<u8>>(1)?,
            r.get::<_, i64>(2)? as u64,
            r.get::<_, String>(3)?,
            r.get::<_, Vec<u8>>(4)?,
            r.get::<_, i64>(5)?,
            r.get::<_, Option<i64>>(6)?,
            r.get::<_, Option<String>>(7)?,
        ))
    })?;
    for row in rows {
        let (kind, uid, lamport, device, op_hash, ts, seq, state) = row?;
        let base = op_hash == ZERO;
        let effective = base
            || (state
                .as_deref()
                .is_some_and(|s| s == "applied" || s == "approved")
                && in_cut(cut, &device, seq.unwrap_or(0) as u64));
        if !effective {
            continue;
        }
        let Ok(uid) = <Uid>::try_from(uid) else {
            continue;
        };
        let e = out.entry((kind, uid)).or_insert_with(|| EntityFold {
            fields: BTreeMap::new(),
            tomb: None,
        });
        let cand = (lamport, device, hex(&op_hash), ts);
        if e.tomb
            .as_ref()
            .is_none_or(|t| (t.0, &t.1) < (cand.0, &cand.1))
        {
            e.tomb = Some(cand);
        }
    }
    Ok(out)
}

fn win_json(w: &Win) -> Value {
    json!({ "value": w.value, "lamport": w.lamport, "device_id": w.device, "seq": w.seq })
}

fn entity_alive(e: &EntityFold) -> bool {
    let max_write = e.fields.values().map(|w| w.lamport).max();
    match (max_write, &e.tomb) {
        (None, _) => false,
        (Some(_), None) => true,
        (Some(w), Some(t)) => w > t.0,
    }
}

/// Head ops of the cut as `(timestamp, epoch)`.
fn head_facts(conn: &Connection, cut: &[CutEntry]) -> Result<BTreeMap<DeviceId, (i64, u64)>> {
    let mut out = BTreeMap::new();
    for c in cut {
        if let Some(row) = op_at(conn, &c.device_id, c.seq)? {
            out.insert(c.device_id.clone(), (row.op.timestamp, row.op.epoch));
        }
    }
    Ok(out)
}

fn device_section(
    wins: &BTreeMap<(String, String), Win>,
    heads: &BTreeMap<DeviceId, (i64, u64)>,
    forks: &BTreeSet<DeviceId>,
) -> Vec<Value> {
    let mut out = Vec::new();
    for ((section, key), w) in wins {
        if section != "device" {
            continue;
        }
        let mut d = w.value.clone();
        let wrap = wins.get(&("wrap".to_string(), key.clone()));
        let revoked = wins.contains_key(&("revoke".to_string(), key.clone()));
        let m = d.as_object_mut().expect("device value is an object");
        let mut wrap_epoch = 0u64;
        if let Some(wr) = wrap {
            m.insert("x25519".into(), wr.value["x25519"].clone());
            m.insert("mlkem".into(), wr.value["mlkem"].clone());
            wrap_epoch = wr.ord;
        }
        if let Some(r) = wins.get(&("role".to_string(), key.clone())) {
            m.insert("role".into(), r.value["role"].clone());
        }
        m.insert("wrap_epoch".into(), json!(wrap_epoch));
        m.insert(
            "status".into(),
            json!(if forks.contains(key) {
                "compromised"
            } else if revoked {
                "revoked"
            } else {
                "operational"
            }),
        );
        m.insert(
            "head_ts".into(),
            heads.get(key).map_or(Value::Null, |h| json!(h.0)),
        );
        out.push(d);
    }
    out
}

/// Attended live terminals of a device table: who must sign a checkpoint
/// proposed at `ts`.
fn required_of(devices: &[Value], ts: i64) -> Vec<DeviceId> {
    let mut req: Vec<DeviceId> = devices
        .iter()
        .filter(|d| {
            d["kind"] == "terminal"
                && d["attendance"] == "attended"
                && d["status"] == "operational"
                && d["head_ts"]
                    .as_i64()
                    .is_some_and(|t| t >= ts - LIVE_DEVICE_WINDOW_SECS)
        })
        .filter_map(|d| d["id"].as_str().map(String::from))
        .collect();
    req.sort();
    req
}

fn fork_set(conn: &Connection) -> Result<BTreeSet<DeviceId>> {
    let mut stmt =
        conn.prepare("SELECT device_id FROM sync_devices WHERE status = 'compromised'")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// The state blob as of `cut`, with the tombstone horizon measured at `ts`.
fn fold_state(conn: &Connection, cut_entries: &[CutEntry], ts: i64) -> Result<Vec<u8>> {
    let cut = cut_of(cut_entries);
    let wins = fold_writes(conn, Some(&cut))?;
    let heads = head_facts(conn, cut_entries)?;
    let forks = fork_set(conn)?;
    let devices = device_section(&wins, &heads, &forks);

    let mut entities = Vec::new();
    for ((kind, uid), e) in fold_entities(conn, Some(&cut))? {
        let alive = entity_alive(&e);
        if !alive {
            // Beyond the horizon a deleted entity is forgotten altogether.
            match &e.tomb {
                Some(t) if t.3 >= ts - TOMBSTONE_KEEP_SECS => {}
                _ => continue,
            }
        }
        let mut m = Map::new();
        m.insert("kind".into(), json!(kind));
        m.insert("uid".into(), json!(uid_hex(&uid)));
        m.insert("alive".into(), json!(alive));
        m.insert(
            "fields".into(),
            Value::Object(
                e.fields
                    .iter()
                    .map(|(k, w)| (k.clone(), win_json(w)))
                    .collect(),
            ),
        );
        if let (false, Some(t)) = (alive, &e.tomb) {
            m.insert(
                "tombstone".into(),
                json!({ "lamport": t.0, "device_id": t.1, "ts": t.3 }),
            );
        }
        entities.push(Value::Object(m));
    }

    // Live shares only: a credential that was resharded forgets the wrapped
    // shares of its credential op; envelopes of older epochs are superseded.
    let resharded: BTreeSet<&str> = wins
        .iter()
        .filter(|((s, _), _)| s == "share")
        .filter_map(|(_, w)| w.value["host_uid"].as_str())
        .collect();
    let mut writes = Vec::new();
    for ((section, key), w) in &wins {
        if matches!(section.as_str(), "device" | "revoke" | "wrap") {
            // Folded into the device table below, but kept as writes too so a
            // bootstrapped store has the winners' keys.
        }
        let mut value = w.value.clone();
        if section == "cred" && resharded.contains(key.as_str()) {
            value["shares"] = json!([]);
        }
        writes.push(json!({
            "section": section, "key": key, "ord": w.ord, "lamport": w.lamport,
            "device_id": w.device, "seq": w.seq, "value": value,
        }));
    }
    let epochs: Vec<Value> = heads
        .iter()
        .map(|(d, (_, e))| json!({ "device_id": d, "epoch": e }))
        .collect();
    let mut fork_entries = Vec::new();
    for d in &forks {
        if let Some(ev) = fork::raw_evidence(conn, d)? {
            fork_entries
                .push(json!({ "device_id": d, "a": b64_encode(&ev.0), "b": b64_encode(&ev.1) }));
        }
    }
    let state = json!({
        "cut": cut_entries.iter().map(|c| json!({
            "device_id": c.device_id, "seq": c.seq, "op_hash": hex(&c.op_hash), "lamport": c.lamport,
        })).collect::<Vec<_>>(),
        "devices": devices,
        "entities": entities,
        "epochs": epochs,
        "forks": fork_entries,
        "writes": writes,
    });
    Ok(serde_json::to_vec(&state)?)
}

// ------------------------------------------------------------------- build

fn local_heads(conn: &Connection) -> Result<Vec<CutEntry>> {
    let mut stmt = conn.prepare(
        "SELECT device_id, seq, op_hash, lamport FROM sync_chain_heads ORDER BY device_id",
    )?;
    let rows: Vec<(String, i64, Vec<u8>, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<std::result::Result<_, _>>()?;
    rows.into_iter()
        .map(|(d, s, h, l)| {
            Ok(CutEntry {
                device_id: d,
                seq: s as u64,
                op_hash: h.try_into().map_err(|_| anyhow!("stored hash"))?,
                lamport: l as u64,
            })
        })
        .collect()
}

fn recovery_in_flight(conn: &Connection) -> Result<bool> {
    // A PhoneReenroll that no cancel names and that no later phone device add closed.
    let mut stmt = conn.prepare(
        "SELECT op_hash, lamport FROM sync_ops WHERE kind = 'PhoneReenroll' AND state IN ('applied','approved','pending')",
    )?;
    let reenrolls: Vec<(Vec<u8>, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    for (hash, lamport) in reenrolls {
        let cancelled = row_cancels(conn, &hash)?;
        let replaced: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sync_writes w WHERE w.section = 'device'
                AND json_extract(w.value, '$.kind') = 'phone' AND w.lamport > ?1)",
            params![lamport],
            |r| r.get(0),
        )?;
        if !cancelled && !replaced {
            return Ok(true);
        }
    }
    Ok(false)
}

fn row_cancels(conn: &Connection, reenroll_hash: &[u8]) -> Result<bool> {
    let mut stmt = conn.prepare(
        "SELECT op_hash FROM sync_ops WHERE kind = 'PhoneReenrollCancel' AND state IN ('applied','approved')",
    )?;
    let hashes: Vec<Vec<u8>> = stmt
        .query_map([], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    for h in hashes {
        let h: [u8; 32] = h.try_into().map_err(|_| anyhow!("stored hash"))?;
        if let Some(row) = load_op(conn, &h)? {
            if let OpBody::PhoneReenrollCancel {
                reenroll_hash: r, ..
            } = row.op.body
            {
                if r[..] == *reenroll_hash {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

fn pending_below(conn: &Connection, cut: &Cut) -> Result<usize> {
    let mut n = 0;
    let mut stmt = conn.prepare("SELECT device_id, seq FROM sync_ops WHERE state = 'pending'")?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64))
    })?;
    for row in rows {
        let (d, s) = row?;
        if in_cut(Some(cut), &d, s) {
            n += 1;
        }
    }
    Ok(n)
}

/// Fold the local state as of the cut. `ts` is the proposal time (tombstone
/// horizon, liveness).
pub(crate) fn build(
    conn: &Connection,
    cut: Option<Vec<CutEntry>>,
    ts: i64,
) -> Result<(Vec<CutEntry>, Vec<u8>, [u8; 32])> {
    let mut cut = match cut {
        Some(c) => c,
        None => local_heads(conn)?,
    };
    cut.sort_by(|a, b| a.device_id.cmp(&b.device_id));
    let n = pending_below(conn, &cut_of(&cut))?;
    ensure!(
        n == 0,
        "{n} ops below the cut are still pending: resolve them first"
    );
    ensure!(
        !recovery_in_flight(conn)?,
        "a phone re-enrollment is running"
    );
    let state = fold_state(conn, &cut, ts)?;
    let hash = chain_hash(&state);
    Ok((cut, state, hash))
}

pub(crate) fn required_for(conn: &Connection, cut: &[CutEntry], ts: i64) -> Result<Vec<DeviceId>> {
    let state: Value = serde_json::from_slice(&fold_state(conn, cut, ts)?)?;
    Ok(required_of(
        state["devices"].as_array().unwrap_or(&Vec::new()),
        ts,
    ))
}

// ------------------------------------------------------------------ verify

const SECTIONS: [&str; 6] = ["cut", "devices", "entities", "epochs", "forks", "writes"];

fn key_of(v: &Value) -> String {
    for k in ["uid", "id", "device_id", "key"] {
        if let Some(s) = v.get(k).and_then(Value::as_str) {
            return s.to_string();
        }
    }
    v.to_string().chars().take(40).collect()
}

pub(crate) fn diff_state(local: &Value, theirs: &Value) -> Option<(String, String)> {
    for section in SECTIONS {
        let (a, b) = (&local[section], &theirs[section]);
        if a == b {
            continue;
        }
        let (aa, bb) = (
            a.as_array().cloned().unwrap_or_default(),
            b.as_array().cloned().unwrap_or_default(),
        );
        let ka: BTreeMap<String, &Value> = aa.iter().map(|v| (key_of(v), v)).collect();
        let kb: BTreeMap<String, &Value> = bb.iter().map(|v| (key_of(v), v)).collect();
        for (k, v) in &ka {
            match kb.get(k) {
                None => return Some((section.into(), format!("only in the local fold: {k}"))),
                Some(o) if o != v => {
                    let clip = |x: &Value| x.to_string().chars().take(700).collect::<String>();
                    return Some((
                        section.into(),
                        format!("{k} differs: local {} vs theirs {}", clip(v), clip(o)),
                    ));
                }
                _ => {}
            }
        }
        for k in kb.keys() {
            if !ka.contains_key(k) {
                return Some((section.into(), format!("only in the proposal: {k}")));
            }
        }
        return Some((section.into(), "section differs".into()));
    }
    None
}

pub(crate) fn latest_accepted_hash(conn: &Connection) -> Result<[u8; 32]> {
    Ok(latest_accepted(conn)?.unwrap_or(ZERO))
}

fn latest_accepted(conn: &Connection) -> Result<Option<[u8; 32]>> {
    let h: Option<Vec<u8>> = conn
        .query_row(
            "SELECT proposal_hash FROM sync_checkpoints WHERE status IN ('accepted','pruned')
             ORDER BY accepted_at DESC, lamport DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    Ok(h.and_then(|h| h.try_into().ok()))
}

type ProposalParts<'a> = (
    &'a Vec<CutEntry>,
    &'a [u8; 32],
    u64,
    &'a [u8; 32],
    &'a Vec<DeviceId>,
);

fn propose_parts(op: &Op) -> Result<ProposalParts<'_>> {
    match &op.body {
        OpBody::CheckpointPropose {
            cut,
            state_hash,
            state_len,
            prev_checkpoint,
            required,
        } => Ok((cut, state_hash, *state_len, prev_checkpoint, required)),
        _ => bail!("not a CheckpointPropose op"),
    }
}

pub(crate) fn verify(conn: &Connection, proposal: &Op, state: &[u8]) -> Result<CheckpointVerdict> {
    let (cut, state_hash, state_len, prev, required) = propose_parts(proposal)?;
    if chain_hash(state) != *state_hash || state.len() as u64 != state_len {
        return Ok(CheckpointVerdict::Differs {
            section: "blob".into(),
            detail: "the state blob does not match the proposal's hash or length".into(),
        });
    }
    let expected_prev = latest_accepted(conn)?.unwrap_or(ZERO);
    if *prev != expected_prev {
        return Ok(CheckpointVerdict::Differs {
            section: "prev_checkpoint".into(),
            detail: "does not continue the last accepted checkpoint".into(),
        });
    }
    let mut missing = Vec::new();
    for c in cut {
        let head: Option<i64> = conn
            .query_row(
                "SELECT seq FROM sync_chain_heads WHERE device_id = ?1",
                params![c.device_id],
                |r| r.get(0),
            )
            .optional()?;
        if head.is_none_or(|h| (h as u64) < c.seq) {
            missing.push((c.device_id.clone(), c.seq));
            continue;
        }
        if let Some(row) = op_at(conn, &c.device_id, c.seq)? {
            if row.hash != c.op_hash {
                return Ok(CheckpointVerdict::Differs {
                    section: "cut".into(),
                    detail: format!("{} seq {} has another hash here", c.device_id, c.seq),
                });
            }
        }
    }
    if !missing.is_empty() {
        return Ok(CheckpointVerdict::MissingOps(missing));
    }
    let n = pending_below(conn, &cut_of(cut))?;
    if n > 0 {
        return Ok(CheckpointVerdict::Blocked(format!(
            "{n} ops below the cut are pending here"
        )));
    }
    if recovery_in_flight(conn)? {
        return Ok(CheckpointVerdict::Blocked(
            "a phone re-enrollment is running".into(),
        ));
    }
    let mine = fold_state(conn, cut, proposal.timestamp)?;
    if mine != state {
        let (a, b): (Value, Value) = (
            serde_json::from_slice(&mine)?,
            serde_json::from_slice(state)?,
        );
        let (section, detail) =
            diff_state(&a, &b).unwrap_or_else(|| ("blob".into(), "not in canonical form".into()));
        return Ok(CheckpointVerdict::Differs { section, detail });
    }
    let want = required_of(a_devices(&mine)?.as_slice(), proposal.timestamp);
    if want != *required {
        return Ok(CheckpointVerdict::Differs {
            section: "required".into(),
            detail: format!("local device table requires {want:?}"),
        });
    }
    Ok(CheckpointVerdict::Matches)
}

fn a_devices(state: &[u8]) -> Result<Vec<Value>> {
    let v: Value = serde_json::from_slice(state)?;
    Ok(v["devices"].as_array().cloned().unwrap_or_default())
}

// ------------------------------------------------------------------ record

fn device_pub(conn: &Connection, device: &str) -> Result<Option<[u8; 32]>> {
    let raw: Option<Vec<u8>> = conn
        .query_row(
            "SELECT ed25519 FROM sync_devices WHERE device_id = ?1",
            params![device],
            |r| r.get(0),
        )
        .optional()?;
    Ok(raw.and_then(|r| r.try_into().ok()))
}

fn op_blob(op: &Op) -> Vec<u8> {
    let mut b = op.signature.to_vec();
    b.extend_from_slice(&op.canonical_bytes());
    b
}

fn op_from_blob(b: &[u8]) -> Result<Op> {
    ensure!(b.len() > 64, "bad op blob");
    Op::from_canonical(&b[64..], b[..64].try_into()?)
}

fn cut_json(cut: &[CutEntry]) -> String {
    serde_json::to_string(
        &cut.iter()
            .map(|c| json!([c.device_id, c.seq, hex(&c.op_hash), c.lamport]))
            .collect::<Vec<_>>(),
    )
    .expect("cut serialises")
}

fn cut_from_json(s: &str) -> Result<Vec<CutEntry>> {
    let v: Vec<Value> = serde_json::from_str(s)?;
    v.iter()
        .map(|e| {
            let a = e.as_array().context("cut entry")?;
            let h = a[2].as_str().context("hash")?;
            let mut op_hash = [0u8; 32];
            for (i, b) in op_hash.iter_mut().enumerate() {
                *b = u8::from_str_radix(&h[2 * i..2 * i + 2], 16)?;
            }
            Ok(CutEntry {
                device_id: a[0].as_str().context("device")?.to_string(),
                seq: a[1].as_u64().context("seq")?,
                op_hash,
                lamport: a[3].as_u64().context("lamport")?,
            })
        })
        .collect()
}

fn signers_of(conn: &Connection, proposal: &[u8; 32]) -> Result<Vec<DeviceId>> {
    let mut stmt = conn.prepare(
        "SELECT device_id FROM sync_checkpoint_signs WHERE proposal_hash = ?1 ORDER BY device_id",
    )?;
    let rows = stmt.query_map(params![&proposal[..]], |r| r.get(0))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

pub(crate) fn attended_terminal(conn: &Connection, device: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sync_devices WHERE device_id = ?1 AND kind = 'terminal'
           AND attendance = 'attended' AND status NOT IN ('revoked','compromised'))",
        params![device],
        |r| r.get(0),
    )?)
}

fn set_status(conn: &Connection, hash: &[u8; 32], st: CheckpointStatus) -> Result<()> {
    conn.execute(
        "UPDATE sync_checkpoints SET status = ?2 WHERE proposal_hash = ?1",
        params![&hash[..], st.as_str()],
    )?;
    Ok(())
}

pub(crate) fn record(
    conn: &Connection,
    bundle: &CheckpointBundle,
    now: i64,
) -> Result<CheckpointStatus> {
    let proposal = &bundle.proposal;
    let (cut, state_hash, state_len, prev, required) = propose_parts(proposal)?;
    let proposer_pub = device_pub(conn, &proposal.device_id)?.context("unknown proposer")?;
    ensure!(proposal.verify(&proposer_pub), "bad proposal signature");
    let hash = proposal.hash();

    let existing: Option<String> = conn
        .query_row(
            "SELECT status FROM sync_checkpoints WHERE proposal_hash = ?1",
            params![&hash[..]],
            |r| r.get(0),
        )
        .optional()?;
    if existing.is_none() {
        conn.execute(
            "INSERT INTO sync_checkpoints (proposal_hash, device_id, prev, state_hash, state_len, cut,
                 required, status, lamport, proposed_at, op_blob)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'proposed', ?8, ?9, ?10)",
            params![
                &hash[..],
                proposal.device_id,
                &prev[..],
                &state_hash[..],
                state_len as i64,
                cut_json(cut),
                serde_json::to_string(required)?,
                proposal.lamport as i64,
                proposal.timestamp,
                op_blob(proposal),
            ],
        )?;
    }
    for sign in &bundle.signs {
        let OpBody::CheckpointSign { proposal_hash } = &sign.body else {
            continue;
        };
        if *proposal_hash != hash {
            continue;
        }
        let Some(p) = device_pub(conn, &sign.device_id)? else {
            continue;
        };
        // Unattended devices never sign: their signature proves a file was read.
        if !sign.verify(&p) || !attended_terminal(conn, &sign.device_id)? {
            continue;
        }
        conn.execute(
            "INSERT OR IGNORE INTO sync_checkpoint_signs (proposal_hash, device_id, sign_op_hash, op_blob)
             VALUES (?1, ?2, ?3, ?4)",
            params![&hash[..], sign.device_id, &sign.hash()[..], op_blob(sign)],
        )?;
    }

    let verdict = verify(conn, proposal, &bundle.state)?;
    let status = match verdict {
        CheckpointVerdict::Differs { .. } => {
            fork::mark_suspect(conn, &proposal.device_id)?;
            CheckpointStatus::Disputed
        }
        CheckpointVerdict::MissingOps(_) | CheckpointVerdict::Blocked(_) => {
            CheckpointStatus::Proposed
        }
        CheckpointVerdict::Matches => {
            let signers: BTreeSet<DeviceId> = signers_of(conn, &hash)?.into_iter().collect();
            let attended = signers
                .iter()
                .filter(|d| attended_terminal(conn, d).unwrap_or(false))
                .count();
            if required.iter().all(|r| signers.contains(r)) && attended >= CHECKPOINT_SIGN_MIN {
                CheckpointStatus::Accepted
            } else {
                CheckpointStatus::Verified
            }
        }
    };
    // Never downgrade an accepted checkpoint.
    if existing.as_deref() == Some("accepted") && status != CheckpointStatus::Disputed {
        return Ok(CheckpointStatus::Accepted);
    }
    set_status(conn, &hash, status)?;
    if status == CheckpointStatus::Accepted {
        conn.execute(
            "UPDATE sync_checkpoints SET accepted_at = ?2, state = ?3 WHERE proposal_hash = ?1",
            params![&hash[..], now, bundle.state],
        )?;
        flag_stale_devices(conn, cut, proposal.timestamp)?;
    }
    settle_competitors(conn, prev)?;
    drop_old_blobs(conn)?;
    let status: String = conn.query_row(
        "SELECT status FROM sync_checkpoints WHERE proposal_hash = ?1",
        params![&hash[..]],
        |r| r.get(0),
    )?;
    CheckpointStatus::parse(&status)
}

/// Same `prev_checkpoint`: lowest `(lamport, device_id)` wins, the others are
/// void (an accepted one is never voided).
fn settle_competitors(conn: &Connection, prev: &[u8; 32]) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT proposal_hash, status FROM sync_checkpoints
         WHERE prev = ?1 AND status NOT IN ('void','pruned') ORDER BY lamport, device_id",
    )?;
    let rows: Vec<(Vec<u8>, String)> = stmt
        .query_map(params![&prev[..]], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let accepted = rows.iter().any(|(_, s)| s == "accepted");
    for (i, (h, status)) in rows.iter().enumerate() {
        let keep = status == "accepted" || (i == 0 && !accepted);
        if !keep {
            conn.execute(
                "UPDATE sync_checkpoints SET status = 'void' WHERE proposal_hash = ?1",
                params![h],
            )?;
        }
    }
    Ok(())
}

fn drop_old_blobs(conn: &Connection) -> Result<()> {
    conn.execute(
        "UPDATE sync_checkpoints SET state = NULL WHERE proposal_hash NOT IN (
            SELECT proposal_hash FROM sync_checkpoints WHERE status IN ('accepted','pruned')
            ORDER BY accepted_at DESC, lamport DESC LIMIT ?1)",
        params![CHECKPOINT_RETAIN_SPANS as i64],
    )?;
    Ok(())
}

fn flag_stale_devices(conn: &Connection, cut: &[CutEntry], ts: i64) -> Result<()> {
    for c in cut {
        let head = op_at(conn, &c.device_id, c.seq)?;
        if head.is_some_and(|h| h.op.timestamp < ts - STALE_AFTER_SECS) {
            conn.execute(
                "INSERT OR REPLACE INTO sync_meta (key, value) VALUES (?1, ?2)",
                params![
                    format!("stale:{}", c.device_id),
                    c.seq.to_string().as_bytes()
                ],
            )?;
        }
    }
    Ok(())
}

/// Was the op's author stale at an accepted checkpoint, and is the op above
/// its cut head?
pub(crate) fn stale_above_cut(conn: &Connection, op: &Op) -> Result<bool> {
    let v: Option<Vec<u8>> = conn
        .query_row(
            "SELECT value FROM sync_meta WHERE key = ?1",
            params![format!("stale:{}", op.device_id)],
            |r| r.get(0),
        )
        .optional()?;
    Ok(
        v.and_then(|b| String::from_utf8(b).ok()?.parse::<u64>().ok())
            .is_some_and(|cut_seq| op.seq > cut_seq),
    )
}

pub(crate) fn list(conn: &Connection) -> Result<Vec<CheckpointRow>> {
    let mut stmt = conn.prepare(
        "SELECT proposal_hash, device_id, prev, state_hash, state_len, cut, required, status,
                accepted_at, state IS NOT NULL
         FROM sync_checkpoints ORDER BY lamport, device_id",
    )?;
    let rows: Vec<_> = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, Vec<u8>>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Vec<u8>>(2)?,
                r.get::<_, Vec<u8>>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, Option<i64>>(8)?,
                r.get::<_, bool>(9)?,
            ))
        })?
        .collect::<std::result::Result<_, _>>()?;
    let h32 = |v: Vec<u8>| -> Result<[u8; 32]> { v.try_into().map_err(|_| anyhow!("stored hash")) };
    rows.into_iter()
        .map(|(h, d, p, sh, len, cut, req, st, acc, has)| {
            let proposal_hash = h32(h)?;
            Ok(CheckpointRow {
                signers: signers_of(conn, &proposal_hash)?,
                proposal_hash,
                device_id: d,
                prev: h32(p)?,
                state_hash: h32(sh)?,
                state_len: len as u64,
                cut: cut_from_json(&cut)?,
                required: serde_json::from_str(&req)?,
                status: CheckpointStatus::parse(&st)?,
                accepted_at: acc,
                has_state: has,
            })
        })
        .collect()
}

/// The bundle of a recorded checkpoint, for serving to a fresh device.
pub(crate) fn bundle_of(
    conn: &Connection,
    proposal_hash: &[u8; 32],
) -> Result<Option<CheckpointBundle>> {
    let row: Option<(Vec<u8>, Option<Vec<u8>>, String)> = conn
        .query_row(
            "SELECT op_blob, state, cut FROM sync_checkpoints WHERE proposal_hash = ?1",
            params![&proposal_hash[..]],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((blob, Some(state), cut)) = row else {
        return Ok(None);
    };
    let proposal = op_from_blob(&blob)?;
    let mut signs = Vec::new();
    let mut stmt = conn.prepare(
        "SELECT op_blob FROM sync_checkpoint_signs WHERE proposal_hash = ?1 ORDER BY device_id",
    )?;
    for b in stmt.query_map(params![&proposal_hash[..]], |r| r.get::<_, Vec<u8>>(0))? {
        signs.push(op_from_blob(&b?)?);
    }
    let mut heads = Vec::new();
    for c in cut_from_json(&cut)? {
        if let Some(row) = op_at(conn, &c.device_id, c.seq)? {
            heads.push(row.op);
        }
    }
    Ok(Some(CheckpointBundle {
        proposal,
        signs,
        heads,
        state,
    }))
}

// -------------------------------------------------------------------- prune

fn count_shares(op: &Op) -> u64 {
    match &op.body {
        OpBody::CredUpdate { wrapped_shares, .. } => wrapped_shares.len() as u64,
        OpBody::ShareReshareBatch { items, .. } => {
            items.iter().map(|i| i.sub_shares.len() as u64).sum()
        }
        OpBody::ShareReshareConfirmBatch { items, .. } => items.len() as u64,
        OpBody::ShareReshareSession { .. } | OpBody::ShareStorageKey { .. } => 1,
        _ => 0,
    }
}

pub(crate) fn prune(conn: &Connection, now: i64) -> Result<PruneReport> {
    let accepted: Vec<CheckpointRow> = list(conn)?
        .into_iter()
        .filter(|c| {
            matches!(
                c.status,
                CheckpointStatus::Accepted | CheckpointStatus::Pruned
            )
        })
        .collect();
    let mut by_time: Vec<&CheckpointRow> = accepted.iter().collect();
    by_time.sort_by_key(|c| c.accepted_at);
    let [.., older, newest] = by_time.as_slice() else {
        return Ok(PruneReport::default());
    };
    if now - newest.accepted_at.unwrap_or(now) < CHECKPOINT_PRUNE_DELAY_SECS {
        return Ok(PruneReport::default());
    }
    let size_before = db_bytes(conn)?;
    let mut report = PruneReport::default();

    // Ops below the floor.
    conn.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS _pruned (op_hash BLOB PRIMARY KEY); DELETE FROM _pruned;",
    )?;
    for c in &older.cut {
        conn.execute(
            "INSERT OR IGNORE INTO _pruned SELECT op_hash FROM sync_ops WHERE device_id = ?1 AND seq < ?2",
            params![c.device_id, c.seq as i64],
        )?;
    }
    let mut stmt = conn.prepare(
        "SELECT o.canonical, o.signature FROM sync_ops o JOIN _pruned p ON p.op_hash = o.op_hash",
    )?;
    let doomed: Vec<(Vec<u8>, Vec<u8>)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    drop(stmt);
    for (canonical, sig) in &doomed {
        if let Ok(sig) = <[u8; 64]>::try_from(sig.as_slice()) {
            if let Ok(op) = Op::from_canonical(canonical, sig) {
                report.wrapped_share_blobs_dropped += count_shares(&op);
            }
        }
    }

    // Winners of the pruned (and base) writes become base writes.
    report.field_writes_deleted += collapse(
        conn,
        "sync_fields",
        "kind, uid, field",
        "lamport, device_id, seq",
        "kind, uid, field, op_hash, lamport, device_id, seq, value",
    )?;
    report.field_writes_deleted += collapse(
        conn,
        "sync_writes",
        "section, key",
        "ord, lamport, device_id, seq",
        "section, key, op_hash, ord, lamport, device_id, seq, value",
    )?;
    collapse_tombstones(conn)?;
    forget_dead_entities(conn, now)?;

    // Credentials that were resharded keep no wrapped shares of the old op.
    conn.execute(
        "UPDATE sync_cred_state SET wrapped_shares = '[]' WHERE lower(hex(host_uid)) IN
            (SELECT json_extract(value, '$.host_uid') FROM sync_writes WHERE section = 'share')",
        [],
    )?;
    conn.execute(
        "UPDATE sync_writes SET value = json_set(value, '$.shares', json('[]'))
         WHERE section = 'cred' AND key IN (SELECT json_extract(value, '$.host_uid') FROM sync_writes WHERE section = 'share')",
        [],
    )?;
    conn.execute(
        "DELETE FROM sync_head_attests WHERE seq < (SELECT MAX(h.seq) FROM sync_head_attests h WHERE h.device_id = sync_head_attests.device_id)",
        [],
    )?;
    conn.execute(
        "DELETE FROM sync_wrap_history WHERE wrap_epoch < (SELECT wrap_epoch FROM sync_devices d WHERE d.device_id = sync_wrap_history.device_id)",
        [],
    )?;
    report.ops_deleted = conn.execute(
        "DELETE FROM sync_ops WHERE op_hash IN (SELECT op_hash FROM _pruned)",
        [],
    )? as u64;
    conn.execute(
        "DELETE FROM sync_pending_ops WHERE op_hash IN (SELECT op_hash FROM _pruned)",
        [],
    )?;
    conn.execute_batch("DROP TABLE _pruned;")?;

    for c in &older.cut {
        conn.execute(
            "INSERT OR REPLACE INTO sync_prune_floor (device_id, seq, op_hash) VALUES (?1, ?2, ?3)",
            params![c.device_id, c.seq as i64, &c.op_hash[..]],
        )?;
    }
    // Checkpoints older than the floor checkpoint are history.
    for c in accepted
        .iter()
        .filter(|c| c.accepted_at < older.accepted_at)
    {
        set_status(conn, &c.proposal_hash, CheckpointStatus::Pruned)?;
    }
    // Only the newest state blob is kept: an older one still holds the shares
    // that were live then, and a device that is behind adopts the newest.
    conn.execute(
        "UPDATE sync_checkpoints SET state = NULL WHERE proposal_hash != ?1",
        params![&newest.proposal_hash[..]],
    )?;
    report.bytes_freed = size_before.saturating_sub(db_bytes(conn)?);
    Ok(report)
}

fn db_bytes(conn: &Connection) -> Result<u64> {
    let pages: i64 = conn.query_row("PRAGMA page_count", [], |r| r.get(0))?;
    let size: i64 = conn.query_row("PRAGMA page_size", [], |r| r.get(0))?;
    let free: i64 = conn.query_row("PRAGMA freelist_count", [], |r| r.get(0))?;
    Ok(((pages - free) * size) as u64)
}

/// For every key, replace the effective writes of pruned ops (and the old base)
/// by one base write holding the winner; drop the non-effective ones. Returns
/// the number of rows removed.
fn collapse(
    conn: &Connection,
    table: &str,
    key_cols: &str,
    ord_cols: &str,
    all_cols: &str,
) -> Result<u64> {
    let cols: Vec<&str> = all_cols.split(", ").collect();
    let sel = cols
        .iter()
        .map(|c| format!("w.{c}"))
        .collect::<Vec<_>>()
        .join(", ");
    let keys: Vec<&str> = key_cols.split(", ").collect();
    let order = ord_cols
        .split(", ")
        .map(|c| format!("w.{c}"))
        .collect::<Vec<_>>()
        .join(", ");
    let q = format!(
        "SELECT {sel}, o.state FROM {table} w JOIN _pruned p ON p.op_hash = w.op_hash
           LEFT JOIN sync_ops o ON o.op_hash = w.op_hash
         UNION ALL
         SELECT {sel}, 'applied' FROM {table} w WHERE w.op_hash = zeroblob(32)
         ORDER BY {order}"
    );
    let mut stmt = conn.prepare(&q)?;
    let n = cols.len();
    let rows: Vec<(Vec<rusqlite::types::Value>, String)> = stmt
        .query_map([], |r| {
            let mut v = Vec::new();
            for i in 0..n {
                v.push(r.get::<_, rusqlite::types::Value>(i)?);
            }
            Ok((v, r.get::<_, String>(n)?))
        })?
        .collect::<std::result::Result<_, _>>()?;
    drop(stmt);
    let key_idx: Vec<usize> = keys
        .iter()
        .map(|k| cols.iter().position(|c| c == k).expect("key col"))
        .collect();
    let hash_idx = cols
        .iter()
        .position(|c| *c == "op_hash")
        .expect("op_hash col");
    let mut winners: BTreeMap<Vec<String>, Vec<rusqlite::types::Value>> = BTreeMap::new();
    for (row, state) in &rows {
        if state != "applied" && state != "approved" {
            continue;
        }
        let k: Vec<String> = key_idx.iter().map(|i| format!("{:?}", row[*i])).collect();
        winners.insert(k, row.clone());
    }
    let mut removed = 0u64;
    for (row, _) in &rows {
        let where_ = keys
            .iter()
            .enumerate()
            .map(|(i, k)| format!("{k} = ?{}", i + 1))
            .collect::<Vec<_>>()
            .join(" AND ");
        let mut p: Vec<rusqlite::types::Value> = key_idx.iter().map(|i| row[*i].clone()).collect();
        p.push(row[hash_idx].clone());
        removed += conn.execute(
            &format!(
                "DELETE FROM {table} WHERE {where_} AND op_hash = ?{}",
                keys.len() + 1
            ),
            rusqlite::params_from_iter(p),
        )? as u64;
    }
    for mut row in winners.into_values() {
        row[hash_idx] = rusqlite::types::Value::Blob(vec![0; 32]);
        let ph = (1..=n)
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        conn.execute(
            &format!("INSERT OR REPLACE INTO {table} ({all_cols}) VALUES ({ph})"),
            rusqlite::params_from_iter(row),
        )?;
        removed = removed.saturating_sub(1);
    }
    Ok(removed)
}

fn collapse_tombstones(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT t.kind, t.uid, t.lamport, t.device_id, t.ts, t.op_hash, o.state
         FROM sync_tombstones t LEFT JOIN sync_ops o ON o.op_hash = t.op_hash
         WHERE t.op_hash IN (SELECT op_hash FROM _pruned) OR t.op_hash = zeroblob(32)
         ORDER BY t.lamport, t.device_id",
    )?;
    type TombRow = (String, Vec<u8>, i64, String, i64, Vec<u8>, Option<String>);
    let rows: Vec<TombRow> = stmt
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        })?
        .collect::<std::result::Result<_, _>>()?;
    drop(stmt);
    let mut winners: BTreeMap<(String, Vec<u8>), (i64, String, i64)> = BTreeMap::new();
    for (kind, uid, lamport, device, ts, hash, state) in &rows {
        conn.execute(
            "DELETE FROM sync_tombstones WHERE kind = ?1 AND uid = ?2 AND op_hash = ?3",
            params![kind, uid, hash],
        )?;
        let effective = *hash == ZERO
            || state
                .as_deref()
                .is_some_and(|s| s == "applied" || s == "approved");
        if effective {
            winners.insert((kind.clone(), uid.clone()), (*lamport, device.clone(), *ts));
        }
    }
    for ((kind, uid), (lamport, device, ts)) in winners {
        conn.execute(
            "INSERT OR REPLACE INTO sync_tombstones (kind, uid, lamport, device_id, op_hash, ts)
             VALUES (?1, ?2, ?3, ?4, zeroblob(32), ?5)",
            params![kind, uid, lamport, device, ts],
        )?;
    }
    Ok(())
}

/// Deleted entities past the tombstone horizon: forget fields and tombstone.
fn forget_dead_entities(conn: &Connection, now: i64) -> Result<()> {
    let folds = fold_entities(conn, None)?;
    for ((kind, uid), e) in folds {
        if entity_alive(&e) {
            continue;
        }
        if e.tomb
            .as_ref()
            .is_some_and(|t| t.3 < now - TOMBSTONE_KEEP_SECS)
        {
            conn.execute(
                "DELETE FROM sync_fields WHERE kind = ?1 AND uid = ?2 AND op_hash = zeroblob(32)",
                params![kind, &uid[..]],
            )?;
            conn.execute("DELETE FROM sync_tombstones WHERE kind = ?1 AND uid = ?2 AND op_hash = zeroblob(32)", params![kind, &uid[..]])?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- fingerprint

/// 8 BIP39 words (88 bits) of BLAKE3 over the canonical `devices` section.
pub fn ring_fingerprint(state: &[u8]) -> Result<String> {
    let v: Value = serde_json::from_slice(state).context("state is not JSON")?;
    let devices = v.get("devices").context("state has no devices section")?;
    let digest = chain_hash(&serde_json::to_vec(devices)?);
    let words = bip39::Language::English.word_list();
    let mut bits = 0u128;
    for b in &digest[..11] {
        bits = (bits << 8) | u128::from(*b);
    }
    Ok((0..RING_FINGERPRINT_WORDS)
        .map(|i| words[((bits >> (11 * (RING_FINGERPRINT_WORDS - 1 - i))) & 0x7ff) as usize])
        .collect::<Vec<_>>()
        .join(" "))
}

// ------------------------------------------------------------------ bootstrap

pub(crate) fn bootstrap(
    conn: &Connection,
    bundle: &CheckpointBundle,
    trust: &BootstrapTrust,
    now: i64,
) -> Result<()> {
    let proposal = &bundle.proposal;
    let (cut, state_hash, state_len, _prev, required) = propose_parts(proposal)?;
    ensure!(
        chain_hash(&bundle.state) == *state_hash && bundle.state.len() as u64 == state_len,
        "the state blob does not match the proposal"
    );
    let state: Value = serde_json::from_slice(&bundle.state).context("state is not JSON")?;
    let devices = state["devices"].as_array().cloned().unwrap_or_default();
    let key_of_device = |id: &str| -> Option<[u8; 32]> {
        let d = devices.iter().find(|d| d["id"] == id)?;
        let h = d["ed25519"].as_str()?;
        let mut out = [0u8; 32];
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(h.get(2 * i..2 * i + 2)?, 16).ok()?;
        }
        Some(out)
    };
    ensure!(
        key_of_device(&proposal.device_id).is_some_and(|k| proposal.verify(&k)),
        "the proposal is not signed by a device of its own table"
    );
    // The cut in the state is the cut in the proposal.
    let state_cut = cut_from_state(&state)?;
    ensure!(
        state_cut == *cut,
        "the state's cut differs from the proposal's"
    );
    // Required set from the blob's own table, and every one of them signed.
    let want = required_of(&devices, proposal.timestamp);
    ensure!(
        want == *required,
        "the proposal's required set does not match the state's device table"
    );
    let proposal_hash = proposal.hash();
    let mut signers = BTreeSet::new();
    for s in &bundle.signs {
        let OpBody::CheckpointSign { proposal_hash: ph } = &s.body else {
            continue;
        };
        if *ph == proposal_hash && key_of_device(&s.device_id).is_some_and(|k| s.verify(&k)) {
            signers.insert(s.device_id.clone());
        }
    }
    ensure!(
        required.iter().all(|r| signers.contains(r)),
        "not every required device signed"
    );
    ensure!(
        signers.len() >= CHECKPOINT_SIGN_MIN,
        "fewer than {CHECKPOINT_SIGN_MIN} signers"
    );
    // Chain anchors: the head ops of the cut.
    for c in cut {
        let head = bundle
            .heads
            .iter()
            .find(|h| h.device_id == c.device_id && h.seq == c.seq)
            .with_context(|| format!("head op of {} is missing", c.device_id))?;
        ensure!(
            head.hash() == c.op_hash,
            "head op of {} does not match the cut",
            c.device_id
        );
        ensure!(
            key_of_device(&head.device_id).is_some_and(|k| head.verify(&k)),
            "head op of {} is not signed by its device",
            c.device_id
        );
    }
    // The trust anchor, separate from the signatures.
    match trust {
        BootstrapTrust::RingFingerprint(words) => {
            let norm = |s: &str| {
                s.split_whitespace()
                    .map(str::to_lowercase)
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            ensure!(
                norm(words) == ring_fingerprint(&bundle.state)?,
                "the ring fingerprint does not match"
            );
        }
        BootstrapTrust::Anchor(id) => {
            ensure!(
                signers.contains(id),
                "the anchor device did not sign this checkpoint"
            );
        }
    }
    let have_ops: i64 = conn.query_row("SELECT COUNT(*) FROM sync_ops", [], |r| r.get(0))?;
    ensure!(have_ops == 0, "bootstrap needs a store without op history");

    // Heads: anchors of the chains, never applied.
    for h in &bundle.heads {
        let hash = insert_op(conn, h, State::Void, None, false, now)?;
        conn.execute(
            "UPDATE sync_ops SET reason = 'checkpoint-anchor' WHERE op_hash = ?1",
            params![&hash[..]],
        )?;
        conn.execute(
            "DELETE FROM sync_pending_ops WHERE op_hash = ?1",
            params![&hash[..]],
        )?;
    }
    for c in cut {
        conn.execute(
            "INSERT OR REPLACE INTO sync_prune_floor (device_id, seq, op_hash) VALUES (?1, ?2, ?3)",
            params![c.device_id, c.seq as i64, &c.op_hash[..]],
        )?;
    }
    for e in state["epochs"].as_array().unwrap_or(&Vec::new()) {
        conn.execute(
            "INSERT OR REPLACE INTO sync_epochs (device_id, epoch) VALUES (?1, ?2)",
            params![
                e["device_id"].as_str().unwrap_or(""),
                e["epoch"].as_i64().unwrap_or(0)
            ],
        )?;
    }
    for w in state["writes"].as_array().unwrap_or(&Vec::new()) {
        conn.execute(
            "INSERT OR REPLACE INTO sync_writes (section, key, op_hash, ord, lamport, device_id, seq, value)
             VALUES (?1, ?2, zeroblob(32), ?3, ?4, ?5, ?6, ?7)",
            params![
                w["section"].as_str().unwrap_or(""),
                w["key"].as_str().unwrap_or(""),
                w["ord"].as_i64().unwrap_or(0),
                w["lamport"].as_i64().unwrap_or(0),
                w["device_id"].as_str().unwrap_or(""),
                w["seq"].as_i64().unwrap_or(0),
                serde_json::to_string(&w["value"])?,
            ],
        )?;
    }
    let mut touched = BTreeSet::new();
    for e in state["entities"].as_array().unwrap_or(&Vec::new()) {
        let kind = EntityKind::parse(e["kind"].as_str().unwrap_or("")).context("entity kind")?;
        let uid = uid_from_hex(e["uid"].as_str().unwrap_or("")).context("entity uid")?;
        for (field, w) in e["fields"].as_object().unwrap_or(&Map::new()) {
            conn.execute(
                "INSERT OR REPLACE INTO sync_fields (kind, uid, field, op_hash, lamport, device_id, seq, value)
                 VALUES (?1, ?2, ?3, zeroblob(32), ?4, ?5, ?6, ?7)",
                params![
                    kind.as_str(),
                    &uid[..],
                    field,
                    w["lamport"].as_i64().unwrap_or(0),
                    w["device_id"].as_str().unwrap_or(""),
                    w["seq"].as_i64().unwrap_or(0),
                    serde_json::to_string(&w["value"])?,
                ],
            )?;
        }
        if let Some(t) = e.get("tombstone") {
            conn.execute(
                "INSERT OR REPLACE INTO sync_tombstones (kind, uid, lamport, device_id, op_hash, ts)
                 VALUES (?1, ?2, ?3, ?4, zeroblob(32), ?5)",
                params![
                    kind.as_str(),
                    &uid[..],
                    t["lamport"].as_i64().unwrap_or(0),
                    t["device_id"].as_str().unwrap_or(""),
                    t["ts"].as_i64().unwrap_or(0),
                ],
            )?;
        }
        touched.insert((kind, uid));
    }
    for f in state["forks"].as_array().unwrap_or(&Vec::new()) {
        let (Some(a), Some(b)) = (
            f["a"].as_str().and_then(b64_decode),
            f["b"].as_str().and_then(b64_decode),
        ) else {
            continue;
        };
        conn.execute(
            "UPDATE sync_devices SET status = 'compromised', fork_a = ?2, fork_b = ?3 WHERE device_id = ?1",
            params![f["device_id"].as_str().unwrap_or(""), a, b],
        )?;
    }
    rebuild_caches(conn, now)?;
    reproject(conn, &touched)?;

    // Record the checkpoint itself as accepted.
    conn.execute(
        "INSERT OR REPLACE INTO sync_checkpoints (proposal_hash, device_id, prev, state_hash, state_len, cut,
             required, status, accepted_at, lamport, proposed_at, op_blob, state)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'accepted', ?8, ?9, ?10, ?11, ?12)",
        params![
            &proposal_hash[..],
            proposal.device_id,
            &propose_parts(proposal)?.3[..],
            &state_hash[..],
            state_len as i64,
            cut_json(cut),
            serde_json::to_string(required)?,
            now,
            proposal.lamport as i64,
            proposal.timestamp,
            op_blob(proposal),
            bundle.state,
        ],
    )?;
    for s in &bundle.signs {
        conn.execute(
            "INSERT OR IGNORE INTO sync_checkpoint_signs (proposal_hash, device_id, sign_op_hash, op_blob)
             VALUES (?1, ?2, ?3, ?4)",
            params![&proposal_hash[..], s.device_id, &s.hash()[..], op_blob(s)],
        )?;
    }
    Ok(())
}

fn cut_from_state(state: &Value) -> Result<Vec<CutEntry>> {
    state["cut"]
        .as_array()
        .context("state has no cut")?
        .iter()
        .map(|c| {
            let h = c["op_hash"].as_str().context("op_hash")?;
            let mut op_hash = [0u8; 32];
            for (i, b) in op_hash.iter_mut().enumerate() {
                *b = u8::from_str_radix(h.get(2 * i..2 * i + 2).context("hash length")?, 16)?;
            }
            Ok(CutEntry {
                device_id: c["device_id"].as_str().context("device")?.to_string(),
                seq: c["seq"].as_u64().context("seq")?,
                op_hash,
                lamport: c["lamport"].as_u64().context("lamport")?,
            })
        })
        .collect()
}

/// Rebuild the cache tables stage 3 reads from the winning writes (after a
/// bootstrap, where no op ever ran the effects).
fn rebuild_caches(conn: &Connection, now: i64) -> Result<()> {
    let wins = fold_writes(conn, None)?;
    let unhex = |s: &str| -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap_or(0))
            .collect()
    };
    for ((section, key), w) in &wins {
        let v = &w.value;
        match section.as_str() {
            "device" => {
                let wrap = wins.get(&("wrap".to_string(), key.clone()));
                let (x, ml, we) = match wrap {
                    Some(wr) => (
                        unhex(wr.value["x25519"].as_str().unwrap_or("")),
                        b64_decode(wr.value["mlkem"].as_str().unwrap_or("")).unwrap_or_default(),
                        wr.ord as i64,
                    ),
                    None => (
                        unhex(v["x25519"].as_str().unwrap_or("")),
                        b64_decode(v["mlkem"].as_str().unwrap_or("")).unwrap_or_default(),
                        0,
                    ),
                };
                let revoked = wins.contains_key(&("revoke".to_string(), key.clone()));
                conn.execute(
                    "INSERT OR REPLACE INTO sync_devices
                       (device_id, ed25519, x25519, mlkem, name, role, attendance, kind, status, wrap_epoch, verified_by, last_seen)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                    params![
                        key,
                        unhex(v["ed25519"].as_str().unwrap_or("")),
                        x,
                        ml,
                        v["name"].as_str().unwrap_or(""),
                        wins.get(&("role".to_string(), key.clone()))
                            .and_then(|r| r.value["role"].as_str())
                            .or(v["role"].as_str())
                            .unwrap_or("mirror"),
                        v["attendance"].as_str().unwrap_or("attended"),
                        v["kind"].as_str().unwrap_or("terminal"),
                        if revoked { "revoked" } else { "operational" },
                        we,
                        v["verified_by"].as_str().unwrap_or("sas"),
                        now,
                    ],
                )?;
            }
            "cred" => {
                let shares: Vec<Value> = v["shares"].as_array().cloned().unwrap_or_default();
                conn.execute(
                    "INSERT OR REPLACE INTO sync_cred_state (host_uid, ciphertext, wrapped_shares, lamport, device_id, op_hash)
                     VALUES (?1, ?2, ?3, ?4, ?5, zeroblob(32))",
                    params![
                        unhex(key),
                        b64_decode(v["ciphertext"].as_str().unwrap_or("")).unwrap_or_default(),
                        serde_json::to_string(&shares)?,
                        w.lamport as i64,
                        w.device
                    ],
                )?;
            }
            "mode" => {
                conn.execute(
                    "INSERT OR REPLACE INTO sync_meta (key, value) VALUES ('cred_mode', ?1)",
                    params![v["to"].as_str().unwrap_or("vault").as_bytes()],
                )?;
                conn.execute(
                    "INSERT OR REPLACE INTO sync_meta (key, value) VALUES ('cred_mode_key', ?1)",
                    params![format!("{:020}:{}", w.lamport, w.device).as_bytes()],
                )?;
            }
            "cert" => {
                conn.execute(
                    "INSERT OR REPLACE INTO sync_delegation_certs
                       (cert_id, device_id, scope, expiry, max_new_hosts_per_day, epoch_lo, epoch_hi, issuer, revoked)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        unhex(key),
                        v["device_id"].as_str().unwrap_or(""),
                        serde_json::to_string(&v["scope"])?,
                        v["expiry"].as_i64().unwrap_or(0),
                        v["max_new_hosts_per_day"].as_i64().unwrap_or(0),
                        v["epoch_lo"].as_i64().unwrap_or(0),
                        v["epoch_hi"].as_i64().unwrap_or(0),
                        v["issuer"].as_str().unwrap_or(""),
                        wins.contains_key(&("cert_revoked".to_string(), key.clone())),
                    ],
                )?;
            }
            "cert_revoked" => {
                conn.execute(
                    "INSERT OR REPLACE INTO sync_meta (key, value) VALUES (?1, x'01')",
                    params![format!("cert-revoked:{key}")],
                )?;
            }
            "cert_open" => {
                if let Some((cert, host)) = key.split_once(':') {
                    conn.execute(
                        "INSERT OR REPLACE INTO sync_cert_hosts_opened (cert_id, host_uid, opened_at) VALUES (?1, ?2, ?3)",
                        params![unhex(cert), unhex(host), v["opened_at"].as_i64().unwrap_or(0)],
                    )?;
                }
            }
            "head" => {
                conn.execute(
                    "INSERT OR REPLACE INTO sync_head_attests (device_id, seq, chain_hash, signature)
                     VALUES (?1, ?2, ?3, zeroblob(64))",
                    params![key, w.ord as i64, unhex(v["hash"].as_str().unwrap_or(""))],
                )?;
            }
            "wrap" => {
                conn.execute(
                    "INSERT OR REPLACE INTO sync_wrap_history (device_id, wrap_epoch, x25519, mlkem) VALUES (?1, ?2, ?3, ?4)",
                    params![
                        key,
                        w.ord as i64,
                        unhex(v["x25519"].as_str().unwrap_or("")),
                        b64_decode(v["mlkem"].as_str().unwrap_or("")).unwrap_or_default()
                    ],
                )?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Live reshare envelopes of one credential.
pub(crate) fn share_envelopes(
    conn: &Connection,
    host: &Uid,
) -> Result<Vec<(DeviceId, u64, Vec<u8>)>> {
    let hex_host = uid_hex(host);
    let wins = fold_writes(conn, None)?;
    Ok(wins
        .iter()
        .filter(|((s, _), w)| s == "share" && w.value["host_uid"] == hex_host.as_str())
        .filter_map(|(_, w)| {
            Some((
                w.value["holder"].as_str()?.to_string(),
                w.ord,
                b64_decode(w.value["sealed"].as_str()?)?,
            ))
        })
        .collect())
}

pub(crate) fn storage_key(conn: &Connection, device: &str) -> Result<Option<(u64, Vec<u8>)>> {
    let wins = fold_writes(conn, None)?;
    Ok(wins
        .get(&("storage".to_string(), device.to_string()))
        .and_then(|w| Some((w.ord, b64_decode(w.value["sealed"].as_str()?)?))))
}
