//! `LauncherStore::sync_*`: the op-log surface of the store (contract section 2).
//! Every method is one short transaction; nothing here touches the network.

use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

use super::fork::{self, ForkEvidence, HeadVerdict, SignedHead};
use super::ingest::{ingest, max_lamport};
use super::merge::{attendance_db, insert_op, load_op, role_db, Settler, State};
use super::op::*;
use super::pending;
use crate::store::{now_ts, HostUid, LauncherStore};
use crate::sync::apply::{self, reproject, validate_body};
use crate::sync::crypto::{DeviceIdentity, DevicePublic, ALG_CURRENT};
use crate::sync::limits::{BEHIND_AFTER_SECS, MAX_OPS_PER_TAIL, STALE_AFTER_SECS};

#[derive(Debug)]
pub enum IngestOutcome {
    Applied,
    Pending,
    Rejected(String),
    ForkProof(Box<Op>, Box<Op>),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeviceStatus {
    Operational,
    Behind,
    Stale,
    Suspect,
    Compromised,
    Revoked,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DeviceRow {
    pub id: DeviceId,
    pub public: DevicePublic,
    pub name: String,
    pub role: DeviceRole,
    pub attendance: Attendance,
    pub status: DeviceStatus,
    pub exempt: bool,
    pub last_seen: i64,
    pub head_seq: u64,
}

fn tx(conn: &Connection) -> Result<Transaction<'_>> {
    Ok(Transaction::new_unchecked(
        conn,
        TransactionBehavior::Immediate,
    )?)
}

fn register(
    conn: &Connection,
    public: &DevicePublic,
    name: &str,
    role: DeviceRole,
    attendance: Attendance,
    now: i64,
) -> Result<DeviceId> {
    public.validate().context("invalid device key")?;
    let id = public.device_id();
    conn.execute(
        "INSERT OR IGNORE INTO sync_devices
           (device_id, ed25519, x25519, mlkem, name, role, attendance, last_seen)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            id,
            &public.ed25519_pub[..],
            &public.x25519_pub[..],
            public.mlkem_pub,
            name,
            role_db(role),
            attendance_db(attendance),
            now
        ],
    )?;
    Ok(id)
}

impl LauncherStore {
    /// Trust a device directly: the result of an enrollment ceremony (SAS) or
    /// the first entry of a fresh ring. Later devices arrive as `DeviceAdd` ops.
    pub fn sync_register_device(
        &self,
        public: &DevicePublic,
        name: &str,
        role: DeviceRole,
        attendance: Attendance,
    ) -> Result<DeviceId> {
        self.with_conn(|conn| {
            let tx = tx(conn)?;
            let id = register(&tx, public, name, role, attendance, now_ts())?;
            tx.commit()?;
            Ok(id)
        })
    }

    /// Chain, sign and apply a local change. The op is ours, so it is approved
    /// by definition; its effects are projected like any other.
    pub fn sync_append_local(&self, id: &DeviceIdentity, body: OpBody) -> Result<Op> {
        validate_body(&body).map_err(|e| anyhow!("invalid op: {e}"))?;
        self.with_conn(|conn| {
            let tx = tx(conn)?;
            let now = now_ts();
            let public = id.public();
            let device = register(
                &tx,
                &public,
                "this device",
                DeviceRole::Holder,
                Attendance::Attended,
                now,
            )?;
            let head: Option<(i64, Vec<u8>)> = tx
                .query_row(
                    "SELECT seq, op_hash FROM sync_chain_heads WHERE device_id = ?1",
                    params![device],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let (seq, prev_hash) = match head {
                Some((s, h)) => (
                    s as u64 + 1,
                    <[u8; 32]>::try_from(h).map_err(|_| anyhow!("stored hash"))?,
                ),
                None => (1, [0u8; 32]),
            };
            let epoch: Option<i64> = tx
                .query_row(
                    "SELECT epoch FROM sync_epochs WHERE device_id = ?1",
                    params![device],
                    |r| r.get(0),
                )
                .optional()?;
            let mut op = Op {
                device_id: device.clone(),
                seq,
                lamport: max_lamport(&tx)? + 1,
                prev_hash,
                timestamp: now,
                epoch: epoch.unwrap_or(0) as u64,
                alg: ALG_CURRENT,
                body,
                signature: [0u8; 64],
            };
            op.signature = id.sign(&op.canonical_bytes());
            let hash = insert_op(&tx, &op, State::Rejected, None, true, now)?;
            let row = load_op(&tx, &hash)?.context("just inserted")?;
            let mut settler = Settler::new(&tx, now);
            settler.transition(&row, State::Approved, None)?;
            settler.run()?;
            reproject(&tx, &settler.touched)?;
            tx.execute(
                "UPDATE sync_devices SET last_seen = ?2 WHERE device_id = ?1",
                params![device, now],
            )?;
            tx.commit()?;
            Ok(op)
        })
    }

    /// Ops of `device` after `after_seq`, oldest first, at most one tail frame.
    pub fn sync_chain_tail(&self, device: &DeviceId, after_seq: u64) -> Result<Vec<Op>> {
        self.with_conn(|conn| {
            let after = i64::try_from(after_seq).unwrap_or(i64::MAX);
            let mut stmt = conn.prepare(
                "SELECT op_hash FROM sync_ops WHERE device_id = ?1 AND seq > ?2 ORDER BY seq LIMIT ?3",
            )?;
            let hashes: Vec<Vec<u8>> = stmt
                .query_map(params![device, after, MAX_OPS_PER_TAIL as i64], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?;
            hashes
                .into_iter()
                .map(|h| {
                    let h: [u8; 32] = h.try_into().map_err(|_| anyhow!("stored hash"))?;
                    Ok(load_op(conn, &h)?.context("op vanished")?.op)
                })
                .collect()
        })
    }

    /// Verify, chain and apply (or quarantine) a foreign op.
    pub fn sync_ingest(&self, op: Op) -> Result<IngestOutcome> {
        self.with_conn(|conn| {
            let tx = tx(conn)?;
            let outcome = ingest(&tx, op, now_ts())?;
            tx.commit()?;
            Ok(outcome)
        })
    }

    /// Ops waiting for review, oldest first.
    pub fn sync_pending(&self) -> Result<Vec<Op>> {
        self.with_conn(pending::list)
    }

    /// Why an op is waiting: `tier`, `resurrection` or `revoke-conflict`.
    pub fn sync_pending_reason(&self, op_hash: &[u8; 32]) -> Result<Option<String>> {
        self.with_conn(|conn| pending::reason(conn, op_hash))
    }

    pub fn sync_resolve_pending(&self, op_hash: &[u8; 32], approve: bool) -> Result<()> {
        self.with_conn(|conn| {
            let tx = tx(conn)?;
            pending::resolve(&tx, op_hash, approve, now_ts())?;
            tx.commit()?;
            Ok(())
        })
    }

    pub fn sync_devices(&self) -> Result<Vec<DeviceRow>> {
        self.with_conn(|conn| {
            let now = now_ts();
            let max_epoch: i64 = conn.query_row("SELECT COALESCE(MAX(epoch), 0) FROM sync_epochs", [], |r| r.get(0))?;
            let mut stmt = conn.prepare(
                "SELECT d.device_id, d.ed25519, d.x25519, d.mlkem, d.name, d.role, d.attendance,
                        d.status, d.exempt, d.last_seen,
                        COALESCE((SELECT seq FROM sync_chain_heads h WHERE h.device_id = d.device_id), 0),
                        COALESCE((SELECT epoch FROM sync_epochs e WHERE e.device_id = d.device_id), 0)
                 FROM sync_devices d ORDER BY d.device_id",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Vec<u8>>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                    r.get::<_, Vec<u8>>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, bool>(8)?,
                    r.get::<_, i64>(9)?,
                    r.get::<_, i64>(10)?,
                    r.get::<_, i64>(11)?,
                ))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (id, ed, x, mlkem, name, role, att, status, exempt, last_seen, head, epoch) = row?;
                let status = match status.as_str() {
                    "compromised" => DeviceStatus::Compromised,
                    "revoked" => DeviceStatus::Revoked,
                    "suspect" => DeviceStatus::Suspect,
                    _ if now - last_seen >= STALE_AFTER_SECS => DeviceStatus::Stale,
                    _ if now - last_seen >= BEHIND_AFTER_SECS || max_epoch - epoch >= 1 => {
                        DeviceStatus::Behind
                    }
                    _ => DeviceStatus::Operational,
                };
                out.push(DeviceRow {
                    id,
                    public: DevicePublic {
                        ed25519_pub: ed.try_into().map_err(|_| anyhow!("stored key"))?,
                        x25519_pub: x.try_into().map_err(|_| anyhow!("stored key"))?,
                        mlkem_pub: mlkem,
                    },
                    name,
                    role: if role == "holder" { DeviceRole::Holder } else { DeviceRole::Mirror },
                    attendance: if att == "attended" { Attendance::Attended } else { Attendance::Unattended },
                    status,
                    exempt,
                    last_seen,
                    head_seq: head as u64,
                });
            }
            Ok(out)
        })
    }

    pub fn sync_cred_mode(&self) -> Result<CredMode> {
        self.with_conn(|conn| {
            let v: Option<Vec<u8>> = conn
                .query_row(
                    "SELECT value FROM sync_meta WHERE key = 'cred_mode'",
                    [],
                    |r| r.get(0),
                )
                .optional()?;
            Ok(match v.as_deref() {
                Some(b"quorum") => CredMode::Quorum,
                _ => CredMode::Vault,
            })
        })
    }

    pub fn sync_cred_ciphertext(
        &self,
        host: &HostUid,
    ) -> Result<Option<(Vec<u8>, Vec<WrappedShare>)>> {
        self.with_conn(|conn| {
            let row: Option<(Vec<u8>, String)> = conn
                .query_row(
                    "SELECT ciphertext, wrapped_shares FROM sync_cred_state WHERE host_uid = ?1",
                    params![&host[..]],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let Some((ct, shares)) = row else {
                return Ok(None);
            };
            let v: Vec<serde_json::Value> = serde_json::from_str(&shares)?;
            let shares = v
                .iter()
                .map(|s| {
                    let alg: [u8; 2] = serde_json::from_value(s["alg"].clone())?;
                    Ok(WrappedShare {
                        recipient: s["recipient"].as_str().context("recipient")?.to_string(),
                        alg: crate::sync::crypto::AlgTag::from_bytes(alg)?,
                        sealed: serde_json::from_value(s["sealed"].clone())?,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(Some((ct, shares)))
        })
    }

    pub fn sync_cert_revoked(&self, cert_id: &[u8; 32]) -> Result<bool> {
        self.with_conn(|conn| {
            Ok(conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM sync_meta WHERE key = ?1)",
                params![format!("cert-revoked:{}", super::merge::hex(cert_id))],
                |r| r.get(0),
            )?)
        })
    }

    pub fn sync_cert_hosts_opened(&self, cert_id: &[u8; 32]) -> Result<Vec<HostUid>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT host_uid FROM sync_cert_hosts_opened WHERE cert_id = ?1 ORDER BY host_uid",
            )?;
            let rows: Vec<Vec<u8>> = stmt
                .query_map(params![&cert_id[..]], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?;
            Ok(rows
                .into_iter()
                .filter_map(|u| HostUid::try_from(u).ok())
                .collect())
        })
    }

    pub fn sync_cert_new_hosts_since(&self, cert_id: &[u8; 32], since: i64) -> Result<u32> {
        self.with_conn(|conn| {
            let n: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sync_cert_hosts_opened WHERE cert_id = ?1 AND opened_at >= ?2",
                params![&cert_id[..], since],
                |r| r.get(0),
            )?;
            Ok(u32::try_from(n).unwrap_or(u32::MAX))
        })
    }

    /// Newest stored head claim for `device`.
    pub fn sync_head_attest(&self, device: &DeviceId) -> Result<Option<HeadAttest>> {
        self.with_conn(|conn| {
            let row: Option<(i64, Vec<u8>)> = conn
                .query_row(
                    "SELECT seq, chain_hash FROM sync_head_attests WHERE device_id = ?1
                     ORDER BY seq DESC LIMIT 1",
                    params![device],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            Ok(row.and_then(|(seq, h)| {
                Some(HeadAttest {
                    device_id: device.clone(),
                    seq: seq as u64,
                    chain_hash: h.try_into().ok()?,
                })
            }))
        })
    }

    pub fn sync_epoch(&self, device: &DeviceId) -> Result<u64> {
        self.with_conn(|conn| {
            let e: Option<i64> = conn
                .query_row(
                    "SELECT epoch FROM sync_epochs WHERE device_id = ?1",
                    params![device],
                    |r| r.get(0),
                )
                .optional()?;
            Ok(e.unwrap_or(0) as u64)
        })
    }

    /// `(device, seq, hash)` of every chain head we hold.
    pub fn sync_chain_heads(&self) -> Result<Vec<(DeviceId, u64, [u8; 32])>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT device_id, seq, op_hash FROM sync_chain_heads ORDER BY device_id",
            )?;
            let rows: Vec<(String, i64, Vec<u8>)> = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<std::result::Result<_, _>>()?;
            rows.into_iter()
                .map(|(d, s, h)| {
                    Ok((
                        d,
                        s as u64,
                        h.try_into().map_err(|_| anyhow!("stored hash"))?,
                    ))
                })
                .collect()
        })
    }

    /// A peer's self-signed head: stored, and checked against the chain.
    pub fn sync_record_head(&self, head: &SignedHead) -> Result<HeadVerdict> {
        self.with_conn(|conn| {
            let tx = tx(conn)?;
            let v = fork::record_head(&tx, head)?;
            tx.commit()?;
            Ok(v)
        })
    }

    pub fn sync_fork_evidence(&self, device: &DeviceId) -> Result<Option<ForkEvidence>> {
        self.with_conn(|conn| fork::evidence(conn, device))
    }

    /// Gossip disagrees about a third device: ask it directly next time.
    pub fn sync_mark_suspect(&self, device: &DeviceId) -> Result<()> {
        self.with_conn(|conn| fork::mark_suspect(conn, device))
    }

    /// Hash of the logical replicated state (registers and liveness).
    pub fn sync_state_hash(&self) -> Result<[u8; 32]> {
        self.with_conn(apply::state_hash)
    }

    /// Hash of the replicated columns of the projected rows.
    pub fn sync_projection_hash(&self) -> Result<[u8; 32]> {
        self.with_conn(apply::projection_hash)
    }

    /// Readable form of what [`Self::sync_projection_hash`] hashes (diagnostics).
    pub fn sync_projection_dump(&self) -> Result<String> {
        self.with_conn(apply::projection_dump)
    }

    /// Host hop tokens as typed locally to the wire form (`#uid`).
    pub fn sync_hops_to_wire(&self, local: &str) -> Result<String> {
        self.with_conn(|conn| apply::hops_to_wire(conn, local))
    }
}
