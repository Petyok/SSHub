//! Enrollment of a device by a short authentication string (SAS) with commitments.
//!
//! Both sides commit to everything they will reveal (identity keys, ephemeral key, nonce, name)
//! BEFORE either reveals. The SAS is four words from the hash of both reveals, so a man in the
//! middle must fix the keys it presents to each side before it learns the other leg's reveal:
//! every attempt is an independent 44-bit draw, and grinding after the fact is impossible because
//! a reveal that does not open its commitment is refused. Each side also signs the transcript with
//! its device key, so nobody can enrol a public key it does not hold.
//!
//! ```text
//! R Commit   I Commit   R Reveal   I Reveal   (compare words)   I Confirm   R Confirm
//! ```
//! `R` is the existing device (`sshub sync-enroll`), `I` the joining one. Both persist the other
//! as a device and the pair key `K(0)` derived from the ephemeral exchange.

use anyhow::{anyhow, bail, ensure, Result};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use x25519_dalek::{EphemeralSecret, PublicKey as XPublic};

use super::crypto::{chain_hash, sas_words, verify, DeviceIdentity, DevicePublic};
use super::oplog::{Attendance, DeviceRole, OpBody};
use super::protocol::{b64, hex32, hex64, Session, SyncLink, SyncMsg};
use super::ratchet::{pair_key_from_exchange, save_pair, PairState};
use crate::store::LauncherStore;

const COMMIT_CTX: &[u8] = b"sshub/sync/enroll/commit/v1";
const TRANSCRIPT_CTX: &[u8] = b"sshub/sync/enroll/transcript/v1";
const MAX_NAME_BYTES: usize = 64;

/// Role bytes inside the commitments (not the transport roles).
pub(crate) const INITIATOR: u8 = 0;
pub(crate) const RESPONDER: u8 = 1;

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Reveal {
    #[serde(with = "hex32")]
    pub ed25519_pub: [u8; 32],
    #[serde(with = "hex32")]
    pub x25519_pub: [u8; 32],
    #[serde(with = "b64")]
    pub mlkem_pub: Vec<u8>,
    #[serde(with = "hex32")]
    pub eph: [u8; 32],
    #[serde(with = "hex32")]
    pub nonce: [u8; 32],
    pub name: String,
    pub attended: bool,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Confirm {
    pub ok: bool,
    /// Device-key signature over the enrollment transcript and the decision.
    #[serde(with = "hex64")]
    pub sig: [u8; 64],
}

impl Reveal {
    pub(crate) fn bytes(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(1400);
        b.extend_from_slice(&self.ed25519_pub);
        b.extend_from_slice(&self.x25519_pub);
        b.extend_from_slice(&(self.mlkem_pub.len() as u16).to_be_bytes());
        b.extend_from_slice(&self.mlkem_pub);
        b.extend_from_slice(&self.eph);
        b.extend_from_slice(&self.nonce);
        b.extend_from_slice(&(self.name.len() as u16).to_be_bytes());
        b.extend_from_slice(self.name.as_bytes());
        b.push(u8::from(self.attended));
        b
    }

    pub fn public(&self) -> DevicePublic {
        DevicePublic {
            ed25519_pub: self.ed25519_pub,
            x25519_pub: self.x25519_pub,
            mlkem_pub: self.mlkem_pub.clone(),
        }
    }
}

/// Hash that commits `role`'s side to a reveal.
pub fn commit_of(role: u8, r: &Reveal) -> [u8; 32] {
    let mut m = COMMIT_CTX.to_vec();
    m.push(role);
    m.extend_from_slice(&r.bytes());
    chain_hash(&m)
}

/// A reveal that opens its commitment and is structurally sound.
pub fn check_reveal(role: u8, commit: &[u8; 32], r: &Reveal) -> Result<()> {
    ensure!(
        commit_of(role, r) == *commit,
        "reveal does not open the commitment"
    );
    ensure!(
        r.name.len() <= MAX_NAME_BYTES && !r.name.chars().any(char::is_control),
        "bad device name"
    );
    r.public().validate()
}

/// Hash both reveals feed into the SAS and the pair key.
pub fn transcript_hash(initiator: &Reveal, responder: &Reveal) -> [u8; 32] {
    let mut m = TRANSCRIPT_CTX.to_vec();
    m.extend_from_slice(&initiator.bytes());
    m.extend_from_slice(&responder.bytes());
    chain_hash(&m)
}

pub fn sas_of(initiator: &Reveal, responder: &Reveal) -> String {
    sas_words(&transcript_hash(initiator, responder))
}

/// One side's ephemeral state.
pub struct Party {
    eph: EphemeralSecret,
    pub reveal: Reveal,
}

impl Party {
    pub fn new(identity: &DeviceIdentity, name: &str, attended: bool) -> Self {
        let eph = EphemeralSecret::random_from_rng(OsRng);
        let mut nonce = [0u8; 32];
        rand_core::RngCore::fill_bytes(&mut OsRng, &mut nonce);
        let p = identity.public();
        let reveal = Reveal {
            ed25519_pub: p.ed25519_pub,
            x25519_pub: p.x25519_pub,
            mlkem_pub: p.mlkem_pub,
            eph: XPublic::from(&eph).to_bytes(),
            nonce,
            name: name.chars().take(MAX_NAME_BYTES).collect(),
            attended,
        };
        Self { eph, reveal }
    }

    pub fn commit(&self, role: u8) -> [u8; 32] {
        commit_of(role, &self.reveal)
    }

    fn shared(self, peer_eph: &[u8; 32]) -> [u8; 32] {
        *self
            .eph
            .diffie_hellman(&XPublic::from(*peer_eph))
            .as_bytes()
    }
}

/// The human decision. Implementations show `sas` and the peer's claimed name and return whether
/// the words match; give up (return `false`) after their own timeout.
pub trait SasConfirm {
    fn confirm(&mut self, sas: &str, peer_name: &str) -> bool;
}

impl<F: FnMut(&str, &str) -> bool> SasConfirm for F {
    fn confirm(&mut self, sas: &str, peer_name: &str) -> bool {
        self(sas, peer_name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnrollOutcome {
    pub peer: DevicePublic,
    pub peer_name: String,
    pub sas: String,
}

pub(crate) fn confirm_message(role: u8, ok: bool, th: &[u8; 32]) -> Vec<u8> {
    let mut m = b"sshub/sync/enroll/confirm/v1".to_vec();
    m.push(role);
    m.push(u8::from(ok));
    m.extend_from_slice(th);
    m
}

fn run<L: SyncLink>(
    link: &mut L,
    store: &LauncherStore,
    id: &DeviceIdentity,
    name: &str,
    attended: bool,
    initiator: bool,
    ui: &mut dyn SasConfirm,
) -> Result<EnrollOutcome> {
    let mut s = Session::new(link, initiator);
    let me = Party::new(id, name, attended);
    let (my_role, peer_role) = if initiator {
        (INITIATOR, RESPONDER)
    } else {
        (RESPONDER, INITIATOR)
    };
    let my_commit = me.commit(my_role);
    let peer_commit;
    let peer_reveal;
    macro_rules! want {
        ($pat:pat => $out:expr) => {
            match s.recv()? {
                $pat => $out,
                other => bail!("protocol violation: unexpected {other:?} during enrollment"),
            }
        };
    }
    if initiator {
        peer_commit = want!(SyncMsg::EnrollCommit { commit } => commit);
        s.send(&SyncMsg::EnrollCommit { commit: my_commit })?;
        peer_reveal = want!(SyncMsg::EnrollReveal(r) => r);
        check_reveal(peer_role, &peer_commit, &peer_reveal)?;
        s.send(&SyncMsg::EnrollReveal(me.reveal.clone()))?;
    } else {
        s.send(&SyncMsg::EnrollCommit { commit: my_commit })?;
        peer_commit = want!(SyncMsg::EnrollCommit { commit } => commit);
        s.send(&SyncMsg::EnrollReveal(me.reveal.clone()))?;
        peer_reveal = want!(SyncMsg::EnrollReveal(r) => r);
        check_reveal(peer_role, &peer_commit, &peer_reveal)?;
    }
    ensure!(
        peer_reveal.ed25519_pub != me.reveal.ed25519_pub,
        "the peer presents this device's own key"
    );
    let (ri, rr) = if initiator {
        (&me.reveal, &peer_reveal)
    } else {
        (&peer_reveal, &me.reveal)
    };
    let th = transcript_hash(ri, rr);
    let sas = sas_words(&th);
    let ok = ui.confirm(&sas, &peer_reveal.name);
    let mine = Confirm {
        ok,
        sig: id.sign(&confirm_message(my_role, ok, &th)),
    };
    let theirs = if initiator {
        s.send(&SyncMsg::EnrollConfirm(mine))?;
        want!(SyncMsg::EnrollConfirm(c) => c)
    } else {
        let c = want!(SyncMsg::EnrollConfirm(c) => c);
        s.send(&SyncMsg::EnrollConfirm(mine))?;
        c
    };
    ensure!(
        verify(
            &peer_reveal.ed25519_pub,
            &confirm_message(peer_role, theirs.ok, &th),
            &theirs.sig
        ),
        "bad confirmation signature"
    );
    if !ok {
        bail!("enrollment declined: the words did not match");
    }
    ensure!(
        theirs.ok,
        "the other device declined: the words did not match"
    );

    let key = pair_key_from_exchange(&me.shared(&peer_reveal.eph), &th);
    let peer_pub = peer_reveal.public();
    // The joining device is a mirror until the ring approves more; the existing device is a holder.
    let peer_device_role = if initiator {
        DeviceRole::Holder
    } else {
        DeviceRole::Mirror
    };
    let attendance = if peer_reveal.attended {
        Attendance::Attended
    } else {
        Attendance::Unattended
    };
    store.sync_register_device(&peer_pub, &peer_reveal.name, peer_device_role, attendance)?;
    save_pair(store, &peer_pub.device_id(), &PairState::fresh(key))?;
    if !initiator {
        store
            .sync_append_local(
                id,
                OpBody::DeviceAdd {
                    device: peer_pub.clone(),
                    name: peer_reveal.name.clone(),
                    role: DeviceRole::Mirror,
                    attendance,
                },
            )
            .map_err(|e| {
                anyhow!("device registered locally but the ring could not be told: {e}")
            })?;
    }
    Ok(EnrollOutcome {
        peer: peer_pub,
        peer_name: peer_reveal.name,
        sas,
    })
}

/// Existing device (`sshub sync-enroll`): waits for a joining device to connect.
pub fn run_sync_enroll<L: SyncLink>(
    link: &mut L,
    store: &LauncherStore,
    id: &DeviceIdentity,
    name: &str,
    attended: bool,
    ui: &mut dyn SasConfirm,
) -> Result<EnrollOutcome> {
    run(link, store, id, name, attended, false, ui)
}

/// Joining device: connects to an existing one and verifies the words.
pub fn run_sync_join<L: SyncLink>(
    link: &mut L,
    store: &LauncherStore,
    id: &DeviceIdentity,
    name: &str,
    attended: bool,
    ui: &mut dyn SasConfirm,
) -> Result<EnrollOutcome> {
    run(link, store, id, name, attended, true, ui)
}

#[cfg(test)]
mod tests;
