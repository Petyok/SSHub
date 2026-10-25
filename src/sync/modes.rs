//! Credential mode changes (Red): Vault to Quorum and back. The machine that migrates opens every
//! credential, so it is the one machine that sees them all; the ceremony names it.

use anyhow::{ensure, Context, Result};
use zeroize::Zeroizing;

use super::cred_ops::{cred_update_body_for, CredPayload};
use super::crypto::DevicePublic;
use super::oplog::{CredMode, DeviceId, DeviceRole, DeviceStatus, HostUid, Op, OpBody};
use crate::store::LauncherStore;

pub fn credentials(store: &LauncherStore) -> Result<Vec<HostUid>> {
    store.with_conn(|c| {
        let mut st = c.prepare("SELECT host_uid FROM sync_cred_state ORDER BY host_uid")?;
        let rows = st
            .query_map([], |r| r.get::<_, Vec<u8>>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            .filter_map(|u| HostUid::try_from(u).ok())
            .collect())
    })
}

/// The `ModeChange` op body to propose. `cosigner` is required for Quorum to Vault. The credentials are
/// NOT re-encrypted here: a proposer applies its own op at once, but nothing it publishes about the
/// new mode may reach the ring before the ceremony completed. Once it did, [`mode_migration`] builds
/// the re-encryption.
pub fn mode_change_body(
    store: &LauncherStore,
    me: &DevicePublic,
    to: CredMode,
    cosigner: Option<DeviceId>,
) -> Result<OpBody> {
    let from = super::ceremony::effective_cred_mode(store)?;
    ensure!(from != to, "the ring is already in that mode");
    if to == CredMode::Vault {
        ensure!(
            cosigner.is_some(),
            "going back to Vault downgrades the ring's promise and needs a holder's co-signature"
        );
    }
    Ok(OpBody::ModeChange {
        to,
        migrated_on: me.device_id(),
        cosigner,
    })
}

/// This machine's newest `ModeChange` op, if any.
pub fn own_mode_change(store: &LauncherStore, me: &DeviceId) -> Result<Option<Op>> {
    Ok(super::quorum::ops_of(store, "ModeChange", Some(me))?.pop())
}

/// One `CredUpdate` per credential not yet in the shape of the mode change, re-encrypted under a fresh
/// DEK and wrapped for the new mode. Refuses until the mode change is in force (its ceremony complete):
/// until then the ring has not agreed to the new mode. `open` returns a credential's plaintext payload
/// (vault: phrase and ring secret; quorum: two shares).
pub fn mode_migration(
    store: &LauncherStore,
    me: &DevicePublic,
    open: &dyn Fn(&HostUid) -> Result<Zeroizing<Vec<u8>>>,
) -> Result<Vec<OpBody>> {
    let mc =
        own_mode_change(store, &me.device_id())?.context("this machine proposed no mode change")?;
    ensure!(
        super::ceremony::in_force(store, &mc)?,
        "the mode change is still waiting for its ceremony: nothing is re-encrypted before it completes"
    );
    let OpBody::ModeChange { to, .. } = mc.body else {
        anyhow::bail!("not a mode change")
    };
    let vault = super::quorum::vault_pub(store)?.map(|v| v.device_id());
    let mut bodies = Vec::new();
    for host in credentials(store)? {
        let wraps = store
            .sync_cred_ciphertext(&host)?
            .map(|(_, w)| w)
            .unwrap_or_default();
        let vault_shaped = vault
            .as_ref()
            .is_some_and(|v| wraps.iter().any(|w| w.recipient == *v));
        if (to == CredMode::Quorum) != vault_shaped {
            continue;
        }
        let plain = open(&host).context("opening a credential for the migration")?;
        let payload = CredPayload::from_bytes(&plain)?;
        bodies.push(cred_update_body_for(store, me, &host, &payload, to)?);
    }
    Ok(bodies)
}

/// Checks a `ModeChange` op before it is shown for approval.
pub fn validate_mode_change(store: &LauncherStore, op: &Op) -> Result<()> {
    let OpBody::ModeChange {
        to,
        migrated_on,
        cosigner,
    } = &op.body
    else {
        anyhow::bail!("not a mode change")
    };
    ensure!(
        *migrated_on == op.device_id,
        "a mode change names the machine that migrates: it must be the author"
    );
    if *to == CredMode::Quorum {
        super::quorum::paper_rule(store, super::quorum::live_holder_count(store)?)?;
    }
    if *to == CredMode::Vault {
        let cosigner = cosigner
            .as_ref()
            .context("going back to Vault needs a co-signing holder")?;
        ensure!(
            *cosigner != op.device_id,
            "the co-signer must be another device"
        );
        let ok = super::ceremony::effective_devices(store, None)?
            .into_iter()
            .any(|d| {
                d.id == *cosigner
                    && d.role == DeviceRole::Holder
                    && !matches!(d.status, DeviceStatus::Revoked | DeviceStatus::Compromised)
            });
        ensure!(ok, "the co-signer is not a live holder");
    }
    Ok(())
}
