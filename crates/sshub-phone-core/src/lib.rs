//! SSHub phone core. Phase 1: one function proving the toolchain end to end (Rust, cargo-ndk,
//! uniffi, Kotlin, Compose test) over the unmodified sync primitives of `src/sync/`.
#![allow(dead_code, unused_imports)]

uniffi::setup_scaffolding!();

// The sync sources are compiled in place, not copied. `crate::sync::{crypto,limits}` is the path
// they use inside the `sshub` crate; the tiny `sync` module below re-creates it.
#[path = "../../../src/sync/limits.rs"]
mod limits;
#[path = "../../../src/sync/crypto/mod.rs"]
mod crypto;
mod sync {
    pub(crate) use crate::{crypto, limits};
}

use crypto::{bip39_encode, chain_hash, verify, DeviceIdentity};

/// Context of the phone identity (contract C16, `assets/companion/src/keys.mjs`).
const PHONE_CONTEXT: &str = "sshub/sync/phone/v1";

#[derive(uniffi::Record)]
pub struct Probe {
    /// `hex(ed25519_pub[..8])` of the identity derived from the secret.
    pub device_id: String,
    /// The 24 BIP39 backup words of the secret.
    pub words: String,
    /// `hex(BLAKE3(""))`, the official known answer.
    pub blake3_empty: String,
    /// The identity signed a message and the strict verifier accepted it.
    pub signature_ok: bool,
}

#[derive(Debug, uniffi::Error)]
#[uniffi(flat_error)]
pub enum CoreError {
    BadSecret,
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the phone secret must be 32 bytes")
    }
}
impl std::error::Error for CoreError {}

/// Derives the phone identity from its 32 byte secret exactly like the page and the terminal
/// fixtures do and returns values the Kotlin side can compare with `challenges.json`.
#[uniffi::export]
pub fn probe(secret: Vec<u8>) -> Result<Probe, CoreError> {
    let secret: [u8; 32] = secret.try_into().map_err(|_| CoreError::BadSecret)?;
    let id = DeviceIdentity::from_seed(&secret, PHONE_CONTEXT);
    let public = id.public();
    let sig = id.sign(b"sshub android probe");
    Ok(Probe {
        device_id: public.device_id(),
        words: bip39_encode(&secret),
        blake3_empty: hex(&chain_hash(b"")),
        signature_ok: verify(&public.ed25519_pub, b"sshub android probe", &sig),
    })
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `assets/companion/test/fixtures/challenges.json`: phone.secret_hex = 42 x 32.
    #[test]
    fn phone_vector_matches_the_terminal_fixture() {
        let p = probe(vec![0x42; 32]).unwrap();
        assert_eq!(p.device_id, "ee8a1d5cf97016a7");
        assert_eq!(p.blake3_empty, "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262");
        assert!(p.signature_ok);
        assert_eq!(p.words.split(' ').count(), 24);
    }

    #[test]
    fn short_secret_is_refused() {
        assert!(probe(vec![0; 31]).is_err());
    }
}
