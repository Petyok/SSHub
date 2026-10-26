//! The SAS enrollment core (see `sync::enroll` in `sshub` for the SSH driver): reveals, commitments, the
//! transcript hash, the SAS words and the confirm messages. Pure functions, no I/O.

use anyhow::{anyhow, bail, ensure, Result};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use x25519_dalek::{EphemeralSecret, PublicKey as XPublic};

use crate::crypto::{chain_hash, sas_words, DeviceIdentity, DevicePublic};
use crate::serde_hex::{b64, hex32, hex64};

const COMMIT_CTX: &[u8] = b"sshub/sync/enroll/commit/v1";
const TRANSCRIPT_CTX: &[u8] = b"sshub/sync/enroll/transcript/v1";
const MAX_NAME_BYTES: usize = 64;

/// Role bytes inside the commitments (not the transport roles).
pub const INITIATOR: u8 = 0;
pub const RESPONDER: u8 = 1;

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
    /// Whether the signer had a human at a screen. It is inside the signature, so the other side learns
    /// it from the key that signs, not from the unauthenticated hello.
    pub screen: bool,
    /// Device-key signature over the enrollment transcript and the decision.
    #[serde(with = "hex64")]
    pub sig: [u8; 64],
}

impl Reveal {
    pub fn bytes(&self) -> Vec<u8> {
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

    /// Strict inverse of [`Reveal::bytes`] (the phone's reveal arrives as these bytes in a kind 7 frame).
    pub fn from_bytes(b: &[u8]) -> Result<Self> {
        let short = || anyhow!("reveal is truncated");
        let mut at = 0usize;
        let mut take = |n: usize| -> Result<&[u8]> {
            let s = b.get(at..at + n).ok_or_else(short)?;
            at += n;
            Ok(s)
        };
        let ed25519_pub: [u8; 32] = take(32)?.try_into().unwrap();
        let x25519_pub: [u8; 32] = take(32)?.try_into().unwrap();
        let ml = u16::from_be_bytes(take(2)?.try_into().unwrap()) as usize;
        let mlkem_pub = take(ml)?.to_vec();
        let eph: [u8; 32] = take(32)?.try_into().unwrap();
        let nonce: [u8; 32] = take(32)?.try_into().unwrap();
        let nl = u16::from_be_bytes(take(2)?.try_into().unwrap()) as usize;
        let name =
            String::from_utf8(take(nl)?.to_vec()).map_err(|_| anyhow!("name is not UTF-8"))?;
        let attended = match take(1)?[0] {
            0 => false,
            1 => true,
            _ => bail!("bad attended flag"),
        };
        ensure!(at == b.len(), "reveal has trailing bytes");
        Ok(Self {
            ed25519_pub,
            x25519_pub,
            mlkem_pub,
            eph,
            nonce,
            name,
            attended,
        })
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

    pub fn shared(self, peer_eph: &[u8; 32]) -> [u8; 32] {
        *self
            .eph
            .diffie_hellman(&XPublic::from(*peer_eph))
            .as_bytes()
    }
}

/// The confirmation between two terminals: the phone's message plus the signer's screen bit.
pub fn terminal_confirm_message(role: u8, ok: bool, th: &[u8; 32], screen: bool) -> Vec<u8> {
    let mut m = confirm_message(role, ok, th);
    m.push(u8::from(screen));
    m
}

pub fn confirm_message(role: u8, ok: bool, th: &[u8; 32]) -> Vec<u8> {
    let mut m = b"sshub/sync/enroll/confirm/v1".to_vec();
    m.push(role);
    m.push(u8::from(ok));
    m.extend_from_slice(th);
    m
}
