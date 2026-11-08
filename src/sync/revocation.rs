//! Revoking a holder without leaking its shares (C14), as a driver every remaining holder runs after
//! each round. The Red ceremony authorizes the `DeviceRevoke`; everything after it is cryptographic
//! bookkeeping that needs no further prompt:
//!
//! 1. every remaining software holder rotates its wrap keys and announces them (`WrapKeyRotate`);
//! 2. once all have (or after [`ROTATION_GRACE_SECS`]), each contributes sub-shares for the new
//!    holder set, wrapped to the NEW keys (`reshare::reissue_plan`); the paper is a permanent holder;
//! 3. each new holder combines two contributors' sub-shares and publishes its envelope
//!    (`reshare::reissue_receive`);
//! 4. only when every credential was confirmed by a real unlock (never on a timer) each holder retires its
//!    previous wrap keys.
//!
//! The functions read the log, so a device that was offline simply finds its next step when it returns.
//! Nothing here persists the identity file: after [`perform`] returned a `WrapKeyRotate` or `Retire`
//! the caller must save the changed `DeviceIdentity` BEFORE appending the op.

use std::collections::BTreeSet;

use anyhow::{bail, ensure, Result};

use super::crypto::DeviceIdentity;
use super::oplog::{DeviceId, DeviceKind, DeviceRole, DeviceStatus, OpBody};
use super::quorum::{ops_of, paper_pub, reshare_ops};
use super::reshare::{next_epoch, reissue_plan, reissue_receive, unadopted};
use crate::store::LauncherStore;

/// How long the others wait for a holder that has not rotated yet.
pub const ROTATION_GRACE_SECS: i64 = 24 * 3600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevocationAction {
    /// Rotate this device's wrap keys and announce them.
    Rotate,
    /// Contribute sub-shares at `epoch` for `new_holders`.
    Contribute {
        epoch: u64,
        new_holders: Vec<DeviceId>,
    },
    /// Combine the sub-shares addressed to this device at `epoch`.
    Receive { epoch: u64 },
    /// Every credential was adopted: drop the previous wrap keys.
    Retire,
}

fn live_holders(store: &LauncherStore) -> Result<Vec<super::oplog::DeviceRow>> {
    Ok(super::ceremony::effective_devices(store, None)?
        .into_iter()
        .filter(|d| {
            d.role == DeviceRole::Holder
                && !matches!(d.status, DeviceStatus::Revoked | DeviceStatus::Compromised)
        })
        .collect())
}

/// Terminals (computers and servers, not the phone) that hold shares and are not removed.
pub fn live_terminal_holders(store: &LauncherStore) -> Result<usize> {
    Ok(live_holders(store)?
        .iter()
        .filter(|d| d.kind == DeviceKind::Terminal)
        .count())
}

/// How many terminals a ring needs before a stolen one can be cut off: the one removed and two that remain
/// to contribute (the phone approves, it does not contribute).
pub const TERMINALS_TO_CUT_OFF: usize = 3;

/// The plain sentence for a ring with `terminals` full terminals that cannot cut one off.
pub fn cut_off_sentence(terminals: usize) -> String {
    format!(
        "To remove a stolen or lost device you need {TERMINALS_TO_CUT_OFF} computers or servers you control, the one you remove included, so that two others can approve with the phone. You have {terminals}. Until then: change the passwords of its hosts and remove its key from every host's authorized_keys."
    )
}

/// The Sync tab's header answer: `None` when a stolen terminal can be cut off, else how many more are needed.
pub fn terminals_missing_to_cut_off(store: &LauncherStore) -> Result<Option<usize>> {
    let n = live_terminal_holders(store)?;
    Ok((n < TERMINALS_TO_CUT_OFF).then(|| TERMINALS_TO_CUT_OFF - n))
}

/// The rule that makes a revocation possible at all: two contributors must remain, so at least three
/// holders exist before it. `target` is the holder to revoke. The error is the sentence a person reads.
pub fn check_revocable(store: &LauncherStore, target: &DeviceId, now: i64) -> Result<()> {
    let holders = live_holders(store)?;
    ensure!(
        holders.iter().any(|d| d.id == *target),
        "{target} is not a live holder"
    );
    let remaining: Vec<_> = holders
        .iter()
        .filter(|d| d.id != *target && d.kind == DeviceKind::Terminal)
        .collect();
    let terminals = remaining.len()
        + usize::from(
            holders
                .iter()
                .any(|d| d.id == *target && d.kind == DeviceKind::Terminal),
        );
    ensure!(remaining.len() >= 2, "{}", cut_off_sentence(terminals));
    super::quorum::paper_rule(store, holders.len() - 1)?;
    // Alive means: wrote an op to the ring recently (the log, the same on every device), not "was last heard from here".
    let fresh =
        super::ceremony::log_live(store, remaining.iter().map(|d| (*d).clone()).collect(), now)?
            .len();
    ensure!(
        fresh >= 2,
        "fewer than two remaining holders have written to the ring recently enough to contribute"
    );
    Ok(())
}

/// The newest revoked holder whose reshare is not finished: `(op lamport, timestamp, revoked id)`.
fn open_revocation(store: &LauncherStore) -> Result<Option<(u64, i64, DeviceId)>> {
    let devices = super::ceremony::effective_devices(store, None)?;
    let batches = reshare_ops(store, "ShareReshareBatch")?;
    let mut open = None;
    for op in ops_of(store, "DeviceRevoke", None)? {
        let OpBody::DeviceRevoke { device_id } = &op.body else {
            continue;
        };
        // A revocation nobody completed reshares nothing.
        if !super::ceremony::in_force(store, &op)? {
            continue;
        }
        let was_holder = devices
            .iter()
            .any(|d| d.id == *device_id && d.role == DeviceRole::Holder);
        if !was_holder {
            continue;
        }
        let resolved = batches.iter().any(|b| {
            b.lamport > op.lamport && matches!(&b.body, OpBody::ShareReshareBatch { new_holders, .. } if !new_holders.contains(device_id))
        });
        if !resolved {
            open = Some((op.lamport, op.timestamp, device_id.clone()));
        }
    }
    Ok(open)
}

fn rotated_since(store: &LauncherStore, holder: &DeviceId, lamport: u64) -> Result<bool> {
    Ok(ops_of(store, "WrapKeyRotate", Some(holder))?
        .iter()
        .any(|o| o.lamport > lamport))
}

/// What this device should do next about a revocation, if anything. Safe to call after every round.
pub fn next_actions(
    store: &LauncherStore,
    id: &DeviceIdentity,
    now: i64,
) -> Result<Vec<RevocationAction>> {
    let me = id.public().device_id();
    let devices = super::ceremony::effective_devices(store, None)?;
    let am_holder = devices
        .iter()
        .any(|d| d.id == me && d.role == DeviceRole::Holder && d.status != DeviceStatus::Revoked);
    if !am_holder {
        return Ok(Vec::new());
    }
    // Stage 4 first: it does not depend on an open revocation.
    let mut out = Vec::new();
    let latest_epoch = reshare_ops(store, "ShareReshareBatch")?
        .iter()
        .filter_map(|o| match &o.body {
            OpBody::ShareReshareBatch { epoch, .. } => Some((*epoch, o.timestamp)),
            _ => None,
        })
        .max();
    if let Some((epoch, _)) = latest_epoch {
        // Never on a timer: until every credential was adopted by a real unlock, the previous wrap
        // keys are the only way to read the shares of the old sharing.
        if id.wrap_key_generations() > 1 && unadopted(store, epoch)?.is_empty() {
            out.push(RevocationAction::Retire);
        }
    }
    // 3. Receive: the newest reshare, once two contributors' batches exist and this device, a new
    // holder of it, has not confirmed. Independent of the revocation being open: the first batch
    // already closes it.
    if let Some((epoch, _)) = latest_epoch {
        let batches: Vec<_> = reshare_ops(store, "ShareReshareBatch")?
            .into_iter()
            .filter(
                |b| matches!(&b.body, OpBody::ShareReshareBatch { epoch: e, .. } if *e == epoch),
            )
            .collect();
        let contributors: BTreeSet<&DeviceId> = batches.iter().map(|b| &b.device_id).collect();
        let is_new_holder = batches
            .iter()
            .any(|b| matches!(&b.body, OpBody::ShareReshareBatch { new_holders, .. } if new_holders.contains(&me)));
        let confirmed = ops_of(store, "ShareReshareConfirmBatch", Some(&me))?
            .iter()
            .any(|o| matches!(&o.body, OpBody::ShareReshareConfirmBatch { epoch: e, .. } if *e == epoch));
        if contributors.len() >= 2 && is_new_holder && !confirmed {
            out.push(RevocationAction::Receive { epoch });
        }
    }
    let Some((lamport, revoked_at, _revoked)) = open_revocation(store)? else {
        out.extend(membership_reshare(store, &me)?);
        return Ok(out);
    };
    let remaining = live_holders(store)?;
    let software: Vec<&super::oplog::DeviceRow> = remaining
        .iter()
        .filter(|d| d.kind == DeviceKind::Terminal)
        .collect();
    // 1. Rotate.
    if !rotated_since(store, &me, lamport)? {
        out.push(RevocationAction::Rotate);
        return Ok(out);
    }
    // 2. Contribute, once every software holder rotated or the grace ran out.
    let mut new_holders: Vec<DeviceId> = remaining.iter().map(|d| d.id.clone()).collect();
    new_holders.sort();
    let mut all_rotated = true;
    for d in &software {
        all_rotated &= rotated_since(store, &d.id, lamport)?;
    }
    if !(all_rotated || now - revoked_at >= ROTATION_GRACE_SECS) {
        return Ok(out);
    }
    let mut expected: BTreeSet<DeviceId> = new_holders.iter().cloned().collect();
    if let Some(p) = paper_pub(store)? {
        expected.insert(p.device_id());
    }
    let epoch = reshare_ops(store, "ShareReshareBatch")?
        .iter()
        .filter(|b| b.lamport > lamport)
        .find_map(|b| match &b.body {
            OpBody::ShareReshareBatch {
                epoch, new_holders, ..
            } if new_holders.iter().cloned().collect::<BTreeSet<_>>() == expected => Some(*epoch),
            _ => None,
        });
    let epoch = match epoch {
        Some(e) => e,
        None => next_epoch(store)?,
    };
    let contributed = ops_of(store, "ShareReshareSession", Some(&me))?
        .iter()
        .any(|o| matches!(&o.body, OpBody::ShareReshareSession { epoch: e, .. } if *e == epoch));
    if !contributed {
        out.push(RevocationAction::Contribute { epoch, new_holders });
        return Ok(out);
    }
    Ok(out)
}

/// A reshare that is not a revocation: the holders of the ring (plus the paper and the exemption slots)
/// differ from the holders the credentials are shared among: a holder was promoted (C3), or an
/// exemption was enabled or disabled (C10). No wrap-key rotation is needed: nobody lost access.
fn membership_reshare(store: &LauncherStore, me: &DeviceId) -> Result<Option<RevocationAction>> {
    use super::oplog::CredMode;
    if super::ceremony::effective_cred_mode(store)? != CredMode::Quorum
        || super::reshare::quorum_credentials(store)?.is_empty()
    {
        return Ok(None);
    }
    let live = live_holders(store)?;
    let terminals = live.iter().filter(|d| d.kind == DeviceKind::Terminal);
    if terminals.clone().count() < 2 || !terminals.clone().any(|d| d.id == *me) {
        return Ok(None);
    }
    let mut new_holders: Vec<DeviceId> = live.iter().map(|d| d.id.clone()).collect();
    new_holders.sort();
    let mut expected: BTreeSet<DeviceId> = new_holders.iter().cloned().collect();
    if let Some(p) = paper_pub(store)? {
        expected.insert(p.device_id());
    }
    for (_, slot) in super::exemption::active_slots(store)? {
        expected.insert(slot.device_id());
    }
    let batches = reshare_ops(store, "ShareReshareBatch")?;
    let latest = batches
        .iter()
        .filter_map(|b| match &b.body {
            OpBody::ShareReshareBatch {
                epoch, new_holders, ..
            } => Some((*epoch, new_holders.iter().cloned().collect::<BTreeSet<_>>())),
            _ => None,
        })
        .max_by_key(|(e, _)| *e);
    let (epoch, needed) = match latest {
        // Someone already started the right sharing: join it until two contributors exist.
        Some((e, set)) if set == expected => {
            let contributors: BTreeSet<&DeviceId> = batches
                .iter()
                .filter(
                    |b| matches!(&b.body, OpBody::ShareReshareBatch { epoch, .. } if *epoch == e),
                )
                .map(|b| &b.device_id)
                .collect();
            let mine = ops_of(store, "ShareReshareSession", Some(me))?.iter().any(
                |o| matches!(&o.body, OpBody::ShareReshareSession { epoch, .. } if *epoch == e),
            );
            (
                e,
                contributors.len() < 2 && !mine && !contributors.contains(me),
            )
        }
        Some(_) => (next_epoch(store)?, true),
        None => {
            let mut differ = false;
            for host in super::reshare::quorum_credentials(store)? {
                if let Some((_, wraps)) = store.sync_cred_ciphertext(&host)? {
                    let have: BTreeSet<DeviceId> =
                        wraps.iter().map(|w| w.recipient.clone()).collect();
                    differ |= have != expected;
                }
            }
            (next_epoch(store)?, differ)
        }
    };
    Ok(needed.then_some(RevocationAction::Contribute { epoch, new_holders }))
}

/// Do one action. Returns the bodies to append. `Rotate` and `Retire` change `id`: save it first.
pub fn perform(
    store: &LauncherStore,
    id: &mut DeviceIdentity,
    action: &RevocationAction,
) -> Result<Vec<OpBody>> {
    match action {
        RevocationAction::Rotate => {
            let me = id.public().device_id();
            let row = store.sync_devices()?.into_iter().find(|d| d.id == me);
            let wrap_epoch = row.map_or(0, |d| d.wrap_epoch) + 1;
            let public = id.rotate_wrap_keys();
            Ok(vec![OpBody::WrapKeyRotate {
                device_id: me,
                wrap_epoch,
                x25519_pub: public.x25519_pub,
                mlkem_pub: public.mlkem_pub,
            }])
        }
        RevocationAction::Contribute { epoch, new_holders } => {
            Ok(reissue_plan(store, id, new_holders, *epoch)?.bodies)
        }
        RevocationAction::Receive { epoch } => Ok(reissue_receive(store, id, *epoch)?.bodies),
        RevocationAction::Retire => {
            if id.retire_previous_wrap_keys() == 0 {
                bail!("there were no previous wrap keys");
            }
            Ok(Vec::new())
        }
    }
}
