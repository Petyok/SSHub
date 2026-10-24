//! PoC A crypto interop: Rust half of the Rust↔JS vector rig.
//!
//! Implements the stage-0 crypto contract (`src/sync/crypto.rs` as specified in
//! `.claude/host-sync/stage-0-crypto.md`, Tasks 1-12) plus the stage-1 op
//! canonical form and the stage-3 channel-binding formula, so that
//! `emit-vectors` can freeze them into `vectors.json` for the JS side.
pub mod support;
