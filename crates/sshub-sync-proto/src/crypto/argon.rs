//! Argon2id with named profiles, so parameters never drift across files.

use anyhow::{bail, Result};
use argon2::{Algorithm, Argon2, Block, Params, Version};
use std::cell::Cell;

use crate::limits::{
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

thread_local! {
    /// Test hook: pretend the device cannot allocate an Argon2 state above this many KiB (see `set_alloc_limit_kib`).
    static ALLOC_LIMIT_KIB: Cell<Option<u32>> = const { Cell::new(None) };
}

/// Makes every Argon2 derivation of THIS thread that needs more than `limit` KiB of memory fail the way a failed
/// allocation does (an `Err`, never an abort). `None` removes the limit. For tests of the fallback profiles.
#[doc(hidden)]
pub fn set_alloc_limit_kib(limit: Option<u32>) {
    ALLOC_LIMIT_KIB.with(|l| l.set(limit));
}

/// Argon2id, one lane, 32-byte output. Fails for a profile outside the bounds, and when the memory the profile asks for
/// cannot be allocated: the state is reserved with `try_reserve_exact` and handed to argon2, so a device short of memory
/// gets an `Err` (the caller can take a smaller profile) instead of an allocation abort that kills the process.
pub fn try_argon2_derive(input: &str, salt: &[u8; 16], p: Argon2Profile) -> Result<[u8; 32]> {
    p.validate()?;
    let params = Params::new(p.mem_kib, p.iters, 1, Some(32))
        .map_err(|e| anyhow::anyhow!("argon2 params: {e}"))?;
    let blocks = params.block_count();
    if ALLOC_LIMIT_KIB
        .with(Cell::get)
        .is_some_and(|l| p.mem_kib > l)
    {
        anyhow::bail!("argon2: cannot allocate {} KiB (injected limit)", p.mem_kib);
    }
    let mut memory: Vec<Block> = Vec::new();
    memory
        .try_reserve_exact(blocks)
        .map_err(|e| anyhow::anyhow!("argon2: cannot allocate {} KiB: {e}", p.mem_kib))?;
    memory.resize(blocks, Block::new());
    let mut out = [0u8; 32];
    let r = Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into_with_memory(input.as_bytes(), salt, &mut out, &mut memory[..]);
    // the state is derived from the secret: overwrite it before it goes back to the allocator
    memory.iter_mut().for_each(|b| *b = Block::new());
    std::hint::black_box(&memory);
    r.map_err(|e| anyhow::anyhow!("argon2: {e}"))?;
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
    fn the_fallible_path_matches_the_allocating_one_and_an_injected_limit_is_an_error() {
        let p = Argon2Profile {
            mem_kib: 64,
            iters: 2,
        };
        // the same bytes as argon2's own allocating call
        let params = Params::new(p.mem_kib, p.iters, 1, Some(32)).unwrap();
        let mut want = [0u8; 32];
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
            .hash_password_into(b"pw", &[3; 16], &mut want)
            .unwrap();
        assert_eq!(try_argon2_derive("pw", &[3; 16], p).unwrap(), want);
        set_alloc_limit_kib(Some(63));
        let e = try_argon2_derive("pw", &[3; 16], p).unwrap_err();
        assert!(e.to_string().contains("cannot allocate"), "{e}");
        set_alloc_limit_kib(Some(64));
        assert_eq!(try_argon2_derive("pw", &[3; 16], p).unwrap(), want);
        set_alloc_limit_kib(None);
    }

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
