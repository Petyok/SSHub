//! Per-credential data encryption key: XChaCha20-Poly1305, `nonce(24) || ciphertext+tag`.

use anyhow::{anyhow, bail, Result};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    XChaCha20Poly1305,
};
use rand_core::{OsRng, RngCore};

pub fn dek_generate() -> [u8; 32] {
    let mut k = [0u8; 32];
    OsRng.fill_bytes(&mut k);
    k
}

pub fn dek_seal(dek: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    let mut nonce = [0u8; 24];
    OsRng.fill_bytes(&mut nonce);
    dek_seal_with(dek, plaintext, &nonce)
}

pub(crate) fn dek_seal_with(dek: &[u8; 32], plaintext: &[u8], nonce: &[u8; 24]) -> Vec<u8> {
    let ct = XChaCha20Poly1305::new(dek.into())
        .encrypt(nonce.into(), plaintext)
        .expect("XChaCha20-Poly1305 encrypt");
    let mut out = Vec::with_capacity(24 + ct.len());
    out.extend_from_slice(nonce);
    out.extend_from_slice(&ct);
    out
}

/// XChaCha20-Poly1305 under any 32-byte key with associated data, `nonce(24) || ciphertext+tag`.
/// For symmetric envelopes whose context (credential, epoch, holder) must be bound to the bytes.
pub fn aead_seal(key: &[u8; 32], aad: &[u8], plaintext: &[u8]) -> Vec<u8> {
    let mut nonce = [0u8; 24];
    OsRng.fill_bytes(&mut nonce);
    let ct = XChaCha20Poly1305::new(key.into())
        .encrypt(
            (&nonce).into(),
            chacha20poly1305::aead::Payload {
                msg: plaintext,
                aad,
            },
        )
        .expect("XChaCha20-Poly1305 encrypt");
    [nonce.as_slice(), ct.as_slice()].concat()
}

pub fn aead_open(key: &[u8; 32], aad: &[u8], sealed: &[u8]) -> Result<Vec<u8>> {
    if sealed.len() < 24 + 16 {
        bail!("sealed data too short: {} bytes", sealed.len());
    }
    let (nonce, body) = sealed.split_at(24);
    XChaCha20Poly1305::new(key.into())
        .decrypt(
            nonce.into(),
            chacha20poly1305::aead::Payload { msg: body, aad },
        )
        .map_err(|_| anyhow!("authentication failed (wrong key, wrong context or tampered)"))
}

pub fn dek_open(dek: &[u8; 32], ciphertext: &[u8]) -> Result<Vec<u8>> {
    if ciphertext.len() < 24 + 16 {
        bail!("ciphertext too short: {} bytes", ciphertext.len());
    }
    let (nonce, body) = ciphertext.split_at(24);
    XChaCha20Poly1305::new(dek.into())
        .decrypt(nonce.into(), body)
        .map_err(|_| anyhow!("credential decryption failed (wrong key or tampered)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_rejections() {
        let dek = dek_generate();
        for pt in [&b""[..], b"x", &[7u8; 1000]] {
            assert_eq!(dek_open(&dek, &dek_seal(&dek, pt)).unwrap(), pt);
        }
        let mut ct = dek_seal(&dek, b"secret");
        assert!(dek_open(&dek_generate(), &ct).is_err(), "wrong key");
        let last = ct.len() - 1;
        ct[last] ^= 1;
        assert!(dek_open(&dek, &ct).is_err(), "tampered");
        assert!(dek_open(&dek, &[0u8; 39]).is_err(), "too short");
    }

    #[test]
    fn aead_binds_its_context() {
        let key = dek_generate();
        let sealed = aead_seal(&key, b"host-a|epoch-3|holder-2", b"share bytes");
        assert_eq!(
            aead_open(&key, b"host-a|epoch-3|holder-2", &sealed).unwrap(),
            b"share bytes"
        );
        assert!(
            aead_open(&key, b"host-b|epoch-3|holder-2", &sealed).is_err(),
            "moved to another credential"
        );
        assert!(
            aead_open(&key, b"host-a|epoch-4|holder-2", &sealed).is_err(),
            "moved to another epoch"
        );
        assert!(aead_open(&key, b"", &sealed[..30]).is_err());
    }

    #[test]
    fn nonces_are_fresh() {
        let dek = [9u8; 32];
        assert_ne!(dek_seal(&dek, b"a")[..24], dek_seal(&dek, b"a")[..24]);
    }
}
