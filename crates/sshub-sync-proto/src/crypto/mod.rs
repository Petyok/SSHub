//! Stage 0: crypto primitives for host sync. Pure functions, no I/O, no store.
//!
//! The public paths below are the contract (`docs/host-sync-contract.md`
//! section 1); the file split is internal. Test vectors under
//! `tests/vectors/crypto/` are checked by `vectors::tests` and reproduced by the
//! Python oracle in `tests/oracles/`.

mod alg;
mod argon;
mod dek;
mod hash;
mod identity;
mod mlkem;
mod paper;
mod sas;
mod shamir;
mod vault;
mod words;
mod wrap;

#[cfg(test)]
mod vectors;

pub use alg::{AlgTag, SigSuite, WrapSuite, ALG_CURRENT};
pub use argon::{
    argon2_derive, set_alloc_limit_kib, try_argon2_derive, Argon2Profile, ARGON2_EMAIL,
    ARGON2_MASTER, ARGON2_SNAPSHOT,
};
pub use dek::{aead_open, aead_seal, dek_generate, dek_open, dek_seal};
#[doc(hidden)]
pub use hash::derive_xof;
pub use hash::{chain_hash, derive_key};
pub use identity::{device_id_of, open_sealed_with_secret, verify, DeviceIdentity, DevicePublic};
pub use paper::{paper_derive, PaperDomain};
pub use sas::{handover_words, sas_words};
#[doc(hidden)]
pub use shamir::COMBINE_CALLS;
pub use shamir::{
    reshare_combine, reshare_contribute, reshare_verify, shamir_combine, shamir_split,
    shamir_split_at, ReshareContribution, Share,
};
pub use vault::vault_recipient;
#[doc(hidden)]
pub use vault::{exempt_slot_with, vault_recipient_with};
pub use words::{bip39_decode, bip39_encode, eff_decode, eff_encode, is_bip39_word};
pub use wrap::{blob_alg, hybrid_unwrap, hybrid_wrap, try_hybrid_wrap};
