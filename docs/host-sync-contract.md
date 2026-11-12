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
  (every credential confirmed by a real unlock; never on a timer)
  each holder calls `retire_previous_wrap_keys`. `Vec<u8>` fields of ops are
  base64 strings (standard alphabet, padded) in the canonical form. Residuals:
  a key file copied before retirement, holders that cannot rotate (paper,
  a phone that has not contacted since), backups of deleted keys,
  a lying contributor (denial of service). See `docs/host-sync-poc-results.md` (PoC B).
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
    // The signed body is exactly {host_uid, request_id, requester, eph_x25519_pub}: the ephemeral key
    // the grant is sealed to is part of what the requester signed. A phone MUST refuse a challenge whose
    // frame ephemeral key differs from the signed one.
    ShareRelease  { host_uid: HostUid, request_id: u64, requester: DeviceId, eph_x25519_pub: [u8; 32] },
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
`"Paper"` or `"EmailCode"`; `PhoneReenroll` ends with `paper_sig`, `null` or a
128 character lowercase hex string (the 64 byte signature); `DeviceAdd` ends with
`kind` (`"Terminal"` or `"Phone"`); `CertScope` is
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

### 2a'. Checkpoints as built (C15)

Additions to 2a for the checkpoint package; the code in
`src/sync/oplog/checkpoint.rs` is the reference.

**Op types.** `DeviceKind { Terminal, Phone }` (`DeviceAdd.kind`,
`DeviceRow.kind`, `sync_register_device(.., kind)`); `ThirdFactor` is `Paper |
EmailCode`; `RecoveryApprove { reenroll_hash }` is Red and `PhoneReenroll` carries
`paper_sig: Option<[u8; 64]>` (stage 3 validates the state machine and the
signature); `CheckpointPropose` and `CheckpointSign` are Green and inert. The
canonical form of `CutEntry` is `{device_id, seq, op_hash (hex), lamport}`.

**Multi-version writes.** Everything that is not an entity field is kept as a
write in `sync_writes(section, key, op_hash, ord, lamport, device_id, seq, value)`:
sections `device`, `revoke`, `wrap`, `cred`, `share`, `storage`, `mode`, `cert`,
`cert_revoked`, `cert_open`, `head`. The winner of a `(section, key)` is the newest
`(ord, lamport, device_id, seq)` among writes of effective ops (`ord` is the epoch,
wrap epoch or head seq where one exists). `op_hash` = 32 zero bytes marks a *base
write*: always effective; used for devices registered without an op
(`sync_register_device` writes one), for pruned winners and for bootstrap.
`sync_fields` and `sync_tombstones` follow the same rule. The cache tables of
section 2 (`sync_cred_state`, `sync_delegation_certs`, ...) are still written by
the op effects and rebuilt from the winners by a bootstrap. `sync_cred_state.wrapped_shares`
holds base64 `sealed`. Read helpers over the writes: `sync_share_envelopes(host)`
and `sync_storage_key(device)`.

**State blob.** Canonical JSON (keys sorted), sections `cut`, `devices` (key
material of the current wrap generation, role, attendance, kind, status
`operational | revoked | compromised`, `wrap_epoch`, `added_by`, `head_ts` = timestamp
of the device's op at the cut), `entities` (per entity the winning write of every
field with its `(lamport, device_id, seq)`; a deleted entity keeps its fields and a
tombstone `{lamport, device_id, ts}` until `TOMBSTONE_KEEP_SECS` before the proposal
time, then it is omitted), `epochs` (epoch of each head op), `forks` (the two
evidence blobs of every compromised device) and `writes` (every `sync_writes`
winner; a credential that was resharded has `shares: []` in its `cred` entry).
The fold is *as of the cut*: only effective ops with `seq` at or below the cut of
their device count, plus base writes. Pending ops below the cut make
`sync_checkpoint_build` fail and `sync_checkpoint_verify` answer `Blocked`.
`required` is the sorted set of devices with `kind = terminal`, `attendance =
attended`, status `operational` and `head_ts` within `LIVE_DEVICE_WINDOW_SECS` of the
proposal timestamp. `prev_checkpoint` must be the last accepted proposal hash (zeros
for the first) or the verdict is `Differs { section: "prev_checkpoint" }`.
`ring_fingerprint(state)` is 88 bits of BLAKE3 over the canonical `devices` section
as 8 BIP39 words.

**API as built** beyond 2b: `sync_checkpoint_propose(id, cut, now)` (attended
terminal only; the proposal carries timestamp `now`, which is also the clock of
the fold), `sync_checkpoint_sign(id, proposal, state)` (signs only on `Matches`;
`Differs` records the proposal as `Disputed` and the proposer as `suspect`),
`sync_checkpoint_bundle(proposal_hash)` (proposal, signs, head ops, state of a
recorded checkpoint, for serving), `sync_append_local_at(id, body, now)`.
`sync_chain_tail` returns `Err(Pruned { floor_seq })` when `after_seq + 1 <
floor_seq`.

**Recording and acceptance.** `sync_checkpoint_record` stores the proposal and the
valid `CheckpointSign` ops, runs the verdict and sets the status: `Disputed` on
`Differs`, `Proposed` on `MissingOps` or `Blocked`, `Verified` on `Matches` without
enough signatures, `Accepted` on `Matches` when every `required` device signed and at
least `CHECKPOINT_SIGN_MIN` distinct attended terminals signed. Accepting flags
every device whose head op at the cut is older than `STALE_AFTER_SECS`: its ops above
that seq enter pending with reason `stale` whatever their class. Proposals with the
same `prev_checkpoint`: the lowest `(lamport, device_id)` stays, the others become
`Void` (an accepted one is never voided).

**Pruning.** `sync_prune(now)` does nothing until two checkpoints are accepted and
the newest is `CHECKPOINT_PRUNE_DELAY_SECS` old. Then, with the cut of the older
accepted checkpoint as the floor: ops below the floor are deleted, the head op of
every device at the floor stays; the winners of the writes of the deleted ops
become base writes and every other write is dropped; tombstones are collapsed the
same way and a deleted entity past the horizon is forgotten; a resharded
credential loses its wrapped shares; old head attestations and older wrap key
generations go; only the newest state blob is kept (an older one still holds the
shares that were live then); the file is `VACUUM`ed so deleted blobs do not stay in
free pages. `PruneReport` counts deleted ops, deleted writes, dropped share blobs
and bytes. Ingest: `seq` below the floor is `Rejected("covered ...")`, `seq` equal to
the floor with another hash is a `ForkProof` against the kept head.

**Bootstrap.** `sync_bootstrap` checks the blob hash and length, the proposer and
every sign against the blob's own device table, the cut, the required set and its
signatures (all required, at least `CHECKPOINT_SIGN_MIN`), the head ops, and the
trust anchor (`RingFingerprint`, case and spacing ignored, or an `Anchor` device
that signed); it needs a store without ops. It stores the head ops as anchors
(state `void`, reason `checkpoint-anchor`), writes the base rows, rebuilds the cache
tables and the projection, sets the floor to the cut and records the checkpoint as
accepted. Tombstone state is `{lamport, device_id, ts}`: the op hash is not part of
the blob, so a pruned or bootstrapped store folds to the same bytes as one that kept
the log.

**Ceremony ops (Red ceremony, stage 3).** `RedApprove { op_hash }`, `RedPhoneSign
{ op_hash }` and `RedFactor { op_hash, factor, sig }` are Green and inert (the
ceremony code checks who authored them). `FactorRegister { factor, public, sealed,
salt, sender }` (sealed empty for Paper, at most 128 bytes for EmailCode),
`DeviceRoleChange { device_id, to }` and `BulkExport { requester, window_secs }`
(1 to 3600) are Red. `DeviceRoleChange` updates `sync_devices.role` and the
checkpoint fold (promotion and demotion). `DeviceRevoke` is Yellow when the target
is a mirror and Red when it is a holder or unknown (`merge::effective_tier`;
`Op::tier()` stays the conservative Red). `sync_resolve_pending` settles any
pending op programmatically. Canonical forms: hashes and `public` are hex, `sig`
is 128 hex characters, `salt` is 32 hex characters, `sealed` is base64.

**Checkpoint wire and driver (C6).** The hello announces `sync_checkpoint_head()`.
A `WantTail` below a peer's prune floor is answered `Pruned { device_id,
floor_seq }`; the asker then sends `WantCheckpoint { hash, offset }` and receives
`CheckpointChunk { hash, offset, total, data }` chunks of `CHECKPOINT_CHUNK_BYTES`
(`total` 0: the peer does not hold the blob; at most 64 MiB; chunks must arrive in
order). The bytes are `checkpoint::bundle_to_bytes` (proposal, signs, heads,
state), untrusted until `sync_checkpoint_accept(bytes, peer, adopt)`: with
`adopt` the bundle replaces the local log (`sync_bootstrap` rules with the peer
as `Anchor`; refused when this device has local ops above the cut), otherwise it
is recorded as a proposal to verify. Proposals known as ops without a blob are
fetched in the same phase. After every round `checkpoint_driver::after_round`
records proposals with the signs held, signs those that match the local fold
(never on an unattended device), proposes when `due` (ops, bytes, minimum
interval, a finished reshare), and prunes after the delay. A proposer keeps its
own blob; blobs of proposals in flight are kept, accepted ones only for the
newest.

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

### 2a''. Rule: static validation changes need a version gate after release

`apply::validate_body` (shapes of device ids, text limits, control and bidi characters, per-kind byte caps) is a pure
function every device runs before it chains an op; an op it refuses is not stored and the author's later seqs then
answer `gap`. While nothing is released this may tighten freely. From the first release on, a change that makes a
previously accepted op invalid MUST be gated by a protocol or op-format version (old ops stay valid under the old rule,
new rules apply to ops the author stamps with the new version), or an honest author on another build stalls its whole
chain at the stricter receivers. Loosening needs no gate.

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

### 3b. Enrollment roles and honest SAS (supersedes the enrollment lines of 3a)

* First frame of an enrollment, both ways: `EnrollHello { ring: Option<String>, screen: bool }`
  (`ring` is the founder's device id kept in `sync_meta` `ring_id`; `screen` is "a human at a tty will
  decide"). The responder speaks first as before; then `EnrollCommit`, `EnrollReveal`, `EnrollConfirm`
  unchanged (the commitment scheme and `confirm_message` bytes are untouched, the phone page is unaffected).
* `enroll::decide_role(mine_ring, their_ring, am_initiator)`: a ring on one side makes that side the
  `Inviter`; a blank side is the `Joiner`; two blank sides: the initiator invites and founds the ring
  (ring id = its device id); the same ring on both sides: a repeat ceremony, the initiator invites and no
  second `DeviceAdd` is written; two different rings: `Bye` and `Err("merging rings is not supported ...")`.
* The inviter registers the joiner as `Mirror` and appends `DeviceAdd` (once, if the joiner was not known);
  the joiner registers the inviter as `Holder` and takes over the ring id. An unattended inviter does the
  same; the op is Yellow on the other devices, pending until an attended user approves.
* `run_sync_enroll(link, store, id, name, attended, Option<&mut dyn SasConfirm>)` (the remote end, roles from
  the handshake) and `run_sync_join(..)` (the initiating end) take `None` when there is no tty: that side
  never calls a confirm, sends `screen: false` and its `Confirm { ok: true }` means "admitted on the strength
  of the authenticated ssh session". `SasConfirm::confirm(&SasPrompt { sas, peer_name, peer_has_screen })`
  must say plainly when `peer_has_screen` is false that the words prove nothing and that the peer's identity
  rests on the SSH host key.
* `EnrollOutcome { peer, peer_name, sas, role, verified_by, ring }`; `VerifiedBy::{Sas, SshHostKey}` is
  `Sas` only when both sides had a screen. The record is the replicated `DeviceAdd.verified_by` / `DeviceRow.verified_by` (op-log type
  `VerifiedBy`), shown per device on the Sync tab.
* Per verb. `sshub sync-enroll` (remote, run by `ssh P sshub sync-enroll`): responder, tty prompt only when
  stdin is a tty (`ssh -t`), else `None`. `sshub sync-join <peer>`: initiator, prompt on its tty. Hosts-tab
  `t` on P: the initiator end over `ssh P sshub sync-enroll` without a tty on P, so the TUI modal shows the
  words with `peer_has_screen = false` and the plain warning; the local machine invites when it has a ring,
  founds one when both are blank, joins P's ring when only P has one.

### 3d. Synced identity keys: a private ssh-agent (C19)

A synced identity's private key must never be written to disk on any OS (macOS has no tmpfs runtime dir,
Linux sessions without logind have none either), so the earlier `materialise_key` file is replaced:

* External `ssh` (sessions, tunnels, exec): `cred_ops::PrivateAgent` is one `ssh-agent -D -a <socket>` per
  process in a 0700 temp dir, started on the first synced key. `add_key(pem, ttl, askpass)` feeds the key over
  stdin to `ssh-add -t <ttl> -` (a passphrase through the existing askpass helper, `SSH_ASKPASS_REQUIRE=force`),
  reads the new public line back with `ssh-add -L`, and writes the **public** key file. `ssh_args(&key)` returns
  `-o IdentityAgent=<socket> -o IdentitiesOnly=yes -i <public file>`. Drop kills the agent and removes the dir.
  TTL = the unlock TTL. A missing OpenSSH client makes the unlock fail loudly with that reason.
* In-process libssh2 (SFTP): authenticate with the key bytes held in the gate, in memory
  (`userauth_pubkey_memory`), no agent, no file.
* Jump hosts: a `ProxyJump` hop must use the same `-o IdentityAgent` and `IdentitiesOnly`; the connect path
  passes them to the jump command too (TUI worker, to verify on a real two-hop run).

Who: core (me) `PrivateAgent`, `AskpassEnv`, `AgentKey` in `cred_ops.rs`; `materialise_key` is removed once the
gate has moved over. TUI worker: the gate's `insert_private_key` starts or reuses one agent and stores the
`AgentKey`; connect paths add `ssh_args`; lock/drop drops the agent; the sftp path uses memory auth.
No private key byte on disk, ever; the agent socket and public files are the only artifacts.

### 3c. Join-time anchor (C18): spec

After `EnrollConfirm` succeeds and the inviter appended `DeviceAdd` for the joiner, the inviter sends the
join anchor over the same authenticated enrollment channel and the joiner adopts it; new frames after
`EnrollConfirm`: `EnrollAnchor { index: u32, total: u32, #[b64] bytes }` (chunks of at most
`CHECKPOINT_CHUNK_BYTES`, at most `CHECKPOINT_BYTES` in all) and `EnrollAnchorDone { #[hex32] bundle_hash }`.
The bundle is the C15 checkpoint bundle format (cut = the inviter's chain heads, canonical state blob, the
inviter's signed head); it carries the signature of the inviter only and is accepted only through
`sync_join_adopt(bundle, inviter)`, which verifies the inviter's signature and that the state hash recomputes,
then adopts like `sync_bootstrap` with trust `BootstrapTrust::Enrollment(inviter)`. A bundle with one signature
is never accepted as a prune checkpoint.

Rules: (1) the joiner's own `DeviceAdd` is inside the cut and never pending on the joiner; (2) everything at or
below the cut is the starting state, not reviewed; (3) ops above the cut are reviewed as usual; (4) an op
whose author is not an accepted device is stored as pending with reason `unknown-author` (capped by
`PENDING_QUARANTINE_MAX_PER_DEVICE`) instead of being rejected, and settles when the author is admitted;
(5) `sync_pending()` returns every pending op and the review screens and `sync-review` list all of them with
`sync_pending_reason`; (6) the joiner's local hosts stay local until the user chooses to share them.
Op-log: `sync_join_anchor_build(&self, joiner: &DeviceId) -> Result<CheckpointBundle>`, `sync_join_adopt`,
the `unknown-author` reason, `BootstrapTrust::Enrollment`. Core (me): the two frames in `enroll.rs` and the
call order; TUI: the review listing and the progress text "adopting the ring (N hosts)".

### 4a. Stage 3 as built (WP3a, WP3c): supersedes the sketch above where they differ

Code: `src/sync/{quorum,reshare,cred_ops,cert,modes,recovery}.rs` (plus the phone wire frames in `wire.rs`).

* **Share indices are positions.** A sharing is a sorted list of holder ids; a holder's index is its
  1-based position. The list of the original sharing is the recipient list of the `CredUpdate` (the
  vault recipient excluded); the list of reshare epoch `e` is `new_holders` of its batch ops.
  Nothing else stores indices, so receivers compute contributor and own indices themselves.
* **Vault.** The ring salt is `BLAKE3 derive_key("sshub/sync/vault/salt/v1", ring_secret)[..16]`, so a
  new machine needs only the kit and the phrase. `vault_setup(store, phrase, ring_secret)` derives the
  vault recipient and keeps its PUBLIC key in `sync_meta` under `vault_pub`. That key is LOCAL and is
  never replicated: a replicated vault key would let one device redirect every new credential to
  itself. The ring secret is kept by the caller in the profile credential store (`sync-ring-secret`).
  The printed ring-secret sheet (poc/tui onboarding) encodes the text `sshub:ring:1:<32 lowercase hex>`
  in its QR (the hex is also printed in groups); `ring_secret_text` and `parse_ring_secret` are the only
  writer and reader of it, and `parse_ring_secret` also accepts typed hex groups. `vault_setup` takes the
  phrase and ring secret of `SyncIntent::OnboardingDone` and refuses a phrase that is not 6 EFF words.
  A vault credential is one `WrappedShare { recipient: vault id, sealed: hybrid_wrap(vault_pub,
  host_uid || DEK) }`; `vault_open_dek(store, host, phrase, ring_secret)` opens it and checks the
  credential by AEAD.
* **Quorum.** `cred_put(store, id, host, &CredPayload)` writes `CredUpdate` under the ring's mode:
  vault, or `shamir_split_at(dek, 2, 1..=n)` to `share_recipients` (holders that are not revoked or
  compromised, the device itself, plus the paper unwrap key from `paper_setup`), sorted by id.
  `CredPayload` is strict JSON `{"v":1,"password"?,"private_key"?}` sealed under the DEK.
* **Reading shares.** `own_shares(store, id, host)` returns this holder's shares newest sharing first
  (envelope shares of reshare epochs, then the original wrap); `assemble_dek(store, host, mine,
  second)` combines two and checks by opening the credential. A reshare is adopted only by a real
  unlock under it (`mark_adopted`, `unadopted(store, epoch)`); only then may previous wrap keys be retired.
* **Reshare (`reissue_plan`, `reissue_receive`).** `reissue_plan(store, id, new_holders, epoch) ->
  ReissuePlan { epoch, bodies }` replaces the sketch `reissue_plan(store, new_holders)`: it needs the
  contributor's identity. Bodies: one `ShareReshareSession` per new holder, then `ShareReshareBatch` ops
  of at most `RESHARE_ITEMS_PER_OP` items. A sub-share is `aead_seal(session key, aad, old_epoch u64 BE
  || from index || 32 bytes)` with `aad = host_uid || epoch u64 BE || contributor id || recipient id`
  (device id strings, not indices: a receiver does not know the contributor's old index before it
  decrypts). `reissue_receive` combines two contributors' sub-shares with `reshare_combine`, publishes
  a `ShareStorageKey` and a `ShareReshareConfirmBatch` (`aead_seal(storage key, host_uid || epoch ||
  own index, share)`). `shamir_combine` is never called; `COMBINE_CALLS` (per thread, test only) proves it.
  The paper recipient is offline and cannot take part: it keeps the sharing it was written for.
* **Challenge.** `Challenge { op, nonce, expiry, eph_x25519_pub, request_id, phone_material }` with
  `to_wire`, `to_qr_payload(frame_bytes) -> Result<Vec<Vec<u8>>>`, `binding_hash`, `binding_preimage`;
  `make_challenge(store, id, host, phone, now, ttl)` appends the `ShareRelease` op and returns the
  challenge and the ephemeral secret; `release_share(store, id, host, epoch, eph_pub)` seals a share
  for a requester.
* **Unlocker.** `QuorumUnlocker::new(store, id, gate, ShareSource, ring_secret)` implements
  `credentials::Unlocker`: it sets the gate's synced check (`host:<id>` / `identity:<id>` map to uids,
  synced means a `sync_cred_state` row exists) and opens credentials into the gate. `ShareSource`
  (`master_phrase()`, `second_share(host, epoch, mine)`) is implemented by whoever owns the interaction.
* **Certificates.** `DelegationCert::from_op(&Op)`; `cert_allows(cert, issuer_ed25519, store, requester,
  host, phone_head_ts, now)` adds the `requester` argument (the device that asks) and checks signature,
  issuer, delegatee, revocation, expiry and term cap, phone freshness (`PHONE_FRESH_SECS`), the phone's
  epoch range, scope (group membership or explicit list) and the ring-wide budget read from
  `sync_cert_hosts_opened`; hosts already opened are free. `validate_cert_issue` adds the three-holder rule.
* **Modes.** `mode_change_plan(store, me_public, to, cosigner, open)` returns the `ModeChange` followed
  by one `CredUpdate` per credential re-encrypted under a fresh DEK; Quorum to Vault needs a co-signer;
  `validate_mode_change` checks `migrated_on` and the co-signer. History is not erased: old log entries
  stay openable by whoever held the old shape.
* **Recovery.** `recovery::Recovery` is a pure state machine (events `Propose`, `Approve`, `Ack`,
  `CancelByPhone`, `CancelByDevice`, `Complete { now }`); verdicts add `AwaitingApproval { have, need }`
  and `Completed` to the sketch. A second `PhoneReenroll`, replayed or fresh, is refused; a backdated
  `initiated_at` is refused; approvals do not count next to a paper signature.
* **Identity keys and migration.** `materialise_key(payload, dir, identity_uid)` writes the key with
  mode 0600 (create-new) and removes it on drop; the caller chooses a tmpfs `dir`.
  `migrate_local_secrets(store, id, ungated_store)` READS `host:<id>` secrets and identity key files
  into credentials; `remove_local_copies` deletes the local entries only after the user confirmed.
* **Phone wire (C16, owned here).** Courier op lists are capped at `MAX_COURIER_OPS` (8) ops of at
  most `MAX_COURIER_OP_BYTES` (64 KiB) each, in both `PhoneOpsWire` and the reply extension.
  `ReplyExt::approved_by(phone_ed25519, binding_preimage)` is the check that ties an extension to the
  phone that was asked: replies are sealed to a public ephemeral key, so anyone can build one.

### 4c. Red ceremony (C17): spec for the op-log and the core

Design: `docs/host-sync-design.md`, "The Red ceremony". New ops, authored like any op, signed by their author:

```rust
RedApprove    { op_hash: [u8; 32] }                 // Green, inert
RedPhoneSign  { op_hash: [u8; 32] }                 // Green, inert; author must be the phone
RedFactor     { op_hash: [u8; 32], factor: ThirdFactor, sig: [u8; 64] }   // Green, inert
FactorRegister{ factor: ThirdFactor, public: [u8; 32], sealed: Vec<u8>, salt: [u8; 16],
                sender: Option<DeviceId> }          // Red: sealed is empty for Paper, <= 128 bytes for EmailCode
DeviceRoleChange { device_id: DeviceId, to: DeviceRole }   // Red
BulkExport    { requester: DeviceId, window_secs: u32 }    // Red, window <= 3600
```

Already Red and gated by the ceremony: `DeviceAdd` with `role: Holder`, `DeviceRevoke` of a holder (of a mirror it
stays Yellow), `CertIssue`, `ModeChange`. `PhoneReenroll` keeps its own rules (4a, recovery). The op-log gives the
new bodies canonical bytes, vectors and oracle checks; `apply` does nothing for the three proof ops.

Rules, evaluated by `ceremony::red_status(store, op, now)` over the log (core, not op-log):

* Proofs count only if they name `op.hash()`, and were written within `RED_CEREMONY_TTL_SECS` of
  its timestamp (no lamport order: naming `op.hash()` already puts a proof after the op, and a phone enrolled on another terminal cannot adopt the lamport of an author it never met; a completion lamport is clamped to the op's lamport + 1) (the AUTHORS' stamps; never the local receipt time and never `now`: D0b, a verdict is a pure function of the
  log, so a device that was away longer than the window reaches the verdict of the others), and their author is valid: `RedApprove` by an attended `Holder` `Terminal` that is not the proposer
  (and, for `ModeChange` to Vault, is the named `cosigner`); `RedPhoneSign` by the device with `kind == Phone`
  (not needed for `CertIssue`, whose author is the phone); `RedFactor` whose `sig` verifies under the registered
  factor key over `b"sshub/sync/red/factor/v1" || op_hash || proposer id`.
* Factor keys come from the newest applied `FactorRegister` per factor kind: `Paper` carries the `paper/recover`
  ed25519 public key; `EmailCode` carries the `email` ed25519 public key and the sealed seed
  (`aead_seal(Argon2id(code, salt), seed)`, `ARGON2_EMAIL`), `sender` names the device that holds the code. A first
  `FactorRegister` needs phone + second-device proofs only; a replacement also needs a `RedFactor` of the factor
  being replaced.
* A ring with one attended `Holder` terminal waives the second-device proof; it needs the phone and the paper. The subject
  of a promotion (`DeviceAdd` or `DeviceRoleChange` as holder) is not one of the terminals counted for that waiver, so a
  replacement terminal in a one-terminal ring completes with the phone and the paper (Phase B, crypto M1).
* `DeviceRoleChange` and `DeviceAdd` as holder additionally require at least two attended holder terminals in the ring when a
  Quorum credential exists (counted from the device view at the op's own position, never from contact: a verdict is the same
  on every device). A third live holder (the phone included) and a `ModeChange` to Quorum, whatever the ring size, require a
  registered paper (`FactorRegister{Paper}` in force at the op's position); without it the op is blocked.
* `DeviceRevoke` of a holder needs the phone and, for a terminal, the PAPER: the emailed code never counts for it, and
  the second-device proof is waived when the paper is among the proofs. It is blocked when the target is the last attended
  terminal (read from the view only, never from whether a credential exists) and, for a terminal, when no paper is
  registered. `revocation::revoke_mode` reports `Full` (two or more other attended terminals), `Degraded` (one) or
  `Refused`; a degraded revocation reshares nothing (it needs two share-holding terminals) and leaves the credentials the
  removed device held a share of on the exposure list (`quorum::exposed_credentials`). A revocation closes only when two
  distinct share-holding terminals wrote `ShareReshareBatch` ops of one epoch that leave the target out, or when no
  credential is shared with the target; `next_actions` offers `Contribute` only to a share-holding terminal and only when
  two exist. Whether a contributor is reachable now is advice for the screens (`ceremony::log_live`), not a rule.
* Recoveries are bound to their phone (H1): a cancellation by the phone is judged against the phone of the view at the
  cancellation's own lamport, and a completion `DeviceRevoke` stands in for the ceremony only when it targets the phone of
  the view at the recovery's own lamport and that phone is still live just before the completion. (The `Superseded` verdict
  for an in-force `PhoneHandover` comes with the `PhoneHandover` op.)
* One-terminal recovery (design 5.2): with no other attended holder terminal the requester's own `RecoveryAck` starts the
  lock (`RECOVERY_TIMELOCK_SOLO_SECS`, 14 days), the paper authorizes it, and the paper again is the third factor
  (`ThirdFactor::Paper` with `via_paper`); the live phone still cancels with no factor. `recovery::open_for_alert` lists
  the open recoveries started by other devices (one alert per requester is the caller's job; the alert writes no op).
* `RedStatus { missing: Vec<RedProof>, have: Vec<RedProof>, complete: bool, expired: bool }` is what the review
  screen shows ("phone: signed, second device: waiting, paper: missing"). `expired` is display and hygiene only: the window
  passed on THIS clock and the ceremony is not complete; it decides nothing. `settle_red_ops(store, now)` runs after
  every ingest: it resolves a pending Red op as approved when complete and as rejected when a rule blocks it. An op past
  the window stays pending (it is not rejected), so a late proof still completes it on every device alike. Residual: a
  signer can stamp its own proof inside the window; a proof still needs three independent participants.
* Builders for the UI and the phone: `red_approve_body(op)`, `red_factor_paper(recover_identity, proposer, op)`,
  `red_factor_email(code, store, proposer, op)`, and for the phone `RedPhoneSign` is encoded by the page like its
  cancel op; the terminal shows a Red notice (the op as kind-3 notice frame) for the phone to scan or paste.
* The email sender (`hub::announce` family) mails the code only when the op has its phone and second-device
  proofs and only once per op; the message states its purpose and nothing else.
* `BulkExport`: once complete, each holder releases shares for any host to `requester` until
  `op.timestamp + window_secs` without a prompt per host (like a certificate, recorded the same way).

### 4c-2. Trust-session certificates on the phone (P1) and the phone in resharing

* **CertIssue.** The phone is the AUTHOR and signer of the op; the user's choices reach it as a kind 9
  `CertRequestWire` (`device_id`, scope, `expiry`, `max_new_hosts_per_day` 1..=50, `epoch_lo`, `epoch_hi`) that the
  terminal shows (`cert::cert_request`). The phone shows the request, and on approval builds
  `CertIssue { device_id, scope, expiry, max_new_hosts_per_day, epoch_lo, epoch_hi }` with the fields of the request
  UNCHANGED (`CertRequestWire::to_body`), chains it on its own chain (header as for its `HeadAttest` ops) and returns it
  in a kind 5 list. Limits the phone enforces before signing: `expiry - now` between 1 s and 90 days, budget 1..=50,
  at most 64 hosts, scope never empty, no wildcard (there is none in the type).
* **Attaching the ceremony.** `CertIssue` is a Red op proposed by the phone, so its phone proof is the op itself. The
  second proof is a `RedApprove` by an attended Holder terminal that is neither the phone nor the delegate; the third
  is a `RedFactor` (paper or email) bound to `op_hash` and the proposer (the phone's device id). Both are authored on
  terminals; the phone does nothing more. The cert is valid for `cert_allows` once `settle_red_ops` applied it.
* **CertRevoke** `{ cert_id }` (Yellow): written by the phone or by any attended holder; peers stop honouring the
  certificate on their next sync. The phone page signs it like its cancel op.
* **R3b daily budget extension.** New op `CertExtend { cert_id: [u8;32], extra_hosts: u32 }` (Green, inert), authored
  by the phone that issued the certificate. It adds `extra_hosts` to the certificate's budget for the rolling day
  that contains its timestamp (`max_new_hosts_per_day + sum of extensions in the last 24 hours`); limits:
  `extra_hosts` between 1 and `CERT_NEW_HOSTS_PER_DAY`, at most one extension per certificate per rolling 24 hours,
  none after expiry; it never changes scope or expiry. A holder that refuses for the budget answers
  `ShareDeferred` with the reason; the terminal then shows a `CertExtendRequest` (wire kind 10:
  `cert_id(32) || extra_hosts u32 BE`) for the phone to confirm and sign.
* **The phone in resharing.** The phone is a RECEIVER only: it never contributes (it keeps no shares), so
  `reissue_plan` needs only terminal holders as contributors. It receives its `ShareReshareSession` and the
  epoch's `ShareReshareBatch` ops in a kind 5 list, unwraps its session keys with its wrap key, combines two
  contributors' sub-shares and publishes `ShareStorageKey` and `ShareReshareConfirmBatch` exactly as a terminal does
  (`reshare::reissue_receive`). Consequence for the rules: the contributor count is the number of remaining TERMINAL
  holders, so a revocation needs at least two remaining terminal holders (a ring of two terminals and a phone cannot
  revoke a terminal by resharing; the paper is offline and cannot contribute either).

### 4e. Mixed-mode exemption (C10)

Design: "The mixed-mode exemption". New op for the op-log worker:

```rust
ExemptionSet { device_id: DeviceId, slot: Option<DevicePublic> }   // Red, gated by the ceremony (4c)
```

`slot: Some(public)` enables, `None` disables. Effect: the projection sets `sync_devices.exempt` to 1 / 0 for
`device_id` (the Sync tab reads `DeviceRow.exempt` and prints "exemption: master phrase" on that row); the op
is otherwise inert and the core reads the newest `ExemptionSet` per device from the log. `slot.mlkem_pub` must
validate like any `DevicePublic`.

Core (this package, after the op lands):

* `exemption::slot_identity(id, phrase, profile) -> DeviceIdentity` as in the design; `enable_body(id, phrase)`
  builds `ExemptionSet { Some(slot public) }` on the exempt machine, `disable_body(device)` the `None` form.
* Ring mode must be Quorum; the device must be an attended holder terminal; `ceremony::red_status` adds the rule that
  the second approver is not `device_id`.
* `quorum::share_recipients` adds the slot of every active exemption (not revoked device) as a recipient of new
  credentials; `reshare` treats slots like the paper: permanent recipients, sub-shares hybrid-wrapped to the
  slot key, never a session key; `revocation::next_actions` contributes when the set of permanent recipients
  differs from the newest reshare's holder set (enable or disable) without a rotation step.
* Unlock: `QuorumUnlocker` on an exempt device with the master phrase (`ShareSource::master_phrase`) derives the
  slot identity and uses `own_shares(store, &slot, host)` at the same epoch as its own share as the second share,
  asking no peer. Wrong phrase: no slot share unwraps, the normal paths continue.
* The UI offers enabling only after showing the alternatives (trust session, Vault), and shows the label per row.

### 4d. Builders and screening for the UI (C4, C7)

* `recovery::start_body(store, me, now, paper_recover: Option<&DeviceIdentity>, third_factor)` builds the
  `PhoneReenroll` body (liveness set, `paper_sig` when the paper phrase was typed); `approve_body(op)`,
  `ack_body(op, now)`, `cancel_body(op, by_phone, factor)` build the votes; `unacknowledged(store, me, now)` lists
  open recoveries this attended device has not acknowledged (the UI shows each, then appends `ack_body`; an
  unattended device gets none); `validate_phone_reenroll(store, op, now)` is the status for the screen;
  `completion_bodies(store, op, now, new_phone_public, name)` yields `DeviceRevoke` of the old phone and
  `DeviceAdd` of the new one once the clock ran out; `clear_snapshots(store)` forgets the old phone's
  short-code blobs. The caller then reshares (`reshare` / the revocation driver).
* `screen::screen_pending(store, now)` runs after every `run_round` / `run_sync_serve` (ending badly or not):
  a pending `PhoneReenroll`, `ModeChange` or `CertIssue` that its validator refuses is rejected at once with the
  reason; the others stay pending for review.
* `cert::cert_request(store, delegate, scope, term_secs, budget, epoch_slack, now) -> CertRequestWire` is what the
  terminal shows the phone (wire kind 9, `CertRequestWire::{encode, decode, to_body}`, layout in `wire.rs`);
  the phone shows it, builds and signs `CertIssue` itself and returns it as a courier op / phone-ops frame.
  `cert::cert_revoke_body(&cert)` revokes (Yellow).
* A device records itself (role, attendance) in its own `sync_devices` when it enrolls, so `quorum::is_unattended`
  and `recovery::unacknowledged` know an unattended device (the Hub) is one. Attendance is a property of the device,
  not of the ssh session: it is attended unless `[sync] unattended = true` is in its config (`runtime::device_attended`,
  read by `sync-enroll` and by the joining side), the ring is told at enrollment, and an unattended device is never a
  required checkpoint signer and never votes.

### 4b. Second-share paths (how a terminal gets the share it does not hold)

`ShareSource::second_share(host, epoch, mine)` is answered by one of five paths. Which ring device is
the phone: the unique non-revoked device whose `DeviceRow.kind` is `Phone` (`quorum::phone_device(store)`).

**Peer, trust session (synchronous, inside an authenticated session).** After `Prove` the client may send
`ShareRequest { host_uid, request_id, eph_x25519_pub, epoch }` instead of `Heads`; the holder's `sync-serve`
answers `ShareGranted { request_id, sealed }` (`sealed` = `DeviceIdentity::seal_to(eph, share bytes)`, 105
bytes) or `ShareDeferred { request_id, reason }`, then both sides finish the round as usual. It grants only
when this device holds the share, the requester holds a non-revoked `CertIssue` from the phone for it, and
`cert_allows` passes (scope, expiry, phone freshness, ring-wide budget); a new host is recorded with a
`CertHostOpened` op. A holder that is attended or unattended is treated the same on this path: the
certificate IS the consent. `Deferred` sends the requester to the next path. Built: client
`protocol::run_share_request(conn, store, id, holder, &ShareAsk { host_uid, epoch, eph, request_id }) ->
ShareReply::{Granted(sealed), Deferred(reason)}`; holder side inside `run_sync_serve` via
`quorum::trust_grant`.

**Peer, attended (asynchronous, through the op-log).** The requester appends `ShareRelease { host_uid,
request_id, requester, eph_x25519_pub }` (the op gains the ephemeral key; Yellow, so it shows up in the
holder's review modal naming the requester on its next sync). On approve the holder's TUI appends one
`ShareGrant { host_uid, request_id, requester, epoch, sealed }` per share epoch it holds (newest first, at
most 3; Green, inert data) with `sealed` as above; the requester finds it on its next sync
(`quorum::peer_request`, `quorum::grant_bodies`, `quorum::peer_poll`; `ShareRelease` carries `eph_x25519_pub`). The grant is sealed to the EPHEMERAL key, never to the
requester's wrap key: a grant wrapped to a long-lived key would let one stolen requester device open the
credential forever. The ephemeral secret lives in the requester's memory only; if sshub exits the request
is made again. An unattended holder releases only on the trust-session path. There is no `sync-release`
verb; the command line takes part through two existing commands instead. **Requester:** the CLI unlocker
(`cli::unlock::PromptShares`, used by `host connect`, `host resolve` and the other blocking commands) calls
`peer_request`, then loops `sync_with` each reachable attended holder and `peer_poll` until a grant arrives or
`SSHUB_UNLOCK_WAIT` seconds (default 120, 0 = do not ask) pass. **Holder:** `sshub sync-review --approve <id>` on a
`ShareRelease` runs `cli::sync_cmd::approve_release`: it refuses a request older than
`limits::CHALLENGE_EXPIRY_SECS`, appends the `grant_bodies` and resolves the op. The explicit approval is that
command (the listing names the requester, its full device id and the request age, the TUI modal shows the
same). The phone and trust-session paths are not tried from the CLI.

**Phone reply (QR, clipboard, audio).** The input is the bytes of a `ShareResponseWire`. The clipboard text
is the base64 of exactly those bytes (standard alphabet, padded, whitespace ignored), what the page copies
(948 characters in the e2e). `unlock_paths::open_phone_reply` / `open_phone_reply_clipboard` check the
request id, open the share with the challenge's ephemeral secret and, when the reply carries the courier
extension, require the phone's signature over `Challenge::binding_preimage()`: an extension that does not
verify refuses the whole reply. The audio reply is the share only. `apply_reply_ext` chains the courier ops
and keeps the snapshot blob.

**Short code.** The 16 Crockford characters (case, spaces, hyphens and look-alike letters tolerated) open
the snapshot blob this terminal stored from an earlier reply extension. Blobs are kept in `sync_meta` under
`snapshot:<host uid hex>`: local only, never replicated, replaced by a newer one, expired after
`SNAPSHOT_TTL_SECS` (`unlock_paths::share_from_code`).

**Full entry.** The user types the 32 share bytes as 24 BIP39 words (the words the phone page shows). The
share index is not typed: it is the holder's 1-based position in the sorted holder list of that credential's
sharing at that epoch (`unlock_paths::full_entry_share`). A wrong word fails the BIP39 checksum; a wrong
but valid phrase fails the credential's AEAD at `assemble_dek`.

## 5. Stage 3.5: Hub

`deploy/hub/docker-compose.yml`. There is NO `hub-serve` verb and no daemon: the Hub is an ordinary ring
device reached with `ssh hub sshub sync-serve` behind the container's sshd. What makes it a Hub is
enforced in `src/sync/hub.rs`, `quorum::release_share_for` and the recovery state machine (see
`docs/host-sync-poc-results.md`, Stage 3.5): unattended devices neither approve nor acknowledge a recovery, an unattended holder
releases only under a valid certificate, and `hub::run_sync_serve_notify` announces recovery ops (once per
op, never with a code). Role in the ring: mirror, optionally unattended holder. Hard
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
`unwrap_share_response`, `SYNC_WIRE_V1`) and the phone frames of C16 below. The release
fingerprint is a const in `src/sync/companion.rs`, shown by the TUI. Fingerprint = lowercase
hex sha256 of the built file with its own 64 hex characters replaced by 64 zeros (a file
cannot hold its own hash); the page shows it, the build writes it to
`assets/companion/FINGERPRINT`, and the About screen says how to recompute it. It detects a wrong
or outdated build, not a malicious page (a substituted page can print any string; that residual is
already in the design).

### C21: native Android companion

A second phone implementation next to the page (design: `docs/host-sync-android.md`). It adds no frame kind, no channel and no
wire field: everything in this section is a rule the app obeys so that a terminal cannot tell, and does not need to tell, which phone
it talks to. The page stays the iOS and fallback phone; where this section says "the phone" it means both.

- **Identity.** Same secret, same derivation: 32 random bytes, `BLAKE3 derive_key("sshub/sync/phone/v1", secret)` as 128 bytes, wrap generations
  `"sshub/sync/phone/wrap/v1"`, the 24 BIP39 words are the backup. The same words on the page and in the app are the SAME device.
  The at-rest form is NOT part of the contract: the page keeps its `"SSHP"` PIN blob; the app wraps the secret with an Android Keystore
  AES-256-GCM key (StrongBox when present) under per-use biometric or device-credential authentication. The terminal never sees either.
- **One signer at a time.** A restored phone (page or app) has `head_pending` and signs nothing until it has taken its own chain from a terminal (kind 5 list:
  every op by this key, every signature valid, one chain, continue after the newest: `seq + 1`, `lamport` above every lamport seen, `epoch`
  the highest seen) or the user confirmed that the key never signed anything. Migrating page to app is a restore; the user MUST stop using the
  page copy first (clear its site data). A second copy that signs after the first took over produces two ops with the same `seq`: fork evidence, handled as any fork.
- **Same refusals, same bytes.** The app applies every rule of C16 and of the companion guide unchanged: strict canonical parsing of received
  op bytes (never re-serialised), screen built from the bytes, challenge age and expiry limits (120 s plus 600 s skew), requester typed-id for an
  unmet device, binding words from the signed material, trust-session limits (1 s to 90 days, budget 1..=50, 1..=64 hosts, epoch range starting at the
  phone's own epoch and at most one wide, no self-request), extension rules (own certificate, not revoked or expired, `extra_hosts` 1..=50, 24 hours
  apart), courier tail of at most 8 ops, snapshot blob header version 2 and one live blob per credential. The Rust implementation
  (`sshub-phone-core`) and the page are pinned by the SAME fixtures (`assets/companion/test/fixtures/*.json`, `tests/vectors/`): for equal inputs
  they MUST emit byte-identical ops, frames and replies (Ed25519 and the canonical encoders are deterministic; fresh randomness is the only difference).
- **Channels.** The app carries the frames of kinds 1 to 10 over: camera QR (fountain frames, same size and parity as the page), QR display, base64
  text (clipboard, share sheet, paste), FSK audio (send only; no microphone permission), and Bluetooth LE as the C20 **central** implemented natively
  (same service and characteristic UUIDs, same messages, same limits, `startNotifications` on TX before the first write to RX). A native central
  knows its MTU: its HELLO `chunk` is `min(negotiated MTU - 3, 512)`; the terminal's rule `min(proposal, 244, notification MTU - 3)` is unchanged. The
  app never accepts-all in a scan: it filters on the service UUID and lists `SSHub-<TAG>` for the person to pick.
- **No network.** The app declares no `INTERNET` permission (the native counterpart of the page's `connect-src 'none'`); the permissions are
  `CAMERA`, `BLUETOOTH_SCAN` (`neverForLocation`), `BLUETOOTH_CONNECT`, `USE_BIOMETRIC` and `HIDE_OVERLAY_WINDOWS` (normal, API 31: hides overlays drawn over the window).
- **App fingerprint.** `sha256` of the DER signing certificate of the release APK, lowercase hex (64 characters), as `apksigner verify --print-certs`
  prints it and as the app shows it on its About screen. The terminal ships `COMPANION_APK_CERT_SHA256` (`src/sync/companion.rs`, from
  `assets/companion/APK_CERT_SHA256`) and shows it beside the page fingerprint with the same compare hint, and in `sync-status` JSON as
  `companion_apk_cert_sha256`. It detects an app signed by someone else; like the page fingerprint it does not detect a malicious build signed by the
  same key. The public debug certificate is never listed.
- **Shared code.** The primitives the terminal and the phone core use are one crate, `sshub-sync-proto` (crypto, canonical ops, wire, fountain,
  snapshot, share math, FSK and QR decode, the BLE link layer). Moving a module there changes no byte of this contract.

### C16: frames and replies only the phone uses

Additive to `SYNC_WIRE_V1` (frame kinds 1 and 2 are unchanged). Rust: `wire.rs`
(`RecoveryNoticeWire`, `PhoneOpsWire`, `EnrollFrame`) and `share_release.rs` (`ReplyExt`,
`unwrap_share_reply`); JS: `assets/companion/src/frames.mjs`. `unwrap_share_response` still returns
the share and ignores an extension.

```text
kind 3 Red notice      (terminal to phone; any Red op, PhoneReenroll included) = op_len u32 BE || op_canonical || op_signature(64)
kind 5 ops list        (BOTH directions) = n u8 || n * (op_len u32 BE || op_canonical || signature(64)) [|| 0x01 || sender(8)]
                       phone to terminal: the phone's own ops (no sender block); terminal to phone: the epoch's
                       ShareReshareSession addressed to the phone and that epoch's ShareReshareBatch ops (it combines
                       them, see 4c), or the phone's own newest ops for a restored phone (head adoption, below)
kind 6 enroll commit   = role u8 || commit(32)
kind 7 enroll reveal   = role u8 || reveal bytes (enroll.rs Reveal::bytes, opaque to the wire)
kind 8 enroll confirm  = role u8 || ok u8 || signature(64)       role 0 = joining (phone), 1 = existing terminal

share reply plaintext (inside the C11 sealed box of the kind 2 frame):
  share(33)                                      audio channel: nothing else fits a 2 to 15 second burst
  share(33) || 0x01 || approval_sig(64) || n u8 || n * (op_len u32 BE || op || sig(64))
            || snap u8 [|| host_uid(16) || blob_len u16 BE || snapshot blob]       reverse QR and clipboard
```

- `approval_sig` is Ed25519 by the phone over the binding preimage of the challenge (section 4).
- Kind 5 sender block (C-M2). A terminal that renders a kind 5 list for the phone appends `0x01 || sender(8)`, its own
  device id (the raw 8 bytes of the 16 hex characters). A frame without the block still decodes (every frame written before
  it, and every phone export); a truncated block (tag plus fewer than 8 bytes), a longer one, or another tag after the list
  is refused. Rust: `PhoneOpsWire.sender: Option<[u8; 8]>`; JS: `encodePhoneOps(ops, sender)`, `decodePhoneOps(frame).sender`
  (null when absent). **The sender id is UNSIGNED, on purpose.** A phone restored from its 24 words has an empty ring, so it
  cannot check a terminal's signature, and a signature it could not verify would only look like proof. The id exists to tell
  two lists apart (an honest terminal that is behind on sync, or one animated QR scanned twice), not to prove who made a
  list: a forger can name any id, and a phone cannot tell. Head adoption of a restored phone (`adopt_own_chain`,
  `adoptOwnChain`) therefore needs the SAME newest op (seq and hash) from two DIFFERENT sender ids; a list without a sender
  id can be the first candidate but never completes a pair (and never costs the first candidate its id); the same id twice
  is refused with "This list comes from the same terminal as the first one. Scan the list from another terminal."; the
  same terminal with a newer head is simply a new candidate. After two lists agree, accepting the head is bound to the
  (seq, hash) the user was shown (`accept_head`, `acceptHead(state, shown)`): if the proposal changed, it is refused. The
  terminal side is `sshub sync-phone-chain <phone-id>` and the TUI entry "Show a restored phone's own ops" (the newest at
  most 8 ops of that phone this terminal holds, naming this terminal). Still true: two terminals that are both behind look
  right; compare with the newest seq of the phone on a terminal.
- Courier: the ops are the newest (at most 8) of the phone's own chain, each approval appends one
  `HeadAttest { device_id: phone, seq: previous own seq, chain_hash: previous own op hash }`
  (seq 0 and 32 zero bytes before the first op) with a fresh timestamp, `lamport` above every
  lamport seen, `epoch` the highest seen. A recovery cancellation is a `PhoneReenrollCancel` with
  `by_phone: true`, `third_factor: null`, `reenroll_hash` = BLAKE3 of the `PhoneReenroll` op bytes,
  delivered in a kind 5 frame. The terminal that receives them ingests them like any other ops.
- Snapshot blob header version 2 (written by the page since review round 1): `"SSHN" || 2 || mem u32 BE || iters u32 BE ||
  issued_at i64 BE || salt(16) || host_uid(16) || nonce(24) || ct`; everything before the nonce, `host_uid` included, is the AEAD
  associated data, so a blob names the credential it opens and a terminal can refuse a second live blob for one `host_uid`
  (OWED on the Rust side: `snapshot.rs` still reads version 1 only; `phone_fixtures.rs` opens version 2 with a local shim).
  Version 1 stays readable.
- The snapshot blob is the contract `SnapshotBlob::to_bytes` and `host_uid` names the credential it
  opens; the phone issues one per credential only while none is live (TTL 7 days recorded in the
  blob), the user can declare a blob dead (never delivered, or DEK reissued).
- Kind 5 list limits: at most 8 ops, each at most 256 KiB (a 200 item `ShareReshareBatch` for a few holders is
  about 100 KiB), at most 1 MiB in all.
- Trust sessions on the page (4c-2, `assets/companion/src/cert.mjs`): kind 9 and kind 10 are scanned or pasted, shown from
  their bytes, and signed on the phone's own chain. Before signing it refuses a term that is not 1 s to 90 days, a budget
  outside 1..=50, a host list outside 1..=64, an inverted epoch range and a request that names the phone itself; a
  `CertExtend` is refused for an unknown (not issued by this phone), revoked or expired certificate, for `extra_hosts`
  outside 1..=50, and when this phone already extended that certificate less than 24 hours ago. The phone reads what it
  issued back from its own chain (`CertIssue` hash = certificate id), so there is no second record. The page e2e writes
  its chain to `cert-out.json` and `phone_fixtures::e2e_the_pages_trust_session_ops_validate_on_the_terminal` validates
  it with `validate_cert_issue`, `cert_allows` and `extension_budget`.
- Residual: the phone's wrap-key generations are derived from its secret and the generation number, so an old
  generation can always be re-derived: unlike a terminal it cannot really retire previous wrap keys, and the
  forward secrecy of C14 does not extend to the phone's own envelopes (it stays in the B6 residual list).
- Manual entry: the phone shows `index` and the 24 BIP39 words of the 32 share bytes.
- Phone identity: 32 random bytes are the secret; `BLAKE3 derive_key("sshub/sync/phone/v1", secret)`
  as 128 bytes = the usual identity split (section 1); the 24 BIP39 words are the backup. At rest
  the secret is `"SSHP" || 1 || mem u32 BE || iters u32 BE || salt(16) || nonce(24) || ct` under
  Argon2id of the PIN (64 MiB x 3 by default), header as AAD.
- Binding words: the phone prints `sas_words(binding_hash)` of the challenge it is about to approve;
  a terminal that shows the same four words lets the person compare before approving.
- Enrollment over QR runs the commit, reveal, SAS and confirm steps of `enroll.rs` with the same
  byte layouts; the phone is the joining side (role 0), the terminal commits first.

### C20: Bluetooth Low Energy frame channel (opt-in feature `bluetooth`)

A fifth way to carry the frames the QR, audio and clipboard channels already carry. The terminal is the GATT
peripheral (server); the phone page is the central, through Web Bluetooth (Chrome on Android;
`navigator.bluetooth.requestDevice` needs a user gesture and shows the browser's device chooser, which keeps
the "the person is physically there" property). The channel moves whole C16/section 4 frames (kinds 1 to 10,
`"SWR1" || version u16 BE || kind u8 || body`) unchanged and in both directions. **The link adds no security
and none is assumed**: no pairing, no bonding, no encrypted characteristics required, no BLE-level identity.
Every frame is already signed or sealed end to end, exactly as when it crosses a camera or a speaker; a
listener on the link learns what a person looking at the QR learns. The page keeps `connect-src 'none'` (Web
Bluetooth is not a network API) and needs no other CSP change. Rust: `src/sync/media/` (`ble_link.rs` the
pure chunking and session state machine, `ble.rs` the BlueZ and CoreBluetooth transports); JS: the page
worker against this section.

**GATT layout.** One primary service. All UUIDs are fixed (no per-session UUID: the chooser filter needs a
constant).

| Item | UUID | Properties | Direction |
|---|---|---|---|
| Service | `b62f7d3f-0001-42dd-8371-6ffd96411583` | primary | |
| RX | `b62f7d3f-0002-42dd-8371-6ffd96411583` | write, write without response | phone to terminal |
| TX | `b62f7d3f-0003-42dd-8371-6ffd96411583` | notify | terminal to phone |
| INFO | `b62f7d3f-0004-42dd-8371-6ffd96411583` | read | `proto u8 (1) || max_frame u32 BE || tag(4 ASCII)` |

Nothing else is exported by the application (BlueZ adds Generic Access and Generic Attribute, which the page
does not list in `optionalServices` and therefore cannot read). The page calls
`requestDevice({ filters: [{ services: [SERVICE] }] })`, never `acceptAllDevices`, connects,
`getPrimaryService`, reads INFO, then `startNotifications()` on TX **before** its first write to RX.

**Advertising: what the terminal says, and what it must not.** While the unlock or enrollment modal that
armed the channel is open, and at most `BLE_ADVERTISE_SECS` (300) per arming, the terminal advertises the
service UUID (128 bit) and the local name `SSHub-<TAG>`: `TAG` is 4 characters of the Crockford base32
alphabet (no `I L O U`), drawn from the OS RNG when the channel is armed (20 bits), never derived from
anything stable. The terminal shows the name and the tag on the screen next to the QR ("Bluetooth: pick
SSHub-7QK2 in the phone's list") so the user picks the right entry in Chrome's chooser among whatever else
is in the room. The name MUST NOT contain the host name, the user name, a device name, a credential or a
counter; there is no manufacturer data, no TX power, no appearance, no service data. The advertised name
replaces the adapter alias in the advertisement; BlueZ's Generic Access device name (the machine's alias)
stays unreadable to the page and MUST NOT be requested. Not hideable: the controller's address is on the
air while advertising (Linux and macOS may use a private address; the channel does not rely on it). The tag
is a pointer for the person, not a secret and not authentication: a nearby device may advertise the same
name and is, at worst, a denial of service (it can connect first and send nothing useful; every frame it
sends fails the same checks a forged QR fails). One central at a time: advertising stops at the first
connection; if that connection ends (BYE, disconnect, or `BLE_HELLO_SECS` without HELLO) the terminal
advertises again with the SAME tag until the arming window ends. The TUI offers a key to drop the current
central.

**Messages.** Both directions use one message layout, one message per GATT write or notification. First
byte = type, big-endian integers.

```text
0x01 HELLO      version u8 (1) || chunk u16 BE      phone proposes its message size
0x02 HELLO_ACK  version u8 (1) || chunk u16 BE || window u8
0x10 DATA       frame_id u16 BE || idx u16 BE || count u16 BE
                    [idx = 0 only: len u32 BE || crc32 u32 BE] || payload
0x11 ACK        frame_id u16 BE || next u16 BE      all chunks below `next` arrived; next = count = done
0x12 ABORT      frame_id u16 BE || reason u8        1 crc, 2 too large, 3 busy, 4 order, 5 version, 6 timeout
0x20 BYE        reason u8                           0 done, 1 timeout, 2 user, 3 error, 4 busy
0x22 PING                                           no body, either side, no reply
```

- **Session start.** Until HELLO_ACK every message in either direction is at most 20 bytes (a link at the
  minimum ATT MTU 23 carries 20). The phone sends HELLO with `chunk` = the message size it will use
  (recommended 244, never above 512). The terminal answers HELLO_ACK with `chunk` = min(proposal, 244,
  its notification MTU minus 3), and `window` = 8. Both sides then use messages of at most that `chunk`
  bytes, DATA header included. `version` mismatch: the terminal answers ABORT(frame 0, reason 5) and BYE.
  The page cannot read the ATT MTU; if a write fails it MAY send HELLO again with a smaller proposal, which
  restarts the session state: the terminal forgets a half-received frame, queues its own frame in flight again
  under a new `frame_id` and answers a new HELLO_ACK. The page MUST treat a HELLO_ACK that arrives while the
  session is open the same way (forget the half-received frame, queue its own frame in flight again under a
  new `frame_id`).
- **Frames.** A frame (kinds 1 to 10, `len >= 7`) is cut into DATA chunks. `frame_id` starts at 1 for each
  sender and each direction and wraps after 65535 to 1. One frame in flight per direction. Chunk 0 carries
  `len` and `crc32` (IEEE 802.3, over the whole frame) after the 7 byte header, so its payload is `chunk - 15`
  bytes; chunks 1 and up have `chunk - 7` payload bytes; `count = 1 + ceil(max(0, len - (chunk - 15)) / (chunk
  - 7))`. The last chunk is as short as needed.
- **Receiver rules** (the terminal enforces them on what the phone sends; the page SHOULD enforce them on what
  the terminal sends): `len` greater than `BLE_MAX_FRAME_BYTES` (1 MiB, equal to `MAX_FRAME_BYTES`) or not
  matching `count`: ABORT(2) before buffering anything; a chunk whose `frame_id` is not the one in progress while
  one is in progress and not complete: ABORT(3); `idx` other than the next expected (gaps, duplicates): drop
  it and re-send ACK(next); reassembled length different from `len` or CRC mismatch: ABORT(1) and the sender
  may send the frame again under a new `frame_id`; memory held per direction is one frame, released on ABORT,
  BYE or disconnect. The first four bytes of a completed frame MUST be `"SWR1"` and its kind in 1..=10, else
  ABORT(3), before anything is handed to the application.
- **Flow control.** The sender may have `window` chunks unacknowledged. The receiver sends ACK(next) after
  every `window / 2` chunks and after the last chunk of a frame. The page awaits each GATT write promise before
  the next (`writeValueWithoutResponse` preferred, `writeValueWithResponse` fallback). A sender that sees no
  ACK for `BLE_ACK_SECS` (5) re-sends from the last acknowledged chunk, at most `BLE_RETRIES` (3) times, then
  ABORT(6) and BYE(1). An ACK may acknowledge chunks sent before a go-back: a sender accepts any ACK up to the
  highest chunk it has sent. An ACK(next) equal to the last acknowledged `next` while chunks are outstanding
  means the receiver saw a gap (or a repeat): the sender goes back to `next` once and waits for progress before
  doing so again. A chunk of a frame the receiver already completed is answered with ACK(frame id, count) again
  and delivers nothing twice.
- **Timing.** HELLO must arrive within `BLE_HELLO_SECS` (10) of the connection or the terminal ends it. A
  frame must complete within `BLE_FRAME_SECS` (120). PING every `BLE_PING_SECS` (30) while idle; nothing heard
  for `BLE_IDLE_SECS` (90) ends the session with BYE(1). The channel lives exactly as long as the modal that
  armed it: closing the modal sends BYE(2), disconnects and stops advertising; the phone page BYE(0) after
  its last step.
- **Who sends what.** The terminal pushes the frame of the step it is on as soon as HELLO_ACK is sent, and
  again whenever the step changes (the same document the QR shows); the phone sends its frames when its
  user has acted. The application above the channel never sees chunks, only whole frames, and treats a
  frame from BLE exactly like the same bytes read from a QR or heard over audio.

**Which ceremony steps use it** (each is the same frame as on the other channels, no new frame kind):

| Step | Terminal to phone | Phone to terminal |
|---|---|---|
| Share release (unlock modal, CLI phone verbs with `--ble`) | challenge, kind 1 | `ShareResponseWire`, kind 2, the full reply with courier ops and snapshot (no audio size limit) |
| Enrollment of the phone | commit/reveal/confirm of the existing side, kinds 6 to 8 | its reveal and confirm, kinds 6 to 8 |
| Red notice and recovery | Red notice, kind 3 | phone ops, kind 5 (courier) |
| Trust-session certificate | request, kind 9; budget extension, kind 10 | phone ops with the `CertIssue` or `CertExtend`, kind 5 |

What stays outside the channel: the SAS words (the person compares the screens), the binding words, the paper
phrase and the PIN. Nothing that needs a human decision is skipped because the link is convenient.

**Limits and platforms.** Web Bluetooth exists in Chrome and Chromium based browsers on Android, ChromeOS,
Linux, macOS and Windows; not in Safari or any iOS browser (those users keep QR, audio and paste), and not in
Firefox. Range is a few metres and is the point: it is a short-range, line-of-room channel, not a network.
Typical throughput through Chrome on Android is a few KB per second to a few tens of KB per second; the
largest honest frame (a reply with 8 courier ops) takes seconds. Terminal side: Linux through BlueZ (D-Bus,
needs `bluetoothd` running and an adapter that can advertise as a peripheral; no root, the user's session bus
only), macOS through CoreBluetooth (peripheral manager; the first use asks the user for Bluetooth permission
for the terminal app). Windows terminals: the feature builds, the channel reports that it is not implemented.

**Constants** (in `src/sync/media/ble_link.rs`, same names as above): `BLE_ADVERTISE_SECS` 300,
`BLE_HELLO_SECS` 10, `BLE_ACK_SECS` 5, `BLE_RETRIES` 3, `BLE_FRAME_SECS` 120, `BLE_PING_SECS` 30,
`BLE_IDLE_SECS` 90, `BLE_MAX_FRAME_BYTES` 1 MiB, `BLE_CHUNK_DEFAULT` 244, `BLE_CHUNK_MIN` 20,
`BLE_CHUNK_MAX` 512, `BLE_WINDOW` 8.

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
| phone page `QR_FRAME_BYTES`, `QR_FPS` | `106`, `4` | the companion page's reply QR (phone screen to the terminal's camera) uses QR version 6 level M frames (1.4 mm modules on a 68 mm screen), as wide as the screen with an integer number of device pixels per module, at 4 fps; the fountain format is the same (the decoder takes the block size from each frame), only the frame count grows (PoC H, "Reverse QR") |
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
| `RESHARE_ITEMS_PER_OP` | `200` | credentials per `ShareReshareBatch` or `ShareReshareConfirmBatch` op |
| `SAS_WORDS`, `MASTER_PHRASE_WORDS`, `PAPER_WORDS` | 4, 6, 24 | word counts |

The same table is appended to `docs/host-sync-design.md` as "Defaults chosen at
implementation" by WP-DOCS.

### 4f. Paper replication and the mandatory-paper rule (C5)

`FactorRegister { Paper }` carries in `sealed` the paper's UNWRAP public key (`pub_to_bytes`: ed25519, x25519,
ML-KEM-768; at most 1500 bytes) next to the recovery verifier in `public`. `quorum::paper_pub` reads the newest
applied registration, so every device wraps new credentials to the paper and reshares include it
(`quorum::paper_registration(phrase)` builds the op: the setup helper, a Red op whose first registration needs only
the phone and a second device). Typed paper words open a credential as the second share next to the device's own:
`unlock_paths::paper_second_share(store, host, epoch, phrase)`. A Quorum ring with exactly two live holders needs
the paper (`quorum::paper_rule`): refused for new credentials, a mode change to Quorum and a revocation that would
leave two holders. A reshare also runs when the holders differ from those the credentials are shared among
(a promoted holder, an enabled or disabled exemption): `revocation::next_actions` returns `Contribute` and
`Receive` without a rotation step.

### 4h. Recovery completion and ring setup (review rounds 2 and 3, director decision)

**Recovery completion.** A `PhoneReenroll` that reached `Ready` stands in for the three proofs of exactly two ops
by its requester: the `DeviceRevoke` of the holder phone and the `DeviceAdd` of the new phone (`recovery::ready_recovery_for`,
read by `ceremony::red_status`). `Ready` means the recovery machine's own rules held: two distinct attended devices other than
the requester or the paper phrase, a third factor from another artifact that is PROVEN (a `RedFactor` over the recovery hash,
signed by the registered paper or email key; the mailbox sender is neither requester nor voter), a third-party acknowledgement,
the 7 day lock on the local receipt clock, and no cancellation. The recovery lapses `RECOVERY_READY_WINDOW_SECS` (3 days)
after the lock ended, and one recovery admits exactly one new phone. A completion op is judged at `min(its stamp, its local
receipt)`, so a stamp cannot pull the lock forward.

**Ring setup.** A ring built from a fresh install has one holder, mirrors, no phone, no factor and no credential, so the
phone and the second holder would each be the other's missing proof. The ring is in SETUP while no credential holds Quorum
shares and the ring has not yet had a phone and two terminal holders (rows that are revoked still count, so setup is never
re-entered; derived from ops, identical on every device). In setup:

* "adding or promoting a holder needs two other live holders for the reshare" applies only when a Quorum credential exists
  (nothing to reshare otherwise, in Vault mode or before the first credential);
* promoting a terminal (`DeviceRoleChange` to holder, `DeviceAdd` holder terminal) needs the proposer, the promoted device's
  OWN attended `RedApprove` (other holders' votes do not replace it), the phone's proof once a phone exists, and a factor proof
  once a factor is registered; the phone's proof while there is no phone and the factor while none is registered are not
  required, and the review output says so (`RedStatus.note`: "ring setup: not required yet: ...");
* the first phone's admission needs its own proof for its own admission and, when it is the ring's first phone and no factor
  exists, no third factor (`first_phone_ever`, closed once any phone was ever a holder).

After setup none of this applies. Test: `ceremony::tests::a_ring_from_a_fresh_install_reaches_a_phone_and_two_holders_and_setup_then_ends`.

### 4g. The ceremony-complete rule for Red consumers (audit)

A proposer applies its own op at once (`sync_append_local` approves by definition); peers hold a Red op pending
until `settle_red_ops` finds its ceremony complete. Anything that reaches OTHER devices' data (who is wrapped to,
who is reshared to, which mode credentials are written in, which certificate or export window lets a share go)
must therefore be read through the ceremony-complete view on EVERY device, the proposer included, never from the
raw table or from "an op of that kind exists":

* `ceremony::in_force(store, op)`: not gated, or all proofs present (time-independent; an op that completed stays
  in force). `ceremony::effective_devices(store, upto)`: the device table with a promotion that is not in force
  read as a mirror and a revocation that is not in force read as not revoked. `ceremony::effective_cred_mode`:
  the mode before a `ModeChange` that is not in force. Proofs of an op count only from devices that were
  holders in force before it (`upto = op.lamport`), so a sybil holder cannot approve; a factor signs only ops
  proposed after its registration (which must itself be in force).
* Consumers by Red op kind: `FactorRegister{Paper}`: `quorum::paper_pub`, hence `share_recipients`,
  `reissue_plan`, `paper_rule`, the membership reshare; every factor verifier (`registered`). `ExemptionSet`:
  `exemption::active_slots`. `ModeChange`: `cred_ops::cred_update_body` and the unlocker (effective mode), and the
  re-encryption itself, which is a separate step (`modes::mode_migration`) that refuses until the change is in
  force. `DeviceAdd{Holder}` and `DeviceRoleChange`: `share_recipients`, `holder_public`, `live_holders`,
  `live_holder_count`, `phone_device`, recovery approvals, `validate_cert_issue`, the proof counting in
  `red_status`. `DeviceRevoke`: `revocation::open_revocation`, `live_holders`, the membership reshare.
  `CertIssue`: `cert::cert_allows` (the single choke point). `BulkExport`: `export::active_window`.
  Not Red-gated by this ceremony and therefore unchanged: `PhoneReenroll`/`RecoveryApprove` (the recovery machine
  counts its own approvals).
* Test per kind: "proposed, not complete" changes nothing a peer would act on; "complete" does
  (`ceremony::tests::*_in_force`). Known limit: the helpers read ops from the log, so ops pruned below a
  checkpoint cannot be re-evaluated; the checkpoint state must carry the effective result (op-log worker).

### 4i. Quorum to Vault: the migration step (from the TUI worker, poc/tui)

The `ModeChange` to Vault is Red and gated (4c); its second-device proof must be the named `cosigner`, an attended Holder terminal
other than the proposer (`validate_mode_change` checks `migrated_on` and `cosigner`). In the UI the cosigner is an attended full terminal
other than the proposer: not the phone, not an unattended device. The migration is a separate step that refuses until the change is in force.

* **Step two (migration).** Once the ceremony completed, the proposer opens every credential through the ordinary unlock paths (its own
  share plus a second share from an attended holder's approval, the phone, a trust session or the paper; the user sees the unlock dialog)
  and appends one `CredUpdate` per credential under a fresh DEK wrapped to the Vault recipient. A credential that could not be opened stays
  Quorum-shaped. `QuorumUnlocker` decides a credential's shape from its wrapped recipients (Vault recipient present means Vault), not from the
  effective ring mode, until every credential is migrated (`quorum.rs` `open_one`). The step is repeatable.
* **What the downgrade does to shares: nothing is revoked.** Old `CredUpdate` ciphertexts and every holder's Quorum shares stay in the log and
  still open whatever was copied under them; the new shape protects only what is written after the migration. The UI says so.
* After the ring is Vault the two-holder paper rule (`quorum::paper_rule`) no longer applies to new credentials.

### 4j. Phase B interface freeze (UX fixes of epic #13; design: `host-sync-plan/review/ux-13-phase-b-design.md`, Revision 2)

This section is the FROZEN interface the Phase B lanes code against. A change to anything here goes through the lane that owns the
thing (see the lane briefs in `host-sync-plan/phase-b/`) and is announced to every lane first. Nothing here is released yet, so no
op-format version gate is needed; all new kinds must land before the first release (2a'').

**New ops** (crate `sshub-sync-proto`, `oplog/op.rs`; canonical bytes in `canonical.rs`; tiers in `tier.rs`; vectors, python oracle and JS
interop test updated by the proto lane):

```rust
CertRenew     { cert_id: [u8; 32], expiry: i64 }                // Green, inert; author: the certificate's issuer or its successor phone
PhoneHandover { new_phone: DevicePublic, name: String }         // Red; author: the ring's live phone, judged in the view at lamport - 1
```

* Canonical `CertRenew`: object key `CertRenew` with `cert_id` (hex, 64 characters) and `expiry` (integer), like `CertExtend`.
* Canonical `PhoneHandover`: object key `PhoneHandover` with `new_phone` = `{ed25519_pub: hex32, x25519_pub: hex32, mlkem_pub: base64}` (the
  key names of `DeviceAdd.device`) and `name` (one plain line, the `DeviceAdd` name rule).
* Tiers: `CertRenew` Green (listed with `CertExtend`), `PhoneHandover` Red (listed with `CertIssue`). `PhoneHandover` is gated: `ceremony::is_gated`.
* Validity of `CertRenew` (clock-free, log only, evaluated in `(lamport, device_id, seq)` order): author is the issuer or the issuer's
  successor; the issuing op is in force; the delegate is a holder in force and not revoked in the view at the renewal's lamport; the certificate
  is not revoked; `expiry > expiry so far` and `expiry <= expiry so far + original term`; `expiry - issued_at <= CERT_LIFETIME_MAX_SECS`;
  renewals counted so far `< CERT_MAX_RENEWALS`. `cert_allows` uses the local `now` only for the use-time check against the effective expiry.
* Validity of `PhoneHandover`: author, replay and known-key rules in `effective_devices(store, Some(op.lamport - 1))`; proofs: the author
  (the op itself), the new phone's own `RedPhoneSign`, a second attended holder terminal, and `ThirdFactor::Paper` only; the effect lamport is
  the highest lamport among the counted proofs; among handovers by one author the lowest `(completion lamport, device, seq)` wins.
* `PhoneReenroll` verdict gains `Superseded` (design 5.3): an in-force `PhoneHandover` written by the phone that recovery was started against (the live phone in the view at the recovery's own lamport) supersedes it, whatever the two lamports (a phone never met by the recovery's author lags its clock, 78d); a recovery that already took effect (`Completed`) is never superseded and the replaced phone's later handover is void. `sync-notice` serves a `PhoneReenroll` as the kind 3 recovery notice, so the live phone can cancel it.

**Wire** (crate `sshub-sync-proto`, `wire.rs`; the types and constants exist already, encode and decode are the proto lane's first delivery):

* kind 11 `CertRenewRequestWire { cert_id: [u8; 32], expiry: i64, evidence: Vec<HeadEvidence> }`:
  `cert_id(32) || expiry i64 BE || n u8 || n x (terminal(8) || head_seq u64 BE || head_ts i64 BE)`, `n <= MAX_RENEW_EVIDENCE = 32`;
  `HeadEvidence { terminal: [u8; 8], head_seq: u64, head_ts: i64 }`: one entry per enforcing holder terminal other than the delegate.
* kind 12 `PhoneHandoverRequestWire { new_phone: DevicePublic, name: String, terminal: [u8; 8] }`:
  `ed25519(32) || x25519(32) || mlkem_len u16 BE || mlkem || name_len u8 || name utf8 || terminal(8)`; the old phone shows the name and
  `HANDOVER_WORDS = 8` words: the first 88 bits of BLAKE3 over `"sshub/handover/v1" || ed25519 || x25519 || mlkem || name_len u8 || name || old_phone_id(8)`,
  encoded with the project's word list (`sas_words` with 8 words).
* kind 5 trailing `terminals` block after the optional sender block (`0x01 || id(8)`): `0x02 || count u8 || newest_op_ts i64 BE`
  (`PHONE_OPS_TERMINALS_TAG = 2`, type `TerminalsBlock`). `count` = non-revoked, non-compromised devices of kind Terminal in the sender's view, itself included,
  attended or not, holders or mirrors, saturating at 255. `newest_op_ts` = the author stamp of the newest op in the list. UNSIGNED, informational.
  Blocks must come in tag order; an unknown or truncated block stays an error; frames without it decode as before. `PhoneOpsWire` gains
  `terminals: Option<TerminalsBlock>`.

**Limits** (`limits.rs`, contract section 9): `RECOVERY_TIMELOCK_SOLO_SECS = 14 days`, `CERT_MAX_RENEWALS = 2`, `CERT_LIFETIME_MAX_SECS = 180 days`,
`CERT_RENEW_EVIDENCE_MAX_AGE_SECS = 6 hours`, `HANDOVER_WORDS = 8` (all present). `RED_CEREMONY_TTL_SECS` becomes 7 days with the compile-time assert
`RED_CEREMONY_TTL_SECS < CHECKPOINT_PRUNE_DELAY_SECS + CHECKPOINT_MIN_INTERVAL_SECS`; that change belongs to the ceremony lane with the tests that pin 3 days (W-TTL).

**Host core API** (lane C; `pub`, in `src/sync`):

```rust
// ceremony.rs
pub struct Blocker { pub proof: RedProof, pub who: Vec<String>, pub since: Option<i64> }
pub fn blocking_view(store: &LauncherStore, op: &Op, now: i64) -> Result<Vec<Blocker>>;
pub(crate) fn log_live(store: &LauncherStore, devices: Vec<DeviceRow>, t: i64) -> Result<Vec<DeviceRow>>;   // ADVICE only, never a verdict (exists)
// quorum.rs
pub struct Exposure { pub host_uid: HostUid, pub devices: Vec<DeviceId>, pub kind: ExposureKind }       // Removed (revoked device holds a share) | NotHeldBy (current holder lacks one)
pub fn exposed_credentials(store: &LauncherStore) -> Result<Vec<Exposure>>;
// revocation.rs
pub enum RevokeMode { Full, Degraded, Refused(String) }
pub fn revoke_mode(store: &LauncherStore, target: &DeviceId) -> Result<RevokeMode>;
// recovery.rs
pub fn open_for_alert(store: &LauncherStore, me: &DeviceIdentity, now: i64) -> Result<Vec<Op>>;        // deduped by requester by the caller
// cert.rs
impl DelegationCert { pub fn effective_expiry(&self, store: &LauncherStore) -> Result<i64>; pub fn renewals(&self, store: &LauncherStore) -> Result<u32>; }
pub fn cert_renew_request(store: &LauncherStore, cert_id: [u8; 32], term_secs: i64, now: i64) -> Result<CertRenewRequestWire>;
// handover.rs (new, lane C)
pub fn handover_words(new_phone: &DevicePublic, name: &str, old_phone: &DeviceId) -> Vec<String>;      // HANDOVER_WORDS words
// cred_ops.rs / unlock_paths.rs (heal)
pub fn heal_plan(store: &LauncherStore) -> Result<Vec<HostUid>>;                                         // credentials whose newest sharing lacks a current holder and are not exposed
pub fn heal_credential(store: &LauncherStore, id: &DeviceIdentity, host: &HostUid, payload: &CredPayload) -> Result<Op>;   // re-seal under a fresh DEK to the current recipients
```

**Phone core API** (`crates/sshub-phone-core`; the page `assets/companion/src` mirrors every entry point under the same name in camelCase; UniFFI exposes them to Kotlin):

```rust
pub struct RenewView { pub delegate: String, pub scope: String, pub term_secs: i64, pub budget: u32, pub renewals_left: u32, pub new_end: i64,
                       pub opened: Vec<String>, pub evidence_age_secs: Option<i64>, pub refusal: Option<String> }
pub fn describe_cert_renew(state: &PhoneState, req: &CertRenewRequestWire, now: i64) -> Result<RenewView>;
pub fn build_cert_renew(state: &mut PhoneState, id: &PhoneIdentity, req: &CertRenewRequestWire, now: i64) -> Result<Vec<u8>>;   // refuses stale evidence, the caps, a non-live phone

pub struct HandoverView { pub name: String, pub words: Vec<String> /* HANDOVER_WORDS */, pub certs_carried: Vec<String> }
pub fn describe_phone_handover(state: &PhoneState, req: &PhoneHandoverRequestWire) -> Result<HandoverView>;
pub fn build_phone_handover(state: &mut PhoneState, id: &PhoneIdentity, req: &PhoneHandoverRequestWire, now: i64) -> Result<Vec<u8>>;
pub fn sign_handover_admission(state: &mut PhoneState, id: &PhoneIdentity, handover_op_hash: [u8; 32], now: i64) -> Result<Vec<u8>>; // the NEW phone's own RedPhoneSign

pub fn adopt_own_chain(state: &mut PhoneState, id: &PhoneIdentity, ops: &[SignedOpBytes], sender: Option<[u8; 8]>, terminals: Option<TerminalsBlock>) -> Result<AdoptInfo>;
pub fn accept_head(state: &mut PhoneState, shown: (u64, &[u8]), acknowledged_single_source: bool) -> Result<(u64, u32)>;
// AdoptInfo gains: single_source: bool, terminals: Option<TerminalsBlock>
```

**Rules every lane keeps:** a Red or Green verdict reads only the log (D0, D0b): no `last_seen`, no `received_at`, no `now` except the inherent use-time checks;
`log_live` is advice for screens. The alert has no acknowledge key. Group approval touches only hold reason `stale`. The emailed code never counts for a holder revoke or a handover.

### 4k. Phase B host core as built (lane C; wave 1 and 2)

What exists on top of 4j, with the names the screens and the scenarios use:

* **Verdicts** (D0b): `ceremony::red_status` never reads `now` or the local receipt time for a verdict; `expired` only says the window passed on this clock
  and the op is not complete. `ceremony::blocking_view(store, op, now) -> Vec<Blocker { proof, who, since }>` names, per missing proof, the devices
  (or "the paper phrase", "the emailed code (mailed by NAME)") that can give it; for removing a computer the second-device entry adds "or the paper
  phrase instead". `since` is the op's author stamp, display only.
* **Revocation:** `revocation::revoke_mode(store, &target) -> RevokeMode { Full, Degraded, Refused(String) }`; `check_revocable` refuses only on
  `Refused`. `TERMINALS_TO_CUT_OFF` is 2 (the removed device plus one remaining). `quorum::exposed_credentials(store) -> Vec<Exposure { host_uid,
  devices, kind: Removed | NotHeldBy }>` is derived from the log and never stored.
* **Heal** (`cred_ops`): `heal_plan(store)` lists the Quorum credentials whose newest sharing lacks a current holder and that no removed device holds a
  share of; `heal_credential(store, id, host, payload)` re-seals one under a fresh DEK to the current recipients and is refused for any other. Heal on use:
  `QuorumUnlocker::unlock` calls it after a successful open (best effort, no extra prompt).
* **Recovery:** `recovery::open_for_alert(store, &DeviceIdentity, now)` and `open_for_alert_for(store, &DeviceId, now)`; `ReenrollVerdict::Superseded` for a
  recovery that a later in-force `PhoneHandover` replaced (`PhoneHandover` is a gated Red op: three proofs until the handover rules land);
  `recovery::is_solo`. A build with the cargo feature `e2e-hooks` (the e2e image only) reads `SSHUB_E2E_SOLO_LOCK_SECS` to shorten the 14 day solo lock, `SSHUB_E2E_STALE_AFTER_SECS` the stale horizon, `SSHUB_E2E_LIVE_WINDOW_SECS` the 14 day live-device window (required checkpoint signers, ceremony voters), and `SSHUB_E2E_CHECKPOINT_OPS` / `SSHUB_E2E_CHECKPOINT_MIN_INTERVAL_SECS` the checkpoint triggers.
* **Certificates:** `DelegationCert::effective_expiry(store)` and `renewals(store)` read the folded record of the log (`oplog::cert_record`): renewals count
  clock-free, in `(lamport, device, seq)` order, and only while the delegate is a holder in force; `cert_allows` and `extension_budget` use the effective end;
  `cert::cert_renew_request(store, cert_id, term_secs, now)` builds the kind 11 request (new end = effective end + term, at most the first term, at most
  `CERT_LIFETIME_MAX_SECS` after the issue, at most `CERT_MAX_RENEWALS` renewals; evidence = one `HeadEvidence` per holder terminal other than the
  delegate). Every ingested `CertRenew` is mailed once through `[sync.notify]` (`cert::owed_renew_mails`, `cert::send_renew_mails`, hooked in
  `SyncRuntime::mail_owed_codes`; the CLI `sshub sync` path must call `send_renew_mails` too); the sticky notice is a terminal screen.
* **Phone list:** `hub::phone_chain_wire` fills the kind 5 `terminals` block: the number of non-removed terminals (this one included) and the newest op stamp
  of the list. Unsigned, informational.
* **PhoneHandover (wave 3):** `PhoneHandover` is a gated Red op judged in the view just BEFORE it (the old phone is the live one). Proofs: the author (the live
  phone), the new phone's own `RedPhoneSign` (folded into `RedProof::Phone`; it may still wait as the op of an unknown author), a second attended terminal,
  and `ThirdFactor::Paper` ONLY (the emailed code never counts; no registered paper blocks the op). The new phone's keys must be new to the ring (else a rule
  violation). `ceremony::handover_effect(store, op) -> Option<u64>` is the completion lamport: the highest, over the proof kinds, of the lowest counted proof
  lamport. Among handovers by one phone the lowest `(completion lamport, device, seq)` wins; any other is a rule violation (applied and still pending rivals both
  count). `effective_devices(upto)` retires the old phone and makes the new phone a holder only from the completion lamport on (it undoes the oplog's row
  effects for earlier views), so everything written before it is judged under the old phone. `handover::handover_words` is the proto crate's eight word
  function; `handover::handover_request(new_phone, name, &terminal)` builds the kind 12 request. Certificates (design 5.4): `cert::issuer_phones(store, &current)`
  lists the phone and the phones it took over from; `trust_grant` honours certificates of all of them (the retired key still verifies the issue op) and
  `extension_budget` counts extensions by the successor (the oplog's `sync_cert_holder_at`, which orders handovers by their op lamport, not the completion
  lamport: an old-phone extension written between the op and its completion is not counted, a harmless edge).

### 4l. Two interface decisions that amend 4j (S, 2026-11-09; announced to every lane)

**(a) The renewal card shows the hosts the delegate opened: DECIDED, kind 11 carries them.** The phone keeps no host list and the renewal screen exists to show
what the delegate did, so the host, which reads it from the log, supplies it. Kind 11 gets a trailing block after the evidence entries:
`m u8 (m <= 8) || m x (len u8 || host name utf8, 1 to 48 bytes) || total u16 BE`: the `m` most recently opened hosts of the certificate (newest first) and the total
number opened under it. Hosts come from `cert::recorded_openings` (what the holders recorded, never the delegate's own claim), names from the host table (a host
with no name here is listed as its first 8 hex digits); `RenewView.opened` is the name list and `RenewView` gains `opened_total: u32`. A frame without the block
decodes as before and reads as "the terminal sent no list": the phone says so and shows the evidence age only (the card never claims an empty list means nothing
was opened). Owners and cost (hours come out of the lane's slack, not new budget): P wire encode/decode, vectors, oracle 3 h; C `cert_renew_request` fills it 1 h;
PH-core `describe_cert_renew`, page parity 3 h; PH-android card 1 h; T the terminal view 1 h; E scenario 75a asserts the list 1 h. About 10 h in all.
Rejected: dropping the list from the design. The mail and the sticky notice come after the fact; this is the only moment before the signature, and 10 h is
small next to a card that cannot show what it exists to show.

**(b) The new phone receives the handover op hash inside kind 12 itself.** Two stages of one frame, nothing new on the wire:
1. *Request (old phone).* The terminal shows `PhoneHandoverRequestWire` as in 4j; the old phone compares the 8 words, signs and builds the `PhoneHandover` op.
2. *Admission (new phone).* After the terminal ingested that op it shows the NEW phone a second kind 12 frame whose layout is the 4j layout followed by
   `0x01 || op_hash(32) || op_lamport u64 BE` (the hash and the lamport of the `PhoneHandover` op just ingested). Without the trailing block the frame is a request; a
   trailing block that is not exactly `0x01` and 40 bytes is an error. The new phone's proof only counts when its lamport is HIGHER than the op's (a proof must come
   after what it names), so `sign_handover_admission` writes its `RedPhoneSign` with `lamport = max(own next, op_lamport + 1)`. The new phone's `describe_phone_handover` returns `HandoverView.op_hash: Option<[u8; 32]>` and `op_lamport: Option<u64>` (present in stage 2 only) and shows the
   same 8 words so the new owner can compare them with the old phone; `sign_handover_admission(state, id, op_hash, now)` takes the hash from the view, never from a typed
   argument. A new phone refuses to sign unless the frame's `new_phone` is its own public key. Its `RedPhoneSign` returns to the terminal as ordinary phone ops (kind 5)
   and counts only for the op that names its key (a wrong hash signs nothing that can be used).
   The trailer is unsigned, so the new phone refuses a frame whose `op_lamport` is above `u32::MAX` (a forged huge value would push its counters out of range). The host
   needs no matching cap of its own: it never issues such a lamport (a device's lamport is at most the ring's maximum plus one), ingest rejects any op more than
   `LAMPORT_JUMP_CAP` (100 000) above the highest lamport it holds, and `red_status` counts a proof only when its lamport is higher than its op's (`valid_proof`), so
   a `RedPhoneSign` written before its op never counts.
Owners and cost: P kind 12 optional trailer, vectors 2 h; PH-core view and refusal, page parity 2 h; PH-android new-phone screen (inside the 12 h handover line);
T `sync-phone-enroll --replace` shows the second frame (inside its 10 h line); E `phone2` scans stage 2 (inside 78a); about 4 h extra in all.
