# Host Sync: P2P synchronization of hosts and credentials (design)

Date: 2026-09-20. Status: **v4**, after four rounds of adversarial review; the
blockers from the 19.09 and 20.09 audits are fixed here. Tracked as epic
[#13](https://github.com/Petyok/SSHub/issues/13). Branch: the whole feature is
built on a single feature branch (`feature/host-sync`) and released as one piece.

This is the **technical** document: the rules an implementation must satisfy.
The narrative version for readers (same design, prose and diagrams, RU/EN) is
`host-sync-design-v4.html`. Where the two disagree, this file wins — it is what
`.claude/host-sync/` codes against.

Reviews: `security-review-2026-09-19-host-sync-design.md` (against v3),
`security-review-2026-09-20-host-sync-v4.md` (against v4). Section "Deltas from
the 20.09 review" at the end lists what this document changes beyond v4's prose.

## One-line idea

P2P synchronization of hosts between a user's machines over SSH itself (no
servers, no daemons), with signed hash-chained op-logs, danger tiers for
operations, and two credential modes: **Vault** (a master phrase, the default)
and **Quorum** (Shamir k=2 with the phone, a hardware key or paper as the second
holder). In Quorum mode a single peer is unable to decrypt credentials; in Vault
mode a single peer plus an offline search of the master phrase is enough, and we
say so everywhere the promise is made.

## Threat model

A compromised peer is an active attacker, not just a data leak:

- tampering with `address`: MITM of SSH connections;
- tampering with `proxy_jump`: traffic routed through the attacker's machine;
- tampering with `remote_command`: code execution on every connect;
- tampering with `identity_id`: the wrong key offered to the wrong server;
- bulk exfiltration of credentials with a single request.

Design consequences: foreign changes are **never** applied silently; history
rewriting is detected by all peers; and in Quorum mode credential release
requires bytes that a single peer does not have.

### Accepted residual risks (the honest list)

This list is part of the design. It goes into user-facing documentation word for
word; a promise we cannot keep is worse than no promise.

- **Genesis:** if the first machine is compromised while the ring is created, no
  scheme detects it.
- **Metadata:** addresses, `proxy_jump`, notes, groups are readable by every ring
  device — that is what makes sync work. Only credentials are encrypted.
- **Revocation is not retroactive:** a revoked device keeps what it already
  downloaded; revocation protects the future (reissue), not the past.
- **Revocation is not instant:** it propagates on sync. The window is narrowed by
  the freshness check before every release, not closed.
- **The trust-certificate window:** an attacker on a trusted machine reaches
  credentials inside the certificate's scope, bounded by the new-host budget.
  Over a full term they cover the scope; what bounds them is the **scope**, what
  notices them is the visible list of opened hosts.
- **A stolen device in Vault mode:** the machine holds the ciphertext and the
  ring secret, so the master phrase remains to be guessed — strength by cost, not
  by impossibility. In Quorum mode there is nothing to guess.
- **A holder in someone else's datacenter:** a VPS holder puts a share of every
  key on the most exposed machine in the ring. VPS + any one device = every
  credential. Allowed, warned about, not defended.
- **Paper plus any one computer:** two shares of two. The recovery phrase is a
  key to everything, like a wallet seed.
- **The printed kit** (ring secret QR, and in Vault mode the master phrase): it
  reconstitutes Vault mode without any device of yours. Treat it as the
  credential store itself, never photograph it.
- **A phone that only ever sees one machine:** a compromised courier can withhold
  its ops until the phone touches any other device; bounded by the freshness
  threshold.
- **A password changed on the server itself**, outside sshub, is not detected.

## Operation tiers

Tiers apply to **incoming requests from other devices** and to the release of key
material. Local edits on your own device (creating a host, changing your own
credential) need no ceremony: the device originates the secret rather than
requesting someone else's; local data is protected by the passphrase/keyring.

Code uses `OpTier::{Green, Yellow, Red}`; the user-facing document uses the
colours only.

| Tier | Operations | Requirements |
|------|------------|--------------|
| Green | reading metadata (host list, host N), no credentials | valid peer signature, served immediately |
| Yellow | releasing a share (= the credential of one host); applying a foreign host modification/deletion; admitting a **mirror** | signature + op-log + user approve; a share release additionally requires phone participation or a valid delegation certificate |
| Red | bulk credential export; adding/revoking a **holder**; enabling a trust session; replacing the phone; **changing the credential mode** | phone participation + a signature from a second **attended** device + a third factor that lives outside your devices (paper phrase by default; a second hardware key or an emailed code as options). The three factors must come from three different participants. For phone loss, the recovery path below. |

Promotion of a device to holder is Red **and** additionally requires two genuine
holders online (the resharing protocol needs k contributors). A mirror's
signature counts as the second vote for admitting a mirror, never for creating a
holder, and an unattended participant never votes at Red level at all.

### Principle: factors are bytes, not checkboxes

There is no trusted server in P2P. A boolean check ("code matched, let it
through") on a compromised peer is simply skipped. Every factor must be **key
material**: without its bytes, decryption is mathematically impossible, not
"forbidden by policy". The phone does not answer "yes", it hands over a share;
the emailed code is a KDF input, not a password; the 24 paper words are a full
share living outside every device.

The price of the principle: a factor cannot be reset by support. Hence the
mandatory paper phrase with exactly two holders, and a designed recovery path.

## Architecture

### Device identity and roles

- Every device (including the phone): **Ed25519** (signatures) + **X25519**
  (wrapping), plus an **ML-KEM-768** public key for hybrid wrapping. Ed25519 does
  not encrypt, so X25519 is a separate pair (no ed-to-x conversion: one less
  cross-platform footgun with `@noble`).
- Private keys stored locally under a passphrase/keyring; never synced.
- Identity = pubkeys + a human-readable name (`alice-laptop`).
- The `devices` table replicates as part of the synced data.

Two roles and one property, all three replicated in `devices`:

| | Meaning | Consequence |
|---|---|---|
| **Holder** | holds a share of every credential key, counts toward the quorum | adding one re-forms the quorum: Red, plus two holders online |
| **Mirror** | receives hosts, settings and ciphertext, no shares | adding one changes nothing for anyone else: Yellow. Default offer. |
| **Attended** (property) | a human sits at it and can be asked | only attended devices vote at Red level |
| **Unattended** (property) | VPS, NAS, home server: its signature proves a file was read | never votes Red; releases shares only under a trust certificate |

A mirror **can** open a credential by gathering two shares belonging to others
(phone + paper, say). "A mirror holds no shares" is therefore not the same as "a
mirror cannot reach a credential", and the release ceremony must name the
requester (see "Share release").

### Transport and threading model

- No daemons, no ports: `ssh <peer> sshub sync-serve` runs the sync protocol over
  stdin/stdout, modeled on `git-upload-pack`. A peer is a regular host from the
  sshub list, marked as "my device".
- Protocol: length-prefixed JSON, with size and depth caps on every frame.
- **The sync client lives in a background thread** (same pattern as the file
  watcher: thread + `mpsc` into the event loop), so the TUI never freezes. SSH to
  peers is strictly `BatchMode=yes`; if a peer is unreachable without
  interaction, it is offline.
- **Reachability:** syncing means connecting to any reachable participant, not to
  all of them. Ops spread transitively.
- **The meeting point (the Hub):** one participant with a permanent address is
  enough for the ring to converge. Normally a mirror; optionally an unattended
  holder under a trust certificate. It relays signed ops and cannot alter them.
  It has no accounts, no authoritative copy, and no Red-level vote. The ring must
  work without it, only worse.

### Op-log

- Every mutation is a signed op:
  `{device_id, seq, lamport, prev_hash, timestamp, epoch, alg, op_body, signature}`.
- `alg` is the **algorithm tag** (signature suite + wrap suite). It is mandatory
  from the first op of the first release: without it, a post-quantum transition
  needs a new protocol and a manual migration of every ring.
- Each device owns its chain; peers store copies of other chains and their heads
  (`chain_heads`). Sync = exchanging chain tails.
- **Fork detection strictly by proof:** the `compromised` verdict and the
  full-screen alert fire only on cryptographic proof of equivocation, verifiable
  by any peer: **two signed ops with the same `seq` and different hashes**, OR
  **two signed head attestations of one device where neither is an ancestor of
  the other**.
- **Anti-equivocation (n>=3):** on every sync, peers exchange self-signed head
  attestations of third devices. Diverging gossip yields `suspect` plus a direct
  check on next contact, NOT a red screen: rumor without proof must not defame an
  honest device.
- Fork detection is the secondary mechanism. A compromised peer's primary attack
  is appending valid malicious ops; that is closed by pending-ops review plus the
  tiers.
- **Conflicts:** deterministic tie-break `(lamport, device_id)`, with two
  defenses against lamport inflation: (a) a **jump cap** — an op whose lamport
  outruns the network's observed clock beyond a threshold is rejected; (b) a
  **revoke op never loses** an auto tie-break; any conflict against a revoke goes
  to manual review.
- A credential op is **atomic**: ciphertext plus all wrapped shares travel in one
  op, so "ciphertext from DEK1, shares from DEK2" cannot exist.

### Applying changes

- Verified foreign ops land in `pending_ops`; nothing applies silently.
- **Safe fields apply automatically:** name, note, group, colour, list position.
  Waking the user for a rename teaches them to approve without looking.
- **Dangerous fields wait:** `address`, `proxy_jump`, `remote_command`,
  `identity_id` — the four the threat model is made of. Review screen shows a
  diff with the changed field in red; approve/reject with one key.

### Epoch ratchet (with counterparty entropy)

- `epoch_key(n+1) = HMAC(epoch_key(n), nonce)` where `nonce` is random and
  generated by the counterparty during sync. A date-derived ratchet is worthless:
  a disk clone would compute all future epochs itself. With the nonce, a clone
  that misses even one sync of the original falls behind irreversibly.
- **What a peer verifies (the consumer of `epoch_key`):** each sync round carries
  `epoch_mac = HMAC(epoch_key(n), transcript)` over the round transcript (both
  hellos, both head sets, the nonce). A peer that cannot produce a MAC for an
  epoch at or above its last-known epoch for that device is `behind`. Nothing
  else reads `epoch_key`; it never leaves the device.
- A legitimately offline device goes `stale` without panic. **Rehabilitation:**
  behind by less than one epoch — one user approve on an active device; beyond
  that — a phone signature or a repeat of the SAS ceremony, i.e. a factor a disk
  clone cannot copy after the fact.
- **Residual:** whichever copy syncs first advances the epoch, so an actively used
  clone keeps the original looking stale, and the ceremony falls on the victim.
  Not closed; surfaced in the UI as "this device was rehabilitated N times".

## Credentials

### Common to both modes

- A credential is encrypted with a random per-credential DEK
  (XChaCha20-Poly1305); the ciphertext replicates and backs up freely.
- **Hybrid wrapping:** every wrapped share/DEK is protected by X25519 **and**
  ML-KEM-768 at once (the KDF takes both shared secrets); either one holding is
  enough. This is the only place with a harvest-now-decrypt-later exposure, so it
  ships in the first release rather than later. Signatures stay Ed25519: a PQ
  signature costs kilobytes per op and the threat does not accumulate — the
  algorithm tag is what buys the transition.
- Decryption happens in memory only; plaintext is never written to disk.
- A credential has no "home" device: the op-log records who created it, but that
  grants no rights.
- **Cost consequence:** a hybrid wrap adds ~1 KiB per recipient, so a credential
  op with four holders will not fit one static QR. The animated/fountain QR is
  therefore **mandatory**, not a fallback, and is measured at stage 0.

### Vault mode (the default)

- No shares. The per-credential DEK is wrapped to a single recipient key derived
  from `Argon2id(master_phrase, salt, 1 GiB, t=1)` mixed with the **ring secret**.
- The **master phrase** is issued by the product: **6 words from the EFF large
  list (7776 words, ~77 bits)**. Not the BIP39 list — 6 BIP39 words is 66 bits,
  and the published promise assumes 77.
- The **ring secret** is 128 random bits that never enter the synced data. It is
  identical on every machine of the ring (a new machine must derive the same
  recipient key), so it is a **ring** secret, not a device secret, and any one
  machine leaks it for all. It travels to a new machine as a QR on the printed
  kit.
- Effect: a copy of the data alone (a VPS snapshot, a backup, the hoster's disk)
  is useless at any phrase strength. A stolen machine reduces to an offline
  search of the phrase.
- The kit is a **separate sheet** from the phrase sheet, and both carry a "do not
  photograph" warning at the moment they are generated.

### Quorum mode

- The DEK is split with Shamir k-of-n, **k=2**: one share per holder (machine,
  phone, hardware key, paper).
- **Replication invariant:** every recipient's wrapped share travels inside the
  op and replicates to **all** peers, so revoking a holder loses nobody else's
  share.
- Requesting a credential: the peer assembles k shares; every release is Yellow.
- **Reissue without reconstruction.** On adding or revoking a holder, shares are
  **reshared**, never reassembled: each of k contributing holders splits its own
  share under a fresh polynomial for the new holder set and sends sub-shares;
  each new holder sums the sub-shares it received with Lagrange coefficients. The
  DEK is not reconstructed on any machine at any moment, which matters most
  exactly when revoking a suspected-compromised device. Verifiable (Feldman)
  commitments accompany each sub-share so a malicious contributor cannot hand out
  garbage undetected.
- **The paper share:** a printable offline share encoded as a **BIP39 24-word
  phrase**. A permanent recipient key is derived from it and every new credential
  gets a share wrapped to that key, so the paper need not be online at creation
  time. Mandatory whenever the ring has exactly two holders.
- **Paper derives two independent keys** under separate domain-separation tags:
  `paper/unwrap` (the share recipient key) and `paper/recover` (the recovery
  authorization key). Typing the phrase to unlock a credential hands the machine
  `paper/unwrap`; it must not also hand it the ability to authorize a recovery.
- **A hardware key as holder** stores the secret that unwraps its shares, not the
  shares themselves. **One user-presence gesture per credential unwrap** — a
  session-wide touch would let a compromised laptop drain every credential the
  moment the key is inserted.

### Mode transitions (Red, both directions)

- Vault → Quorum requires decrypting every DEK under the master phrase and
  re-wrapping as shares. That is the one moment a single machine holds every DEK
  in RAM. It is unavoidable; therefore: the user **picks the machine
  interactively**, the UI states plainly what that machine sees, and the
  transition is a `ModeChange` op at Red level.
- Quorum → Vault is a **downgrade of the ring's promise** and needs the same
  ceremony plus a co-signature from a holder that still exists, so "we lost a
  holder" cannot be asserted by the machine that benefits from it. The product
  never performs it silently on holder loss; it offers it.
- The mode is shown per device on the Sync tab, never as one banner: a ring in
  Quorum with one exempt machine reads "Quorum, except one device".

### The mixed-mode exemption

One machine may keep opening credentials with the master phrase inside a Quorum
ring. The master-phrase slot is then bound to that device: the share replicates
like all the others, but unwraps only for someone holding both the phrase and
that machine's key. No invariant breaks. For that machine, "one stolen device is
not enough" stops being true, so it is labelled an **exemption**, per row, and
the alternatives (hardware key, unattended holder + trust mode) are offered
first — the exemption is usually requested for the laptop most likely to be
stolen.

## Share release

### Challenge (terminal to phone)

- A QR code in the TUI (animated/fountain when the payload exceeds one frame,
  which with hybrid wrapping is the normal case).
- The challenge = **the entire canonical op** + nonce + expiry + the requester's
  ephemeral X25519 pubkey + `request_id`.
- The phone **itself** deterministically renders the human-readable description
  from the op; there is no separate description field, so the meaning cannot be
  spoofed. The description **names the requesting device**.
- **Channel binding:** the phone signs
  `H(domain_tag || proto_version || canonical_op || nonce || expiry || eph_pubkey || request_id)`.
  The whole op is under the signature, not just `op_body`: the approval must bind
  *who asked*, not only *what for*. Replay to another holder is impossible
  (nonce/expiry); substituting a foreign ephemeral key is impossible (the
  recipient key is signed); reusing an approval for a different requester is
  impossible (the op header is signed).
- `ShareRelease { host_id, request_id, requester: DeviceId }` — the requester is
  in the body as well as the header, so a holder that enforces on the body alone
  still sees it.
- The phone's own wrapped share arrives inside the challenge; the phone stores no
  credential keys.

### Response (phone to terminal): one format, five channels

The response is always the same: the share encrypted with `crypto_box` to the
ephemeral key from the challenge (~56 bytes). Eavesdropping any transport yields
ciphertext for a one-time key. On "Approve" the phone fires every armed channel
at once; the TUI listens to all of them and the first success wins.

1. **Audio (primary):** a 2-3 second FSK/DSSS burst (ggwave-style, with FEC); the
   terminal listens on the microphone (`cpal`). The unlock modal opens the
   microphone only for its own lifetime, and says so.
2. **Reverse QR (primary for quiet places):** the phone shows the response as a
   static code, turned toward the laptop camera. No microphone, works in an
   open-plan office. If there is no camera, the channel does not arm.
3. **Clipboard (opportunistic):** the same ciphertext base64-encoded; with a
   shared clipboard it arrives on its own. Validated **by AEAD decryption
   success** (`request_id` is a fast filter, not trust). The design intends key
   material to transit a third-party clipboard sync service in encrypted form;
   that is stated, not implied.
4. **Hardware key:** not a channel — a touch replaces the whole ceremony.
5. **Short code (repeat access):** see snapshots below.
6. **Full manual entry (guaranteed fallback):** the share typed as a BIP39
   24-word phrase with autocompletion and checksum.

A **USB cable** is the better channel and arrives with the native app: a browser
page cannot act as a USB device. The protocol does not change.

### Short-code snapshots

A share cannot be pre-encrypted (the phone only learns it at challenge time), so
after a successful release the phone additionally ships the peer one blob
`AEAD(share_X, key = Argon2id(code, salt))`:

- the code is **16 Crockford base32 characters (80 bits)** of true randomness
  that exists only on the phone, single use;
- **one live snapshot per credential**, enforced by the **phone**: it refuses to
  issue a second while it believes one is live. A peer-side cap is a courtesy —
  the machine holding the blob is the one that benefits from ignoring it;
- **TTL 7 days**, recorded inside the blob;
- the Argon2 profile is recorded inside the blob and surfaced in the TUI; a
  weakened profile from a slow phone is flagged;
- a DEK reissue invalidates the credential's snapshot.

Rejected: HOTP / pre-agreed chains (anything derivable from shared state a
compromised laptop computes itself); WebRTC (needs the same return channel:
chicken and egg).

### The phone's chain: terminals as couriers

The phone maintains its own op chain (approvals, certificates) but has no SSH.
Every response carries its signed head and new ops; the terminal it contacted
delivers them during normal sync. Freshness = the signed timestamp of last
contact; a holder rejects a delegation certificate if the phone's head is older
than the threshold — this is what gives the ratchet teeth. Residual risk: a
single compromised courier can withhold the phone's ops until the phone touches
another device.

## Trust session: the delegation certificate

So the phone is not needed for every connection to a test server.

- Enabling it is a full Red ceremony: you are authorising a month of work without
  a second factor.
- The phone issues a signed certificate: "device X may perform Yellow within
  scope S until date D". It is written to the op-log.
- **The constraints are enforced by the share-holding peers**, never by the
  trusted machine:
  - **Yellow only.** Red is never delegated.
  - **Scope:** a host group or an explicit list. **`everything` is not an
    allowed scope** — the scope is the only control that actually bounds a
    patient attacker, and a wildcard removes it. The picker defaults to the group
    the user last worked in.
  - **Budget: 50 new hosts per rolling day, counted ring-wide** from the op-log
    (not per holder: a per-holder limit gets weaker as the ring grows). Hosts
    already opened under this certificate, and key-based connections, do not
    count.
  - **The opened-host list lives in the op-log**, not in the trusted machine's
    memory: the first release for a host adds it, after which that host is free
    until expiry, including across reboots. It is displayed, and it is the main
    way to notice breadth that is not yours.
  - **Phone freshness** is checked before every release.
  - Revocation arrives on the next sync, not instantly.
- Term: a week / **a month (default)** / 3 months as a hard cap.
- Requires **n>=3**: with one machine and a phone there is nobody to enforce the
  certificate, and the feature is honestly unavailable.
- An **unattended holder** exists only here: outside an active certificate it
  releases nothing, so "VPS holder + trust mode" is workable rather than a
  dispenser for anyone who compromised one device.
- Reaching the daily budget asks the phone for one confirmation to extend the
  day, rather than a ceremony per host. That confirmation is a prompt on a
  possibly-compromised machine; it extends the budget, never the scope.

## Phone loss / re-enrollment (recovery without deadlock)

Replacing the phone is Red, and the phone is what is missing. The recovery path
must not be available to one compromised machine, which means no single artifact
may satisfy two of its requirements.

`PhoneReenroll` requires **all** of:

1. **Authorization:** signatures of **two distinct attended devices other than
   the requester**, OR the **paper phrase** (`paper/recover`). Unattended devices
   do not count. "All devices" is not an option: losing a bag with the phone and
   a laptop must not be a permanent deadlock.
2. **A third factor from outside your devices**, and it must be a **different
   artifact than (1)**: if paper provided the authorization, the third factor is
   a second hardware key or an emailed code. In a one-machine ring this means the
   user needs one factor bought in advance; the product says so at setup rather
   than at the emergency.
3. **A timelock of 7 days**, during which the op sits on every device's screen.
4. **At least one `RecoveryAck`** — a signed "I have seen this recovery op at
   time T" from a device other than the requester and other than the relay that
   delivered it. Without it the clock does not start. Rationale: the Hub is
   normally both the only path between NAT'd devices and the sender of the
   out-of-band notification, so a compromised Hub could otherwise run the seven
   days on nobody's screen.
5. **Proof of liveness:** the op carries the self-signed head attestations that
   justify its "live" set, so other devices reject a set that excludes a device
   they know to be fresh.

**Cancelling:** from a device, requires the third factor (otherwise a compromised
peer DoSes recovery with endless cancellations, risking nothing). **A live phone
cancels with no factor at all**: the fact of its signature proves the "lost"
phone is alive, i.e. the recovery is an attack; demanding paper there is harmful
(it is at home, and the mailbox may already be captured).

After the timelock: the new phone enrolls via QR+SAS, the old key is revoked,
shares are **reshared** (not reassembled), and snapshots are cleared. While there
is no phone, the second share comes from the machines (if there are >=2) or from
paper.

## The third factor

Paper by default. Email is an option, not a requirement.

| Factor | Setup | On the road | For whom |
|---|---|---|---|
| **Paper phrase** (default) | none, it already exists | no — it is in a safe | everyone |
| **A second hardware key** | buy and enroll | yes | people doing Red operations away from home |
| **Emailed code** (option) | send-only SMTP | yes | people whom the first two do not suit |

Rationale for the demotion: a factor that requires writing SMTP credentials into
the config of a console SSH client is a factor nobody configures, which makes Red
operations impossible and pressures us into quietly dropping to two factors. What
we needed was a source outside your devices; paper has that property already.

Rules that stay in force for anyone who enables the emailed code:

- the sender is **neither the requesting machine nor the machine signing as the
  second vote** — three factors, three participants;
- **send-only credentials only** (a dedicated submission account or a local MTA).
  An ordinary mailbox password also grants reading, and then "the machine was
  compromised" and "the mailbox was taken" are one event while we counted two;
- the code is >=20 base32 characters (~100 bits), copy-pasted, and is a KDF input
  for decrypting a fragment — not a boolean;
- SMTP config lives on the Hub when there is one, not on every machine.

Email's real role is **notification**: a recovery announces itself for seven days,
and device screens are somewhere the user may not look for a week. A notification
opens nothing, so it needs no secrecy and no special credentials.

## Enrollment ("Mark as Trusted")

1. Keybind `T` on a host: "make this machine one of my devices". A blank machine
   comes from the other end: `sshub sync-join <reachable participant>`, usually
   the Hub.
2. `ssh peer sshub sync-enroll`: identity key exchange (Ed25519 + X25519 +
   ML-KEM).
3. **SAS verification with 4 words** using a **commitment scheme** (both sides
   commit to their ephemeral keys by hash, then reveal; otherwise a MITM grinds
   keys until the SAS matches): ~44 bits, `harbor-globe-lantern-mint`. Words, not
   emoji: terminals do not always render emoji, and words can be read over the
   phone. The handiest second screen is the phone.
4. The device enters as a **mirror** (Yellow: one signature + user approve).
5. **Promotion to holder is separate and Red**, and needs two holders online for
   the resharing protocol.

Minimal topology: 1 machine + phone (Quorum k=2 works, without trust sessions and
with a mandatory paper share). Recommended: >=2 machines + phone or key, plus a
mirror at a permanent address as the meeting point.

## UI

- **Tab "Sync"**: the ring control panel. Devices (not hosts) with status
  (`operational` / `co-signer` / `stale` / `suspect` / `COMPROMISED`), role,
  **credential mode per row** ("exemption: master phrase" where it applies), trust
  scope + opened-host count + term, last contact. Details: chain head, epoch,
  shares, revoke. Actions: `S` sync now, `R` review, `T` trust session.
- **A compact "Sync Net" panel** in the right stack of the dashboard: device
  glyphs, term, pending counter; the whole panel turns red on compromise.
- **Status bar everywhere**: `⇅2 ⏳2ops`, red on compromise.
- **A fork is a full-screen alert**, naming the device and saying what to do.
- **The review screen**: diff per pending op, dangerous fields in red,
  approve/reject with one key. Safe fields never arrive here.
- **The unlock modal**: "1 of 2 shares", the QR, simultaneous listening on audio
  and clipboard, a field for the short code, per-path status, retry.
- **The release confirmation** (holder side): names **which device is asking** for
  which host. A release prompt that does not name the requester is not a
  decision the user can make.
- **Onboarding**: Vault first run issues the 6 words and prints the kit; both
  sheets carry the "do not photograph" warning and the sentence "this sheet is
  the credential store".

## Components (subsystems)

1. **Sync core**: `sync-serve`/`sync-enroll`/`sync-join` CLI modes, the background
   thread + mpsc, the protocol, merge + tie-break, identity, hash chain + gossip
   cross-check + fork detection, the ratchet with nonces and its MAC.
2. **Credential crypto**: DEK, hybrid wrapping + algorithm tags, Shamir,
   resharing without reconstruction, Vault-mode derivation, atomic cred ops,
   share release, snapshots, the paper/master phrases, delegation certificates.
3. **Phone app**: a single page — keys, QR scanning, description rendering,
   release over every channel, enroll/re-enroll, key export.
4. **TUI**: the Sync tab, the dashboard panel, review, the unlock modal, the
   release confirmation, the SAS screen, the fork alert, onboarding.
5. **The Hub**: one docker-compose — a ring participant at a permanent address:
   inbound connections, relaying, couriering for the phone, notifications, a copy
   of the ciphertext. Role: mirror; optionally an unattended holder. Hard
   boundaries: no accounts, no authoritative copy, no Red-level vote.

Sharing hosts with **other people** is explicitly out of scope for this epic: a
second owner changes revocation rights, visibility boundaries and the meaning of
the quorum. It builds on these primitives later; it does not reshape them now.

## Release order

| Stage | Builds | Sections |
|---|---|---|
| 0 | crypto: identity, hybrid wrapping + alg tags, DEK, Shamir + **resharing**, Vault derivation, Argon2 profiles, BIP39 + EFF wordlists, SAS | Credentials |
| 1 | op-log: schema, ops, hash chain, fork proof, tie-break, pending review | Op-log |
| 2 | sync: protocol over SSH, background thread, SAS enrollment, gossip, ratchet + MAC | Transport |
| 3 | credentials: release, challenge/binding, certificates, recovery, snapshots | Share release, Trust session, Recovery |
| 3.5 | the Hub: compose stack, inbound, relaying, couriering, notifications | Transport |
| 4 | TUI: Sync tab, review, unlock modal, release confirmation, onboarding | UI |
| 5 | the phone app: signed release, fingerprint, key export, QR + channels | Share release |

Stages 4 and 5 may run in parallel after 3. A stage is not merged until it has
proven the thing it exists for; a failed prototype is written up, not merged.

## Open questions (close by measurement, not discussion)

| Question | Closes at | What we need to learn |
|---|---|---|
| Animated QR frames and redundancy | stage 0 | with hybrid wrapping a static code is ruled out; how many frames, how much FEC |
| Argon2 profiles on weak phones | stage 0 | memory, time, battery on the weakest supported models; publish the numbers |
| Library choice | stage 0 | signatures, ML-KEM, Shamir + VSS, Argon2, AEAD, `cpal`, `arboard`; auditability over convenience |
| Resharing protocol details | stage 0 | Feldman commitments over which group; behaviour when a contributor drops mid-protocol |
| Lamport jump cap | stage 1 | strict breaks long-offline devices, lenient does not protect |
| Device status thresholds | stage 1 | when `behind`, when `suspect` |
| Metadata conflicts | stage 1 | always auto tie-break, or offer a manual merge |
| SAS dictionary language | stage 2 | EN, RU or both; must be dictatable over the phone |
| Phone freshness threshold | stage 3 | must exceed normal contact cadence |
| "Live device" definition | stage 3 | formal, inside the op, for the recovery quorum |
| Where the app is served from | stage 5 | own hosting or third-party plus instructions |
| Audio channel reliability | stage 5 | off-the-shelf library or our own; noise, retries, permissions |

## What changed from v3

| Was (v3) | Now (v4) | Section |
|---|---|---|
| reissue reassembled the DEK on one machine | resharing; the DEK is never reconstructed | Quorum mode |
| recovery took "a majority of live devices (at least one)" | two distinct attended devices or paper, plus a different third factor, plus an ack | Recovery |
| the Argon2 code cache accumulated 60-bit blobs forever | one 80-bit snapshot per credential, 7 days, issuer-enforced | Snapshots |
| 5 releases/day per holder, 3-month TTL | ring-wide budget of 50 new hosts/day, a mandatory scope, an opened-host list | Trust session |
| a stale device returned on one keypress | one keypress within an epoch; beyond it, an uncopyable factor | Epoch ratchet |
| the emailed code was a mandatory tier-1 factor | paper by default; email optional and send-only | The third factor |
| the phone app was an ordinary hosted page | signed release, fingerprint, key export, native app as target | Phone |
| one mode, quorum only | Vault (default) and Quorum, with Red transitions both ways | Credentials |
| no post-quantum story | hybrid wrapping now, algorithm tags now, PQ signatures deferred | Credentials |
| the meeting point was a deployment note | the Hub as a subsystem with hard boundaries | Transport |

## Deltas from the 20.09 review

These go beyond v4's prose. They are the reason this document, not the HTML, is
what the implementation codes against. Each is cheap now and structural later.

1. **Paper may satisfy at most one recovery requirement**, and the paper seed
   derives `paper/unwrap` and `paper/recover` under separate domain tags
   (review C2).
2. **Mode transitions are Red ops**, the migration machine is chosen
   interactively, and the downgrade needs a surviving holder's co-signature
   (review C3).
3. **`ShareRelease` carries `requester`**, the binding hash covers the whole
   canonical op, and the release prompt names the requester (review H1).
4. **A recovery op needs a `RecoveryAck` from a third party** before its clock
   starts (review H2).
5. **"Device secret" is renamed the ring secret**, is documented as replicated,
   lives on its own sheet, and is in the residual-risk list (review H3).
6. **`everything` is not a valid trust scope**, and the picker has a default
   (review H4).
7. **One user-presence gesture per credential** on a hardware-key holder
   (review M1).
8. **The master phrase uses the EFF 7776 list**, named explicitly, because the
   product already ships a 2048-word BIP39 list and 6 of those words is 66 bits,
   not 77 (review M2).
9. **The holder-promotion rule lives in the tier table**, not only in prose
   (review M3).
10. **`epoch_key` has a defined consumer** — a transcript MAC — and the
    clone-wins-the-race case is a listed residual (review M4).
11. **The app fingerprint ships inside the sshub binary** and is displayed by the
    TUI for comparison (review M5).
12. **Snapshot limits are enforced by the phone**, the issuer, not by the peer
    holding the blob (review M6).
