//! Vault mode: credentials are wrapped to ONE recipient key derived from the
//! master phrase (6 EFF words, stretched with Argon2id) mixed with the ring secret
//! (128 bits, identical on every machine of the ring, never in synced data).
//!
//! `stretched = Argon2id(normalised phrase, salt, ARGON2_MASTER)`;
//! `seed = BLAKE3 derive_key("sshub/sync/vault/seed/v1", stretched || ring_secret)`;
//! identity = `DeviceIdentity::from_seed(seed, "sshub/sync/vault/v1")`.
//! A phrase is not validated here: callers check it with `eff_decode` first.

use zeroize::Zeroizing;

use super::argon::{argon2_derive, Argon2Profile, ARGON2_MASTER};
use super::hash::derive_key;
use super::identity::{DeviceIdentity, DevicePublic};
use super::words::normalize;

const SEED_CTX: &str = "sshub/sync/vault/seed/v1";
const IDENTITY_CTX: &str = "sshub/sync/vault/v1";

pub fn vault_recipient(
    master_phrase: &str,
    ring_secret: &[u8; 16],
    salt: &[u8; 16],
) -> (DevicePublic, DeviceIdentity) {
    vault_recipient_with(master_phrase, ring_secret, salt, ARGON2_MASTER)
}

/// Same derivation under a given Argon2 profile (the cheap TEST profile for vectors).
#[doc(hidden)]
pub fn vault_recipient_with(
    master_phrase: &str,
    ring_secret: &[u8; 16],
    salt: &[u8; 16],
    profile: Argon2Profile,
) -> (DevicePublic, DeviceIdentity) {
    let stretched = Zeroizing::new(argon2_derive(&normalize(master_phrase), salt, profile));
    let ikm = Zeroizing::new([stretched.as_slice(), ring_secret.as_slice()].concat());
    let seed = Zeroizing::new(derive_key(SEED_CTX, &ikm));
    let id = DeviceIdentity::from_seed(&seed, IDENTITY_CTX);
    (id.public(), id)
}

/// The exemption slot's identity (contract 4e): the phrase stretched like the vault's, mixed with a
/// per-device binding that only the device key can produce, so it opens for someone holding both.
#[doc(hidden)]
pub fn exempt_slot_with(
    master_phrase: &str,
    device_bind: &[u8; 32],
    salt: &[u8; 16],
    profile: Argon2Profile,
) -> DeviceIdentity {
    let stretched = Zeroizing::new(argon2_derive(&normalize(master_phrase), salt, profile));
    let ikm = Zeroizing::new([stretched.as_slice(), device_bind.as_slice()].concat());
    let seed = Zeroizing::new(derive_key("sshub/sync/exempt/seed/v1", &ikm));
    DeviceIdentity::from_seed(&seed, "sshub/sync/exempt/v1")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST: Argon2Profile = Argon2Profile {
        mem_kib: 64,
        iters: 1,
    };

    #[test]
    fn phrase_ring_secret_and_salt_all_matter() {
        let base = vault_recipient_with("a b c d e f", &[1; 16], &[2; 16], TEST).0;
        assert_eq!(
            base,
            vault_recipient_with("A  B c\td e f", &[1; 16], &[2; 16], TEST).0,
            "normalised phrase"
        );
        assert_ne!(
            base,
            vault_recipient_with("a b c d e g", &[1; 16], &[2; 16], TEST).0
        );
        assert_ne!(
            base,
            vault_recipient_with("a b c d e f", &[9; 16], &[2; 16], TEST).0,
            "ring secret"
        );
        assert_ne!(
            base,
            vault_recipient_with("a b c d e f", &[1; 16], &[9; 16], TEST).0,
            "salt"
        );
    }

    #[test]
    fn the_derived_identity_unwraps_what_was_wrapped_to_its_public_key() {
        let (public, id) = vault_recipient_with("a b c d e f", &[1; 16], &[2; 16], TEST);
        let blob = crate::crypto::hybrid_wrap(&public, b"dek");
        assert_eq!(crate::crypto::hybrid_unwrap(&id, &blob).unwrap(), b"dek");
    }

    #[test]
    #[ignore = "runs the real 1 GiB Argon2id master profile"]
    fn master_profile_runs() {
        let (p, _) = vault_recipient("a b c d e f", &[1; 16], &[2; 16]);
        p.validate().unwrap();
    }
}
