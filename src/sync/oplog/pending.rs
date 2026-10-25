//! Review of quarantined ops.

use anyhow::{bail, Result};
use rusqlite::Connection;

use super::merge::{
    desired_state, forget_unverified, load_op, OpRow, Settler, State, UNKNOWN_AUTHOR,
};
use super::op::Op;
use super::store_impl::IngestOutcome;
use crate::sync::apply::reproject;

pub(crate) fn list(conn: &Connection) -> Result<Vec<Op>> {
    Ok(list_with_reasons(conn)?
        .into_iter()
        .map(|(o, _)| o)
        .collect())
}

pub(crate) fn list_with_reasons(conn: &Connection) -> Result<Vec<(Op, String)>> {
    let mut stmt = conn.prepare(
        "SELECT op_hash FROM sync_ops WHERE state = 'pending' ORDER BY lamport, device_id, seq",
    )?;
    let hashes: Vec<Vec<u8>> = stmt
        .query_map([], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    let mut out = Vec::new();
    for h in hashes {
        let h: [u8; 32] = h
            .try_into()
            .map_err(|_| anyhow::anyhow!("stored hash length"))?;
        if let Some(row) = load_op(conn, &h)? {
            out.push((row.op, row.reason.unwrap_or_default()));
        }
    }
    Ok(out)
}

pub(crate) fn reason(conn: &Connection, hash: &[u8; 32]) -> Result<Option<String>> {
    Ok(load_op(conn, hash)?
        .filter(|r| r.state == State::Pending)
        .and_then(|r| r.reason))
}

/// Approve or reject one pending op. A rejected op stays in the chain (peers
/// still need it to verify later ops) but never takes effect here.
pub(crate) fn resolve(conn: &Connection, hash: &[u8; 32], approve: bool, now: i64) -> Result<()> {
    let Some(row) = load_op(conn, hash)? else {
        bail!("no such op");
    };
    if row.state != State::Pending {
        bail!("op is not pending");
    }
    if row.reason.as_deref() == Some(UNKNOWN_AUTHOR) {
        return resolve_unknown_author(conn, row, approve, now);
    }
    let target = if approve {
        // A revoked author's op can be approved but stays void.
        let probe = OpRow {
            state: State::Approved,
            ..clone_row(&row)
        };
        match desired_state(conn, &probe, now)?.0 {
            State::Void => State::Void,
            _ => State::Approved,
        }
    } else {
        State::Rejected
    };
    let mut settler = Settler::new(conn, now);
    settler.transition(&row, target, None)?;
    settler.run()?;
    reproject(conn, &settler.touched)
}

/// An op of an author this device does not know. Rejecting forgets it. Approving
/// needs the author to be known by now, and then runs the op through the ordinary
/// ingest (signature, chain, tier): what it returns is what the op becomes.
fn resolve_unknown_author(conn: &Connection, row: OpRow, approve: bool, now: i64) -> Result<()> {
    if approve {
        let known: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sync_devices WHERE device_id = ?1)",
            [&row.op.device_id],
            |r| r.get(0),
        )?;
        if !known {
            bail!(
                "the author of this op is not a device of the ring yet: approve the device first"
            );
        }
    }
    forget_unverified(conn, &row.op.device_id, Some(row.op.seq))?;
    if approve {
        if let IngestOutcome::Rejected(why) = super::ingest::ingest(conn, row.op, now)? {
            bail!("the op does not verify: {why}");
        }
    }
    Ok(())
}

fn clone_row(r: &OpRow) -> OpRow {
    OpRow {
        op: r.op.clone(),
        hash: r.hash,
        state: r.state,
        reason: r.reason.clone(),
        local: r.local,
    }
}
