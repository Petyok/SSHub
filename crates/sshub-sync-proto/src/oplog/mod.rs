//! Op types, canonical bytes and the tier table. Ingest, merge and the store live in `sshub`.

pub mod canonical;
pub mod op;
pub mod tier;

pub use op::*;
pub use tier::{EntityKind, FieldClass};

#[cfg(test)]
mod vectors;
