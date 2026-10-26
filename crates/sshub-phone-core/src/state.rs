//! What the phone keeps besides its key: the devices it enrolled with, names the user gave to other devices, its own op
//! chain (courier cargo and the record of the certificates it issued) and the short-code snapshots it issued. All of it
//! is public or low value data; the secret is held apart. Port of `assets/companion/src/state.mjs`.
//!
//! At rest the whole state is one XChaCha20-Poly1305 blob under `derive_key("sshub/sync/phone/state/v1", secret)`:
//! `"SSHS" || 1 || nonce(24) || ct`, the 5 byte header as associated data. Losing the file loses only the local record.

use std::collections::BTreeMap;

use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305,
};
use rand_core::RngCore;
use serde::{Deserialize, Serialize};
use sshub_sync_proto::crypto::derive_key;
use zeroize::Zeroizing;

use crate::error::{CoreError, Result};

pub const STATE_VERSION: u32 = 1;
const SEAL_CONTEXT: &str = "sshub/sync/phone/state/v1";
const SEAL_MAGIC: &[u8; 4] = b"SSHS";
const SEAL_HEADER: usize = 5;

/// The courier carries the newest ops of the phone's chain with every response; the ring dedups by seq.
pub const COURIER_TAIL: usize = sshub_sync_proto::wire::MAX_COURIER_OPS;

pub const HEAD_PENDING_MESSAGE: &str = "This phone was restored from its 24 words, so it does not know how far its own chain went and must not sign yet. Take its chain from a terminal first, or confirm that this key never signed anything.";

mod hexbytes {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(b: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(b))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        hex::decode(s).map_err(serde::de::Error::custom)
    }
}

/// One op of the phone's own chain as kept: `hash` is `BLAKE3(canonical)` and is the next op's `prev_hash` (and, for a
/// `CertIssue`, the certificate id).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainEntry {
    pub seq: u64,
    pub lamport: u64,
    pub timestamp: i64,
    #[serde(with = "hexbytes")]
    pub hash: Vec<u8>,
    #[serde(with = "hexbytes")]
    pub canonical: Vec<u8>,
    #[serde(with = "hexbytes")]
    pub signature: Vec<u8>,
}

impl ChainEntry {
    pub fn hash32(&self) -> [u8; 32] {
        self.hash.as_slice().try_into().unwrap_or([0; 32])
    }
    pub fn signature64(&self) -> [u8; 64] {
        self.signature.as_slice().try_into().unwrap_or([0; 64])
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Enrolled {
    pub device_id: String,
    pub name: String,
    pub at: i64,
}

/// A terminal this phone enrolled with: name it announced and its Ed25519 public key (hex).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RingDevice {
    pub name: String,
    pub ed25519_pub: String,
    pub enrolled_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotRecord {
    pub issued_at: i64,
    pub expires_at: i64,
    pub mem_kib: u32,
    pub iters: u32,
    pub weak: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeadCandidate {
    pub seq: u64,
    #[serde(with = "hexbytes")]
    pub hash: Vec<u8>,
}

/// A head two lists agreed on, waiting for the user's acceptance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeadProposal {
    pub chain: Vec<ChainEntry>,
    pub epoch: u64,
    pub wrap_epoch: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhoneState {
    pub v: u32,
    /// Version counter: a save only goes through when the stored counter is the one this copy loaded.
    pub rev: u64,
    pub enrolled: Option<Enrolled>,
    pub ring: BTreeMap<String, RingDevice>,
    pub aliases: BTreeMap<String, String>,
    pub chain: Vec<ChainEntry>,
    pub epoch: u64,
    pub lamport: u64,
    pub wrap_epoch: u32,
    /// host uid hex -> the short code issued for it.
    pub snapshots: BTreeMap<String, SnapshotRecord>,
    pub head_pending: bool,
    pub head_candidate: Option<HeadCandidate>,
    pub head_proposal: Option<HeadProposal>,
}

impl Default for PhoneState {
    fn default() -> Self {
        Self {
            v: STATE_VERSION,
            rev: 0,
            enrolled: None,
            ring: BTreeMap::new(),
            aliases: BTreeMap::new(),
            chain: Vec::new(),
            epoch: 0,
            lamport: 0,
            wrap_epoch: 0,
            snapshots: BTreeMap::new(),
            head_pending: false,
            head_candidate: None,
            head_proposal: None,
        }
    }
}

impl PhoneState {
    /// A restored key has the same device id as before, so its next op would reuse a seq the ring already holds (a fork,
    /// which the ring treats as proof of compromise). Until the chain head comes back from a terminal or the user
    /// confirms that the key never signed anything, nothing may be authored.
    pub fn restored() -> Self {
        Self {
            head_pending: true,
            ..Self::default()
        }
    }

    pub fn head(&self) -> Option<&ChainEntry> {
        self.chain.last()
    }

    /// Short-code snapshot still live at `now` for this host (uid hex).
    pub fn live_snapshot(&self, host_hex: &str, now: i64) -> Option<&SnapshotRecord> {
        self.snapshots.get(host_hex).filter(|s| now < s.expires_at)
    }

    pub fn record_snapshot(&mut self, host_hex: &str, rec: SnapshotRecord) {
        self.snapshots.insert(host_hex.to_string(), rec);
    }

    /// The user declares the old code dead (the credential's DEK was reissued, or the blob never reached the peer).
    pub fn invalidate_snapshot(&mut self, host_hex: &str) {
        self.snapshots.remove(host_hex);
    }

    /// The seal of the state: version counter bumped, the whole thing encrypted under the secret.
    pub fn seal(&mut self, secret: &[u8; 32], rng: &mut dyn RngCore) -> Vec<u8> {
        self.rev += 1;
        let mut nonce = [0u8; 24];
        rng.fill_bytes(&mut nonce);
        self.seal_with(secret, &nonce)
    }

    /// Deterministic form (tests).
    pub fn seal_with(&self, secret: &[u8; 32], nonce: &[u8; 24]) -> Vec<u8> {
        let key = Zeroizing::new(derive_key(SEAL_CONTEXT, secret));
        let mut header = Vec::with_capacity(SEAL_HEADER);
        header.extend_from_slice(SEAL_MAGIC);
        header.push(1);
        let json = Zeroizing::new(serde_json::to_vec(self).expect("state serialises"));
        let ct = XChaCha20Poly1305::new((&*key).into())
            .encrypt(
                nonce.into(),
                Payload {
                    msg: &json,
                    aad: &header,
                },
            )
            .expect("XChaCha20-Poly1305 encrypt");
        [header.as_slice(), nonce.as_slice(), ct.as_slice()].concat()
    }

    /// `Err` when the blob is damaged or was sealed under another secret (the caller then starts from a restored
    /// state: the words restore the device and the chain comes back from a terminal).
    pub fn open(blob: &[u8], secret: &[u8; 32]) -> Result<Self> {
        if blob.len() < SEAL_HEADER + 24 + 16 || &blob[..4] != SEAL_MAGIC || blob[4] != 1 {
            return Err(CoreError::bad("the stored state is damaged"));
        }
        let key = Zeroizing::new(derive_key(SEAL_CONTEXT, secret));
        let nonce: [u8; 24] = blob[SEAL_HEADER..SEAL_HEADER + 24].try_into().unwrap();
        let plain = Zeroizing::new(
            XChaCha20Poly1305::new((&*key).into())
                .decrypt(
                    (&nonce).into(),
                    Payload {
                        msg: &blob[SEAL_HEADER + 24..],
                        aad: &blob[..SEAL_HEADER],
                    },
                )
                .map_err(|_| CoreError::bad("the stored state does not open with this key"))?,
        );
        let s: PhoneState = serde_json::from_slice(&plain)
            .map_err(|e| CoreError::bad(format!("the stored state is damaged: {e}")))?;
        if s.v != STATE_VERSION {
            return Err(CoreError::bad("the stored state has an unknown version"));
        }
        Ok(s)
    }

    /// The compare-and-set of the version counter: Ok when the copy this state was loaded from is still the stored one.
    pub fn check_rev(&self, stored_rev: u64) -> Result<()> {
        if stored_rev != self.rev {
            return Err(CoreError::refused(
                "Another copy of this app changed this phone since this one started. Close the other copy and start again.",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_state_round_trips_and_needs_its_secret() {
        let mut s = PhoneState::default();
        s.ring.insert(
            "aaaaaaaaaaaaaaaa".into(),
            RingDevice {
                name: "laptop".into(),
                ed25519_pub: "00".repeat(32),
                enrolled_at: 5,
            },
        );
        s.snapshots.insert(
            "0a".repeat(16),
            SnapshotRecord {
                issued_at: 1,
                expires_at: 2,
                mem_kib: 3,
                iters: 4,
                weak: false,
            },
        );
        let blob = s.seal_with(&[1; 32], &[9; 24]);
        let back = PhoneState::open(&blob, &[1; 32]).unwrap();
        assert_eq!(back, s);
        assert!(PhoneState::open(&blob, &[2; 32]).is_err());
        let mut damaged = blob.clone();
        damaged[40] ^= 1;
        assert!(PhoneState::open(&damaged, &[1; 32]).is_err());
        assert!(PhoneState::open(&blob[..20], &[1; 32]).is_err());
        // the file on disk shows no plaintext field
        assert!(!String::from_utf8_lossy(&blob).contains("laptop"));
    }

    #[test]
    fn version_counter_refuses_a_stale_copy() {
        let mut a = PhoneState::default();
        let mut rng = rand_core::OsRng;
        let _ = a.seal(&[1; 32], &mut rng);
        let _ = a.seal(&[1; 32], &mut rng);
        assert_eq!(a.rev, 2);
        assert!(a.check_rev(2).is_ok());
        assert!(a.check_rev(3).is_err());
        assert!(a.check_rev(1).is_err());
    }

    #[test]
    fn live_snapshot_expires() {
        let mut s = PhoneState::default();
        s.record_snapshot(
            "h",
            SnapshotRecord {
                issued_at: 0,
                expires_at: 100,
                mem_kib: 1,
                iters: 1,
                weak: false,
            },
        );
        assert!(s.live_snapshot("h", 99).is_some());
        assert!(s.live_snapshot("h", 100).is_none());
        s.invalidate_snapshot("h");
        assert!(s.snapshots.is_empty());
    }
}
