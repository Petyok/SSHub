//! `SYNC_WIRE_V1`: the byte contract between the terminal and the phone page. Frames are
//! hand-encoded (no serialisation library) so the page can implement them in a few lines.
//!
//! ```text
//! frame     = "SWR1" || version u16 BE || kind u8 || body
//! challenge = kind 1: nonce(32) || expiry i64 BE || eph_x25519_pub(32) || request_id u64 BE ||
//!                     op_len u32 BE || op_canonical(op_len) || op_signature(64)
//! response  = kind 2: request_id u64 BE || len u64 BE || ciphertext(len)
//! ```
//!
//! The op travels as its canonical bytes plus its signature, exactly as the terminal hashed
//! and signed them: the phone verifies those bytes and renders its description from them,
//! it never re-serialises. The channel binding hash covers the WHOLE canonical op.

use anyhow::{bail, Result};

use super::crypto::chain_hash;
use super::limits::MAX_FRAME_BYTES;

pub const SYNC_WIRE_V1: u16 = 1;
pub const WIRE_MAGIC: [u8; 4] = *b"SWR1";
pub const KIND_CHALLENGE: u8 = 1;
pub const KIND_SHARE_RESPONSE: u8 = 2;
/// Domain separation tag of the channel binding.
pub const DOMAIN_TAG_CHALLENGE: &[u8] = b"sshub/sync/challenge/v1";
const HEADER: usize = 7;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChallengeWire {
    pub op_canonical: Vec<u8>,
    pub op_signature: [u8; 64],
    pub nonce: [u8; 32],
    pub expiry: i64,
    pub eph_x25519_pub: [u8; 32],
    pub request_id: u64,
}

/// The exact bytes an approving holder signs:
/// `tag || proto_version u16 LE || canonical_op || nonce || expiry i64 LE || eph || request_id u64 LE`.
pub fn binding_preimage(op_canonical: &[u8], nonce: &[u8; 32], expiry: i64, eph: &[u8; 32], request_id: u64) -> Vec<u8> {
    let mut b = Vec::with_capacity(DOMAIN_TAG_CHALLENGE.len() + 2 + op_canonical.len() + 32 + 8 + 32 + 8);
    b.extend_from_slice(DOMAIN_TAG_CHALLENGE);
    b.extend_from_slice(&SYNC_WIRE_V1.to_le_bytes());
    b.extend_from_slice(op_canonical);
    b.extend_from_slice(nonce);
    b.extend_from_slice(&expiry.to_le_bytes());
    b.extend_from_slice(eph);
    b.extend_from_slice(&request_id.to_le_bytes());
    b
}

/// `BLAKE3(binding_preimage(..))`.
pub fn binding_hash(op_canonical: &[u8], nonce: &[u8; 32], expiry: i64, eph: &[u8; 32], request_id: u64) -> [u8; 32] {
    chain_hash(&binding_preimage(op_canonical, nonce, expiry, eph, request_id))
}

impl ChallengeWire {
    pub fn binding_hash(&self) -> [u8; 32] {
        binding_hash(&self.op_canonical, &self.nonce, self.expiry, &self.eph_x25519_pub, self.request_id)
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = frame_header(KIND_CHALLENGE);
        out.extend_from_slice(&self.nonce);
        out.extend_from_slice(&self.expiry.to_be_bytes());
        out.extend_from_slice(&self.eph_x25519_pub);
        out.extend_from_slice(&self.request_id.to_be_bytes());
        out.extend_from_slice(&u32::try_from(self.op_canonical.len()).expect("op fits a frame").to_be_bytes());
        out.extend_from_slice(&self.op_canonical);
        out.extend_from_slice(&self.op_signature);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let body = unframe(bytes, KIND_CHALLENGE)?;
        if body.len() < 32 + 8 + 32 + 8 + 4 + 64 {
            bail!("challenge too short");
        }
        let op_len = u32::from_be_bytes(body[80..84].try_into().unwrap()) as usize;
        if op_len > MAX_FRAME_BYTES {
            bail!("challenge op of {op_len} bytes exceeds the frame cap");
        }
        if body.len() != 84 + op_len + 64 {
            bail!("challenge length does not match its op length");
        }
        Ok(Self {
            nonce: body[..32].try_into().unwrap(),
            expiry: i64::from_be_bytes(body[32..40].try_into().unwrap()),
            eph_x25519_pub: body[40..72].try_into().unwrap(),
            request_id: u64::from_be_bytes(body[72..80].try_into().unwrap()),
            op_canonical: body[84..84 + op_len].to_vec(),
            op_signature: body[84 + op_len..].try_into().unwrap(),
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
        Ok(Self { request_id: u64::from_be_bytes(body[..8].try_into().unwrap()), ciphertext: body[16..].to_vec() })
    }
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
        }
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
        let r = ShareResponseWire { request_id: 42, ciphertext: vec![1, 2, 3] };
        let bytes = r.encode();
        assert_eq!(ShareResponseWire::decode(&bytes).unwrap(), r);
        assert_eq!(&bytes[7..], &[0, 0, 0, 0, 0, 0, 0, 42, 0, 0, 0, 0, 0, 0, 0, 3, 1, 2, 3]);
        assert!(ShareResponseWire::decode(&bytes[..bytes.len() - 1]).is_err());
        let mut bomb = bytes.clone();
        bomb[15..23].copy_from_slice(&u64::MAX.to_be_bytes());
        assert!(ShareResponseWire::decode(&bomb).is_err());
        assert!(ChallengeWire::decode(&bytes).is_err(), "kind mismatch");
        let empty = ShareResponseWire { request_id: 1, ciphertext: vec![] };
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
        ] {
            let mut t = c.clone();
            mutate(&mut t);
            assert_ne!(t.binding_hash(), h);
        }
        // the domain tag comes first and the version is little endian
        let pre = binding_preimage(&c.op_canonical, &c.nonce, c.expiry, &c.eph_x25519_pub, c.request_id);
        assert!(pre.starts_with(b"sshub/sync/challenge/v1\x01\x00"));
    }
}
