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
    Attendance, CredMode, DeviceRole, DeviceRow, DeviceStatus, HostFields, Op, OpBody,
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
        old(&|m| format!("{} groups", m.groups.len())),
        f.group_uids.as_ref().map(|g| format!("{} groups", g.len())),
    );
    add(
        "identity_uid",
        old(&|m| shown(&m.identity.as_ref().map(|i| i.name.clone()))),
        f.identity_uid
            .as_ref()
            .map(|i| shown(&i.as_ref().map(uid_hex).map(|h| short(&h)))),
    );
    (changes, current)
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
            let name = fields
                .name
                .clone()
                .unwrap_or_else(|| short(&uid_hex(group_uid)));
            format!("change group {name}")
        }
        OpBody::GroupDelete { group_uid } => format!("delete group {}", short(&uid_hex(group_uid))),
        OpBody::IdentityUpsert { fields, .. } => {
            format!("change identity {}", shown(&fields.name))
        }
        OpBody::IdentityDelete { identity_uid } => {
            format!("delete identity {}", short(&uid_hex(identity_uid)))
        }
        OpBody::DeviceAdd { name, .. } => format!("add device {name}"),
        OpBody::DeviceRevoke { device_id } => format!("revoke device {}", short(device_id)),
        OpBody::ModeChange { to, .. } => format!("switch credential mode to {to:?}"),
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
            format!("release a share of {what} to {who}")
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
        summary.push_str(&format!(" [{reason}]"));
    }
    PendingOpView {
        device,
        summary,
        changes,
        hash: op.hash(),
        reason: reason_text,
        red: red_view(store, op),
    }
}

fn proof_label(p: &crate::sync::ceremony::RedProof) -> &'static str {
    use crate::sync::ceremony::RedProof;
    use crate::sync::oplog::ThirdFactor;
    match p {
        RedProof::Phone => "phone signature",
        RedProof::SecondDevice => "second attended device",
        RedProof::Factor(None) => "third factor (paper or emailed code)",
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
            "operation seq {} lamport {} hash {}",
            o.seq,
            o.lamport,
            h(&o.hash())
        )
    };
    match e {
        ForkEvidence::Ops(a, b) => vec![
            "Proof: two different operations signed by the same device at the same position".into(),
            format!("  {}", op(a)),
            format!("  {}", op(b)),
        ],
        ForkEvidence::OpAndHead(o, head) => vec![
            "Proof: an operation and a signed head that contradict each other".into(),
            format!("  {}", op(o)),
            format!("  signed head seq {} hash {}", head.seq, h(&head.hash)),
        ],
        ForkEvidence::Heads(a, b) => vec![
            "Proof: two signed heads that cannot both be true".into(),
            format!("  signed head seq {} hash {}", a.seq, h(&a.hash)),
            format!("  signed head seq {} hash {}", b.seq, h(&b.hash)),
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
        return Ok("the ceremony is complete: the change took effect".into());
    }
    let left = ceremony::red_status(&env.store, &op, now)?
        .missing
        .iter()
        .map(|p| proof_label(p))
        .collect::<Vec<_>>()
        .join(", ");
    Ok(if left.is_empty() {
        "proof added".to_string()
    } else {
        format!("proof added; still missing: {left}")
    })
}

impl App {
    /// Re-read the Sync view from the store. A ring exists once the store knows
    /// a device. On a read failure the view keeps what it had and says why.
    pub fn refresh_sync_view(&mut self) {
        if let Err(e) = self.try_refresh_sync_view() {
            self.sync.last_error = Some(format!("{e:#}"));
        }
    }

    fn try_refresh_sync_view(&mut self) -> Result<()> {
        let store = self.store.clone();
        let rows = store.sync_devices()?;
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
            Some(match store.sync_cred_mode()? {
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
        // A holder is asked for a share: the prompt names the requester.
        if self.mode == AppMode::Normal && self.sync.release.is_none() {
            let answered = self.sync.answered_releases.clone();
            let asked = ops.iter().find_map(|op| match &op.body {
                OpBody::ShareRelease {
                    host_uid,
                    requester,
                    ..
                } if !answered.contains(&op.hash()) => {
                    Some((op.hash(), *host_uid, requester.clone()))
                }
                _ => None,
            });
            if let Some((hash, host_uid, requester)) = asked {
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
