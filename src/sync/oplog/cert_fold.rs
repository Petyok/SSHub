//! The folded record of a delegation certificate (Phase B design 6.3).
//!
//! A certificate is issued by one op and renewed by up to `CERT_MAX_RENEWALS` `CertRenew` ops. A checkpoint prune
//! deletes both, so the checkpoint carries one record per certificate: the facts of the issue op (issuer, issue
//! time, the first end date, the signature and canonical hash of the issue op) and the accepted renewals (at most
//! `CERT_MAX_RENEWALS` entries). The record is the `cert_fold` section of the state's writes: it is signed with
//! the rest of the state, and a bootstrapping device re-checks it by replaying the accepted renewals.
//!
//! The fold is a pure function of the log: renewals are judged in `(lamport, device, seq)` order and `now` is
//! never read. The reader [`cert_record`] gives `quorum::trust_grant` the effective end date and the count; the
//! use-time check against the local clock stays with the caller.

use anyhow::{ensure, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

use super::checkpoint::{fold_writes, Win};
use crate::sync::limits::{CERT_LIFETIME_MAX_SECS, CERT_MAX_RENEWALS};

/// One accepted renewal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Renewal {
    pub expiry: i64,
    pub lamport: u64,
    pub device_id: String,
    pub seq: u64,
}

/// The folded record of one certificate (hex fields as in the canonical op form).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertRecord {
    pub cert_id: String,
    pub issuer: String,
    pub issued_at: i64,
    pub original_expiry: i64,
    pub effective_expiry: i64,
    pub renewals: u32,
    pub issue_sig: String,
    pub issue_canonical_hash: String,
    /// The renewals counted so far, oldest first (never more than `CERT_MAX_RENEWALS`).
    pub accepted: Vec<Renewal>,
}

/// Fold `candidates` (any order, duplicates allowed) onto a certificate's issue facts. `revoked` is the order key
/// of the `CertRevoke` in force: a renewal after it is refused. Author rule: the issuer (the successor phone of a
/// handover joins it with the handover op).
fn fold_one(
    mut rec: CertRecord,
    candidates: impl IntoIterator<Item = Renewal>,
    revoked: Option<(u64, String, u64)>,
) -> CertRecord {
    let term = rec.original_expiry - rec.issued_at;
    let all: BTreeSet<(u64, String, u64, i64)> = candidates
        .into_iter()
        .map(|r| (r.lamport, r.device_id, r.seq, r.expiry))
        .collect();
    rec.effective_expiry = rec.original_expiry;
    rec.renewals = 0;
    rec.accepted.clear();
    for (lamport, device_id, seq, expiry) in all {
        let after_revoke = revoked
            .as_ref()
            .is_some_and(|r| (r.0, r.1.as_str(), r.2) < (lamport, device_id.as_str(), seq));
        let ok = !after_revoke
            && device_id == rec.issuer
            && term > 0
            && expiry > rec.effective_expiry
            && expiry <= rec.effective_expiry + term
            && expiry - rec.issued_at <= CERT_LIFETIME_MAX_SECS
            && rec.renewals < CERT_MAX_RENEWALS;
        if ok {
            rec.effective_expiry = expiry;
            rec.renewals += 1;
            rec.accepted.push(Renewal {
                expiry,
                lamport,
                device_id,
                seq,
            });
        }
    }
    rec
}

fn issue_record(cert_id: &str, w: &Win) -> Option<CertRecord> {
    let v = &w.value;
    Some(CertRecord {
        cert_id: cert_id.to_string(),
        issuer: v["issuer"].as_str()?.to_string(),
        issued_at: v["issued_at"].as_i64()?,
        original_expiry: v["expiry"].as_i64()?,
        effective_expiry: v["expiry"].as_i64()?,
        renewals: 0,
        issue_sig: v["issue_sig"].as_str()?.to_string(),
        issue_canonical_hash: v["issue_canonical_hash"].as_str()?.to_string(),
        accepted: Vec::new(),
    })
}

/// One folded record per certificate in `wins`: the issue write, the folded record of an earlier checkpoint
/// (`cert_fold`) and the `cert_renew` writes still in the log.
pub(super) fn fold_certs(
    wins: &BTreeMap<(String, String), Win>,
) -> BTreeMap<String, (CertRecord, &Win)> {
    let mut out = BTreeMap::new();
    for ((section, id), issue) in wins {
        if section != "cert" {
            continue;
        }
        let Some(rec) = issue_record(id, issue) else {
            continue;
        };
        let mut candidates: Vec<Renewal> = wins
            .get(&("cert_fold".to_string(), id.clone()))
            .and_then(|w| serde_json::from_value::<CertRecord>(w.value.clone()).ok())
            .map(|old| old.accepted)
            .unwrap_or_default();
        let prefix = format!("{id}:");
        for ((s, k), w) in wins {
            if s == "cert_renew" && k.starts_with(&prefix) {
                if let Some(expiry) = w.value["expiry"].as_i64() {
                    candidates.push(Renewal {
                        expiry,
                        lamport: w.lamport,
                        device_id: w.device.clone(),
                        seq: w.seq,
                    });
                }
            }
        }
        let revoked = wins
            .get(&("cert_revoked".to_string(), id.clone()))
            .map(|w| (w.lamport, w.device.clone(), w.seq));
        out.insert(id.clone(), (fold_one(rec, candidates, revoked), issue));
    }
    out
}

/// The `cert_fold` write of a state for one folded certificate. It borrows the order key of the issue write, so
/// every device derives the same bytes.
pub(super) fn state_write(rec: &CertRecord, issue: &Win) -> Value {
    json!({
        "section": "cert_fold", "key": rec.cert_id, "ord": 0, "lamport": issue.lamport,
        "device_id": issue.device, "seq": issue.seq, "value": rec,
    })
}

/// A bootstrapping device checks the folded records of a state it received: every record names an issue write
/// of the same state, matches its facts, is its own replay (the accepted renewals pass the rules in order and
/// yield exactly the effective end date and count) and carries a consistent canonical hash.
pub(super) fn verify_state_writes(writes: &[Value]) -> Result<()> {
    let issue: BTreeMap<&str, &Value> = writes
        .iter()
        .filter(|w| w["section"] == "cert")
        .filter_map(|w| Some((w["key"].as_str()?, &w["value"])))
        .collect();
    for w in writes.iter().filter(|w| w["section"] == "cert_fold") {
        let rec: CertRecord = serde_json::from_value(w["value"].clone())
            .map_err(|e| anyhow::anyhow!("a certificate record is malformed: {e}"))?;
        ensure!(
            w["key"] == rec.cert_id.as_str() && rec.cert_id == rec.issue_canonical_hash,
            "a certificate record does not match its id"
        );
        let v = issue
            .get(rec.cert_id.as_str())
            .ok_or_else(|| anyhow::anyhow!("a certificate record has no issue write"))?;
        ensure!(
            v["issuer"] == rec.issuer.as_str()
                && v["issued_at"] == rec.issued_at
                && v["expiry"] == rec.original_expiry
                && v["issue_sig"] == rec.issue_sig.as_str()
                && v["issue_canonical_hash"] == rec.issue_canonical_hash.as_str(),
            "a certificate record disagrees with its issue write"
        );
        ensure!(
            rec.accepted.len() <= CERT_MAX_RENEWALS as usize,
            "a certificate record counts too many renewals"
        );
        let base = CertRecord {
            accepted: Vec::new(),
            ..rec.clone()
        };
        ensure!(
            fold_one(base, rec.accepted.clone(), None) == rec,
            "a certificate record is not the replay of its renewals"
        );
    }
    Ok(())
}

/// The folded record of one certificate as the log stands now (all effective writes, no cut), or `None` when the
/// issue is unknown. `cert_id_hex` is the lowercase hex of the issue op hash.
pub fn cert_record(conn: &Connection, cert_id_hex: &str) -> Result<Option<CertRecord>> {
    let wins = fold_writes(conn, None, Some(cert_id_hex))?;
    Ok(fold_certs(&wins).remove(cert_id_hex).map(|(r, _)| r))
}

/// A prune turns the winners of deleted ops into base writes; a renewal write of a certificate is then replaced by
/// the folded record, so what a pruned store keeps is bounded by the renewal limit and not by the spam it saw.
pub(super) fn compact(conn: &Connection) -> Result<()> {
    let mut wins = fold_writes(conn, None, None)?;
    // Only the renewals that are base writes (below the prune floor): a renewal still in the log is folded when a
    // state is built at a cut, and one above that cut must not leak into the record.
    wins.retain(|(s, _), _| s != "cert_renew");
    let mut stmt = conn.prepare(
        "SELECT key, ord, lamport, device_id, seq, value FROM sync_writes
         WHERE section = 'cert_renew' AND op_hash = zeroblob(32)",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            Win {
                ord: r.get::<_, i64>(1)? as u64,
                lamport: r.get::<_, i64>(2)? as u64,
                device: r.get(3)?,
                seq: r.get::<_, i64>(4)? as u64,
                value: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or(Value::Null),
            },
        ))
    })?;
    for row in rows {
        let (key, w) = row?;
        wins.insert(("cert_renew".into(), key), w);
    }
    drop(stmt);
    for (id, (rec, issue)) in fold_certs(&wins) {
        let value = serde_json::to_string(&rec)?;
        conn.execute(
            "DELETE FROM sync_writes WHERE section = 'cert_renew' AND key LIKE ?1 || ':%' AND op_hash = zeroblob(32)",
            [&id],
        )?;
        conn.execute(
            "INSERT OR REPLACE INTO sync_writes (section, key, op_hash, ord, lamport, device_id, seq, value)
             VALUES ('cert_fold', ?1, zeroblob(32), 0, ?2, ?3, ?4, ?5)",
            rusqlite::params![id, issue.lamport as i64, issue.device, issue.seq as i64, value],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;

    fn base() -> CertRecord {
        CertRecord {
            cert_id: "aa".repeat(32),
            issuer: "issuer".into(),
            issued_at: 1_000,
            original_expiry: 1_000 + 10 * DAY,
            effective_expiry: 1_000 + 10 * DAY,
            renewals: 0,
            issue_sig: "bb".repeat(64),
            issue_canonical_hash: "aa".repeat(32),
            accepted: vec![],
        }
    }

    fn r(lamport: u64, device: &str, expiry: i64) -> Renewal {
        Renewal {
            expiry,
            lamport,
            device_id: device.into(),
            seq: lamport,
        }
    }

    #[test]
    fn renewals_follow_the_rules_in_log_order_whatever_the_input_order() {
        let t = |n: i64| 1_000 + n * DAY;
        // out of order input, a stranger, a shorter date, a leap over one term, then a third one
        let cands = vec![
            r(9, "issuer", t(40)),
            r(5, "issuer", t(20)),
            r(3, "stranger", t(15)),
            r(4, "issuer", t(5)),
            r(6, "issuer", t(31)),
            r(7, "issuer", t(30)),
        ];
        let rec = fold_one(base(), cands.clone(), None);
        // 20 (<= 10 + 10) ok, 31 refused (more than one term past 20), 30 ok, 40 is the third: refused
        assert_eq!((rec.effective_expiry, rec.renewals), (t(30), 2));
        assert_eq!(
            rec.accepted,
            vec![r(5, "issuer", t(20)), r(7, "issuer", t(30))]
        );
        let mut shuffled = cands;
        shuffled.reverse();
        assert_eq!(fold_one(base(), shuffled, None), rec);
        // replaying the accepted list is a fixed point (what bootstrap checks)
        assert_eq!(fold_one(base(), rec.accepted.clone(), None), rec);
    }

    #[test]
    fn a_renewal_after_the_revoke_or_past_the_lifetime_cap_is_refused() {
        let t = |n: i64| 1_000 + n * DAY;
        let revoked = Some((6, "z".to_string(), 0));
        let rec = fold_one(
            base(),
            vec![r(5, "issuer", t(20)), r(7, "issuer", t(30))],
            revoked,
        );
        assert_eq!(rec.renewals, 1);
        let mut long = base();
        long.original_expiry = long.issued_at + 100 * DAY;
        long.effective_expiry = long.original_expiry;
        let rec = fold_one(
            long,
            vec![
                r(1, "issuer", 1_000 + 190 * DAY),
                r(2, "issuer", 1_000 + 180 * DAY),
            ],
            None,
        );
        assert_eq!(
            rec.effective_expiry,
            1_000 + 180 * DAY,
            "the 190 day date is over the cap"
        );
    }

    #[test]
    fn a_forged_record_is_refused_at_bootstrap() {
        let issue = json!({ "section": "cert", "key": "aa".repeat(32), "value": {
            "issuer": "issuer", "issued_at": 1_000, "expiry": 1_000 + 10 * DAY,
            "issue_sig": "bb".repeat(64), "issue_canonical_hash": "aa".repeat(32) } });
        let rec = fold_one(base(), vec![r(5, "issuer", 1_000 + 20 * DAY)], None);
        let good = json!({ "section": "cert_fold", "key": rec.cert_id, "value": rec });
        verify_state_writes(&[issue.clone(), good.clone()]).unwrap();
        // a longer end date than the replay yields
        let mut forged = rec.clone();
        forged.effective_expiry += DAY;
        let bad = json!({ "section": "cert_fold", "key": rec.cert_id, "value": forged });
        assert!(verify_state_writes(&[issue.clone(), bad]).is_err());
        // a count that was reset (the third renewal would then be allowed)
        let mut reset = rec.clone();
        reset.renewals = 0;
        let bad = json!({ "section": "cert_fold", "key": rec.cert_id, "value": reset });
        assert!(verify_state_writes(&[issue.clone(), bad]).is_err());
        // a record with no issue write, and one that disagrees with it
        assert!(verify_state_writes(std::slice::from_ref(&good)).is_err());
        let mut other = issue.clone();
        other["value"]["issuer"] = json!("someone else");
        assert!(verify_state_writes(&[other, good]).is_err());
    }
}
