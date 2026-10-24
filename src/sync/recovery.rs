//! Phone loss and re-enrollment: the state machine that decides whether a `PhoneReenroll` may take
//! effect. Replaying the log through [`Recovery::apply`] gives the verdict; the machine itself
//! knows nothing about ops, stores or clocks beyond the timestamps it is given, so every state and
//! event pair can be tested in a table.
//!
//! A re-enrollment needs, all at once: an authorization (two `RecoveryApprove` ops from distinct
//! attended devices other than the requester, or the paper signature carried in the op, never both
//! counted from one artifact), a third factor that is a different artifact, one `RecoveryAck`
//! (the automatic "I have seen it" record) from a device other than the requester that starts the
//! 7 day clock, and proof of liveness. A live phone cancels with no factor; any other device needs
//! the third factor.

use std::collections::BTreeSet;

use anyhow::{bail, ensure, Result};

use super::limits::{RECOVERY_TIMELOCK_SECS, TIMESTAMP_SKEW_GRACE_SECS};
use super::oplog::DeviceId;

/// Approvals from distinct attended devices that authorize a recovery without paper.
pub const APPROVALS_NEEDED: usize = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReenrollVerdict {
    CancelledByPhone,
    CancelledByFactor,
    AwaitingApproval {
        have: usize,
        need: usize,
    },
    AwaitingAck,
    NeedsMoreTime {
        ready_at: i64,
    },
    Ready,
    /// The old phone is revoked and the new one enrolled: the recovery is spent.
    Completed,
}

/// What the machine is told about the op that proposes the recovery. The caller has already checked
/// the signature chain; these are the facts that need the store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proposal {
    pub hash: [u8; 32],
    pub requester: DeviceId,
    /// The op's own timestamp.
    pub timestamp: i64,
    /// `initiated_at` as written in the op: must be close to `timestamp` (no backdating).
    pub initiated_at: i64,
    pub authorization: Authorization,
    /// The third factor is a different artifact than the authorization (paper authorization needs email).
    pub third_factor_distinct: bool,
    /// The op's liveness set matches what this device knows to be fresh.
    pub liveness_ok: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Authorization {
    /// `via_paper`: the paper signature in the op verified (or not).
    Paper { signature_valid: bool },
    /// Counted from `RecoveryApprove` ops.
    Approvals,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Propose(Proposal),
    /// An approval by `by`; `attended` is whether that device is attended and not revoked.
    Approve {
        by: DeviceId,
        attended: bool,
    },
    /// "I have seen the op at `seen_at`" by `by`; `attended` is whether that device is attended
    /// (an unattended device, the Hub for one, has seen nothing and cannot start the clock).
    Ack {
        by: DeviceId,
        seen_at: i64,
        attended: bool,
    },
    /// Cancellation authored by the phone itself (`by_phone`), proving it is alive.
    CancelByPhone {
        by_phone: bool,
    },
    /// Cancellation by another device; needs the third factor.
    CancelByDevice {
        with_factor: bool,
    },
    /// The old phone is being revoked at `now`: only valid once the clock ran out.
    Complete {
        now: i64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recovery {
    proposal: Option<Proposal>,
    approvals: BTreeSet<DeviceId>,
    first_ack: Option<i64>,
    cancelled: Option<ReenrollVerdict>,
    completed: bool,
}

impl Default for Recovery {
    fn default() -> Self {
        Self::new()
    }
}

impl Recovery {
    pub fn new() -> Self {
        Self {
            proposal: None,
            approvals: BTreeSet::new(),
            first_ack: None,
            cancelled: None,
            completed: false,
        }
    }

    pub fn is_idle(&self) -> bool {
        self.proposal.is_none()
    }

    fn open(&self) -> Result<&Proposal> {
        let p = self
            .proposal
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no recovery has been proposed"))?;
        ensure!(!self.completed, "the recovery is already completed");
        ensure!(self.cancelled.is_none(), "the recovery was cancelled");
        Ok(p)
    }

    pub fn apply(&mut self, ev: Event) -> Result<()> {
        match ev {
            Event::Propose(p) => {
                ensure!(self.proposal.is_none(), "a recovery was already proposed: a replayed or second PhoneReenroll is refused");
                ensure!(
                    (p.initiated_at - p.timestamp).abs() <= TIMESTAMP_SKEW_GRACE_SECS,
                    "initiated_at is not the time of the op"
                );
                if let Authorization::Paper { signature_valid } = p.authorization {
                    ensure!(signature_valid, "the paper signature does not verify");
                }
                ensure!(
                    p.third_factor_distinct,
                    "the third factor must be a different artifact than the authorization"
                );
                ensure!(
                    p.liveness_ok,
                    "the liveness set excludes a device known to be fresh"
                );
                self.proposal = Some(p);
            }
            Event::Approve { by, attended } => {
                let p = self.open()?;
                ensure!(
                    p.authorization == Authorization::Approvals,
                    "this recovery is authorized by paper: approvals do not count"
                );
                ensure!(
                    by != p.requester,
                    "the requester cannot approve its own recovery"
                );
                ensure!(attended, "only attended devices approve a recovery");
                self.approvals.insert(by);
            }
            Event::Ack {
                by,
                seen_at,
                attended,
            } => {
                let p = self.open()?;
                ensure!(attended, "only an attended device acknowledges a recovery: an unattended device has seen nothing");
                ensure!(
                    by != p.requester,
                    "the requester cannot acknowledge its own recovery"
                );
                ensure!(
                    seen_at >= p.timestamp - TIMESTAMP_SKEW_GRACE_SECS,
                    "an acknowledgement cannot predate the op it acknowledges"
                );
                self.first_ack = Some(self.first_ack.map_or(seen_at, |t| t.min(seen_at)));
            }
            Event::CancelByPhone { by_phone } => {
                // The phone's own signature proves it is alive, so it outranks a cancellation
                // that needed the third factor, whichever arrived first.
                ensure!(self.proposal.is_some(), "no recovery has been proposed");
                ensure!(!self.completed, "the recovery is already completed");
                ensure!(
                    by_phone,
                    "only the phone's own signature cancels without a factor"
                );
                self.cancelled = Some(ReenrollVerdict::CancelledByPhone);
            }
            Event::CancelByDevice { with_factor } => {
                self.open()?;
                ensure!(
                    with_factor,
                    "a cancellation from a device needs the third factor"
                );
                self.cancelled = Some(ReenrollVerdict::CancelledByFactor);
            }
            Event::Complete { now } => {
                self.open()?;
                let v = self.verdict(now);
                ensure!(
                    v == ReenrollVerdict::Ready,
                    "the recovery cannot complete: {v:?}"
                );
                self.completed = true;
            }
        }
        Ok(())
    }

    /// Where the recovery stands at `now`. Idle machines have no verdict: ask [`Recovery::is_idle`].
    pub fn verdict(&self, now: i64) -> ReenrollVerdict {
        if self.completed {
            return ReenrollVerdict::Completed;
        }
        if let Some(c) = &self.cancelled {
            return c.clone();
        }
        let Some(p) = &self.proposal else {
            return ReenrollVerdict::AwaitingApproval {
                have: 0,
                need: APPROVALS_NEEDED,
            };
        };
        if p.authorization == Authorization::Approvals && self.approvals.len() < APPROVALS_NEEDED {
            return ReenrollVerdict::AwaitingApproval {
                have: self.approvals.len(),
                need: APPROVALS_NEEDED,
            };
        }
        let Some(first) = self.first_ack else {
            return ReenrollVerdict::AwaitingAck;
        };
        let ready_at = first + RECOVERY_TIMELOCK_SECS;
        if now < ready_at {
            ReenrollVerdict::NeedsMoreTime { ready_at }
        } else {
            ReenrollVerdict::Ready
        }
    }
}

/// Rejects an event stream that does not start with a proposal.
pub fn replay(events: impl IntoIterator<Item = Event>) -> Result<Recovery> {
    let mut r = Recovery::new();
    for e in events {
        if r.is_idle() && !matches!(e, Event::Propose(_)) {
            bail!("an event arrived before any proposal");
        }
        r.apply(e)?;
    }
    Ok(r)
}

// ------------------------------------------------------------------ over the log

use super::crypto::{verify, DeviceIdentity, ALG_CURRENT};
use super::limits::LIVE_DEVICE_WINDOW_SECS;
use super::oplog::{DeviceKind, DeviceRow, DeviceStatus, HeadAttest, Op, OpBody, ThirdFactor};
use super::quorum::{paper_recover_pub, phone_device};
use crate::store::LauncherStore;

const PAPER_DOMAIN: &[u8] = b"sshub/sync/reenroll/paper/v1";

/// The bytes the paper/recover key signs for a re-enrollment: a domain tag, the requester and the
/// op body without its `paper_sig` under a fixed header, so the signature does not depend on the
/// requester's chain position (the op does not exist yet when the paper is typed).
pub fn paper_reenroll_preimage(requester: &DeviceId, body: &OpBody) -> Vec<u8> {
    let mut body = body.clone();
    if let OpBody::PhoneReenroll { paper_sig, .. } = &mut body {
        *paper_sig = None;
    }
    let header = Op {
        device_id: requester.clone(),
        seq: 0,
        lamport: 0,
        prev_hash: [0; 32],
        timestamp: 0,
        epoch: 0,
        alg: ALG_CURRENT,
        body,
        signature: [0; 64],
    };
    [
        PAPER_DOMAIN,
        requester.as_bytes(),
        &header.canonical_bytes(),
    ]
    .concat()
}

/// `paper_sig` for a `PhoneReenroll` body, made on the machine where the phrase was typed.
pub fn paper_reenroll_sign(
    paper_recover: &DeviceIdentity,
    requester: &DeviceId,
    body: &OpBody,
) -> [u8; 64] {
    paper_recover.sign(&paper_reenroll_preimage(requester, body))
}

/// The head attestations a requester puts into `PhoneReenroll.liveness`: itself and every device it
/// knows to be fresh, each with the chain head it holds.
pub fn liveness_set(store: &LauncherStore, me: &DeviceId, now: i64) -> Result<Vec<HeadAttest>> {
    let heads = store.sync_chain_heads()?;
    let mut ids: Vec<DeviceId> = fresh_devices(store, now)?
        .into_iter()
        .map(|d| d.id)
        .collect();
    if !ids.contains(me) {
        ids.push(me.clone());
    }
    Ok(ids
        .into_iter()
        .map(|id| {
            let (seq, chain_hash) = heads
                .iter()
                .find(|(d, _, _)| *d == id)
                .map_or((0, [0; 32]), |(_, s, h)| (*s, *h));
            HeadAttest {
                device_id: id,
                seq,
                chain_hash,
            }
        })
        .collect())
}

fn fresh_devices(store: &LauncherStore, now: i64) -> Result<Vec<DeviceRow>> {
    Ok(super::ceremony::effective_devices(store, None)?
        .into_iter()
        .filter(|d| {
            !matches!(d.status, DeviceStatus::Revoked | DeviceStatus::Compromised)
                && now - d.last_seen <= LIVE_DEVICE_WINDOW_SECS
        })
        .collect())
}

fn attended_live(row: &DeviceRow) -> bool {
    // Approvers and ackers are holders in force: a mirror (or a device whose promotion never completed)
    // is not a participant of a recovery.
    row.role == super::oplog::DeviceRole::Holder
        && row.attendance == super::oplog::Attendance::Attended
        && !matches!(
            row.status,
            DeviceStatus::Revoked | DeviceStatus::Compromised
        )
}

/// Ops of one kind in any state but rejected or void, in log order: a recovery vote is a signed
/// statement whether or not this device's user has reviewed it yet.
fn signed_ops(store: &LauncherStore, kind: &str) -> Result<Vec<Op>> {
    store.with_conn(|c| {
        let mut st = c.prepare(
            "SELECT canonical, signature FROM sync_ops
             WHERE kind = ?1 AND state NOT IN ('rejected','void') ORDER BY lamport, device_id, seq",
        )?;
        let rows = st
            .query_map(rusqlite::params![kind], |r| {
                Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(c, s)| {
                Op::from_canonical(
                    &c,
                    s.try_into()
                        .map_err(|_| anyhow::anyhow!("stored signature"))?,
                )
            })
            .collect()
    })
}

/// Where the re-enrollment proposed by `op` stands at `now`, judged from the log in `store`.
/// A bad proposal (backdated, same factor twice, bad paper signature, a liveness set that leaves out
/// a fresh device, a replay of an earlier proposal) is an error. Events that do not qualify (an
/// unattended acknowledgement, a cancellation without the factor) are ignored, not errors: anyone
/// can send them.
pub fn validate_phone_reenroll(
    store: &LauncherStore,
    op: &Op,
    now: i64,
) -> Result<ReenrollVerdict> {
    let OpBody::PhoneReenroll {
        initiated_at,
        via_paper,
        third_factor,
        liveness,
        paper_sig,
        ..
    } = &op.body
    else {
        bail!("not a PhoneReenroll op");
    };
    let hash = op.hash();
    let devices = super::ceremony::effective_devices(store, None)?;
    let requester = devices
        .iter()
        .find(|d| d.id == op.device_id)
        .ok_or_else(|| anyhow::anyhow!("the requester is not a device of this ring"))?;
    ensure!(
        attended_live(requester),
        "an unattended or revoked device cannot request a recovery"
    );
    ensure!(
        requester.kind == DeviceKind::Terminal,
        "the phone does not request its own replacement"
    );

    // A second proposal with the same content is a replay, whatever op carries it.
    for other in signed_ops(store, "PhoneReenroll")? {
        if other.hash() != hash && other.body == op.body {
            ensure!(
                (other.lamport, &other.device_id, other.seq) > (op.lamport, &op.device_id, op.seq),
                "this PhoneReenroll repeats an earlier one: replays are refused"
            );
        }
    }

    // Authorization and the third factor.
    let paper_ok = match (via_paper, paper_sig) {
        (true, Some(sig)) => paper_recover_pub(store)?
            .is_some_and(|k| verify(&k, &paper_reenroll_preimage(&op.device_id, &op.body), sig)),
        (true, None) => false,
        (false, None) => true,
        (false, Some(_)) => {
            bail!("a paper signature on a recovery that is not authorized by paper")
        }
    };
    let factor_distinct = match third_factor {
        ThirdFactor::EmailCode => true,
        ThirdFactor::Paper => !*via_paper,
        #[allow(unreachable_patterns)]
        _ => false,
    };

    // Liveness: every device this store knows to be fresh must be in the set, and where the set and
    // this store hold the same chain position they must agree on the head.
    let heads = store.sync_chain_heads()?;
    let mut liveness_ok = true;
    for d in fresh_devices(store, *initiated_at)? {
        match liveness.iter().find(|a| a.device_id == d.id) {
            None => liveness_ok = false,
            Some(a) => {
                if let Some((_, seq, h)) = heads.iter().find(|(id, _, _)| *id == d.id) {
                    if a.seq == *seq && a.chain_hash != *h {
                        liveness_ok = false;
                    }
                }
            }
        }
    }

    let mut m = Recovery::new();
    m.apply(Event::Propose(Proposal {
        hash,
        requester: op.device_id.clone(),
        timestamp: super::ceremony::base_time(store, op),
        initiated_at: *initiated_at,
        authorization: if *via_paper {
            Authorization::Paper {
                signature_valid: paper_ok,
            }
        } else {
            Authorization::Approvals
        },
        third_factor_distinct: factor_distinct,
        liveness_ok,
    }))?;

    let phone = phone_device(store)?;
    let row = |id: &DeviceId| devices.iter().find(|d| d.id == *id);
    let mut events: Vec<(u64, DeviceId, u64, Event)> = Vec::new();
    for a in signed_ops(store, "RecoveryApprove")? {
        if let OpBody::RecoveryApprove { reenroll_hash } = &a.body {
            if *reenroll_hash == hash {
                let attended = row(&a.device_id).is_some_and(attended_live);
                events.push((
                    a.lamport,
                    a.device_id.clone(),
                    a.seq,
                    Event::Approve {
                        by: a.device_id.clone(),
                        attended,
                    },
                ));
            }
        }
    }
    for a in signed_ops(store, "RecoveryAck")? {
        if let OpBody::RecoveryAck {
            reenroll_hash,
            seen_at,
        } = &a.body
        {
            if *reenroll_hash == hash {
                let attended = row(&a.device_id).is_some_and(attended_live);
                events.push((
                    a.lamport,
                    a.device_id.clone(),
                    a.seq,
                    Event::Ack {
                        by: a.device_id.clone(),
                        // The clock is this device's own first sight of the ack, never earlier than the
                        // claim: a backdated acknowledgement cannot start the time lock early.
                        seen_at: super::ceremony::received_at(store, &a)
                            .map_or(*seen_at, |r| r.max(*seen_at)),
                        attended,
                    },
                ));
            }
        }
    }
    for c in signed_ops(store, "PhoneReenrollCancel")? {
        if let OpBody::PhoneReenrollCancel {
            reenroll_hash,
            by_phone,
            third_factor,
        } = &c.body
        {
            if *reenroll_hash == hash {
                let ev = if *by_phone {
                    Event::CancelByPhone {
                        by_phone: phone.as_ref() == Some(&c.device_id),
                    }
                } else {
                    Event::CancelByDevice {
                        with_factor: third_factor.is_some()
                            && row(&c.device_id).is_some_and(attended_live),
                    }
                };
                events.push((c.lamport, c.device_id.clone(), c.seq, ev));
            }
        }
    }
    events.sort_by(|a, b| (a.0, &a.1, a.2).cmp(&(b.0, &b.1, b.2)));
    for (_, _, _, ev) in events {
        let _ = m.apply(ev); // an event that does not qualify changes nothing
    }
    Ok(m.verdict(now))
}

// ------------------------------------------------------------------ builders for the UI

/// The `PhoneReenroll` op body a terminal proposes. `via_paper` authorizes by the paper phrase
/// (typed on this machine, `paper_recover` is the identity derived from it) and then the third factor
/// must be the emailed code; otherwise two other attended devices authorize with `RecoveryApprove`.
pub fn start_body(
    store: &LauncherStore,
    me: &DeviceIdentity,
    now: i64,
    paper_recover: Option<&DeviceIdentity>,
    third_factor: ThirdFactor,
) -> Result<OpBody> {
    let via_paper = paper_recover.is_some();
    ensure!(
        !(via_paper && third_factor == ThirdFactor::Paper),
        "paper authorizes this recovery, so the third factor must be the emailed code"
    );
    let mut body = OpBody::PhoneReenroll {
        initiated_at: now,
        signers: Vec::new(),
        via_paper,
        third_factor,
        liveness: liveness_set(store, &me.public().device_id(), now)?,
        paper_sig: None,
    };
    if let Some(recover) = paper_recover {
        let sig = paper_reenroll_sign(recover, &me.public().device_id(), &body);
        if let OpBody::PhoneReenroll { paper_sig, .. } = &mut body {
            *paper_sig = Some(sig);
        }
    }
    Ok(body)
}

/// An attended device's vote, after its user confirmed the Red screen.
pub fn approve_body(reenroll: &Op) -> Result<OpBody> {
    ensure!(
        matches!(reenroll.body, OpBody::PhoneReenroll { .. }),
        "not a PhoneReenroll op"
    );
    Ok(OpBody::RecoveryApprove {
        reenroll_hash: reenroll.hash(),
    })
}

/// "I have seen this recovery at `now`": appended by an attended device when it displayed the op.
pub fn ack_body(reenroll: &Op, now: i64) -> Result<OpBody> {
    ensure!(
        matches!(reenroll.body, OpBody::PhoneReenroll { .. }),
        "not a PhoneReenroll op"
    );
    Ok(OpBody::RecoveryAck {
        reenroll_hash: reenroll.hash(),
        seen_at: now,
    })
}

/// Cancel a recovery: by the live phone (no factor) or by a device with the third factor.
pub fn cancel_body(reenroll: &Op, by_phone: bool, factor: Option<ThirdFactor>) -> Result<OpBody> {
    ensure!(
        matches!(reenroll.body, OpBody::PhoneReenroll { .. }),
        "not a PhoneReenroll op"
    );
    ensure!(
        by_phone || factor.is_some(),
        "a cancellation from a device needs the third factor"
    );
    Ok(OpBody::PhoneReenrollCancel {
        reenroll_hash: reenroll.hash(),
        by_phone,
        third_factor: if by_phone { None } else { factor },
    })
}

/// Recoveries this device (attended, not the requester) has not acknowledged yet and that are still
/// open. The UI shows each one and, once it is on screen, appends [`ack_body`].
pub fn unacknowledged(store: &LauncherStore, me: &DeviceIdentity, now: i64) -> Result<Vec<Op>> {
    let mine = me.public().device_id();
    let devices = super::ceremony::effective_devices(store, None)?;
    if devices.iter().any(|d| d.id == mine && !attended_live(d)) {
        return Ok(Vec::new());
    }
    let acks: Vec<[u8; 32]> = signed_ops(store, "RecoveryAck")?
        .into_iter()
        .filter(|a| a.device_id == mine)
        .filter_map(|a| match a.body {
            OpBody::RecoveryAck { reenroll_hash, .. } => Some(reenroll_hash),
            _ => None,
        })
        .collect();
    let mut out = Vec::new();
    for op in signed_ops(store, "PhoneReenroll")? {
        if op.device_id == mine || acks.contains(&op.hash()) {
            continue;
        }
        if matches!(
            validate_phone_reenroll(store, &op, now),
            Ok(ReenrollVerdict::CancelledByPhone
                | ReenrollVerdict::CancelledByFactor
                | ReenrollVerdict::Completed)
                | Err(_)
        ) {
            continue;
        }
        out.push(op);
    }
    Ok(out)
}

/// The ops that finish a recovery whose clock ran out: revoke the old phone and admit the new one
/// (enrolled beforehand, see `phone_enroll`). The caller then reshares to the new holder set
/// (`revocation::plan`) and calls [`clear_snapshots`].
pub fn completion_bodies(
    store: &LauncherStore,
    reenroll: &Op,
    now: i64,
    new_phone: &super::crypto::DevicePublic,
    name: &str,
) -> Result<Vec<OpBody>> {
    let verdict = validate_phone_reenroll(store, reenroll, now)?;
    ensure!(
        verdict == ReenrollVerdict::Ready,
        "the recovery is not ready: {verdict:?}"
    );
    let old =
        phone_device(store)?.ok_or_else(|| anyhow::anyhow!("the ring has no phone to replace"))?;
    Ok(vec![
        OpBody::DeviceRevoke { device_id: old },
        OpBody::DeviceAdd {
            device: new_phone.clone(),
            name: name.to_string(),
            role: super::oplog::DeviceRole::Holder,
            attendance: super::oplog::Attendance::Attended,
            kind: DeviceKind::Phone,
            verified_by: super::oplog::VerifiedBy::Sas,
        },
    ])
}

/// Forget every stored short-code snapshot (they hold the OLD phone's shares). Returns how many.
pub fn clear_snapshots(store: &LauncherStore) -> Result<usize> {
    store.with_conn(|c| Ok(c.execute("DELETE FROM sync_meta WHERE key LIKE 'snapshot:%'", [])?))
}

#[cfg(test)]
mod tests;
