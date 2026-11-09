//! Checkpoint scheduling (C15): called once per completed sync round.
//!
//! In order: record proposals we hold a blob for (collecting the signs we know),
//! sign the ones that match our own fold (never on an unattended device), propose
//! when the log has grown enough, and prune once the delay has passed. Every
//! step is a store call; nothing here touches the network.

use anyhow::Result;

use crate::store::LauncherStore;
use crate::sync::crypto::DeviceIdentity;
use crate::sync::limits::*;
use crate::sync::oplog::{CheckpointStatus, PruneReport};

/// Thresholds; [`Policy::default`] is the contract values.
#[derive(Clone, Copy, Debug)]
pub struct Policy {
    pub ops: u64,
    pub bytes: u64,
    pub min_interval_secs: i64,
    pub after_reshare_secs: i64,
}

impl Default for Policy {
    fn default() -> Self {
        #[allow(unused_mut)]
        let mut p = Self {
            ops: CHECKPOINT_OPS,
            bytes: CHECKPOINT_BYTES,
            min_interval_secs: CHECKPOINT_MIN_INTERVAL_SECS,
            after_reshare_secs: CHECKPOINT_AFTER_RESHARE_SECS,
        };
        // The e2e image (built with `--features e2e-hooks`, never a release) may shrink the thresholds so a scenario
        // need not write 5000 ops or wait a day: SSHUB_E2E_CHECKPOINT_OPS and SSHUB_E2E_CHECKPOINT_MIN_INTERVAL_SECS.
        #[cfg(feature = "e2e-hooks")]
        {
            if let Some(v) = std::env::var("SSHUB_E2E_CHECKPOINT_OPS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
            {
                p.ops = v;
            }
            if let Some(v) = std::env::var("SSHUB_E2E_CHECKPOINT_MIN_INTERVAL_SECS")
                .ok()
                .and_then(|v| v.parse::<i64>().ok())
            {
                p.min_interval_secs = v;
            }
        }
        p
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct DriverReport {
    pub proposed: Option<[u8; 32]>,
    pub signed: Vec<[u8; 32]>,
    pub accepted: Vec<[u8; 32]>,
    pub pruned: PruneReport,
}

/// Is a new checkpoint due at `now`? Pure over what the store reports.
pub fn due(store: &LauncherStore, now: i64, policy: &Policy) -> Result<bool> {
    let rows = store.sync_checkpoints()?;
    // One open proposal at a time.
    if rows.iter().any(|c| {
        matches!(
            c.status,
            CheckpointStatus::Proposed | CheckpointStatus::Verified
        )
    }) {
        return Ok(false);
    }
    let last_accepted = rows
        .iter()
        .filter(|c| c.status == CheckpointStatus::Accepted)
        .max_by_key(|c| c.accepted_at);
    let last_proposed_at = store.sync_last_checkpoint_time()?;
    if last_proposed_at.is_some_and(|t| now - t < policy.min_interval_secs) {
        return Ok(false);
    }
    let since_lamport = last_accepted
        .map(|c| c.cut.iter().map(|e| e.lamport).max().unwrap_or(0))
        .unwrap_or(0);
    let (ops, bytes) = store.sync_log_growth(since_lamport)?;
    if ops >= policy.ops || bytes >= policy.bytes {
        return Ok(true);
    }
    // A finished reshare retires old wrap keys: checkpoint soon after it.
    if let Some(t) = store.sync_last_reshare_time(since_lamport)? {
        return Ok(now - t >= policy.after_reshare_secs);
    }
    Ok(false)
}

/// One pass of the driver. Errors of single steps (a proposal that cannot be
/// built right now) are not failures of the round.
pub fn after_round(
    store: &LauncherStore,
    id: &DeviceIdentity,
    now: i64,
    policy: &Policy,
) -> Result<DriverReport> {
    let mut report = DriverReport::default();
    let me = id.public().device_id();
    for row in store.sync_checkpoints()? {
        if !matches!(
            row.status,
            CheckpointStatus::Proposed | CheckpointStatus::Verified
        ) {
            continue;
        }
        let Some(bundle) = store.sync_checkpoint_bundle(&row.proposal_hash)? else {
            continue;
        };
        if !row.signers.contains(&me) {
            if let Some(sign) = store.sync_checkpoint_sign(id, &bundle.proposal, &bundle.state)? {
                let _ = sign;
                report.signed.push(row.proposal_hash);
            }
        }
        if store.sync_checkpoint_update(&row.proposal_hash)? == Some(CheckpointStatus::Accepted) {
            report.accepted.push(row.proposal_hash);
        }
    }
    if due(store, now, policy)? {
        // Not building (pending ops, a running recovery, not an attended terminal)
        // just means: not now.
        match store.sync_checkpoint_propose(id, None, now) {
            Ok((op, _)) => {
                report.proposed = Some(op.hash());
                let _ = crate::sync::quorum::meta_set(store, "checkpoint_propose_error", b"");
            }
            // Not a failure of the round, but never silent: the reason is kept where support can read it.
            Err(e) => {
                let _ = crate::sync::quorum::meta_set(
                    store,
                    "checkpoint_propose_error",
                    format!("{now}: {e:#}").as_bytes(),
                );
            }
        }
    }
    report.pruned = store.sync_prune(now)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::oplog::{Attendance, DeviceKind, DeviceRole, HostFields, OpBody, VerifiedBy};

    fn store_with_device() -> (LauncherStore, DeviceIdentity) {
        let store = LauncherStore::open_in_memory().unwrap();
        let id = DeviceIdentity::generate();
        store
            .sync_register_device(
                &id.public(),
                "me",
                DeviceRole::Holder,
                Attendance::Attended,
                DeviceKind::Terminal,
                VerifiedBy::Sas,
            )
            .unwrap();
        (store, id)
    }

    fn host(store: &LauncherStore, id: &DeviceIdentity, n: u8, now: i64) {
        store
            .sync_append_local_at(
                id,
                OpBody::HostUpsert {
                    host_uid: [n; 16],
                    fields: HostFields {
                        name: Some(format!("h{n}")),
                        ..Default::default()
                    },
                },
                now,
            )
            .unwrap();
    }

    fn quiet() -> Policy {
        Policy {
            ops: u64::MAX,
            bytes: u64::MAX,
            min_interval_secs: 0,
            after_reshare_secs: 7 * 86_400,
        }
    }

    #[test]
    fn nothing_is_due_in_an_empty_log() {
        let (store, _) = store_with_device();
        assert!(!due(&store, 1_000, &Policy::default()).unwrap());
    }

    #[test]
    fn growth_in_ops_or_bytes_makes_a_checkpoint_due() {
        let (store, id) = store_with_device();
        for n in 0..3 {
            host(&store, &id, n, 100);
        }
        assert!(!due(&store, 200, &Policy { ops: 4, ..quiet() }).unwrap());
        assert!(due(&store, 200, &Policy { ops: 3, ..quiet() }).unwrap());
        assert!(!due(
            &store,
            200,
            &Policy {
                bytes: u64::MAX,
                ..quiet()
            }
        )
        .unwrap());
        assert!(due(
            &store,
            200,
            &Policy {
                bytes: 1,
                ..quiet()
            }
        )
        .unwrap());
    }

    #[test]
    fn an_open_proposal_and_the_minimum_interval_hold_the_next_one_back() {
        let (store, id) = store_with_device();
        for n in 0..3 {
            host(&store, &id, n, 100);
        }
        let policy = Policy {
            ops: 1,
            min_interval_secs: 86_400,
            ..quiet()
        };
        assert!(due(&store, 200, &policy).unwrap());
        let report = after_round(&store, &id, 200, &policy).unwrap();
        assert!(report.proposed.is_some(), "{report:?}");
        // A proposal is open (one device cannot reach two signers): not due again.
        assert!(!due(&store, 200 + 2 * 86_400, &policy).unwrap());
        assert_eq!(
            after_round(&store, &id, 300, &policy).unwrap().proposed,
            None
        );
    }

    #[test]
    fn a_finished_reshare_makes_one_due_after_the_delay() {
        let (store, id) = store_with_device();
        let now = 10 * 86_400;
        store
            .sync_append_local_at(
                &id,
                OpBody::ShareReshareBatch {
                    epoch: 1,
                    new_holders: vec![],
                    items: vec![],
                },
                now,
            )
            .unwrap();
        assert!(!due(&store, now + 86_400, &quiet()).unwrap());
        assert!(due(&store, now + 7 * 86_400, &quiet()).unwrap());
    }

    #[test]
    fn an_unattended_device_never_proposes_or_signs() {
        let store = LauncherStore::open_in_memory().unwrap();
        let id = DeviceIdentity::generate();
        store
            .sync_register_device(
                &id.public(),
                "nas",
                DeviceRole::Mirror,
                Attendance::Unattended,
                DeviceKind::Terminal,
                VerifiedBy::Sas,
            )
            .unwrap();
        // sync_append_local registers the device as attended only when unknown.
        for n in 0..3 {
            host(&store, &id, n, 100);
        }
        let report = after_round(&store, &id, 200, &Policy { ops: 1, ..quiet() }).unwrap();
        assert_eq!(report.proposed, None);
        assert!(report.signed.is_empty());
    }

    /// Scale: at the default policy a log of 5100 ops of one attended terminal gets a checkpoint proposed.
    #[test]
    fn a_log_past_the_op_trigger_gets_a_proposal_at_the_default_policy() {
        let (store, id) = store_with_device();
        for n in 0..5100u32 {
            store
                .sync_append_local_at(
                    &id,
                    OpBody::HostUpsert {
                        host_uid: {
                            let mut u = [0u8; 16];
                            u[..4].copy_from_slice(&n.to_be_bytes());
                            u
                        },
                        fields: HostFields {
                            name: Some(format!("h{n}")),
                            ..Default::default()
                        },
                    },
                    100,
                )
                .unwrap();
        }
        let report = after_round(&store, &id, 100_000, &Policy::default()).unwrap();
        assert!(report.proposed.is_some(), "{report:?}");
    }

    /// The thresholds shrink through the environment only in a build with `--features e2e-hooks` (the e2e image).
    #[test]
    fn the_checkpoint_env_hooks_exist_only_in_the_e2e_build() {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("SSHUB_E2E_CHECKPOINT_OPS", "20");
        std::env::set_var("SSHUB_E2E_CHECKPOINT_MIN_INTERVAL_SECS", "5");
        let p = Policy::default();
        std::env::remove_var("SSHUB_E2E_CHECKPOINT_OPS");
        std::env::remove_var("SSHUB_E2E_CHECKPOINT_MIN_INTERVAL_SECS");
        #[cfg(feature = "e2e-hooks")]
        assert_eq!((p.ops, p.min_interval_secs), (20, 5));
        #[cfg(not(feature = "e2e-hooks"))]
        assert_eq!(
            (p.ops, p.min_interval_secs),
            (CHECKPOINT_OPS, CHECKPOINT_MIN_INTERVAL_SECS)
        );
    }
}
