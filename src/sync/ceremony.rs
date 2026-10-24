//! The Red ceremony (contract 4c, design "The Red ceremony"): a Red op takes effect only when the log
//! holds three proofs bound to its hash, from three different participants:
//! the phone (`RedPhoneSign`, or the op itself when the phone authored it), a second attended holder
//! terminal (`RedApprove`) and a third factor from outside the devices (`RedFactor`, a signature by
//! the paper/recover key or by the key sealed in the emailed code). Every peer evaluates the same
//! rule on the same log, so there is no check a compromised peer could skip.

use anyhow::{bail, ensure, Context, Result};

use super::codes::gen_email_code;
use super::crypto::{verify, DeviceIdentity};
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
}

/// Is `op` one of the Red ops this ceremony gates? (`PhoneReenroll` has its own path.)
pub fn is_gated(store: &LauncherStore, op: &Op) -> Result<bool> {
    Ok(match &op.body {
        OpBody::DeviceAdd { role, .. } => *role == DeviceRole::Holder,
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
    Ok(red_status(store, op, op.timestamp)?.missing.is_empty())
}

/// Test switch: with it on, every op counts as in force on this store (fixtures that exercise what
/// happens AFTER a ceremony, not the ceremony). Off for the tests of the ceremony itself.
#[cfg(test)]
pub(crate) fn test_force(store: &LauncherStore, on: bool) {
    meta_set(store, "test_all_in_force", if on { b"1" } else { b"" }).unwrap();
}

/// The newest op with `lamport <= upto` that made `device` a holder, if any, as `(lamport, op)`.
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
        return Ok(true); // a founder, or a holder adopted with an anchor
    };
    let to_holder = match &op.body {
        OpBody::DeviceAdd { role, .. } => *role == DeviceRole::Holder,
        OpBody::DeviceRoleChange { to, .. } => *to == DeviceRole::Holder,
        _ => false,
    };
    if !to_holder || upto == Some(op.lamport) {
        return Ok(true);
    }
    in_force(store, &op)
}

fn revoke_in_force(store: &LauncherStore, device: &DeviceId, upto: Option<u64>) -> Result<bool> {
    let mut any = false;
    for op in ops_of(store, "DeviceRevoke", None)? {
        if matches!(&op.body, OpBody::DeviceRevoke { device_id } if device_id == device)
            && !upto.is_some_and(|u| op.lamport > u)
        {
            any = true;
            if upto == Some(op.lamport) || in_force(store, &op)? {
                return Ok(true);
            }
        }
    }
    // No revoke op in the log (an anchor, or a revoked non-holder): the row stands.
    Ok(!any)
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

/// Where the ceremony of `op` stands at `now`.
pub fn red_status(store: &LauncherStore, op: &Op, now: i64) -> Result<RedStatus> {
    let hash = op.hash();
    let proposer = &op.device_id;
    let devices = effective_devices(store, Some(op.lamport))?;
    let phone = super::quorum::phone_device_in(&devices)?;
    let expired = now - op.timestamp > RED_CEREMONY_TTL_SECS;
    let valid_proof =
        |p: &Op| p.lamport > op.lamport && p.timestamp - op.timestamp <= RED_CEREMONY_TTL_SECS;
    let mut have = Vec::new();
    let mut missing = Vec::new();
    let mut blocked = None;

    // Counting rules that make an op impossible.
    let live_terminals = devices
        .iter()
        .filter(|d| terminal_holder(d) && now - d.last_seen <= LIVE_DEVICE_WINDOW_SECS)
        .count();
    match &op.body {
        OpBody::DeviceAdd {
            role: DeviceRole::Holder,
            ..
        }
        | OpBody::DeviceRoleChange {
            to: DeviceRole::Holder,
            ..
        } => {
            if live_terminals < 2 {
                blocked = Some(
                    "adding or promoting a holder needs two other live holders for the reshare"
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
            slot: Some(_),
        } => {
            let attended_terminal = devices.iter().any(|d| {
                d.id == *device_id && terminal_holder(d) && d.attendance == Attendance::Attended
            });
            if store.sync_cred_mode()? != CredMode::Quorum {
                blocked = Some("the exemption exists only in a Quorum ring".to_string());
            } else if !attended_terminal {
                blocked = Some("only an attended holder terminal can be exempt".to_string());
            }
        }
        OpBody::BulkExport { window_secs, .. } if *window_secs == 0 || *window_secs > 3600 => {
            blocked = Some("a bulk export window is 1 to 3600 seconds".to_string());
        }
        _ => {}
    }

    // Phone participation: the op itself when the phone authored it.
    let authored_by_phone = phone.as_ref() == Some(proposer);
    if authored_by_phone {
        have.push(RedProof::Phone);
    } else if let Some(ph) = &phone {
        if ops_of(store, "RedPhoneSign", Some(ph))?.iter().any(|p| {
            valid_proof(p)
                && matches!(&p.body, OpBody::RedPhoneSign { op_hash } if *op_hash == hash)
        }) {
            have.push(RedProof::Phone);
        } else {
            missing.push(RedProof::Phone);
        }
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
    for p in ops_of(store, "RedApprove", None)? {
        if !valid_proof(&p)
            || !matches!(&p.body, OpBody::RedApprove { op_hash } if *op_hash == hash)
        {
            continue;
        }
        let Some(row) = devices.iter().find(|d| d.id == p.device_id) else {
            continue;
        };
        if !terminal_holder(row)
            || p.device_id == *proposer
            || delegate.as_ref() == Some(&p.device_id)
        {
            continue;
        }
        if cosigner.as_ref().is_some_and(|c| *c != p.device_id) {
            continue;
        }
        approved = true;
    }
    if approved {
        have.push(RedProof::SecondDevice);
    } else if !single_machine_ring {
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
    let bootstrap = replacing.is_some() && !any_registered;
    let mut factor_ok = false;
    let mut factor_named: Option<ThirdFactor> = None;
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
        // Replacing a registered factor needs the factor being replaced, not just any.
        if let Some(r) = &replacing {
            if registered_before(store, r.clone(), Some(op.lamport))?.is_some() && factor != r {
                continue;
            }
        }
        if verify(&key, &factor_message(&hash, proposer), sig) {
            factor_ok = true;
            factor_named = Some(factor.clone());
        }
    }
    if factor_ok {
        have.push(RedProof::Factor(factor_named.clone()));
    } else if !bootstrap {
        missing.push(RedProof::Factor(
            replacing
                .clone()
                .filter(|r| registered(store, r.clone()).ok().flatten().is_some()),
        ));
    }
    // A ring of one terminal needs the paper: with no second device it is the only independent proof left.
    if single_machine_ring
        && !matches!(factor_named, Some(ThirdFactor::Paper))
        && !bootstrap
        && !missing.iter().any(|m| matches!(m, RedProof::Factor(_)))
    {
        missing.push(RedProof::Factor(Some(ThirdFactor::Paper)));
    }
    let complete = missing.is_empty() && blocked.is_none() && !expired;
    Ok(RedStatus {
        have,
        missing,
        complete,
        expired,
        blocked,
        single_machine_ring,
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

const CODE_KEY: &str = "email_factor_code";

/// The sender device keeps the code locally (never in the log).
pub fn remember_email_code(store: &LauncherStore, code: &str) -> Result<()> {
    meta_set(store, CODE_KEY, code.as_bytes())
}

/// The mails the sender owes: for each pending gated op that already has its phone and second-device
/// proofs and has not been mailed, `(op hash, subject, body)`. The body names the purpose and
/// carries the code; call `mark_code_sent` after the send succeeded.
pub fn owed_code_mails(
    store: &LauncherStore,
    me: &DeviceId,
    now: i64,
) -> Result<Vec<([u8; 32], String, String)>> {
    let Some((_, _, Some(sender))) = registered(store, ThirdFactor::EmailCode)? else {
        return Ok(Vec::new());
    };
    if sender != *me {
        return Ok(Vec::new());
    }
    let Some(code) = meta_get(store, CODE_KEY)?.and_then(|b| String::from_utf8(b).ok()) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for op in store.sync_pending()? {
        if !is_gated(store, &op)? || op.device_id == *me {
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
                "A change that needs your third factor is waiting (reference {}). Paste this code into sshub on the machine that proposed it:\n\n{code}\n\nIf you did not expect this, ignore this message: the code alone opens nothing.",
                hex::encode(&op.hash()[..4])
            ),
        ));
    }
    Ok(out)
}

/// Mail every owed code through `send(subject, body)`; an op is marked mailed only after its send
/// succeeded, so a failing mail server loses nothing and blocks nobody. Returns how many went out.
pub fn send_owed_codes(
    store: &LauncherStore,
    me: &DeviceId,
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
