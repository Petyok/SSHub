# Host sync: interface contract

Status: binding for every work package of epic #13. Precedence:
`docs/host-sync-design.md` (v4) > this file > stage plans. The contract is the
single source of names, signatures, table names and constants shared between
packages. Change it only by editing this file in the same commit that changes
the code.

Rust paths are under `crate::sync` (`src/sync/`), declared in `src/lib.rs` as
`pub mod sync;`. Constants live in `src/sync/limits.rs` (section 9).

## 0. Conventions and amendments in force

- **Stable ids (C1).** `pub type Uid = [u8; 16];` random from the OS CSPRNG,
  serialised as 32 lowercase hex characters in canonical form and JSON. Aliases
  `HostUid`, `GroupUid`, `IdentityUid`. Local SQLite integer ids stay local row
  keys and foreign-key targets; they never appear in an op, a frame or a
  challenge. `DeviceId = String = hex(ed25519_pub[..8])`.
- **Canonical bytes (C2, proven by PoC A claim A5).** One canonical form for ops
  and challenges, written by an explicit hand-ordered encoder (not by
  `#[derive(Serialize)]` field order), frozen by committed vectors under
  `tests/vectors/`. Form: deterministic compact JSON, fixed field order,
  `prev_hash` and every fixed 32-byte value as lowercase hex strings, every
  `Vec<u8>` as a base64 string (standard alphabet, padded; PoC B: number arrays
  triple the size of the log; the PoC A rule vectors in `tests/vectors/op-rules/`
  still show number arrays and are superseded by the final op vectors),
  integers as plain JSON number literals (u64::MAX is
  `18446744073709551615`), strings as raw UTF-8 with only the mandatory JSON
  escapes. Rule for every consumer, the phone included: hash and verify over the
  exact bytes received, never re-serialise; a consumer that must parse numbers
  uses an exact-integer parser (native `JSON.parse` rounds above 2^53). There is
  never a second format.
- **Tombstones (C3).** `HostDelete`, `GroupDelete`, `IdentityDelete` write a
  tombstone `(kind, uid, lamport, device_id, op_hash)`. Order is
  `(lamport, device_id)`. A delete beats a concurrent edit unless the edit has a
  strictly higher lamport; then the entity is resurrected AND a pending review is
  raised (resurrection with a new `address` is a dangerous change). Safe fields
  merge per field, last writer wins; dangerous fields go to review as a whole
  entity.
- **Module layout (C4).** `src/sync/crypto/` is a directory; `crypto/mod.rs`
  re-exports every item below, so `crate::sync::crypto::X` is the stable path.
- **Dangerous set (C5).** One const table in `src/sync/oplog/tier.rs`
  classifying every replicated field as `Safe`, `Dangerous` or `LocalOnly`, with
  a test that fails when a `ManagedHost`/`HostGroup`/`Identity` field is not
  classified.
  - Safe (auto-applied): `name, label, notes, tags, os_icon, favorite`.
  - Dangerous (pending review): `address, port, username, proxy_jump,
    remote_command, forward_agent, transport, environment, secret_lookup,
    has_password, identity_uid`, group membership changes, and group
    `default_proxy_jump, default_identity_uid, default_username,
    default_port, default_transport, default_forward_agent`, `parent_uid`.
  - LocalOnly: `id, created_at, updated_at, last_connected, sort_order, ping,
    session_logging, ssh_config_hash, source=ssh_config rows`, key file paths.
  - Every apply path (auto or approved) calls the existing validators:
    `reject_option_like` (`store/hosts.rs`), `ssh::destination_problem`,
    `map::hops::validate_raw`.
- **Gating seam (C6).** `credentials::QuorumGatedStore` wraps the store from
  `open_for_profile`, returns `Err(Locked)` until unlocked;
  `credentials::ensure_unlocked(&[SecretRef], &dyn Unlocker)`; TUI pre-flight
  before connect, sftp, broadcast and tunnel start; blocking CLI variant; hop
  secrets are staged to the askpass helper instead of re-read.
- **Forward-secure resharing (C14, PoC B finding B6).** A reshare keeps the same
  DEK, so old wrapped shares stay decryptable forever by a holder's long-lived
  wrap key. Order of a revocation: remaining holders `rotate_wrap_keys` and
  publish `WrapKeyRotate`; the reshare wraps to the new keys; after adoption
  (every credential confirmed by a real unlock, or `WRAP_KEY_RETIRE_AFTER_SECS`)
  each holder calls `retire_previous_wrap_keys`. `Vec<u8>` fields of ops are
  base64 strings (standard alphabet, padded) in the canonical form. Residuals:
  a key file copied before retirement, holders that cannot rotate (paper,
  a phone that has not contacted since), backups of deleted keys,
  a lying contributor (denial of service). See `poc/results/B-reshare-cost.md`.
- **Device key storage (C8).** `load_or_init_identity(&ProfilePaths, &dyn
  PasswordStore) -> Result<DeviceIdentity>`; file `<profile root>/sync/device.key`
  mode 0600, content encrypted with a random key held in the OS keyring, plain
  0600 only when the store is the `credentials.json` fallback. Headless
  `sync-serve` reads it without a prompt. Identity is per profile.
- **SyncHandle (C9).** `spawn_sync_thread` returns `SyncHandle`; `Drop` sends
  `Shutdown`, drops the command sender and joins. A disconnected event channel is
  surfaced as `SyncEvent::Error("sync worker stopped")`.
- **Tab and keys (C10).** Sync is dashboard tab index 6, key `'7'`,
  `KeyAction::TabSync`. New actions: `SyncNow` (default `y` on the Sync tab),
  `SyncReview` (`R`), `SyncTrust` (`t`), `SyncEnroll` (`t` on the hosts tab).
  All go through the seven-point `keybinds.rs` recipe and the
  `SYNC_TAB_ACTIONS` conflict test.
- **B1..B6 resolutions.** B1: `ShareReshare`/`WrappedShare`/`Op`/`DeviceRow` are
  exactly as in this file (with `alg`, role, attendance, exempt). B2:
  `DevicePublic.mlkem_pub` is mandatory everywhere. B3: `sync_cred_state` is
  created by the stage 1 migration. B4: the read helpers of section 3 are
  exported by stage 1. B5: `AppMode::TrustSessionScope` exists next to
  `TrustSessionTtl`. B6: test apps use `AppDeps { resolver, metadata, store,
  password_store }`.
- **Replicated scope.** Hosts, groups (with `default_*` and memberships),
  identities (metadata; key bytes as an encrypted credential), credentials,
  devices, tombstones. Local only: tunnels, snippets, workspaces,
  command_history, auth_events, log_bookmarks, ui_state, `source=ssh_config`
  rows. Seeded rows (Favorites group, "Default" identity) get fixed well-known
  uids.
- **Schema.** Migration v22 (`migrate_v21_to_v22`, `SCHEMA_VERSION` 21 to 22).
  Sync tables never `REFERENCES hosts(id)` (the hosts rebuild cascades); they
  reference `uid` BLOBs. `uid` columns are added with `ALTER TABLE ... ADD COLUMN`,
  backfilled in the migration, with a UNIQUE INDEX (no table rebuild).
- **Dependencies.** The RustCrypto 0.10/0.5/dalek-2 generation stays for the
  whole epic: argon2 0.5, chacha20poly1305 0.10, ed25519-dalek 2,
  x25519-dalek 2, ml-kem `=0.2.3` (pinned exactly), bip39 2, blake3 1,
  rand_core 0.6, plus direct `zeroize`, `hex`, `getrandom`/`rand`. No SMTP
  crate, no `crypto_box`, no `bincode` (C11, C13). `audio` and
  `clipboard-read` are opt-in cargo features.
- **Sealed box (C11, PoC A finding 3).** `seal_to`/`open_sealed` are
  `eph_pub(32) || nonce(24) || XChaCha20-Poly1305(key, plaintext)` with
  `key = BLAKE3.derive_key("sshub/sync/seal/v1", dh || eph_pub || recipient_pub)`,
  the same shape as the hybrid wrap. The `crypto_box` crate and the legacy
  ChaCha20 construction are not used.
- **Authenticated header (C12, PoC A finding 2).** The hybrid wrap blob is
  `"SSHW" || sig_suite || wrap_suite || eph_pub || kem_ct_len u16 BE || kem_ct ||
  nonce || ct`; everything before `ct` is AEAD associated data, so the
  algorithm tag cannot be flipped in transit.
- **Wire frames without bincode (C13).** The share response frame is
  `"SWR1" || version u16 BE || kind u8 || request_id u64 BE || len u64 BE ||
  bytes`, hand-encoded on both sides.

## 1. Stage 0: `src/sync/crypto/` (WP0a, WP0b, WP0c)

```rust
// Algorithm tag. Mandatory from the first op of the first release.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AlgTag { pub sig: SigSuite, pub wrap: WrapSuite }
#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum SigSuite  { Ed25519 }
#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum WrapSuite { X25519MlKem768 }
pub const ALG_CURRENT: AlgTag;

// Device identity: signature pair + two wrapping pairs (classical + PQ).
pub struct DeviceIdentity { /* ed25519 signing + x25519 static + ml-kem decaps */ }
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DevicePublic {          // replicated in the devices table
    pub ed25519_pub: [u8; 32],
    pub x25519_pub:  [u8; 32],
    pub mlkem_pub:   Vec<u8>,      // ML-KEM-768 encapsulation key
}
impl DeviceIdentity {
    pub fn generate() -> Self;
    pub fn public(&self) -> DevicePublic;
    pub fn sign(&self, msg: &[u8]) -> [u8; 64];
    // Sealed box (C11), only for the phone share reply (recipient lives ~3 s).
    pub fn seal_to(recipient_x25519: &[u8; 32], plaintext: &[u8]) -> Vec<u8>;
    pub fn open_sealed(&self, ciphertext: &[u8]) -> anyhow::Result<Vec<u8>>;
    pub fn to_encrypted(&self, passphrase: &str) -> Vec<u8>;
    pub fn from_encrypted(blob: &[u8], passphrase: &str) -> anyhow::Result<Self>;
    // Forward secrecy of wrapped shares (C14). Rotation returns the new public half to
    // publish; the previous generation keeps unwrapping old blobs until retired.
    pub fn rotate_wrap_keys(&mut self) -> DevicePublic;
    pub fn retire_previous_wrap_keys(&mut self) -> usize;
    pub fn wrap_key_generations(&self) -> usize;
}
pub fn verify(ed25519_pub: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> bool;

// Hybrid wrap of long-lived material. The KDF takes BOTH shared secrets.
// The alg tag travels inside the blob.
pub fn hybrid_wrap(recipient: &DevicePublic, plaintext: &[u8]) -> Vec<u8>;
pub fn hybrid_unwrap(id: &DeviceIdentity, blob: &[u8]) -> anyhow::Result<Vec<u8>>;
pub fn blob_alg(blob: &[u8]) -> anyhow::Result<AlgTag>;
// Fallible form for recipient keys that arrive from other devices.
pub fn try_hybrid_wrap(recipient: &DevicePublic, plaintext: &[u8]) -> anyhow::Result<Vec<u8>>;
pub fn device_id_of(ed25519_pub: &[u8; 32]) -> String;   // hex of the first 8 bytes
impl DevicePublic {
    pub fn validate(&self) -> anyhow::Result<()>;        // run on every key from another device
    pub fn device_id(&self) -> String;
}
pub fn derive_key(context: &str, ikm: &[u8]) -> [u8; 32];  // BLAKE3 derive_key
impl AlgTag {
    pub const fn to_bytes(self) -> [u8; 2];
    pub fn from_bytes(b: [u8; 2]) -> anyhow::Result<Self>;
}

// DEK: XChaCha20-Poly1305.
pub fn dek_generate() -> [u8; 32];
pub fn dek_seal(dek: &[u8; 32], plaintext: &[u8]) -> Vec<u8>;
pub fn dek_open(dek: &[u8; 32], ciphertext: &[u8]) -> anyhow::Result<Vec<u8>>;
// Symmetric envelope with associated data (epoch share envelopes, C14): nonce(24) || ct.
pub fn aead_seal(key: &[u8; 32], aad: &[u8], plaintext: &[u8]) -> Vec<u8>;
pub fn aead_open(key: &[u8; 32], aad: &[u8], sealed: &[u8]) -> anyhow::Result<Vec<u8>>;

// Shamir k-of-n over GF(256) on a 32-byte secret.
pub struct Share { pub index: u8, pub bytes: Vec<u8> }
pub fn shamir_split(secret: &[u8; 32], k: u8, n: u8) -> Vec<Share>;
// Explicit x coordinates (holder indices need not be 1..=n after a revocation).
pub fn shamir_split_at(secret: &[u8; 32], k: u8, indices: &[u8]) -> Vec<Share>;
pub fn shamir_combine(shares: &[Share]) -> anyhow::Result<[u8; 32]>; // requester only, in memory

// Resharing without reconstructing the secret. Reissue on holder add/revoke
// goes ONLY through it.
pub struct ReshareContribution {
    pub from: u8, pub sub_shares: Vec<(u8, Vec<u8>)>, pub commitments: Vec<Vec<u8>>,
}
pub fn reshare_contribute(my: &Share, new_indices: &[u8], k: u8) -> ReshareContribution;
pub fn reshare_verify(c: &ReshareContribution, my_index: u8) -> anyhow::Result<()>;
pub fn reshare_combine(my_index: u8, contributions: &[ReshareContribution])
    -> anyhow::Result<Share>;

// Argon2id with named profiles.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Argon2Profile { pub mem_kib: u32, pub iters: u32 }
pub const ARGON2_MASTER:   Argon2Profile;  // 1 GiB x t=1
pub const ARGON2_SNAPSHOT: Argon2Profile;  // phone default; actual profile rides in the blob
pub const ARGON2_EMAIL:    Argon2Profile;  // code has >= 100 bits, memory buys nothing
pub fn argon2_derive(input: &str, salt: &[u8; 16], p: Argon2Profile) -> [u8; 32];
// Fallible form with profile bounds, for profiles read from untrusted blobs.
pub fn try_argon2_derive(input: &str, salt: &[u8; 16], p: Argon2Profile) -> anyhow::Result<[u8; 32]>;
impl Argon2Profile { pub fn validate(&self) -> anyhow::Result<()>; }

// Vault mode: recipient derived from the master phrase AND the ring secret.
// The ring secret is 128 bits, identical across the ring, never in synced data.
pub fn vault_recipient(master_phrase: &str, ring_secret: &[u8; 16], salt: &[u8; 16])
    -> (DevicePublic, DeviceIdentity);

// Word lists. There are two and they must not be confused: 6 BIP39 words are
// 66 bits, the promise is 77.
pub fn bip39_encode(bytes: &[u8; 32]) -> String;               // 24 words + checksum
pub fn bip39_decode(phrase: &str) -> anyhow::Result<[u8; 32]>; // error names the word
pub fn eff_encode(entropy: &[u8; 10]) -> String;               // 6 EFF-7776 words (~77 bits)
pub fn eff_decode(phrase: &str) -> anyhow::Result<[u8; 10]>;

// Paper phrase yields TWO independent keys under different domain tags.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaperDomain { Unwrap, Recover }
pub fn paper_derive(phrase: &str, d: PaperDomain) -> anyhow::Result<DeviceIdentity>;

// SAS: 4 words from the handshake transcript.
pub fn sas_words(transcript_hash: &[u8; 32]) -> String;        // "harbor-globe-lantern-mint"

// BLAKE3 hash for op chains and channel binding.
pub fn chain_hash(bytes: &[u8]) -> [u8; 32];
```

Secret-bearing types (`DeviceIdentity`, `Share`, DEKs) zeroize on drop.

Frozen derivations (committed vectors in `tests/vectors/crypto/`, recomputed by
the Python oracle `tests/oracles/crypto_oracle.py`):

- `verify` is strict RFC 8032 (canonical S); browsers call noble with
  `zip215: false`.
- Identity from seed: `BLAKE3 derive_key(context, ikm)` as 128 bytes of output =
  Ed25519 seed (32) || X25519 secret (32) || ML-KEM `d` (32) || ML-KEM `z` (32).
- Paper: `ikm` = the 32 bytes of the 24 word phrase, contexts
  `sshub/sync/paper/unwrap/v1` and `sshub/sync/paper/recover/v1`.
- Vault: `stretched = Argon2id(lowercase single-spaced phrase, salt, ARGON2_MASTER)`;
  `seed = derive_key("sshub/sync/vault/seed/v1", stretched || ring_secret)`; identity
  from `seed` with context `sshub/sync/vault/v1`.
- Hybrid wrap key: `derive_key("sshub/sync/wrap/v1", x25519_ss || mlkem_ss ||
  eph_pub || recipient_x25519_pub || kem_ct)`, header as AEAD associated data (C12).
- Sealed box: C11, AEAD associated data `eph_pub || recipient_pub`.
- Device key file: `"SSHK" || 1 || mem u32 BE || iters u32 BE || salt(16) ||
  nonce(24) || ct`, header (everything before `ct`) as associated data, key =
  Argon2id(passphrase, salt, profile from the header). Plaintext, exactly:
  `ed25519 seed (32) || current x25519 secret (32) || current ML-KEM dk (2400) ||
  n u8 || n * (older x25519 secret (32) || older ML-KEM dk (2400))`, older wrap
  key generations newest first (C14). A file with no older generation has `n = 0`
  and a plaintext of 2465 bytes (32 + 32 + 2400 + 1); each older generation adds
  2432 bytes. The ML-KEM public key is not stored: it is derived from the dk.

## 2. Stage 1: `src/sync/oplog/` and schema v22 (WP1a, WP1b)

```rust
pub type DeviceId = String;
pub enum DeviceRole { Holder, Mirror }
pub enum Attendance { Attended, Unattended }   // unattended never votes at Red
pub enum CredMode   { Vault, Quorum }

pub enum OpBody {                              // canonical form: section 0 (C2)
    HostUpsert    { host_uid: HostUid, fields: HostFields },
    HostDelete    { host_uid: HostUid },
    GroupUpsert   { group_uid: GroupUid, fields: GroupFields },
    GroupDelete   { group_uid: GroupUid },
    IdentityUpsert{ identity_uid: IdentityUid, fields: IdentityFields }, // metadata only
    IdentityDelete{ identity_uid: IdentityUid },
    CredUpdate    { host_uid: HostUid, ciphertext: Vec<u8>, wrapped_shares: Vec<WrappedShare> },
    DeviceAdd     { device: DevicePublic, name: String, role: DeviceRole, attendance: Attendance, kind: DeviceKind },
    DeviceRevoke  { device_id: DeviceId },
    // requester is in the body as well as the header: a holder that checks only
    // the body must still see WHO asks.
    ShareRelease  { host_uid: HostUid, request_id: u64, requester: DeviceId },
    // Forward-secure resharing (C14, PoC B). The contributor is the op's device.
    // One per (contributor, recipient) per epoch: hybrid_wrap(to.pub, 32 byte session key).
    ShareReshareSession { epoch: u64, to: DeviceId, sealed: Vec<u8> },
    // One per holder per epoch: hybrid_wrap(own current pub, 32 byte storage key).
    ShareStorageKey { epoch: u64, sealed: Vec<u8> },
    // At most RESHARE_ITEMS_PER_OP items. Each sub-share is aead_seal(session key,
    // aad = host_uid || epoch u64 BE || from index || to index, bytes).
    ShareReshareBatch { epoch: u64, new_holders: Vec<DeviceId>, items: Vec<ReshareItem> },
    // The holder's NEW share per credential: aead_seal(storage key,
    // aad = host_uid || epoch u64 BE || own index, bytes). Replicates to every peer.
    ShareReshareConfirmBatch { epoch: u64, items: Vec<ConfirmItem> },
    // A device announces fresh wrap keys (self op, applied automatically). The signing key never rotates.
    WrapKeyRotate { device_id: DeviceId, wrap_epoch: u64, x25519_pub: [u8; 32], mlkem_pub: Vec<u8> },
    CertIssue     { device_id: DeviceId, scope: CertScope, expiry: i64,
                    max_new_hosts_per_day: u32, epoch_lo: u64, epoch_hi: u64 },
    CertRevoke    { cert_id: [u8; 32] },
    CertHostOpened{ cert_id: [u8; 32], host_uid: HostUid },
    PhoneReenroll { initiated_at: i64, signers: Vec<DeviceId>, via_paper: bool,
                    third_factor: ThirdFactor, liveness: Vec<HeadAttest> },
    PhoneReenrollCancel { reenroll_hash: [u8; 32], by_phone: bool,
                          third_factor: Option<ThirdFactor> },
    // Without a third party's ack the timelock does not start.
    RecoveryAck   { reenroll_hash: [u8; 32], seen_at: i64 },
    ModeChange    { to: CredMode, migrated_on: DeviceId, cosigner: Option<DeviceId> },
    HeadAttest    { device_id: DeviceId, seq: u64, chain_hash: [u8; 32] },
}
pub enum ThirdFactor { Paper, EmailCode }   // hardware keys are out of this version (PoC E not run)
// sealed = hybrid_wrap(recipient pub, host_uid || index || share): the credential id is inside the
// wrap and every unwrapper compares it with the credential it is acting for (confused deputy).
pub struct WrappedShare { pub recipient: DeviceId, pub alg: AlgTag, pub sealed: Vec<u8> }
pub struct ReshareItem { pub host_uid: HostUid, pub sub_shares: Vec<(DeviceId, Vec<u8>)>, pub commitments: Vec<Vec<u8>> }
pub struct ConfirmItem { pub host_uid: HostUid, pub sealed_share: Vec<u8> }

pub struct Op {
    pub device_id: DeviceId, pub seq: u64, pub lamport: u64,
    pub prev_hash: [u8; 32], pub timestamp: i64, pub epoch: u64,
    pub alg: AlgTag, pub body: OpBody, pub signature: [u8; 64],
}
impl Op {
    pub fn canonical_bytes(&self) -> Vec<u8>;   // everything except signature, deterministic
    pub fn hash(&self) -> [u8; 32];             // chain_hash(canonical_bytes)
    pub fn verify(&self, signer_pub: &[u8; 32]) -> bool;
    pub fn tier(&self) -> OpTier;
}
pub enum OpTier { Green, Yellow, Red }

impl crate::store::LauncherStore {
    pub fn sync_append_local(&self, id: &DeviceIdentity, body: OpBody) -> anyhow::Result<Op>;
    pub fn sync_chain_tail(&self, device: &DeviceId, after_seq: u64) -> anyhow::Result<Vec<Op>>;
    pub fn sync_ingest(&self, op: Op) -> anyhow::Result<IngestOutcome>;
    pub fn sync_pending(&self) -> anyhow::Result<Vec<Op>>;
    pub fn sync_resolve_pending(&self, op_hash: &[u8; 32], approve: bool) -> anyhow::Result<()>;
    pub fn sync_devices(&self) -> anyhow::Result<Vec<DeviceRow>>;
    pub fn sync_cred_mode(&self) -> anyhow::Result<CredMode>;
    // read helpers consumed by stage 3 (thin selects over stage 1 tables):
    pub fn sync_cred_ciphertext(&self, host: &HostUid) -> anyhow::Result<Option<(Vec<u8>, Vec<WrappedShare>)>>;
    pub fn sync_cert_revoked(&self, cert_id: &[u8; 32]) -> anyhow::Result<bool>;
    pub fn sync_cert_hosts_opened(&self, cert_id: &[u8; 32]) -> anyhow::Result<Vec<HostUid>>;
    pub fn sync_cert_new_hosts_since(&self, cert_id: &[u8; 32], since: i64) -> anyhow::Result<u32>;
    pub fn sync_head_attest(&self, device: &DeviceId) -> anyhow::Result<Option<HeadAttest>>;
    pub fn sync_epoch(&self, device: &DeviceId) -> anyhow::Result<u64>;
}
pub enum IngestOutcome { Applied, Pending, Rejected(String), ForkProof(Box<Op>, Box<Op>) }
pub struct DeviceRow {
    pub id: DeviceId, pub public: DevicePublic, pub name: String,
    pub role: DeviceRole, pub attendance: Attendance, pub status: DeviceStatus,
    pub exempt: bool, pub last_seen: i64, pub head_seq: u64,
    pub wrap_epoch: u64,        // generation of the current wrap keys (C14)
}
pub enum DeviceStatus { Operational, Behind, Stale, Suspect, Compromised, Revoked }
// DeviceRow also carries `kind: DeviceKind` (C15).
```

Tables created by migration v22 (all prefixed `sync_`; BLOB keys, no foreign
keys to `hosts`): `sync_ops`, `sync_chain_heads`, `sync_pending_ops`,
`sync_devices`, `sync_tombstones`, `sync_head_attests`, `sync_delegation_certs`,
`sync_cert_hosts_opened`, `sync_cred_state`, `sync_epochs`, `sync_meta`; plus
`uid BLOB` columns (UNIQUE INDEX) on `hosts`, `host_groups`, `identities`.
Exact DDL is owned by WP1a and recorded in that package's commit.

Replicated fields: `HostFields = name, label, address, port: Option<u16>,
username, proxy_jump, forward_agent: Option<bool>, remote_command, transport,
environment, os_icon, tags, notes, favorite, secret_lookup, has_password,
group_uids, identity_uid`. Hop tokens in `proxy_jump` are stored as uids and
rendered as names locally; apply rewrites them.

### 2b. Checkpoints and pruning (C15)

Design: `docs/host-sync-design.md`, section "Checkpoints and pruning". Owner: the op-log package.

```rust
pub enum DeviceKind { Terminal, Phone }   // only terminals keep a store, only they can sign checkpoints

// New ops. Tier GREEN: they never apply anything by themselves.
pub struct CutEntry { pub device_id: DeviceId, pub seq: u64, pub op_hash: [u8; 32], pub lamport: u64 }
// body variants of OpBody:
//   CheckpointPropose { cut: Vec<CutEntry> /*sorted by device_id*/, state_hash: [u8; 32],
//                       state_len: u64, prev_checkpoint: [u8; 32] /*zeros for the first*/,
//                       required: Vec<DeviceId> /*sorted: attended live terminals of the state*/ }
//   CheckpointSign    { proposal_hash: [u8; 32] }

pub struct CheckpointBundle { pub proposal: Op, pub signs: Vec<Op>, pub heads: Vec<Op>, pub state: Vec<u8> }
pub enum BootstrapTrust { RingFingerprint(String /*8 words*/), Anchor(DeviceId) }
pub enum CheckpointStatus { Proposed, Verified, Disputed, Accepted, Pruned, Void }
pub enum CheckpointVerdict {
    Matches,
    Differs { section: String, detail: String },
    MissingOps(Vec<(DeviceId, u64)>),   // ops up to the cut this device does not have yet
    Blocked(String),                    // pending ops below the cut, a recovery in flight, ...
}
pub struct PruneReport { pub ops_deleted: u64, pub field_writes_deleted: u64,
                         pub wrapped_share_blobs_dropped: u64, pub bytes_freed: u64 }

impl crate::store::LauncherStore {
    /// Folds the local state as of the cut and returns the draft; refuses (Err) while an op
    /// below the cut is pending locally, a PhoneReenroll is running, or the device is not an
    /// attended terminal. `cut` None = this device's current heads for every device.
    pub fn sync_checkpoint_build(&self, cut: Option<Vec<CutEntry>>, now: i64) -> anyhow::Result<(Vec<CutEntry>, Vec<u8> /*state*/, [u8; 32] /*state_hash*/)>;
    /// Rebuilds the state from the local store as of `proposal.cut` and compares; never trusts the blob.
    pub fn sync_checkpoint_verify(&self, proposal: &Op, state: &[u8]) -> anyhow::Result<CheckpointVerdict>;
    /// Records proposal, signs and state; flips to Accepted when the signatures cover `required`
    /// and the local verdict is Matches.
    pub fn sync_checkpoint_record(&self, bundle: &CheckpointBundle) -> anyhow::Result<CheckpointStatus>;
    pub fn sync_checkpoints(&self) -> anyhow::Result<Vec<CheckpointRow>>;
    /// Deletes what the policy allows (below the cut of the second newest accepted checkpoint,
    /// 7 days after the newest was accepted); idempotent, reclaims space.
    pub fn sync_prune(&self, now: i64) -> anyhow::Result<PruneReport>;
    /// Materialises a fresh store from a bundle after verifying hash, heads, signatures, the
    /// required set and the trust anchor; sets chain heads from the cut.
    pub fn sync_bootstrap(&self, bundle: &CheckpointBundle, trust: &BootstrapTrust) -> anyhow::Result<()>;
    /// Existing in section 2; below the prune floor it returns Err(Pruned { floor_seq }).
    pub fn sync_chain_tail(&self, device: &DeviceId, after_seq: u64) -> anyhow::Result<Vec<Op>>;
}
/// For the TUI "is this my ring" prompt: 8 BIP39 words (88 bits) of BLAKE3 over the canonical
/// `devices` section of the state.
pub fn ring_fingerprint(state: &[u8]) -> anyhow::Result<String>;
```

`DeviceAdd` and `DeviceRow` gain `kind: DeviceKind`. New tables (migration v22 or its successor):
`sync_checkpoints(proposal_hash BLOB PK, device_id TEXT, prev BLOB, state_hash BLOB, state_len
INTEGER, cut TEXT, required TEXT, status TEXT, accepted_at INTEGER, state BLOB NULL)` (the state blob
is kept for the two newest accepted checkpoints only), `sync_checkpoint_signs(proposal_hash BLOB,
device_id TEXT, sign_op_hash BLOB, PRIMARY KEY (proposal_hash, device_id))`,
`sync_prune_floor(device_id TEXT PK, seq INTEGER, op_hash BLOB)`. Head ops of the cut are never
pruned. `sync_ingest`: an op with `seq` below the floor is ignored as covered; with `seq` equal to the
floor's and a different hash it is a fork proof against the kept head op; ops of a device that
was `Stale` at the accepted checkpoint and have `seq` above its cut head enter pending whatever their fields.
Competing proposals with the same `prev_checkpoint`: lowest `(lamport, device_id)` wins, the others are void.
Honest devices auto-sign (`CheckpointSign`) only when `sync_checkpoint_verify` says `Matches`;
`Differs` marks the proposal `Disputed` and the proposer `suspect`, never `compromised`.
The state blob is canonical JSON with the C2 rules, sections and sort orders as in the design.
Constants in `limits.rs`: `CHECKPOINT_OPS`, `CHECKPOINT_BYTES`, `CHECKPOINT_MIN_INTERVAL_SECS`,
`CHECKPOINT_AFTER_RESHARE_SECS`, `CHECKPOINT_PRUNE_DELAY_SECS`, `CHECKPOINT_RETAIN_SPANS`,
`CHECKPOINT_SIGN_MIN`, `CHECKPOINT_CHUNK_BYTES`, `TOMBSTONE_KEEP_SECS`, `RING_FINGERPRINT_WORDS`.

### 2a. Stage 1 as built (WP1a, WP1b)

Binding addenda to section 2; the code in `src/sync/oplog/`, `src/sync/apply.rs`
and `src/sync/identity_store.rs` is the reference.

**Canonical form details (C2).** Header field order is `device_id, seq, lamport,
prev_hash, timestamp, epoch, alg, body`; `alg` is the two-byte wire form
`[sig, wrap]` as a JSON array (`[1,1]` today, the only number array left); the
body is externally tagged, `{"Variant":{fields in the order of section 2}}`;
uids are 32 lowercase hex characters; fixed `[u8; 32]` values (`prev_hash`,
`chain_hash`, `cert_id`, `reenroll_hash`, `ed25519_pub`, `x25519_pub`) are
lowercase hex strings; every `Vec<u8>` (`ciphertext`, `sealed`, `sealed_share`,
`mlkem_pub`, each `commitments` entry, the bytes of a `sub_shares` pair) is a
base64 string (standard alphabet, padded); `sub_shares` entries are
`["<device id>","<base64>"]` pairs; `ReshareItem` is `{host_uid, sub_shares,
commitments}`, `ConfirmItem` is `{host_uid, sealed_share}`; `ThirdFactor` is
`"Paper"`, `"EmailCode"` or `{"HardwareKey":"<device id>"}`; `CertScope` is
`{"Group":"<uid>"}` or `{"Hosts":["<uid>",...]}`; `HeadAttest` (in
`PhoneReenroll.liveness`) is `{"device_id","seq","chain_hash"}`. Decoding is
strict: `Op::from_canonical` re-encodes and refuses bytes that differ from what
the encoder would write (wrong alphabet, missing padding, uppercase hex, key
order, whitespace). An op larger than `MAX_FRAME_BYTES` is refused at ingest.
Final vectors: `tests/vectors/op/ops.json` (every body variant plus edge cases,
inputs with `Vec<u8>` as hex), reproduced by the independent oracle
`tests/oracles/op_canonical.py`; `tests/vectors/op-rules/` is the superseded PoC A
number-array set.

**C14 ops.** `ShareReshareSession`, `ShareStorageKey`, `ShareReshareBatch`,
`ShareReshareConfirmBatch` (at most `RESHARE_ITEMS_PER_OP` items, enforced at
ingest and by `sync_append_local`) and `WrapKeyRotate` are Green: cryptographic
data only, the ring ceremony around them (`DeviceRevoke`, holder changes) is
Red. Stage 1 stores them in the chain; stage 3 reads them with
`sync_chain_tail`. `WrapKeyRotate` must be signed by `device_id` itself
(`Rejected` otherwise); on effect it records the replaced and the new
generation in `sync_wrap_history` and, when `wrap_epoch` is above the stored one
and the keys validate (`DevicePublic::validate`), replaces `x25519`/`mlkem` in
`sync_devices` and sets `DeviceRow.wrap_epoch`. The signing key never changes.

**Patch fields.** `HostUpsert`, `GroupUpsert` and `IdentityUpsert` carry a patch:
absent key = untouched, explicit `null` = cleared, absent keys are omitted, key
order fixed:

- host: `name, label, address, port, username, proxy_jump, forward_agent,
  remote_command, transport, environment, os_icon, tags, notes, favorite,
  secret_lookup, has_password, group_uids, identity_uid`. `transport` is the DB
  string (`ssh`, `mosh`); `group_uids` is sorted, unique and never holds the
  Favorites uid (that is `favorite`); `tags` keep their order.
- group: `name, default_identity_uid, default_username, default_port,
  default_proxy_jump, default_transport, default_forward_agent, parent_uid`.
- identity: `name, username, has_password` (key file paths never replicate).

A hop token in `proxy_jump` or `default_proxy_jump` that names a saved launcher
host travels as `#<uid hex>`; any other hop token is a plain `[user@]host[:port]`
checked by `map::hops::validate_raw`. Apply renders `#uid` as the local host
name (`sync_hops_to_wire` does the inverse when authoring); an unresolvable hop
clears the column and keeps the register.

**Types added.** `CertScope` lives in `oplog::op` (stage 3 re-exports it, it does
not redefine it). `HeadAttest` is an unsigned claim; the self-signed form is
`fork::SignedHead { device_id, seq, hash, sig }` (signature over
`"sshub/sync/head/v1" || device_id || 0x00 || seq u64 BE || hash`).
`fork::HeadVerdict { Consistent, Ahead, Fork, Rejected }`,
`fork::ForkEvidence { Ops, OpAndHead, Heads }`, `merge::TieBreak { First, Second,
ManualReview }` and `merge::tie_break(a, b)` are exported from `oplog`.

**Extra `LauncherStore` methods** (beyond section 2): `sync_register_device(public,
name, role, attendance)` (the trust anchor written by an enrollment ceremony),
`sync_chain_heads()`, `sync_pending_reason(hash)`, `sync_record_head(&SignedHead)`,
`sync_fork_evidence(device)`, `sync_mark_suspect(device)`, `sync_state_hash()`,
`sync_projection_hash()`, `sync_projection_dump()`, `sync_hops_to_wire(str)`.
`sync_append_local` validates the body like ingest does and stores the op as
`approved` (the user made the change). `sync_chain_tail` returns at most
`MAX_OPS_PER_TAIL` ops.

**Ingest verdicts.** `Rejected` (not chained, nothing stored): seq, lamport or
epoch above `i64::MAX` or zero; unknown or compromised signer; unsupported alg;
bad signature; invalid field values (option-like names and addresses,
`destination_problem`, bad hops, port 0, unknown transport, unsorted
`group_uids`, ops on the local-only Favorites group, `ShareRelease` whose
`requester` is not the signer); duplicate; gap; `prev_hash` mismatch; lamport not
increasing along the chain; lamport above `max_known + LAMPORT_JUMP_CAP`;
quarantine full for the device. `ForkProof(stored, new)` when a valid op has the
seq of a different stored op (device becomes `compromised`, evidence kept).
Otherwise the op is chained and stored: `Applied` or `Pending`; an op signed at
or after its author's approved revocation is chained but `void` and reported as
`Rejected("void: ...")`.

**Op states** (`sync_ops.state`): `applied` (Green, automatic), `pending`,
`approved` (review or own op), `rejected` (review refused; kept so the chain
stays whole), `void`. Applied and approved ops are effective; only they feed
state. The state is a function of the op and the rest of the log, never of
arrival order. Pending reasons: `tier` (Yellow or Red, or a dangerous field
anywhere in the op: the whole op waits, its safe fields included),
`resurrection` (an edit with lamport strictly above an effective tombstone),
`revoke-conflict` (author has a pending revoke at or below the op's lamport).
An approved `DeviceRevoke` voids the target's ops with lamport at or above the
revoke's; a revoke never loses an automatic tie-break.

**Tiers** (`tier::body_tier`): Green = upserts with only safe fields,
`CertHostOpened`, `RecoveryAck`, `HeadAttest`, `WrapKeyRotate` and the four reshare
ops. Yellow = upserts touching any
dangerous (or unclassified) field, the three deletes, `CredUpdate`,
`ShareRelease`, `CertRevoke`, `PhoneReenrollCancel`,
`DeviceAdd` of a mirror. Red = `DeviceAdd` of a holder, `DeviceRevoke`,
`CertIssue`, `PhoneReenroll`, `ModeChange`. Only Green applies
without review.

**State model.** Entity state is a deterministic function of the effective ops:
per-field last writer wins by `(lamport, device_id, seq)` over `sync_fields`; an
entity is alive iff its newest effective write has a strictly higher lamport than
its newest effective tombstone. Rows are a projection written through the
existing `*_on` helpers and validators. Two live entities that want one name: the
holder of the older name write `(lamport, device_id)` keeps it, the other renders
as `name (<first 8 uid hex>)`; a local-only row (ssh_config, pre-sync) always
keeps its name. A group whose parent chain cycles loses its parent on every
device. Effects of non-entity ops (`DeviceAdd`, `CredUpdate`, certificates,
`ModeChange`, `HeadAttest`) run once when the op becomes effective and are not
undone by a later void (ring-membership ops are always reviewed by a human).

**Schema.** Tables of `migrate_v21_to_v22`: the 11 of section 2 plus
`sync_fields` (per-field write registers), `sync_entities` (cache of
liveness and base name) and `sync_wrap_history` (wrap key generations). `sync_ops` also stores `kind` and `subject`. The DDL is
`SYNC_SCHEMA` in `src/store/migrate.rs`. Uids of new rows come from an
`AFTER INSERT` trigger (`randomblob(16)`, SQLite's OS-seeded CSPRNG); the seeded
Favorites group and Default identity have fixed uids
(`FAVORITES_GROUP_UID`, `DEFAULT_IDENTITY_UID` in `src/store/uid.rs`).

**Device key (C8) as built.** `load_or_init_identity` keeps the passphrase of
`device.key` (random 256 bits, `to_encrypted`) in the profile's credential store
under `sync-device-key`: the OS keyring normally; in the `credentials.json`
fallback it sits next to the file, so the 0600 mode is the whole protection, as for
every secret there. A key file whose passphrase is missing is an error, never a
silent regeneration.

## 3. Stage 2: `src/sync/{protocol,ratchet,enroll,client}.rs` (WP2a, WP2b)

```rust
pub enum SyncMsg {
    // `checkpoint`: hash of the latest accepted checkpoint proposal (C15), if any.
    Hello { proto: u16, device_id: DeviceId, epoch: u64, checkpoint: Option<[u8; 32]> },
    Heads(Vec<HeadRef>), WantTail { device_id: DeviceId, after_seq: u64 },
    Tail(Vec<Op>),
    // The peer no longer has the ops after `after_seq`: they are below its prune floor.
    Pruned { device_id: DeviceId, floor_seq: u64 },
    // A checkpoint bundle (proposal, signs, head ops, state) is requested and sent in chunks of
    // at most CHECKPOINT_CHUNK_BYTES; the receiver reassembles by `hash` and verifies.
    WantCheckpoint { proposal_hash: [u8; 32] },
    CheckpointChunk { proposal_hash: [u8; 32], index: u32, total: u32, bytes: Vec<u8> },
    EnrollCommit([u8; 32]),
    EnrollReveal { ed25519_pub: [u8; 32], x25519_pub: [u8; 32], mlkem_pub: Vec<u8>, nonce: [u8; 32] },
    Bye,
}
pub struct HeadRef { pub device_id: DeviceId, pub seq: u64, pub hash: [u8; 32], pub sig: [u8; 64] }

pub fn run_sync_serve(store: &LauncherStore, id: &DeviceIdentity) -> anyhow::Result<()>;
pub fn run_sync_enroll(store: &LauncherStore, id: &DeviceIdentity) -> anyhow::Result<()>;
pub fn run_sync_join(store: &LauncherStore, id: &DeviceIdentity, peer: &str) -> anyhow::Result<()>;

pub enum SyncCmd { SyncNow(DeviceId), EnrollBegin(DeviceId), EnrollConfirmSas(bool), Shutdown }
pub enum SyncEvent { Progress(String), PendingArrived(usize), SasPrompt(String),
    ForkDetected(DeviceId), BehindPeer(DeviceId), Done, Error(String) }
pub struct SyncHandle { /* cmd Sender, event Receiver, JoinHandle; Drop = shutdown + join */ }
pub fn spawn_sync_thread(store: Arc<LauncherStore>, id: Arc<DeviceIdentity>) -> SyncHandle;
impl SyncHandle {
    pub fn send(&self, c: SyncCmd) -> Result<(), SendError<SyncCmd>>;
    pub fn try_recv(&self) -> Result<SyncEvent, TryRecvError>;
}

pub fn ratchet_epoch(prev_epoch_key: &[u8; 32], peer_nonce: &[u8; 32]) -> [u8; 32];
pub fn epoch_mac(epoch_key: &[u8; 32], transcript: &[u8]) -> [u8; 32];
pub fn epoch_mac_verify(epoch_key: &[u8; 32], transcript: &[u8], mac: &[u8; 32]) -> bool;
```

Transport: `ssh <peer> sshub [--profile P] sync-serve`, `BatchMode=yes`,
length-prefixed JSON frames (4-byte big-endian length), size and depth caps
from `limits.rs`, stderr separate, EOF and timeout handled. `sync-serve`,
`sync-enroll`, `sync-join` and `hub-serve` are headless branches in `main.rs`
next to `theme` (no `CliContext`, stdout carries frames only). The existing
`sshub sync` (ssh_config refresh) is unchanged.

### 3a. Stage 2 as built (WP2a): supersedes the sketch above where they differ

Code: `src/sync/{protocol,ratchet,enroll}.rs`. The `SyncMsg` shapes above were a sketch; the built set is
`Hello`, `Auth`, `Prove`, `Heads`, `WantTail`, `Tail`, `Pruned`, `WantCheckpoint`, `CheckpointChunk`,
`EndWants`, `Finish`, `Ack`, `EnrollCommit`, `EnrollReveal`, `EnrollConfirm`, `Bye`, as internally tagged
JSON (`{"t": "..."}`; byte fields hex for fixed arrays, base64 for `Vec<u8>`). The round order is
`Hello(S) Hello(C) Auth(C) Auth(S) Prove(C) Prove(S) Heads(C) Heads(S)`, the client pulls
(`WantTail`/`Tail`, `EndWants`), the server pulls, `Finish(C) Finish(S) Ack(C)`.

* `Hello { proto, min_proto, device_id, nonce, eph, checkpoint }` has NO epoch: the server speaks first
  and does not know the client yet. The pair epoch travels in `Prove { epoch: Option<u64>, mac }`
  after `Auth`. `Auth` is a device-key signature over the hello transcript.
* `Heads { heads: Vec<HeadRef{device_id, seq, hash}>, own: Option<WireHead>, gossip: Vec<WireHead>,
  forks: Vec<WireFork> }`. `WireHead` is a self-signed head `{device_id, seq, hash, sig}`; the unsigned
  `HeadRef` carries no signature (supersedes `HeadRef.sig` above).
* `Tail { device_id, ops: Vec<WireOp> }` with `WireOp { canonical: base64, sig: hex64 }`; the receiver
  decodes with `Op::from_canonical(canonical, sig)` (strict) and sorts per chain before ingest.
* Caps: `MAX_OPS_PER_TAIL`, `MAX_HEADS` 256, `MAX_GOSSIP_HEADS` 64, `MAX_FORK_PROOFS` 16,
  `MAX_WANTS_PER_PHASE` 1024, `MAX_OPS_PER_ROUND` 200000; a `Tail` frame is cut at half of `MAX_FRAME_BYTES`.
* Ratchet: pair keys are per pair of devices, stored in `sync_meta` under `pair:<device_id>`
  (`PairState { epoch, key, next, prev }`). `ratchet_epoch` and `epoch_mac` are BLAKE3 keyed hashes
  (not HMAC) with domain strings `sshub/sync/ratchet/v1` and `sshub/sync/epoch-mac/v1`. `K(0)` comes from
  the ephemeral X25519 exchange of the first authenticated round or of an enrollment. The server commits
  a step only after the client's `Ack`; the client keeps `K(epoch-1)`.
* Round functions are generic over `SyncLink` (`send(&Value)`, `recv() -> Value`, `recv_first()`),
  implemented by `transport::Conn`:
  `run_round(&mut L, &LauncherStore, &DeviceIdentity, &DeviceId) -> Result<RoundSummary>` (client),
  `run_sync_serve(&LauncherStore, &DeviceIdentity, &mut L) -> Result<RoundSummary>`.
* `RoundSummary { peer, pending_arrived, applied, rejected, duplicates, reject_reasons (max 8), received,
  sent, forks, behind, suspect, epoch_advanced, first_contact, peer_checkpoint: Option<[u8;32]>,
  needs_checkpoint: Vec<DeviceId> }`. Hard failures are `Err`; `behind` is `Ok` and no op moved.
* Enrollment: `enroll::run_sync_enroll(&mut L, store, id, name, attended, &mut dyn SasConfirm)` (existing
  device, speaks first) and `run_sync_join(..)` (joining device) return
  `EnrollOutcome { peer, peer_name, sas }`. `SasConfirm::confirm(sas, peer_name) -> bool` has a blanket
  impl for `FnMut(&str, &str) -> bool`; the caller blocks in it on the human. The existing device
  registers the joiner as `Mirror` and appends a `DeviceAdd`; the joiner registers the existing device as
  `Holder`. Both save `K(0)` as a fresh pair. Enrollment waits for a human: build its `Conn` with long
  `Limits` (idle and round), not the 30 s idle default.

## 4. Stage 3: `src/sync/{wire,quorum,share_release,snapshot,cert,recovery,modes,cred_ops}.rs` (WP3a, WP3c)

```rust
pub struct Challenge { pub op: Op, pub nonce: [u8; 32], pub expiry: i64,
    pub eph_x25519_pub: [u8; 32], pub request_id: u64,
    pub phone_material: PhoneMaterial }
// What the phone needs to recover ITS share for the credential (it stores no credential keys):
// kind 0 = the hybrid wrapped share of the credential op, plaintext host_uid || index || share;
// kind 1 = after a reshare, the envelope share (aead aad host_uid || epoch || index) plus the
// phone's ShareStorageKey blob of that epoch. The phone MUST compare the host_uid inside the
// unwrapped share (or the envelope aad) with the host_uid of the op it is asked to approve,
// otherwise a requester could pair a description for host A with the share of host B.
pub struct PhoneMaterial { pub kind: ShareKind, pub epoch: u64, pub share_blob: Vec<u8>, pub storage_blob: Vec<u8> }
impl Challenge {
    pub fn to_qr_payload(&self, frame_bytes: usize) -> Vec<Vec<u8>>;   // fountain frames, see src/sync/fountain.rs
    // H(domain_tag || proto_version_le || canonical_op || nonce || expiry_le || eph || request_id_le
    //   || H(phone_material)); domain_tag = b"sshub/sync/challenge/v1", proto_version u16.
    // WHOLE op, not body, and the exact material that was shown.
    pub fn binding_hash(&self) -> [u8; 32];
}
pub const SYNC_WIRE_V1: u16 = 1;   // frame: b"SWR1" || version u16 BE || kind u8 || body, layouts in src/sync/wire.rs
pub fn unwrap_share_response(eph_priv: &[u8; 32], ciphertext: &[u8]) -> anyhow::Result<Share>;

pub enum UnlockPath { Peer(DeviceId), PhoneAudio, PhoneReverseQr, PhoneClipboard,
                      ShortCode, FullEntry }
pub fn assemble_dek(store: &LauncherStore, host: &HostUid, my_share: Share, second: Share)
    -> anyhow::Result<[u8; 32]>;
pub fn vault_open_dek(store: &LauncherStore, host: &HostUid, master: &str, ring_secret: &[u8; 16])
    -> anyhow::Result<[u8; 32]>;
pub fn reissue_plan(store: &LauncherStore, new_holders: &[DeviceId])
    -> anyhow::Result<Vec<ReshareContribution>>;

pub enum CertScope { Group(GroupUid), Hosts(Vec<HostUid>) }   // `everything` does not exist
pub struct DelegationCert { pub id: [u8; 32], pub device_id: DeviceId, pub scope: CertScope,
    pub expiry: i64, pub max_new_hosts_per_day: u32, pub epoch_range: (u64, u64), pub sig: [u8; 64] }
pub fn cert_allows(cert: &DelegationCert, issuer_ed25519: &[u8; 32], store: &LauncherStore,
    host: &HostUid, phone_head_ts: i64, now: i64) -> anyhow::Result<()>;

pub struct SnapshotBlob { pub salt: [u8; 16], pub profile: Argon2Profile,
    pub issued_at: i64, pub sealed: Vec<u8> }
pub fn snapshot_seal(share: &Share, code: &str, now: i64) -> SnapshotBlob;
pub fn snapshot_open(blob: &SnapshotBlob, code: &str, now: i64) -> anyhow::Result<Share>;
pub fn gen_snapshot_code() -> String;      // 16 Crockford chars, "K7Q2-9FMX-C41T-8PWD"

pub fn email_fragment_seal(fragment: &[u8], code: &str) -> Vec<u8>;   // option, code >= 100 bits
pub fn email_fragment_open(blob: &[u8], code: &str) -> anyhow::Result<Vec<u8>>;

pub enum ReenrollVerdict { CancelledByPhone, CancelledByFactor, AwaitingAck,
                           NeedsMoreTime { ready_at: i64 }, Ready }
pub fn validate_phone_reenroll(store: &LauncherStore, op: &Op, now: i64)
    -> anyhow::Result<ReenrollVerdict>;
```

Invariants: `shamir_combine` is called only by the requester assembling k=2 in
memory; reissue is resharing only; the phone signs the binding hash; a release
prompt names the requester; plaintext never touches disk.

## 5. Stage 3.5: Hub

`deploy/hub/docker-compose.yml` and `sshub hub-serve` (headless branch in
`main.rs`). Role in the ring: mirror, optionally unattended holder. Hard
boundaries: no accounts, no authoritative copy, no Red-level vote; the ring
works without it. Notifications use a sendmail-compatible command from
`[sync.notify] command`, spawned by argv (no shell, no SMTP crate).

## 6. Stage 4: TUI

Sync is tab 6 (key `'7'`). Adds `AppMode::{SyncReview, SyncUnlock, SyncSas,
SyncForkAlert, TrustSessionScope, TrustSessionTtl, SyncRelease, SyncOnboarding}`,
`src/tui/screens/sync*.rs`, a sync badge beside the update badge in the tab bar,
a summary folded into the Recent/Auth column (reusing the `AUTH_PANEL` roles),
`src/app/sync.rs`, `KeyAction`s of section 0. Consumes `SyncCmd`, `SyncEvent`,
`DeviceRow`, `Challenge`, `CertScope`. Credential mode and exemption are shown
per device row, never as a banner. Colours come from the theme (`ColorRole`,
`StyleRole`), never raw `Color::Red`.

## 7. Stage 5: phone page

`assets/companion/sshub-companion.html` (built file committed; sources, vendor,
tests, `build.mjs`, `package*.json` excluded from the published crate). Consumes
only the byte contract of section 4 (`Challenge::to_qr_payload`,
`unwrap_share_response`, `SYNC_WIRE_V1`). The release fingerprint is a const in
`src/sync/companion.rs`, shown by the TUI.

## 8. Global constraints

Branch topology: child `poc/<name>` branches from `feature/host-sync`. No async
runtime; sync is a background thread plus `mpsc`; ssh to peers is `BatchMode=yes`.
Factors are bytes, not booleans. `AlgTag` in every op and every wrapped blob.
Long-lived material is wrapped hybrid (X25519 + ML-KEM-768). The wrapped share of
every recipient replicates to every peer. A credential op is atomic. Fork verdict
`compromised` only on cryptographic proof (two ops with one seq, or two
incomparable head attestations). SAS uses the commitment scheme and words.
Certificate scope is mandatory, budget 50 new hosts per rolling day ring-wide,
terms 7/30/90 days, Yellow only, n>=3. Recovery: two distinct attended devices
other than the requester OR the paper phrase; a third factor from a different
artifact; ack from a third party; 7-day timelock. The master phrase is 6 EFF-7776
words. Mode change is a Red op with an explicit migration machine. Tests run
offline; no real credentials, no real user directories.

## 9. `src/sync/limits.rs` constants

| Const | Value | Meaning |
|---|---|---|
| `LAMPORT_JUMP_CAP` | `100_000` | reject `op.lamport > max_known + cap` |
| `BEHIND_AFTER_SECS` | `14 * 86_400` | no contact (or epoch lag >= 1) means `Behind` |
| `STALE_AFTER_SECS` | `30 * 86_400` | no round with any peer means `Stale` |
| `PHONE_FRESH_SECS` | `14 * 86_400` | max age of phone head for cert use |
| `PHONE_FRESH_WARN_SECS` | `7 * 86_400` | TUI warning threshold |
| `LIVE_DEVICE_WINDOW_SECS` | `14 * 86_400` | recovery liveness window |
| `RECOVERY_TIMELOCK_SECS` | `7 * 86_400` | `PhoneReenroll` timelock |
| `CHALLENGE_EXPIRY_SECS` | `120` | challenge lifetime |
| `CHALLENGE_NONCE_LEN` | `32` | nonce bytes |
| `CERT_TERM_WEEK_SECS`, `CERT_TERM_MONTH_SECS`, `CERT_TERM_QUARTER_SECS` | 7, 30, 90 days | selectable terms |
| `CERT_TERM_DEFAULT_SECS`, `CERT_TERM_MAX_SECS` | month, quarter | default, hard cap |
| `CERT_NEW_HOSTS_PER_DAY` | `50` | ring-wide rolling budget |
| `SNAPSHOT_CODE_CHARS` | `16` | Crockford chars (80 bits) |
| `SNAPSHOT_TTL_SECS` | `7 * 86_400` | snapshot lifetime |
| `SNAPSHOT_ARGON2_FLOOR` | `{ mem_kib: 262_144, iters: 4 }` | phone profile floor |
| `ARGON2_MASTER_PROFILE` | `{ mem_kib: 1_048_576, iters: 1 }` | master phrase |
| `ARGON2_EMAIL_PROFILE` | `{ mem_kib: 19_456, iters: 2 }` | email code |
| `ARGON2_DEVICE_KEY_PROFILE` | `{ mem_kib: 65_536, iters: 3 }` | local device key file passphrase |
| `ARGON2_MAX_MEM_KIB`, `ARGON2_MAX_ITERS` | `2 * 1_048_576`, `16` | bounds for a profile read from an untrusted blob |
| `QR_FRAME_PAYLOAD_BYTES` | `250` | frame size when the terminal area is unknown (QR byte capacity at level M, version 11); the unlock modal normally uses `fountain::frame_bytes_for_area` (84 bytes full 80x24, 331 bytes 120x40) |
| `QR_FPS` | `8` | animated frame rate (PoC C measured the pipeline at 8; the real phone measurement tunes it) |
| `QR_PARITY_PERCENT` | `25` | XOR parity frames in the loop |
| `MAX_FRAME_BYTES` | `1 << 20` | transport frame cap |
| `MAX_JSON_DEPTH` | `32` | frame JSON depth cap |
| `MAX_OPS_PER_TAIL` | `2_000` | ops per `Tail` frame |
| `HELLO_TIMEOUT_SECS`, `ROUND_TIMEOUT_SECS`, `IDLE_TIMEOUT_SECS` | 10, 120, 30 | transport timeouts |
| `PENDING_QUARANTINE_MAX_PER_DEVICE` | `10_000` | quarantine cap per device |
| `TIMESTAMP_SKEW_GRACE_SECS` | `600` | informational skew grace (never used for ordering) |
| `CHECKPOINT_OPS`, `CHECKPOINT_BYTES` | `5_000`, `8 MiB` | ops or bytes since the last accepted checkpoint that trigger a proposal |
| `CHECKPOINT_MIN_INTERVAL_SECS`, `CHECKPOINT_AFTER_RESHARE_SECS` | 1 day, 7 days | minimum spacing; proposal after a finished reshare |
| `CHECKPOINT_PRUNE_DELAY_SECS`, `CHECKPOINT_RETAIN_SPANS` | 7 days, 2 | prune only this long after acceptance; spans kept on disk |
| `CHECKPOINT_SIGN_MIN` | `2` | distinct attended terminals that must sign (and every live one) |
| `CHECKPOINT_CHUNK_BYTES` | `256 KiB` | state blob chunk on the wire |
| `TOMBSTONE_KEEP_SECS` | `90 * 86_400` | tombstones carried in checkpoint states |
| `RING_FINGERPRINT_WORDS` | `8` | 88 bit ring fingerprint |
| `WRAP_KEY_RETIRE_AFTER_SECS` | `7 * 86_400` | previous wrap keys are dropped when all credentials are confirmed or after this long |
| `RESHARE_ITEMS_PER_OP` | `200` | credentials per `ShareReshareBatch` or `ShareReshareConfirmBatch` op |
| `SAS_WORDS`, `MASTER_PHRASE_WORDS`, `PAPER_WORDS` | 4, 6, 24 | word counts |

The same table is appended to `docs/host-sync-design.md` as "Defaults chosen at
implementation" by WP-DOCS.
