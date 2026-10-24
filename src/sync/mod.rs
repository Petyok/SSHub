//! Host sync (epic #13). Interface contract: `docs/host-sync-contract.md`.

pub mod apply;
pub mod cert;
pub mod client;
pub mod codes;
pub mod cred_ops;
pub mod crypto;
pub mod email;
pub mod enroll;
pub mod fountain;
pub mod identity_store;
pub mod limits;
pub mod modes;
pub mod oplog;
pub mod protocol;
pub mod quorum;
pub mod ratchet;
pub mod reshare;
#[cfg(test)]
mod phone_fixtures;
pub mod share_release;
pub mod snapshot;
pub mod transport;
#[cfg(test)]
mod vectors;
pub mod wire;
