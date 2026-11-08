//! The one place the Sync UI's view model is filled from the store:
//! devices, pending operations (with a per-field diff against what this
//! device has now) and the credential mode. Approving or rejecting a pending
//! operation goes through the store too and re-reads the view.

use super::sync::{
    ChangeView, CredModeView, DeviceRoleView, DeviceStatusView, DeviceView, PendingOpView,
};
use super::*;
use crate::store::{uid_hex, LauncherStore, ManagedHost};
use crate::sync::oplog::tier::{classify, EntityKind, FieldClass};
use crate::sync::oplog::{
    Attendance, CertScope, CredMode, DeviceRole, DeviceRow, DeviceStatus, GroupFields, HostFields,
    IdentityFields, Op, OpBody, ThirdFactor,
};

fn short(id: &str) -> String {
    id.chars().take(8).collect()
}

pub(crate) fn device_view(row: &DeviceRow) -> DeviceView {
    DeviceView {
        id: row.id.clone(),
        name: row.name.clone(),
        role: match row.role {
            DeviceRole::Holder => DeviceRoleView::Holder,
            DeviceRole::Mirror => DeviceRoleView::Mirror,
        },
        unattended: row.attendance == Attendance::Unattended,
        status: match row.status {
            DeviceStatus::Operational => DeviceStatusView::Operational,
            DeviceStatus::Behind => DeviceStatusView::Behind,
            DeviceStatus::Stale => DeviceStatusView::Stale,
            DeviceStatus::Suspect => DeviceStatusView::Suspect,
            DeviceStatus::Compromised => DeviceStatusView::Compromised,
            DeviceStatus::Revoked => DeviceStatusView::Revoked,
        },
        exempt: row.exempt,
        last_seen: (row.last_seen > 0).then_some(row.last_seen),
        head_seq: row.head_seq,
        verified_by: None,
        this: false,
        phone: row.kind == crate::sync::oplog::DeviceKind::Phone,
    }
}

fn shown<T: ToString>(v: &Option<T>) -> String {
    v.as_ref().map_or_else(|| "(none)".into(), T::to_string)
}

/// `Some(new text)` when the patch sets or clears the field.
fn patch<T: ToString>(v: &Option<Option<T>>) -> Option<String> {
    v.as_ref().map(shown)
}

fn dangerous(kind: EntityKind, field: &str) -> bool {
    classify(kind, field) != Some(FieldClass::Safe)
}

fn group_name(store: &LauncherStore, uid: &crate::store::GroupUid) -> String {
    store
        .group_id_by_uid(uid)
        .ok()
        .flatten()
        .and_then(|id| store.get_group(id).ok().flatten())
        .map_or_else(
            || format!("(unknown group {})", short(&uid_hex(uid))),
            |g| g.name,
        )
}

fn identity_name(store: &LauncherStore, uid: &crate::store::IdentityUid) -> String {
    store
        .identity_id_by_uid(uid)
        .ok()
        .flatten()
        .and_then(|id| store.get_identity(id).ok().flatten())
        .map_or_else(
            || format!("(unknown identity {})", short(&uid_hex(uid))),
            |i| i.name,
        )
}

fn device_name(devices: &[DeviceRow], id: &str) -> String {
    devices
        .iter()
        .find(|d| d.id == id)
        .map_or_else(|| short(id), |d| format!("{} ({})", d.name, short(id)))
}

/// A change list builder: one row per field the op sets, `[!]` by tier.
struct Rows {
    kind: EntityKind,
    rows: Vec<ChangeView>,
}

impl Rows {
    fn new(kind: EntityKind) -> Self {
        Self {
            kind,
            rows: Vec::new(),
        }
    }

    fn add(&mut self, field: &str, from: String, to: Option<String>) {
        if let Some(to) = to {
            self.rows.push(ChangeView {
                field: field.into(),
                from,
                to,
                dangerous: dangerous(self.kind, field),
            });
        }
    }
}

fn group_changes(
    store: &LauncherStore,
    uid: &crate::store::GroupUid,
    f: &GroupFields,
) -> (Vec<ChangeView>, Option<crate::store::HostGroup>) {
    let current = store
        .group_id_by_uid(uid)
        .ok()
        .flatten()
        .and_then(|id| store.get_group(id).ok().flatten());
    let old = |get: &dyn Fn(&crate::store::HostGroup) -> String| {
        current
            .as_ref()
            .map_or_else(|| "(new group)".to_string(), get)
    };
    let mut r = Rows::new(EntityKind::Group);
    r.add("name", old(&|g| g.name.clone()), f.name.clone());
    r.add(
        "default_identity_uid",
        old(&|g| {
            shown(
                &g.default_identity_id
                    .and_then(|i| store.get_identity(i).ok().flatten())
                    .map(|i| i.name),
            )
        }),
        f.default_identity_uid
            .as_ref()
            .map(|i| shown(&i.as_ref().map(|u| identity_name(store, u)))),
    );
    r.add(
        "default_username",
        old(&|g| shown(&g.default_username)),
        patch(&f.default_username),
    );
    r.add(
        "default_port",
        old(&|g| shown(&g.default_port)),
        patch(&f.default_port),
    );
    r.add(
        "default_proxy_jump",
        old(&|g| shown(&g.default_proxy_jump)),
        patch(&f.default_proxy_jump),
    );
    r.add(
        "default_transport",
        old(&|g| shown(&g.default_transport.map(|t| format!("{t:?}").to_lowercase()))),
        patch(&f.default_transport),
    );
    r.add(
        "default_forward_agent",
        old(&|g| shown(&g.default_forward_agent)),
        patch(&f.default_forward_agent),
    );
    r.add(
        "parent_uid",
        old(&|g| {
            shown(
                &g.parent_id
                    .and_then(|i| store.get_group(i).ok().flatten())
                    .map(|p| p.name),
            )
        }),
        f.parent_uid
            .as_ref()
            .map(|p| shown(&p.as_ref().map(|u| group_name(store, u)))),
    );
    (r.rows, current)
}

fn identity_changes(
    store: &LauncherStore,
    uid: &crate::store::IdentityUid,
    f: &IdentityFields,
) -> (Vec<ChangeView>, Option<crate::store::Identity>) {
    let current = store
        .identity_id_by_uid(uid)
        .ok()
        .flatten()
        .and_then(|id| store.get_identity(id).ok().flatten());
    let old = |get: &dyn Fn(&crate::store::Identity) -> String| {
        current
            .as_ref()
            .map_or_else(|| "(new identity)".to_string(), get)
    };
    let mut r = Rows::new(EntityKind::Identity);
    r.add("name", old(&|i| i.name.clone()), f.name.clone());
    r.add("username", old(&|i| shown(&i.username)), patch(&f.username));
    r.add(
        "has_password",
        old(&|i| i.has_password.to_string()),
        f.has_password.map(|b| b.to_string()),
    );
    (r.rows, current)
}

fn host_changes(
    store: &LauncherStore,
    uid: &crate::store::HostUid,
    f: &HostFields,
) -> (Vec<ChangeView>, Option<ManagedHost>) {
    let current = store
        .host_id_by_uid(uid)
        .ok()
        .flatten()
        .and_then(|id| store.get_host(id).ok().flatten());
    let old = |get: &dyn Fn(&ManagedHost) -> String| {
        current
            .as_ref()
            .map_or_else(|| "(new host)".to_string(), get)
    };
    let mut changes = Vec::new();
    let mut add = |field: &str, from: String, to: Option<String>| {
        if let Some(to) = to {
            changes.push(ChangeView {
                field: field.into(),
                from,
                to,
                dangerous: dangerous(EntityKind::Host, field),
            });
        }
    };
    add("name", old(&|m| m.name.clone()), f.name.clone());
    add("label", old(&|m| shown(&m.label)), patch(&f.label));
    add("address", old(&|m| m.address.clone()), f.address.clone());
    add("port", old(&|m| shown(&m.port)), patch(&f.port));
    add("username", old(&|m| shown(&m.username)), patch(&f.username));
    add(
        "proxy_jump",
        old(&|m| shown(&m.proxy_jump)),
        patch(&f.proxy_jump),
    );
    add(
        "forward_agent",
        old(&|m| shown(&m.forward_agent)),
        patch(&f.forward_agent),
    );
    add(
        "remote_command",
        old(&|m| shown(&m.remote_command)),
        patch(&f.remote_command),
    );
    add(
        "transport",
        old(&|m| shown(&m.transport.map(|t| format!("{t:?}").to_lowercase()))),
        patch(&f.transport),
    );
    add(
        "environment",
        old(&|m| shown(&m.environment)),
        patch(&f.environment),
    );
    add("os_icon", old(&|m| shown(&m.os_icon)), patch(&f.os_icon));
    add(
        "tags",
        old(&|m| m.tags.join(", ")),
        f.tags.as_ref().map(|t| t.join(", ")),
    );
    add("notes", old(&|m| shown(&m.notes)), patch(&f.notes));
    add(
        "favorite",
        old(&|m| m.favorite.to_string()),
        f.favorite.map(|b| b.to_string()),
    );
    add(
        "secret_lookup",
        old(&|m| shown(&m.secret_lookup)),
        patch(&f.secret_lookup),
    );
    add(
        "has_password",
        old(&|m| m.has_password.to_string()),
        f.has_password.map(|b| b.to_string()),
    );
    add(
        "group_uids",
        old(&|m| {
            let names: Vec<&str> = m.groups.iter().map(|g| g.name.as_str()).collect();
            if names.is_empty() {
                "(none)".into()
            } else {
                names.join(", ")
            }
        }),
        f.group_uids.as_ref().map(|g| {
            if g.is_empty() {
                "(none)".to_string()
            } else {
                g.iter()
                    .map(|u| group_name(store, u))
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        }),
    );
    add(
        "identity_uid",
        old(&|m| shown(&m.identity.as_ref().map(|i| i.name.clone()))),
        f.identity_uid
            .as_ref()
            .map(|i| shown(&i.as_ref().map(|u| identity_name(store, u)))),
    );
    (changes, current)
}

/// The completion of a ready phone-loss recovery (revoke the lost phone, admit the new one) is
/// not a Red change that waits for the lost phone's signature: the recovery already was the
/// ceremony. Say so in the review, or the reader looks for a phone proof that cannot come.
pub(crate) const COMPLETES_RECOVERY: &str =
    "completes the ready phone recovery: no signature of the lost phone is needed";

fn recovery_suffix(store: &LauncherStore, op: &Op) -> String {
    let completes = matches!(
        &op.body,
        OpBody::DeviceAdd {
            kind: crate::sync::oplog::DeviceKind::Phone,
            ..
        } | OpBody::DeviceRevoke { .. }
    ) && crate::sync::recovery::ready_recovery_for(
        store,
        op,
        // The ceremony's own clock: never later than the op was first seen here.
        crate::store::now_ts()
            .min(crate::sync::ceremony::received_at(store, op).unwrap_or(i64::MAX)),
    )
    .unwrap_or(false);
    if completes {
        format!(" [{COMPLETES_RECOVERY}]")
    } else {
        String::new()
    }
}

/// A share release is asked about only while its requester's challenge could still be live.
pub(crate) fn release_is_fresh(asked_at: i64, now: i64) -> bool {
    (now - asked_at).abs() <= crate::sync::limits::CHALLENGE_EXPIRY_SECS
}

/// What a pending op does, as the review screen shows it.
pub(crate) fn pending_view(
    store: &LauncherStore,
    op: &Op,
    devices: &[DeviceRow],
    reason: Option<String>,
) -> PendingOpView {
    let device = devices
        .iter()
        .find(|d| d.id == op.device_id)
        .map_or_else(|| short(&op.device_id), |d| d.name.clone());
    let mut changes = Vec::new();
    let mut summary = match &op.body {
        OpBody::HostUpsert { host_uid, fields } => {
            let (c, current) = host_changes(store, host_uid, fields);
            changes = c;
            let name = current
                .as_ref()
                .map(|m| m.name.clone())
                .or_else(|| fields.name.clone())
                .unwrap_or_else(|| short(&uid_hex(host_uid)));
            if current.is_some() {
                format!("change host {name}")
            } else {
                format!("add host {name}")
            }
        }
        OpBody::HostDelete { host_uid } => {
            let name = store
                .host_id_by_uid(host_uid)
                .ok()
                .flatten()
                .and_then(|id| store.get_host(id).ok().flatten())
                .map_or_else(|| short(&uid_hex(host_uid)), |m| m.name);
            format!("delete host {name}")
        }
        OpBody::GroupUpsert { group_uid, fields } => {
            let (c, current) = group_changes(store, group_uid, fields);
            changes = c;
            let name = current
                .as_ref()
                .map(|g| g.name.clone())
                .or_else(|| fields.name.clone())
                .unwrap_or_else(|| short(&uid_hex(group_uid)));
            if current.is_some() {
                format!("change group {name}")
            } else {
                format!("add group {name}")
            }
        }
        OpBody::GroupDelete { group_uid } => format!("delete group {}", short(&uid_hex(group_uid))),
        OpBody::IdentityUpsert {
            identity_uid,
            fields,
        } => {
            let (c, current) = identity_changes(store, identity_uid, fields);
            changes = c;
            let name = current
                .as_ref()
                .map(|i| i.name.clone())
                .or_else(|| fields.name.clone())
                .unwrap_or_else(|| short(&uid_hex(identity_uid)));
            if current.is_some() {
                format!("change identity {name}")
            } else {
                format!("add identity {name}")
            }
        }
        OpBody::IdentityDelete { identity_uid } => {
            format!("delete identity {}", short(&uid_hex(identity_uid)))
        }
        OpBody::DeviceAdd {
            device,
            name,
            role,
            kind,
            ..
        } => format!(
            "add {name} as a {} ({}), key {}",
            match (kind, role) {
                (crate::sync::oplog::DeviceKind::Phone, _) => "phone",
                (_, crate::sync::oplog::DeviceRole::Holder) => "full device",
                (_, crate::sync::oplog::DeviceRole::Mirror) => "view-only copy",
            },
            match kind {
                crate::sync::oplog::DeviceKind::Phone => "phone",
                crate::sync::oplog::DeviceKind::Terminal => "computer or server",
            },
            short(&device.device_id())
        ) + &recovery_suffix(store, op),
        OpBody::DeviceRevoke { device_id } => format!(
            "remove device {}{}",
            short(device_id),
            recovery_suffix(store, op)
        ),
        OpBody::ModeChange { to, cosigner, .. } => match cosigner {
            Some(c) => format!(
                "switch passwords to {to:?}, second signer {}",
                device_name(devices, c)
            ),
            None => format!("switch passwords to {to:?}"),
        },
        OpBody::ShareRelease {
            host_uid,
            requester,
            ..
        } => {
            let who = devices
                .iter()
                .find(|d| d.id == *requester)
                .map_or_else(|| short(requester), |d| d.name.clone());
            let what = store
                .host_id_by_uid(host_uid)
                .ok()
                .flatten()
                .and_then(|id| store.get_host(id).ok().flatten())
                .map_or_else(|| short(&uid_hex(host_uid)), |m| m.name);
            format!(
                "let {who} open the saved password of {what} (device {requester}, asked {} s ago)",
                (crate::store::now_ts() - op.timestamp).max(0)
            )
        }
        OpBody::BulkExport {
            requester,
            window_secs,
        } => format!(
            "bulk export of EVERY credential by {} for {window_secs} s",
            device_name(devices, requester)
        ),
        OpBody::CertIssue {
            device_id,
            scope,
            expiry,
            max_new_hosts_per_day,
            ..
        } => {
            let scope = match scope {
                CertScope::Group(g) => format!("group {}", group_name(store, g)),
                CertScope::Hosts(h) => {
                    let names: Vec<String> = h
                        .iter()
                        .take(5)
                        .map(|u| {
                            store
                                .host_id_by_uid(u)
                                .ok()
                                .flatten()
                                .and_then(|id| store.get_host(id).ok().flatten())
                                .map_or_else(|| short(&uid_hex(u)), |m| m.name)
                        })
                        .collect();
                    let more = h.len().saturating_sub(names.len());
                    if more > 0 {
                        format!("{} hosts ({}, and {more} more)", h.len(), names.join(", "))
                    } else {
                        format!("{} hosts ({})", h.len(), names.join(", "))
                    }
                }
            };
            let days = (expiry - op.timestamp).max(0) / 86_400;
            format!(
                "trust session: {} may open {scope} for {days} days, {max_new_hosts_per_day} new hosts a day",
                device_name(devices, device_id)
            )
        }
        OpBody::ExemptionSet { device_id, slot } => match slot {
            Some(key) => format!(
                "turn on the master phrase exemption for {} (it made its own key: only as trustworthy as that computer, slot key {})",
                device_name(devices, device_id),
                short(&key.device_id())
            ),
            None => format!(
                "turn off the master phrase exemption for {}",
                device_name(devices, device_id)
            ),
        },
        OpBody::DeviceRoleChange { device_id, to } => format!(
            "make {} a {}",
            device_name(devices, device_id),
            format!("{to:?}").to_lowercase()
        ),
        OpBody::FactorRegister { factor, sender, .. } => {
            let kind = match factor {
                ThirdFactor::Paper => "paper phrase",
                _ => "emailed code",
            };
            match sender {
                Some(s) => format!(
                    "register the {kind} as the third approval, sent by {}",
                    device_name(devices, s)
                ),
                None => format!("register the {kind} as the third approval"),
            }
        }
        other => {
            let dbg = format!("{other:?}");
            dbg.split([' ', '{', '('])
                .next()
                .unwrap_or("change")
                .to_string()
        }
    };
    let reason_text = reason.clone();
    if let Some(reason) = reason {
        summary.push_str(&format!(" [{}]", reason_words(&reason)));
    }
    PendingOpView {
        device,
        author: op.device_id.to_string(),
        summary,
        changes,
        hash: op.hash(),
        reason: reason_text,
        red: red_view(store, op),
    }
}

/// Group approval (design 7c): only a change held for being `stale` that touches no dangerous field and is no
/// ceremony. Everything else (address, jump host, command, deletes, share requests, big changes, any other reason)
/// stays per change with its full diff.
pub(crate) fn groupable(v: &PendingOpView) -> bool {
    v.reason.as_deref() == Some("stale")
        && v.red.is_none()
        && !v.changes.is_empty()
        && v.changes.iter().all(|c| !c.dangerous)
}

/// The hashes held for the same device as `review[selected]` that group approval would approve; empty when the
/// selected change is not groupable.
pub(crate) fn stale_group(review: &[PendingOpView], selected: usize) -> Vec<[u8; 32]> {
    let Some(sel) = review.get(selected).filter(|v| groupable(v)) else {
        return Vec::new();
    };
    review
        .iter()
        .filter(|v| v.author == sel.author && groupable(v))
        .map(|v| v.hash)
        .collect()
}

/// What a reason a change waits for means, in the glossary's words (the stored codes stay in JSON).
pub(crate) fn reason_words(reason: &str) -> String {
    match reason {
        "tier" => "needs your approval",
        "stale" => "made by a device that was away too long",
        "resurrection" => "brings back something that was deleted",
        "revoke-conflict" => "conflicts with a removal",
        "unknown-author" => "from a device this computer does not know yet",
        "future-dated" => "its time is in the future",
        "not-holder" => "made by a device that may not make this change",
        "revoked-author" => "made by a device that was removed",
        other => other,
    }
    .to_string()
}

pub(crate) fn proof_label(p: &crate::sync::ceremony::RedProof) -> &'static str {
    use crate::sync::ceremony::RedProof;
    use crate::sync::oplog::ThirdFactor;
    match p {
        RedProof::Phone => "the phone's approval",
        RedProof::SecondDevice => "a second device with a person at the keyboard",
        RedProof::Factor(None) => "paper phrase or emailed code",
        RedProof::Factor(Some(ThirdFactor::Paper)) => "paper phrase (24 words)",
        RedProof::Factor(Some(_)) => "emailed code",
    }
}

/// The ceremony of `op`, when it is a gated Red change.
fn red_view(store: &LauncherStore, op: &Op) -> Option<super::sync::RedView> {
    use crate::sync::ceremony::{is_gated, red_status};
    if !is_gated(store, op).ok()? {
        return None;
    }
    let st = red_status(store, op, crate::store::now_ts()).ok()?;
    Some(super::sync::RedView {
        have: st.have.iter().map(|p| proof_label(p).to_string()).collect(),
        missing: st
            .missing
            .iter()
            .map(|p| proof_label(p).to_string())
            .collect(),
        expired: st.expired,
        blocked: st.blocked,
        rule_violation: st.rule_violation,
        note: st.note,
        single_machine: st.single_machine_ring,
    })
}

/// The proof of a fork as lines for the alert: what was signed twice and where.
pub(crate) fn evidence_lines(e: &crate::sync::oplog::ForkEvidence) -> Vec<String> {
    use crate::sync::oplog::ForkEvidence;
    let h = |b: &[u8; 32]| {
        b[..4]
            .iter()
            .map(|x| format!("{x:02x}"))
            .collect::<String>()
    };
    let op = |o: &Op| {
        format!(
            "change number {} (lamport {}) hash {}",
            o.seq,
            o.lamport,
            h(&o.hash())
        )
    };
    match e {
        ForkEvidence::Ops(a, b) => vec![
            "Evidence: two different changes signed by the same device at the same position".into(),
            format!("  {}", op(a)),
            format!("  {}", op(b)),
        ],
        ForkEvidence::OpAndHead(o, head) => vec![
            "Evidence: a change and a last signed change that contradict each other".into(),
            format!("  {}", op(o)),
            format!(
                "  last signed change number {} hash {}",
                head.seq,
                h(&head.hash)
            ),
        ],
        ForkEvidence::Heads(a, b) => vec![
            "Evidence: two last signed changes that cannot both be true".into(),
            format!("  last signed change number {} hash {}", a.seq, h(&a.hash)),
            format!("  last signed change number {} hash {}", b.seq, h(&b.hash)),
        ],
    }
}

enum RedProofInput {
    Approve,
    Paper(zeroize::Zeroizing<String>),
    Email(zeroize::Zeroizing<String>),
}

/// Add one proof to a pending Red change on the worker and settle: the change
/// takes effect only when the ceremony is complete.
fn red_proof(
    env: &crate::sync::runtime::JobEnv,
    hash: &[u8; 32],
    proof: RedProofInput,
) -> Result<String> {
    use crate::sync::ceremony;
    let op = env
        .store
        .sync_pending()?
        .into_iter()
        .find(|o| o.hash() == *hash)
        .ok_or_else(|| anyhow::anyhow!("that change is no longer waiting"))?;
    ceremony::require_gated(&env.store, &op)?;
    let body = match proof {
        RedProofInput::Approve => ceremony::red_approve_body(&op),
        RedProofInput::Paper(words) => {
            let recover = crate::sync::crypto::paper_derive(
                words.trim(),
                crate::sync::crypto::PaperDomain::Recover,
            )?;
            ceremony::red_factor_paper(&recover, &op)
        }
        RedProofInput::Email(code) => ceremony::red_factor_email(&env.store, code.trim(), &op)?,
    };
    env.store.sync_append_local(&env.identity, body)?;
    let now = crate::store::now_ts();
    let report = ceremony::settle_red_ops(&env.store, now)?;
    if report.applied.contains(hash) {
        return Ok("all three approvals are in: the change took effect".into());
    }
    let left = ceremony::red_status(&env.store, &op, now)?
        .missing
        .iter()
        .map(|p| proof_label(p))
        .collect::<Vec<_>>()
        .join(", ");
    Ok(if left.is_empty() {
        "approval added".to_string()
    } else {
        format!("approval added; still missing: {left}")
    })
}

impl App {
    /// Re-read the Sync view from the store. A ring exists once the store knows
    /// a device. On a read failure the view keeps what it had and says why.
    pub fn refresh_sync_view(&mut self) {
        if let Err(e) = self.try_refresh_sync_view() {
            self.sync.last_error = Some(format!("{e:#}"));
        }
        self.sync.finish_row = super::sync_flows::finish_row(self);
        self.update_recovery_alert();
    }

    /// One alert for the recoveries open on this terminal (`recovery::open_for_alert`), raised full-screen unless
    /// every one of them was dismissed. It never writes an op.
    fn update_recovery_alert(&mut self) {
        let ops = self.recovery_alert_ops();
        let now = crate::store::now_ts();
        let view = crate::cli::sync_admin::alert_view(&self.store, &ops, now).ok().flatten();
        let Some(view) = view else {
            self.sync.alert = None;
            if self.mode == AppMode::SyncRecoveryAlert {
                self.mode = self.sync.alert_return_take();
            }
            return;
        };
        let fresh = view.hashes.iter().any(|h| !self.sync.alert_dismissed.contains(h));
        self.sync.alert = Some(super::sync::RecoveryAlert {
            lines: crate::cli::sync_admin::alert_lines(&view, now),
            hashes: view.hashes,
        });
        if fresh && self.mode != AppMode::SyncRecoveryAlert {
            self.sync.alert_return_set(self.mode);
            self.mode = AppMode::SyncRecoveryAlert;
        }
    }

    /// The recoveries to alert about; empty until this device's identity is known and loadable.
    fn recovery_alert_ops(&mut self) -> Vec<crate::sync::oplog::Op> {
        if self.sync.this_device.is_none() {
            return Vec::new();
        }
        if self.sync_identity.is_none() {
            let Some(paths) = self.profile.as_ref() else {
                return Vec::new();
            };
            let secrets = crate::credentials::open_for_profile(paths);
            match crate::sync::identity_store::load_or_init_identity(paths, secrets.as_ref()) {
                Ok(id) => self.sync_identity = Some(std::sync::Arc::new(id)),
                Err(_) => return Vec::new(),
            }
        }
        let me = self.sync_identity.as_ref().expect("loaded above");
        crate::sync::recovery::open_for_alert(&self.store, me, crate::store::now_ts()).unwrap_or_default()
    }

    fn try_refresh_sync_view(&mut self) -> Result<()> {
        let store = self.store.clone();
        // Roles and the mode as the ceremony sees them: a change whose proofs are not in is not in force.
        let rows = crate::sync::ceremony::effective_devices(&store, None)?;
        // Every pending op with its reason, `unknown-author` ones included.
        let held = store.sync_pending_with_reasons()?;
        let review: Vec<PendingOpView> = held
            .iter()
            .map(|(op, reason)| pending_view(&store, op, &rows, Some(reason.clone())))
            .collect();
        let ops: Vec<Op> = held.into_iter().map(|(op, _)| op).collect();
        let cred_mode = if rows.is_empty() {
            None
        } else {
            Some(match crate::sync::ceremony::effective_cred_mode(&store)? {
                CredMode::Vault => CredModeView::Vault,
                CredMode::Quorum => CredModeView::Quorum,
            })
        };
        let this_device = self.sync.this_device.clone();
        let sync = &mut self.sync;
        sync.configured = !rows.is_empty();
        sync.devices = rows
            .iter()
            .map(|r| {
                let mut v = device_view(r);
                v.verified_by = Some(r.verified_by);
                v.this = this_device.as_deref() == Some(r.id.as_str());
                v
            })
            .collect();
        sync.selected = sync.selected.min(sync.devices.len().saturating_sub(1));
        sync.pending = review.len();
        sync.review_selected = sync.review_selected.min(review.len().saturating_sub(1));
        sync.review = review;
        sync.cred_mode = cred_mode;
        let now = crate::store::now_ts();
        sync.health = crate::sync::health::health(&store, this_device.as_deref(), now)?;
        sync.cut_off_missing = crate::sync::revocation::terminals_missing_to_cut_off(&store)?;
        sync.exposure = crate::cli::sync_admin::exposure_lines(&store)?;
        sync.exposed_hosts = crate::cli::sync_admin::exposed_host_names(&store)?;
        sync.no_paper = sync.configured && crate::sync::quorum::paper_pub(&store)?.is_none();
        if sync.ring_secret_missing.is_none() {
            // Only a Vault ring with a master key set up needs the ring secret on this machine.
            let needs = cred_mode == Some(CredModeView::Vault)
                && crate::sync::quorum::vault_pub(&store)?.is_some();
            sync.ring_secret_missing = Some(
                needs
                    && self.profile.as_ref().is_some_and(|p| {
                        crate::credentials::open_ungated_for_profile(p)
                            .get(crate::sync::runtime::RING_SECRET_KEY)
                            .ok()
                            .flatten()
                            .is_none()
                    }),
            );
        }
        (self.sync.trust_scopes, self.sync.trust_scope_values) =
            super::sync_trust::scope_choices(&store);
        // A holder is asked for a share: the prompt names the requester.
        if self.mode == AppMode::Normal && self.sync.release.is_none() {
            let answered = self.sync.answered_releases.clone();
            let asked = ops.iter().find_map(|op| match &op.body {
                OpBody::ShareRelease {
                    host_uid,
                    requester,
                    ..
                } if !answered.contains(&op.hash())
                    // A stale request is not asked about: the requester's challenge is long gone.
                    && release_is_fresh(op.timestamp, crate::store::now_ts()) =>
                {
                    Some((op.hash(), *host_uid, requester.clone(), op.timestamp))
                }
                _ => None,
            });
            if let Some((hash, host_uid, requester, asked_at)) = asked {
                let requester_name = rows
                    .iter()
                    .find(|d| d.id == requester)
                    .map_or_else(|| short(&requester), |d| d.name.clone());
                let host = store
                    .host_id_by_uid(&host_uid)
                    .ok()
                    .flatten()
                    .and_then(|id| store.get_host(id).ok().flatten())
                    .map_or_else(|| short(&uid_hex(&host_uid)), |m| m.name);
                self.open_sync_release_confirm(super::sync::ReleaseView {
                    host,
                    requester: requester_name,
                    requester_id: requester,
                    hash,
                    age_secs: (crate::store::now_ts() - asked_at).max(0),
                });
            }
        }
        Ok(())
    }

    /// An identity's key file was set or changed: replicate its bytes with the
    /// credential when the profile is in a ring (the passphrase goes through
    /// `PasswordStore::set` on its own).
    pub(crate) fn replicate_identity_key(&mut self, id: i64) {
        let result = self
            .password_store
            .gate()
            .and_then(|g| g.replicator())
            .map(|r| r.replicate_identity_key(id));
        if let Some(Err(e)) = result {
            self.identity_notice = Some(format!("Saved, but replicating the key failed: {e:#}"));
        }
    }

    /// `[sync] auto_sync_on_start`: queue one sync with every ring device that
    /// has a known host. Quiet when there is no ring or no host to reach.
    pub fn queue_auto_sync(&mut self) {
        if !self.config.sync.auto_sync_on_start || !self.sync.configured {
            return;
        }
        let Some(paths) = self.profile.as_ref() else {
            return;
        };
        let Ok(peers) = crate::sync::runtime::Peers::load(paths) else {
            return;
        };
        let known: Vec<String> = self
            .sync
            .devices
            .iter()
            .filter(|d| peers.host_of(&d.id).is_some())
            .map(|d| d.id.clone())
            .collect();
        for device in known {
            self.sync
                .intents
                .push(super::sync::SyncIntent::SyncNow { device });
        }
    }

    /// A key in the review list: `a` approve (a Red change: add this device's
    /// approval), `r` reject, `p` type the paper phrase, `e` type the emailed code.
    pub(crate) fn sync_review_key(&mut self, c: char) {
        let Some(sel) = self.sync.review.get(self.sync.review_selected) else {
            return;
        };
        let hash = sel.hash;
        match (c, sel.red.is_some()) {
            ('r', _) | ('a', false) => self.sync_resolve_review(c == 'a'),
            ('a', true) => {
                self.run_sync_job(move |env| red_proof(env, &hash, RedProofInput::Approve))
            }
            ('p', true) => self.sync_prompt(
                "Paper phrase",
                "Type the 24 words of the paper phrase (nothing is shown)",
                true,
                move |app, words| {
                    app.run_sync_job(move |env| {
                        red_proof(env, &hash, RedProofInput::Paper(words.into()))
                    })
                },
            ),
            ('e', true) => self.sync_prompt(
                "Emailed code",
                "Type the code from the email (nothing is shown)",
                true,
                move |app, code| {
                    app.run_sync_job(move |env| {
                        red_proof(env, &hash, RedProofInput::Email(code.into()))
                    })
                },
            ),
            _ => {}
        }
    }

    /// `A` in the review: approve, on this device, every change of the highlighted change's device that is held only
    /// for being away and touches no dangerous field (design 7c). Nothing else is approved.
    pub(crate) fn sync_approve_group(&mut self) {
        let group = stale_group(&self.sync.review, self.sync.review_selected);
        if group.is_empty() {
            self.host_notice = Some(
                "group approval is only for changes held for being away that touch no address, jump host or command: approve this one on its own".into(),
            );
            return;
        }
        let name = self.sync.review[self.sync.review_selected].device.clone();
        let now = crate::store::now_ts();
        let done = group
            .iter()
            .filter(|h| crate::sync::ceremony::resolve(&self.store, h, true, now).is_ok())
            .count();
        self.refresh_sync_view();
        let _ = self.reload_hosts();
        if self.sync.review.is_empty() {
            self.mode = AppMode::Normal;
        }
        self.host_notice = Some(format!(
            "approved {done} change(s) from {name} on this device only: your next edits will be held on other devices until approved there"
        ));
    }

    /// Approve or reject the highlighted pending operation in the store.
    pub(crate) fn sync_resolve_review(&mut self, approve: bool) {
        let Some(op) = self.sync.review.get(self.sync.review_selected) else {
            return;
        };
        let hash = op.hash;
        // The only way a screen approves: a Red change needs its ceremony.
        match crate::sync::ceremony::resolve(&self.store, &hash, approve, crate::store::now_ts()) {
            Ok(()) => {
                self.refresh_sync_view();
                // A change that took effect shows in the hosts list.
                if approve {
                    let _ = self.reload_hosts();
                }
                if self.sync.review.is_empty() {
                    self.mode = AppMode::Normal;
                }
            }
            Err(e) => self.host_notice = Some(format!("sync: {e:#}")),
        }
    }
}

#[cfg(test)]
mod release_age_tests {
    use super::release_is_fresh;

    #[test]
    fn a_release_request_older_than_the_challenge_lifetime_is_not_asked_about() {
        let limit = crate::sync::limits::CHALLENGE_EXPIRY_SECS;
        assert!(release_is_fresh(1_000, 1_000));
        assert!(release_is_fresh(1_000, 1_000 + limit));
        assert!(!release_is_fresh(1_000, 1_000 + limit + 1));
        assert!(
            !release_is_fresh(1_000 + limit + 1, 1_000),
            "future-dated too"
        );
    }
}
