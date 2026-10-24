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
    fn nonces_are_fresh() {
        let dek = [9u8; 32];
        assert_ne!(dek_seal(&dek, b"a")[..24], dek_seal(&dek, b"a")[..24]);
    }
}
