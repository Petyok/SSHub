//! Canonical `Op` serialisation (stage-1 oplog) and the stage-3 channel
//! binding: `Challenge`, `binding_hash`, the signed-approval preimage, and the
//! `SWR1` share-response wire frame.
//!
//! Canonical form = deterministic JSON of `OpCanonical` (fixed field order,
//! `prev_hash` hex-encoded); signatures cover exactly those bytes. Inside the
//! body, 32-byte hash fields stay JSON NUMBER ARRAYS - only the header's
//! `prev_hash` is hex. All rules verbatim from stage-1:398-603 and stage-3.

use anyhow::bail;
use serde::{Deserialize, Serialize};

use super::primitives::chain_hash;

/// Stable short device id: lowercase hex of the first 8 bytes of the ed25519 key.
pub type DeviceId = String;

/// Protocol version (stage-3:576).
pub const PROTO_VERSION: u16 = 1;
/// Challenge domain tag (stage-3:577).
pub const DOMAIN_TAG_CHALLENGE: &[u8] = b"sshub/sync/challenge/v1";

// Wire frame (stage-3 wire.rs): WIRE_MAGIC ‖ version u16 BE ‖ kind u8 ‖ body.
pub const WIRE_MAGIC: [u8; 4] = *b"SWR1";
pub const SYNC_WIRE_V1: u16 = 1;
pub const KIND_CHALLENGE: u8 = 1;
pub const KIND_SHARE_RESPONSE: u8 = 2;

/// One wrapped Shamir share riding inside a `CredUpdate` op. Replicated to all
/// peers so revoking a holder never loses anyone else's share.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WrappedShare {
    pub recipient: DeviceId,
    pub sealed: Vec<u8>,
}

/// Third reenrollment artifact (stage-1/README): unit variants serialize as
/// JSON strings, the newtype as `{"HardwareKey":"<device_id>"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThirdFactor {
    Paper,
    HardwareKey(DeviceId),
    EmailCode,
}

/// Credentials held in Vault vs released to the quorum (stage-1:45).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CredMode {
    Vault,
    Quorum,
}

/// Body of a mutation. Serde external tagging gives a deterministic, versionable
/// JSON shape. Every variant's fields serialize in declaration order.
///
/// SUBSET of stage-1:424-520 - exactly the variants the PoC A vectors exercise,
/// field names/types verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpBody {
    HostUpsert {
        host_id: i64,
        name: String,
        address: String,
        port: u16,
        /// Remaining host fields as a canonical JSON string (kept opaque here;
        /// applied by the review flow in stage 4).
        fields_json: String,
    },
    CredUpdate {
        host_id: i64,
        ciphertext: Vec<u8>,
        wrapped_shares: Vec<WrappedShare>,
    },
    ShareRelease {
        host_id: i64,
        request_id: u64,
        requester: DeviceId,
    },
    ShareReshareConfirm {
        reshare_hash: [u8; 32],
        by: DeviceId,
    },
    PhoneReenrollCancel {
        reenroll_hash: [u8; 32],
        /// A live phone cancels without factors.
        by_phone: bool,
        third_factor: Option<ThirdFactor>,
    },
    ModeChange {
        to: CredMode,
        migrated_on: DeviceId,
        cosigner: Option<DeviceId>,
    },
    HeadAttest {
        device_id: DeviceId,
        seq: u64,
        chain_hash: [u8; 32],
    },
}

/// A signed op in a device's chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Op {
    pub device_id: DeviceId,
    pub seq: u64,
    pub lamport: u64,
    pub prev_hash: [u8; 32],
    pub timestamp: i64,
    pub epoch: u64,
    pub body: OpBody,
    pub signature: [u8; 64],
}

/// Everything under the signature, in a fixed order. `signature` is excluded so
/// the op's identity (`hash`) is stable regardless of who signed it.
#[derive(Serialize)]
struct OpCanonical<'a> {
    device_id: &'a str,
    seq: u64,
    lamport: u64,
    prev_hash: String, // hex, stable across platforms
    timestamp: i64,
    epoch: u64,
    body: &'a OpBody,
}

/// Derive the [`DeviceId`] from an ed25519 public key: lowercase hex of the
/// first 8 bytes (stage-1:406-412).
pub fn device_id_of(ed25519_pub: &[u8; 32]) -> DeviceId {
    hex_encode(&ed25519_pub[..8])
}

pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    hex::encode(bytes)
}

impl Op {
    /// Deterministic byte form of everything the signature commits to.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let c = OpCanonical {
            device_id: &self.device_id,
            seq: self.seq,
            lamport: self.lamport,
            prev_hash: hex_encode(&self.prev_hash),
            timestamp: self.timestamp,
            epoch: self.epoch,
            body: &self.body,
        };
        // to_vec on a struct with fixed field order is deterministic; OpBody is
        // externally tagged with declaration-order fields → stable bytes.
        serde_json::to_vec(&c).expect("OpCanonical serializes")
    }

    /// Content-addressed id = chain_hash(canonical_bytes).
    pub fn hash(&self) -> [u8; 32] {
        chain_hash(&self.canonical_bytes())
    }

    /// True iff `signature` is a valid ed25519 signature by `signer_pub` over the
    /// canonical bytes.
    pub fn verify(&self, signer_pub: &[u8; 32]) -> bool {
        super::primitives::verify(signer_pub, &self.canonical_bytes(), &self.signature)
    }
}

// ---------------------------------------------------------------- channel binding

/// A `ShareRelease` challenge handed to a holder (stage-3:579-586).
#[derive(Debug, Clone)]
pub struct Challenge {
    pub op: Op,
    pub nonce: [u8; 32],
    pub expiry: i64,
    pub eph_x25519_pub: [u8; 32],
    pub request_id: u64,
}

impl Challenge {
    /// The exact bytes an approving holder signs (Q2 decision: the PREIMAGE,
    /// not the hash output). Domain tag FIRST, proto u16 LE, then the FULL
    /// canonical op, then only fixed-width fields (stage-3:599-616).
    pub fn preimage_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(DOMAIN_TAG_CHALLENGE); // b"sshub/sync/challenge/v1"
        buf.extend_from_slice(&PROTO_VERSION.to_le_bytes()); // u16 LE
        buf.extend_from_slice(&self.op.canonical_bytes()); // the WHOLE op, not only the body
        buf.extend_from_slice(&self.nonce); // 32 bytes raw
        buf.extend_from_slice(&self.expiry.to_le_bytes()); // i64 LE
        buf.extend_from_slice(&self.eph_x25519_pub); // 32 bytes raw
        buf.extend_from_slice(&self.request_id.to_le_bytes()); // u64 LE
        buf
    }

    /// Channel binding: chain_hash over [`Challenge::preimage_bytes`].
    pub fn binding_hash(&self) -> [u8; 32] {
        chain_hash(&self.preimage_bytes())
    }
}

// ------------------------------------------------- share-response wire frame

/// Body of the `share_response` wire blob (stage-3:222-226). The ciphertext is
/// a `ChaChaBox` sealed box to the challenge's `eph_x25519_pub`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareResponse {
    pub request_id: u64,
    pub ciphertext: Vec<u8>,
}

/// Wire config pinned by stage-3:254-260 and empirically by RECON §3d-ii:
/// standard() with fixed-int + big-endian → `request_id u64 BE ‖ len u64 BE ‖
/// bytes` for `ShareResponse`.
pub const WIRE_CONFIG: bincode::config::Configuration<
    bincode::config::BigEndian,
    bincode::config::Fixint,
> = bincode::config::standard()
    .with_fixed_int_encoding()
    .with_big_endian();

/// Frame a share response for the sync wire: `SWR1 ‖ 00 01 ‖ 02 ‖ body`.
pub fn encode_share_response_frame(resp: &ShareResponse) -> anyhow::Result<Vec<u8>> {
    let body = bincode::serde::encode_to_vec(resp, WIRE_CONFIG)?;
    let mut out = Vec::with_capacity(7 + body.len());
    out.extend_from_slice(&WIRE_MAGIC);
    out.extend_from_slice(&SYNC_WIRE_V1.to_be_bytes());
    out.push(KIND_SHARE_RESPONSE);
    out.extend_from_slice(&body);
    Ok(out)
}

/// Wire-frame parser: checks magic, version, kind; returns the body of a
/// `share_response` (stage-3 decoder contract).
pub fn decode_wire_frame(frame: &[u8]) -> anyhow::Result<ShareResponse> {
    if frame.len() < 7 {
        bail!("wire frame too short: {} bytes", frame.len());
    }
    if frame[..4] != WIRE_MAGIC {
        bail!("bad wire magic");
    }
    let version = u16::from_be_bytes([frame[4], frame[5]]);
    if version != SYNC_WIRE_V1 {
        bail!("unsupported wire version {}", u16::from_be_bytes([frame[4], frame[5]]));
    }
    if frame[6] != KIND_SHARE_RESPONSE {
        bail!("unexpected wire kind {}", frame[6]);
    }
    let (resp, _used) = bincode::serde::decode_from_slice::<ShareResponse, _>(
        &frame[7..],
        WIRE_CONFIG,
    )?;
    Ok(resp)
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;



    fn base_op(body: OpBody) -> Op {
        Op {
            device_id: "0123456789abcdef".into(),
            seq: 1,
            lamport: 1,
            prev_hash: [0u8; 32],
            timestamp: 1_893_452_400,
            epoch: 0,
            body,
            signature: [0u8; 64],
        }
    }

    /// Stage-1 test `canonical_bytes_is_deterministic_and_excludes_signature`
    /// (stage-1:326-342): two ops differing only in `signature` have equal
    /// canonical bytes and hash.
    #[test]
    fn canonical_bytes_is_deterministic_and_excludes_signature() {
        let op = base_op(OpBody::ShareRelease {
            host_id: 7,
            request_id: 42,
            requester: "aa11".into(),
        });
        let mut other = op.clone();
        other.signature[0] = 0xAB;
        assert_eq!(op.canonical_bytes(), other.canonical_bytes());
        assert_eq!(op.hash(), other.hash());
    }

    /// Canonical bytes are valid JSON with declaration-order fields, lowercase
    /// hex prev_hash, and externally tagged body.
    #[test]
    fn canonical_shape() {
        let op = base_op(OpBody::HostUpsert {
            host_id: 7,
            name: "Хост ⚡".into(),
            address: "h.example".into(),
            port: 22,
            fields_json: "{}".into(),
        });
        let s = String::from_utf8(op.canonical_bytes()).unwrap();
        assert!(s.starts_with("{\"device_id\":\""));
        assert!(s.contains("\"prev_hash\":\"0000000000000000000000000000000000000000000000000000000000000000\""));
        assert!(s.contains("\"HostUpsert\""));
        // serde_json emits raw UTF-8, not \u escapes.
        assert!(s.contains("Хост ⚡"));
    }

    /// The exact wire body pinned empirically during recon:
    /// ShareResponse{request_id:99, ciphertext:[10,20,30]} →
    /// 0000000000000063 0000000000000003 0a141e.
    #[test]
    fn wire_frame_matches_recon_pin() {
        let resp = ShareResponse {
            request_id: 99,
            ciphertext: vec![10, 20, 30],
        };
        let body = bincode::serde::encode_to_vec(&resp, WIRE_CONFIG).unwrap();
        assert_eq!(
            hex::encode(&body),
            "00000000000000630000000000000003_0a141e".replace('_', "")
        );
        let frame = encode_share_response_frame(&resp).unwrap();
        assert_eq!(&frame[..4], b"SWR1");
        assert_eq!(&frame[4..6], &[0x00, 0x01]);
        assert_eq!(frame[6], 2);
        let (back, _) = bincode::serde::decode_from_slice::<ShareResponse, _>(&frame[7..], WIRE_CONFIG)
            .unwrap();
        assert_eq!(back, resp);
        assert_eq!(decode_wire_frame(&frame).unwrap(), resp);
    }

    /// Empty ciphertext encodes to a zero length field, not null/absent.
    #[test]
    fn wire_frame_empty_ciphertext() {
        let resp = ShareResponse { request_id: 1, ciphertext: vec![] };
        let body = bincode::serde::encode_to_vec(&resp, WIRE_CONFIG).unwrap();
        assert_eq!(hex::encode(&body), "00000000000000010000000000000000");
        assert_eq!(decode_wire_frame(&encode_share_response_frame(&resp).unwrap()).unwrap(), resp);
    }

    /// binding_hash recompute from raw pieces (stage-3:457 formula).
    #[test]
    fn binding_hash_matches_manual_preimage() {
        let op = base_op(OpBody::ShareRelease {
            host_id: 7,
            request_id: 42,
            requester: "aa11".into(),
        });
        let ch = Challenge {
            op,
            nonce: [9u8; 32],
            expiry: 1_893_456_000,
            eph_x25519_pub: [7u8; 32],
            request_id: 42,
        };
        let mut buf = Vec::new();
        buf.extend_from_slice(b"sshub/sync/challenge/v1");
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&ch.op.canonical_bytes());
        buf.extend_from_slice(&[9u8; 32]);
        buf.extend_from_slice(&1_893_456_000i64.to_le_bytes());
        buf.extend_from_slice(&[7u8; 32]);
        buf.extend_from_slice(&42u64.to_le_bytes());
        assert_eq!(ch.preimage_bytes(), buf);
        assert_eq!(ch.binding_hash(), chain_hash(&buf));
    }
}
