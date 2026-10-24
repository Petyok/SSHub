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
    /// "I have seen the op at `seen_at`" by `by`.
    Ack {
        by: DeviceId,
        seen_at: i64,
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
            Event::Ack { by, seen_at } => {
                let p = self.open()?;
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
                self.open()?;
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

#[cfg(test)]
mod tests;
