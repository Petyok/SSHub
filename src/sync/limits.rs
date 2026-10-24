//! Every numeric default chosen for host sync, in one place so that changing a
//! default is one diff. Reasons are in `docs/host-sync-contract.md` section 9
//! and `docs/host-sync-design.md` ("Defaults chosen at implementation").

use super::crypto::Argon2Profile;

const DAY: i64 = 86_400;

/// Reject an op whose lamport clock exceeds the highest known one by more than this.
pub const LAMPORT_JUMP_CAP: u64 = 100_000;
/// No contact for this long (or an epoch lag of one) marks a device `Behind`.
pub const BEHIND_AFTER_SECS: i64 = 14 * DAY;
/// No round with any peer for this long marks a device `Stale`.
pub const STALE_AFTER_SECS: i64 = 30 * DAY;
/// Maximum age of the phone's signed head for a delegation certificate to be usable.
pub const PHONE_FRESH_SECS: i64 = 14 * DAY;
/// Age of the phone's head at which the TUI starts to warn.
pub const PHONE_FRESH_WARN_SECS: i64 = 7 * DAY;
/// Window in which a device's self-signed head counts as "live" for recovery.
pub const LIVE_DEVICE_WINDOW_SECS: i64 = 14 * DAY;
/// Timelock of `PhoneReenroll`.
pub const RECOVERY_TIMELOCK_SECS: i64 = 7 * DAY;
/// Lifetime of a share-release challenge.
pub const CHALLENGE_EXPIRY_SECS: i64 = 120;
/// Challenge nonce length in bytes.
pub const CHALLENGE_NONCE_LEN: usize = 32;
/// Selectable delegation certificate terms.
pub const CERT_TERM_WEEK_SECS: i64 = 7 * DAY;
pub const CERT_TERM_MONTH_SECS: i64 = 30 * DAY;
pub const CERT_TERM_QUARTER_SECS: i64 = 90 * DAY;
/// Default and hard cap of a certificate term.
pub const CERT_TERM_DEFAULT_SECS: i64 = CERT_TERM_MONTH_SECS;
pub const CERT_TERM_MAX_SECS: i64 = CERT_TERM_QUARTER_SECS;
/// New hosts a certificate may open per rolling day, counted ring-wide.
pub const CERT_NEW_HOSTS_PER_DAY: u32 = 50;
/// Crockford characters of a short-code snapshot (80 bits).
pub const SNAPSHOT_CODE_CHARS: usize = 16;
/// Snapshot lifetime.
pub const SNAPSHOT_TTL_SECS: i64 = 7 * DAY;
/// Floor of the phone's snapshot Argon2 profile (the actual profile rides in the blob).
pub const SNAPSHOT_ARGON2_FLOOR: Argon2Profile = Argon2Profile {
    mem_kib: 262_144,
    iters: 4,
};
/// Master phrase stretching (laptop class machines).
pub const ARGON2_MASTER_PROFILE: Argon2Profile = Argon2Profile {
    mem_kib: 1_048_576,
    iters: 1,
};
/// Emailed code: at least 100 bits of entropy, memory buys nothing.
pub const ARGON2_EMAIL_PROFILE: Argon2Profile = Argon2Profile {
    mem_kib: 19_456,
    iters: 2,
};
/// Local device key file passphrase (normally a random key from the OS keyring).
pub const ARGON2_DEVICE_KEY_PROFILE: Argon2Profile = Argon2Profile {
    mem_kib: 65_536,
    iters: 3,
};
/// Largest memory and iteration count accepted from an untrusted blob.
pub const ARGON2_MAX_MEM_KIB: u32 = 2 * 1_048_576;
pub const ARGON2_MAX_ITERS: u32 = 16;
/// QR payload bytes per frame (error correction level M) and animation settings.
pub const QR_FRAME_PAYLOAD_BYTES: usize = 250;
/// Frames per second of the animated QR (PoC C measured the pipeline at 8; the real phone
/// measurement, C4, tunes it).
pub const QR_FPS: u32 = 8;
pub const QR_PARITY_PERCENT: u32 = 25;
/// Transport caps.
pub const MAX_FRAME_BYTES: usize = 1 << 20;
pub const MAX_JSON_DEPTH: usize = 32;
pub const MAX_OPS_PER_TAIL: usize = 2_000;
pub const HELLO_TIMEOUT_SECS: u64 = 10;
/// Client side only: hard deadline from spawning the ssh child to the peer's first frame
/// (PoC F: ssh `ConnectTimeout=4` plus a margin), so a dead or slow host fails fast.
/// `HELLO_TIMEOUT_SECS` above is the other direction: how long an established session
/// waits for the opening Hello of the other side once the connection is up.
pub const CONNECT_TO_HELLO_MS: u64 = 4_500;
pub const ROUND_TIMEOUT_SECS: u64 = 120;
pub const IDLE_TIMEOUT_SECS: u64 = 30;
/// Previous wrapping keys are dropped when every credential is confirmed under the new
/// sharing or after this long, whichever comes first.
pub const WRAP_KEY_RETIRE_AFTER_SECS: i64 = 7 * DAY;
/// Credentials per `ShareReshareBatch` or `ShareReshareConfirmBatch` op.
pub const RESHARE_ITEMS_PER_OP: usize = 200;
/// Checkpoints (docs/host-sync-design.md, "Checkpoints and pruning"): proposed after this many
/// ops or bytes since the last accepted one, at most once per `CHECKPOINT_MIN_INTERVAL_SECS`,
/// and `CHECKPOINT_AFTER_RESHARE_SECS` after a reshare finished.
pub const CHECKPOINT_OPS: u64 = 5_000;
pub const CHECKPOINT_BYTES: u64 = 8 * 1024 * 1024;
pub const CHECKPOINT_MIN_INTERVAL_SECS: i64 = DAY;
pub const CHECKPOINT_AFTER_RESHARE_SECS: i64 = 7 * DAY;
/// Pruning waits this long after the newest checkpoint is accepted.
pub const CHECKPOINT_PRUNE_DELAY_SECS: i64 = 7 * DAY;
/// Accepted checkpoints whose span stays on disk (the cut of the older one is the prune floor).
pub const CHECKPOINT_RETAIN_SPANS: usize = 2;
/// Distinct attended terminals that must sign, besides every live attended terminal.
pub const CHECKPOINT_SIGN_MIN: usize = 2;
/// State blob chunk size on the wire (well under `MAX_FRAME_BYTES`).
pub const CHECKPOINT_CHUNK_BYTES: usize = 256 * 1024;
/// Tombstones are carried in checkpoint states this long (longer than `STALE_AFTER_SECS`).
pub const TOMBSTONE_KEEP_SECS: i64 = 90 * DAY;
/// Words of the ring fingerprint (88 bits: a static artifact can be ground offline).
pub const RING_FINGERPRINT_WORDS: usize = 8;
/// Quarantined (pending) ops accepted per device before the peer is throttled.
pub const PENDING_QUARANTINE_MAX_PER_DEVICE: usize = 10_000;
/// Informational clock skew grace; wall clocks never order ops.
pub const TIMESTAMP_SKEW_GRACE_SECS: i64 = 600;
/// Word counts.
pub const SAS_WORDS: usize = 4;
pub const MASTER_PHRASE_WORDS: usize = 6;
pub const PAPER_WORDS: usize = 24;
