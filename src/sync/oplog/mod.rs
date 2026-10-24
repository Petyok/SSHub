//! The replicated, signed op-log: types, canonical bytes, ingest, fork proof,
//! merge rules and review. The `LauncherStore::sync_*` methods live in
//! [`store_impl`].

pub mod canonical;
pub mod checkpoint;
pub mod fork;
pub mod ingest;
pub mod merge;
pub mod op;
pub mod pending;
pub mod store_impl;
pub mod tier;

pub use checkpoint::{
    ring_fingerprint, BootstrapTrust, CheckpointBundle, CheckpointRow, CheckpointStatus,
    CheckpointVerdict, PruneReport, Pruned,
};
pub use fork::{ForkEvidence, HeadVerdict, SignedHead};
pub use merge::{tie_break, TieBreak};
pub use op::*;
pub use store_impl::{DeviceRow, DeviceStatus, IngestOutcome};
pub use tier::{EntityKind, FieldClass};

pub use crate::store::{GroupUid, HostUid, IdentityUid, Uid};

#[cfg(test)]
mod tests;
#[cfg(test)]
mod vectors;
