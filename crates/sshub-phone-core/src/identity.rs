//! The phone's identity: 32 random bytes (the secret), from which the Ed25519, X25519 and ML-KEM keys are derived with
//! the same BLAKE3 `derive_key` construction as every other identity (`DeviceIdentity::from_seed`, context
//! `sshub/sync/phone/v1`). The 24 BIP39 words ARE the secret, so writing them down is the backup and typing them into
//! any phone (this app or the page) restores the same device. Port of `assets/companion/src/keys.mjs`.
//!
//! Wrap-key generations: generation 0 is the identity's own X25519 and ML-KEM pair; generation `e > 0` (published by a
//! `WrapKeyRotate` op) is derived from the secret and `e` (`derive_xof("sshub/sync/phone/wrap/v1", secret || e u64 BE)`,
//! 96 bytes = X25519 secret, ML-KEM `d`, ML-KEM `z`), so the 24 words still restore every generation. Unlike a key file
//! the phone therefore cannot truly retire an old generation.

use sshub_sync_proto::crypto::{
    bip39_decode, bip39_encode, derive_xof, hybrid_unwrap, DeviceIdentity, DevicePublic,
};
use zeroize::Zeroizing;

use crate::error::{CoreError, Result};

pub const PHONE_CONTEXT: &str = "sshub/sync/phone/v1";
const WRAP_GEN_CONTEXT: &str = "sshub/sync/phone/wrap/v1";
/// A restored phone does not know its wrap generation unless the adopted chain says so: `unwrap` also tries the
/// generations above the one it was built for, up to this bound (derived once, then kept).
pub const WRAP_GEN_MAX: u32 = 32;

fn push_generation(id: &mut DeviceIdentity, secret: &[u8; 32], epoch: u32) {
    let mut ikm = Zeroizing::new([0u8; 40]);
    ikm[..32].copy_from_slice(secret);
    ikm[32..].copy_from_slice(&u64::from(epoch).to_be_bytes());
    let mut m = Zeroizing::new([0u8; 96]);
    derive_xof(WRAP_GEN_CONTEXT, &ikm[..], &mut m[..]);
    id.push_wrap_generation(
        m[..32].try_into().unwrap(),
        m[32..64].try_into().unwrap(),
        m[64..96].try_into().unwrap(),
    );
}

fn identity_at(secret: &[u8; 32], wrap_epoch: u32) -> DeviceIdentity {
    let mut id = DeviceIdentity::from_seed(secret, PHONE_CONTEXT);
    for e in 1..=wrap_epoch {
        push_generation(&mut id, secret, e);
    }
    id
}

/// See [`PhoneIdentity::snapshot`].
#[derive(Clone, Copy, Debug)]
pub struct IdentitySnapshot {
    wrap_epoch: u32,
    seen_wrap_epoch: u32,
}

pub struct PhoneIdentity {
    secret: Zeroizing<[u8; 32]>,
    id: DeviceIdentity,
    device_id: String,
    wrap_epoch: u32,
    seen_wrap_epoch: u32,
    /// Higher generations derived on demand: `(generation, identity whose current wrap keys are that generation)`.
    probes: Vec<(u32, DeviceIdentity)>,
}

impl PhoneIdentity {
    /// The identity at wrap generation `wrap_epoch` (the newest the chain announced).
    pub fn new(secret: &[u8; 32], wrap_epoch: u32) -> Result<Self> {
        if wrap_epoch > WRAP_GEN_MAX {
            return Err(CoreError::bad(format!(
                "wrap key generation {wrap_epoch} is above {WRAP_GEN_MAX}"
            )));
        }
        let id = identity_at(secret, wrap_epoch);
        let device_id = id.public().device_id();
        Ok(Self {
            secret: Zeroizing::new(*secret),
            id,
            device_id,
            wrap_epoch,
            seen_wrap_epoch: wrap_epoch,
            probes: Vec::new(),
        })
    }

    pub fn from_slice(secret: &[u8], wrap_epoch: u32) -> Result<Self> {
        let s: [u8; 32] = secret
            .try_into()
            .map_err(|_| CoreError::bad("the phone secret must be 32 bytes"))?;
        Self::new(&s, wrap_epoch)
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }
    /// Signing key plus the CURRENT wrap keys.
    pub fn public(&self) -> DevicePublic {
        self.id.public()
    }
    pub fn wrap_epoch(&self) -> u32 {
        self.wrap_epoch
    }
    /// Highest generation an unwrap actually needed above `wrap_epoch`: a generation the ring uses, so a rotation
    /// must start above it.
    pub fn seen_wrap_epoch(&self) -> u32 {
        self.seen_wrap_epoch
    }
    pub fn secret(&self) -> &[u8; 32] {
        &self.secret
    }
    pub fn sign(&self, msg: &[u8]) -> [u8; 64] {
        self.id.sign(msg)
    }
    pub fn words(&self) -> String {
        bip39_encode(&self.secret)
    }

    /// What `rollback` needs to put the identity back after a failed transaction.
    pub fn snapshot(&self) -> IdentitySnapshot {
        IdentitySnapshot {
            wrap_epoch: self.wrap_epoch,
            seen_wrap_epoch: self.seen_wrap_epoch,
        }
    }

    /// Back to the wrap generation of `snap` (keys are derived from the secret again, only when the generation moved).
    pub fn rollback(&mut self, snap: &IdentitySnapshot) {
        if self.wrap_epoch != snap.wrap_epoch {
            if let Ok(mut old) = Self::new(&self.secret, snap.wrap_epoch) {
                old.seen_wrap_epoch = snap.seen_wrap_epoch;
                *self = old;
            }
        } else {
            self.seen_wrap_epoch = snap.seen_wrap_epoch;
        }
    }

    /// The same device one wrap generation later (the caller announces it with a `WrapKeyRotate` first).
    pub fn at_generation(&self, wrap_epoch: u32) -> Result<Self> {
        let mut next = Self::new(&self.secret, wrap_epoch)?;
        next.seen_wrap_epoch = next.seen_wrap_epoch.max(self.seen_wrap_epoch);
        Ok(next)
    }

    /// Opens a blob wrapped to any generation this phone holds, then to the generations above up to [`WRAP_GEN_MAX`].
    pub fn unwrap(&mut self, blob: &[u8]) -> Result<Vec<u8>> {
        let mut last = match hybrid_unwrap(&self.id, blob) {
            Ok(p) => return Ok(p),
            Err(e) => e,
        };
        for g in self.wrap_epoch + 1..=WRAP_GEN_MAX {
            if !self.probes.iter().any(|(e, _)| *e == g) {
                let mut probe = DeviceIdentity::from_seed(&self.secret, PHONE_CONTEXT);
                push_generation(&mut probe, &self.secret, g);
                self.probes.push((g, probe));
            }
            let probe = &self.probes.iter().find(|(e, _)| *e == g).unwrap().1;
            match hybrid_unwrap(probe, blob) {
                Ok(p) => {
                    self.seen_wrap_epoch = self.seen_wrap_epoch.max(g);
                    return Ok(p);
                }
                Err(e) => last = e,
            }
        }
        Err(CoreError::bad(format!("{last:#}")))
    }
}

/// 24 backup words to the secret.
pub fn secret_from_words(phrase: &str) -> Result<[u8; 32]> {
    bip39_decode(phrase).map_err(|e| CoreError::bad(format!("{e:#}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sshub_sync_proto::crypto::hybrid_wrap;

    fn secret() -> [u8; 32] {
        [0x42; 32]
    }

    #[test]
    fn device_id_matches_the_terminal_fixture() {
        // assets/companion/test/fixtures/challenges.json: phone.secret_hex = 42 x 32
        let id = PhoneIdentity::new(&secret(), 0).unwrap();
        assert_eq!(id.device_id(), "ee8a1d5cf97016a7");
        assert_eq!(id.words().split(' ').count(), 24);
        assert_eq!(secret_from_words(&id.words()).unwrap(), secret());
    }

    #[test]
    fn generations_change_the_wrap_keys_but_never_the_signing_key() {
        let g0 = PhoneIdentity::new(&secret(), 0).unwrap();
        let g1 = PhoneIdentity::new(&secret(), 1).unwrap();
        let g2 = PhoneIdentity::new(&secret(), 2).unwrap();
        assert_eq!(g0.device_id(), g2.device_id());
        assert_eq!(g0.public().ed25519_pub, g2.public().ed25519_pub);
        assert_ne!(g0.public().x25519_pub, g1.public().x25519_pub);
        assert_ne!(g1.public().mlkem_pub, g2.public().mlkem_pub);
    }

    #[test]
    fn every_generation_unwraps_and_a_lost_counter_is_found_upward() {
        let g0 = PhoneIdentity::new(&secret(), 0).unwrap();
        let g2 = PhoneIdentity::new(&secret(), 2).unwrap();
        let old = hybrid_wrap(&g0.public(), b"old");
        let fresh = hybrid_wrap(&g2.public(), b"fresh");
        let mut g2 = g2;
        assert_eq!(g2.unwrap(&old).unwrap(), b"old");
        assert_eq!(g2.unwrap(&fresh).unwrap(), b"fresh");
        // a phone restored without its counter (generation 0) opens generation 2 and remembers it
        let mut lost = PhoneIdentity::new(&secret(), 0).unwrap();
        assert_eq!(lost.unwrap(&fresh).unwrap(), b"fresh");
        assert_eq!(lost.seen_wrap_epoch(), 2);
        // a generation beyond the bound is not tried
        let far = PhoneIdentity::new(&secret(), WRAP_GEN_MAX).unwrap();
        let blob = hybrid_wrap(&far.public(), b"far");
        let mut near = PhoneIdentity::new(&secret(), 1).unwrap();
        assert!(near.unwrap(&blob).is_ok(), "the bound itself is tried");
        assert!(PhoneIdentity::new(&secret(), WRAP_GEN_MAX + 1).is_err());
    }

    #[test]
    fn a_foreign_blob_does_not_open_and_costs_no_state() {
        let other = PhoneIdentity::new(&[7; 32], 0).unwrap();
        let blob = hybrid_wrap(&other.public(), b"x");
        let mut me = PhoneIdentity::new(&secret(), 0).unwrap();
        assert!(me.unwrap(&blob).is_err());
        assert_eq!(me.seen_wrap_epoch(), 0);
    }
}
