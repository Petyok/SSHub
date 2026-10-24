//! Host sync (epic #13). Interface contract: `docs/host-sync-contract.md`.

pub mod apply;
pub mod codes;
pub mod crypto;
pub mod email;
pub mod fountain;
pub mod identity_store;
pub mod limits;
pub mod oplog;
pub mod share_release;
pub mod snapshot;
#[cfg(test)]
mod vectors;
pub mod wire;
