# Host sync: the native Android companion

Status: design plus a proven toolchain (phase 1). Branch `poc/android`. Contract rules: `docs/host-sync-contract.md`, C21.
Read `docs/host-sync-design.md` (what a share holder is), `docs/host-sync-companion.md` (the page this app replaces on
Android) and contract C16 and C20 first.

## 1. Decision and scope

The phone role today is `assets/companion/sshub-companion.html`: one offline page, about 5000 lines of JS (`src/*.mjs`)
that re-implement in JavaScript what `src/sync/` already does in Rust (identity, wrap generations, op chain, canonical ops,
frames, fountain QR, snapshot blobs, share release, trust sessions, reshare). Every rule exists twice and is pinned by
shared vectors. The native app must not add a third copy.

**One implementation of the phone logic, in Rust, shared with the terminal.** The app is a thin Kotlin + Jetpack Compose
shell around a Rust crate (`sshub-phone-core`) that depends on the sync primitives the terminal uses (`sshub-sync-proto`,
extracted from the `sshub` crate, section 3). Kotlin owns: screens, camera, BLE radio, audio output, clipboard and share
sheet, Android Keystore and biometrics, file storage. Rust owns: everything that decides what is signed, sealed, parsed or
refused.

The page stays, unchanged, as the iOS and fallback phone. Both phones are the same device when started from the same 24
words, so they MUST obey the same rules (C21) and are pinned by the same fixtures (section 9).

Non-goals: the app is not a sync peer (no SSH, no host copy), not a password manager, has no network permission at all,
no push, no accounts, no analytics, no cloud backup.

## 2. Architecture

```
 Compose screens (Kotlin)  <-- state/events -->  ViewModels
        |                                           |
   platform adapters (Kotlin)                 uniffi bindings (generated Kotlin)
   CameraX frames, BLE GATT central,                 |
   AudioTrack, ClipboardManager, share        sshub-phone-core (Rust, cdylib, JNA via uniffi)
   sheet, Keystore + BiometricPrompt                 |
        |                                      sshub-sync-proto (Rust)  <-- also linked into `sshub`
   app-private files (noBackupFilesDir)       crypto, canonical ops, wire, fountain, snapshot, FSK, QR decode, BLE link
```

Rules of the split:

- **Rust never touches the platform.** No camera, radio, clock, files. Kotlin passes bytes and `now` (unix seconds) in;
  Rust returns bytes, views and next-step instructions. This keeps every flow testable on the host with fixtures and keeps the
  native surface small.
- **Kotlin never decides security.** It does not parse ops, compare hashes or format "what you are approving"; it renders the
  view structs Rust returns, built from the signed bytes (same rule as the page: the screen is built from the bytes, there is no
  description field).
- Randomness is the OS CSPRNG inside Rust (`getrandom`, works on Android). Tests inject a seeded RNG through
  `PhoneCore::with_rng` (cfg(test) and a `testing` feature, never exported to Kotlin in release).
- Panics never cross the boundary as crashes: uniffi converts a panic into an exception; the app shows "internal error" and
  locks. `panic = unwind` stays (do not set `abort`).

### 2.1 Rust: `crates/sshub-phone-core`

`cdylib` + `lib`, `uniffi` proc-macros (library mode, no UDL), builds with `cargo-ndk` for `aarch64-linux-android` and
`x86_64-linux-android` (arm32 and x86 are not built; `minSdk` 30 devices are all 64 bit).

Android build of the core (the library the app loads; the uniffi Kotlin is generated from it in library mode):

```sh
cargo ndk -t arm64-v8a -t x86_64 --platform 30 build -p sshub-phone-core --profile phone
```

Modules (all new code is a port of the page; sources of truth listed):

| module | content | port of |
|---|---|---|
| `identity` | secret to keys (`DeviceIdentity::from_seed`, context `sshub/sync/phone/v1`), wrap generations (`sshub/sync/phone/wrap/v1`), 24 words | `keys.mjs` |
| `state` | `PhoneState` (serde, versioned): enrolled ring, aliases, own chain (seq, lamport, epoch, newest ops), wrap epoch, live snapshots, `head_pending`; sealed blob in/out | `state.mjs` |
| `chain` | authoring: `HeadAttest`, `RedPhoneSign`, `CertIssue`, `CertRevoke`, `CertExtend`, `PhoneReenrollCancel`, `WrapKeyRotate`, `ShareStorageKey`, `ShareReshareConfirmBatch`; head adoption; courier tail (8) | `state.mjs`, `ops.mjs`, `reshare.mjs` |
| `describe` | `OpView` rows built from received canonical bytes, highlight flags, `safe_text` | `ops.mjs` |
| `release` | challenge (kind 1) check, binding words, share unwrap, reply (kind 2) with courier ops and snapshot | `release.mjs`, `frames.mjs` |
| `cert` | trust-session request (kind 9) and extension (kind 10) validation, issued-cert reading | `cert.mjs` |
| `reshare` | adopt a reshare, publish new shares | `reshare.mjs`, `quorum.mjs` |
| `enroll` | joining side (role 0) of the commit, reveal, SAS, confirm exchange | `app.mjs`, `frames.mjs` |
| `channels` | fountain encode of a frame to QR payloads and the module matrix (crate `qrcode`), fountain decode, `fsk::encode` to PCM, QR decode from a gray frame, `ble_link::Link::phone` | `channels.mjs`, `scan.mjs` |
| `ffi` | the uniffi surface below | |

The surface is one stateful object, no global state:

```rust
#[derive(uniffi::Object)] pub struct PhoneCore { /* Mutex<Inner>: Option<Unlocked{ secret: Zeroizing<[u8;32]>, id, state }> */ }
#[uniffi::export] impl PhoneCore {
    #[uniffi::constructor] pub fn new(sealed_state: Option<Vec<u8>>) -> Arc<Self>;
    pub fn create_secret() -> Vec<u8>;                    // 32 CSPRNG bytes; Kotlin wraps them with the Keystore key at once
    pub fn unlock(&self, secret: Vec<u8>) -> Result<UnlockInfo, CoreError>;   // device id, enrolled?, head_pending
    pub fn lock(&self);                                   // zeroizes secret and keys
    pub fn backup_words(&self) -> Result<String, CoreError>;
    pub fn restore_from_words(&self, words: String) -> Result<Vec<u8>, CoreError>;  // returns the secret to wrap; sets head_pending
    // every flow: bytes in (a frame from any channel), a view out, then an explicit decision
    pub fn open_frame(&self, frame: Vec<u8>, now: i64) -> Result<Opened, CoreError>;      // kind 1,3,5,9,10,6,7 ...
    pub fn decide(&self, token: u64, decision: Decision, now: i64) -> Result<Outgoing, CoreError>;
    pub fn take_chain(&self, ops_frame: Vec<u8>) -> Result<ChainAdopted, CoreError>;      // head adoption
    pub fn never_signed(&self) -> Result<(), CoreError>;
    pub fn export_ops_text(&self) -> Result<String, CoreError>;
    pub fn sealed_state(&self) -> Result<Vec<u8>, CoreError>;       // call after every mutation, Kotlin persists it
    // channels
    pub fn qr_frames(&self, frame: Vec<u8>, parity_percent: u32) -> Vec<QrFrame>;        // payload + module matrix
    pub fn scan_gray(&self, w: u32, h: u32, luma: Vec<u8>) -> Vec<ScanEvent>;             // camera: decode QR, feed fountain
    pub fn audio_burst(&self, frame: Vec<u8>, profile: AudioProfile) -> Vec<i16>;
    pub fn ble_new(&self, mtu: u32) -> Arc<BleSession>;    // sans-IO link: on_message / take_outbox / tick (C20, phone role)
}
```

`Outgoing` carries the reply frame bytes plus pre-rendered channel payloads, so Kotlin just displays and plays.
`Decision` for a share release carries the per-channel options and the optional short code. Anything that needs a typed
confirmation (stranger requester id) is a field of the view and of `Decision`, checked in Rust.

State persistence: `sealed_state()` returns the state encrypted (XChaCha20-Poly1305) under
`BLAKE3 derive_key("sshub/sync/phone/state/v1", secret)`, so the whole app needs one Keystore-protected secret and the file on
disk is useless without it. Kotlin writes it atomically (write temp, fsync, rename) into `noBackupFilesDir`. Losing the
file loses only the local record (enrollments, short code list, chain record); the 24 words restore the device and the chain
is taken back from a terminal (section 5).

### 2.2 Kotlin app: `android/`

- Single module `:app`, `uk.petruha.sshub.phone`, Kotlin 2.1, Compose BOM 2025.05, Material 3, Navigation Compose.
  One `Activity`; screens as composables over a `PhoneViewModel` that owns the `PhoneCore` and is the only caller of it.
- Visual direction: the maintainer chose **A, "Chain"** (dark-first mono, one amber focus, devices as a chain of squares, proofs as squares,
  time as a tick scale, Red as a diff); the normative spec is `host-sync-plan/android-design/spec-A.md`, FINAL (tokens mapped to M3 roles dark and light, bundled JetBrains Mono 2.304 and
  Inter 4.1, both OFL 1.1, type scale, spacing, components, motion with reduced-motion, accessibility, dangerous-field rules, section 12 = catalogue of
  35 screens; prototype `design-demos/A-chain.html`, screenshots `screenshots/A-final/`). Prototype-only aids (dashed prototype-controls rows, the demo PIN,
  auto-advancing receivers) are not app features: the app unlocks with the Keystore (section 4), not a PIN. The UI layer is built around it in three packages:
  `ui.theme` (one `SshubTheme`: M3 `ColorScheme` from the spec's roles, dark first with a light variant, `Typography` from fonts bundled in
  `res/font` so there is no downloadable-font network call, shapes, motion specs, an `Amber` focus token), `ui.components` (ChainSquare, ProofSquare,
  TickScale, DiffRow, the highlighted-row and countdown components, each with a `@Preview` and a Compose test) and `ui.screens`. Screens use only
  theme tokens and components, never raw colours, so a spec change is one package. Phase 1 stays unstyled until the spec is final.
- Screens mirror the page (functional parity, no more): first run and backup words, lock, home, enroll, scan or paste a request
  (release, Red notice, trust session, extension, recovery notice, reshare), approval, outputs (QR loop, clipboard text, sound, by
  hand), short codes, trust sessions, reshare and wrap keys, take my chain, export ops text, About and fingerprint.
- Manifest permissions: `CAMERA`, `BLUETOOTH_SCAN` with `neverForLocation`, `BLUETOOTH_CONNECT`, `USE_BIOMETRIC`, `HIDE_OVERLAY_WINDOWS` (a normal permission since API 31: the window hides overlays other apps draw over it, against tapjacking). **No `INTERNET`**, no
  storage, no location, no `RECORD_AUDIO` (the phone only plays sound; the terminal listens). A CI check dumps the merged
  manifest (`aapt2 dump permissions`) and fails on any other permission: this is the native counterpart of the page's
  `connect-src 'none'`.
- `allowBackup=false`, `dataExtractionRules` exclude everything, `FLAG_SECURE` on the backup-words, approval and output screens
  (no screenshots, no recents thumbnail), screen kept on and brightness raised on the QR output screen only.
- No R8 in phase 1; release minify stays off until the uniffi/JNA keep rules are tested (open item, no security impact:
  the app has no secrets in code).

### 2.3 Kotlin adapter

`UniffiPhoneCore` (`android/app/src/main/java/uk/petruha/sshub/phone/core/uniffi/`) is the one implementation of `PhoneCoreApi` the release app
uses: a thin adapter over the generated `uniffi.sshub_phone_core.PhoneCore` (errors are aliased on import: the generated `CoreException`
and `Opened` share names with ours). One `PhoneCore` per process; it reads the sealed state from `SealedStateStore` (`noBackupFilesDir/core/state.bin`)
at construction and writes `sealed_state()` after every call that changes it (unlock, `openFrame` of anything but a refusal, every decision,
rotate, never signed, short code declared dead). Blocking calls run on `Dispatchers.Default` where the Kotlin method is `suspend`.

**Token cache.** Rust is `open_frame` -> view with a token -> `decide(token, ...)`; our interfaces fetch views by token. `openFrame` maps
the Rust `Opened` to the group's Kotlin `Opened` subtype and keeps `token -> (Kotlin view, Rust view)` in memory. `pendingView`, `trustRequest`,
`trustExtend`, `reshareCandidate`, `recoveryNotice(s)` and the Home `pending` list are answered from it. The core drops all pending requests
when it opens the next frame that carries the frame magic and when a decision runs. A decision that fails with `BadInput` (a wrong typed id)
keeps the request pending: the core leaves it and the cache keeps the token and the view. A refusal, a success and every other error (Refused,
Locked, Internal) consume it, in the core and in the cache, so the cache holds what the core can still decide. It is never persisted.

| Rust | Kotlin |
|---|---|
| `Opened::Refused` | `OpenedRefused` (reason as title, checks as `ReleaseCheck`); for kinds 6 to 8 `EnrollRefusal` |
| `ShareRequest` / `RedNotice` | `OpenedRelease` / `OpenedRed`; `Row` -> `RedField(from = null)`; `typeToApprove` -> `mustType`, the "never introduced" `warn` check -> `needsTypedId` |
| `CertRequest` / `CertExtend` | `OpenedTrustRequest` / `OpenedTrustExtend`; `problems` -> `refusal` (no Approve) |
| `Reshare` | `ReshareOpened`; `decide(Approve)` is adopt and sign in one step, and `decideReshare` is that one call: it returns the new `WrapKeyState` and the signed `Outgoing` together (`ReshareOutcome`), nothing is held between two Kotlin calls |
| `ReenrollNotice` | `RecoveryNoticeOpened`; cancel = `decide(Approve)` |
| `Chain` | `OpenedChain`; `adoptChain` = `decide(Approve)` |
| `Enroll` / `Enrolled` | `OpenedEnroll` / `OpenedEnrolled`; `EnrollName` / `EnrollSas` decisions |
| `Outgoing` | `Outgoing`: `QrFrame` -> `QrMatrix`, `audioBurst(frame, profile)` for each sound profile asked for (48 kHz, `fsk::RATE`) |
| `CoreError` | `CoreException`: `BadInput` Malformed, `Refused` Refused (HeadPending while `head_pending`, Expired for a release past its expiry), `Locked` Refused, `Internal` Internal |

The BLE link is `CoreBleLink` over `BleSession` (started at creation, monotonic `elapsedRealtime` milliseconds); `CoreBleLinkFactory` makes one per connection.

**What the Rust core cannot fill** (null, empty or a neutral value on the Kotlin side; nothing is invented):

- Models carry only what the core can give, so a missing field is a missing Kotlin field, not a placeholder. Removed for that reason: ring hosts, credentials, threshold, quorum, last sync, device roles, "seen" texts, stale flags, per-kind pending counts and the age of a pending item (`PendingItem.age` is null).
- Names: an empty or absent core name becomes `null` (`RingDevice.name`, `ReleaseView.requesterName`, `RedView.proposerName`, `IssuedCert.deviceName`); the screens show `displayName(name)` ("not met"). `PhoneStatus.deviceName` and `HomePhoneView.name` are empty before the first enrollment. `RingView.epoch` and `PhoneStatus.epoch` are `UnlockInfo.ring_epoch`. The ring list is `UnlockInfo.ring` plus this phone when it is not in it.
- `ReleaseView`: the host IS in the signed op (`ShareRelease` body carries `host_uid`). `hostUid`, `bindingHash` and `opRows` (every leaf of the op, with the core's `danger` flag) are mapped and drawn above the binding words, the host in groups of 4, so the user can compare it with the terminal. The credential name and mode are not in the op. `requesterKey` is null unless the requester's signature verified. `signedAt` is `expiresAt - 120` (the core gives `expires_at` only). `ShareWords` has the share index and the words; the number of shares is not given. Channels: QR on and one sound choice (off, fast, slow) off by default; there is no clipboard channel ("Copy as text" is always offered); the card says "Shown as: ...", never "Sent"; Approve is disabled with no channel and no by-hand.
- `RedView`: `mustType` is the proposer id to type when the proposer was never met (D13); Approve stays disabled until it matches and the core checks it again. Only this phone's own proof is a state (`phoneProof`); the other two proofs are one sentence, not tracked state. Time, lamport, seq and `lapsesAt` (proposal + 3 days) are the signed op's own values; row `danger` comes from the core.
- `TrustRequestView`: device id and name are read from the `Trusted device` row; `requesterKey` is always null by design (kind 9 and 10 frames are unsigned, so the core sends no key; the page shows only its problems list and so does the app: `problems` become the refusal text); `epoch` is the `Phone epochs it holds for` row, null when the core sends none, with no per-check list; `signedSummary` is the headline. `TrustExtendView`: `again` is always null (the refusal text carries the timing); diff is the budget row and the extra hosts row.
- `IssuedCert`: every field comes from the structured record except `lamport`, which is always null (the record has no chain position). A group scope has `group` set and `hosts` empty; the revoked flag is `CertState.Revoked`.
- `ExportedOps.rows`: kind, time and seq from the core's op view; `hash` is the core's op hash (hex of the BLAKE3 of the canonical bytes; the line is shown only when present); `danger` is set when the op has a danger row; only the newest ops the frame carries are listed, `total` is the newest seq.
- Recovery notice: `startedAt` and `lamport` are the signed values of the `PhoneReenroll` op; `phoneName` is empty until the phone has a name; `startedUtc` and `endsUtc` are formatted in Kotlin in the core's own `iso()` form; always `cancellable` (the core refuses at decision).
- Reshare: `note` is a count of verified contributors, `adoptable` is true whenever the core returned a view. The stages are Start, Rotated, Candidate (rotated and a reshare is held for review), Published (adopted and published by one call) and Empty; Start is shown until a rotation (so Rotate is reachable); the sequence is in memory and restarts as Start after a process restart. The signed ops are shown once, from the result of `decideReshare`; reopening the screen later offers Export my ops instead.
- Own-ops list (kind 5): the core gives `agreed`, the heads, and the page's own sentences `summary` and `note`, which the chain-head screen shows as they are; there is no `ChainCheck` list (the type stays for the screen's own states). `ScanEvent.Foreign` (a completed document that is not an SSHub frame) is mapped and the scan screen says 'not what this step needs'. `validateWord` is the core's `is_word` (BIP39 list; a partial word is Bad). `nameProblem` is null: the name rules run in `enrollDecide(Name)`.
- `state_lost` of `UnlockInfo` and `damaged` of the scan progress are not carried (a lost state shows as a restored phone with `headPending`).

## 3. Extraction plan: `sshub-sync-proto`

Today `src/sync/` lives inside the `sshub` crate (lib + bin) whose dependencies (`ssh2` + vendored OpenSSL, `rusqlite`
bundled, `dbus-secret-service`, `crossterm`, `ratatui`, `portable-pty`, `cpal`, `nokhwa`, `linuxvideo`) do not build for Android.
The primitives, however, depend only on pure-Rust crates. Phase 1 proves it: `crates/sshub-phone-core` compiles the unmodified
`src/sync/crypto/mod.rs` and `src/sync/limits.rs` for `aarch64-linux-android` and `x86_64-linux-android` through `#[path]`
(two lines in `lib.rs`; the terminal's own known-answer test still passes there).

The extraction is mechanical and is to be done in ONE pass by the integrator after the daily review merges settle (no
change of behaviour, `git mv` plus path fixes), as one commit `refactor(sync): extract sshub-sync-proto`.

### 3.1 Workspace

Root `Cargo.toml` gains

```toml
[workspace]
members = [".", "crates/sshub-sync-proto", "crates/sshub-phone-core"]
default-members = ["."]          # `cargo build/test` at the root is unchanged; add `-p sshub-sync-proto` to CI
resolver = "2"
```

One workspace, one `Cargo.lock`: the terminal and the phone resolve the SAME versions of the crypto crates (a separate lock
for the phone could silently skew them). `sshub-phone-core` is not a default member, so `uniffi` is never compiled for the
terminal. The skeleton's own `[workspace]` table and `Cargo.lock` in `crates/sshub-phone-core` are deleted at that point and
its two `#[path]` lines become `sshub-sync-proto = { path = "../sshub-sync-proto", features = ["qr-decode"] }`.

`cargo install sshub` consumes published crates, so **`sshub-sync-proto` must be published to crates.io** before `sshub`
(`sshub` depends on it with `path` + `version`). This changes the release flow (`just release`: publish proto first,
same version number, exact `=` requirement); it is the one cost of the plan and is listed in the open decisions. `sshub-phone-core`
is `publish = false`.

### 3.2 What moves into `crates/sshub-sync-proto` (`git mv`, content unchanged except where noted)

| from (`src/`) | to (`crates/sshub-sync-proto/src/`) | note |
|---|---|---|
| `sync/crypto/*.rs` (14 files, incl. `vectors.rs`) | `crypto/` | `words.rs` `include_str!` of `assets/wordlists/eff_large.txt`: move the file to `crates/sshub-sync-proto/assets/wordlists/` and fix its 3 other readers (`tests/oracles/crypto_oracle.py`, `assets/companion/build.mjs`, `assets/companion/src/lib.mjs`) |
| `sync/limits.rs` | `limits.rs` | |
| `sync/codes.rs` | `codes.rs` | |
| `sync/fountain.rs` | `fountain.rs` | |
| `sync/wire.rs` | `wire.rs` | uses `oplog::CertScope`, hence oplog types move too |
| `sync/share_release.rs` | `share_release.rs` | |
| `sync/snapshot.rs` | `snapshot.rs` | |
| `sync/oplog/{canonical,op,tier}.rs`, `oplog/vectors.rs` | `oplog/` | `tier.rs`: its `#[cfg(test)] mod tests` (needs `store::{ManagedHost,HostGroup,Identity}`) stays in `sshub` as `src/sync/oplog/tier_tests.rs` |
| `store/uid.rs` lines 1-45 (`Uid`, `HostUid`, `GroupUid`, `IdentityUid`, `FAVORITES_GROUP_UID`, `DEFAULT_IDENTITY_UID`, `uid_hex`, `uid_from_hex`) | `uid.rs` | `store/uid.rs` keeps the SQL lookups and `pub use sshub_sync_proto::uid::*` |
| `sync/enroll.rs` lines 30-195 and 314-327 (`COMMIT_CTX`, `TRANSCRIPT_CTX`, `INITIATOR`, `RESPONDER`, `Reveal`, `Confirm`, `commit_of`, `check_reveal`, `transcript_hash`, `sas_of`, `Party`, `confirm_message`, `terminal_confirm_message`) | `enroll_core.rs` | `enroll.rs` re-exports them; the SSH driver (`run`, `run_sync_enroll`, `run_sync_join`, anchors) stays |
| `sync/media/fsk.rs` | `media/fsk.rs` | plus NEW `fsk::encode` (port of `channels.mjs` `fskEncode`), pinned by the page's output |
| `sync/media/qrscan.rs` | `media/qrscan.rs` | behind feature `qr-decode` (`quircs`) |
| `sync/media/ble_link.rs`, `ble_link/` (after `poc/ble` is merged) | `media/ble_link.rs` | pure sans-IO; the phone uses `Link::phone` |
| decoder-only tests of `sync/media/tests.rs` | `tests/media.rs` | tests that use `listen()`/`ListenHandle` stay |

`pub(crate)` items that the rest of `sshub` uses become `pub` (with `#[doc(hidden)]` where they are test hooks): `from_seed`,
`derive_xof`, `dek_seal_with`, `hybrid_wrap_with`, `vault_recipient_with`, `exempt_slot_with`, `COMBINE_CALLS`, `split_at_indices`,
`reshare_contribute_with`, `seal_to_with`, `to_encrypted_with`, `wrap_keys`, `WrapKeys`, `mlkem::{EK_LEN, DK_LEN, CT_LEN}` and the
rest of `mlkem` used by wrap, `normalize` (words), `PaperDomain::context`, `encode_op_list`, `decode_op_list`, `seal_with`
(snapshot), `INITIATOR`, `RESPONDER`, `confirm_message`, `b64_encode`/`b64_decode` (canonical, `pub(super)` becomes `pub(crate)`
inside proto; they are not needed outside). The list is exactly the output of `cargo check` on the moved tree; no other
semantic edit.

Test vectors stay in `tests/vectors/` (the Python oracles and the page tests read them). The moved vector tests change
`env!("CARGO_MANIFEST_DIR").join("tests/vectors/...")` to `.join("../../tests/vectors/...")`
(`crypto/vectors.rs`, `oplog/vectors.rs`; `sync/vectors.rs` for quorum stays in `sshub`).

### 3.3 What stays in `sshub`

Everything that needs the store, SSH, the TUI or a device: `apply`, `ceremony*`, `cert.rs` (reads `LauncherStore`), `checkpoint_driver`,
`client`, `cred_ops`, `email`, `enroll` (the driver), `entity_capture`, `exemption`, `export`, `hub`, `identity_store`, `modes`,
`notify`, `oplog/{checkpoint,fork,ingest,merge,pending,store_impl,mod,tests}`, `phone_enroll` (terminal side), `protocol`, `quorum`,
`ratchet`, `recovery`, `reshare`, `revocation`, `runtime`, `screen`, `transport`, `ui_shares`, `unlock_paths`, `media/{mod,cam,mic,ble*}`,
`companion.rs`, `phone_fixtures.rs`, `vectors.rs`.

### 3.4 Re-exports: call sites change minimally

`src/sync/mod.rs` replaces the moved `pub mod` lines with

```rust
pub use sshub_sync_proto::{codes, crypto, fountain, limits, share_release, snapshot, wire};
```

and `src/sync/oplog/mod.rs` keeps its current `pub use` lines but sources `canonical`, `op`, `tier` from
`sshub_sync_proto::oplog`; `src/store/mod.rs` keeps `pub use uid::{...}` (now re-exported from proto). Every existing path
(`crate::sync::crypto::X`, `super::wire::Y`, `crate::store::Uid`) keeps compiling, so the diff outside the moved files is the
three `mod.rs`, `store/uid.rs`, `enroll.rs` and `Cargo.toml`.

### 3.5 Crate boundary and features

`sshub-sync-proto`: `std` only, no I/O, no threads, no clock, no global state (the ban already holds: "pure functions, no store").
Dependencies, the exact set of the root `Cargo.toml` crypto block: `anyhow`, `argon2 0.5`, `base64 0.22`, `bip39 2`, `blake3 1`,
`chacha20poly1305 0.10`, `ed25519-dalek 2`, `ml-kem =0.2.3`, `rand_core 0.6`, `serde_json 1`, `sha2 0.10`, `x25519-dalek 2`, `zeroize 1`,
plus `serde` for the enrollment structs. Features: `default = []`, `qr-decode = ["dep:quircs"]` (`sshub` and the phone enable it).
No `cpal`, `nokhwa`, `qrcode`, `rusqlite` or `uniffi` in proto. `[dev-dependencies]`: `hex`.

`sshub-phone-core`: `sshub-sync-proto` (+`qr-decode`), `uniffi`, `qrcode` (QR rendering), `serde`, `serde_json`. Features: `cli`
(builds the `uniffi-bindgen` binary), `testing` (seedable RNG).

### 3.6 Order of work and checks

1. `git mv` + path fixes as above; make `pub(crate)` items `pub` until `cargo check -p sshub-sync-proto -p sshub` is clean.
2. `verify.sh`: fmt, clippy, full test: the extraction must change no test result (the vector tests run in proto now).
3. `cargo ndk` build of `sshub-phone-core` (phase 1 toolchain, `android/ci/build.sh`) to prove proto stays Android-clean. Add
   `cargo tree -p sshub-sync-proto --target aarch64-linux-android` to CI: no crate from the "does not build" list may appear.
4. Page parity tests, e2e scenario 65 unchanged.

## 4. Keys at rest

Requirements: the 32-byte phone secret is never stored in plaintext, never leaves the Keystore except inside the unlock
window, and a stolen unlocked-looking phone is not enough.

- **Key**: Android Keystore AES-256 GCM key, alias `sshub.phone.secret.v1`, `PURPOSE_ENCRYPT | PURPOSE_DECRYPT`, `BLOCK_MODE_GCM`,
  `ENCRYPTION_PADDING_NONE`, `setRandomizedEncryptionRequired(true)`, `setIsStrongBoxBacked(true)` when
  `PackageManager.FEATURE_STRONGBOX_KEYSTORE` is present (fallback: TEE-backed; the About screen states which, from
  `KeyInfo.securityLevel`), `setUserAuthenticationRequired(true)` with
  `setUserAuthenticationParameters(0, AUTH_BIOMETRIC_STRONG or AUTH_DEVICE_CREDENTIAL)` (timeout 0 = per-use authentication, bound to a
  `CryptoObject`; the combination of strong biometric, device credential and `CryptoObject` needs API 30, which is why `minSdk` is 30),
  `setInvalidatedByBiometricEnrollment(false)` (UX review U13-6: adding a fingerprint must not destroy the phone's share; the device-credential
  fallback already bounds what that invalidation protected, see the residual below).
- **Stored**: `secret.bin` = `version u8 || iv(12) || AES-GCM(secret32) with AAD "sshub/phone/secret/v1" || ...`, plus the sealed state of
  2.1. Nothing else about the key is on disk.
- **Use**: each unlock shows `BiometricPrompt` with the `Cipher` as `CryptoObject`; on success Kotlin decrypts, hands the bytes to
  `PhoneCore.unlock` and zero-fills its copy at once. The core holds the secret in `Zeroizing` and wipes on `lock()`. The app locks when
  it leaves the foreground (`ON_STOP`), after 60 s idle on a flow screen, and when the device locks.
- **Changed security**: adding a fingerprint does NOT invalidate the key. Removing or resetting the secure screen lock still makes Android
  delete it (`KeyPermanentlyInvalidatedException` or a missing key). The app then offers
  "Restore from 24 words", exactly the page's restore path with head adoption (section 5). The words are the backup, not the Keystore.
- **No app PIN, no Argon2 of a PIN.** The page needs one because a browser has no secure element; the app uses the platform's
  credential and hardware. Residual: a device credential that is a short PIN is the phone's, not the app's, to harden;
  rate limiting is the Keystore's (lockout after failed attempts, enforced in the TEE/StrongBox).
- **Words**: shown only after a fresh authentication, on a `FLAG_SECURE` screen, never copied to the clipboard by the app.
- **Fallback when the device has no secure lock screen**: the app refuses to create a key ("set a screen lock first"); it does
  not fall back to an unprotected key.
- Residuals to state in the About screen: rooted or compromised OS can read the secret while it is unlocked in memory; the
  Keystore does not protect against that, the 7 and 14 day freshness limits and revocation do.

## 5. Migration page to app (same 24 words)

The page's "Show backup words" and the app's "Restore from 24 words" use the same BIP39 words: same secret, same keys, same
device id. Therefore the page and the app are ONE device on the ring, and two of them signing is a fork (proof of
compromise). Rules (C21):

1. **App restore** = restore on a fresh store: `head_pending = true`, the app does not sign anything.
2. **Head adoption**, same as the page: ask any terminal for this phone's own newest ops (kind 5 list as QR, BLE, text or paste), `take_chain`
   verifies that every op is by this key, every signature, one chain, continues after the newest (seq + 1, lamport above, epoch
   the highest seen). Only for a key that never signed anything may the user confirm `never_signed`.
3. **The page must stop signing before the app starts.** The app's restore screen asks "Did you use the companion page in a browser on this phone before?". Only on Yes it shows the steps (open the page's menu, take
   "Show backup words", then clear the page's site data) and the required "I cleared the site data" box; on No there is no box. After adoption the app shows a one-time warning
   that any signature the old copy makes later will look like a fork. Order that is safe: (a) finish or abandon any release on the page,
   (b) read the words, (c) wipe the page's data, (d) restore in the app, (e) adopt the chain. If (c) is skipped and the page signs after (e), the terminal
   sees two chains with the same `seq`: fork evidence. The terminal cannot tell page from app; nothing in C21 changes that.
4. The app never imports the page's `SSHP` blob (it is PIN + Argon2 protected browser storage, not reachable from an app); the words are the only bridge.
5. Enrolled terminals, aliases and short-code lists are NOT migrated (they were local to the browser): the ring holds the
   truth (the phone is already enrolled by key), unknown requesters are met again, live snapshot blobs on terminals stay valid until expiry, and the
   app lists none of them (it cannot know). It says so on the restore screen.

## 6. Channels

| channel | how | where decided |
|---|---|---|
| Camera QR (in) | CameraX `ImageAnalysis`, `STRATEGY_KEEP_ONLY_LATEST`, 1280x720 YUV_420_888; the Y plane goes to `PhoneCore.scan_gray`, which runs the Rust decoder (`quircs`, the decoder `sshub` uses for its camera path) and the fountain decoder; progress events drive the UI | Rust |
| QR (out) | Rust fountain-encodes the reply (frames of the page's size, 4 fps, 25% parity) and returns module matrices; Compose draws them with whole device pixels per module on a white field, max brightness, looping | Rust + Canvas |
| Bluetooth LE | native GATT **central** for contract C20 (terminal = peripheral): scan with `ScanFilter(ParcelUuid(service))`, never accept-all; pick by `SSHub-<TAG>` in an own list (the same "person picks the tag" property); `requestMtu(517)`; `startNotifications` (enable CCCD on TX) before the first write to RX; the link layer (`Link::phone`, chunking, window, ACK, timers) is Rust and sans-IO, Kotlin only moves GATT writes/notifications and calls `tick` | Rust link, Kotlin radio |
| Paste / share text | frames as base64 text: system clipboard read on a button (no background clipboard access), share sheet (`ACTION_SEND`) and copy for output; clipboard output marked `EXTRA_IS_SENSITIVE` and cleared after 60 s; optional share target for `text/plain` into the paste screen (never auto-processed: the view and the decision are always shown) | Rust parse |
| Sound | Rust `fsk::encode` returns PCM, `AudioTrack` plays it at the fast (about 8 s) or slow (about 16 s) profile; sound is opt-in per approval as on the page; no `RECORD_AUDIO` | Rust modem |
| By hand | index and 24 words | Rust |

**Why the Rust decoder and not ML Kit or ZXing:** one decoder tested on the same fixtures as the terminal (`media/tests`, the fountain
decoder is fed the exact same bytes), no Google Play Services dependency (the app must run on de-Googled phones and install from F-Droid),
no proprietary blobs, and the terminal's reverse-QR measurements of module size and frame rate transfer. Cost: `quircs` is a small
decoder; the camera path therefore hands it a full-resolution luma plane and the app falls back to the page's lessons (large modules,
max brightness). If field tests show `quircs` loses to ZXing on real phones, swapping the decoder behind `scan_gray` is local to one
function; this is flagged, not assumed.

All channels carry the same frames (kinds 1 to 10); no frame is channel-specific, so a frame from BLE, a QR, a sound or paste goes through
the same `open_frame`.

## 7. Signing and the app fingerprint

- **Debug / test**: the public Android debug key, committed at `android/debug.keystore` (password `android`, alias `androiddebugkey`) so
  every debug build and the emulator tests carry the same certificate. It is public by design and the doc says so; the terminal
  never accepts it.
- **Release**: the maintainer's key, generated and kept by him, never in the repo or on gpu-serv. Recommended: a 4096-bit RSA or EC
  P-256 key in a PKCS12 file kept offline, APK Signature Scheme v3 (v2 + v3 so a later key rotation works through a proof-of-rotation lineage),
  `apksigner sign --ks <maintainer file>` run by him on the unsigned release APK produced by the reproducible build, then `apksigner verify --print-certs`.
  The build container produces `app-release-unsigned.apk` only; no signing material enters CI.
- **Fingerprint shown by sshub**, like `COMPANION_FINGERPRINT`: `sha256` of the DER-encoded signing certificate, lowercase hex, the
  value `apksigner verify --print-certs` prints as "SHA-256 digest" and the app shows from `PackageManager.getPackageInfo(GET_SIGNING_CERTIFICATES)`
  on its About screen. The terminal ships it as `COMPANION_APK_CERT_SHA256` in `src/sync/companion.rs` (from `assets/companion/APK_CERT_SHA256`,
  one file, written when the maintainer first signs a release) and shows it next to the page fingerprint with the same compare hint.
  What it proves, honestly: that the installed app is signed by the holder of the maintainer key (a repackaged app has another
  certificate). It does not identify a build version (the APK sha256 and `versionName` are in the release notes) and does not stop
  a malicious app signed by the same key. After a key rotation both fingerprints are listed.
- The same model as the page: the terminal's value is the reference; compare with the app's About screen.

## 8. Reproducible build

Everything runs in a pinned container on gpu-serv; nothing is built on the laptop.

- Image `sshub-hostsync-androidbuild:local`, `android/docker/Dockerfile` (the only definition): `ubuntu:24.04` by digest; Temurin JDK 17.0.21+9
  (sha256 checked); Gradle 8.11.1 (sha256 checked; only to generate the wrapper, builds use `./gradlew`, whose `distributionSha256Sum` is
  set); Android cmdline-tools 13114758 (sha256 checked); `platforms;android-35`, `build-tools;35.0.0`, `ndk;27.2.12479018` (r27c) through
  `sdkmanager` (verifies Google's manifest checksums); rustup 1.28.2 (sha256 checked) with Rust 1.99.1 and the two Android targets;
  `cargo-ndk =4.1.2` (`--locked`). The resolved versions are written to `/opt/VERSIONS.txt` and printed in every log.
- Rust: `Cargo.lock` committed, `--locked` everywhere; `RUSTFLAGS="--remap-path-prefix=/work=/src --remap-path-prefix=/cargo-home=/cargo -C link-arg=-Wl,--build-id=none -C link-arg=-Wl,-z,max-page-size=16384"`
  (no absolute paths, no build id, 16 KiB page alignment as Android 15 and the Play requirement need).
- Gradle: plugin versions pinned in `build.gradle.kts`, dependency versions pinned inline, and Gradle dependency verification is ON:
  `android/gradle/verification-metadata.xml` holds the sha256 of every artifact the build, the release variant and the instrumented tests resolve
  (about 500 components). A changed or new artifact fails the build. To refresh after a dependency change, run on gpu-serv
  `GRADLE_EXTRA="--write-verification-metadata sha256" TEST_CLASS=uk.petruha.sshub.phone.app.AppRootTest gpu-verify/android.sh <checkout> <name> test`
  (it covers assembleDebug, assembleRelease and connectedDebugAndroidTest), review the diff of the XML and commit it. Signature verification is not used (sha256 only).
- Reproducibility (checked 2026-10-26): `app-release-unsigned.apk` built twice in two clean containers (separate source copy, cargo target and Gradle project
  dirs; only the download caches are shared) is byte for byte identical: 541 entries; first check (49dd4cc) sha256 75b128a71af436f9dc62e8680683483a19156c23148c48524fe3f807eb36f4a3, re-check after the core additions (runs rc and rd) sha256 6d060ba07f781685b8d6335775c59c7e9e83c6b4f77b65f04551e187d221fef5 (`android/ci/apk-diff.py A B` lists differing
  entries). Signing adds only META-INF. The inputs that make it so: pinned image, `--locked`, `--remap-path-prefix`, no build id, `-z max-page-size=16384`, AGP's fixed zip timestamps.
- R8 / minify is off. It is a later option to shrink the APK (about 31 MB unsigned release today); it needs keep rules for uniffi and JNA and a re-run of the reproducibility check.
- Driver: `gpu-verify/android.sh <checkout> <name> bootstrap|build|test|chrome` syncs the checkout, builds the image, runs
  `android/ci/build.sh` as uid 10001 with a target dir and caches under `/root/sshub-ci/android-app/`, logs to
  `gpu-verify/logs/android-<name>-<UTC>.log`, registered in `/agents/host-sync-sshub.md`.
- Check "reproducible": build twice in different target dirs and compare `classes*.dex` and `lib/*/*.so` (and the unsigned APK entry hashes
  ignoring `META-INF`); signature blocks differ by design. This check is a CI step of phase 2 (not claimed in phase 1).

## 9. Tests

1. **Core unit tests** (`cargo test -p sshub-phone-core` on the host, inside the existing Rust container): authoring and parsing of
   every op, refusals table of the companion doc, head adoption, cert limits, reshare adopt.
2. **Shared vectors**, one source of truth: `tests/vectors/{crypto,op,quorum}` (already run by `sshub-sync-proto`) and
   `assets/companion/test/fixtures/{challenges,enroll,ops.vectors,reshare}.json` (emitted by `phone_fixtures.rs` with the real encoders).
   The core's integration tests load the same JSON and assert byte-identical outputs (Ed25519 and the canonical encoders are deterministic;
   randomness is injected). The page tests (`assets/companion/test/*.mjs`) keep running; while both phones exist, a regression in either
   fails against the same files. Phase 1 already pins one of them: the Compose test asserts the device id `ee8a1d5cf97016a7` of
   `challenges.json` `phone.pub`.
3. **Compose UI tests** (`androidTest`, `createAndroidComposeRule`) on the emulator service: restore, approve a release from the fixtures'
   challenge text, refusal screens, lock behaviour, permission manifest check.
4. **Emulator**: the shared `sshub-hostsync-android` service (Android 15 Google Play x86_64, KVM). `android.sh ... test` builds
   first (no emulator, no lock), then takes `flock /root/sshub-ci/android.lock`, brings the project up, runs `connectedDebugAndroidTest` in a container on its
   `_e2e` network and tears the project down. The emulator container runs the adb SERVER on `android:5037` (internal network only); the guest adbd is a
   secure user build that authorizes only that server, so clients use `adb -H android`, a socat relay to `localhost:5037` for Gradle, or Playwright
   `_android.devices({host:'android',port:5037})`; a direct `adb connect android:5555` fails with "failed to authenticate". The test container also
   joins the default bridge because Gradle resolves its test-runner artifacts at that point. Never a physical device.
5. **E2E with the terminal**: the existing headless phone driver (`deploy/e2e/phone-driver.mjs`, commands `enroll-*`, `red-*`, `share-*`,
   `verify-ops`; its `targets` table) gets a second target `android`. The app adds NOTHING to release builds. The `debug` build type
   adds (a) `testTagsAsResourceId`, so every control is a UI Automator resource id, and (b) a debug-only broadcast receiver for
   "paste this file" and "write the shown text to a file" (frames can be 100 KB; `adb shell input text` cannot carry them). The driver
   reads the reverse QR with `adb exec-out screencap -p` plus the decoder it already has. Camera input for the emulator uses its
   virtual scene when a test needs the camera path; the CLI text channel (paste) covers the ceremonies. This is the mapping of every
   existing scenario to the app; scenario 65 runs once against each phone.
6. **Manifest and size gates**: permission list, no `INTERNET`, `allowBackup=false`, `.so` alignment (`zipalign -c -P 16`).

## 10. What the terminal needs

Nothing in the protocol: the wire, frame kinds, channels and ceremonies are unchanged (C21). The only change is cosmetic and
safety related: show `COMPANION_APK_CERT_SHA256` with the page fingerprint (TUI phone screens, `sync-status` JSON key
`companion_apk_cert_sha256`, `sync-phone-*` verbs' first lines), and document it. The CLI verbs and the TUI already take frames
as pasted base64 and as BLE frames, which is exactly what the app produces.

## 11. Phase 1 result (this branch)

- `docs/host-sync-android.md` (this file) and contract C21.
- `crates/sshub-phone-core/`: standalone skeleton, one uniffi function `probe(secret)` over the unmodified `src/sync/crypto` and
  `src/sync/limits` (device id, BIP39 words, BLAKE3 known answer, strict Ed25519 verify), with host unit tests pinned to the `challenges.json` vector.
- `android/`: Gradle 8.11.1 wrapper, AGP 8.10.1, Kotlin 2.1.21, Compose BOM 2025.05.01, `minSdk 30`, `compileSdk 35`, one unstyled screen and a
  Compose test; `android/docker/Dockerfile`, `android/ci/build.sh`; driver `gpu-verify/android.sh`.
- Measured (logs `gpu-verify/logs/android-*.log`, gpu-serv under load average about 19):
  JDK Temurin 17.0.21+9, Gradle 8.11.1, AGP 8.10.1, Kotlin 2.1.21, Compose BOM 2025.05.01, NDK 27.2.12479018 (r27c), build-tools 35.0.0, platform 35,
  platform-tools 37.0.1, Rust 1.99.1 with the two Android targets, cargo-ndk 4.1.2, uniffi =0.32.2, JNA 5.17.0. Host unit tests of the core: 42 passed
  (the committed crypto vectors included). Debug APK `app-debug.apk` 25.6 MB (unminified Compose, debug; the Rust `.so` is 0.6 MB arm64 and
  0.7 MB x86_64), test APK 2.3 MB. Emulator: Android 15 (API 35) Google Play x86_64, emulator 37.2.12, boot to healthy 25 to 91 s, Chrome 124.0.6367.219.
  Instrumented test: 3 tests on `emulator-5560 - 15`, all passed (device id vector, BLAKE3 known answer, strict Ed25519 verify). `android.sh ... chrome`:
  Playwright 1.64.1 reaches Chrome through the emulator's adb server and loads a page (see section 9).
- Playwright and Chrome on the emulator: `device.launchBrowser()` does not work on this image (Chrome ignores the command-line file on a user build, so the
  `--remote-debugging-socket-name` socket never appears); `android/ci/chrome-probe.mjs` answers Chrome's first run through uiautomator, relays
  `chrome_devtools_remote` with `device.open` and attaches with `chromium.connectOverCDP`. A `data:` page is not a secure context: the page target will serve
  the companion page on `http://localhost` through an adb reverse (still to do, needs `reverse:` over the adb server).

## 11b. Phase 2 result: the Kotlin side

- Code: `android/app/src/main/java/uk/petruha/sshub/phone/`: `ui/theme` (spec-A tokens dark and light, bundled fonts, reduced motion), `ui/components`
  (about 30 components, Canvas glyphs, no icon font), `ui/screens/{identity,home,release,trust,recovery,enroll}` (all 35 screens of spec 12, stateless
  `XScreen(state, actions)` plus a ViewModel and a nav builder per group), `core/` (the Kotlin `PhoneCoreApi`, ports in `Platform.kt`, and
  `core/uniffi/UniffiPhoneCore`, the adapter over the Rust core), `platform/` (Keystore AES-GCM vault with StrongBox and BiometricPrompt, atomic sealed-state
  file in `noBackupFilesDir`, CameraX luma frames, BLE GATT central for C20 driving the Rust `BleSession`, AudioTrack FSK playback, sensitive clipboard with
  the 60 s clear, FLAG_SECURE helper), `app/AppRoot` (nav host, bottom bar, lock on stop and screen off, theme choice).
- Debug builds run the REAL core (the debug APK, signed with the committed public debug key, is what is installed for a device session); the Kotlin
  `FakePhoneCore` (src/debug) is for instrumented tests only (the `core=fake` extra is ignored outside instrumentation); the debug app id is `uk.petruha.sshub.phone.debug`. Release builds always use the Rust core.
- CI (`android/ci/build.sh`): host tests of the core, `cargo ndk` with `--profile phone`, uniffi bindings, `lint.sh` (no `Color(0x` outside the theme, no
  `Ellipsis` or `maxLines` in ui, no `Log.` in platform and core), `manifest-check.sh` (permissions exactly CAMERA, BLUETOOTH_SCAN neverForLocation,
  BLUETOOTH_CONNECT, USE_BIOMETRIC, HIDE_OVERLAY_WINDOWS; no INTERNET; backup and cleartext off), debug APK and test APK.
- Tests on the emulator (`gpu-verify/android.sh <checkout> <name> test`, optional `TEST_CLASS=a.B,c.D`): 563 instrumented tests: every screen's main path and
  variants, dark and light, font scale 1.3 and 2.0 with a real-clipping check, 48 dp targets, the dangerous-field rules (D1 to D20 where they apply), the
  platform adapters, the real native core (public vectors: device id, words round trip) and every challenge fixture of the page through the adapter
  (`FixtureFlowTest`), and the app shell over the fakes. Each tested state leaves a screenshot (`gpu-verify/logs/android-<name>-out/shots`).
- Known limits of what the Rust core reports (the UI hides what it cannot fill, see 2.3): what remains is listed under 2.3 ("What the Rust core cannot fill"): ring hosts, credentials, roles and last sync, a requester
  key for kind 9 and 10 requests (by design), `IssuedCert.lamport`, `state_lost`.
- Not done: R8 (later size option), and screens for scenes that need a second device (BLE against a real terminal,
  camera against a real screen): covered by fakes and by the e2e target, not by this run.

## 12. Plan after phase 1

1. Maintainer picks the visual direction.
2. Integrator performs the extraction (section 3).
3. Core: identity, state, chain, describe, release, cert, reshare, enroll, channels; each lands with its fixtures.
4. UI per flow on top of the finished core; BLE after `poc/ble` merges.
5. Reproducibility check, signed release candidate by the maintainer, terminal fingerprint const.

## 13. Open decisions for the maintainer

1. **Release signing**: key type and storage (recommendation in section 7), who signs, and the fingerprint file.
2. **Distribution**: GitHub release APK (sideload, verify with the printed cert), F-Droid (fits the reproducible build and no-Google
   stance; needs a metadata recipe), Google Play (Play App Signing replaces the signing certificate: the terminal fingerprint would
   have to be Play's, and the camera/BLE permissions need a store review). Recommendation: GitHub release now, F-Droid next, Play only if wanted.
3. **minSdk**: recommended 30 (Android 11): per-use authentication with a strong biometric OR the device credential on a `CryptoObject` needs it.
   With 26 the app would need a weaker, time-window authentication for devices below 30.
4. **Publishing `sshub-sync-proto` to crates.io** with every `sshub` release (the cost of the extraction). The alternative is to leave the sources in
   `src/sync/` and `#[path]`-include them from the phone core, as the phase 1 skeleton does for `crypto` and `limits`; that needs a small shim per module
   that touches `crate::store` (`oplog`, `wire`) and keeps the phone core tied to the terminal's source layout. Recommendation: extract and publish.
5. **Page parity period**: how long the page keeps feature parity (new ceremonies land in Rust first; the page port becomes optional).

## 14. Review round A1 notes

- Taking a restored phone's chain: the kind 5 list names its terminal (an unsigned id); two lists from two DIFFERENT terminals naming the same head are needed; the same terminal twice is refused with the core's sentence (BadInput, the token stays); the id is not proof of who sent the list (a restored phone has no ring to verify against). `ChainAdopted.sender` is that id, null when the list did not say.
- Per-signature system lock: `UniffiPhoneCore.vault` is set by the shell; every mutating call (decide on Approve for releases, Red, trust and reshare, revokeCert, rotateWrapKeys, decideRecoveryCancel, enrollDecide on the SAS match, adoptChain, neverSigned) asks the vault once before the core is touched. Decline and cancel paths are not gated. Title is the action, subtitle a short id or hash. A test lists every `PhoneCoreApi` method as gated or reviewed. The screens no longer prompt themselves.
- `decide` finishes once the core is asked (NonCancellable). `ReleaseApi.lastOutcome(token)` returns the last Approve outcome until the phone locks, so a recreated screen can show it again. It holds the reply (QR frames, by-hand words) in memory until lock.
- Unreadable saved state: unlock keeps the old bytes as `state.bin.lost`, writes nothing, and the shell stays on a stop screen until the user chooses; every core flow refuses meanwhile.
- Core calls never run on the main thread (platform M6). `UniffiPhoneCore` runs every call into Rust on ONE `sshub-core` thread, in order. The reads that need the core are `suspend` (`status`, `ring`, `thisPhone`, `unlock`, `restoreFromWords`, `backupWords`, `frameFromText`, `enrollTextKind`, `confirmStateLost`); `lock` and `resetScan` are queued and return at once (the Kotlin cache is dropped immediately); `pending` reads only the Kotlin cache. `scanGray` runs on the camera thread and throws in debug builds if called on Main. Debug builds also throw when a guarded core call is made on Main. The tab badge is loaded in a LaunchedEffect, not during composition. Screens that need the status show without it until the core answers (empty name and id), Home draws nothing until then. A short-code release still holds the core for seconds on a phone; the UI stays responsive meanwhile (CoreOffMainTest).

## 15. Review round A2: screens and view mapping

- Share release: the host uid (groups of 4, the same text as the short-code and trust lists) and every op row with its `danger` flag are drawn above the four words; the note is "Compare the host and the four words with the terminal.". A request on a restored phone (`head_pending`) shows a stop box up front and Approve names "take the chain first".
- Release channels: QR on, one sound choice (off, fast or slow, off by default), no clipboard channel; "Copy as text" is always offered; the card says "Shown as: ...", never "Sent"; Approve is disabled with no channel and no by-hand. `ReleaseOptions.by_hand` is passed through.
- Red: a proposer never met needs its device id typed (`typedId` goes to the core); only this phone's own proof is a state, the rule of three proofs is one sentence. Banners, tick labels, headings and the header stack their trailing text from font scale 1.3 so no word breaks inside a token (the 2.0 tests fail on a broken word).
- Reshare: holders afterwards (this phone marked), the contributor flags (signed, known, session key) as check lines, whether a storage key is published; Adopt no longer depends on a rotation in this process.
- Trust: the extension shows the old allowance and the new one for the next 24 hours (`N` to `N + E`), or the extra as a plain row; a declined request offers Back, not "Review again" (the core dropped the token). The review shows this phone's own proof square and says no hash exists.
- Recovery notice: every op row with its danger flag; the sentence is "A phone recovery was proposed. If you did not start it, cancel it."; the signed state says "Cancellation signed", a revocation says "Revocation signed".
- Home and menu: the head-pending card and "head seq: unknown, not taken yet" follow the status, with or without a ring; the Security menu lists "Take my chain from a terminal" while it is pending.
- Enrollment: the refusal kind comes from the core (`RefusalKind`): a mismatch is the stop box (compared values only when present), other stops are titled with the core's sentence, a wrong-order or stray frame is a note. Pairing over Bluetooth is off and warned when two devices show the typed tag. The name rules are the core's `check_enroll_name`, asked before the scan; Scan stays disabled while there is a problem.
- Export: a dangerous op carries a glyph and the word; hashes are shown in groups of 4. The 24 recovery words and the words paste box use the password input variation.
- Tests: `RealCoreScreensTest` and `RealCoreNoticesTest` feed the screens from the real core over challenges.json, enroll.json and notices.json; `ScanRoutesTest` registers the graphs and presses every scan button; `RestoredPhoneTest` restarts a restored core; `WordsKeyboardTest` reads the EditorInfo.

## 15. Review round 2 notes

- The core seals its state on `lock()`: lock then unlock in one process opens the same ring, head and certificates (test `SealedStateRoundTripTest`). A failed save after a decide locks the core and the vault and says "The phone state could not be saved; the request was discarded."; the kept Kotlin cache follows `pending_tokens()`.
- The gate fails closed: without an attached vault or block check every signing call is refused. `lastOutcome` keeps only the reply frames and the note (never the short code or the by-hand words), newest 8, cleared on lock.
- D17 signer block: typing only compares. Closing the app needs the tick "I compared the value digit by digit", the button, and the system lock. While blocked, a matching value plus the system lock reopens the app (`clearSignerMismatch`). Only Lock and About stay open under the block (no first run, no restore). Reinstalling deletes the key and the saved state: the 24 words are needed.
- `SecureWindow` never clears a FLAG_SECURE the Activity already holds. A QR keeps the screen lit for at most 2 minutes, then the normal screen timeout and the auto-lock apply; in landscape the code is capped to the screen height.
- CI: `build` (the release APK) starts from clean cargo outputs and Gradle build dirs; the manifest check runs on the release APK too (non-debuggable, HIDE_OVERLAY_WINDOWS present) and gates the focused run; the uniffi-bindgen cache key includes the uniffi packages of Cargo.lock.
- Not done: the blind clipboard clear can wipe a clip another app put there while the window has no focus (needs a device to confirm the changed-clip listener); `settings.txt` is still written on the calling thread (the D17 tests rely on write-then-read ordering); the platform and core are still built in `onCreate`; flows L1 to L3, L11 and L12 and the remaining low items beyond the ones listed in the commit message.

## 16. UX batch A

### Home notice, reshare name, paste Finish, glossary

- Home: `HomeState.updateNeededAfter` (device name) draws the card "Update needed after removing <device>" with "Update this phone" (`HomeActions.onUpdateShares`, routed to `Routes.Reshare`). `HomeViewModel` takes `updateNeeded: StateFlow<String?>` (default none) and `homeGraph(env, router, updateNeeded)` passes it on. Wiring still owed outside Home: `AppRoot` passes `settings.updateNeededAfter` to `homeGraph`; the Red approve of a removal calls `settings.setUpdateNeededAfter(name)` and the reshare adopt clears it with `setUpdateNeededAfter(null)`.
- The Reshare screen is titled "Update this phone after a device was removed" (`RESHARE_TITLE`). It says "Passwords ready for this phone: N of M"; the key generation, the sharing round and the full device number sit behind a collapsed "Technical details". Buttons: "Renew the phone keys"; "full devices" replace "holders".
- Paste enrollment no longer skips Finish: after the terminal confirmation is pasted the phone shows step 5 (Finish, confirmation received) and only its Finish button opens Enrolled, as on QR. Test `EnrollPasteFinishTest`.
- The mismatch box labels the two values "The terminal showed first" and "The terminal now shows", both in full in groups of 4, with "These two values must be the same. They are not: someone may be interfering. Nothing was enrolled."
- Glossary: steps are "Terminal code", "Phone code", "Terminal keys", "Compare the words", "Finish" with "Step N of 5" on every enrollment screen; the screens are titled "Connect to <terminal>" and Home says "Connect to a terminal"; "the four check words", "full device", "your devices".

### Approval screens: one timer, scan the device id

- One timer: the Red and share release screens show a single "expires in ..." line (`ExpiryLine`): the request's own time (`ReleaseView.expiresAt`, `RedView.lapsesAt`), as `1:58` under an hour, `5 h 12 min` under a day, `2 days 4 h` beyond (`expiresText`). The 12 minute screen window is still a rule (`closed`, `WindowClosedBox`) but is written out only when it ends before the request does (`RedState.windowIsTimer`, `ReleaseState.windowIsTimer`); otherwise it is not displayed. The spoken live region stays on that one line (`spokenWindow` for the window, the 120/60/30/10 steps for a share release). The card countdown and "expires after N s" are gone.
- Scan the device id: the typed-id field of Red, share release and trust request has a "Scan its QR" button (`TypedIdField`, `RedActions/ReleaseActions/TrustRequestActions.onScanId`) that opens the scan screen with `ScanPurpose.DeviceId` ("Scan the device id"). The QR payload is plain ASCII `sshub:dev:1:<16 hex>`; `parseDeviceIdPayload` accepts any case and spaces or dashes inside the hex. `ScanViewModel` does not call `openFrame` for this purpose: it publishes `DeviceIdScanned(id)` and the calling screen fills its field (`ConsumeScannedDeviceId`). A wrong code shows "That is not a device id code from SSHub." and scanning goes on. The core still re-checks the typed id.
- Core owed: `scan_gray` (`ffi.rs`) drops a plain QR (its payload is not a fountain frame, so the collector counts it as damaged and no event is returned). It must hand a payload that starts with `sshub:dev:1:` through as `ScanEvent::Document { bytes }` (skipping the `is_sshub_frame` filter for that prefix); the Kotlin side already reads those bytes from `ScanEvent.Frame`. The paste path of the scan screen takes the text directly and works today.
- Glossary on these screens: "Big change" (title of the Red screen, banner word kept), "approvals" for the three proofs, "full device", "trust session" for certificate, "sharing round" for epoch, "paper phrase or emailed code".

### Backup check, first run, new phone

- Backup check: before the first approval the phone asks for two random words of the 24 (`BackupVerifyScreen`, route `backup-verify`, `pickCheckWords`). The words come from `backupWords()` behind the system lock; only the two needed ones are kept until the check ends. A wrong word shows "That is not word no. N. Look at your paper again." and a "Show the words again" button. The flag `SettingsStore.backupVerified` (`verified=1` in the settings file) is sticky and is set only by a right check or by a restore from the 24 words (all 24 were typed). The tick box "I wrote them on paper" stays: its Done leads to the check while the flag is false (first run: words, tick, two words, Home).
- The gate is navigation only: one `LaunchedEffect` in `AppRoot` opens `backup-verify` on top of an approval route (`Routes.approvalBases`: Red, release, trust request, trust extension, reshare, recovery notice, enrollment compare) while the vault is unlocked and the flag is false. Back on the check pops it and the approval screen with it, so Back reaches the list instead of reopening the check. The Approve buttons are not touched.
- Update marker for another screen: `SettingsStore.updateNeededAfter` / `setUpdateNeededAfter(name)` (`updatefor=<name>`, blank or absent is null).
- First run says plainly that with one computer there is no backup of the host list (this phone and the paper words hold pieces of passwords, not hosts) and to add a second computer or a view-only copy before relying on sync.
- Security menu: "Recovery and a new phone" is two rows. "I am getting a new phone" (`onNewPhone`, route `new-phone`, `NewPhoneScreen`) says what works today: the 24 paper words on the new phone, then its history from a terminal; no direct pairing with the old phone yet; your devices accept one phone at a time, so remove the old one. "I lost my phone" opens the recovery screen (title now "I lost my phone").
- Glossary on the identity strings: "full device", "trust sessions", "update the shares", "phone key generation", "your devices" instead of share holder, certificates, reshare and wrap keys, wrap key generation, ring.
