# Host Sync: P2P synchronization of hosts and credentials (design)

Date: 2026-07-11. Status: v3, after three rounds of adversarial review;
all blockers from the second round fixed. Translated from the Russian
original for publication; tracked as epic
[#13](https://github.com/Petyok/SSHub/issues/13).
Branch: the whole feature is built on a single feature branch
(`feature/host-sync`) and released as one piece.

## One-line idea

P2P synchronization of hosts between a user's machines over SSH itself (no
servers, no daemons), with a Bitcoin-inspired security model: signed
hash-chained op-logs, danger tiers for operations, a Shamir quorum for
credential release, the phone as a first-class holder of key material (PWA),
and an email code as key material for tier-1. Compromising a single peer
gives the attacker no credentials and no way to rewrite history unnoticed.

## Threat model

A compromised peer is an active attacker, not just a data leak:

- tampering with `address`: MITM of SSH connections;
- tampering with `proxy_jump`: traffic routed through the attacker's machine;
- tampering with `remote_command`: code execution on every connect;
- bulk exfiltration of credentials with a single request.

Design consequences: foreign changes are **never** applied silently; a single
peer is **mathematically unable** to decrypt credentials; history rewriting
is detected by all peers.

### Accepted residual risks (the honest list)

- **Genesis:** if the first machine is compromised at the moment the network
  is created, no scheme detects it.
- **Tier-3 leaks on any compromise:** metadata (addresses, proxy_jump,
  notes) is reconnaissance data; a compromised peer holds a valid signature
  and can read it.
- **Trust-session window:** an attacker on a blessed device can pull tier-2
  credentials within the rate limit for the duration of the window (see the
  TTL cap and limits below).
- **Revocation is not retroactive:** a revoked device keeps the shares and
  plaintexts it already received; revocation protects the future (DEK
  reissue), not the past.
- **Revocation is not instant:** it propagates on sync; the window between
  revoke and delivery is narrowed by requiring share holders to check chain
  freshness before any release.
- **Compromised peer + captured mailbox:** full takeover of both lets the
  attacker wait out the recovery timelock; the user sees the
  delay-and-announce alert but can only cancel with a live phone or by
  recovering the mailbox out of band.

## Operation tiers

Tiers apply to **incoming requests from other devices** and to the release
of key material. Local edits on your own device (creating a host, changing
your own credential) need no ceremony: the device originates the secret
rather than requesting someone else's; local data is protected by the
passphrase/keyring.

| Tier | Operations | Requirements |
|------|------------|--------------|
| 3 | reading metadata (full host list, host N), no credentials | valid peer signature, served immediately |
| 2 | releasing a Shamir share (= credentials of one host); applying a foreign host modification/deletion | signature + op-log + user approve; share release additionally requires phone participation (share release) or a valid delegation certificate |
| 1 | bulk credential export; adding/revoking a device; trust-session; phone re-enrollment | phone participation + a second device's signature + email code (as key material); for phone loss, a recovery path with a timelock (below) |

### Principle: factors are bytes, not checkboxes

There is no trusted server in P2P. A boolean check ("code matched, let it
through") on a compromised peer is simply skipped. Every factor must be
**key material**: without its bytes, decryption/signing is mathematically
impossible, not "forbidden by policy".

## Architecture

### Device identity

- Every device (including the phone): **Ed25519** (signatures) + **X25519**
  (encryption/wrapping shares; Ed25519 does not encrypt, so either a separate
  pair or a deterministic ed-to-x conversion, identical across platforms,
  including `@noble` on the phone).
- Private keys stored locally under a passphrase/keyring.
- Identity = pubkeys + a human-readable name (`alice-laptop`).
- The `devices` table replicates as part of the synced data.
- Adding/revoking a device is tier-1.

### Transport and threading model

- No daemons, no ports: `ssh <peer> sshub sync-serve` runs the sync protocol
  over stdin/stdout, modeled on `git-upload-pack`. A peer is a regular host
  from the sshub list, marked as "my device".
- Protocol: length-prefixed JSON.
- **The sync client lives in a background thread** (same pattern as the file
  watcher: thread + `mpsc` into the event loop), so the TUI never freezes.
  SSH to peers is strictly `BatchMode=yes` (no interactive prompts inside
  raw mode); if a peer is unreachable without interaction, it is offline.

### Op-log (the Bitcoin part)

- Every mutation is a signed op:
  `{device_id, seq, lamport, prev_hash, timestamp, epoch, op_body, signature}`.
- Each device owns its chain; peers store copies of other chains and their
  heads (`chain_heads`). Sync = exchanging chain tails.
- **Fork detection strictly by proof:** the `compromised` verdict and the
  full-screen alert fire only on cryptographic proof of equivocation,
  verifiable by any peer: **two signed ops with the same `seq` and different
  hashes**, OR **two signed head attestations of one device where neither is
  an ancestor of the other** (otherwise a careful attacker would equivocate
  via attestations and sit in `suspect` forever).
- **Anti-equivocation (n>=3):** on every sync, peers exchange **self-signed
  head attestations** of third devices (a head signed by its owning device).
  Diverging gossip yields `suspect` status plus a direct check on next
  contact, NOT a red screen: rumor without proof must not defame an honest
  device (otherwise gossip itself becomes a DoS vector).
- Fork detection is the secondary mechanism: a compromised peer's primary
  attack is not rewriting history but appending valid malicious ops; that is
  closed by pending-ops review plus the tiers.
- **Conflicts (two ops on one object in different chains):** deterministic
  tie-break `(lamport, device_id)` at the protocol level, with two defenses
  against lamport inflation by a compromised peer: (a) **a jump cap**: an op
  whose lamport outruns the network's observed clock by more than a threshold
  is rejected as invalid; (b) **a revoke op never loses** an auto tie-break;
  any conflict against a revoke always goes to manual review.
  A credential op is atomic: the ciphertext plus all wrapped shares travel in
  one op, so the state "ciphertext from DEK1, shares from DEK2" cannot exist.

### Epoch ratchet (with counterparty entropy)

- `epoch_key(n+1) = HMAC(epoch_key(n), nonce)` where `nonce` is random and
  generated by the counterparty during sync. A deterministic date-based
  ratchet is worthless: a disk clone would compute all future epochs itself.
  With the nonce, a clone that misses even one sync of the original with any
  peer falls behind irreversibly.
- A legitimately offline device (a laptop in a drawer for a month) "goes
  stale" without panic: `stale` status; rehabilitation = a normal sync plus
  user approve on one active device ("yes, that really is my laptop back").

### Applying changes

- Verified foreign ops land in `pending_ops`; nothing applies silently.
- A review screen in the TUI: diff with dangerous fields (`address`,
  `proxy_jump`, `remote_command`, `identity_id`) highlighted in red;
  approve/reject.

## Credentials: the Shamir hybrid

- A credential is encrypted with a random DEK (XChaCha20-Poly1305); the
  ciphertext replicates and backs up freely: without the DEK it is garbage.
- The DEK is split with Shamir k-of-n, **k=2**: one share per device plus
  the phone.
- **Replication invariant:** every recipient's wrapped share (including the
  phone's) travels inside the op and replicates to **all** peers, so revoking
  a share-holding device loses nobody else's shares.
- Requesting a credential: the peer assembles k shares; every release is a
  tier-2 op.
- Decryption happens in memory only; plaintext is never written to disk.
- Device revocation: tier-1 op, then DEK/share reissue for affected
  credentials (requires a k=2 quorum among the remaining holders; see the
  invariant above).
- **Recovery share (a genesis option):** a printable offline share encoded
  as a **BIP39 seed phrase (24 words)**, exactly like crypto wallets;
  insurance for the "one machine + phone" topology against total credential
  loss if the phone dies.

## The phone as key-material holder (PWA)

The phone is not a confirmation gadget but a peer device: its own key pair,
its own Shamir share in the quorum. Without bytes from the phone (or a valid
certificate), credentials are not released.

### Challenge (terminal to phone)

- A QR code in the TUI. The challenge = **the entire canonical serialized
  op** + nonce + expiry + the requester's ephemeral X25519 pubkey.
- The phone **itself** deterministically renders the human-readable
  description from `op_body`; there is no separate description field, so the
  meaning cannot be spoofed.
- **Channel binding:** the phone signs
  `hash(op_body || nonce || expiry || eph_pubkey || request_id)`. Everything
  that affects the response route is under the signature: replay to another
  holder is impossible (nonce/expiry), and substituting a foreign ephemeral
  key (so the share gets crypto_boxed to the attacker) is impossible because
  the recipient key is signed.
- The phone's wrapped share arrives inside the challenge (the phone stores
  no credential keys). The QR size budget is computed at planning time; if
  the full op does not fit one QR on an 80x24 terminal, use animated/fountain
  QR.

### Share release (phone to terminal): one format, three transports

The response is always the same: the share encrypted with `crypto_box` to
the ephemeral key from the challenge (~56 bytes: share 32 + tag 16 +
request-id 8). Eavesdropping on any transport yields ciphertext for a
one-time key: useless. On "Approve" the phone fires all layers at once; the
TUI modal listens to everything simultaneously and the first successful path
wins:

1. **Audio (primary):** a 2-3 second FSK/DSSS burst (ggwave-style, ~56 bytes
   plus Reed-Solomon FEC); the terminal listens on the microphone (`cpal`).
   Tap, chirp, field filled. In the PWA: Web Audio, offline.
2. **Clipboard (opportunistic):** the same ciphertext base64-encoded into
   the clipboard; with KDE Connect / Handoff it arrives on its own. The TUI
   picks it up (`arboard`) and validates it **by AEAD decryption success**
   (the request-id is only a fast filter; it is not secret and carries no
   trust).
3. **One-time code from the Argon2 cache (fallback for repeat access):**
   a share cannot be pre-encrypted, since the phone only learns it at
   challenge time. So the cache refills **per credential after first use**:
   on every successful share release for credential X, the phone additionally
   ships the peer 1-2 fresh blobs `AEAD(share_X, key=Argon2id(code_i,
   salt_i))`, where `code_i` is 60 bits of true randomness that exist only on
   the phone. Repeat access to X on a headless machine = 12 Crockford base32
   characters (`K7Q2-9FMX-C41T`) or 5 EFF words. Argon2id is memory-hard
   (1 GiB x ~1 s; on weak phones 256 MiB x t=4 with recomputed strength):
   offline brute force is bounded by GPU memory (~80 parallel attempts per
   card, on the order of 10^7-10^8 years). Codes are single-use; a DEK
   reissue invalidates the credential's cache (it refills on next contact).
4. **Full manual entry (guaranteed fallback, always available):** first
   access to a credential on a headless machine without audio/clipboard
   means typing the share by hand as a **seed phrase: 24 BIP39 words** with
   autocompletion in the TUI (the first 4 letters of a word are unique, so
   ~100 keystrokes) and the built-in checksum (a typo is caught immediately,
   pointing at the bad word). Painful but rare; afterwards the Argon2 cache
   (path 3) takes over.

**Encoding principle:** all long key material that passes through human
hands/eyes is encoded as BIP39 words (full share entry, the printable
recovery share: literally "a seed phrase for all your credentials", with
handling rules users already know from wallets). The email code is base32:
it is copy-pasted, hands are not involved.

Rejected: HOTP/pre-agreed chains (anything derivable from shared state a
compromised laptop computes itself), WebRTC (SDP needs the same return
channel: chicken and egg), reverse QR (rejected by the user; kept as a
hidden option).

### The phone's chain: terminals as couriers

The phone does not participate in `sshub sync-serve` (a PWA has no SSH), but
it maintains its own op chain (certificates, approvals). Transport: **every
QR response carries the phone's signed chain head plus its new ops**; the
terminal the phone contacted delivers them to the other peers during normal
sync (courier). The freshness of the phone's head = the signed timestamp of
last contact; a share holder rejects a delegation certificate if the phone's
head is older than the threshold (see open questions). Residual risk: if the
phone only ever contacts one (compromised) terminal, that terminal can
withhold the phone's ops until the phone contacts another device; accepted,
bounded by the freshness threshold.

### Platform (iOS + Android)

- A single HTML file, `sshub-companion.html`: IndexedDB for keys
  (PIN/WebAuthn), camera for QR, Web Audio for share release.
- Crypto is vendored `@noble` (pure JS: ed25519 + x25519 + argon2-wasm),
  **not** WebCrypto curves (patchy support).
- The camera requires a secure context: `file://` does not qualify, so HTTPS
  hosting (GitHub Pages from the repo) plus Subresource Integrity / signed
  page version; the hosting supply-chain risk is accepted, mitigations
  (version pinning, self-host instructions) are for the planning stage.
- iOS can evict IndexedDB (`storage.persist()` is effectively ignored), so
  **losing the phone key is a normal scenario**; see recovery below.
- A native mini-app (Tauri Mobile) is the evolution path if the PWA hits its
  limits; the share release protocol does not change.

### Phone loss / re-enrollment (recovery without deadlock)

Revoking the phone is tier-1, but the phone is gone. The recovery path must
be unavailable to a single compromised peer:

- The `phone_reenroll` op requires: signatures of **a majority of the
  network's live devices (at least one)** + the email code + a **timelock of
  N days** (delay-and-announce): the op is visible to all devices for the
  whole period. "All signatures" is not an option: losing the phone and one
  laptop simultaneously (a stolen bag) must not be a permanent deadlock.
- **Canceling recovery:** from a device, requires a fresh email code (a
  compromised peer without the mailbox cannot DoS recovery with endless
  cancellations). **A live phone cancels without the email code**: the very
  fact of its signature proves the "lost" phone is alive, i.e. the recovery
  is an attack; demanding a code there is redundant and harmful (the mailbox
  may already be captured).
- After the timelock expires: the new phone enrolls via QR+SAS, the old key
  is revoked, Argon2 caches and shares are reissued.
- During the phone-less period the k=2 quorum is provided by the machines
  (if there are >=2) or by the recovery share.

## Trust-session: the delegation certificate

So that the ceremony is not required for every little thing:

- `sshub trust-session` on a device triggers the full tier-1 ceremony
  (deliberately maximum-friction: that is a feature).
- The phone issues a signed **delegation certificate**: "device X may perform
  tier-2 without me until date D". It is written to the op-log.
- Inside the window: tier-2 with a local one-key approve, no QR.
- The constraints are enforced by the **share-holding peers** (a compromised
  computer cannot bypass them):
  - scope: tier-2 only; tier-1 is never delegated;
  - **per-holder rate limit**, default **5 releases/day** (configurable);
    holders count from the op-log; caveat: with k=2 only one foreign share
    is needed, so the protection equals a global limit only at n=3; accepted
    deliberately;
  - binding to an epoch range; before releasing, a holder must verify the
    freshness of the phone's chain head (this gives the ratchet teeth);
  - `revoke` from the phone or any device **arrives on the next sync** (not
    "instantly"; see residual risks).
- Requires **n>=3** (two machines + phone): with a single machine there is
  nobody to verify the certificate, so trust-session is unavailable, with an
  honest message to the user.
- Multiple trusted devices are allowed; the default is one.
- The TTL is user-chosen (radiobox): a week / **a month (recommended,
  default)** / 3 months as a **hard cap**. (It used to go up to a year; cut
  during review: TTL x rate-limit must not cover every credential in the
  network.)

## The email code as key material (tier-1)

- Requirement: **the code generator != the requesting device**. The key
  fragment is encrypted and sent by a counterparty holder; if SMTP is
  configured only on the requesting device, the operation is unavailable
  (honest error to the user).
- The code is **long**: >=20 base32 characters (~100 bits). Not TOTP; the
  user copy-pastes it from the email. Offline brute force is ruled out by
  entropy.
- The code is the KDF input for decrypting the fragment: without the bytes
  from the email the operation mathematically cannot proceed. The factor is
  mailbox ownership.
- SMTP/sendmail config is a section of config.toml.

## Enrollment ("Mark as Trusted")

1. Keybind `T` on a host (or from the detail panel): "make this machine a
   trusted device of the sync network".
2. `ssh peer sshub sync-enroll`: identity key exchange.
3. **SAS verification with words** using a **commitment scheme** (both sides
   first commit to their ephemeral keys by hash, then reveal; otherwise a
   MITM could online-grind keys until the SAS matches): 4 words (~44 bits
   with a 2048-word dictionary), e.g. `harbor-globe-lantern-mint`. Words,
   not emoji: terminals do not always render emoji, and words can be read
   out over the phone.
4. With >=2 devices in the network: plus an approval signature from an
   existing device (tier-1).
5. The new device receives its shares (reissue) and a `device_added` op goes
   into the chain.

Minimal sync topology: **1 machine + phone** (k=2 works, but without
trust-session and with a mandatory recovery share). Recommended: >=2
machines + phone.

## Everyday UX flow

Scenario: 5 hosts, 2 of which are network devices (desktop + laptop), a
phone, desktop in a trust-session. The user changes host A's credential on
the desktop:

1. **Locally, no ceremony** (your own device creates a new secret): new DEK,
   credential encrypted, shares wrapped to each recipient's X25519, one
   atomic signed op `cred_updated(host_A)` (ciphertext + all wrapped shares)
   appended to the desktop's chain. The user just saved a form.
2. **Delivery** on the next sync (`S` manually, or automatically when sshub
   starts on the other machine); sync is a background thread, the TUI stays
   live.
3. **On the laptop**: the op lands in `pending_ops`, review shows "desktop
   updated the credential of host A" (identity is a dangerous field,
   highlighted). One-key approve.
4. **Connecting from the laptop to A** (k=2: own share + one more):
   - desktop online + trust-session: the share is released automatically
     (the holder verifies the certificate, rate limit, chain freshness);
   - desktop offline: a modal appears: QR challenge, phone "Approve", audio
     chirp / clipboard / 12-character code, connect;
   - no phone either: "unavailable offline" plus a hint about trust-session.
5. Nobody ferries a share to the phone: its wrapped share rides inside the
   QR challenge.

Caveat: this covers credentials stored in sshub; changing the password on
the host itself outside sshub is not detected.

## UI

- **Tab "Sync"**: the network control panel. A list of **network devices**
  (not regular hosts): status (`●` operational / `◐` co-signer / `○`
  offline-stale past the lag threshold / `✖` COMPROMISED), certificate TTL,
  last seen. Details: chain head, epoch, shares, revoke. Actions: `S` sync
  now, `R` review pending, `T` trust-session.
- **A compact "Sync Net" panel** in the right stack of the main dashboard:
  device glyphs, TTL, pending counter; the whole panel turns red on
  compromised; no actions, the tab key opens the full tab.
- **Status bar everywhere**: `⇅2 ⏳2ops`, turns red on compromised.
- **A fork is a full-screen alert**, not just a line in a tab.
- **The unlock-credential modal**: "have 1 of 2 shares", per-path statuses,
  QR + simultaneous audio/clipboard listening + code field, retry.

## Components (subsystems)

1. **Sync core**: `sync-serve`/`sync-enroll` CLI modes, the background sync
   thread + mpsc, the protocol, merge + tie-break, identity (ed25519 +
   x25519), hash chain + gossip cross-check + fork detection, the ratchet
   with nonces.
2. **Credential crypto layer**: DEK + Shamir, atomic cred ops, share
   release, Argon2 code pools, reissue on revocation, the email fragment,
   the recovery share, delegation certificates + rate limiting.
3. **Phone PWA**: a single HTML page: keys, QR scanning, rendering the
   description from op_body, share release (audio + clipboard + code),
   enroll/re-enroll.
4. **TUI**: the Sync tab, the dashboard panel, pending-ops review, the
   unlock modal (QR + microphone + clipboard + code), the SAS screen, the
   fork alert, keybind `T`, the TTL radiobox.

## Open questions (for the plan)

- Crates: ed25519/x25519 (dalek?), Shamir (vsss-rs / sharks?), Argon2,
  XChaCha20-Poly1305, cpal, arboard; auditability matters more than
  convenience.
- Audio protocol: ggwave-wasm/bindings vs a custom FSK; FEC parameters.
- Argon2 parameters on weak phones (1 GiB x 1 s vs 256 MiB x t=4) with
  recomputed strength.
- QR budget: does a canonical op fit one QR on an 80x24 terminal;
  animated/fountain fallback.
- PWA hosting: GitHub Pages + SRI/version pinning vs self-host instructions.
- The timelock N days for `phone_reenroll` (default?).
- A protocol-level definition of "live devices" for the recovery majority
  (e.g. a self-signed head no older than T, fixed inside the op itself).
- Channel binding: domain separation tag + proto_version under the hash;
  bind the whole canonical op (not just op_body); where the phone's
  signature physically travels (in the release payload vs the phone's op
  chain via courier: enforcement vs audit).
- Argon2 cache strength given M blobs accumulating between DEK reissues; a
  recommendation for periodic reissue.
- Phone head freshness threshold vs trust-session: threshold >= contact
  cadence, plus a UX hint ("sync your phone, the certificate has N days
  left").
- Clipboard: the final integrity check is decrypting the credential itself;
  clipboard injection is at most a DoS (fix this in the implementation).
- The freshness threshold of the phone's chain head for certificate
  validity.
- The lamport jump-cap threshold.
- Numeric thresholds for `stale`/`offline`/`suspect`; certificate behavior
  across skipped epochs.
- ed25519-to-x25519 conversion: one scheme across Rust and `@noble`.
- The SAS dictionary (Russian/English/both).
- Metadata conflicts: always auto tie-break, or manual merge on review when
  both sides edited the same host.
