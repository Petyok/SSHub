//! The share response: what a holder sends back for a challenge. The share (index byte plus 32
//! bytes) is sealed to the requester's ephemeral X25519 key from the challenge with the sealed
//! box (`DeviceIdentity::seal_to`), so eavesdropping any channel (audio, reverse QR, clipboard,
//! a typed code) yields ciphertext for a one-time key. Validity is decided by AEAD success;
//! the request id is only a fast filter.

use anyhow::{bail, Result};

use super::crypto::{
    aead_open, dek_open, open_sealed_with_secret, shamir_combine, DeviceIdentity, Share,
};

/// `index || bytes`: 33 bytes.
pub fn share_to_bytes(s: &Share) -> Vec<u8> {
    let mut b = Vec::with_capacity(1 + s.bytes.len());
    b.push(s.index);
    b.extend_from_slice(&s.bytes);
    b
}

pub fn share_from_bytes(b: &[u8]) -> Result<Share> {
    if b.len() != 33 || b[0] == 0 {
        bail!("a share is 33 bytes with a nonzero index");
    }
    Ok(Share {
        index: b[0],
        bytes: b[1..].to_vec(),
    })
}

/// Plaintext of a hybrid wrapped share: `host_uid(16) || index || share`. The credential id is
/// inside the wrap so a requester cannot pair the description of host A with a share of host B.
pub fn wrapped_share_plaintext(host_uid: &[u8; 16], share: &Share) -> Vec<u8> {
    [host_uid.as_slice(), &share_to_bytes(share)].concat()
}

/// Parses an unwrapped share and REFUSES it unless it names `expected_host`.
pub fn parse_wrapped_share_plaintext(expected_host: &[u8; 16], plain: &[u8]) -> Result<Share> {
    if plain.len() != 16 + 33 {
        bail!("a wrapped share is 49 bytes (host id, index, share)");
    }
    if &plain[..16] != expected_host {
        bail!("this share belongs to another credential");
    }
    share_from_bytes(&plain[16..])
}

/// What a holder (a peer terminal) puts on the wire for a requester.
pub fn seal_share_response(eph_x25519_pub: &[u8; 32], share: &Share) -> Vec<u8> {
    DeviceIdentity::seal_to(eph_x25519_pub, &share_to_bytes(share))
}

/// The requester's side: opens a response with the ephemeral secret of the challenge.
pub fn unwrap_share_response(eph_priv: &[u8; 32], ciphertext: &[u8]) -> Result<Share> {
    share_from_bytes(&open_sealed_with_secret(eph_priv, ciphertext)?)
}

/// Combines the requester's own share with a released one in memory and checks the result by
/// decrypting the credential: a wrong or stale share fails AEAD instead of returning garbage.
/// Returns the DEK; the caller opens the credential with it and drops it.
pub fn combine_and_check(mine: &Share, second: &Share, ciphertext: &[u8]) -> Result<[u8; 32]> {
    let dek = shamir_combine(&[mine.clone(), second.clone()])?;
    dek_open(&dek, ciphertext)?;
    Ok(dek)
}

/// Opens an epoch envelope share (`aead_seal` under a storage key with its context as associated data).
pub fn open_envelope_share(storage_key: &[u8; 32], aad: &[u8], sealed: &[u8]) -> Result<Share> {
    share_from_bytes(&aead_open(storage_key, aad, sealed)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::crypto::{dek_generate, dek_seal, shamir_split};
    use x25519_dalek::{PublicKey, StaticSecret};

    #[test]
    fn a_released_share_round_trips_only_for_the_ephemeral_key() {
        let eph = StaticSecret::from([7u8; 32]);
        let eph_pub = PublicKey::from(&eph).to_bytes();
        let share = Share {
            index: 2,
            bytes: vec![9; 32],
        };
        let sealed = seal_share_response(&eph_pub, &share);
        assert_eq!(
            sealed.len(),
            32 + 24 + 33 + 16,
            "about 105 bytes, small enough for audio and reverse QR"
        );
        assert_eq!(
            unwrap_share_response(&eph.to_bytes(), &sealed).unwrap(),
            share
        );
        assert!(
            unwrap_share_response(&[8u8; 32], &sealed).is_err(),
            "another key"
        );
        let mut t = sealed.clone();
        let last = t.len() - 1;
        t[last] ^= 1;
        assert!(
            unwrap_share_response(&eph.to_bytes(), &t).is_err(),
            "tampered (clipboard injection is a denial of service at most)"
        );
    }

    #[test]
    fn a_share_of_another_credential_is_refused() {
        let share = Share {
            index: 4,
            bytes: vec![5; 32],
        };
        let plain = wrapped_share_plaintext(&[1; 16], &share);
        assert_eq!(plain.len(), 49);
        assert_eq!(
            parse_wrapped_share_plaintext(&[1; 16], &plain).unwrap(),
            share
        );
        assert!(
            parse_wrapped_share_plaintext(&[2; 16], &plain).is_err(),
            "confused deputy: wrong credential"
        );
        assert!(parse_wrapped_share_plaintext(&[1; 16], &plain[..48]).is_err());
    }

    #[test]
    fn share_bytes_are_validated() {
        assert!(share_from_bytes(&[0; 33]).is_err(), "index 0");
        assert!(share_from_bytes(&[1; 32]).is_err(), "short");
        assert_eq!(
            share_to_bytes(&share_from_bytes(&[3; 33]).unwrap()),
            vec![3; 33]
        );
    }

    #[test]
    fn combining_is_checked_against_the_credential() {
        let dek = dek_generate();
        let ct = dek_seal(&dek, b"hunter2");
        let shares = shamir_split(&dek, 2, 3);
        assert_eq!(combine_and_check(&shares[0], &shares[2], &ct).unwrap(), dek);
        let stale = Share {
            index: 3,
            bytes: vec![0xAB; 32],
        };
        assert!(combine_and_check(&shares[0], &stale, &ct).is_err());
    }
}
