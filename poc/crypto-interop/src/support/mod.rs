//! Shared crypto helpers for the interop rig (both binaries link these).
//!
//! Layout mirrors the future `src/sync/crypto.rs` split: one module per
//! concern, stage-0 signatures kept recognizable.
pub mod op;
pub mod primitives;
pub mod shamir;
pub mod words;
