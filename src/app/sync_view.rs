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
    if let Some(reason) = reason {
        summary.push_str(&format!(" [{reason}]"));
    }
    PendingOpView {
        device,
        summary,
        changes,
        hash: op.hash(),
    }
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
        let ops = store.sync_pending()?;
        let review: Vec<PendingOpView> = ops
            .iter()
            .map(|op| {
                let reason = store.sync_pending_reason(&op.hash()).ok().flatten();
                pending_view(&store, op, &rows, reason)
            })
            .collect();
        let cred_mode = if rows.is_empty() {
            None
        } else {
            Some(match store.sync_cred_mode()? {
                CredMode::Vault => CredModeView::Vault,
                CredMode::Quorum => CredModeView::Quorum,
            })
        };
        let sync = &mut self.sync;
        sync.configured = !rows.is_empty();
        sync.devices = rows.iter().map(device_view).collect();
        sync.selected = sync.selected.min(sync.devices.len().saturating_sub(1));
        sync.pending = review.len();
        sync.review_selected = sync.review_selected.min(review.len().saturating_sub(1));
        sync.review = review;
        sync.cred_mode = cred_mode;
        Ok(())
    }

    /// Approve or reject the highlighted pending operation in the store.
    pub(crate) fn sync_resolve_review(&mut self, approve: bool) {
        let Some(op) = self.sync.review.get(self.sync.review_selected) else {
            return;
        };
        let hash = op.hash;
        match self.store.sync_resolve_pending(&hash, approve) {
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
