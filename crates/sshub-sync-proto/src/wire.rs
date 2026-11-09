//! `SYNC_WIRE_V1`: the byte contract between the terminal and the phone page. Frames are
//! hand-encoded (no serialisation library) so the page can implement them in a few lines.
//!
//! ```text
//! frame     = "SWR1" || version u16 BE || kind u8 || body
//! challenge = kind 1: nonce(32) || expiry i64 BE || eph_x25519_pub(32) || request_id u64 BE ||
//!                     op_len u32 BE || op_canonical(op_len) || op_signature(64) ||
//!                     material_len u32 BE || phone_material(material_len)
//! material  = kind u8 (0 hybrid, 1 envelope) || epoch u64 BE || share_len u16 BE || share_blob ||
//!             storage_len u16 BE || storage_blob
//! response  = kind 2: request_id u64 BE || len u64 BE || ciphertext(len)
//! ```
//!
//! The op travels as its canonical bytes plus its signature, exactly as the terminal hashed
//! and signed them: the phone verifies those bytes and renders its description from them,
//! it never re-serialises. The phone stores no credential keys, so everything it needs to
//! unwrap ITS share of the credential rides along as `phone_material`: either the hybrid
//! wrapped share of the original credential op (kind 0) or, after a reshare, its envelope share
//! plus its storage key blob for that epoch (kind 1). The channel binding covers the WHOLE
//! canonical op and the hash of the phone material, so the approval is for exactly what was shown.

use anyhow::{bail, Result};

use super::crypto::chain_hash;
use super::fountain::fountain_encode;
use super::limits::{MAX_FRAME_BYTES, QR_PARITY_PERCENT};

pub const SYNC_WIRE_V1: u16 = 1;
pub const WIRE_MAGIC: [u8; 4] = *b"SWR1";
pub const KIND_CHALLENGE: u8 = 1;
pub const KIND_SHARE_RESPONSE: u8 = 2;
/// Domain separation tag of the channel binding.
pub const DOMAIN_TAG_CHALLENGE: &[u8] = b"sshub/sync/challenge/v1";
const HEADER: usize = 7;

/// Which shape of wrapped share the phone is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ShareKind {
    /// `hybrid_wrap(phone pub, host_uid || index || share)` from the credential op.
    Hybrid = 0,
    /// `aead_seal(storage key, aad = host_uid || epoch || index, index || share)` plus the
    /// `hybrid_wrap(phone pub, storage key)` blob of that epoch.
    Envelope = 1,
}

/// Everything the phone needs to recover its own share for one credential.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhoneMaterial {
    pub kind: ShareKind,
    pub epoch: u64,
    pub share_blob: Vec<u8>,
    /// Empty for [`ShareKind::Hybrid`].
    pub storage_blob: Vec<u8>,
}

impl PhoneMaterial {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(13 + self.share_blob.len() + self.storage_blob.len());
        out.push(self.kind as u8);
        out.extend_from_slice(&self.epoch.to_be_bytes());
        out.extend_from_slice(
            &u16::try_from(self.share_blob.len())
                .expect("share blob fits u16")
                .to_be_bytes(),
        );
        out.extend_from_slice(&self.share_blob);
        out.extend_from_slice(
            &u16::try_from(self.storage_blob.len())
                .expect("storage blob fits u16")
                .to_be_bytes(),
        );
        out.extend_from_slice(&self.storage_blob);
        out
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() < 13 {
            bail!("phone material too short");
        }
        let kind = match b[0] {
            0 => ShareKind::Hybrid,
            1 => ShareKind::Envelope,
            v => bail!("unknown phone material kind {v}"),
        };
        let epoch = u64::from_be_bytes(b[1..9].try_into().unwrap());
        let share_len = u16::from_be_bytes(b[9..11].try_into().unwrap()) as usize;
        let storage_at = 11 + share_len;
        if b.len() < storage_at + 2 {
            bail!("phone material truncated");
        }
        let storage_len =
            u16::from_be_bytes(b[storage_at..storage_at + 2].try_into().unwrap()) as usize;
        if b.len() != storage_at + 2 + storage_len {
            bail!("phone material length does not match its fields");
        }
        if kind == ShareKind::Hybrid && storage_len != 0 {
            bail!("a hybrid share carries no storage key blob");
        }
        if kind == ShareKind::Envelope && storage_len == 0 {
            bail!("an envelope share needs its storage key blob");
        }
        Ok(Self {
            kind,
            epoch,
            share_blob: b[11..storage_at].to_vec(),
            storage_blob: b[storage_at + 2..].to_vec(),
        })
    }

    pub fn hash(&self) -> [u8; 32] {
        chain_hash(&self.encode())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChallengeWire {
    pub op_canonical: Vec<u8>,
    pub op_signature: [u8; 64],
    pub nonce: [u8; 32],
    pub expiry: i64,
    pub eph_x25519_pub: [u8; 32],
    pub request_id: u64,
    pub phone_material: PhoneMaterial,
}

/// The exact bytes an approving holder signs:
/// `tag || proto_version u16 LE || canonical_op || nonce || expiry i64 LE || eph || request_id u64 LE ||
/// BLAKE3(phone material)`.
pub fn binding_preimage(
    op_canonical: &[u8],
    nonce: &[u8; 32],
    expiry: i64,
    eph: &[u8; 32],
    request_id: u64,
    material_hash: &[u8; 32],
) -> Vec<u8> {
    let mut b = Vec::with_capacity(
        DOMAIN_TAG_CHALLENGE.len() + 2 + op_canonical.len() + 32 + 8 + 32 + 8 + 32,
    );
    b.extend_from_slice(DOMAIN_TAG_CHALLENGE);
    b.extend_from_slice(&SYNC_WIRE_V1.to_le_bytes());
    b.extend_from_slice(op_canonical);
    b.extend_from_slice(nonce);
    b.extend_from_slice(&expiry.to_le_bytes());
    b.extend_from_slice(eph);
    b.extend_from_slice(&request_id.to_le_bytes());
    b.extend_from_slice(material_hash);
    b
}

/// `BLAKE3(binding_preimage(..))`.
pub fn binding_hash(
    op_canonical: &[u8],
    nonce: &[u8; 32],
    expiry: i64,
    eph: &[u8; 32],
    request_id: u64,
    material_hash: &[u8; 32],
) -> [u8; 32] {
    chain_hash(&binding_preimage(
        op_canonical,
        nonce,
        expiry,
        eph,
        request_id,
        material_hash,
    ))
}

impl ChallengeWire {
    /// Animated QR frames for this challenge: the encoded frame cut into `frame_bytes` pieces
    /// (the QR byte capacity at level M, see `fountain::frame_bytes_for_area`) with the default parity.
    pub fn to_qr_frames(&self, frame_bytes: usize) -> Result<Vec<Vec<u8>>> {
        fountain_encode(&self.encode(), frame_bytes, QR_PARITY_PERCENT)
    }

    pub fn binding_hash(&self) -> [u8; 32] {
        binding_hash(
            &self.op_canonical,
            &self.nonce,
            self.expiry,
            &self.eph_x25519_pub,
            self.request_id,
            &self.phone_material.hash(),
        )
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = frame_header(KIND_CHALLENGE);
        out.extend_from_slice(&self.nonce);
        out.extend_from_slice(&self.expiry.to_be_bytes());
        out.extend_from_slice(&self.eph_x25519_pub);
        out.extend_from_slice(&self.request_id.to_be_bytes());
        out.extend_from_slice(
            &u32::try_from(self.op_canonical.len())
                .expect("op fits a frame")
                .to_be_bytes(),
        );
        out.extend_from_slice(&self.op_canonical);
        out.extend_from_slice(&self.op_signature);
        let material = self.phone_material.encode();
        out.extend_from_slice(&(material.len() as u32).to_be_bytes());
        out.extend_from_slice(&material);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let body = unframe(bytes, KIND_CHALLENGE)?;
        if body.len() < 32 + 8 + 32 + 8 + 4 + 64 + 4 {
            bail!("challenge too short");
        }
        let op_len = u32::from_be_bytes(body[80..84].try_into().unwrap()) as usize;
        if op_len > MAX_FRAME_BYTES {
            bail!("challenge op of {op_len} bytes exceeds the frame cap");
        }
        let material_at = 84 + op_len + 64;
        if body.len() < material_at + 4 {
            bail!("challenge truncated before the phone material");
        }
        let material_len =
            u32::from_be_bytes(body[material_at..material_at + 4].try_into().unwrap()) as usize;
        if material_len > MAX_FRAME_BYTES || body.len() != material_at + 4 + material_len {
            bail!("challenge length does not match its op and material lengths");
        }
        Ok(Self {
            nonce: body[..32].try_into().unwrap(),
            expiry: i64::from_be_bytes(body[32..40].try_into().unwrap()),
            eph_x25519_pub: body[40..72].try_into().unwrap(),
            request_id: u64::from_be_bytes(body[72..80].try_into().unwrap()),
            op_canonical: body[84..84 + op_len].to_vec(),
            op_signature: body[84 + op_len..material_at].try_into().unwrap(),
            phone_material: PhoneMaterial::decode(&body[material_at + 4..])?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShareResponseWire {
    pub request_id: u64,
    pub ciphertext: Vec<u8>,
}

impl ShareResponseWire {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = frame_header(KIND_SHARE_RESPONSE);
        out.extend_from_slice(&self.request_id.to_be_bytes());
        out.extend_from_slice(&(self.ciphertext.len() as u64).to_be_bytes());
        out.extend_from_slice(&self.ciphertext);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let body = unframe(bytes, KIND_SHARE_RESPONSE)?;
        if body.len() < 16 {
            bail!("share response too short");
        }
        let len = u64::from_be_bytes(body[8..16].try_into().unwrap());
        if len > MAX_FRAME_BYTES as u64 || body.len() as u64 != 16 + len {
            bail!("share response length does not match its length field");
        }
        Ok(Self {
            request_id: u64::from_be_bytes(body[..8].try_into().unwrap()),
            ciphertext: body[16..].to_vec(),
        })
    }
}

/// Frame kinds that only the phone page uses (contract C16). Layouts:
///
/// ```text
/// recovery notice (kind 3, terminal to phone) = op_len u32 BE || op_canonical || op_signature(64)
/// phone ops       (kind 5, phone to terminal) = n u8 || n * (op_len u32 BE || op_canonical || signature(64))
/// enroll commit   (kind 6) = role u8 || commit(32)
/// enroll reveal   (kind 7) = role u8 || reveal bytes (the layout of `enroll.rs` `Reveal::bytes`, opaque here)
/// enroll confirm  (kind 8) = role u8 || ok u8 || signature(64)
/// ```
pub const KIND_RECOVERY_NOTICE: u8 = 3;
pub const KIND_PHONE_OPS: u8 = 5;
pub const KIND_ENROLL_COMMIT: u8 = 6;
pub const KIND_ENROLL_REVEAL: u8 = 7;
pub const KIND_ENROLL_CONFIRM: u8 = 8;

/// A phone chain moves a few small ops per courier trip: more than this, or a bigger op, is not a
/// phone and is refused before it is parsed further.
pub const MAX_COURIER_OPS: usize = 8;
pub const MAX_COURIER_OP_BYTES: usize = 256 * 1024;

/// An op together with its signature, as the phone carries it.
pub type SignedOpBytes = (Vec<u8>, [u8; 64]);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryNoticeWire {
    pub op_canonical: Vec<u8>,
    pub op_signature: [u8; 64],
}

impl RecoveryNoticeWire {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = frame_header(KIND_RECOVERY_NOTICE);
        out.extend_from_slice(&(self.op_canonical.len() as u32).to_be_bytes());
        out.extend_from_slice(&self.op_canonical);
        out.extend_from_slice(&self.op_signature);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let body = unframe(bytes, KIND_RECOVERY_NOTICE)?;
        if body.len() < 4 + 64 {
            bail!("recovery notice too short");
        }
        let len = u32::from_be_bytes(body[..4].try_into().unwrap()) as usize;
        if len > MAX_FRAME_BYTES || body.len() != 4 + len + 64 {
            bail!("recovery notice length does not match its fields");
        }
        Ok(Self {
            op_canonical: body[4..4 + len].to_vec(),
            op_signature: body[4 + len..].try_into().unwrap(),
        })
    }
}

pub fn encode_op_list(ops: &[SignedOpBytes]) -> Vec<u8> {
    assert!(
        ops.len() <= MAX_COURIER_OPS,
        "at most {MAX_COURIER_OPS} ops per list"
    );
    let mut out = vec![ops.len() as u8];
    for (op, sig) in ops {
        out.extend_from_slice(&(op.len() as u32).to_be_bytes());
        out.extend_from_slice(op);
        out.extend_from_slice(sig);
    }
    out
}

/// Parses a list starting at `body[0]`; returns the ops and the number of bytes used.
pub fn decode_op_list(body: &[u8]) -> Result<(Vec<SignedOpBytes>, usize)> {
    if body.len() > MAX_FRAME_BYTES {
        bail!("op list exceeds the frame cap");
    }
    let Some((&n, _)) = body.split_first() else {
        bail!("op list truncated");
    };
    if n as usize > MAX_COURIER_OPS {
        bail!("op list of {n} ops exceeds the cap of {MAX_COURIER_OPS}");
    }
    let mut at = 1;
    let mut ops = Vec::with_capacity(n as usize);
    for _ in 0..n {
        if body.len() < at + 4 {
            bail!("op list truncated");
        }
        let len = u32::from_be_bytes(body[at..at + 4].try_into().unwrap()) as usize;
        at += 4;
        if len > MAX_COURIER_OP_BYTES || body.len() < at + len + 64 {
            bail!("op list entry is oversize or truncated");
        }
        ops.push((
            body[at..at + len].to_vec(),
            body[at + len..at + len + 64].try_into().unwrap(),
        ));
        at += len + 64;
    }
    Ok((ops, at))
}

/// Tag of the optional trailing sender block of a kind 5 frame.
pub const PHONE_OPS_SENDER_TAG: u8 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhoneOpsWire {
    pub ops: Vec<SignedOpBytes>,
    /// The terminal that rendered the list for the phone (its device id, raw 8 bytes), as an optional trailing block
    /// `0x01 || id(8)`. UNSIGNED: a phone restored from its words has no ring to check a signature against, so the id
    /// only tells two lists apart (an honest terminal that is behind, or one animation scanned twice). It is not a proof
    /// of who sent the list. Phone-to-terminal exports carry none.
    pub sender: Option<[u8; 8]>,
    /// Phase B: what the sender knows about the ring's terminals, as an optional trailing block after the sender block
    /// (`0x02 || count u8 || newest_op_ts i64 BE`). UNSIGNED and informational. Blocks come in tag order.
    pub terminals: Option<TerminalsBlock>,
}

impl PhoneOpsWire {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = frame_header(KIND_PHONE_OPS);
        out.extend_from_slice(&encode_op_list(&self.ops));
        if let Some(id) = &self.sender {
            out.push(PHONE_OPS_SENDER_TAG);
            out.extend_from_slice(id);
        }
        if let Some(t) = &self.terminals {
            out.push(PHONE_OPS_TERMINALS_TAG);
            out.push(t.count);
            out.extend_from_slice(&t.newest_op_ts.to_be_bytes());
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let body = unframe(bytes, KIND_PHONE_OPS)?;
        let (ops, used) = decode_op_list(body)?;
        let mut rest = &body[used..];
        let mut sender = None;
        if let [PHONE_OPS_SENDER_TAG, tail @ ..] = rest {
            let Some((id, tail)) = tail.split_first_chunk::<8>() else {
                bail!("phone ops frame: the sender block is truncated");
            };
            sender = Some(*id);
            rest = tail;
        }
        let mut terminals = None;
        if let [PHONE_OPS_TERMINALS_TAG, tail @ ..] = rest {
            let Some((b, tail)) = tail.split_first_chunk::<9>() else {
                bail!("phone ops frame: the terminals block is truncated");
            };
            terminals = Some(TerminalsBlock {
                count: b[0],
                newest_op_ts: i64::from_be_bytes(b[1..].try_into().unwrap()),
            });
            rest = tail;
        }
        if !rest.is_empty() {
            bail!(
                "phone ops frame has trailing bytes, an unknown block or blocks out of tag order"
            );
        }
        Ok(Self {
            ops,
            sender,
            terminals,
        })
    }
}

/// One enrollment step between the terminal and the phone (QR in both directions).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnrollFrame {
    Commit { role: u8, commit: [u8; 32] },
    Reveal { role: u8, reveal: Vec<u8> },
    Confirm { role: u8, ok: bool, sig: [u8; 64] },
}

impl EnrollFrame {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Self::Commit { role, commit } => {
                let mut out = frame_header(KIND_ENROLL_COMMIT);
                out.push(*role);
                out.extend_from_slice(commit);
                out
            }
            Self::Reveal { role, reveal } => {
                let mut out = frame_header(KIND_ENROLL_REVEAL);
                out.push(*role);
                out.extend_from_slice(reveal);
                out
            }
            Self::Confirm { role, ok, sig } => {
                let mut out = frame_header(KIND_ENROLL_CONFIRM);
                out.push(*role);
                out.push(u8::from(*ok));
                out.extend_from_slice(sig);
                out
            }
        }
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < HEADER + 1 {
            bail!("enrollment frame too short");
        }
        let kind = bytes[6];
        let body = unframe(bytes, kind)?;
        let role = body[0];
        match kind {
            KIND_ENROLL_COMMIT => {
                if body.len() != 33 {
                    bail!("enroll commit must be 33 bytes");
                }
                Ok(Self::Commit {
                    role,
                    commit: body[1..].try_into().unwrap(),
                })
            }
            KIND_ENROLL_REVEAL => Ok(Self::Reveal {
                role,
                reveal: body[1..].to_vec(),
            }),
            KIND_ENROLL_CONFIRM => {
                if body.len() != 66 || body[1] > 1 {
                    bail!("enroll confirm must be 66 bytes with a boolean");
                }
                Ok(Self::Confirm {
                    role,
                    ok: body[1] == 1,
                    sig: body[2..].try_into().unwrap(),
                })
            }
            k => bail!("wire kind {k} is not an enrollment frame"),
        }
    }
}

/// Kind 9, terminal to phone: what the user chose for a trust session. The phone shows it, and on
/// approval builds and signs the `CertIssue` op itself (it is the author); nothing here is a
/// signature. Layout: `device_id(8) || scope tag u8 [0: group uid(16) | 1: n u16 BE || n * uid(16)]
/// || expiry i64 BE || max_new_hosts_per_day u32 BE || epoch_lo u64 BE || epoch_hi u64 BE`.
pub const KIND_CERT_REQUEST: u8 = 9;
/// A QR-sized list: more hosts than this belong in a group.
pub const MAX_CERT_REQUEST_HOSTS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertRequestWire {
    /// Raw 8 bytes of the delegate's device id (the id is their lowercase hex).
    pub device_id: [u8; 8],
    pub scope: super::oplog::CertScope,
    pub expiry: i64,
    pub max_new_hosts_per_day: u32,
    pub epoch_lo: u64,
    pub epoch_hi: u64,
}

impl CertRequestWire {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = frame_header(KIND_CERT_REQUEST);
        out.extend_from_slice(&self.device_id);
        match &self.scope {
            super::oplog::CertScope::Group(g) => {
                out.push(0);
                out.extend_from_slice(g);
            }
            super::oplog::CertScope::Hosts(h) => {
                out.push(1);
                out.extend_from_slice(&(h.len() as u16).to_be_bytes());
                for uid in h {
                    out.extend_from_slice(uid);
                }
            }
        }
        out.extend_from_slice(&self.expiry.to_be_bytes());
        out.extend_from_slice(&self.max_new_hosts_per_day.to_be_bytes());
        out.extend_from_slice(&self.epoch_lo.to_be_bytes());
        out.extend_from_slice(&self.epoch_hi.to_be_bytes());
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        use super::oplog::CertScope;
        let body = unframe(bytes, KIND_CERT_REQUEST)?;
        let short = || anyhow::anyhow!("certificate request is truncated");
        let device_id: [u8; 8] = body.get(..8).ok_or_else(short)?.try_into().unwrap();
        let mut at = 9;
        let scope = match *body.get(8).ok_or_else(short)? {
            0 => {
                let g: [u8; 16] = body.get(at..at + 16).ok_or_else(short)?.try_into().unwrap();
                at += 16;
                CertScope::Group(g)
            }
            1 => {
                let n =
                    u16::from_be_bytes(body.get(at..at + 2).ok_or_else(short)?.try_into().unwrap())
                        as usize;
                at += 2;
                if n == 0 || n > MAX_CERT_REQUEST_HOSTS {
                    bail!(
                        "a certificate request lists 1..={MAX_CERT_REQUEST_HOSTS} hosts, got {n}"
                    );
                }
                let mut hosts = Vec::with_capacity(n);
                for _ in 0..n {
                    hosts.push(body.get(at..at + 16).ok_or_else(short)?.try_into().unwrap());
                    at += 16;
                }
                CertScope::Hosts(hosts)
            }
            t => bail!("unknown certificate scope tag {t}"),
        };
        let rest = body.get(at..).ok_or_else(short)?;
        if rest.len() != 8 + 4 + 8 + 8 {
            bail!(
                "certificate request has {} trailing or missing bytes",
                rest.len() as i64 - 28
            );
        }
        Ok(Self {
            device_id,
            scope,
            expiry: i64::from_be_bytes(rest[..8].try_into().unwrap()),
            max_new_hosts_per_day: u32::from_be_bytes(rest[8..12].try_into().unwrap()),
            epoch_lo: u64::from_be_bytes(rest[12..20].try_into().unwrap()),
            epoch_hi: u64::from_be_bytes(rest[20..28].try_into().unwrap()),
        })
    }

    /// The op body the phone will sign.
    pub fn to_body(&self) -> super::oplog::OpBody {
        super::oplog::OpBody::CertIssue {
            device_id: hex::encode(self.device_id),
            scope: self.scope.clone(),
            expiry: self.expiry,
            max_new_hosts_per_day: self.max_new_hosts_per_day,
            epoch_lo: self.epoch_lo,
            epoch_hi: self.epoch_hi,
        }
    }
}

pub const KIND_CERT_EXTEND_REQUEST: u8 = 10;

/// What the terminal shows the phone to extend a trust-session certificate's daily budget (R3b):
/// `cert_id(32) || extra_hosts u32 BE`. The phone builds and signs `CertExtend` from it unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertExtendRequestWire {
    pub cert_id: [u8; 32],
    pub extra_hosts: u32,
}

impl CertExtendRequestWire {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = frame_header(KIND_CERT_EXTEND_REQUEST);
        out.extend_from_slice(&self.cert_id);
        out.extend_from_slice(&self.extra_hosts.to_be_bytes());
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let body = unframe(bytes, KIND_CERT_EXTEND_REQUEST)?;
        if body.len() != 36 {
            bail!(
                "a certificate extension request is 36 bytes, got {}",
                body.len()
            );
        }
        let extra_hosts = u32::from_be_bytes(body[32..].try_into().unwrap());
        if extra_hosts == 0 || extra_hosts > super::limits::CERT_NEW_HOSTS_PER_DAY {
            bail!(
                "an extension is 1 to {} hosts",
                super::limits::CERT_NEW_HOSTS_PER_DAY
            );
        }
        Ok(Self {
            cert_id: body[..32].try_into().unwrap(),
            extra_hosts,
        })
    }

    pub fn to_body(&self) -> super::oplog::OpBody {
        super::oplog::OpBody::CertExtend {
            cert_id: self.cert_id,
            extra_hosts: self.extra_hosts,
        }
    }
}

/// Phase B (contract 4j): the request a terminal shows the phone to renew a trust-session certificate.
pub const KIND_CERT_RENEW_REQUEST: u8 = 11;
/// Phase B (contract 4j): the request a terminal shows the OLD phone to hand the ring over to a new phone.
pub const KIND_PHONE_HANDOVER_REQUEST: u8 = 12;
/// Phase B (contract 4j): tag of the optional `terminals` block that follows the sender block of a kind 5 frame.
pub const PHONE_OPS_TERMINALS_TAG: u8 = 2;
/// Most evidence entries a renew request carries.
pub const MAX_RENEW_EVIDENCE: usize = 32;

/// What one enforcing holder terminal (never the delegate) knows: its newest op seq and that op's author stamp.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadEvidence {
    pub terminal: [u8; 8],
    pub head_seq: u64,
    pub head_ts: i64,
}

/// Kind 11 (contract 4j), type only: `cert_id(32) || expiry i64 BE || n u8 || n x (terminal(8) || head_seq u64 BE || head_ts i64 BE)`,
/// `n <= MAX_RENEW_EVIDENCE`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertRenewRequestWire {
    pub cert_id: [u8; 32],
    pub expiry: i64,
    pub evidence: Vec<HeadEvidence>,
}

/// Kind 12 (contract 4j, 4l b): `ed25519(32) || x25519(32) || mlkem_len u16 BE || mlkem || name_len u8 || name utf8 || terminal(8)`,
/// then for the admission frame shown to the NEW phone `0x01 || op_hash(32)` (the ingested `PhoneHandover` op). `op_hash` is `None` for the request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhoneHandoverRequestWire {
    pub new_phone: super::crypto::DevicePublic,
    pub name: String,
    pub terminal: [u8; 8],
    pub op_hash: Option<[u8; 32]>,
}

impl CertRenewRequestWire {
    pub fn encode(&self) -> Vec<u8> {
        assert!(
            self.evidence.len() <= MAX_RENEW_EVIDENCE,
            "too much evidence"
        );
        let mut out = frame_header(KIND_CERT_RENEW_REQUEST);
        out.extend_from_slice(&self.cert_id);
        out.extend_from_slice(&self.expiry.to_be_bytes());
        out.push(self.evidence.len() as u8);
        for e in &self.evidence {
            out.extend_from_slice(&e.terminal);
            out.extend_from_slice(&e.head_seq.to_be_bytes());
            out.extend_from_slice(&e.head_ts.to_be_bytes());
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let body = unframe(bytes, KIND_CERT_RENEW_REQUEST)?;
        if body.len() < 41 {
            bail!("a certificate renewal request is truncated");
        }
        let n = body[40] as usize;
        if n > MAX_RENEW_EVIDENCE {
            bail!("a renewal request carries at most {MAX_RENEW_EVIDENCE} evidence entries");
        }
        if body.len() != 41 + n * 24 {
            bail!("a certificate renewal request does not match its evidence count");
        }
        let evidence = body[41..]
            .as_chunks::<24>()
            .0
            .iter()
            .map(|c| HeadEvidence {
                terminal: c[..8].try_into().unwrap(),
                head_seq: u64::from_be_bytes(c[8..16].try_into().unwrap()),
                head_ts: i64::from_be_bytes(c[16..].try_into().unwrap()),
            })
            .collect();
        Ok(Self {
            cert_id: body[..32].try_into().unwrap(),
            expiry: i64::from_be_bytes(body[32..40].try_into().unwrap()),
            evidence,
        })
    }

    pub fn to_body(&self) -> super::oplog::OpBody {
        super::oplog::OpBody::CertRenew {
            cert_id: self.cert_id,
            expiry: self.expiry,
        }
    }
}

impl PhoneHandoverRequestWire {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = frame_header(KIND_PHONE_HANDOVER_REQUEST);
        out.extend_from_slice(&self.new_phone.ed25519_pub);
        out.extend_from_slice(&self.new_phone.x25519_pub);
        out.extend_from_slice(
            &u16::try_from(self.new_phone.mlkem_pub.len())
                .expect("ML-KEM key fits u16")
                .to_be_bytes(),
        );
        out.extend_from_slice(&self.new_phone.mlkem_pub);
        out.push(u8::try_from(self.name.len()).expect("name fits u8"));
        out.extend_from_slice(self.name.as_bytes());
        out.extend_from_slice(&self.terminal);
        if let Some(h) = &self.op_hash {
            out.push(1);
            out.extend_from_slice(h);
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let body = unframe(bytes, KIND_PHONE_HANDOVER_REQUEST)?;
        let short = || anyhow::anyhow!("a phone handover request is truncated");
        if body.len() < 66 {
            return Err(short());
        }
        let mlkem_len = u16::from_be_bytes(body[64..66].try_into().unwrap()) as usize;
        let name_at = 66 + mlkem_len;
        let name_len = *body.get(name_at).ok_or_else(short)? as usize;
        let term_at = name_at + 1 + name_len;
        let rest = body.get(term_at + 8..).ok_or_else(short)?;
        let op_hash = match rest {
            [] => None,
            [1, h @ ..] if h.len() == 32 => Some(h.try_into().unwrap()),
            _ => bail!("a phone handover request does not match its lengths"),
        };
        Ok(Self {
            new_phone: super::crypto::DevicePublic {
                ed25519_pub: body[..32].try_into().unwrap(),
                x25519_pub: body[32..64].try_into().unwrap(),
                mlkem_pub: body[66..name_at].to_vec(),
            },
            name: String::from_utf8(body[name_at + 1..term_at].to_vec())
                .map_err(|_| anyhow::anyhow!("a handover name is not UTF-8"))?,
            terminal: body[term_at..term_at + 8].try_into().unwrap(),
            op_hash,
        })
    }
}

/// The optional `terminals` block of a kind 5 frame (contract 4j): `0x02 || count u8 || newest_op_ts i64 BE`.
/// UNSIGNED and informational: the `terminals` field of `PhoneOpsWire`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalsBlock {
    pub count: u8,
    pub newest_op_ts: i64,
}

fn frame_header(kind: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER + 64);
    out.extend_from_slice(&WIRE_MAGIC);
    out.extend_from_slice(&SYNC_WIRE_V1.to_be_bytes());
    out.push(kind);
    out
}

fn unframe(bytes: &[u8], want: u8) -> Result<&[u8]> {
    if bytes.len() < HEADER {
        bail!("wire frame too short: {} bytes", bytes.len());
    }
    if bytes[..4] != WIRE_MAGIC {
        bail!("bad wire magic");
    }
    let version = u16::from_be_bytes([bytes[4], bytes[5]]);
    if version != SYNC_WIRE_V1 {
        bail!("unsupported wire version {version}");
    }
    if bytes[6] != want {
        bail!("unexpected wire kind {} (wanted {want})", bytes[6]);
    }
    Ok(&bytes[HEADER..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ChallengeWire {
        ChallengeWire {
            op_canonical: br#"{"device_id":"aabbccdd11223344","seq":7}"#.to_vec(),
            op_signature: [4; 64],
            nonce: [2; 32],
            expiry: 1_893_456_000,
            eph_x25519_pub: [3; 32],
            request_id: 0xDEAD_BEEF,
            phone_material: PhoneMaterial {
                kind: ShareKind::Hybrid,
                epoch: 3,
                share_blob: vec![0xAB; 40],
                storage_blob: vec![],
            },
        }
    }

    fn envelope() -> PhoneMaterial {
        PhoneMaterial {
            kind: ShareKind::Envelope,
            epoch: 5,
            share_blob: vec![1; 73],
            storage_blob: vec![2; 60],
        }
    }

    #[test]
    fn the_kind_5_sender_block_is_optional_and_a_truncated_one_is_refused() {
        let ops = vec![(b"a".to_vec(), [1; 64])];
        let plain = PhoneOpsWire {
            ops: ops.clone(),
            sender: None,
            terminals: None,
        };
        let with = PhoneOpsWire {
            ops,
            sender: Some([7; 8]),
            terminals: None,
        };
        // without the block (every frame written before it, and every phone export): decodes, no sender
        assert_eq!(PhoneOpsWire::decode(&plain.encode()).unwrap().sender, None);
        // with it: round trip, and the block is the last nine bytes
        let bytes = with.encode();
        assert_eq!(PhoneOpsWire::decode(&bytes).unwrap(), with);
        assert_eq!(&bytes[bytes.len() - 9..], &[1, 7, 7, 7, 7, 7, 7, 7, 7]);
        assert_eq!(
            bytes[..bytes.len() - 9],
            plain.encode()[..],
            "the list itself is unchanged"
        );
        // a truncated block is refused, whatever its length
        for cut in 1..9 {
            assert!(
                PhoneOpsWire::decode(&bytes[..bytes.len() - cut]).is_err(),
                "cut {cut}"
            );
        }
        // a longer block, an unknown tag
        let mut long = bytes.clone();
        long.push(0);
        assert!(PhoneOpsWire::decode(&long).is_err());
        let mut tag = bytes.clone();
        let at = tag.len() - 9;
        tag[at] = 2;
        assert!(PhoneOpsWire::decode(&tag).is_err());
    }

    #[test]
    fn the_kind_5_terminals_block_is_optional_ordered_and_strict() {
        let ops = vec![(b"a".to_vec(), [1; 64])];
        let plain = PhoneOpsWire {
            ops: ops.clone(),
            sender: None,
            terminals: None,
        };
        let t = TerminalsBlock {
            count: 3,
            newest_op_ts: -2,
        };
        let only = PhoneOpsWire {
            terminals: Some(t),
            ..plain.clone()
        };
        let both = PhoneOpsWire {
            sender: Some([7; 8]),
            ..only.clone()
        };
        // frames without the block decode as before
        assert_eq!(PhoneOpsWire::decode(&plain.encode()).unwrap(), plain);
        for w in [&only, &both] {
            let bytes = w.encode();
            assert_eq!(&PhoneOpsWire::decode(&bytes).unwrap(), w);
            let n = bytes.len();
            assert_eq!(&bytes[n - 10..n - 8], &[PHONE_OPS_TERMINALS_TAG, 3]);
            assert_eq!(&bytes[n - 8..], &(-2i64).to_be_bytes());
            // truncated at every length, trailing byte
            for cut in 1..10 {
                assert!(
                    PhoneOpsWire::decode(&bytes[..n - cut]).is_err(),
                    "cut {cut}"
                );
            }
            let mut long = bytes.clone();
            long.push(0);
            assert!(PhoneOpsWire::decode(&long).is_err());
            // an unknown tag after the known blocks
            let mut unknown = bytes.clone();
            unknown.extend_from_slice(&[3, 0]);
            assert!(PhoneOpsWire::decode(&unknown).is_err());
        }
        // blocks out of tag order: terminals before sender
        let mut swapped = plain.encode();
        swapped.extend_from_slice(&[PHONE_OPS_TERMINALS_TAG, 3]);
        swapped.extend_from_slice(&(-2i64).to_be_bytes());
        swapped.push(PHONE_OPS_SENDER_TAG);
        swapped.extend_from_slice(&[7; 8]);
        assert!(PhoneOpsWire::decode(&swapped).is_err());
        // two sender blocks, two terminals blocks
        let mut twice = both.encode();
        twice.extend_from_slice(&both.encode()[both.encode().len() - 10..]);
        assert!(PhoneOpsWire::decode(&twice).is_err());
    }

    fn evidence(i: u8) -> HeadEvidence {
        HeadEvidence {
            terminal: [i; 8],
            head_seq: u64::from(i) << 40,
            head_ts: -i64::from(i),
        }
    }

    #[test]
    fn kind_11_carries_zero_to_32_evidence_entries_and_refuses_33() {
        for n in [0u8, 1, 32] {
            let req = CertRenewRequestWire {
                cert_id: [5; 32],
                expiry: 1_800_000_000,
                evidence: (0..n).map(evidence).collect(),
            };
            let bytes = req.encode();
            assert_eq!(bytes.len(), HEADER + 41 + 24 * n as usize);
            assert_eq!(CertRenewRequestWire::decode(&bytes).unwrap(), req);
            assert_eq!(
                req.to_body(),
                crate::oplog::OpBody::CertRenew {
                    cert_id: [5; 32],
                    expiry: 1_800_000_000
                }
            );
            // trailing byte and truncation
            let mut long = bytes.clone();
            long.push(0);
            assert!(CertRenewRequestWire::decode(&long).is_err());
            assert!(CertRenewRequestWire::decode(&bytes[..bytes.len() - 1]).is_err());
        }
        // 33 entries on the wire (built by hand: the encoder refuses to write them)
        let mut over = CertRenewRequestWire {
            cert_id: [5; 32],
            expiry: 1,
            evidence: (0..32).map(evidence).collect(),
        }
        .encode();
        over[HEADER + 40] = 33;
        over.extend_from_slice(&[0; 24]);
        assert!(CertRenewRequestWire::decode(&over).is_err());
        assert!(CertRenewRequestWire::decode(&frame_header(KIND_CERT_RENEW_REQUEST)).is_err());
        assert!(CertRenewRequestWire::decode(&frame_header(KIND_CERT_EXTEND_REQUEST)).is_err());
    }

    #[test]
    fn kind_12_round_trips_with_the_longest_name_and_refuses_bad_lengths() {
        for name in ["", "p", &"\u{1F4F1}".repeat(63), &"n".repeat(255)] {
            let req = PhoneHandoverRequestWire {
                new_phone: crate::crypto::DevicePublic {
                    ed25519_pub: [1; 32],
                    x25519_pub: [2; 32],
                    mlkem_pub: vec![3; 1184],
                },
                name: name.into(),
                terminal: [9; 8],
                op_hash: None,
            };
            let bytes = req.encode();
            assert_eq!(PhoneHandoverRequestWire::decode(&bytes).unwrap(), req);
            assert_eq!(bytes.len(), HEADER + 66 + 1184 + 1 + name.len() + 8);
            let mut long = bytes.clone();
            long.push(0);
            assert!(PhoneHandoverRequestWire::decode(&long).is_err());
            for cut in [1, 8, 9] {
                assert!(PhoneHandoverRequestWire::decode(&bytes[..bytes.len() - cut]).is_err());
            }
            // a name that is not UTF-8
            if !name.is_empty() {
                let mut bad = bytes.clone();
                bad[HEADER + 66 + 1184 + 1] = 0xff;
                assert!(PhoneHandoverRequestWire::decode(&bad).is_err());
            }
        }
        assert!(
            PhoneHandoverRequestWire::decode(&frame_header(KIND_PHONE_HANDOVER_REQUEST)).is_err()
        );
    }

    #[test]
    fn kind_12_matches_the_python_oracle_vectors() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/vectors/wire/phone_handover_request.json"
        );
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let hx = |v: &serde_json::Value| hex::decode(v.as_str().unwrap()).unwrap();
        for v in doc["vectors"].as_array().unwrap() {
            let i = &v["inputs"];
            let req = PhoneHandoverRequestWire {
                new_phone: crate::crypto::DevicePublic {
                    ed25519_pub: hx(&i["ed"]).try_into().unwrap(),
                    x25519_pub: hx(&i["x"]).try_into().unwrap(),
                    mlkem_pub: hx(&i["mlkem"]),
                },
                name: i["name"].as_str().unwrap().into(),
                terminal: hx(&i["terminal"]).try_into().unwrap(),
                op_hash: i["op_hash"]
                    .as_str()
                    .map(|h| hex::decode(h).unwrap().try_into().unwrap()),
            };
            let frame = hx(&v["outputs"]["frame"]);
            assert_eq!(req.encode(), frame, "{}", v["id"]);
            assert_eq!(
                PhoneHandoverRequestWire::decode(&frame).unwrap(),
                req,
                "{}",
                v["id"]
            );
        }
    }

    #[test]
    fn kind_12_admission_frame_carries_the_op_hash_and_nothing_else_is_accepted() {
        let req = PhoneHandoverRequestWire {
            new_phone: crate::crypto::DevicePublic {
                ed25519_pub: [1; 32],
                x25519_pub: [2; 32],
                mlkem_pub: vec![3; 1184],
            },
            name: "new phone".into(),
            terminal: [9; 8],
            op_hash: None,
        };
        let plain = req.encode();
        let stage2 = PhoneHandoverRequestWire {
            op_hash: Some([0xAB; 32]),
            ..req.clone()
        };
        let bytes = stage2.encode();
        assert_eq!(
            bytes[..plain.len()],
            plain[..],
            "the 4j layout is unchanged"
        );
        assert_eq!(bytes[plain.len()..], [&[1u8][..], &[0xAB; 32]].concat());
        assert_eq!(PhoneHandoverRequestWire::decode(&bytes).unwrap(), stage2);
        assert_eq!(PhoneHandoverRequestWire::decode(&plain).unwrap(), req);
        // a trailer that is not exactly 0x01 and 32 bytes is an error
        for cut in 1..=33 {
            assert!(
                PhoneHandoverRequestWire::decode(&bytes[..bytes.len() - cut]).is_err()
                    == (cut < 33),
                "cut {cut}"
            );
        }
        let mut long = bytes.clone();
        long.push(0);
        assert!(PhoneHandoverRequestWire::decode(&long).is_err());
        let mut tag = bytes.clone();
        tag[plain.len()] = 2;
        assert!(PhoneHandoverRequestWire::decode(&tag).is_err());
    }

    #[test]
    fn challenge_round_trip_and_header() {
        let c = sample();
        let bytes = c.encode();
        assert_eq!(&bytes[..4], b"SWR1");
        assert_eq!(&bytes[4..7], &[0, 1, 1]);
        assert_eq!(ChallengeWire::decode(&bytes).unwrap(), c);
    }

    #[test]
    fn challenge_decoder_rejects_malformed_input() {
        let good = sample().encode();
        for cut in [0, 6, 7, 50, good.len() - 1] {
            assert!(ChallengeWire::decode(&good[..cut]).is_err(), "cut at {cut}");
        }
        let mut bad = good.clone();
        bad[5] = 9;
        assert!(ChallengeWire::decode(&bad).is_err(), "version");
        let mut bad = good.clone();
        bad[0] ^= 1;
        assert!(ChallengeWire::decode(&bad).is_err(), "magic");
        let mut bad = good.clone();
        bad[6] = 2;
        assert!(ChallengeWire::decode(&bad).is_err(), "kind");
        let mut bad = good.clone();
        bad[7 + 80..7 + 84].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(ChallengeWire::decode(&bad).is_err(), "op length bomb");
        let mut long = good;
        long.push(0);
        assert!(ChallengeWire::decode(&long).is_err(), "trailing byte");
    }

    #[test]
    fn share_response_round_trip_and_rejections() {
        let r = ShareResponseWire {
            request_id: 42,
            ciphertext: vec![1, 2, 3],
        };
        let bytes = r.encode();
        assert_eq!(ShareResponseWire::decode(&bytes).unwrap(), r);
        assert_eq!(
            &bytes[7..],
            &[0, 0, 0, 0, 0, 0, 0, 42, 0, 0, 0, 0, 0, 0, 0, 3, 1, 2, 3]
        );
        assert!(ShareResponseWire::decode(&bytes[..bytes.len() - 1]).is_err());
        let mut bomb = bytes.clone();
        bomb[15..23].copy_from_slice(&u64::MAX.to_be_bytes());
        assert!(ShareResponseWire::decode(&bomb).is_err());
        assert!(ChallengeWire::decode(&bytes).is_err(), "kind mismatch");
        let empty = ShareResponseWire {
            request_id: 1,
            ciphertext: vec![],
        };
        assert_eq!(ShareResponseWire::decode(&empty.encode()).unwrap(), empty);
    }

    #[test]
    fn the_binding_covers_every_field_and_the_whole_op() {
        let c = sample();
        let h = c.binding_hash();
        let mut t = c.clone();
        t.op_canonical.push(b' ');
        assert_ne!(t.binding_hash(), h, "op bytes");
        for mutate in [
            |c: &mut ChallengeWire| c.nonce[0] ^= 1,
            |c: &mut ChallengeWire| c.expiry += 1,
            |c: &mut ChallengeWire| c.eph_x25519_pub[0] ^= 1,
            |c: &mut ChallengeWire| c.request_id += 1,
            |c: &mut ChallengeWire| c.phone_material.share_blob[0] ^= 1,
            |c: &mut ChallengeWire| c.phone_material.epoch += 1,
        ] {
            let mut t = c.clone();
            mutate(&mut t);
            assert_ne!(t.binding_hash(), h);
        }
        // the domain tag comes first and the version is little endian
        let pre = binding_preimage(
            &c.op_canonical,
            &c.nonce,
            c.expiry,
            &c.eph_x25519_pub,
            c.request_id,
            &c.phone_material.hash(),
        );
        assert!(pre.starts_with(b"sshub/sync/challenge/v1\x01\x00"));
    }

    #[test]
    fn phone_material_round_trips_in_both_shapes_and_rejects_inconsistent_ones() {
        for m in [sample().phone_material, envelope()] {
            assert_eq!(PhoneMaterial::decode(&m.encode()).unwrap(), m);
            let mut c = sample();
            c.phone_material = m.clone();
            assert_eq!(ChallengeWire::decode(&c.encode()).unwrap(), c);
        }
        let hybrid_with_storage = PhoneMaterial {
            storage_blob: vec![1],
            ..sample().phone_material
        };
        assert!(PhoneMaterial::decode(&hybrid_with_storage.encode()).is_err());
        let envelope_without = PhoneMaterial {
            storage_blob: vec![],
            ..envelope()
        };
        assert!(PhoneMaterial::decode(&envelope_without.encode()).is_err());
        let mut bad = envelope().encode();
        bad[0] = 7;
        assert!(PhoneMaterial::decode(&bad).is_err());
        assert!(PhoneMaterial::decode(&envelope().encode()[..20]).is_err());
    }

    /// Measured, not estimated: a realistic ShareRelease op, a real hybrid wrapped share of the
    /// phone, a real storage key blob. The breakdown is printed with `--nocapture`.
    #[test]
    fn a_realistic_challenge_is_under_two_kilobytes() {
        use crate::crypto::{hybrid_wrap, DeviceIdentity};
        use crate::fountain::{fountain_encode, frame_bytes_for_area};
        let phone = DeviceIdentity::generate().public();
        let op = format!(
            r#"{{"device_id":"{}","seq":1234,"lamport":5678,"prev_hash":"{}","timestamp":1893452400,"epoch":3,"alg":{{"sig":1,"wrap":1}},"body":{{"ShareRelease":{{"host_uid":"{}","request_id":1234567890123,"requester":"{}"}}}}}}"#,
            "d04ab232742bb4ab", "ab".repeat(32), "cd".repeat(16), "d04ab232742bb4ab"
        )
        .into_bytes();
        let hybrid = hybrid_wrap(&phone, &[0x11; 49]); // host_uid || index || share
        let storage = hybrid_wrap(&phone, &[0x22; 32]);
        let envelope_share = vec![0x33; 24 + 33 + 16];
        let breakdown = |m: PhoneMaterial| {
            let c = ChallengeWire {
                op_canonical: op.clone(),
                op_signature: [0; 64],
                nonce: [0; 32],
                expiry: 0,
                eph_x25519_pub: [0; 32],
                request_id: 1,
                phone_material: m,
            };
            (c.encode().len(), c.phone_material.encode().len(), c)
        };
        let (h_total, h_mat, h) = breakdown(PhoneMaterial {
            kind: ShareKind::Hybrid,
            epoch: 3,
            share_blob: hybrid.clone(),
            storage_blob: vec![],
        });
        let (e_total, e_mat, _) = breakdown(PhoneMaterial {
            kind: ShareKind::Envelope,
            epoch: 5,
            share_blob: envelope_share,
            storage_blob: storage,
        });
        println!("op canonical {} B, signature 64 B, fixed fields 91 B + header 7 B, material hybrid {h_mat} B (blob {} B), envelope {e_mat} B", op.len(), hybrid.len());
        println!("challenge total: hybrid {h_total} B, envelope {e_total} B");
        for (cols, rows) in [(80u16, 24u16), (120, 40)] {
            let fb = frame_bytes_for_area(cols, rows).unwrap();
            let frames = fountain_encode(&h.encode(), fb, 25).unwrap().len();
            println!(
                "{cols}x{rows}: {fb} B frames, {frames} frames per lap, {:.1} s at 8 fps",
                frames as f64 / 8.0
            );
        }
        assert!(h_total < 2000 && e_total < 2000, "{h_total} {e_total}");
    }
}

#[cfg(test)]
mod phone_frame_tests {
    use super::*;

    #[test]
    fn phone_only_frames_round_trip_and_reject_damage() {
        let notice = RecoveryNoticeWire {
            op_canonical: b"{\"op\":1}".to_vec(),
            op_signature: [9; 64],
        };
        assert_eq!(
            RecoveryNoticeWire::decode(&notice.encode()).unwrap(),
            notice
        );
        let mut cut = notice.encode();
        cut.pop();
        assert!(RecoveryNoticeWire::decode(&cut).is_err());

        let ops = PhoneOpsWire {
            ops: vec![(b"a".to_vec(), [1; 64]), (b"bb".to_vec(), [2; 64])],
            sender: None,
            terminals: None,
        };
        assert_eq!(PhoneOpsWire::decode(&ops.encode()).unwrap(), ops);
        let mut long = ops.encode();
        long.push(0);
        assert!(PhoneOpsWire::decode(&long).is_err(), "trailing bytes");
        let many = encode_op_list(&vec![(b"a".to_vec(), [1; 64]); MAX_COURIER_OPS]);
        assert_eq!(decode_op_list(&many).unwrap().0.len(), MAX_COURIER_OPS);
        let mut over = many.clone();
        over[0] = (MAX_COURIER_OPS + 1) as u8;
        assert!(decode_op_list(&over).is_err(), "too many ops");
        let big = [
            vec![1u8],
            ((MAX_COURIER_OP_BYTES + 1) as u32).to_be_bytes().to_vec(),
            vec![0; MAX_COURIER_OP_BYTES + 1 + 64],
        ]
        .concat();
        assert!(decode_op_list(&big).is_err(), "oversize op");

        for f in [
            EnrollFrame::Commit {
                role: 1,
                commit: [3; 32],
            },
            EnrollFrame::Reveal {
                role: 0,
                reveal: vec![4; 50],
            },
            EnrollFrame::Confirm {
                role: 1,
                ok: false,
                sig: [5; 64],
            },
        ] {
            assert_eq!(EnrollFrame::decode(&f.encode()).unwrap(), f);
        }
        assert!(
            EnrollFrame::decode(&notice.encode()).is_err(),
            "a notice is not an enrollment frame"
        );
    }
}

#[cfg(test)]
mod cert_request_tests {
    use super::*;
    use crate::oplog::CertScope;

    #[test]
    fn a_certificate_request_round_trips_and_refuses_damage() {
        let hosts = CertRequestWire {
            device_id: [7; 8],
            scope: CertScope::Hosts(vec![[1; 16], [2; 16]]),
            expiry: 1_900_000_000,
            max_new_hosts_per_day: 50,
            epoch_lo: 3,
            epoch_hi: 103,
        };
        let group = CertRequestWire {
            scope: CertScope::Group([9; 16]),
            ..hosts.clone()
        };
        for r in [&hosts, &group] {
            assert_eq!(&CertRequestWire::decode(&r.encode()).unwrap(), r);
        }
        let mut long = hosts.encode();
        long.push(0);
        assert!(CertRequestWire::decode(&long).is_err());
        let mut cut = hosts.encode();
        cut.pop();
        assert!(CertRequestWire::decode(&cut).is_err());
        let mut bad_tag = hosts.encode();
        bad_tag[HEADER + 8] = 7;
        assert!(CertRequestWire::decode(&bad_tag).is_err());
        let empty = CertRequestWire {
            scope: CertScope::Hosts(vec![]),
            ..hosts.clone()
        };
        assert!(CertRequestWire::decode(&empty.encode()).is_err());
        let many = CertRequestWire {
            scope: CertScope::Hosts(vec![[0; 16]; MAX_CERT_REQUEST_HOSTS + 1]),
            ..hosts.clone()
        };
        assert!(CertRequestWire::decode(&many.encode()).is_err());
        assert!(
            matches!(hosts.to_body(), crate::oplog::OpBody::CertIssue { ref device_id, .. } if device_id == "0707070707070707")
        );
        assert!(CertRequestWire::decode(
            &ShareResponseWire {
                request_id: 1,
                ciphertext: vec![0; 4]
            }
            .encode()
        )
        .is_err());
    }
}
