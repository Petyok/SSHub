//! Hybrid wrapping of long-lived material (wrapped shares, Vault slots): X25519 and
//! ML-KEM-768 together, so breaking either alone does not open a blob.
//!
//! Blob: `"SSHW" || sig_suite || wrap_suite || eph_pub(32) || kem_ct_len u16 BE || kem_ct ||
//! nonce(24) || ct`. Everything before `ct` is associated data of the AEAD, so the
//! algorithm tag cannot be flipped in transit. The key is
//! `BLAKE3 derive_key("sshub/sync/wrap/v1", x25519_ss || mlkem_ss || eph_pub || recipient_x25519_pub || kem_ct)`.

use anyhow::{anyhow, bail, Result};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305,
};
use rand_core::{OsRng, RngCore};
use x25519_dalek::{PublicKey as XPublic, StaticSecret as XSecret};
use zeroize::Zeroizing;

use super::alg::{AlgTag, ALG_CURRENT};
use super::hash::derive_key;
use super::identity::{DeviceIdentity, DevicePublic};
use super::mlkem;

const MAGIC: &[u8; 4] = b"SSHW";
const KDF_CTX: &str = "sshub/sync/wrap/v1";

fn kdf(
    x_ss: &[u8; 32],
    kem_ss: &[u8; 32],
    eph_pub: &[u8; 32],
    recipient_x: &[u8; 32],
    kem_ct: &[u8],
) -> Zeroizing<[u8; 32]> {
    let ikm = Zeroizing::new(
        [
            x_ss.as_slice(),
            kem_ss.as_slice(),
            eph_pub.as_slice(),
            recipient_x.as_slice(),
            kem_ct,
        ]
        .concat(),
    );
    Zeroizing::new(derive_key(KDF_CTX, &ikm))
}

/// Wraps `plaintext` to `recipient`. Panics only if the recipient key is malformed
/// (validate keys from other devices with [`DevicePublic::validate`] or use [`try_hybrid_wrap`]).
pub fn hybrid_wrap(recipient: &DevicePublic, plaintext: &[u8]) -> Vec<u8> {
    try_hybrid_wrap(recipient, plaintext).expect("recipient public key is well formed")
}

pub fn try_hybrid_wrap(recipient: &DevicePublic, plaintext: &[u8]) -> Result<Vec<u8>> {
    let mut eph = Zeroizing::new([0u8; 32]);
    let mut nonce = [0u8; 24];
    OsRng.fill_bytes(&mut eph[..]);
    OsRng.fill_bytes(&mut nonce);
    let (kem_ct, kem_ss) = mlkem::encapsulate(&recipient.mlkem_pub, &mut OsRng)?;
    Ok(assemble(
        recipient,
        plaintext,
        &eph,
        &nonce,
        &kem_ct,
        &Zeroizing::new(kem_ss),
    ))
}

#[cfg(test)]
/// Deterministic form for test vectors: ephemeral secret, nonce and ML-KEM message given.
pub(crate) fn hybrid_wrap_with(
    recipient: &DevicePublic,
    plaintext: &[u8],
    eph: &[u8; 32],
    nonce: &[u8; 24],
    m: &[u8; 32],
) -> Result<Vec<u8>> {
    let (kem_ct, kem_ss) = mlkem::encapsulate_with(&recipient.mlkem_pub, m)?;
    Ok(assemble(
        recipient,
        plaintext,
        eph,
        nonce,
        &kem_ct,
        &Zeroizing::new(kem_ss),
    ))
}

fn assemble(
    recipient: &DevicePublic,
    plaintext: &[u8],
    eph: &[u8; 32],
    nonce: &[u8; 24],
    kem_ct: &[u8],
    kem_ss: &[u8; 32],
) -> Vec<u8> {
    let eph = XSecret::from(*eph);
    let eph_pub = XPublic::from(&eph).to_bytes();
    let x_ss = eph.diffie_hellman(&XPublic::from(recipient.x25519_pub));
    let key = kdf(
        x_ss.as_bytes(),
        kem_ss,
        &eph_pub,
        &recipient.x25519_pub,
        kem_ct,
    );
    let mut out = Vec::with_capacity(4 + 2 + 32 + 2 + kem_ct.len() + 24 + plaintext.len() + 16);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&ALG_CURRENT.to_bytes());
    out.extend_from_slice(&eph_pub);
    out.extend_from_slice(&(kem_ct.len() as u16).to_be_bytes());
    out.extend_from_slice(kem_ct);
    out.extend_from_slice(nonce);
    let ct = XChaCha20Poly1305::new((&*key).into())
        .encrypt(
            nonce.into(),
            Payload {
                msg: plaintext,
                aad: &out,
            },
        )
        .expect("XChaCha20-Poly1305 encrypt");
    out.extend_from_slice(&ct);
    out
}

/// Reads the algorithm tag without parsing the rest, so ingest can refuse an unknown
/// generation with a legible error.
pub fn blob_alg(blob: &[u8]) -> Result<AlgTag> {
    if blob.len() < 6 || &blob[..4] != MAGIC {
        bail!("not a wrapped blob");
    }
    AlgTag::from_bytes([blob[4], blob[5]])
}

pub fn hybrid_unwrap(id: &DeviceIdentity, blob: &[u8]) -> Result<Vec<u8>> {
    if blob_alg(blob)? != ALG_CURRENT {
        bail!("wrapped blob uses an algorithm suite this build does not support");
    }
    let trunc = || anyhow!("truncated wrapped blob");
    let eph_pub: [u8; 32] = blob.get(6..38).ok_or_else(trunc)?.try_into().unwrap();
    let kem_len =
        u16::from_be_bytes(blob.get(38..40).ok_or_else(trunc)?.try_into().unwrap()) as usize;
    if kem_len != mlkem::CT_LEN {
        bail!("unexpected ML-KEM ciphertext length {kem_len}");
    }
    let kem_ct = blob.get(40..40 + kem_len).ok_or_else(trunc)?;
    let nonce_at = 40 + kem_len;
    let nonce: [u8; 24] = blob
        .get(nonce_at..nonce_at + 24)
        .ok_or_else(trunc)?
        .try_into()
        .unwrap();
    let header_end = nonce_at + 24;
    let ct = blob.get(header_end..).ok_or_else(trunc)?;

    // try every wrapping generation the identity still holds, newest first
    for keys in id.wrap_keys() {
        let x_ss = XSecret::from(*keys.x_secret_bytes()).diffie_hellman(&XPublic::from(eph_pub));
        if !x_ss.was_contributory() {
            bail!("wrapped blob with a low-order ephemeral key");
        }
        let Ok(kem_ss) = mlkem::decapsulate(keys.mlkem_dk_bytes(), kem_ct) else {
            continue;
        };
        let kem_ss = Zeroizing::new(kem_ss);
        let key = kdf(x_ss.as_bytes(), &kem_ss, &eph_pub, &keys.x_public(), kem_ct);
        if let Ok(plain) = XChaCha20Poly1305::new((&*key).into()).decrypt(
            (&nonce).into(),
            Payload {
                msg: ct,
                aad: &blob[..header_end],
            },
        ) {
            return Ok(plain);
        }
    }
    Err(anyhow!("unwrap failed: not our blob, or tampered"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_rejections() {
        let a = DeviceIdentity::generate();
        let b = DeviceIdentity::generate();
        for pt in [&b""[..], b"share", &[9u8; 4096]] {
            let blob = hybrid_wrap(&a.public(), pt);
            assert_eq!(blob_alg(&blob).unwrap(), ALG_CURRENT);
            assert_eq!(hybrid_unwrap(&a, &blob).unwrap(), pt);
            assert!(hybrid_unwrap(&b, &blob).is_err(), "wrong recipient");
        }
    }

    #[test]
    fn every_header_byte_is_authenticated() {
        let a = DeviceIdentity::generate();
        let blob = hybrid_wrap(&a.public(), b"plaintext");
        let header_end = 40 + mlkem::CT_LEN + 24;
        for i in 0..blob.len() {
            let mut t = blob.clone();
            t[i] ^= 1;
            assert!(
                hybrid_unwrap(&a, &t).is_err(),
                "flip of byte {i} (header end {header_end}) was accepted"
            );
        }
    }

    #[test]
    fn an_unknown_suite_is_refused_with_a_legible_error() {
        let a = DeviceIdentity::generate();
        let mut blob = hybrid_wrap(&a.public(), b"x");
        blob[4] = 9;
        assert!(blob_alg(&blob)
            .unwrap_err()
            .to_string()
            .contains("unknown signature suite 9"));
        assert!(hybrid_unwrap(&a, &blob).is_err());
        assert!(blob_alg(b"nope").is_err());
        assert!(hybrid_unwrap(&a, &blob[..30]).is_err());
    }

    #[test]
    fn a_malformed_recipient_is_an_error_not_a_panic_through_try() {
        let mut p = DeviceIdentity::generate().public();
        p.mlkem_pub.truncate(5);
        assert!(try_hybrid_wrap(&p, b"x").is_err());
    }
}
