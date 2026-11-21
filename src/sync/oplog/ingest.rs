//! `sync_ingest`: verify a foreign op, chain it, decide its state, apply.
//!
//! Order of checks (every verdict is a pure function of the op and the log):
//! numeric range, known and not compromised signer, algorithm, signature,
//! static field validation, duplicate or fork, chain position and `prev_hash`,
//! lamport monotonicity and jump cap, quarantine cap. Only then is the op
//! stored (`applied`, `pending` or `void`) and projected.

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use super::fork;
use super::merge::{
    collision_holder, desired_state, device_bytes, forget_unverified, insert_op, insert_unverified,
    load_op, op_at, OpRow, Settler, State,
};
use super::op::*;
use super::store_impl::IngestOutcome;
use crate::sync::apply::{reproject, validate_body};
use crate::sync::crypto::ALG_CURRENT;
use crate::sync::limits::{LAMPORT_JUMP_CAP, PENDING_QUARANTINE_MAX_PER_DEVICE};

fn reject(msg: impl Into<String>) -> Result<IngestOutcome> {
    Ok(IngestOutcome::Rejected(msg.into()))
}

/// Stored bytes of one device's ops, applied or waiting, at most (prune brings it down).
const DEVICE_BYTES_QUOTA: i64 = 64 * 1024 * 1024;

/// Unverified ops held for review at most, over all unknown authors and per claimed author.
const UNKNOWN_AUTHOR_MAX: i64 = 256;
const UNKNOWN_AUTHOR_MAX_PER_ID: usize = 8;
/// Pending bytes one device may keep in quarantine (next to the count cap).
const PENDING_QUARANTINE_MAX_BYTES_PER_DEVICE: usize = 16 * 1024 * 1024;

/// An op signed by nobody we know: its signature cannot be checked, so it only waits for a
/// human (reason `unknown-author`). Nothing of it takes effect: no chain, no fold, never served.
fn unknown_author(
    conn: &Connection,
    op: Op,
    now: i64,
    quarantine_cap: usize,
) -> Result<IngestOutcome> {
    if op.canonical_bytes().len() > MAX_OP_BYTES {
        return reject("op is larger than a transport frame");
    }
    if let Err(why) = validate_body(&op.body) {
        return reject(format!("invalid op: {why}"));
    }
    if load_op(conn, &op.hash())?.is_some() {
        return reject("duplicate op");
    }
    // A slot held by another unverified op gives way: nobody can squat a slot with a claim
    // nobody could check. A slot of a verified op (a known author's chain) never does.
    let slot: Option<Option<String>> = conn
        .query_row(
            "SELECT reason FROM sync_ops WHERE device_id = ?1 AND seq = ?2",
            params![op.device_id, op.seq as i64],
            |r| r.get(0),
        )
        .optional()?;
    match slot {
        Some(r) if r.as_deref() != Some("unknown-author") => return reject("duplicate op"),
        Some(_) => forget_unverified(conn, &op.device_id, Some(op.seq))?,
        None => {}
    }
    let (per_author, all): (i64, i64) = conn.query_row(
        "SELECT COALESCE(SUM(device_id = ?1), 0), COUNT(*) FROM sync_ops WHERE reason = 'unknown-author'",
        params![op.device_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if per_author as usize >= UNKNOWN_AUTHOR_MAX_PER_ID.min(quarantine_cap)
        || all >= UNKNOWN_AUTHOR_MAX
    {
        return reject("quarantine full for unknown authors");
    }
    insert_unverified(conn, &op, now)?;
    Ok(IngestOutcome::Pending)
}

pub(crate) fn max_lamport(conn: &Connection) -> Result<u64> {
    let m: Option<i64> = conn.query_row("SELECT MAX(lamport) FROM sync_ops", [], |r| r.get(0))?;
    Ok(m.unwrap_or(0) as u64)
}

pub(crate) fn ingest(conn: &Connection, op: Op, now: i64) -> Result<IngestOutcome> {
    ingest_capped(conn, op, now, PENDING_QUARANTINE_MAX_PER_DEVICE)
}

/// [`ingest`] with an explicit quarantine cap (tests use a small one).
pub(crate) fn ingest_capped(
    conn: &Connection,
    op: Op,
    now: i64,
    quarantine_cap: usize,
) -> Result<IngestOutcome> {
    let limit = i64::MAX as u64;
    if op.seq == 0 || op.seq > limit || op.lamport == 0 || op.lamport > limit || op.epoch > limit {
        return reject("seq, lamport or epoch out of range");
    }
    if op.alg != ALG_CURRENT {
        return reject("unsupported algorithm tag");
    }
    let device: Option<(Vec<u8>, String)> = conn
        .query_row(
            "SELECT ed25519, status FROM sync_devices WHERE device_id = ?1",
            params![op.device_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((pub_raw, status)) = device else {
        return unknown_author(conn, op, now, quarantine_cap);
    };
    if status == "compromised" {
        return reject("device is compromised");
    }
    let Ok(signer): std::result::Result<[u8; 32], _> = pub_raw.try_into() else {
        return reject("stored device key is malformed");
    };
    if !op.verify(&signer) {
        return reject("bad signature");
    }
    if op.canonical_bytes().len() > MAX_OP_BYTES {
        return reject("op is larger than a transport frame");
    }
    if let Err(why) = validate_body(&op.body) {
        return reject(format!("invalid op: {why}"));
    }
    if let OpBody::ShareRelease { requester, .. } = &op.body {
        if *requester != op.device_id {
            return reject("invalid op: requester differs from the signer");
        }
    }

    if let OpBody::WrapKeyRotate { device_id, .. } = &op.body {
        if *device_id != op.device_id {
            return reject("invalid op: wrap keys can only be rotated by their own device");
        }
    }

    let floor: Option<i64> = conn
        .query_row(
            "SELECT seq FROM sync_prune_floor WHERE device_id = ?1",
            params![op.device_id],
            |r| r.get(0),
        )
        .optional()?;
    if floor.is_some_and(|f| op.seq < f as u64) {
        return reject("covered by an accepted checkpoint");
    }

    // This author is known now: an unverified copy of the same slot gives way.
    forget_unverified(conn, &op.device_id, Some(op.seq))?;
    let hash = op.hash();
    if let Some(stored) = op_at(conn, &op.device_id, op.seq)? {
        if stored.hash == hash {
            return reject("duplicate op");
        }
        fork::record_op_fork(conn, &stored.op, &op)?;
        return Ok(IngestOutcome::ForkProof(Box::new(stored.op), Box::new(op)));
    }
    let head: Option<(i64, Vec<u8>, i64)> = conn
        .query_row(
            "SELECT seq, op_hash, lamport FROM sync_chain_heads WHERE device_id = ?1",
            params![op.device_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let (head_seq, head_hash, head_lamport) = match head {
        Some((s, h, l)) => (s as u64, h, l as u64),
        None => (0, vec![0u8; 32], 0),
    };
    if op.seq != head_seq + 1 {
        return reject(format!("gap: have seq {head_seq}, got {}", op.seq));
    }
    if op.prev_hash[..] != head_hash[..] {
        return reject("prev_hash does not continue the chain");
    }
    if op.lamport <= head_lamport {
        return reject("lamport must increase along a chain");
    }
    if op.lamport > max_lamport(conn)?.saturating_add(LAMPORT_JUMP_CAP) {
        return reject("lamport jump over the cap");
    }

    let probe = OpRow {
        op: op.clone(),
        hash,
        state: State::Applied,
        reason: None,
        local: false,
    };
    let (want, reason) = desired_state(conn, &probe, now, true)?;
    if want == State::Pending {
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sync_pending_ops WHERE device_id = ?1",
            params![op.device_id],
            |r| r.get(0),
        )?;
        if n as usize >= quarantine_cap {
            return reject("quarantine full for this device");
        }
        let bytes: i64 = conn.query_row(
            "SELECT COALESCE(SUM(length(canonical)), 0) FROM sync_ops
             WHERE state = 'pending' AND device_id = ?1",
            params![op.device_id],
            |r| r.get(0),
        )?;
        if bytes as usize + op.canonical_bytes().len() > PENDING_QUARANTINE_MAX_BYTES_PER_DEVICE {
            return reject("quarantine full for this device (bytes)");
        }
    }

    if device_bytes(conn, &op.device_id)? + op.canonical_bytes().len() as i64 > DEVICE_BYTES_QUOTA {
        return reject("byte quota of this device is full");
    }
    insert_op(conn, &op, State::Rejected, None, false, now)?;
    conn.execute(
        "UPDATE sync_devices SET last_seen = ?2 WHERE device_id = ?1",
        params![op.device_id, now],
    )?;
    let row = load_op(conn, &hash)?.expect("just inserted");
    let mut settler = Settler::new(conn, now);
    settler.transition(&row, want, reason)?;
    // Two claims to one name wait on every device, whichever arrived first: the claim this one
    // collides with (applied without a human) goes back to review as well.
    if want == State::Pending && reason == Some("tier") {
        if let Some(holder) = collision_holder(conn, &op)? {
            if let Some(h) = load_op(conn, &holder)? {
                if h.state == State::Applied && !h.local {
                    settler.transition(&h, State::Pending, Some("tier"))?;
                }
            }
        }
    }
    settler.run()?;
    reproject(conn, &settler.touched)?;

    // A signed head stored earlier may contradict this op.
    if fork::check_heads_against_op(conn, &op, &hash)? {
        return reject("fork: a signed head contradicts this op");
    }
    Ok(match load_op(conn, &hash)?.map(|r| r.state) {
        Some(State::Applied | State::Approved) => IngestOutcome::Applied,
        Some(State::Pending) => IngestOutcome::Pending,
        _ => IngestOutcome::Rejected("void: signed at or after its author's revocation".into()),
    })
}
