//! PoC B model: a ring of holders, k = 2 credentials, and the reshare protocol in two shapes.
//!
//! * V1 wraps every sub-share and every adopted share with the hybrid wrap (one ML-KEM
//!   ciphertext, about 1.1 KiB, per share).
//! * V2 sets up one session key per (contributor, recipient) pair and one storage key per
//!   holder per epoch, wraps those once with the hybrid wrap, and moves the per-credential
//!   payload under XChaCha20-Poly1305 with the context as associated data.
//!
//! Only the public `sshub::sync::crypto` API is used, so the model exercises exactly what
//! the production code exposes.

use anyhow::{anyhow, Result};
use rand_core::{OsRng, RngCore};
use sshub::sync::crypto::*;

pub struct Holder {
    pub id: DeviceIdentity,
    pub index: u8,
}

impl Holder {
    pub fn new(index: u8) -> Self {
        Self {
            id: DeviceIdentity::generate(),
            index,
        }
    }
    pub fn public(&self) -> DevicePublic {
        self.id.public()
    }
}

/// A credential as the log carries it: ciphertext plus one wrapped share per holder.
#[derive(Clone)]
pub struct Credential {
    pub host_uid: [u8; 16],
    pub ciphertext: Vec<u8>,
    pub wrapped: Vec<(u8, Vec<u8>)>,
}

pub fn random16() -> [u8; 16] {
    let mut b = [0u8; 16];
    OsRng.fill_bytes(&mut b);
    b
}

fn random32() -> [u8; 32] {
    let mut b = [0u8; 32];
    OsRng.fill_bytes(&mut b);
    b
}

/// Encrypts a fresh credential and wraps a k = 2 share to every holder. Returns the DEK too (tests only).
pub fn create_credential(holders: &[&Holder], plaintext: &[u8]) -> (Credential, [u8; 32]) {
    let dek = dek_generate();
    let indices: Vec<u8> = holders.iter().map(|h| h.index).collect();
    let shares = shamir_split_at(&dek, 2, &indices);
    let wrapped = shares
        .iter()
        .zip(holders)
        .map(|(s, h)| (h.index, hybrid_wrap(&h.public(), &s.bytes)))
        .collect();
    (
        Credential {
            host_uid: random16(),
            ciphertext: dek_seal(&dek, plaintext),
            wrapped,
        },
        dek,
    )
}

pub fn open_share(h: &Holder, c: &Credential) -> Result<Share> {
    let blob = &c
        .wrapped
        .iter()
        .find(|(i, _)| *i == h.index)
        .ok_or_else(|| anyhow!("no wrapped share for holder {}", h.index))?
        .1;
    Ok(Share {
        index: h.index,
        bytes: hybrid_unwrap(&h.id, blob)?,
    })
}

/// What a requester does: two shares, combine in memory, decrypt.
pub fn unlock(c: &Credential, a: &Holder, b: &Holder) -> Result<Vec<u8>> {
    let dek = shamir_combine(&[open_share(a, c)?, open_share(b, c)?])?;
    dek_open(&dek, &c.ciphertext)
}

// ---------------------------------------------------------------- V1: hybrid wrap per share

pub struct ContribOp1 {
    pub from: u8,
    pub host_uid: [u8; 16],
    pub subs: Vec<(u8, Vec<u8>)>,
    pub commitments: Vec<Vec<u8>>,
}

pub struct AdoptOp1 {
    pub holder: u8,
    pub host_uid: [u8; 16],
    pub wrapped_share: Vec<u8>,
    pub contributors: Vec<u8>,
}

pub struct Reshared1 {
    pub contribs: Vec<ContribOp1>,
    pub adopts: Vec<AdoptOp1>,
    pub new_credential: Credential,
}

/// Per-role work time, for the "max per device" figure.
#[derive(Default, Clone)]
pub struct Timing {
    pub per_holder_ns: std::collections::BTreeMap<u8, u128>,
}

impl Timing {
    pub fn add(&mut self, holder: u8, ns: u128) {
        *self.per_holder_ns.entry(holder).or_default() += ns;
    }
    pub fn max_ns(&self) -> u128 {
        self.per_holder_ns.values().copied().max().unwrap_or(0)
    }
}

pub fn reshare_v1(
    c: &Credential,
    contributors: [&Holder; 2],
    new_holders: &[&Holder],
    timing: &mut Timing,
) -> Result<Reshared1> {
    let new_indices: Vec<u8> = new_holders.iter().map(|h| h.index).collect();
    let mut contribs = Vec::new();
    let mut plain_contribs = Vec::new();
    for h in contributors {
        let t = std::time::Instant::now();
        let mine = open_share(h, c)?;
        let rc = reshare_contribute(&mine, &new_indices, 2);
        let subs = rc
            .sub_shares
            .iter()
            .map(|(i, bytes)| {
                let r = new_holders
                    .iter()
                    .find(|n| n.index == *i)
                    .expect("recipient");
                (*i, hybrid_wrap(&r.public(), bytes))
            })
            .collect();
        timing.add(h.index, t.elapsed().as_nanos());
        contribs.push(ContribOp1 {
            from: h.index,
            host_uid: c.host_uid,
            subs,
            commitments: rc.commitments.clone(),
        });
        plain_contribs.push(rc);
    }
    let mut adopts = Vec::new();
    let mut wrapped = Vec::new();
    for r in new_holders {
        let t = std::time::Instant::now();
        let mut mine = Vec::new();
        for (op, plain) in contribs.iter().zip(&plain_contribs) {
            let blob = &op
                .subs
                .iter()
                .find(|(i, _)| *i == r.index)
                .ok_or_else(|| anyhow!("no sub-share for {}", r.index))?
                .1;
            let bytes = hybrid_unwrap(&r.id, blob)?;
            let pos = plain
                .sub_shares
                .iter()
                .position(|(i, _)| *i == r.index)
                .unwrap();
            let one = ReshareContribution {
                from: op.from,
                sub_shares: vec![(r.index, bytes)],
                commitments: vec![op.commitments[pos].clone()],
            };
            reshare_verify(&one, r.index)?;
            mine.push(one);
        }
        let share = reshare_combine(r.index, &mine)?;
        let blob = hybrid_wrap(&r.public(), &share.bytes);
        timing.add(r.index, t.elapsed().as_nanos());
        wrapped.push((r.index, blob.clone()));
        adopts.push(AdoptOp1 {
            holder: r.index,
            host_uid: c.host_uid,
            wrapped_share: blob,
            contributors: contributors.iter().map(|h| h.index).collect(),
        });
    }
    Ok(Reshared1 {
        contribs,
        adopts,
        new_credential: Credential {
            host_uid: c.host_uid,
            ciphertext: c.ciphertext.clone(),
            wrapped,
        },
    })
}

// ---------------------------------------------------------------- V2: epoch envelopes

pub struct EpochSetup {
    /// (from, to, hybrid_wrap(to.pub, session key)) - one op per pair per epoch.
    pub session_blobs: Vec<(u8, u8, Vec<u8>)>,
    /// (holder, hybrid_wrap(holder.pub, storage key)) - one op per holder per epoch.
    pub storage_blobs: Vec<(u8, Vec<u8>)>,
    sessions: std::collections::HashMap<(u8, u8), [u8; 32]>,
    storage: std::collections::HashMap<u8, [u8; 32]>,
}

pub fn setup_epoch(
    contributors: [&Holder; 2],
    new_holders: &[&Holder],
    timing: &mut Timing,
) -> Result<EpochSetup> {
    let mut e = EpochSetup {
        session_blobs: vec![],
        storage_blobs: vec![],
        sessions: Default::default(),
        storage: Default::default(),
    };
    for c in contributors {
        for r in new_holders {
            let t = std::time::Instant::now();
            let sk = random32();
            let blob = hybrid_wrap(&r.public(), &sk);
            timing.add(c.index, t.elapsed().as_nanos());
            let t = std::time::Instant::now();
            let got = hybrid_unwrap(&r.id, &blob)?; // the recipient learns the session key
            timing.add(r.index, t.elapsed().as_nanos());
            assert_eq!(got.as_slice(), sk.as_slice());
            e.sessions.insert((c.index, r.index), sk);
            e.session_blobs.push((c.index, r.index, blob));
        }
    }
    for r in new_holders {
        let t = std::time::Instant::now();
        let sk = random32();
        e.storage_blobs
            .push((r.index, hybrid_wrap(&r.public(), &sk)));
        e.storage.insert(r.index, sk);
        timing.add(r.index, t.elapsed().as_nanos());
    }
    Ok(e)
}

pub struct ContribOp2 {
    pub from: u8,
    pub host_uid: [u8; 16],
    pub subs: Vec<(u8, Vec<u8>)>,
    pub commitments: Vec<Vec<u8>>,
}

pub struct AdoptOp2 {
    pub holder: u8,
    pub host_uid: [u8; 16],
    pub sealed_share: Vec<u8>,
}

pub fn reshare_v2(
    c: &Credential,
    epoch: &EpochSetup,
    epoch_no: u64,
    contributors: [&Holder; 2],
    new_holders: &[&Holder],
    timing: &mut Timing,
) -> Result<(Vec<ContribOp2>, Vec<AdoptOp2>)> {
    let new_indices: Vec<u8> = new_holders.iter().map(|h| h.index).collect();
    let aad =
        |from: u8, to: u8| [c.host_uid.as_slice(), &epoch_no.to_be_bytes(), &[from, to]].concat();
    let mut contribs = Vec::new();
    for h in contributors {
        let t = std::time::Instant::now();
        let mine = open_share(h, c)?;
        let rc = reshare_contribute(&mine, &new_indices, 2);
        let subs = rc
            .sub_shares
            .iter()
            .map(|(i, bytes)| {
                (
                    *i,
                    aead_seal(&epoch.sessions[&(h.index, *i)], &aad(h.index, *i), bytes),
                )
            })
            .collect();
        timing.add(h.index, t.elapsed().as_nanos());
        contribs.push((
            ContribOp2 {
                from: h.index,
                host_uid: c.host_uid,
                subs,
                commitments: rc.commitments.clone(),
            },
            rc,
        ));
    }
    let mut adopts = Vec::new();
    for r in new_holders {
        let t = std::time::Instant::now();
        let mut mine = Vec::new();
        for (op, plain) in &contribs {
            let sealed = &op.subs.iter().find(|(i, _)| *i == r.index).unwrap().1;
            let bytes = aead_open(
                &epoch.sessions[&(op.from, r.index)],
                &aad(op.from, r.index),
                sealed,
            )?;
            let pos = plain
                .sub_shares
                .iter()
                .position(|(i, _)| *i == r.index)
                .unwrap();
            mine.push(ReshareContribution {
                from: op.from,
                sub_shares: vec![(r.index, bytes)],
                commitments: vec![op.commitments[pos].clone()],
            });
        }
        let share = reshare_combine(r.index, &mine)?;
        let sealed = aead_seal(
            &epoch.storage[&r.index],
            &[c.host_uid.as_slice(), &epoch_no.to_be_bytes(), &[r.index]].concat(),
            &share.bytes,
        );
        timing.add(r.index, t.elapsed().as_nanos());
        adopts.push(AdoptOp2 {
            holder: r.index,
            host_uid: c.host_uid,
            sealed_share: sealed,
        });
    }
    Ok((contribs.into_iter().map(|(o, _)| o).collect(), adopts))
}

/// A holder reads its own epoch share back (what a later contribution or unlock does).
pub fn open_v2_share(
    h: &Holder,
    epoch: &EpochSetup,
    epoch_no: u64,
    host_uid: &[u8; 16],
    sealed: &[u8],
) -> Result<Share> {
    let sk = epoch
        .storage
        .get(&h.index)
        .ok_or_else(|| anyhow!("no storage key"))?;
    let bytes = aead_open(
        sk,
        &[host_uid.as_slice(), &epoch_no.to_be_bytes(), &[h.index]].concat(),
        sealed,
    )?;
    Ok(Share {
        index: h.index,
        bytes,
    })
}

// ---------------------------------------------------------------- op size accounting

#[derive(Clone, Copy, Debug)]
pub enum Enc {
    Array,
    Hex,
    Base64,
}

pub fn enc(bytes: &[u8], e: Enc) -> serde_json::Value {
    use base64::Engine;
    match e {
        Enc::Array => serde_json::Value::Array(bytes.iter().map(|b| (*b).into()).collect()),
        Enc::Hex => hex::encode(bytes).into(),
        Enc::Base64 => base64::engine::general_purpose::STANDARD
            .encode(bytes)
            .into(),
    }
}

/// A signed op: the canonical JSON text of everything but the signature, the signature, the signer key.
pub struct SignedOp {
    pub unsigned: Vec<u8>,
    pub sig: [u8; 64],
    pub signer: [u8; 32],
}

impl SignedOp {
    /// Bytes on the wire and in storage: the text plus `,"signature":"<128 hex>"`.
    pub fn wire_len(&self) -> usize {
        self.unsigned.len() + 128 + ",\"signature\":\"\"".len()
    }

    /// What a receiving peer does with an op: parse, hash, verify the signature.
    pub fn check(&self) -> bool {
        let parsed: serde_json::Value = serde_json::from_slice(&self.unsigned).expect("valid json");
        let _ = blake3::hash(&self.unsigned);
        parsed.is_object() && verify(&self.signer, &self.unsigned, &self.sig)
    }
}

pub fn make_op(signer: &DeviceIdentity, seq: u64, epoch: u64, body: serde_json::Value) -> SignedOp {
    let public = signer.public();
    let unsigned = serde_json::to_vec(&serde_json::json!({
        "device_id": public.device_id(), "seq": seq, "lamport": seq, "prev_hash": hex::encode([0u8; 32]),
        "timestamp": 1_893_452_400i64, "epoch": epoch, "alg": { "sig": 1, "wrap": 1 }, "body": body,
    }))
    .unwrap();
    let sig = signer.sign(&unsigned);
    SignedOp {
        unsigned,
        sig,
        signer: public.ed25519_pub,
    }
}
