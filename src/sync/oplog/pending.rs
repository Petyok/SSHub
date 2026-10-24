//! Review of quarantined ops.

use anyhow::{bail, Result};
use rusqlite::Connection;

use super::merge::{desired_state, load_op, OpRow, Settler, State};
use super::op::Op;
use crate::sync::apply::reproject;

pub(crate) fn list(conn: &Connection) -> Result<Vec<Op>> {
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
            out.push(row.op);
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
    let target = if approve {
        // A revoked author's op can be approved but stays void.
        let probe = OpRow {
            state: State::Approved,
            ..clone_row(&row)
        };
        match desired_state(conn, &probe)?.0 {
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

fn clone_row(r: &OpRow) -> OpRow {
    OpRow {
        op: r.op.clone(),
        hash: r.hash,
        state: r.state,
        reason: r.reason.clone(),
        local: r.local,
    }
}
