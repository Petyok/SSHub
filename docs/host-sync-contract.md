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
  `prev_hash` as lowercase hex, other 32-byte values as number arrays exactly as
  the vectors show, integers as plain JSON number literals (u64::MAX is
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
- **ShareReshareConfirm (C7)** is part of the op set (section 3).
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

// Shamir k-of-n over GF(256) on a 32-byte secret.
pub struct Share { pub index: u8, pub bytes: Vec<u8> }
pub fn shamir_split(secret: &[u8; 32], k: u8, n: u8) -> Vec<Share>;
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
  nonce(24) || ct`, plaintext `ed25519 seed || x25519 secret || ML-KEM dk (2400)`,
  header as associated data.

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
    DeviceAdd     { device: DevicePublic, name: String, role: DeviceRole, attendance: Attendance },
    DeviceRevoke  { device_id: DeviceId },
    // requester is in the body as well as the header: a holder that checks only
    // the body must still see WHO asks.
    ShareRelease  { host_uid: HostUid, request_id: u64, requester: DeviceId },
    ShareReshare  { epoch: u64, new_holders: Vec<DeviceId>, commitments: Vec<Vec<u8>> },
    ShareReshareConfirm { reshare_hash: [u8; 32] },
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
pub enum ThirdFactor { Paper, HardwareKey(DeviceId), EmailCode }
pub struct WrappedShare { pub recipient: DeviceId, pub alg: AlgTag, pub sealed: Vec<u8> }

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
}
pub enum DeviceStatus { Operational, Behind, Stale, Suspect, Compromised, Revoked }
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

## 3. Stage 2: `src/sync/{protocol,ratchet,enroll,client}.rs` (WP2a, WP2b)

```rust
pub enum SyncMsg {
    Hello { proto: u16, device_id: DeviceId, epoch: u64 },
    Heads(Vec<HeadRef>), WantTail { device_id: DeviceId, after_seq: u64 },
    Tail(Vec<Op>),
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

## 4. Stage 3: `src/sync/{wire,quorum,share_release,snapshot,cert,recovery,modes,cred_ops}.rs` (WP3a, WP3c)

```rust
pub struct Challenge { pub op: Op, pub nonce: [u8; 32], pub expiry: i64,
    pub eph_x25519_pub: [u8; 32], pub request_id: u64 }
impl Challenge {
    pub fn to_qr_payload(&self, qr_budget: usize) -> Vec<Vec<u8>>;   // fountain frames
    // H(domain_tag || proto_version_le || canonical_op || nonce || expiry_le || eph || request_id_le)
    // domain_tag = b"sshub/sync/challenge/v1", proto_version u16. WHOLE op, not body.
    pub fn binding_hash(&self) -> [u8; 32];
}
pub const SYNC_WIRE_V1: u16 = 1;   // frame: b"SWR1" || version u16 BE || kind u8 || body
pub fn unwrap_share_response(eph_priv: &[u8; 32], ciphertext: &[u8]) -> anyhow::Result<Share>;

pub enum UnlockPath { Peer(DeviceId), PhoneAudio, PhoneReverseQr, PhoneClipboard,
                      HardwareKey, ShortCode, FullEntry }
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
| `QR_FRAME_PAYLOAD_BYTES` | `250` | payload per QR frame, ECC level M |
| `QR_FPS` | `4` | animated frame rate |
| `QR_PARITY_PERCENT` | `25` | XOR parity frames in the loop |
| `MAX_FRAME_BYTES` | `1 << 20` | transport frame cap |
| `MAX_JSON_DEPTH` | `32` | frame JSON depth cap |
| `MAX_OPS_PER_TAIL` | `2_000` | ops per `Tail` frame |
| `HELLO_TIMEOUT_SECS`, `ROUND_TIMEOUT_SECS`, `IDLE_TIMEOUT_SECS` | 10, 120, 30 | transport timeouts |
| `PENDING_QUARANTINE_MAX_PER_DEVICE` | `10_000` | quarantine cap per device |
| `TIMESTAMP_SKEW_GRACE_SECS` | `600` | informational skew grace (never used for ordering) |
| `SAS_WORDS`, `MASTER_PHRASE_WORDS`, `PAPER_WORDS` | 4, 6, 24 | word counts |

The same table is appended to `docs/host-sync-design.md` as "Defaults chosen at
implementation" by WP-DOCS.
