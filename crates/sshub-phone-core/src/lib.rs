//! SSHub phone core: the phone's rules in Rust, shared with the terminal (`docs/host-sync-android.md`).
//!
//! One implementation of everything the page (`assets/companion/src/*.mjs`) decides: identity and wrap generations,
//! state and head adoption, op authoring, what an approval screen shows, the release reply, trust sessions, reshare,
//! enrollment and the channels. Rust never touches the platform: bytes and `now` (unix seconds) go in, bytes and
//! views come out. The uniffi surface is `ffi` (one stateful object, no global state).

uniffi::setup_scaffolding!();

pub mod ble;
pub mod cert;
pub mod chain;
pub mod channels;
pub mod ctx;
pub mod describe;
pub mod enroll;
pub mod error;
pub mod ffi;
pub mod handover;
pub mod identity;
pub mod notices;
pub mod outgoing;
pub mod plain;
pub mod release;
pub mod reshare;
pub mod state;
pub mod text;
pub mod views;

pub use error::CoreError;
use sshub_sync_proto::crypto::{bip39_encode, chain_hash, verify, DeviceIdentity};

/// Phase 1 toolchain probe (the Android instrumented test calls it); superseded by `PhoneCore`.
#[derive(uniffi::Record)]
pub struct Probe {
    pub device_id: String,
    pub words: String,
    pub blake3_empty: String,
    pub signature_ok: bool,
}

#[uniffi::export]
pub fn probe(secret: Vec<u8>) -> Result<Probe, CoreError> {
    let secret: [u8; 32] = secret
        .try_into()
        .map_err(|_| CoreError::bad("the phone secret must be 32 bytes"))?;
    let id = DeviceIdentity::from_seed(&secret, identity::PHONE_CONTEXT);
    let public = id.public();
    let sig = id.sign(b"sshub android probe");
    Ok(Probe {
        device_id: public.device_id(),
        words: bip39_encode(&secret),
        blake3_empty: hex::encode(chain_hash(b"")),
        signature_ok: verify(&public.ed25519_pub, b"sshub android probe", &sig),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_matches_the_terminal_fixture() {
        let p = probe(vec![0x42; 32]).unwrap();
        assert_eq!(p.device_id, "ee8a1d5cf97016a7");
        assert_eq!(
            p.blake3_empty,
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
        assert!(p.signature_ok);
        assert!(probe(vec![0; 31]).is_err());
    }
}
