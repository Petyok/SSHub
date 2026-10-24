//! Argon2id with named profiles, so parameters never drift across files.

use anyhow::{bail, Result};
use argon2::{Algorithm, Argon2, Params, Version};

use crate::sync::limits::{
    ARGON2_EMAIL_PROFILE, ARGON2_MASTER_PROFILE, ARGON2_MAX_ITERS, ARGON2_MAX_MEM_KIB,
    SNAPSHOT_ARGON2_FLOOR,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Argon2Profile {
    pub mem_kib: u32,
    pub iters: u32,
}

/// Master phrase stretching: 1 GiB, one pass (laptop class machines, not phones).
pub const ARGON2_MASTER: Argon2Profile = ARGON2_MASTER_PROFILE;
/// Short-code snapshot floor; the profile actually used travels inside the blob.
pub const ARGON2_SNAPSHOT: Argon2Profile = SNAPSHOT_ARGON2_FLOOR;
/// Emailed code: at least 100 bits of entropy, memory buys nothing.
pub const ARGON2_EMAIL: Argon2Profile = ARGON2_EMAIL_PROFILE;

impl Argon2Profile {
    /// Bounds for a profile read from an untrusted blob (memory and time are an attack surface).
    pub fn validate(&self) -> Result<()> {
        if self.mem_kib < 8 || self.mem_kib > ARGON2_MAX_MEM_KIB {
            bail!(
                "argon2 memory {} KiB outside 8..={} KiB",
                self.mem_kib,
                ARGON2_MAX_MEM_KIB
            );
        }
        if self.iters < 1 || self.iters > ARGON2_MAX_ITERS {
            bail!(
                "argon2 iterations {} outside 1..={}",
                self.iters,
                ARGON2_MAX_ITERS
            );
        }
        Ok(())
    }
}

/// Argon2id, one lane, 32-byte output. Fails only for a profile outside the bounds.
pub fn try_argon2_derive(input: &str, salt: &[u8; 16], p: Argon2Profile) -> Result<[u8; 32]> {
    p.validate()?;
    let params = Params::new(p.mem_kib, p.iters, 1, Some(32))
        .map_err(|e| anyhow::anyhow!("argon2 params: {e}"))?;
    let mut out = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(input.as_bytes(), salt, &mut out)
        .map_err(|e| anyhow::anyhow!("argon2: {e}"))?;
    Ok(out)
}

/// As [`try_argon2_derive`] for the named profiles, which are always valid.
pub fn argon2_derive(input: &str, salt: &[u8; 16], p: Argon2Profile) -> [u8; 32] {
    try_argon2_derive(input, salt, p).expect("argon2 profile within bounds")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_profiles_are_within_bounds() {
        for p in [ARGON2_MASTER, ARGON2_SNAPSHOT, ARGON2_EMAIL] {
            p.validate().unwrap();
        }
    }

    #[test]
    fn untrusted_profiles_are_rejected() {
        assert!(Argon2Profile {
            mem_kib: 0,
            iters: 1
        }
        .validate()
        .is_err());
        assert!(Argon2Profile {
            mem_kib: u32::MAX,
            iters: 1
        }
        .validate()
        .is_err());
        assert!(Argon2Profile {
            mem_kib: 1024,
            iters: 0
        }
        .validate()
        .is_err());
        assert!(Argon2Profile {
            mem_kib: 1024,
            iters: 1_000
        }
        .validate()
        .is_err());
        assert!(try_argon2_derive(
            "x",
            &[0; 16],
            Argon2Profile {
                mem_kib: u32::MAX,
                iters: 1
            }
        )
        .is_err());
    }

    #[test]
    fn same_input_same_key_different_salt_different_key() {
        let p = Argon2Profile {
            mem_kib: 64,
            iters: 1,
        };
        let a = argon2_derive("pw", &[1; 16], p);
        assert_eq!(a, argon2_derive("pw", &[1; 16], p));
        assert_ne!(a, argon2_derive("pw", &[2; 16], p));
        assert_ne!(a, argon2_derive("pw2", &[1; 16], p));
    }
}
