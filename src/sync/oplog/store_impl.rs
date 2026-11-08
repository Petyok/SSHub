//! `LauncherStore::sync_*`: the op-log surface of the store (contract section 2).
//! Every method is one short transaction; nothing here touches the network.

use anyhow::{anyhow, ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

use super::checkpoint::{
    self, BootstrapTrust, CheckpointBundle, CheckpointRow, CheckpointStatus, CheckpointVerdict,
    PruneReport,
};
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
    /// Generation of the current wrap keys (C14).
    pub wrap_epoch: u64,
    pub kind: DeviceKind,
    /// How the device was admitted.
    pub verified_by: VerifiedBy,
}

fn tx(conn: &Connection) -> Result<Transaction<'_>> {
    Ok(Transaction::new_unchecked(
        conn,
        TransactionBehavior::Immediate,
    )?)
}

// One column per argument of the device row; a struct would only rename them.
#[allow(clippy::too_many_arguments)]
fn register(
    conn: &Connection,
    public: &DevicePublic,
    name: &str,
    role: DeviceRole,
    attendance: Attendance,
    kind: DeviceKind,
    verified_by: VerifiedBy,
    now: i64,
) -> Result<DeviceId> {
    public.validate().context("invalid device key")?;
    let id = public.device_id();
    conn.execute(
        "INSERT OR IGNORE INTO sync_devices
           (device_id, ed25519, x25519, mlkem, name, role, attendance, kind, verified_by, last_seen)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            id,
            &public.ed25519_pub[..],
            &public.x25519_pub[..],
            public.mlkem_pub,
            name,
            role_db(role),
            attendance_db(attendance),
            super::merge::kind_db(kind),
            super::merge::verified_db(verified_by),
            now
        ],
    )?;
    // A row that arrived through an anchor has no label (its name is the device id): the first time this
    // device meets it with a name, that name becomes the local label. A real label is never overwritten.
    if !name.is_empty() {
        conn.execute(
            "UPDATE sync_devices SET name = ?2 WHERE device_id = ?1 AND name = device_id",
            params![id, name],
        )?;
    }
    super::checkpoint::record_anchor_device(
        conn,
        public,
        name,
        role,
        attendance,
        kind,
        verified_by,
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
        kind: DeviceKind,
        verified_by: VerifiedBy,
    ) -> Result<DeviceId> {
        self.with_conn(|conn| {
            let tx = tx(conn)?;
            let id = register(
                &tx,
                public,
                name,
                role,
                attendance,
                kind,
                verified_by,
                now_ts(),
            )?;
            tx.commit()?;
            Ok(id)
        })
    }

    /// Chain, sign and apply a local change. The op is ours, so it is approved
    /// by definition; its effects are projected like any other.
    pub fn sync_append_local(&self, id: &DeviceIdentity, body: OpBody) -> Result<Op> {
        self.sync_append_local_at(id, body, now_ts())
    }

    /// [`Self::sync_append_local`] with an explicit clock (checkpoint proposals
    /// are built for the timestamp they carry; tests use it to age a ring).
    pub fn sync_append_local_at(&self, id: &DeviceIdentity, body: OpBody, now: i64) -> Result<Op> {
        self.with_conn(|conn| append_local_on(conn, id, body, now))
    }
}

/// Chain, sign and apply a local change on `conn` (the body of
/// [`LauncherStore::sync_append_local_at`]); the caller holds no transaction.
pub(crate) fn append_local_on(
    conn: &Connection,
    id: &DeviceIdentity,
    body: OpBody,
    now: i64,
) -> Result<Op> {
    validate_body(&body).map_err(|e| anyhow!("invalid op: {e}"))?;
    {
        {
            let tx = tx(conn)?;
            let public = id.public();
            let device = register(
                &tx,
                &public,
                "this device",
                DeviceRole::Holder,
                Attendance::Attended,
                DeviceKind::Terminal,
                VerifiedBy::Sas,
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
            ensure!(
                op.canonical_bytes().len() <= MAX_OP_BYTES,
                "invalid op: larger than the wire allows ({MAX_OP_BYTES} bytes)"
            );
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
        }
    }
}

impl LauncherStore {
    /// Ops of `device` after `after_seq`, oldest first, at most one tail frame.
    pub fn sync_chain_tail(&self, device: &DeviceId, after_seq: u64) -> Result<Vec<Op>> {
        self.with_conn(|conn| {
            let floor: Option<i64> = conn
                .query_row(
                    "SELECT seq FROM sync_prune_floor WHERE device_id = ?1",
                    params![device],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(f) = floor {
                if after_seq.saturating_add(1) < f as u64 {
                    return Err(super::checkpoint::Pruned {
                        floor_seq: f as u64,
                    }
                    .into());
                }
            }
            let after = i64::try_from(after_seq).unwrap_or(i64::MAX);
            let mut stmt = conn.prepare(
                "SELECT op_hash FROM sync_ops WHERE device_id = ?1 AND seq > ?2
                   AND COALESCE(reason, '') <> 'unknown-author' ORDER BY seq LIMIT ?3",
            )?;
            let hashes: Vec<Vec<u8>> = stmt
                .query_map(params![device, after, MAX_OPS_PER_TAIL as i64], |r| {
                    r.get(0)
                })?
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

    /// [`Self::sync_ingest`] for a frame of ops in ONE transaction: one commit (one set of
    /// fsyncs) instead of one per op, the cost that dominates a large log on a real disk.
    /// Outcomes are in the order of `ops`; a database error rolls the whole batch back.
    pub fn sync_ingest_batch(&self, ops: Vec<Op>) -> Result<Vec<IngestOutcome>> {
        // Chunks of 100: the store lock is released between them (the UI shares it), and an error
        // costs one chunk, not the frame. A chunk that fails is retried one op at a time, so every
        // op before the failing one is kept.
        let mut out = Vec::with_capacity(ops.len());
        for chunk in ops.chunks(100) {
            let attempt = self.with_conn(|conn| {
                let tx = tx(conn)?;
                let now = now_ts();
                let mut done = Vec::with_capacity(chunk.len());
                for op in chunk {
                    done.push(ingest(&tx, op.clone(), now)?);
                }
                tx.commit()?;
                Ok(done)
            });
            match attempt {
                Ok(done) => out.extend(done),
                Err(_) => {
                    for op in chunk {
                        out.push(self.sync_ingest(op.clone())?);
                    }
                }
            }
        }
        Ok(out)
    }

    /// Ops waiting for review, oldest first.
    pub fn sync_pending(&self) -> Result<Vec<Op>> {
        self.with_conn(pending::list)
    }

    /// EVERY pending op with the reason it waits (`tier`, `stale`, `resurrection`,
    /// `revoke-conflict`, `unknown-author`), oldest first.
    pub fn sync_pending_with_reasons(&self) -> Result<Vec<(Op, String)>> {
        self.with_conn(pending::list_with_reasons)
    }

    /// Why an op is waiting: `tier`, `stale`, `resurrection`, `revoke-conflict` or `unknown-author`.
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
                        COALESCE((SELECT epoch FROM sync_epochs e WHERE e.device_id = d.device_id), 0),
                        d.wrap_epoch, d.kind, d.verified_by
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
                    r.get::<_, i64>(12)?,
                    r.get::<_, String>(13)?,
                    r.get::<_, String>(14)?,
                ))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (id, ed, x, mlkem, name, role, att, status, exempt, last_seen, head, epoch, wrap_epoch, kind, verified_by) =
                    row?;
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
                    wrap_epoch: wrap_epoch as u64,
                    verified_by: if verified_by == "ssh_host_key" {
                        VerifiedBy::SshHostKey
                    } else {
                        VerifiedBy::Sas
                    },
                    kind: if kind == "phone" {
                        DeviceKind::Phone
                    } else {
                        DeviceKind::Terminal
                    },
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
                        sealed: super::canonical::b64_decode(
                            s["sealed"].as_str().context("sealed")?,
                        )
                        .context("sealed is not base64")?,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(Some((ct, shares)))
        })
    }

    /// The certificate's folded record (effective end date, renewals counted): see `oplog::cert_fold`.
    pub fn sync_cert_record(&self, cert_id: &[u8; 32]) -> Result<Option<super::CertRecord>> {
        self.with_conn(|conn| super::cert_record(conn, &super::merge::hex(cert_id)))
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

    // --- checkpoints and pruning (C15) ---

    pub fn sync_checkpoint_build(
        &self,
        cut: Option<Vec<CutEntry>>,
        now: i64,
    ) -> Result<(Vec<CutEntry>, Vec<u8>, [u8; 32])> {
        self.with_conn(|conn| checkpoint::build(conn, cut, now))
    }

    pub fn sync_checkpoint_verify(&self, proposal: &Op, state: &[u8]) -> Result<CheckpointVerdict> {
        self.with_conn(|conn| checkpoint::verify(conn, proposal, state))
    }

    pub fn sync_checkpoint_record(&self, bundle: &CheckpointBundle) -> Result<CheckpointStatus> {
        self.with_conn(|conn| {
            let tx = tx(conn)?;
            let st = checkpoint::record(&tx, bundle, now_ts())?;
            tx.commit()?;
            Ok(st)
        })
    }

    pub fn sync_checkpoints(&self) -> Result<Vec<CheckpointRow>> {
        self.with_conn(checkpoint::list)
    }

    /// What a peer needs to adopt a recorded checkpoint (state kept for the two
    /// newest accepted ones only).
    pub fn sync_checkpoint_bundle(
        &self,
        proposal_hash: &[u8; 32],
    ) -> Result<Option<CheckpointBundle>> {
        self.with_conn(|conn| checkpoint::bundle_of(conn, proposal_hash))
    }

    /// Build a proposal for the local cut and append it (this device must be an
    /// attended terminal). Returns the op and the state blob to serve.
    pub fn sync_checkpoint_propose(
        &self,
        id: &DeviceIdentity,
        cut: Option<Vec<CutEntry>>,
        now: i64,
    ) -> Result<(Op, Vec<u8>)> {
        let me = id.public().device_id();
        let (cut, state, hash, required, prev) = self.with_conn(|conn| {
            let ok: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM sync_devices WHERE device_id = ?1 AND kind = 'terminal'
                   AND attendance = 'attended')",
                params![me],
                |r| r.get(0),
            )?;
            anyhow::ensure!(ok, "only an attended terminal proposes checkpoints");
            let (cut, state, hash) = checkpoint::build(conn, cut, now)?;
            let required = checkpoint::required_for(conn, &cut, now)?;
            let prev = checkpoint::latest_accepted_hash(conn)?;
            Ok((cut, state, hash, required, prev))
        })?;
        let op = self.sync_append_local_at(
            id,
            OpBody::CheckpointPropose {
                state_len: state.len() as u64,
                cut,
                state_hash: hash,
                prev_checkpoint: prev,
                required,
            },
            now,
        )?;
        // Keep our own proposal (and its blob) so peers can fetch it.
        self.sync_checkpoint_record(&CheckpointBundle {
            proposal: op.clone(),
            signs: vec![],
            heads: vec![],
            state: state.clone(),
        })?;
        Ok((op, state))
    }

    /// Verify a proposal against the local store and sign it only when it
    /// matches. A mismatch records the proposal as disputed and its proposer as
    /// suspect (never compromised); `None` is returned for anything but a match.
    pub fn sync_checkpoint_sign(
        &self,
        id: &DeviceIdentity,
        proposal: &Op,
        state: &[u8],
    ) -> Result<Option<Op>> {
        let me = id.public().device_id();
        if !self.with_conn(|conn| checkpoint::attended_terminal(conn, &me))? {
            return Ok(None); // unattended devices and phones never sign
        }
        match self.sync_checkpoint_verify(proposal, state)? {
            CheckpointVerdict::Matches => Ok(Some(self.sync_append_local(
                id,
                OpBody::CheckpointSign {
                    proposal_hash: proposal.hash(),
                },
            )?)),
            CheckpointVerdict::Differs { .. } => {
                self.sync_checkpoint_record(&CheckpointBundle {
                    proposal: proposal.clone(),
                    signs: vec![],
                    heads: vec![],
                    state: state.to_vec(),
                })?;
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    /// Delete what the policy allows; idempotent. Space is reclaimed with a
    /// `VACUUM`, so deleted blobs do not linger in free pages.
    pub fn sync_prune(&self, now: i64) -> Result<PruneReport> {
        self.with_conn(|conn| {
            let tx = tx(conn)?;
            let report = checkpoint::prune(&tx, now)?;
            tx.commit()?;
            if report.ops_deleted > 0 {
                conn.execute_batch("VACUUM")?;
            }
            Ok(report)
        })
    }

    /// Materialise a fresh store from a bundle (see [`BootstrapTrust`]).
    pub fn sync_bootstrap(&self, bundle: &CheckpointBundle, trust: &BootstrapTrust) -> Result<()> {
        self.with_conn(|conn| {
            let tx = tx(conn)?;
            checkpoint::bootstrap(&tx, bundle, trust, now_ts())?;
            tx.commit()?;
            Ok(())
        })
    }

    /// The join anchor an inviter hands to a device it enrolls: its resolved state at its chain
    /// heads, signed by the inviter alone. It is not a checkpoint and prunes nothing.
    pub fn sync_join_anchor_build(&self, id: &DeviceIdentity) -> Result<CheckpointBundle> {
        self.with_conn(|conn| checkpoint::build_join_anchor(conn, id, now_ts()))
    }

    /// The joiner adopts the anchor as its starting state (only
    /// [`BootstrapTrust::Enrollment`]): the ring's devices and hosts are not its to review, only
    /// ops above the cut are.
    pub fn sync_join_adopt(&self, bundle: &CheckpointBundle, trust: &BootstrapTrust) -> Result<()> {
        ensure!(
            matches!(trust, BootstrapTrust::Enrollment { .. }),
            "a join anchor is adopted with the enrollment trust only"
        );
        self.sync_bootstrap(bundle, trust)
    }

    /// Newest accepted checkpoint we can serve (announced in the hello).
    pub fn sync_checkpoint_head(&self) -> Result<Option<[u8; 32]>> {
        self.with_conn(checkpoint::servable_head)
    }

    /// The wire bytes of a checkpoint we hold the blob of.
    pub fn sync_checkpoint_bytes(&self, proposal_hash: &[u8; 32]) -> Result<Option<Vec<u8>>> {
        self.with_conn(|conn| {
            Ok(
                checkpoint::bundle_of(conn, proposal_hash)?
                    .map(|b| checkpoint::bundle_to_bytes(&b)),
            )
        })
    }

    /// Proposals known as ops without a blob: to fetch from a peer.
    pub fn sync_checkpoint_missing(&self) -> Result<Vec<[u8; 32]>> {
        self.with_conn(checkpoint::missing_blobs)
    }

    /// Take the bytes of a checkpoint fetched from `peer`: record it (a proposal
    /// to verify) or, with `adopt`, make it the base of this store.
    pub fn sync_checkpoint_accept(
        &self,
        bytes: &[u8],
        peer: &DeviceId,
        adopt: bool,
    ) -> Result<[u8; 32]> {
        // Remote bytes are parsed before the connection is taken: a parser bug must not poison it.
        let bundle = checkpoint::bundle_from_bytes(bytes)?;
        self.with_conn(|conn| {
            let tx = tx(conn)?;
            let h = checkpoint::accept_bundle(&tx, bundle, peer, adopt, now_ts())?;
            tx.commit()?;
            Ok(h)
        })
    }

    /// Record a proposal with the sign ops we hold (the driver calls this).
    pub fn sync_checkpoint_update(
        &self,
        proposal_hash: &[u8; 32],
    ) -> Result<Option<CheckpointStatus>> {
        self.with_conn(|conn| {
            let tx = tx(conn)?;
            let Some(mut bundle) = checkpoint::bundle_of(&tx, proposal_hash)? else {
                return Ok(None);
            };
            bundle.signs = checkpoint::known_signs(&tx, proposal_hash)?;
            let st = checkpoint::record(&tx, &bundle, now_ts())?;
            tx.commit()?;
            Ok(Some(st))
        })
    }

    /// Ops and bytes appended since the lamport of the last accepted cut.
    pub fn sync_log_growth(&self, since_lamport: u64) -> Result<(u64, u64)> {
        self.with_conn(|conn| {
            let (n, b): (i64, i64) = conn.query_row(
                "SELECT COUNT(*), COALESCE(SUM(length(canonical)), 0) FROM sync_ops WHERE lamport > ?1",
                params![since_lamport as i64],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            Ok((n as u64, b as u64))
        })
    }

    /// Timestamp of the newest proposal we know of.
    pub fn sync_last_checkpoint_time(&self) -> Result<Option<i64>> {
        self.with_conn(|conn| {
            Ok(
                conn.query_row("SELECT MAX(proposed_at) FROM sync_checkpoints", [], |r| {
                    r.get(0)
                })?,
            )
        })
    }

    /// Timestamp of the newest reshare batch above `since_lamport`.
    pub fn sync_last_reshare_time(&self, since_lamport: u64) -> Result<Option<i64>> {
        self.with_conn(|conn| {
            Ok(conn.query_row(
                "SELECT MAX(timestamp) FROM sync_ops WHERE kind = 'ShareReshareBatch' AND lamport > ?1
                   AND state IN ('applied','approved')",
                params![since_lamport as i64],
                |r| r.get(0),
            )?)
        })
    }

    /// Live reshare envelopes of one credential: `(holder, epoch, sealed share)`.
    pub fn sync_share_envelopes(&self, host: &HostUid) -> Result<Vec<(DeviceId, u64, Vec<u8>)>> {
        self.with_conn(|conn| checkpoint::share_envelopes(conn, host))
    }

    /// The current storage-key envelope of a holder: `(epoch, sealed)`.
    pub fn sync_storage_key(&self, device: &DeviceId) -> Result<Option<(u64, Vec<u8>)>> {
        self.with_conn(|conn| checkpoint::storage_key(conn, device))
    }

    /// Host hop tokens as typed locally to the wire form (`#uid`).
    pub fn sync_hops_to_wire(&self, local: &str) -> Result<String> {
        self.with_conn(|conn| apply::hops_to_wire(conn, local))
    }
}
