//! The mixed-mode exemption (contract 4e): in a Quorum ring one machine keeps opening credentials with
//! the master phrase. Its slot is one extra share per credential, wrapped to a key that exists only for
//! someone holding BOTH the phrase and that machine's device key.
//!
//! `slot = from_seed(BLAKE3 derive_key("sshub/sync/exempt/seed/v1", Argon2id(phrase, salt) || bind))`,
//! `bind = BLAKE3 derive_key(.., sign_device_key("sshub/sync/exempt/bind/v1"))`: an Ed25519 signature is
//! deterministic and unforgeable, so `bind` is a keyed function of the device key that never exposes it.

use anyhow::{Context, Result};

use super::crypto::{
    derive_key, exempt_slot_with, Argon2Profile, DeviceIdentity, DevicePublic, Share,
};
use super::oplog::{DeviceId, DeviceStatus, HostUid, OpBody};
use super::quorum::{ops_of, own_shares};
use crate::store::LauncherStore;

const BIND_MSG: &[u8] = b"sshub/sync/exempt/bind/v1";

/// The slot's identity on `id` for `phrase`. Costs one Argon2id stretch (the master profile).
pub fn slot_identity(id: &DeviceIdentity, phrase: &str, profile: Argon2Profile) -> DeviceIdentity {
    let bind = derive_key("sshub/sync/exempt/bind-prf/v1", &id.sign(BIND_MSG));
    let device = id.public().device_id();
    let salt: [u8; 16] = derive_key("sshub/sync/exempt/salt/v1", device.as_bytes())[..16]
        .try_into()
        .expect("16 of 32 bytes");
    exempt_slot_with(phrase, &bind, &salt, profile)
}

/// Enable the exemption for this device (run on the exempt machine: it needs the device key and the
/// phrase). A Red op: it takes the ceremony.
pub fn enable_body(id: &DeviceIdentity, phrase: &str, profile: Argon2Profile) -> OpBody {
    OpBody::ExemptionSet {
        device_id: id.public().device_id(),
        slot: Some(slot_identity(id, phrase, profile).public()),
    }
}

/// Disable it (any attended holder may propose; also a Red op).
pub fn disable_body(device: &DeviceId) -> OpBody {
    OpBody::ExemptionSet {
        device_id: device.clone(),
        slot: None,
    }
}

/// The slots of devices whose newest `ExemptionSet` enables, and that are live holders: the
/// permanent recipients of every credential besides the paper.
pub fn active_slots(store: &LauncherStore) -> Result<Vec<(DeviceId, DevicePublic)>> {
    let mut newest: Vec<((u64, DeviceId), DeviceId, Option<DevicePublic>)> = Vec::new();
    for op in ops_of(store, "ExemptionSet", None)? {
        let OpBody::ExemptionSet { device_id, slot } = &op.body else {
            continue;
        };
        // Only a completed ceremony changes who is wrapped to.
        if !super::ceremony::in_force(store, &op)? {
            continue;
        }
        let key = (op.lamport, op.device_id.clone());
        match newest.iter_mut().find(|(_, d, _)| d == device_id) {
            Some(e) if e.0 < key => *e = (key, device_id.clone(), slot.clone()),
            Some(_) => {}
            None => newest.push((key, device_id.clone(), slot.clone())),
        }
    }
    let devices = super::ceremony::effective_devices(store, None)?;
    let mut out = Vec::new();
    for (_, device, slot) in newest {
        let Some(slot) = slot else { continue };
        let live = devices.iter().any(|d| {
            d.id == device
                && d.role == super::oplog::DeviceRole::Holder
                && !matches!(d.status, DeviceStatus::Revoked | DeviceStatus::Compromised)
        });
        if live {
            out.push((device, slot));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// The slot's share of `host` at `epoch`: with this device's own share it makes two, so the exempt
/// machine opens alone. Fails when the phrase is wrong, the device key is not the exempt device's, or
/// the exemption was not enabled when the credential was made or last reshared.
pub fn slot_share(
    store: &LauncherStore,
    id: &DeviceIdentity,
    phrase: &str,
    host: &HostUid,
    epoch: u64,
    profile: Argon2Profile,
) -> Result<Share> {
    let slot = slot_identity(id, phrase, profile);
    own_shares(store, &slot, host)?
        .into_iter()
        .find(|(e, _)| *e == epoch)
        .map(|(_, s)| s)
        .context("no exemption slot opens with this phrase on this device at that epoch")
}

#[cfg(test)]
mod tests;
