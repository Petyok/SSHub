//! Peers check what arrives. `screen_pending` runs after every round: a pending op that the rules of
//! stage 3 refuse is rejected at once instead of waiting for a user to notice; the others stay
//! pending for review (with their verdict available from `recovery::validate_phone_reenroll` and
//! the ceremony status).

use anyhow::Result;

use super::cert::validate_cert_issue;
use super::modes::validate_mode_change;
use super::oplog::{Op, OpBody};
use super::quorum::phone_device;
use super::recovery::validate_phone_reenroll;
use crate::store::LauncherStore;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ScreenReport {
    /// Ops rejected, with the reason.
    pub rejected: Vec<([u8; 32], String)>,
}

fn verdict(store: &LauncherStore, op: &Op, now: i64) -> Result<(), String> {
    let r = match &op.body {
        OpBody::PhoneReenroll { .. } => validate_phone_reenroll(store, op, now).map(|_| ()),
        OpBody::ModeChange { .. } => validate_mode_change(store, op),
        OpBody::CertIssue { .. } => match phone_device(store) {
            Ok(Some(phone)) => validate_cert_issue(store, op, &phone),
            Ok(None) => Err(anyhow::anyhow!(
                "the ring has no phone: only the phone issues certificates"
            )),
            Err(e) => Err(e),
        },
        _ => Ok(()),
    };
    r.map_err(|e| format!("{e:#}"))
}

/// Reject every pending op that fails its validator. Never an error for a bad op; `Err` only when
/// the store itself fails.
pub fn screen_pending(store: &LauncherStore, now: i64) -> Result<ScreenReport> {
    let mut report = ScreenReport::default();
    for op in store.sync_pending()? {
        if super::ceremony::awaits_author(store, &op)? {
            continue;
        }
        if let Err(why) = verdict(store, &op, now) {
            store.sync_resolve_pending(&op.hash(), false)?;
            report.rejected.push((op.hash(), why));
        }
    }
    // Red ops whose ceremony completed (or can never complete) are settled here too.
    super::ceremony::settle_red_ops(store, now)?;
    Ok(report)
}
