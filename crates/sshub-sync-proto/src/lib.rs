//! Protocol core of SSHub host sync (`docs/host-sync-contract.md`): pure functions over bytes, no store,
//! no I/O, no threads, no clock. The terminal (`sshub`) and the phone app both build on it.

pub mod codes;
pub mod crypto;
pub mod enroll_core;
pub mod fountain;
pub mod limits;
pub mod media;
pub mod oplog;
pub mod serde_hex;
pub mod share_release;
pub mod snapshot;
pub mod uid;
pub mod wire;
