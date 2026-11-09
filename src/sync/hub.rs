//! The Hub, Rust side. There is no hub daemon and no `hub-serve` verb: a Hub is an ordinary ring
//! device (a mirror, unattended) that a terminal reaches with `ssh hub sshub sync-serve`, behind
//! the sshd of `deploy/hub/`. What makes it a Hub is what it is NOT allowed to do, enforced where
//! the decisions are made:
//!
//! * it never votes: approvals and acknowledgements of a recovery count only from attended devices
//!   (`recovery::Event`), it holds no shares as a mirror, and as an unattended holder it releases a
//!   share only under a valid trust certificate (`quorum::release_share_for`);
//! * it announces recovery events out of band (a notification opens nothing, so it needs no
//!   special credentials): [`announce_recovery_ops`], at most once per op, never with a code;
//! * it relays phone courier ops like any other ops ([`courier_ingest`]).
//!
//! The ring works without it, only worse: nothing here is on the path of any check on another device.

use std::fmt::Write as _;

use anyhow::{bail, Result};
use rusqlite::params;

use super::crypto::DeviceIdentity;
use super::oplog::{DeviceId, DeviceKind, DeviceStatus, Op, OpBody};
use super::protocol::{ingest_ops, run_sync_serve, RoundSummary, SyncLink};
use super::quorum::{meta_get, meta_set};
use super::wire::{
    PhoneOpsWire, RecoveryNoticeWire, SignedOpBytes, TerminalsBlock, MAX_COURIER_OPS,
};
use crate::config::NotifyConfig;
use crate::store::{uid_hex, LauncherStore};

/// Op kinds that are announced: a recovery started, someone approved it, it was cancelled.
/// `RecoveryAck` records that an attended terminal showed the recovery to its user (it is not automatic): not announced.
const ANNOUNCED: [&str; 3] = ["PhoneReenroll", "RecoveryApprove", "PhoneReenrollCancel"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Announcement {
    pub op_hash: [u8; 32],
    pub subject: String,
    pub body: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct AnnounceReport {
    pub sent: usize,
    /// Sends that failed; they are retried at the next call, never counted as sent.
    pub failed: usize,
}

fn notified_key(hash: &[u8; 32]) -> String {
    format!("notified:{}", uid_hex(&hash[..16].try_into().unwrap()))
}

/// `YYYY-MM-DD HH:MM UTC` of a unix time.
pub fn utc(ts: i64) -> String {
    let days = ts.div_euclid(86_400);
    let secs = ts.rem_euclid(86_400);
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        secs / 3600,
        (secs % 3600) / 60
    )
}

fn name_of(store: &LauncherStore, id: &DeviceId) -> String {
    store
        .sync_devices()
        .ok()
        .and_then(|d| {
            d.into_iter()
                .find(|r| r.id == *id)
                .map(|r| format!("{} ({})", super::ceremony::plain_text(&r.name), r.id))
        })
        .unwrap_or_else(|| id.clone())
}

/// Announcements for recovery ops this device received from others and has not announced yet.
pub fn pending_announcements(store: &LauncherStore) -> Result<Vec<Announcement>> {
    type Row = (Vec<u8>, String, Vec<u8>, Vec<u8>, String, i64);
    let rows: Vec<Row> = store.with_conn(|c| {
        let mut st = c.prepare(
            "SELECT op_hash, device_id, canonical, signature, kind, received_at FROM sync_ops
             WHERE local = 0 AND state != 'rejected' AND kind IN (?1, ?2, ?3)
             ORDER BY received_at, lamport, device_id, seq",
        )?;
        let rows = st
            .query_map(params![ANNOUNCED[0], ANNOUNCED[1], ANNOUNCED[2]], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    })?;
    let mut out = Vec::new();
    for (hash, author, canonical, sig, kind, at) in rows {
        let Ok(op_hash) = <[u8; 32]>::try_from(hash.as_slice()) else {
            continue;
        };
        if meta_get(store, &notified_key(&op_hash))?.is_some() {
            continue;
        }
        let op = sig
            .try_into()
            .ok()
            .and_then(|s: [u8; 64]| Op::from_canonical(&canonical, s).ok());
        let who = name_of(store, &author);
        let short = hex::encode(&op_hash[..4]);
        let mut body = String::new();
        let subject = match (kind.as_str(), op.as_ref().map(|o| &o.body)) {
            ("PhoneReenroll", Some(OpBody::PhoneReenroll { via_paper, .. })) => {
                let how = if *via_paper {
                    "the paper phrase"
                } else {
                    "two other devices"
                };
                let _ = write!(
                    body,
                    "{who} asked to replace the phone of this sshub ring (recovery {short}), authorized by {how}.\n\n\
                     The 7 day waiting period starts when an attended device acknowledges it.\n\
                     If you did not start this and your phone is alive, open sshub on a device you trust and cancel it with the phone.\n\
                     If the phone is lost, do nothing and let it run, or cancel it with your third factor."
                );
                "[sshub] a phone replacement was started".to_string()
            }
            ("PhoneReenrollCancel", Some(OpBody::PhoneReenrollCancel { by_phone, .. })) => {
                let by = if *by_phone {
                    "the phone itself"
                } else {
                    "a device with your paper phrase or emailed code"
                };
                let _ = write!(body, "{who} cancelled a phone replacement (recovery {short}); it was signed by {by}.");
                "[sshub] a phone replacement was cancelled".to_string()
            }
            ("RecoveryApprove", _) => {
                let _ = write!(body, "{who} approved the phone replacement {short}.");
                "[sshub] a phone replacement was approved".to_string()
            }
            _ => {
                let _ = write!(
                    body,
                    "{who} sent a request to replace your phone ({kind}, {short})."
                );
                "[sshub] a request to replace your phone arrived".to_string()
            }
        };
        let _ = write!(
            body,
            "\n\nReceived {}. This message contains no code and opens nothing.",
            utc(at)
        );
        out.push(Announcement {
            op_hash,
            subject,
            body,
        });
    }
    Ok(out)
}

/// Send what [`pending_announcements`] found through `send`, marking an op only after its send
/// succeeded: at most once per op, and a failing mail server blocks nobody.
pub fn announce_recovery_ops(
    store: &LauncherStore,
    send: &mut dyn FnMut(&str, &str) -> Result<()>,
) -> Result<AnnounceReport> {
    let mut report = AnnounceReport::default();
    for a in pending_announcements(store)? {
        match send(&a.subject, &a.body) {
            Ok(()) => {
                meta_set(store, &notified_key(&a.op_hash), b"1")?;
                report.sent += 1;
            }
            Err(_) => report.failed += 1,
        }
    }
    Ok(report)
}

/// `run_sync_serve`, then the announcements when `[sync.notify]` is configured. A notification
/// failure never turns a good round into an error and never reaches stdout (it carries frames).
pub fn run_sync_serve_notify<L: SyncLink>(
    store: &LauncherStore,
    id: &DeviceIdentity,
    link: &mut L,
    notify: Option<&NotifyConfig>,
) -> Result<RoundSummary> {
    let round = run_sync_serve(store, id, link);
    if let Some(cfg) = notify.filter(|c| c.command.is_some()) {
        let sent = announce_recovery_ops(store, &mut |subject, body| {
            super::notify::send_notification(cfg, subject, body)
        });
        if let Err(e) = sent {
            eprintln!("sshub: could not announce recovery operations: {e:#}");
        }
        // This device may be the sender of the emailed third factor: mail the code once the other proofs are in.
        let mailed = super::ceremony::send_owed_codes(
            store,
            id,
            crate::store::now_ts(),
            &mut |subject, body| super::notify::send_notification(cfg, subject, body),
        );
        if let Err(e) = mailed {
            eprintln!("sshub: could not mail a factor code: {e:#}");
        }
    }
    round
}

/// Courier ops the phone handed a terminal (a reply extension or a phone-ops frame): decoded
/// strictly, chained and ingested like any ops, so they reach every peer, the Hub included, with the
/// next round. Malformed input is an error and changes nothing.
pub fn courier_ingest(store: &LauncherStore, ops: &[SignedOpBytes]) -> Result<RoundSummary> {
    if ops.len() > MAX_COURIER_OPS {
        bail!("a phone carries at most {MAX_COURIER_OPS} changes");
    }
    let decoded = ops
        .iter()
        .map(|(c, s)| Op::from_canonical(c, *s))
        .collect::<Result<Vec<_>>>()?;
    let mut sum = RoundSummary {
        received: decoded.len(),
        ..Default::default()
    };
    ingest_ops(store, decoded, &mut sum)?;
    Ok(sum)
}

/// The kind 5 list a terminal shows a restored phone so it can take its own chain back: the newest (at most 8) ops this
/// terminal holds for `phone`, carrying this terminal's own device id as the UNSIGNED sender (the phone needs two lists from
/// two different terminals; a phone with no ring cannot check a signature, so the id only tells two lists apart).
pub fn phone_chain_wire(store: &LauncherStore, phone: &str, me: &str) -> Result<PhoneOpsWire> {
    let phone = phone.trim().to_lowercase();
    let sender: [u8; 8] = hex::decode(me)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| anyhow::anyhow!("this terminal's device id is not 16 hex characters"))?;
    let Some((_, head, _)) = store
        .sync_chain_heads()?
        .into_iter()
        .find(|(d, _, _)| *d == phone)
    else {
        bail!("this computer holds no change of that phone yet: sync, or ask another computer");
    };
    let ops = store.sync_chain_tail(&phone, head.saturating_sub(MAX_COURIER_OPS as u64))?;
    // The unsigned count the restored phone shows next to the list (contract 4j): terminals not removed, this one included,
    // attended or not, holders or mirrors.
    let rows = store.sync_devices()?;
    let mut terminals = rows
        .iter()
        .filter(|d| {
            d.kind == DeviceKind::Terminal
                && !matches!(d.status, DeviceStatus::Revoked | DeviceStatus::Compromised)
        })
        .count();
    if !rows.iter().any(|d| d.id == me) {
        terminals += 1;
    }
    Ok(PhoneOpsWire {
        ops: ops
            .iter()
            .map(|o| (o.canonical_bytes(), o.signature))
            .collect(),
        sender: Some(sender),
        terminals: Some(TerminalsBlock {
            count: u8::try_from(terminals).unwrap_or(u8::MAX),
            newest_op_ts: ops.iter().map(|o| o.timestamp).max().unwrap_or(0),
        }),
    })
}

/// The recovery notice a terminal shows the phone (QR, kind 3): the `PhoneReenroll` op as stored.
pub fn recovery_notice(
    store: &LauncherStore,
    reenroll_hash: &[u8; 32],
) -> Result<RecoveryNoticeWire> {
    let (canonical, sig): (Vec<u8>, Vec<u8>) = store.with_conn(|c| {
        Ok(c.query_row(
            "SELECT canonical, signature FROM sync_ops WHERE op_hash = ?1 AND kind = 'PhoneReenroll'",
            params![&reenroll_hash[..]],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    })?;
    Ok(RecoveryNoticeWire {
        op_canonical: canonical,
        op_signature: sig
            .try_into()
            .map_err(|_| anyhow::anyhow!("stored signature"))?,
    })
}

/// The one op of the log whose hash starts with `prefix` (lowercase hex, at least 6 digits).
pub fn op_by_hash_prefix(store: &LauncherStore, prefix: &str) -> Result<Op> {
    let p = prefix.to_lowercase();
    if p.len() < 6 || !p.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("'{prefix}' is not a change id (at least 6 hex digits, see `sshub sync-review`)");
    }
    let rows: Vec<(Vec<u8>, Vec<u8>)> = store.with_conn(|c| {
        let mut stmt = c.prepare(
            "SELECT canonical, signature FROM sync_ops WHERE lower(hex(op_hash)) LIKE ?1 || '%' LIMIT 3",
        )?;
        let it = stmt.query_map(params![p], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(it.collect::<rusqlite::Result<Vec<_>>>()?)
    })?;
    match rows.as_slice() {
        [] => bail!("no change matches '{prefix}'"),
        [(canonical, sig)] => Op::from_canonical(
            canonical,
            sig.as_slice()
                .try_into()
                .map_err(|_| anyhow::anyhow!("stored signature"))?,
        ),
        _ => bail!("'{prefix}' matches several changes; use more characters"),
    }
}

/// The notice (kind 3) of any op the phone is to judge: the op as signed. For a Red op the phone
/// shows what it is asked to approve and signs the ceremony proof.
pub fn op_notice(op: &Op) -> RecoveryNoticeWire {
    RecoveryNoticeWire {
        op_canonical: op.canonical_bytes(),
        op_signature: op.signature,
    }
}

#[cfg(test)]
mod tests;
