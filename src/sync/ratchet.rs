//! The epoch ratchet and its only consumer, the transcript MAC of a sync round.
//!
//! Each pair of devices shares a chain of keys that only they hold: `K(0)` is derived from the
//! authenticated key exchange of their first round (or of an enrollment), and every completed
//! round advances it with randomness both sides contributed, `K(n+1) = ratchet_epoch(K(n),
//! H(nonce_client || nonce_server))`. A round proves possession of the current key with a MAC
//! over the whole transcript. A copy of a disk that misses even one round with a peer cannot
//! compute the next key (it never saw that round's nonces), so that peer sees it as `Behind`.
//!
//! Two phase commit: the server advances only after the client acknowledged, and keeps `next`
//! until then, so a round that dies between the last two frames does not strand an honest device.
//! The residual is exactly that window: a clone one round behind is indistinguishable from the
//! original whose acknowledgement was lost, and is accepted once.

use anyhow::{anyhow, bail, Result};
use rusqlite::{params, OptionalExtension};

use super::crypto::chain_hash;
use super::oplog::DeviceId;
use crate::store::LauncherStore;

const RATCHET_CTX: &[u8] = b"sshub/sync/ratchet/v1";
const MAC_CTX: &[u8] = b"sshub/sync/epoch-mac/v1";
const PAIR_CTX: &str = "sshub/sync/pair/v1";

/// `K(n+1)` from `K(n)` and the nonce material of the round that just finished.
pub fn ratchet_epoch(prev_epoch_key: &[u8; 32], peer_nonce: &[u8; 32]) -> [u8; 32] {
    let mut m = RATCHET_CTX.to_vec();
    m.extend_from_slice(peer_nonce);
    *blake3::keyed_hash(prev_epoch_key, &m).as_bytes()
}

/// MAC of a round transcript under the current epoch key.
pub fn epoch_mac(epoch_key: &[u8; 32], transcript: &[u8]) -> [u8; 32] {
    let mut m = MAC_CTX.to_vec();
    m.extend_from_slice(transcript);
    *blake3::keyed_hash(epoch_key, &m).as_bytes()
}

/// Constant-time comparison of a received MAC.
pub fn epoch_mac_verify(epoch_key: &[u8; 32], transcript: &[u8], mac: &[u8; 32]) -> bool {
    let want = epoch_mac(epoch_key, transcript);
    want.iter().zip(mac).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

/// `K(0)` of a pair from the shared secret of the key exchange and the transcript that authenticated it.
pub fn pair_key_from_exchange(shared_secret: &[u8; 32], transcript_hash: &[u8; 32]) -> [u8; 32] {
    let mut ikm = shared_secret.to_vec();
    ikm.extend_from_slice(transcript_hash);
    super::crypto::derive_key(PAIR_CTX, &ikm)
}

/// What this device holds for one peer.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PairState {
    pub epoch: u64,
    pub key: [u8; 32],
    /// `K(epoch + 1)`, held between verifying the client's finish and receiving its acknowledgement.
    pub next: Option<[u8; 32]>,
}

impl PairState {
    pub fn fresh(key: [u8; 32]) -> Self {
        Self { epoch: 0, key, next: None }
    }

    /// The key to verify a peer that claims `claimed_epoch`, and whether that claim means the peer
    /// did not commit the previous round (so a pending `next` is dropped).
    pub fn key_for_claim(&self, claimed_epoch: u64) -> Option<([u8; 32], bool)> {
        if claimed_epoch == self.epoch {
            Some((self.key, true))
        } else if claimed_epoch == self.epoch + 1 {
            self.next.map(|k| (k, false))
        } else {
            None
        }
    }

    /// Adopt the claimed epoch (promote `next` when the peer committed), returning the key in force.
    pub fn settle(&mut self, claimed_epoch: u64) {
        if claimed_epoch == self.epoch + 1 {
            if let Some(next) = self.next.take() {
                self.epoch += 1;
                self.key = next;
            }
        } else {
            self.next = None;
        }
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut b = self.epoch.to_be_bytes().to_vec();
        b.extend_from_slice(&self.key);
        match self.next {
            Some(n) => {
                b.push(1);
                b.extend_from_slice(&n);
            }
            None => b.push(0),
        }
        b
    }

    fn from_bytes(b: &[u8]) -> Result<Self> {
        if b.len() < 41 {
            bail!("pair state too short");
        }
        let next = match b[40] {
            0 if b.len() == 41 => None,
            1 if b.len() == 73 => Some(b[41..73].try_into().unwrap()),
            _ => bail!("pair state malformed"),
        };
        Ok(Self { epoch: u64::from_be_bytes(b[..8].try_into().unwrap()), key: b[8..40].try_into().unwrap(), next })
    }
}

fn meta_key(peer: &DeviceId) -> String {
    format!("pair:{peer}")
}

/// The pair state for `peer`, if the two devices ever completed a round or an enrollment.
pub fn load_pair(store: &LauncherStore, peer: &DeviceId) -> Result<Option<PairState>> {
    store.with_conn(|conn| {
        let raw: Option<Vec<u8>> = conn
            .query_row("SELECT value FROM sync_meta WHERE key = ?1", params![meta_key(peer)], |r| r.get(0))
            .optional()?;
        raw.map(|b| PairState::from_bytes(&b)).transpose().map_err(|e| anyhow!("pair state of {peer}: {e}"))
    })
}

pub fn save_pair(store: &LauncherStore, peer: &DeviceId, state: &PairState) -> Result<()> {
    store.with_conn(|conn| {
        conn.execute("INSERT OR REPLACE INTO sync_meta (key, value) VALUES (?1, ?2)", params![meta_key(peer), state.to_bytes()])?;
        Ok(())
    })
}

/// Forget the pair (a rehabilitation re-enrolls and writes a new one).
pub fn forget_pair(store: &LauncherStore, peer: &DeviceId) -> Result<()> {
    store.with_conn(|conn| {
        conn.execute("DELETE FROM sync_meta WHERE key = ?1", params![meta_key(peer)])?;
        Ok(())
    })
}

/// A running hash over every frame of a round, in the order they were sent, tagged with the sender's role.
#[derive(Clone)]
pub struct Transcript {
    hasher: blake3::Hasher,
}

impl Default for Transcript {
    fn default() -> Self {
        Self::new()
    }
}

impl Transcript {
    pub fn new() -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"sshub/sync/transcript/v1");
        Self { hasher }
    }

    /// `sender` is the role byte of whoever sent the frame (0 client, 1 server).
    pub fn absorb(&mut self, sender: u8, frame: &[u8]) {
        self.hasher.update(&[sender]);
        self.hasher.update(&(frame.len() as u64).to_be_bytes());
        self.hasher.update(frame);
    }

    pub fn hash(&self) -> [u8; 32] {
        *self.hasher.clone().finalize().as_bytes()
    }

    /// `hash` with a role and purpose label, for signatures and MACs.
    pub fn labelled(&self, label: &str, role: u8) -> Vec<u8> {
        let mut v = label.as_bytes().to_vec();
        v.push(role);
        v.extend_from_slice(&self.hash());
        v
    }
}

/// Nonce material the two hellos contribute to a ratchet step.
pub fn round_nonce(client_nonce: &[u8; 32], server_nonce: &[u8; 32]) -> [u8; 32] {
    let mut m = client_nonce.to_vec();
    m.extend_from_slice(server_nonce);
    chain_hash(&m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ratchet_is_one_way_and_depends_on_the_nonce() {
        let k0 = [1u8; 32];
        let k1 = ratchet_epoch(&k0, &[2; 32]);
        assert_ne!(k1, k0);
        assert_ne!(k1, ratchet_epoch(&k0, &[3; 32]), "another nonce, another key");
        assert_ne!(k1, ratchet_epoch(&[9; 32], &[2; 32]), "another previous key, another key");
        assert_eq!(k1, ratchet_epoch(&k0, &[2; 32]));
    }

    #[test]
    fn the_mac_covers_key_and_transcript() {
        let t = b"transcript";
        let mac = epoch_mac(&[1; 32], t);
        assert!(epoch_mac_verify(&[1; 32], t, &mac));
        assert!(!epoch_mac_verify(&[2; 32], t, &mac));
        assert!(!epoch_mac_verify(&[1; 32], b"transcripu", &mac));
        let mut bad = mac;
        bad[31] ^= 1;
        assert!(!epoch_mac_verify(&[1; 32], t, &bad));
    }

    #[test]
    fn pair_state_round_trips_and_settles() {
        let mut s = PairState { epoch: 4, key: [4; 32], next: Some([5; 32]) };
        assert_eq!(PairState::from_bytes(&s.to_bytes()).unwrap(), s);
        assert_eq!(s.key_for_claim(4).unwrap(), ([4; 32], true));
        assert_eq!(s.key_for_claim(5).unwrap(), ([5; 32], false));
        assert!(s.key_for_claim(3).is_none() && s.key_for_claim(6).is_none());
        let mut a = s.clone();
        a.settle(5);
        assert_eq!((a.epoch, a.key, a.next), (5, [5; 32], None));
        s.settle(4);
        assert_eq!((s.epoch, s.next), (4, None), "the peer did not commit: drop the pending key");
        assert!(PairState::from_bytes(&[0; 10]).is_err());
    }

    #[test]
    fn the_transcript_binds_order_direction_and_content() {
        let mut a = Transcript::new();
        a.absorb(0, b"x");
        a.absorb(1, b"y");
        let mut b = Transcript::new();
        b.absorb(1, b"y");
        b.absorb(0, b"x");
        assert_ne!(a.hash(), b.hash(), "order");
        let mut c = Transcript::new();
        c.absorb(1, b"x");
        c.absorb(1, b"y");
        assert_ne!(a.hash(), c.hash(), "direction");
        let mut d = Transcript::new();
        d.absorb(0, b"x");
        d.absorb(1, b"z");
        assert_ne!(a.hash(), d.hash(), "content");
        assert_ne!(a.labelled("finish", 0), a.labelled("finish", 1));
    }
}
