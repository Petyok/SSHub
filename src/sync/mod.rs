//! Host sync (epic #13). Interface contract: `docs/host-sync-contract.md`.

pub mod apply;
pub mod ceremony;
pub mod cert;
pub mod checkpoint_driver;
pub mod client;
pub mod codes;
pub mod companion;
pub mod cred_ops;
pub mod crypto;
pub mod email;
pub mod enroll;
pub mod exemption;
pub mod export;
pub mod fountain;
pub mod hub;
pub mod identity_store;
pub mod limits;
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
pub mod share_release;
pub mod snapshot;
pub mod transport;
pub mod ui_shares;
pub mod unlock_paths;
#[cfg(test)]
mod vectors;
pub mod wire;
