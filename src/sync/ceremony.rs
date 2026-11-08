//! The Red ceremony (contract 4c, design "The Red ceremony"): a Red op takes effect only when the log
//! holds three proofs bound to its hash, from three different participants:
//! the phone (`RedPhoneSign`, or the op itself when the phone authored it), a second attended holder
//! terminal (`RedApprove`) and a third factor from outside the devices (`RedFactor`, a signature by
//! the paper/recover key or by the key sealed in the emailed code). Every peer evaluates the same
//! rule on the same log, so there is no check a compromised peer could skip.

use anyhow::{bail, ensure, Context, Result};

use super::codes::gen_email_code;
use super::crypto::{dek_open, dek_seal, derive_key, verify, DeviceIdentity};
use super::email::{email_fragment_open, email_fragment_seal};
use super::limits::{LIVE_DEVICE_WINDOW_SECS, RED_CEREMONY_TTL_SECS};
use super::oplog::{
    Attendance, CredMode, DeviceId, DeviceKind, DeviceRole, DeviceRow, DeviceStatus, Op, OpBody,
    ThirdFactor,
};
use super::quorum::{meta_get, meta_set, ops_of};
use crate::store::LauncherStore;

const FACTOR_DOMAIN: &[u8] = b"sshub/sync/red/factor/v1";
const EMAIL_KEY_CONTEXT: &str = "sshub/sync/factor/email/v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedProof {
    Phone,
    SecondDevice,
    /// A signature by a registered factor key (`None` while no factor is named yet).
    Factor(Option<ThirdFactor>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedStatus {
    pub have: Vec<RedProof>,
    pub missing: Vec<RedProof>,
    pub complete: bool,
    pub expired: bool,
    /// A rule makes the op impossible (too few live holders ...): it is rejected, not waited for.
    pub blocked: Option<String>,
    /// Only one attended holder terminal exists: the second-device proof is waived and the paper is required.
    pub single_machine_ring: bool,
    /// The op breaks a rule of its own kind that no later state can repair (a second phone, an exemption
    /// slot the exempt device did not make): it is never in force, whoever completed its proofs.
    pub rule_violation: Option<String>,
    /// Why a proof that looks available does not count (the mailbox sender proposed or voted): the screens say so.
    pub note: Option<String>,
    /// When, on this device's clock, the last required counted proof first arrived: the earliest receipt of each
    /// required kind, the latest of those and the op itself. Meaningful when `complete`; what a time window that
    /// opens "when the ceremony completed" measures from. A junk proof naming the op moves nothing.
    pub complete_at: i64,
}

/// Is `op` one of the Red ops this ceremony gates? (`PhoneReenroll` has its own path.)
pub fn is_gated(store: &LauncherStore, op: &Op) -> Result<bool> {
    Ok(match &op.body {
        // The phone is the ceremony's independent participant: admitting one is Red whatever role it is given.
        OpBody::DeviceAdd { role, kind, .. } => {
            *role == DeviceRole::Holder || *kind == DeviceKind::Phone
        }
        OpBody::DeviceRoleChange { .. }
        | OpBody::CertIssue { .. }
        | OpBody::ModeChange { .. }
        | OpBody::FactorRegister { .. }
        | OpBody::ExemptionSet { .. }
        | OpBody::BulkExport { .. } => true,
        OpBody::DeviceRevoke { device_id } => store
            .sync_devices()?
            .iter()
            .find(|d| d.id == *device_id)
            .is_none_or(|d| d.role == DeviceRole::Holder),
        _ => false,
    })
}

/// The ceremony-complete view of a Red op: every device, the proposer included, acts on a gated op only
/// once its proofs are in. A proposer applies its own op at once (it is "approved by definition"), so
/// anything that reaches OTHER devices' data (shares, wraps, recipients, mode, certificates, export
/// windows) must read the world through [`in_force`] and [`effective_devices`], never the raw table.
/// Time does not matter: an op that completed stays in force after the pending window.
pub fn in_force(store: &LauncherStore, op: &Op) -> Result<bool> {
    #[cfg(test)]
    if meta_get(store, "test_all_in_force")?.is_some_and(|v| !v.is_empty()) {
        return Ok(true);
    }
    if !is_gated(store, op)? {
        return Ok(true);
    }
    let st = red_status(store, op, op.timestamp)?;
    Ok(st.missing.is_empty() && st.rule_violation.is_none())
}

/// Test switch: with it on, every op counts as in force on this store (fixtures that exercise what
/// happens AFTER a ceremony, not the ceremony). Off for the tests of the ceremony itself.
#[cfg(test)]
pub(crate) fn test_force(store: &LauncherStore, on: bool) {
    meta_set(store, "test_all_in_force", if on { b"1" } else { b"" }).unwrap();
}

/// Is the first op that made `device` a holder newer than lamport `l` (the row says holder, the view predates it)?
fn promoted_after(store: &LauncherStore, device: &DeviceId, l: u64) -> Result<bool> {
    Ok(ops_of(store, "DeviceRoleChange", None)?.iter().any(|op| {
        op.lamport > l
            && matches!(&op.body, OpBody::DeviceRoleChange { device_id, to: DeviceRole::Holder } if device_id == device)
    }))
}

/// Did a Quorum credential exist BEFORE lamport `before` (so a new holder then meant a reshare)? Judged at the
/// op's own position in the log, never against the ring as it is now: an op that was valid under ring setup
/// must stay valid when a Quorum credential is created later. A credential is a Quorum one when its NEWEST
/// `CredUpdate` before `before` has at least two wrapped shares (Vault mode wraps to the vault key alone), so a ring
/// whose Quorum credentials were all replaced or moved back to Vault no longer counts. A host whose ops were pruned
/// below a checkpoint cut is read from the folded `sync_cred_state` row, which keeps its newest `CredUpdate`.
fn quorum_credentials_before(store: &LauncherStore, before: u64) -> Result<bool> {
    let mut newest: std::collections::BTreeMap<[u8; 16], (u64, bool)> = Default::default();
    for op in ops_of(store, "CredUpdate", None)? {
        if let OpBody::CredUpdate {
            host_uid,
            wrapped_shares,
            ..
        } = &op.body
        {
            if op.lamport < before && newest.get(host_uid).is_none_or(|(l, _)| *l < op.lamport) {
                newest.insert(*host_uid, (op.lamport, wrapped_shares.len() >= 2));
            }
        }
    }
    if newest.values().any(|(_, quorum)| *quorum) {
        return Ok(true);
    }
    store.with_conn(|c| {
        let mut st =
            c.prepare("SELECT host_uid, wrapped_shares FROM sync_cred_state WHERE lamport < ?1")?;
        let rows = st.query_map([i64::try_from(before).unwrap_or(i64::MAX)], |r| {
            Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (uid, wraps) = row?;
            let known = <[u8; 16]>::try_from(uid.as_slice()).is_ok_and(|u| newest.contains_key(&u));
            let n = serde_json::from_str::<Vec<serde_json::Value>>(&wraps).map_or(0, |v| v.len());
            if !known && n >= 2 {
                return Ok(true);
            }
        }
        Ok(false)
    })
}

/// Ring setup is over for good once the ring has had a phone and two terminal holders (revoked rows still
/// count: setup is never re-entered). The op's own subject is left out: it is what is being decided.
fn setup_ended(devices: &[DeviceRow], op: &Op, promoted: Option<&DeviceId>) -> bool {
    let subject = match &op.body {
        OpBody::DeviceAdd { device, .. } => Some(device.device_id()),
        _ => promoted.cloned(),
    };
    let others = || devices.iter().filter(|d| Some(&d.id) != subject.as_ref());
    let phone = others().any(|d| d.kind == DeviceKind::Phone && d.role == DeviceRole::Holder);
    let terminals = others()
        .filter(|d| d.kind == DeviceKind::Terminal && d.role == DeviceRole::Holder)
        .count();
    phone && terminals >= 2
}

/// Did `device` join the ring (its first `DeviceAdd`) after lamport `l`? A founder or a device adopted with an
/// anchor has no `DeviceAdd` of its own: false.
pub(crate) fn joined_after(store: &LauncherStore, device: &DeviceId, l: u64) -> Result<bool> {
    let first = ops_of(store, "DeviceAdd", None)?
        .into_iter()
        .filter(|op| matches!(&op.body, OpBody::DeviceAdd { device: d, .. } if d.device_id() == *device))
        .map(|op| op.lamport)
        .min();
    Ok(first.is_some_and(|f| f > l))
}

/// The newest op with `lamport <= upto` that made `device` a holder, if any.
fn establishing_op(
    store: &LauncherStore,
    device: &DeviceId,
    upto: Option<u64>,
) -> Result<Option<Op>> {
    let mut newest: Option<Op> = None;
    for kind in ["DeviceAdd", "DeviceRoleChange"] {
        for op in ops_of(store, kind, None)? {
            let (who, to_holder) = match &op.body {
                OpBody::DeviceAdd { device, role, .. } => {
                    (device.device_id(), *role == DeviceRole::Holder)
                }
                OpBody::DeviceRoleChange { device_id, to } => {
                    (device_id.clone(), *to == DeviceRole::Holder)
                }
                _ => continue,
            };
            if who != *device || upto.is_some_and(|u| op.lamport > u) {
                continue;
            }
            if newest.as_ref().is_none_or(|n| n.lamport < op.lamport) {
                newest = Some(op.clone());
            }
            let _ = to_holder;
        }
    }
    Ok(newest)
}

fn holder_established(store: &LauncherStore, device: &DeviceId, upto: Option<u64>) -> Result<bool> {
    let Some(op) = establishing_op(store, device, upto)? else {
        // A founder, or a holder adopted with an anchor; but not a device whose admission is LATER than the
        // view (its row exists, it was not part of the ring yet).
        return Ok(!upto.is_some_and(|u| {
            joined_after(store, device, u).unwrap_or(false)
                || promoted_after(store, device, u).unwrap_or(false)
        }));
    };
    let to_holder = match &op.body {
        OpBody::DeviceAdd { role, .. } => *role == DeviceRole::Holder,
        OpBody::DeviceRoleChange { to, .. } => *to == DeviceRole::Holder,
        _ => false,
    };
    if !to_holder {
        // The newest op up to this view added or kept the device as a mirror: a promotion that comes only
        // LATER is not part of this view (the row says holder because it is read now).
        return Ok(!upto.is_some_and(|u| promoted_after(store, device, u).unwrap_or(false)));
    }
    if upto == Some(op.lamport) {
        return Ok(true);
    }
    in_force(store, &op)
}

fn revoke_in_force(store: &LauncherStore, device: &DeviceId, upto: Option<u64>) -> Result<bool> {
    let mut any = false;
    let mut later = false;
    for op in ops_of(store, "DeviceRevoke", None)? {
        if matches!(&op.body, OpBody::DeviceRevoke { device_id } if device_id == device)
            && upto.is_some_and(|u| op.lamport > u)
        {
            later = true;
        }
        if matches!(&op.body, OpBody::DeviceRevoke { device_id } if device_id == device)
            && !upto.is_some_and(|u| op.lamport > u)
        {
            any = true;
            if upto == Some(op.lamport) || in_force(store, &op)? {
                return Ok(true);
            }
        }
    }
    // No revoke op in the log (an anchor, or a revoked non-holder): the row stands. A revoke that comes only
    // AFTER this view means the device was not revoked yet in it.
    Ok(!any && !later)
}

/// The device table as the ceremony sees it at `upto` (a lamport bound; `None` = now): a device whose
/// promotion to holder is not in force is a mirror, a device whose revocation is not in force is not
/// revoked. Every consumer that counts holders, wraps to them, reshares to them or lets them vote reads this.
pub fn effective_devices(store: &LauncherStore, upto: Option<u64>) -> Result<Vec<DeviceRow>> {
    let mut rows = store.sync_devices()?;
    for d in rows.iter_mut() {
        if d.role == DeviceRole::Holder && !holder_established(store, &d.id, upto)? {
            d.role = DeviceRole::Mirror;
        }
        if d.status == DeviceStatus::Revoked && !revoke_in_force(store, &d.id, upto)? {
            d.status = DeviceStatus::Operational;
        }
    }
    Ok(rows)
}

/// The credential mode the ring has agreed to: the projection's mode, unless the newest `ModeChange` is
/// not in force (its proposer applied it at once, nobody else did), then the mode before it.
pub fn effective_cred_mode(store: &LauncherStore) -> Result<CredMode> {
    let mode = store.sync_cred_mode()?;
    if let Some(mc) = ops_of(store, "ModeChange", None)?.pop() {
        if !in_force(store, &mc)? {
            return Ok(match mode {
                CredMode::Vault => CredMode::Quorum,
                CredMode::Quorum => CredMode::Vault,
            });
        }
    }
    Ok(mode)
}

/// Pending only because its author is not an accepted device yet.
pub(crate) fn awaits_author(store: &LauncherStore, op: &Op) -> Result<bool> {
    Ok(store.sync_pending_reason(&op.hash())?.as_deref() == Some("unknown-author"))
}

fn factor_message(op_hash: &[u8; 32], proposer: &DeviceId) -> Vec<u8> {
    [FACTOR_DOMAIN, op_hash.as_slice(), proposer.as_bytes()].concat()
}

/// A registered factor: verifying key, sealed seed (email), sender (email).
type Registration = ([u8; 32], Vec<u8>, Option<DeviceId>);

/// Has `factor` been PROVEN for `hash`: a `RedFactor` naming `hash`, newer than `after`, whose signature verifies under
/// the key registered (and in force) before `registered_before`, over the message bound to `author` (the proposer of
/// the op, or the device cancelling a recovery)? The emailed code does not count when its registered sender is `author`
/// or one of `voters`: the mailbox duty is a participant of its own. Naming a factor in an op proves nothing.
pub(crate) fn factor_proven(
    store: &LauncherStore,
    hash: &[u8; 32],
    author: &DeviceId,
    factor: &ThirdFactor,
    registered_before_lamport: u64,
    after: u64,
    voters: &[DeviceId],
) -> Result<bool> {
    let Some((key, _, sender)) =
        registered_before(store, factor.clone(), Some(registered_before_lamport))?
    else {
        return Ok(false);
    };
    if *factor == ThirdFactor::EmailCode {
        if let Some(s) = &sender {
            if s == author || voters.contains(s) {
                return Ok(false);
            }
        }
    }
    for p in ops_of(store, "RedFactor", None)? {
        if let OpBody::RedFactor {
            op_hash,
            factor: f,
            sig,
        } = &p.body
        {
            if op_hash == hash
                && f == factor
                && p.lamport > after
                && verify(&key, &factor_message(hash, author), sig)
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// The newest applied registration of `factor`.
fn registered(store: &LauncherStore, factor: ThirdFactor) -> Result<Option<Registration>> {
    registered_before(store, factor, None)
}

/// The newest registration of `factor` that is IN FORCE and older than `before` (a lamport bound; a
/// factor signs only what was proposed after it was registered). Bounded so that evaluating one
/// registration only looks at older ones.
fn registered_before(
    store: &LauncherStore,
    factor: ThirdFactor,
    before: Option<u64>,
) -> Result<Option<Registration>> {
    for o in ops_of(store, "FactorRegister", None)?.into_iter().rev() {
        if before.is_some_and(|b| o.lamport >= b) {
            continue;
        }
        if let OpBody::FactorRegister {
            factor: f,
            public,
            sealed,
            sender,
            ..
        } = &o.body
        {
            if *f == factor && in_force(store, &o)? {
                return Ok(Some((*public, sealed.clone(), sender.clone())));
            }
        }
    }
    Ok(None)
}

fn terminal_holder(d: &DeviceRow) -> bool {
    d.role == DeviceRole::Holder
        && d.kind == DeviceKind::Terminal
        && d.attendance == Attendance::Attended
        && !matches!(d.status, DeviceStatus::Revoked | DeviceStatus::Compromised)
}

/// The devices of `devices` known to be alive AT `t`, decided from the LOG only, so every device that holds the same ops
/// judges the same set: a device that wrote an op stamped BEFORE `t` (strictly: the author of an op at `t` cannot know an
/// op written in its own second) and within `LIVE_DEVICE_WINDOW_SECS` before it. Contact (`last_seen`) is local knowledge:
/// a device learned from an op has `last_seen` 0 until its first round, and two devices disagree about who they talked to,
/// so a verdict that read it was rejected on the devices whose contacts differed from the proposer's. A device that never
/// writes anything is not live here; revoked and compromised devices never are.
pub(crate) fn log_live(
    store: &LauncherStore,
    devices: Vec<DeviceRow>,
    t: i64,
) -> Result<Vec<DeviceRow>> {
    let wrote: std::collections::BTreeMap<String, i64> = store.with_conn(|c| {
        let mut st = c.prepare(
            "SELECT device_id, MAX(timestamp) FROM sync_ops
             WHERE timestamp < ?1 AND COALESCE(reason, '') <> 'unknown-author'
               AND state NOT IN ('rejected', 'void') GROUP BY device_id",
        )?;
        let rows = st.query_map([t], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    })?;
    Ok(devices
        .into_iter()
        .filter(|d| {
            !matches!(d.status, DeviceStatus::Revoked | DeviceStatus::Compromised)
                && wrote
                    .get(&d.id)
                    .is_some_and(|w| t - *w <= LIVE_DEVICE_WINDOW_SECS)
        })
        .collect())
}

/// When `op` counts as having happened: its own stamp, but never later than the moment this device first
/// saw it. The stamp is chosen by the author, so a future date cannot stretch the ceremony window, and a
/// proof's age is measured the same way.
pub(crate) fn base_time(store: &LauncherStore, op: &Op) -> i64 {
    received_at(store, op).map_or(op.timestamp, |s| s.min(op.timestamp))
}

/// When this device first stored `op` (its own clock; an author cannot choose it).
pub(crate) fn received_at(store: &LauncherStore, op: &Op) -> Option<i64> {
    store
        .with_conn(|c| {
            Ok(c.query_row(
                "SELECT received_at FROM sync_ops WHERE op_hash = ?1",
                rusqlite::params![&op.hash()[..]],
                |r| r.get(0),
            )
            .ok())
        })
        .ok()
        .flatten()
}

/// The phone's own proof for its admission: a `RedPhoneSign` for `hash` signed by the key in the
/// `DeviceAdd` (the phone may still be an unknown author here, its ops waiting unverified), and by
/// nobody else. Returns when this device received it.
fn own_admission_proof(
    store: &LauncherStore,
    phone: &super::crypto::DevicePublic,
    hash: &[u8; 32],
    valid_proof: &dyn Fn(&Op) -> bool,
) -> Result<Option<i64>> {
    let id = phone.device_id();
    let rows: Vec<(Vec<u8>, Vec<u8>, i64)> = store.with_conn(|c| {
        let mut st = c.prepare(
            "SELECT canonical, signature, received_at FROM sync_ops
             WHERE device_id = ?1 AND (reason = 'unknown-author' OR
                   (kind = 'RedPhoneSign' AND state IN ('applied', 'approved')))",
        )?;
        let rows = st.query_map(rusqlite::params![id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    })?;
    let mut first = None;
    for (canonical, sig, received) in rows {
        let Ok(sig) = <[u8; 64]>::try_from(sig) else {
            continue;
        };
        let Ok(p) = Op::from_canonical(&canonical, sig) else {
            continue;
        };
        if p.device_id == id
            && matches!(&p.body, OpBody::RedPhoneSign { op_hash } if op_hash == hash)
            && p.verify(&phone.ed25519_pub)
            && valid_proof(&p)
        {
            first = Some(first.map_or(received, |t: i64| t.min(received)));
        }
    }
    Ok(first)
}

/// Where the ceremony of `op` stands at `now`.
pub fn red_status(store: &LauncherStore, op: &Op, now: i64) -> Result<RedStatus> {
    let hash = op.hash();
    let proposer = &op.device_id;
    let devices = effective_devices(store, Some(op.lamport))?;
    // Two phones in force are a verdict on the op being judged, not an error in every evaluation.
    let (phone, phone_verdict) = match super::quorum::phone_device_in(&devices) {
        Ok(p) => (p, None),
        Err(e) => (None, Some(e.to_string())),
    };
    let op_base = base_time(store, op);
    let expired = now - op_base > RED_CEREMONY_TTL_SECS;
    // A proof is as old as the moment this device RECEIVED it: its stamp is the author's word, and a
    // backdated stamp must not let a proof that arrived after the window complete the ceremony.
    let valid_proof = |p: &Op| {
        p.lamport > op.lamport
            && received_at(store, p).unwrap_or(p.timestamp) - op_base <= RED_CEREMONY_TTL_SECS
    };
    let mut have = Vec::new();
    let mut missing = Vec::new();
    let mut blocked = phone_verdict;
    let mut rule_violation: Option<String> = None;

    // RING SETUP (contract 4h): a ring built from a fresh install cannot have a phone and two holders to prove
    // the step that creates them. While no Quorum credential exists (nothing to reshare) and the ring has not
    // yet had a phone and two terminal holders, promoting a terminal needs what can exist: the proposer and the
    // promoted device's own signature (the phone's proof once a phone exists, a factor once one is registered).
    let promoted: Option<DeviceId> = match &op.body {
        OpBody::DeviceRoleChange {
            device_id,
            to: DeviceRole::Holder,
        } => Some(device_id.clone()),
        OpBody::DeviceAdd {
            device,
            role: DeviceRole::Holder,
            kind: DeviceKind::Terminal,
            ..
        } => Some(device.device_id()),
        _ => None,
    };
    let quorum_credentials = quorum_credentials_before(store, op.lamport)?;
    let setup = !quorum_credentials && !setup_ended(&devices, op, promoted.as_ref());
    let setup_promotion = setup && promoted.is_some();
    let mut setup_waived: Vec<&str> = Vec::new();

    // Counting rules that make an op impossible. They count who is IN the ring (the view at the op's own position),
    // never who was heard from lately: contact is local to a device, and "wrote an op recently" is weak evidence
    // (a terminal writes only when it edits), so a verdict built on either differs between devices or blocks a quiet
    // ring. Whether a contributor is reachable is advice for the screens (`log_live`), not a rule.
    let attended_terminals = devices.iter().filter(|d| terminal_holder(d)).count();
    match &op.body {
        OpBody::DeviceAdd {
            kind: DeviceKind::Phone,
            device,
            role,
            ..
        } if phone.as_ref().is_some_and(|p| *p != device.device_id())
            || *role != DeviceRole::Holder =>
        {
            let why = "a phone is admitted as the one holder of its kind: the ring already has one, or the role is not holder"
                .to_string();
            rule_violation = Some(why.clone());
            blocked = Some(why);
        }
        OpBody::DeviceAdd {
            role: DeviceRole::Holder,
            ..
        }
        | OpBody::DeviceRoleChange {
            to: DeviceRole::Holder,
            ..
        } => {
            // The rule follows from the reshare: with no Quorum credential there is nothing to reshare.
            if attended_terminals < 2 && quorum_credentials {
                blocked = Some(
                    "adding or promoting a holder needs two attended holder terminals for the reshare"
                        .to_string(),
                );
            }
        }
        OpBody::DeviceRevoke { device_id } => {
            let remaining = devices
                .iter()
                .filter(|d| terminal_holder(d) && d.id != *device_id)
                .count();
            let holder_count = devices
                .iter()
                .filter(|d| {
                    d.role == DeviceRole::Holder
                        && !matches!(d.status, DeviceStatus::Revoked | DeviceStatus::Compromised)
                })
                .count();
            if devices
                .iter()
                .any(|d| d.id == *device_id && d.role == DeviceRole::Holder)
                && holder_count > 0
            {
                if let Err(e) = super::quorum::paper_rule(store, holder_count - 1) {
                    blocked = Some(e.to_string());
                }
            }
            if blocked.is_none()
                && devices
                    .iter()
                    .any(|d| d.id == *device_id && d.role == DeviceRole::Holder)
                && remaining < 2
            {
                blocked = Some(
                    "revoking a holder needs at least two remaining terminal holders to contribute"
                        .to_string(),
                );
            }
        }
        OpBody::ExemptionSet {
            device_id,
            slot: Some(slot),
        } => {
            let slot_is_known_key = devices.iter().any(|d| {
                d.public.ed25519_pub == slot.ed25519_pub || d.public.x25519_pub == slot.x25519_pub
            }) || super::quorum::paper_pub(store)?.is_some_and(|p| {
                p.ed25519_pub == slot.ed25519_pub || p.x25519_pub == slot.x25519_pub
            });
            let attended_terminal = devices.iter().any(|d| {
                d.id == *device_id && terminal_holder(d) && d.attendance == Attendance::Attended
            });
            if store.sync_cred_mode()? != CredMode::Quorum {
                blocked = Some("the exemption exists only in a Quorum ring".to_string());
            } else if op.device_id != *device_id {
                // The slot is derived from the phrase and the exempt device's own key: only that device can
                // make it. A slot chosen by anyone else is a key its author holds alone.
                let why = "the exemption slot must be made on the exempt device itself".to_string();
                rule_violation = Some(why.clone());
                blocked = Some(why);
            } else if slot_is_known_key {
                let why = "the exemption slot equals a key the ring already knows".to_string();
                rule_violation = Some(why.clone());
                blocked = Some(why);
            } else if !attended_terminal {
                blocked = Some("only an attended holder terminal can be exempt".to_string());
            }
        }
        OpBody::BulkExport { window_secs, .. } if *window_secs == 0 || *window_secs > 3600 => {
            blocked = Some("a bulk export window is 1 to 3600 seconds".to_string());
        }
        _ => {}
    }

    // A phone's admission: the phone is not a device of this view yet, so its proof for its OWN
    // admission is the only one there can be, and it is checked against the key the DeviceAdd carries.
    let admitted_phone = match &op.body {
        OpBody::DeviceAdd {
            device,
            kind: DeviceKind::Phone,
            ..
        } => Some(device.clone()),
        _ => None,
    };
    // Phone participation: the op itself when the phone authored it.
    let authored_by_phone = phone.as_ref() == Some(proposer);
    let heard = |p: &Op| received_at(store, p).unwrap_or(p.timestamp);
    let mut complete_at = heard(op);
    if authored_by_phone {
        have.push(RedProof::Phone);
    } else if let Some(dev) = &admitted_phone {
        match own_admission_proof(store, dev, &hash, &valid_proof)? {
            Some(t) => {
                complete_at = complete_at.max(t);
                have.push(RedProof::Phone);
            }
            None => missing.push(RedProof::Phone),
        }
    } else if let Some(ph) = &phone {
        let first = ops_of(store, "RedPhoneSign", Some(ph))?
            .iter()
            .filter(|p| {
                valid_proof(p)
                    && matches!(&p.body, OpBody::RedPhoneSign { op_hash } if *op_hash == hash)
            })
            .map(heard)
            .min();
        if let Some(t) = first {
            complete_at = complete_at.max(t);
            have.push(RedProof::Phone);
        } else {
            missing.push(RedProof::Phone);
        }
    } else if setup_promotion {
        setup_waived.push("the phone's proof (the ring has no phone yet)");
    } else {
        missing.push(RedProof::Phone);
    }

    // Second attended holder terminal.
    let delegate = match &op.body {
        OpBody::CertIssue { device_id, .. } | OpBody::ExemptionSet { device_id, .. } => {
            Some(device_id.clone())
        }
        _ => None,
    };
    let cosigner = match &op.body {
        OpBody::ModeChange {
            to: CredMode::Vault,
            cosigner,
            ..
        } => cosigner.clone(),
        _ => None,
    };
    let others = devices
        .iter()
        .filter(|d| terminal_holder(d) && d.id != *proposer)
        .count();
    let single_machine_ring = others == 0 && !authored_by_phone;
    let mut approved = false;
    let mut approved_own = false;
    let mut second_at: Option<i64> = None;
    let mut approvers: Vec<DeviceId> = Vec::new();
    for p in ops_of(store, "RedApprove", None)? {
        if !valid_proof(&p)
            || !matches!(&p.body, OpBody::RedApprove { op_hash } if *op_hash == hash)
        {
            continue;
        }
        let Some(row) = devices.iter().find(|d| d.id == p.device_id) else {
            continue;
        };
        // At ring setup the promoted device's own attended signature stands in for the second holder.
        let own = setup_promotion
            && promoted.as_ref() == Some(&p.device_id)
            && row.attendance == Attendance::Attended
            && !matches!(
                row.status,
                DeviceStatus::Revoked | DeviceStatus::Compromised
            );
        // The device being promoted never vouches for itself outside ring setup: in the view at the op's own position
        // it already counts as a holder, so its own approval would otherwise be a second-device vote.
        let self_vote = !own && promoted.as_ref() == Some(&p.device_id);
        if (!own && !terminal_holder(row))
            || self_vote
            || p.device_id == *proposer
            || delegate.as_ref() == Some(&p.device_id)
        {
            continue;
        }
        if cosigner.as_ref().is_some_and(|c| *c != p.device_id) {
            continue;
        }
        if own {
            approved_own = true;
        }
        approved = true;
        second_at = Some(second_at.map_or(heard(&p), |t: i64| t.min(heard(&p))));
        approvers.push(p.device_id.clone());
    }
    if setup_promotion {
        // Other holders' votes do not replace the promoted device's own signature at setup.
        approved = approved_own;
    }
    if approved {
        complete_at = complete_at.max(second_at.unwrap_or(0));
        have.push(RedProof::SecondDevice);
    } else if !single_machine_ring || setup_promotion {
        missing.push(RedProof::SecondDevice);
    }

    // Third factor.
    let paper = registered_before(store, ThirdFactor::Paper, Some(op.lamport))?;
    let email = registered_before(store, ThirdFactor::EmailCode, Some(op.lamport))?;
    let replacing = match &op.body {
        OpBody::FactorRegister { factor, .. } => Some(factor.clone()),
        _ => None,
    };
    let any_registered = paper.is_some() || email.is_some();
    // The first phone of a ring with no factor yet cannot wait for a factor: registering the first
    // factor needs that phone's proof (the other half of the same bootstrap).
    // It is the SETUP of a ring only: no factor is registered, and for a phone no other phone was ever admitted
    // (a revoked phone still counts, so a recovery cannot reopen it, nor can a removed factor).
    let first_phone_ever = admitted_phone.as_ref().is_some_and(|dev| {
        !devices.iter().any(|d| {
            d.kind == DeviceKind::Phone && d.role == DeviceRole::Holder && d.id != dev.device_id()
        })
    });
    let setup_factor = setup_promotion && !any_registered;
    if setup_factor {
        setup_waived.push("the third factor (none is registered yet)");
    }
    let bootstrap = (replacing.is_some() || first_phone_ever || setup_factor) && !any_registered;
    let mut factor_ok = false;
    let mut factor_named: Option<ThirdFactor> = None;
    let mut factor_at: Option<i64> = None;
    for p in ops_of(store, "RedFactor", None)? {
        let OpBody::RedFactor {
            op_hash,
            factor,
            sig,
        } = &p.body
        else {
            continue;
        };
        if !valid_proof(&p) || *op_hash != hash {
            continue;
        }
        let key = match factor {
            ThirdFactor::Paper => paper.as_ref().map(|r| r.0),
            ThirdFactor::EmailCode => email.as_ref().map(|r| r.0),
        };
        let Some(key) = key else { continue };
        // The mailbox duty is a participant of its own: the registered sender neither proposed the op nor
        // gave its second-device vote.
        if *factor == ThirdFactor::EmailCode {
            if let Some(sender) = email.as_ref().and_then(|e| e.2.as_ref()) {
                if sender == proposer || approvers.contains(sender) {
                    continue;
                }
            }
        }
        // Replacing a registered factor needs the factor being replaced, not just any.
        if let Some(r) = &replacing {
            if registered_before(store, r.clone(), Some(op.lamport))?.is_some() && factor != r {
                continue;
            }
        }
        if verify(&key, &factor_message(&hash, proposer), sig) {
            factor_ok = true;
            factor_named = Some(factor.clone());
            factor_at = Some(factor_at.map_or(heard(&p), |t: i64| t.min(heard(&p))));
        }
    }
    let mut note = None;
    if !factor_ok {
        if let Some(sender) = email.as_ref().and_then(|e| e.2.as_ref()) {
            if sender == proposer || approvers.contains(sender) {
                note = Some(
                    "the emailed code cannot count here: its registered sender proposed this change or gave the second-device vote; another attended holder must approve, or use the paper"
                        .to_string(),
                );
            }
        }
    }
    if factor_ok {
        complete_at = complete_at.max(factor_at.unwrap_or(0));
        have.push(RedProof::Factor(factor_named.clone()));
    } else if !bootstrap {
        missing.push(RedProof::Factor(replacing.clone().filter(|r| {
            registered_before(store, r.clone(), Some(op.lamport))
                .ok()
                .flatten()
                .is_some()
        })));
    }
    // The two completion ops of a Ready phone-loss recovery need no proof of the lost phone: the recovery
    // was the ceremony (contract 4h).
    let completion = match &op.body {
        // The view at the op's own lamport already counts the op itself as in force, so the target is
        // recognised by what it is (a holder phone), not by being the live phone of that view.
        OpBody::DeviceRevoke { device_id } => devices.iter().any(|d| {
            d.id == *device_id && d.kind == DeviceKind::Phone && d.role == DeviceRole::Holder
        }),
        // Likewise the new phone is already a row of its own view: admitted only while no OTHER phone lives.
        OpBody::DeviceAdd {
            device,
            kind: DeviceKind::Phone,
            role: DeviceRole::Holder,
            ..
        } => !devices.iter().any(|d| {
            d.kind == DeviceKind::Phone
                && d.role == DeviceRole::Holder
                && !matches!(d.status, DeviceStatus::Revoked | DeviceStatus::Compromised)
                && d.id != device.device_id()
        }),
        _ => false,
    };
    // The clock is this device's: the completion op's own stamp (what `in_force` passes) must not be able
    // to pull the end of the time lock forward.
    let recovery_now = now.min(received_at(store, op).unwrap_or(now));
    let recovery_stands =
        completion && super::recovery::ready_recovery_for(store, op, recovery_now)?;
    if recovery_stands {
        missing.clear();
        have = vec![
            RedProof::Phone,
            RedProof::SecondDevice,
            RedProof::Factor(None),
        ];
    }
    // A ring of one terminal needs the paper: with no second device it is the only independent proof left.
    if single_machine_ring
        && !recovery_stands
        && !matches!(factor_named, Some(ThirdFactor::Paper))
        && !bootstrap
        && !missing.iter().any(|m| matches!(m, RedProof::Factor(_)))
    {
        missing.push(RedProof::Factor(Some(ThirdFactor::Paper)));
    }
    if note.is_none() && !setup_waived.is_empty() {
        note = Some(format!(
            "ring setup: not required yet: {}; setup ends for good once the ring has a phone and two terminal holders",
            setup_waived.join(", ")
        ));
    }
    let complete = missing.is_empty() && blocked.is_none() && !expired;
    Ok(RedStatus {
        have,
        missing,
        complete,
        expired,
        blocked,
        single_machine_ring,
        rule_violation,
        note,
        complete_at,
    })
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SettleReport {
    pub applied: Vec<[u8; 32]>,
    pub rejected: Vec<([u8; 32], String)>,
}

/// Resolve pending Red ops: approved when their ceremony is complete, rejected when it can never be
/// (blocked, or expired). Runs after every ingest.
pub fn settle_red_ops(store: &LauncherStore, now: i64) -> Result<SettleReport> {
    let mut report = SettleReport::default();
    for op in store.sync_pending()? {
        // An op of an unknown author is unverifiable and inert: it waits for the author's admission.
        if !is_gated(store, &op)? || awaits_author(store, &op)? {
            continue;
        }
        let st = red_status(store, &op, now)?;
        if let Some(why) = st.blocked {
            store.sync_resolve_pending(&op.hash(), false)?;
            report.rejected.push((op.hash(), why));
        } else if st.expired {
            store.sync_resolve_pending(&op.hash(), false)?;
            report
                .rejected
                .push((op.hash(), "the ceremony did not complete in time".into()));
        } else if st.complete {
            store.sync_resolve_pending(&op.hash(), true)?;
            report.applied.push(op.hash());
        }
    }
    // An author admitted by a ceremony that just completed (the phone) may have sent ops before it was known:
    // they waited as unverifiable, and now run through the ordinary ingest (signature, chain, tier). Without
    // this a device that received the author's ops BEFORE its admission would never see them, and its verdicts
    // would differ from every device that received them after.
    let held = store.sync_pending_with_reasons()?;
    let mut released = false;
    let known: Vec<DeviceId> = store.sync_devices()?.into_iter().map(|d| d.id).collect();
    let mut held: Vec<Op> = held
        .into_iter()
        .filter(|(_, why)| why == "unknown-author")
        .map(|(op, _)| op)
        .filter(|op| known.contains(&op.device_id))
        .collect();
    held.sort_by_key(|op| (op.device_id.clone(), op.seq));
    for op in held {
        released |= store.sync_resolve_pending(&op.hash(), true).is_ok();
    }
    if released {
        let again = settle_red_ops(store, now)?;
        report.applied.extend(again.applied);
        report.rejected.extend(again.rejected);
    }
    Ok(report)
}

/// The only way a UI may approve a pending op: a gated Red op is approved only when its ceremony is
/// complete; rejecting is always allowed.
pub fn resolve(store: &LauncherStore, op_hash: &[u8; 32], approve: bool, now: i64) -> Result<()> {
    if approve {
        let op = store
            .sync_pending()?
            .into_iter()
            .find(|o| o.hash() == *op_hash)
            .context("no such pending op")?;
        if is_gated(store, &op)? {
            let st = red_status(store, &op, now)?;
            ensure!(st.complete, "this is a Red change: it takes effect when its ceremony is complete (missing {:?})", st.missing);
        }
    }
    store.sync_resolve_pending(op_hash, approve)
}

// ------------------------------------------------------------------ builders

/// The second device's vote, after its user confirmed the Red screen.
pub fn red_approve_body(op: &Op) -> OpBody {
    OpBody::RedApprove { op_hash: op.hash() }
}

/// The phone's participation (encoded by the page; here for tests and the Rust phone fixtures).
pub fn red_phone_sign_body(op: &Op) -> OpBody {
    OpBody::RedPhoneSign { op_hash: op.hash() }
}

/// The paper's factor proof, made on the machine where the 24 words were typed.
pub fn red_factor_paper(paper_recover: &DeviceIdentity, op: &Op) -> OpBody {
    let sig = paper_recover.sign(&factor_message(&op.hash(), &op.device_id));
    OpBody::RedFactor {
        op_hash: op.hash(),
        factor: ThirdFactor::Paper,
        sig,
    }
}

/// The emailed code's factor proof: opens the sealed seed with `code` and signs.
pub fn red_factor_email(store: &LauncherStore, code: &str, op: &Op) -> Result<OpBody> {
    let (public, sealed, _) = registered(store, ThirdFactor::EmailCode)?
        .context("no emailed-code factor is registered")?;
    let seed: [u8; 32] = email_fragment_open(&sealed, code)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("the sealed seed has the wrong length"))?;
    let key = DeviceIdentity::from_seed(&seed, EMAIL_KEY_CONTEXT);
    ensure!(
        key.public().ed25519_pub == public,
        "the code opened something that is not this ring's email key"
    );
    let sig = key.sign(&factor_message(&op.hash(), &op.device_id));
    Ok(OpBody::RedFactor {
        op_hash: op.hash(),
        factor: ThirdFactor::EmailCode,
        sig,
    })
}

/// Register the paper's recovery key as a factor (first registration needs phone + second device only).
pub fn factor_register_paper(phrase: &str) -> Result<OpBody> {
    super::quorum::paper_registration(phrase)
}

/// Register the emailed-code factor: a random seed becomes the `email` key, sealed under a fresh code.
/// Returns the op body and the code, which only the `sender` device keeps (and mails when needed);
/// it is never stored in the log.
pub fn factor_register_email(sender: &DeviceId) -> Result<(OpBody, String)> {
    let mut seed = zeroize::Zeroizing::new([0u8; 32]);
    rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut *seed);
    let code = gen_email_code();
    let sealed = email_fragment_seal(&*seed, &code)?;
    ensure!(sealed.len() <= 128, "sealed seed too large");
    let public = DeviceIdentity::from_seed(&seed, EMAIL_KEY_CONTEXT)
        .public()
        .ed25519_pub;
    Ok((
        OpBody::FactorRegister {
            factor: ThirdFactor::EmailCode,
            public,
            sealed,
            salt: [0; 16],
            sender: Some(sender.clone()),
        },
        code,
    ))
}

const CODE_KEY: &str = "email_factor_code_sealed";

/// The key that seals the stored code: a keyed function of this device's key (a deterministic signature),
/// so a copy of the database alone holds no code.
fn code_key(id: &DeviceIdentity) -> [u8; 32] {
    derive_key(
        "sshub/sync/email-code/seal/v1",
        &id.sign(b"sshub/sync/email-code/bind/v1"),
    )
}

/// The sender device keeps the code locally, sealed under its own device key (never in the log, never in
/// clear on disk).
pub fn remember_email_code(store: &LauncherStore, id: &DeviceIdentity, code: &str) -> Result<()> {
    meta_set(store, CODE_KEY, &dek_seal(&code_key(id), code.as_bytes()))
}

fn stored_code(store: &LauncherStore, id: &DeviceIdentity) -> Result<Option<String>> {
    Ok(meta_get(store, CODE_KEY)?
        .and_then(|b| dek_open(&code_key(id), &b).ok())
        .and_then(|p| String::from_utf8(p).ok()))
}

/// A name or id for a mail or a screen: no control characters, bounded.
pub(crate) fn plain_text(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(64).collect()
}

/// One line saying what a Red op asks for, with its parameters, for the mail that carries the code and
/// for screens that have no richer view.
pub fn describe_red(store: &LauncherStore, op: &Op) -> String {
    let name = |id: &DeviceId| {
        store
            .sync_devices()
            .ok()
            .and_then(|d| d.into_iter().find(|r| r.id == *id))
            .map_or_else(
                || id.clone(),
                |r| format!("{} ({})", plain_text(&r.name), r.id),
            )
    };
    let what = match &op.body {
        OpBody::DeviceAdd {
            name: n,
            role,
            kind,
            ..
        } => {
            format!("add device {} as {role:?} ({kind:?})", plain_text(n))
        }
        OpBody::DeviceRoleChange { device_id, to } => format!("make {} a {to:?}", name(device_id)),
        OpBody::DeviceRevoke { device_id } => format!("revoke {}", name(device_id)),
        OpBody::ModeChange { to, .. } => format!("switch the ring to {to:?} mode"),
        OpBody::FactorRegister { factor, .. } => format!("register the {factor:?} factor"),
        OpBody::ExemptionSet { device_id, slot } => match slot {
            Some(s) => format!(
                "let {} open credentials with the master phrase (slot {})",
                name(device_id),
                s.device_id()
            ),
            None => format!("remove the master phrase exemption of {}", name(device_id)),
        },
        OpBody::CertIssue {
            device_id,
            max_new_hosts_per_day,
            ..
        } => format!(
            "let {} open hosts without asking the phone ({max_new_hosts_per_day} new hosts a day)",
            name(device_id)
        ),
        OpBody::BulkExport {
            requester,
            window_secs,
        } => {
            format!(
                "let {} export every credential for {window_secs} s",
                name(requester)
            )
        }
        _ => "a Red change".to_string(),
    };
    format!("{what}, proposed by {}", name(&op.device_id))
}

/// The mails the sender owes: for each pending gated op that already has its phone and second-device
/// proofs and has not been mailed, `(op hash, subject, body)`. The body names what the change is and
/// carries the code; call `mark_code_sent` after the send succeeded. Nothing is owed for an op this
/// device proposed or gave its second-device vote to.
pub fn owed_code_mails(
    store: &LauncherStore,
    id: &DeviceIdentity,
    now: i64,
) -> Result<Vec<([u8; 32], String, String)>> {
    let me = id.public().device_id();
    let Some((_, _, Some(sender))) = registered(store, ThirdFactor::EmailCode)? else {
        return Ok(Vec::new());
    };
    if sender != me {
        return Ok(Vec::new());
    }
    let Some(code) = stored_code(store, id)? else {
        return Ok(Vec::new());
    };
    let approved_by_me = |hash: &[u8; 32]| -> Result<bool> {
        Ok(ops_of(store, "RedApprove", Some(&me))?
            .iter()
            .any(|p| matches!(&p.body, OpBody::RedApprove { op_hash } if op_hash == hash)))
    };
    let mut out = Vec::new();
    for op in store.sync_pending()? {
        if !is_gated(store, &op)? || op.device_id == me || approved_by_me(&op.hash())? {
            continue;
        }
        let st = red_status(store, &op, now)?;
        let only_factor_left = st.blocked.is_none()
            && !st.expired
            && st.missing.iter().all(|m| matches!(m, RedProof::Factor(_)))
            && !st.missing.is_empty();
        if !only_factor_left
            || meta_get(
                store,
                &format!("code-mailed:{}", hex::encode(&op.hash()[..16])),
            )?
            .is_some()
        {
            continue;
        }
        out.push((
            op.hash(),
            "[sshub] code for a Red change".to_string(),
            format!(
                "A change that needs your third factor is waiting (reference {}): {}.\n\nIf that is what you expect, paste this code into sshub on the machine that proposed it:\n\n{code}\n\nIf you did not expect this, ignore this message: the code alone opens nothing.",
                hex::encode(&op.hash()[..4]),
                describe_red(store, &op)
            ),
        ));
    }
    Ok(out)
}

/// Mail every owed code through `send(subject, body)`; an op is marked mailed only after its send
/// succeeded, so a failing mail server loses nothing and blocks nobody. Returns how many went out.
pub fn send_owed_codes(
    store: &LauncherStore,
    me: &DeviceIdentity,
    now: i64,
    send: &mut dyn FnMut(&str, &str) -> Result<()>,
) -> Result<usize> {
    let mut sent = 0;
    for (hash, subject, body) in owed_code_mails(store, me, now)? {
        if send(&subject, &body).is_ok() {
            mark_code_sent(store, &hash)?;
            sent += 1;
        }
    }
    Ok(sent)
}

pub fn mark_code_sent(store: &LauncherStore, op_hash: &[u8; 32]) -> Result<()> {
    meta_set(
        store,
        &format!("code-mailed:{}", hex::encode(&op_hash[..16])),
        b"1",
    )
}

/// Errors out when `op` is not gated: builders and screens ask first.
pub fn require_gated(store: &LauncherStore, op: &Op) -> Result<()> {
    if !is_gated(store, op)? {
        bail!("this op is not a Red change");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
