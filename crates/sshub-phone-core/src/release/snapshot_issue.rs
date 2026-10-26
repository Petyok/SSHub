//! Issuing the optional short code of a release: the share sealed under a fresh 80 bit code with Argon2id (snapshot blob
//! version 2, bound to the credential). Port of `issueSnapshot` / `newSnapshotCode` of `assets/companion/src/state.mjs`.
//! All randomness comes from the caller's RNG (the proto helpers that read the OS generator are not used).

use rand_core::RngCore;
use sshub_sync_proto::codes::{crockford_encode, format_groups};
use sshub_sync_proto::crypto::{Argon2Profile, Share};
use sshub_sync_proto::limits::{SNAPSHOT_ARGON2_FLOOR, SNAPSHOT_TTL_SECS};
use sshub_sync_proto::snapshot::seal_with;

use crate::error::{CoreError, Result};
use crate::state::SnapshotRecord;

/// Used only when the floor profile cannot run on this device; the blob records it and the terminal flags it
/// (`SNAPSHOT_PROFILE_WEAK` of the page).
pub const SNAPSHOT_PROFILE_WEAK: Argon2Profile = Argon2Profile { mem_kib: 64 * 1024, iters: 8 };

/// Floor first, then the weak fallback.
pub const PROFILES: [Argon2Profile; 2] = [SNAPSHOT_ARGON2_FLOOR, SNAPSHOT_PROFILE_WEAK];

pub struct Issued {
    /// Dash grouped Crockford code, shown once and never stored: `K7Q2-9FMX-C41T-8PWD`.
    pub code: String,
    /// `SnapshotBlob::to_bytes` (version 2, the credential in the associated data).
    pub blob: Vec<u8>,
    pub record: SnapshotRecord,
}

/// Seals `share` (33 bytes, index first) for `host`. The first profile that works is used; `weak` is set when it was
/// not the first. The caller records `record` only after the reply that carries the blob was built.
pub fn issue(share: &[u8; 33], host: &[u8; 16], now: i64, rng: &mut dyn RngCore, profiles: &[Argon2Profile]) -> Result<Issued> {
    let mut code_bytes = [0u8; 10];
    let mut salt = [0u8; 16];
    let mut nonce = [0u8; 24];
    rng.fill_bytes(&mut code_bytes);
    rng.fill_bytes(&mut salt);
    rng.fill_bytes(&mut nonce);
    let code = format_groups(&crockford_encode(&code_bytes), 4);
    let share = Share { index: share[0], bytes: share[1..].to_vec() };
    let mut last = None;
    for (i, p) in profiles.iter().enumerate() {
        match seal_with(&share, &code, now, *p, salt, nonce, Some(*host)) {
            Ok(blob) => {
                let record = SnapshotRecord {
                    issued_at: now,
                    expires_at: now.saturating_add(SNAPSHOT_TTL_SECS),
                    mem_kib: p.mem_kib,
                    iters: p.iters,
                    weak: i > 0,
                };
                return Ok(Issued { code, blob: blob.to_bytes(), record });
            }
            Err(e) => last = Some(e),
        }
    }
    Err(CoreError::refused(format!(
        "no Argon2 profile fits this device: {}",
        last.map_or_else(|| "none given".to_string(), |e| format!("{e:#}"))
    )))
}
