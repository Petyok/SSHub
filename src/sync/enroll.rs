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
use super::limits::CHECKPOINT_CHUNK_BYTES;
use super::oplog::checkpoint::{bundle_from_bytes, bundle_to_bytes, BootstrapTrust};
use super::oplog::{Attendance, DeviceKind, DeviceRole, OpBody, VerifiedBy};
use super::protocol::{b64, hex32, hex64, Session, SyncLink, SyncMsg, MAX_CHECKPOINT_BYTES};
use super::quorum::{meta_get, meta_set};
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
    /// Whether the signer had a human at a screen. It is inside the signature, so the other side learns
    /// it from the key that signs, not from the unauthenticated hello.
    pub screen: bool,
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

    /// Strict inverse of [`Reveal::bytes`] (the phone's reveal arrives as these bytes in a kind 7 frame).
    pub(crate) fn from_bytes(b: &[u8]) -> Result<Self> {
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

    fn shared(self, peer_eph: &[u8; 32]) -> [u8; 32] {
        *self
            .eph
            .diffie_hellman(&XPublic::from(*peer_eph))
            .as_bytes()
    }
}

/// What the human at a screen is shown. `peer_has_screen` is false when the other device cannot show
/// the words to anyone (no tty): then comparing them proves nothing, the other device's identity
/// rests on the SSH host key that was verified, and the prompt must say so plainly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SasPrompt<'a> {
    pub sas: &'a str,
    pub peer_name: &'a str,
    pub peer_has_screen: bool,
}

/// The human decision. Implementations show the prompt and return whether the user admits the
/// device; give up (return `false`) after their own timeout. A side that has no human passes no
/// implementation at all and never confirms as if a human had.
pub trait SasConfirm {
    fn confirm(&mut self, prompt: &SasPrompt<'_>) -> bool;
    /// Progress of the join anchor on the joining side (a screen can show "adopting the ring (N hosts)").
    /// The default ignores it.
    fn progress(&mut self, _p: EnrollProgress) {}
}

/// What the joiner reports while it takes over the ring that existed (contract 3c). `hosts` is the
/// number of live hosts in the bundle's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnrollProgress {
    /// Chunk `chunk` of `total` (both 1-based counts) of the anchor arrived.
    Receiving {
        chunk: u32,
        total: u32,
    },
    Adopting {
        hosts: usize,
    },
    Adopted {
        hosts: usize,
    },
}

fn note(ui: &mut Option<&mut dyn SasConfirm>, p: EnrollProgress) {
    if let Some(ui) = ui.as_deref_mut() {
        ui.progress(p);
    }
}

fn hosts_in(state: &[u8]) -> usize {
    serde_json::from_slice::<serde_json::Value>(state)
        .ok()
        .and_then(|v| {
            v["entities"].as_array().map(|e| {
                e.iter()
                    .filter(|x| x["kind"] == "host" && x["alive"] == true)
                    .count()
            })
        })
        .unwrap_or(0)
}

impl<F: FnMut(&SasPrompt<'_>) -> bool> SasConfirm for F {
    fn confirm(&mut self, prompt: &SasPrompt<'_>) -> bool {
        self(prompt)
    }
}

/// First frame of an enrollment, both ways: ring membership and whether a human can see words.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct EnrollHello {
    /// The ring this device belongs to (the device id of its founder), `None` for a blank device.
    pub ring: Option<String>,
    /// A human at a tty will see the words and decide.
    pub screen: bool,
}

/// Roles follow ring membership, not the direction of the ssh connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnrollRole {
    /// Has (or, being the initiator of two blank devices, creates) the ring and admits the other.
    Inviter,
    /// Joins the inviter's ring.
    Joiner,
}

/// Decides the roles from both hellos. `initiator` is the side that opened the connection.
pub fn decide_role(
    mine: &Option<String>,
    theirs: &Option<String>,
    initiator: bool,
) -> Result<EnrollRole, &'static str> {
    match (mine, theirs) {
        (Some(a), Some(b)) if a != b => {
            Err("merging rings is not supported: both devices already belong to a ring")
        }
        // The same ring: a repeat ceremony (rehabilitation); the initiator invites.
        (Some(_), Some(_)) | (None, None) => Ok(if initiator {
            EnrollRole::Inviter
        } else {
            EnrollRole::Joiner
        }),
        (Some(_), None) => Ok(EnrollRole::Inviter),
        (None, Some(_)) => Ok(EnrollRole::Joiner),
    }
}

/// The ring this device belongs to: the device id of its founder, kept locally.
pub fn ring_id(store: &LauncherStore) -> Result<Option<String>> {
    Ok(meta_get(store, "ring_id")?.and_then(|b| String::from_utf8(b).ok()))
}

fn set_ring_id(store: &LauncherStore, ring: &str) -> Result<()> {
    meta_set(store, "ring_id", ring.as_bytes())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnrollOutcome {
    pub peer: DevicePublic,
    pub peer_name: String,
    pub sas: String,
    pub role: EnrollRole,
    pub verified_by: VerifiedBy,
    pub ring: String,
}

/// The confirmation between two terminals: the phone's message plus the signer's screen bit.
pub(crate) fn terminal_confirm_message(role: u8, ok: bool, th: &[u8; 32], screen: bool) -> Vec<u8> {
    let mut m = confirm_message(role, ok, th);
    m.push(u8::from(screen));
    m
}

pub(crate) fn confirm_message(role: u8, ok: bool, th: &[u8; 32]) -> Vec<u8> {
    let mut m = b"sshub/sync/enroll/confirm/v1".to_vec();
    m.push(role);
    m.push(u8::from(ok));
    m.extend_from_slice(th);
    m
}

/// Inviter side of the join anchor: the bundle in chunks, then its hash.
fn send_anchor<L: SyncLink>(s: &mut Session<'_, L>, bytes: &[u8]) -> Result<()> {
    for (i, chunk) in bytes.chunks(CHECKPOINT_CHUNK_BYTES).enumerate() {
        s.send(&SyncMsg::EnrollAnchor {
            offset: (i * CHECKPOINT_CHUNK_BYTES) as u64,
            total: bytes.len() as u64,
            data: chunk.to_vec(),
        })?;
    }
    s.send(&SyncMsg::EnrollAnchorDone {
        hash: chain_hash(bytes),
    })
}

/// Joiner side: reassemble in order, bounded, and check the hash before anything is parsed.
fn recv_anchor<L: SyncLink>(
    s: &mut Session<'_, L>,
    ui: &mut Option<&mut dyn SasConfirm>,
) -> Result<Vec<u8>> {
    let mut buf: Vec<u8> = Vec::new();
    let mut total: Option<u64> = None;
    loop {
        match s.recv()? {
            SyncMsg::EnrollAnchor {
                offset,
                total: t,
                data,
            } => {
                ensure!(
                    offset == buf.len() as u64
                        && !data.is_empty()
                        && data.len() <= CHECKPOINT_CHUNK_BYTES
                        && t <= MAX_CHECKPOINT_BYTES
                        && total.is_none_or(|p| p == t),
                    "protocol violation: bad join anchor chunk"
                );
                total = Some(t);
                buf.extend_from_slice(&data);
                note(
                    ui,
                    EnrollProgress::Receiving {
                        chunk: (offset / CHECKPOINT_CHUNK_BYTES as u64 + 1) as u32,
                        total: t.div_ceil(CHECKPOINT_CHUNK_BYTES as u64) as u32,
                    },
                );
                ensure!(
                    buf.len() as u64 <= t,
                    "protocol violation: join anchor longer than announced"
                );
            }
            SyncMsg::EnrollAnchorDone { hash } => {
                ensure!(
                    total == Some(buf.len() as u64) && chain_hash(&buf) == hash,
                    "the join anchor is incomplete or damaged"
                );
                return Ok(buf);
            }
            other => bail!("protocol violation: unexpected {other:?} during the join anchor"),
        }
    }
}

fn run<L: SyncLink>(
    link: &mut L,
    store: &LauncherStore,
    id: &DeviceIdentity,
    name: &str,
    attended: bool,
    initiator: bool,
    mut ui: Option<&mut dyn SasConfirm>,
) -> Result<EnrollOutcome> {
    let mut s = Session::new(link, initiator);
    let me = Party::new(id, name, attended);
    let (my_role, peer_role) = if initiator {
        (INITIATOR, RESPONDER)
    } else {
        (RESPONDER, INITIATOR)
    };
    macro_rules! want {
        ($pat:pat => $out:expr) => {
            match s.recv()? {
                $pat => $out,
                other => bail!("protocol violation: unexpected {other:?} during enrollment"),
            }
        };
    }
    // Hellos first: who has a ring decides who invites, whatever the ssh direction.
    let my_ring = ring_id(store)?;
    let my_hello = EnrollHello {
        ring: my_ring.clone(),
        screen: ui.is_some(),
    };
    let peer_hello = if initiator {
        let h = want!(SyncMsg::EnrollHello(h) => h);
        s.send(&SyncMsg::EnrollHello(my_hello.clone()))?;
        h
    } else {
        s.send(&SyncMsg::EnrollHello(my_hello.clone()))?;
        want!(SyncMsg::EnrollHello(h) => h)
    };
    let role = match decide_role(&my_ring, &peer_hello.ring, initiator) {
        Ok(r) => r,
        Err(why) => {
            s.bye(why);
            bail!("{why}");
        }
    };

    let my_commit = me.commit(my_role);
    let peer_commit;
    let peer_reveal;
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
    // A side without a human never confirms as if one had looked: it admits on the strength of the
    // authenticated ssh session and says so in its hello (`screen: false`).
    let ok = match ui.as_deref_mut() {
        Some(ui) => ui.confirm(&SasPrompt {
            sas: &sas,
            peer_name: &peer_reveal.name,
            peer_has_screen: peer_hello.screen,
        }),
        None => true,
    };
    let my_screen = ui.is_some();
    let mine = Confirm {
        ok,
        screen: my_screen,
        sig: id.sign(&terminal_confirm_message(my_role, ok, &th, my_screen)),
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
            &terminal_confirm_message(peer_role, theirs.ok, &th, theirs.screen),
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
    let peer_id = peer_pub.device_id();
    let attendance = if peer_reveal.attended {
        Attendance::Attended
    } else {
        Attendance::Unattended
    };
    // SAS only when BOTH sides had a human at a screen, and the peer's side of that is its signed bit.
    let verified = if ui.is_some() && theirs.screen {
        VerifiedBy::Sas
    } else {
        VerifiedBy::SshHostKey
    };
    let known = store.sync_devices()?.iter().any(|d| d.id == peer_id);
    let ring = match role {
        // The inviter admits the joiner as a mirror (Yellow on the rest of the ring) and, if it had no
        // ring yet, founds one: the ring id is the founder's device id.
        EnrollRole::Inviter => {
            // The anchor carries this device's own row: it must say who it really is.
            if !store
                .sync_devices()?
                .iter()
                .any(|d| d.id == id.public().device_id())
            {
                store.sync_register_device(
                    &id.public(),
                    name,
                    DeviceRole::Holder,
                    if attended {
                        Attendance::Attended
                    } else {
                        Attendance::Unattended
                    },
                    DeviceKind::Terminal,
                    verified,
                )?;
            }
            let ring = match my_ring {
                Some(r) => r,
                None => {
                    let r = id.public().device_id();
                    set_ring_id(store, &r)?;
                    r
                }
            };
            store.sync_register_device(
                &peer_pub,
                &peer_reveal.name,
                DeviceRole::Mirror,
                attendance,
                DeviceKind::Terminal,
                verified,
            )?;
            if !known {
                store
                    .sync_append_local(
                        id,
                        OpBody::DeviceAdd {
                            device: peer_pub.clone(),
                            name: peer_reveal.name.clone(),
                            role: DeviceRole::Mirror,
                            attendance,
                            kind: DeviceKind::Terminal,
                            verified_by: verified,
                        },
                    )
                    .map_err(|e| {
                        anyhow!("device registered locally but the ring could not be told: {e}")
                    })?;
            }
            send_anchor(&mut s, &bundle_to_bytes(&store.sync_join_anchor_build(id)?))?;
            ring
        }
        // The joiner trusts the inviter as a holder and takes over its ring.
        EnrollRole::Joiner => {
            let ring = peer_hello.ring.clone().unwrap_or_else(|| peer_id.clone());
            // The ring that existed is the starting state, not something to review (contract 3c).
            let bundle = bundle_from_bytes(&recv_anchor(&mut s, &mut ui)?)?;
            let hosts = hosts_in(&bundle.state);
            note(&mut ui, EnrollProgress::Adopting { hosts });
            store.sync_join_adopt(
                &bundle,
                &BootstrapTrust::Enrollment {
                    inviter: peer_id.clone(),
                },
            )?;
            // A state carries no label for the inviter's own row; the joiner saw its name in the reveal.
            store.with_conn(|c| {
                c.execute(
                    "UPDATE sync_devices SET name = ?2 WHERE device_id = ?1",
                    rusqlite::params![peer_id, peer_reveal.name],
                )?;
                Ok(())
            })?;
            // Only an adopted ring is ours: a failed adoption leaves the device free to join another.
            set_ring_id(store, &ring)?;
            note(&mut ui, EnrollProgress::Adopted { hosts });
            store.sync_register_device(
                &peer_pub,
                &peer_reveal.name,
                DeviceRole::Holder,
                attendance,
                DeviceKind::Terminal,
                verified,
            )?;
            ring
        }
    };
    // A device knows its own attendance and role from the day it joins a ring: rules such as "an
    // unattended device releases nothing without a certificate" ask the store about this device.
    let my_id = id.public().device_id();
    if !store.sync_devices()?.iter().any(|d| d.id == my_id) {
        store.sync_register_device(
            &id.public(),
            name,
            match role {
                EnrollRole::Inviter => DeviceRole::Holder,
                EnrollRole::Joiner => DeviceRole::Mirror,
            },
            if attended {
                Attendance::Attended
            } else {
                Attendance::Unattended
            },
            DeviceKind::Terminal,
            verified,
        )?;
    }
    save_pair(store, &peer_id, &PairState::fresh(key))?;
    Ok(EnrollOutcome {
        peer: peer_pub,
        peer_name: peer_reveal.name,
        sas,
        role,
        verified_by: verified,
        ring,
    })
}

/// The remote end of an enrollment (`sshub sync-enroll`, run over ssh). It learns its role from the
/// handshake: it invites if it has a ring, joins if it is blank and the caller has one. `ui` is `None`
/// when there is no tty: it then never confirms as a human would.
pub fn run_sync_enroll<L: SyncLink>(
    link: &mut L,
    store: &LauncherStore,
    id: &DeviceIdentity,
    name: &str,
    attended: bool,
    ui: Option<&mut dyn SasConfirm>,
) -> Result<EnrollOutcome> {
    run(link, store, id, name, attended, false, ui)
}

/// The initiating end (`sshub sync-join <peer>`, hosts-tab `t`): opens the connection. Its role is
/// decided the same way; of two blank devices the initiator founds the ring and invites.
pub fn run_sync_join<L: SyncLink>(
    link: &mut L,
    store: &LauncherStore,
    id: &DeviceIdentity,
    name: &str,
    attended: bool,
    ui: Option<&mut dyn SasConfirm>,
) -> Result<EnrollOutcome> {
    run(link, store, id, name, attended, true, ui)
}

#[cfg(test)]
mod tests;
