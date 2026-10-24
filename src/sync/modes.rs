//! Credential mode changes (Red): Vault to Quorum and back. The machine that migrates opens every
//! credential, so it is the one machine that sees them all; the ceremony names it.

use anyhow::{ensure, Context, Result};
use zeroize::Zeroizing;

use super::cred_ops::{cred_update_body_for, CredPayload};
use super::oplog::{CredMode, DeviceId, DeviceRole, DeviceStatus, HostUid, Op, OpBody};
use crate::store::LauncherStore;

pub fn credentials(store: &LauncherStore) -> Result<Vec<HostUid>> {
    store.with_conn(|c| {
        let mut st = c.prepare("SELECT host_uid FROM sync_cred_state ORDER BY host_uid")?;
        let rows = st.query_map([], |r| r.get::<_, Vec<u8>>(0))?.collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows.into_iter().filter_map(|u| HostUid::try_from(u).ok()).collect())
    })
}

/// Bodies of a mode change in the order to append them: the `ModeChange` first, then one
/// `CredUpdate` per credential, re-encrypted under a fresh DEK and wrapped for the new mode.
/// `open` returns a credential's plaintext payload (vault: phrase and ring secret; quorum: two
/// shares). `cosigner` is required for Quorum to Vault.
pub fn mode_change_plan(
    store: &LauncherStore,
    me: &DeviceId,
    to: CredMode,
    cosigner: Option<DeviceId>,
    open: &dyn Fn(&HostUid) -> Result<Zeroizing<Vec<u8>>>,
) -> Result<Vec<OpBody>> {
    let from = store.sync_cred_mode()?;
    ensure!(from != to, "the ring is already in that mode");
    let mut bodies = vec![OpBody::ModeChange { to, migrated_on: me.clone(), cosigner: cosigner.clone() }];
    if to == CredMode::Vault {
        ensure!(cosigner.is_some(), "going back to Vault downgrades the ring's promise and needs a holder's co-signature");
    }
    for host in credentials(store)? {
        let plain = open(&host).with_context(|| format!("opening a credential for the migration"))?;
        let payload = CredPayload::from_bytes(&plain)?;
        bodies.push(cred_update_body_for(store, &host, &payload, to)?);
    }
    Ok(bodies)
}

/// Checks a `ModeChange` op before it is shown for approval.
pub fn validate_mode_change(store: &LauncherStore, op: &Op) -> Result<()> {
    let OpBody::ModeChange { to, migrated_on, cosigner } = &op.body else { anyhow::bail!("not a mode change") };
    ensure!(*migrated_on == op.device_id, "a mode change names the machine that migrates: it must be the author");
    if *to == CredMode::Vault {
        let cosigner = cosigner.as_ref().context("going back to Vault needs a co-signing holder")?;
        ensure!(*cosigner != op.device_id, "the co-signer must be another device");
        let ok = store.sync_devices()?.into_iter().any(|d| {
            d.id == *cosigner && d.role == DeviceRole::Holder && !matches!(d.status, DeviceStatus::Revoked | DeviceStatus::Compromised)
        });
        ensure!(ok, "the co-signer is not a live holder");
    }
    Ok(())
}
