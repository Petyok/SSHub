//! Host sync (epic #13). Interface contract: `docs/host-sync-contract.md`.

pub mod apply;
pub mod ceremony;
pub mod cert;
pub mod checkpoint_driver;
pub mod client;
pub mod companion;
pub mod cred_ops;
pub mod email;
pub mod enroll;
pub mod entity_capture;
pub mod exemption;
pub mod export;
pub mod handover;
pub mod health;
pub mod hub;
pub mod identity_store;
pub mod media;
pub mod modes;
pub mod notify;
pub mod oplog;
pub mod phone_enroll;
#[cfg(test)]
mod phone_fixtures;
pub mod protocol;
pub mod quorum;
pub mod ratchet;
pub mod recovery;
pub mod reshare;
pub mod revocation;
pub mod runtime;
pub mod screen;
pub mod transport;
pub mod ui_shares;
pub mod unlock_paths;
#[cfg(test)]
mod vectors;

// The pure protocol core lives in `crates/sshub-sync-proto`; every path below keeps working.
pub use sshub_sync_proto::{codes, crypto, fountain, limits, share_release, snapshot, wire};
