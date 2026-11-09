//! The handover facts of the log (Phase B design 5): which phone handed its place to which, read from the
//! `handover` writes of the effective `PhoneHandover` ops. A fact survives a prune and travels in the checkpoint
//! state (it is a write), so `successor_of` works on a device that bootstrapped. Whether a handover is the one
//! that wins among several by one author is the ceremony's call (lane C); this module lists what is effective.

use anyhow::Result;
use rusqlite::Connection;
use std::collections::BTreeMap;

use super::checkpoint::{fold_writes, Win};
use super::DeviceId;

/// One effective `PhoneHandover`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandoverFact {
    pub op_hash: String,
    pub old_phone: DeviceId,
    pub new_phone: DeviceId,
    pub name: String,
    /// The op's own order key (the effect lamport of the ceremony is not stored here).
    pub lamport: u64,
    pub device: DeviceId,
    pub seq: u64,
}

/// One `handover` write (key = op hash) as a fact.
pub(super) fn fact(
    key: &str,
    lamport: u64,
    device: &str,
    seq: u64,
    v: &serde_json::Value,
) -> Option<HandoverFact> {
    Some(HandoverFact {
        op_hash: key.to_string(),
        old_phone: v["old_phone"].as_str()?.to_string(),
        new_phone: v["new_phone"].as_str()?.to_string(),
        name: v["name"].as_str()?.to_string(),
        lamport,
        device: device.to_string(),
        seq,
    })
}

pub(super) fn sorted(mut facts: Vec<HandoverFact>) -> Vec<HandoverFact> {
    facts.sort_by(|a, b| (a.lamport, &a.device, a.seq).cmp(&(b.lamport, &b.device, b.seq)));
    facts
}

pub(super) fn facts(wins: &BTreeMap<(String, String), Win>) -> Vec<HandoverFact> {
    sorted(
        wins.iter()
            .filter(|((s, _), _)| s == "handover")
            .filter_map(|((_, key), w)| fact(key, w.lamport, &w.device, w.seq, &w.value))
            .collect(),
    )
}

/// The phone that holds a certificate issued by `issuer` for a renewal ordered at `before`: `issuer` itself, or
/// the end of the chain of handovers ordered before it.
pub(super) fn holder_at(
    issuer: &str,
    facts: &[HandoverFact],
    before: (u64, &str, u64),
) -> DeviceId {
    let mut cur = issuer.to_string();
    for h in facts {
        if (h.lamport, h.device.as_str(), h.seq) < before && h.old_phone == cur {
            cur = h.new_phone.clone();
        }
    }
    cur
}

/// Every effective handover, oldest first by `(lamport, device, seq)`.
pub fn handovers(conn: &Connection) -> Result<Vec<HandoverFact>> {
    Ok(facts(&fold_writes(conn, None, None)?))
}

/// The phone that took over from `phone` (following a chain of handovers to its end), or `None`.
pub fn successor_of(conn: &Connection, phone: &str) -> Result<Option<DeviceId>> {
    let mut cur = phone.to_string();
    for h in handovers(conn)? {
        if h.old_phone == cur {
            cur = h.new_phone;
        }
    }
    Ok((cur != phone).then_some(cur))
}
