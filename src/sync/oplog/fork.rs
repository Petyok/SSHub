//! Fork detection strictly by proof (design: Op-log). A device is
//! `compromised` only on cryptographic proof of equivocation: two signed ops
//! with the same seq and different hashes, a signed head that contradicts a
//! chain op, or two signed heads that contradict each other. Anything weaker
//! (gossip that disagrees) is `suspect` at most.

use anyhow::{anyhow, ensure, Result};
use rusqlite::{params, Connection, OptionalExtension};

use super::merge::op_at;
use super::op::*;
use crate::sync::crypto::{verify, DeviceIdentity};

const HEAD_DOMAIN: &[u8] = b"sshub/sync/head/v1";

/// A device's self-signed statement "my chain had this hash at this seq".
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SignedHead {
    pub device_id: DeviceId,
    pub seq: u64,
    pub hash: [u8; 32],
    pub sig: [u8; 64],
}

impl SignedHead {
    fn preimage(device_id: &str, seq: u64, hash: &[u8; 32]) -> Vec<u8> {
        let mut m = HEAD_DOMAIN.to_vec();
        m.extend_from_slice(device_id.as_bytes());
        m.push(0);
        m.extend_from_slice(&seq.to_be_bytes());
        m.extend_from_slice(hash);
        m
    }

    pub fn sign(id: &DeviceIdentity, seq: u64, hash: [u8; 32]) -> Self {
        let device_id = id.public().device_id();
        let sig = id.sign(&Self::preimage(&device_id, seq, &hash));
        Self {
            device_id,
            seq,
            hash,
            sig,
        }
    }

    pub fn verify(&self, ed25519_pub: &[u8; 32]) -> bool {
        verify(
            ed25519_pub,
            &Self::preimage(&self.device_id, self.seq, &self.hash),
            &self.sig,
        )
    }

    fn to_blob(&self) -> Vec<u8> {
        let mut b = vec![b'H'];
        b.extend_from_slice(&self.sig);
        b.extend_from_slice(&self.hash);
        b.extend_from_slice(&self.seq.to_be_bytes());
        b.extend_from_slice(self.device_id.as_bytes());
        b
    }

    fn from_blob(b: &[u8]) -> Result<Self> {
        ensure!(b.len() > 1 + 64 + 32 + 8 && b[0] == b'H', "bad head blob");
        Ok(Self {
            sig: b[1..65].try_into()?,
            hash: b[65..97].try_into()?,
            seq: u64::from_be_bytes(b[97..105].try_into()?),
            device_id: String::from_utf8(b[105..].to_vec())?,
        })
    }
}

/// What the proof consists of, for the alert screen and for forwarding.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ForkEvidence {
    Ops(Box<Op>, Box<Op>),
    OpAndHead(Box<Op>, SignedHead),
    Heads(SignedHead, SignedHead),
}

fn op_blob(op: &Op) -> Vec<u8> {
    let mut b = vec![b'O'];
    b.extend_from_slice(&op.signature);
    b.extend_from_slice(&op.canonical_bytes());
    b
}

fn op_from_blob(b: &[u8]) -> Result<Op> {
    ensure!(b.len() > 65 && b[0] == b'O', "bad op blob");
    Op::from_canonical(&b[65..], b[1..65].try_into()?)
}

fn store_fork(conn: &Connection, device: &str, a: Vec<u8>, b: Vec<u8>) -> Result<()> {
    conn.execute(
        "UPDATE sync_devices SET status = 'compromised', fork_a = ?2, fork_b = ?3
         WHERE device_id = ?1",
        params![device, a, b],
    )?;
    Ok(())
}

/// Two ops with one seq and different hashes.
pub(crate) fn record_op_fork(conn: &Connection, stored: &Op, new: &Op) -> Result<()> {
    store_fork(conn, &new.device_id, op_blob(stored), op_blob(new))
}

pub(crate) fn evidence(conn: &Connection, device: &str) -> Result<Option<ForkEvidence>> {
    type Blobs = (Option<Vec<u8>>, Option<Vec<u8>>);
    let row: Option<Blobs> = conn
        .query_row(
            "SELECT fork_a, fork_b FROM sync_devices WHERE device_id = ?1",
            params![device],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((Some(a), Some(b))) = row else {
        return Ok(None);
    };
    Ok(Some(match (a[0], b[0]) {
        (b'O', b'O') => ForkEvidence::Ops(Box::new(op_from_blob(&a)?), Box::new(op_from_blob(&b)?)),
        (b'O', b'H') => {
            ForkEvidence::OpAndHead(Box::new(op_from_blob(&a)?), SignedHead::from_blob(&b)?)
        }
        (b'H', b'H') => ForkEvidence::Heads(SignedHead::from_blob(&a)?, SignedHead::from_blob(&b)?),
        _ => return Err(anyhow!("unknown fork evidence")),
    }))
}

/// After a new op at `op.seq` was stored: does a stored signed head of the
/// same device at that seq disagree with it?
pub(crate) fn check_heads_against_op(conn: &Connection, op: &Op, hash: &[u8; 32]) -> Result<bool> {
    let mut stmt = conn.prepare_cached(
        "SELECT chain_hash, signature FROM sync_head_attests
         WHERE device_id = ?1 AND seq = ?2 AND chain_hash != ?3",
    )?;
    let rows: Vec<(Vec<u8>, Vec<u8>)> = stmt
        .query_map(params![op.device_id, op.seq as i64, &hash[..]], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?
        .collect::<std::result::Result<_, _>>()?;
    for (h, sig) in rows {
        let head = SignedHead {
            device_id: op.device_id.clone(),
            seq: op.seq,
            hash: h.try_into().map_err(|_| anyhow!("stored head hash"))?,
            sig: sig.try_into().map_err(|_| anyhow!("stored head sig"))?,
        };
        if verify_head_with_store(conn, &head)? {
            store_fork(conn, &op.device_id, op_blob(op), head.to_blob())?;
            return Ok(true);
        }
    }
    Ok(false)
}

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

fn verify_head_with_store(conn: &Connection, head: &SignedHead) -> Result<bool> {
    Ok(device_pub(conn, &head.device_id)?.is_some_and(|p| head.verify(&p)))
}

/// Verdict on a signed head from a peer.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum HeadVerdict {
    /// Matches the chain we hold (or a head we already stored).
    Consistent,
    /// Beyond our copy of that chain: we are behind, nothing contradicts it.
    Ahead,
    /// Proof of equivocation: the device is now `compromised`.
    Fork,
    Rejected(String),
}

pub(crate) fn record_head(conn: &Connection, head: &SignedHead) -> Result<HeadVerdict> {
    let Some(pubkey) = device_pub(conn, &head.device_id)? else {
        return Ok(HeadVerdict::Rejected("unknown device".into()));
    };
    if !head.verify(&pubkey) {
        return Ok(HeadVerdict::Rejected("bad head signature".into()));
    }
    if head.seq == 0 || head.seq > i64::MAX as u64 {
        return Ok(HeadVerdict::Rejected("head seq out of range".into()));
    }
    // Against the chain we hold.
    if let Some(stored) = op_at(conn, &head.device_id, head.seq)? {
        if stored.hash != head.hash {
            store_fork(conn, &head.device_id, op_blob(&stored.op), head.to_blob())?;
            return Ok(HeadVerdict::Fork);
        }
    }
    // Against heads we already stored at the same seq.
    let others: Vec<(Vec<u8>, Vec<u8>)> = conn
        .prepare("SELECT chain_hash, signature FROM sync_head_attests WHERE device_id = ?1 AND seq = ?2 AND chain_hash != ?3")?
        .query_map(params![head.device_id, head.seq as i64, &head.hash[..]], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?
        .collect::<std::result::Result<_, _>>()?;
    for (h, sig) in others {
        let other = SignedHead {
            device_id: head.device_id.clone(),
            seq: head.seq,
            hash: h.try_into().map_err(|_| anyhow!("stored head hash"))?,
            sig: sig.try_into().map_err(|_| anyhow!("stored head sig"))?,
        };
        if other.verify(&pubkey) {
            store_fork(conn, &head.device_id, other.to_blob(), head.to_blob())?;
            return Ok(HeadVerdict::Fork);
        }
    }
    conn.execute(
        "INSERT OR IGNORE INTO sync_head_attests (device_id, seq, chain_hash, signature)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            head.device_id,
            head.seq as i64,
            &head.hash[..],
            &head.sig[..]
        ],
    )?;
    let have: Option<i64> = conn
        .query_row(
            "SELECT seq FROM sync_chain_heads WHERE device_id = ?1",
            params![head.device_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(if have.is_none_or(|h| (h as u64) < head.seq) {
        HeadVerdict::Ahead
    } else {
        HeadVerdict::Consistent
    })
}

/// A rumour that disagrees is a reason to ask the device directly, not to
/// accuse it: `suspect`, never `compromised`.
pub(crate) fn mark_suspect(conn: &Connection, device: &str) -> Result<()> {
    conn.execute(
        "UPDATE sync_devices SET status = 'suspect' WHERE device_id = ?1 AND status = 'operational'",
        params![device],
    )?;
    Ok(())
}
