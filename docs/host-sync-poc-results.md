# Host sync: PoC and stage results

This document consolidates every proof-of-concept and stage result of epic #13 (P2P encrypted sync of hosts and
credentials). It replaces the per-branch `poc/results/*` files. Design: `docs/host-sync-design.md`; byte formats:
`docs/host-sync-contract.md`; phone page: `docs/host-sync-companion.md`.

Environment names are generic. "Container" means the Linux x86_64 build and test container (rustc 1.99.1, node
22.23.4, headless Chromium 156, Playwright 1.64.1, Python 3.11 oracle venv). "Laptop" means a 4 core, 8 GB x86_64
developer laptop. No phone model is named because no phone was used for any measurement below.

## Summary

| PoC | Question | Verdict |
|---|---|---|
| A | Do Rust and browser crypto agree byte for byte? | PASS, four findings changed stage 0 |
| B | What does revocation (resharing) cost, and does it protect the future? | PASS after redesign, one new risk found and fixed with named residuals |
| C | How much fits in an animated QR on a terminal? | BLOCKED: offline part done, handheld runs NOT run |
| D | Are the Argon2 profiles affordable natively and in a browser? | FAIL on the strict 2 s master bound (kept, mitigated), phone run NOT run |
| E | Can a hardware key be a share holder? | NOT RUN, feature removed |
| F | Is the SSH transport sound, fast and bounded when a peer is offline? | PASS |
| G | iPhone behaviour | NOT APPLICABLE, no iPhone |
| H | Do audio and QR carry a share back to the terminal? | BLOCKED: offline part done, over-the-air runs NOT run |
| Stage 2 | Protocol, ratchet, SAS enrollment | tests pass |
| Stage 3 | Credential quorum, recovery, certificates, modes | tests pass |
| Stage 3.5 | Hub | tests pass, no daemon needed |
| Stage 5 | Phone page end to end | PASS in headless Chromium, NOT run on a phone |

## PoC A: Rust and browser crypto interoperability

Question: can one set of byte formats be produced and verified by both Rust and a single-file browser page?

Verdict: **PASS** on A1 to A5.

| Claim | Criterion | Measured |
|---|---|---|
| A1 Rust to JS | zero mismatches | 84 vectors over 21 primitives, all reproduced in node |
| A1 independent oracle | vectors recomputed by other libraries | 142 checks, 0 failed; flipping one output byte in 8 primitives fails the oracle every time |
| A2 JS to Rust | Rust opens what the page produced | 11 checks pass on blobs from node and from Chromium |
| A3 ML-KEM named | version, licence, provenance, vendored | Rust `ml-kem` 0.2.3, JS `@noble/post-quantum` 0.7.1, two more implementations agree on fixed seeds |
| A4 one file | at most 8 MB, cold start under 3 s, offline | 337,515 bytes (102,769 gzip), cold start median 1,012 ms over 5 runs, 1 document request, 0 external |
| A5 canonical edges | empty, u64::MAX, Cyrillic, emoji, Option | 12 op vectors identical in Rust, JS and Python |

Changed in the design:

1. Canonical bytes keep the serde_json form with one rule: integers above 2^53 are plain digits, so a browser
   consumer verifies over the received bytes or uses an exact-integer parser (contract C2).
2. The hybrid wrap header was not authenticated; the whole header is now AEAD associated data (C12).
3. The `crypto_box` sealed box was replaced by `eph_pub | nonce | XChaCha20-Poly1305(BLAKE3 derived key)` (C11).
4. `ml-kem` 0.2.3 is unaudited and pulls a pre-release crate; it is pinned exactly, the wrap is hybrid, and the
   limitation is in the user-facing risk list.
5. `bincode` is not used; the share response frame is hand-encoded (C13).

Not run: any phone or phone browser (desktop Chromium engine only, no Safari, no Firefox), constant-time analysis,
decoder fuzzing, independent audit of any library, Argon2 timings on a phone (PoC D).

## PoC B: resharing and the cost of revocation

Question: what does it cost to revoke a holder, and does a revoked holder gain anything later?

Verdict: B1 to B4 PASS; B5 time PASS, size FAIL for the first protocol and PASS for the redesigned one; B6 (new
claim) FAIL as designed, PASS with wrap-key rotation and named residuals.

| Claim | Criterion | Result |
|---|---|---|
| B1 any two new shares recover | all pairs over add, remove, replace, all-new | PASS |
| B2 revoked holder gains nothing | old plus new share fails AEAD on all combinations | PASS |
| B3 secret never reconstructed | contributor sub-shares never combine to the secret | PASS (plus a static test) |
| B4 dropped or lying contributor | no credential becomes unopenable | PASS |
| B5 time, 200 credentials x 4 holders | under 10 s | 0.3 s total, 0.12 s busiest holder |
| B5 size, same ring | under 2 MB | 3.51 MB with a hybrid wrap per share, 0.78 MB with epoch envelopes, 0.37 MB batched 200 per op |
| B6 forward secrecy | old share plus theft of one remaining key must not open the credential | fails as designed; passes with rotation and retirement; holders that cannot rotate keep the exposure |

Size by credentials x holders (new op bytes, MB): 50 x 4: 0.88 naive, 0.21 envelopes, 0.109 batched; 200 x 4: 3.51,
0.78, 0.374; 1000 x 4: 17.56, 3.81, 1.80; 1000 x 6: 28.65, 5.74, 2.89. First sync of a fresh device over the
whole log costs 0.05 s for 200 x 4 (parse, hash, verify), so transfer dominates. The same 200 x 4 reshare is 3.51 MB
with base64 byte fields, 4.98 MB with hex and 8.41 MB with number arrays.

Changed in the design:

- `Vec<u8>` fields of ops are base64 strings in the canonical form.
- New ops `WrapKeyRotate`, `ShareReshareSession`, `ShareStorageKey`, `ShareReshareBatch`,
  `ShareReshareConfirmBatch` replace the first reshare ops (C14); constants `WRAP_KEY_RETIRE_AFTER_SECS` (7 days)
  and `RESHARE_ITEMS_PER_OP` (200).
- With two holders revocation is impossible (one remaining holder cannot supply k = 2); at least three holders are
  needed for revocation to be possible.
- A checkpoint and prune chapter was added because a ring grows by megabytes per revocation forever.

Checkpoint state after a revocation (credentials only): 200 x 4: log 1.44 MB before, 4.95 MB after naive wraps,
1.81 MB after batched envelopes, state 0.126 MB; 1000 x 4: 7.18, 24.74, 8.98, 0.611 MB; 1000 x 6: 10.49, 39.14, 13.38,
0.896 MB.

Residuals (in the user-facing list): a key file copied before retirement; holders that cannot rotate (paper, a
phone that has not contacted a terminal since); backups and flash that keep a deleted key; a consistently lying
contributor can make a credential unrecoverable (denial of service, not disclosure); the revoked device keeps its
own old share forever.

Not run: network and storage cost, concurrent reshares, phone-side rotation, slow or battery hardware; the batched
variant is a size computation over real ciphertexts.

## PoC C: QR budget

Question: how much data fits a terminal QR animation, and can a phone read it in time?

Verdict: **BLOCKED.** C1 to C3 measured offline; C4 and C5 need a physical phone and were not run.

| Claim | Measured |
|---|---|
| Largest legible QR version, level M, 80x24 | 5 (light and dark theme, QR forced black on white) |
| Largest legible version, 120x40 | 13 |
| Height binds at 80x24 | v6 needs 25 lines, 24 available |
| Bytes per frame | 84 (v5), 331 capacity (v13) |
| 4 KiB over v5 | 53 frames, 13.25 s per lap at 4 fps, 6.6 s at 8 fps |
| 6 KiB over v5 | 79 frames, 19.75 s at 4 fps, 9.9 s at 8 fps |
| Share-release challenge (1519 B) over v5 | 20 frames, 5.0 s at 4 fps, 2.5 s at 8 fps |
| Same challenge over v13 | 5 frames, 0.625 s lap at 8 fps |
| Headless pipeline proof, 4 KiB, y4m fake camera | PASS in 12.9 s, 53 frames, sha256 match |

Changed in the design: frame size follows the terminal area (`fountain::frame_bytes_for_area`); frames have an 8
byte header and 25% XOR parity; `QR_FPS` is 8 (the real phone measurement tunes it). The sender loops forever, so
the coding is indexed looped blocks with parity, not a rateless code.

Not run (maintainer, physical phone): **C4** handheld worst case (under 5 s at 80x24, five runs) and **C5**
recovery after an interrupted scan. Also not run: a real camera at either geometry and a laptop-browser run of C1.
A desktop decoder on a 2x screenshot shows the code is decodable, not that a handheld camera reads it.

## PoC D: Argon2 profiles

Question: are the master, snapshot and email profiles affordable natively and in a browser?

Verdict: **FAIL** on D1 (strict wall time bound missed by about 0.2 s, criterion not rewritten, mitigated), D2
PASS, D3 NOT RUN, D4 deferred.

| Claim | Criterion | Measured |
|---|---|---|
| D1 1 GiB, t=1, p=1, whole process (laptop, median of 5) | under 2 s | 2.21 s (2.16 to 2.35): FAIL |
| D1 hash only / allocation | informational | 1.27 s / 0.85 to 0.90 s |
| D1 peak RSS | informational | about 1.05 GB |
| D1 under `ulimit -v` 512 MiB | clean error, no abort | clean error, exit 3 |
| D2 container Chromium 64 / 256 / 512 MiB, t=4 | success | 174 / 728 / 1,528 ms |
| D2 laptop Chromium 64 / 256 / 512 MiB, t=4 | success, digest equal | 701 / 2,840 / 5,772 ms |
| D3 phone, 256 MiB t=4 | under 3 s, five runs | NOT RUN |

Digests match across node, container Chromium and laptop Chromium. The laptop browser is about 4 times slower
than the container for the same profile; that is the order a user machine shows.

Changed in the design: `ARGON2_MASTER` stays 1 GiB, t=1; the Vault key is derived once per process in a
background thread with an "unlocking" state, kept in zeroized memory for the session and never written to disk.
The snapshot profile (256 MiB, t=4 floor) rides in the blob and a weakened profile is flagged.

Not run (maintainer, physical phone): **D3**. Also not run: allocation profiling of the 1 GiB cost, browser peak
RSS, non-headless laptop window, **D4** profile round trip.

## PoC E: hardware key as a holder

Question: does a hardware key enforce user presence per operation or cache it per session?

Verdict: **NOT RUN**, feature removed. The maintainer has no hardware security key, and the property cannot be
settled by reasoning or a simulation. An unproven factor does not enter Red operations, so this version ships no
hardware-key holder, no hardware-key third factor and no enum variants for them. Paper is the default third
factor, the emailed code the option. To reopen, run the rig with a physical key and record per operation whether a
second unwrap without a touch succeeds.

## PoC F: SSH transport

Question: is the framing sound, is a sync round cheap, and does an unreachable peer give a bounded, prompt-free
offline verdict?

Verdict: **PASS** (F3 after tightening the timeouts).

| Claim | Measured |
|---|---|
| F1 round time, tail 0 / 100 KB / 1 MB / 5 MB | 227.8 / 219.6 / 233.5 / 247.3 ms; 5 MB over 1 MB is 1.06 (linear would be 5.0) |
| F2 dashboard at 120x40, 30 fps, 10 s, sync on | p50 0.73 ms, p99 1.49 ms, max 2.33 ms, 0 of 299 frames over 33 ms |
| F2 control, sync off | max 1.66 ms, 0 of 299 over 33 ms |
| F3 password-only sshd | offline in 116 ms, no prompt, 0 askpass calls |
| F3 silent listener | 4,015 ms (bound 5 s) |
| F3 blackholed address | 4,012 ms |
| F3 control with plain ssh under the same hostile config | askpass reached 3 times, so the trap is live |
| F4 malformed frames | 9 of 9 rejected in process and 9 of 9 exit 2 in the serve child; depth 32 accepted |

Changed in the design: `ConnectTimeout=4` and a 4,500 ms probe deadline in the production ssh command; the probe
overrides a user config that would ask for passwords. F3 first missed the strict 5 s bound by about 2 ms with
`ConnectTimeout=5`.

Not run: a real tty (F2 uses an in-process test backend), real network latency, F3 against a real user config,
sustained memory profiling beyond the high-water mark, and F1, F2, F4 were not rerun after the timeout change.

## PoC G: iPhone behaviour

NOT APPLICABLE: no iPhone is available. The phone page is verified in desktop Chromium only; iOS and Safari are
unverified (camera on a local page, Web Audio autoplay, IndexedDB persistence).

## PoC H: share channels

Question: can the phone return a share over audio, a reverse QR or the clipboard?

Verdict: **BLOCKED.** H1 and H2 measured offline; H3, H4, H5 need a physical phone, speaker and microphone.

| Claim | Measured |
|---|---|
| H1 framing and corruption | round trip PASS, one corrupted byte rejected |
| H1 clean FSK, 56 byte payload, both profiles | 50 of 50 |
| H1 burst, slow (60 ms symbols) / fast (30 ms) | 7.38 s / 3.84 s |
| H1 simulated noise at 20 / 10 / 5 / 0 dB, slow | 50 / 49 / 41 / 33 of 50 |
| H1 simulated noise at 20 / 10 / 5 / 0 dB, fast | 50 / 50 / 47 / 42 of 50 |
| H2 4 KiB and 6 KiB, in order, reversed with duplicates, 20% gaps then loop | all PASS |
| H2 6 KiB over v10 QR | 30 frames, all decoded and reassembled |
| Headless pipeline proof: audio slow / fast, camera | 8.5 s / 5.0 s, 343 ms (slow exceeds the 5 s target) |

The proof found a bug (the transmit page sent an unframed payload) fixed in the same change. In the shipped phone
page the share reply is 128 bytes (the sealed box adds 72 bytes to the 56 byte estimate), so the fast burst takes
8.16 s: over the 5 s bound before any acoustics. Audio therefore stays opt-in and the reverse QR is the primary
phone to terminal channel.

Not run (maintainer, physical phone and laptop): **H3** over-the-air success (10 attempts, under 5 s), **H4**
channel matrix (10 attempts per cell), **H5** channel order decision. The noise model is simulated white noise,
not room acoustics, speaker or microphone response.

## Stage 2: protocol, ratchet, enrollment (`poc/protocol`)

`sync::protocol` 15 tests and `sync::enroll` 8 tests pass (container, debug profile).

| Case | Result |
|---|---|
| two stores converge, ratchet advances every round | equal state hashes, epochs 0, 1, 2 |
| relay through a third device, 2,300 op tail (more than the cap), reordered and duplicated tails | all converge |
| malformed input, caps, garbage frames, forged signature, unknown author | error or rejection, store unchanged |
| equivocation between two peers, proof forwarded to a third | the third verifies the evidence itself and marks the device compromised |
| clone one round behind, whichever copy syncs first, lost acknowledgement, any modified frame | reported behind or failed, nothing advances past what the peer can verify |
| SAS enrollment, man in the middle through the full protocol | 0 of 300 matching words |
| attacker that grinds 64 alternatives after seeing a reveal | refused 200 of 200 |
| independent SAS draws | 0 full matches in 10,000 pairs, first-word matches 3 (expected about 4.9) |

Residuals: a clone exactly one round behind is accepted when the last acknowledgement was lost; first contact of a
pair is trust on first use, the SAS enrollment is the authenticated start. Not run: real SSH between two machines;
the checkpoint sending side waited for the checkpoint store API.

## Stage 3: credential quorum, recovery, certificates, modes (`poc/quorum2`)

`cargo test --lib sync::`: 206 passed, 0 failed, 2 ignored at the time of the report (230 with the Hub tests); clippy
`-D warnings` clean for these files.

| Claim | Result |
|---|---|
| A copy of one device (key and whole log) opens no Quorum credential by any path | PASS; one peer, phone or paper share completes it |
| A copy of one Vault machine opens nothing without both ring secret and phrase | PASS |
| Resharing never calls `shamir_combine` | PASS (per-thread call counter unchanged) |
| Recovery state machine | 9 states x 14 events, 126 cells, all as expected (135 with the unattended acknowledgement) |
| Certificates enforced by holders: issuer, requester, scope, stale phone, expiry, revocation, ring-wide budget, three-holder rule | PASS |
| Mode change Vault to Quorum re-encrypts under fresh DEKs; back needs a live co-signer | PASS |

Design findings: a device that has written nothing is not in its own device table, so recipient lists add it; the
combine counter must be per thread; one storage key per epoch per holder; the vault decision follows the ring mode
as well as the cached local key.

Not done or not run: the adapter that feeds the recovery state machine from ops waited for op-log additions; the
real 1 GiB master profile (cheap profile in these tests; the stage 0 vectors cover it); the paper recipient takes no
part in resharing; mode change does not erase history; no end to end run with real SSH.

## Stage 3.5: Hub

There is no hub daemon and no `hub-serve` verb: a Hub is an ordinary ring device (a mirror, unattended) behind the
sshd of `deploy/hub/`, reached with `ssh hub sshub sync-serve`. Boundaries and where they are enforced: it never
votes (approvals count only from attended devices), never starts the recovery clock (acknowledgements count only from
attended devices), holds no shares, releases a share only under a valid certificate, announces recovery events out of
band at most once per op and never with a code, and relays phone courier ops without being able to alter them (strict
decode, at most 8 ops). It never signs checkpoints (belongs to the op-log). 230 tests pass. Not run: a real mail
command, an end to end run with sshd and the container.

## Op-log and checkpoints

The op-log package produced no separate results file; its evidence is the committed op vectors (35 canonical ops
replayed by Rust, JS and the page), the stage tests, and the checkpoint size numbers under PoC B. Checkpoint
proposal, verification and pruning are covered by the op-log unit tests and the design chapter "Checkpoints and
pruning".

## Stage 5: phone page (`poc/phone`)

Verdict: PASS in the container (headless Chromium, fake camera and microphone, Rust-generated fixtures). NOT run on
a phone.

| Claim | Measured |
|---|---|
| End to end run (first run, backup, restore, enrollment with SAS, release, refusals, short code, recovery cancel, fingerprint) | 61 checks, 0 failed |
| Node suites | 52 of 52 (58 crypto vectors, 21 quorum vectors, 35 op-log vectors parsed and described) |
| Crypto vectors replayed inside headless Chromium | 58 of 58, 0 external requests |
| Build reproducible, committed file equals the container build | identical |
| Page size | 319,006 bytes |
| Scan of a Rust challenge (84 byte frames, 8 fps loop) to the approval screen | 3.9 s including PIN unlock |
| Reverse QR reply (with courier and short code blob) | 711 bytes, 4 frames; clipboard identical |
| Audio reply, fast profile, 128 bytes | 8.16 s; decoded from a fake microphone WAV by the PoC H decoder |
| Rust opens what the page sent | share equals the fixture, approval signature over the binding preimage verifies, courier ops verify, snapshot blob opens with the shown code |
| Refusals | expired, expiry too far ahead, wrong host in the material (hybrid and envelope), tampered op, bad signature, request id mismatch, requester mismatch, other op kind, material wrapped to another device |

Earlier harness result: 58 of 58 crypto vectors in node and Chromium, single file reproducible, cold start 4.2 s
including the full replay, 0 external requests.

Changed in the design: the phone frames (kinds 3, 5 to 8) and the share reply extension (courier ops, approval
signature and snapshot blob inside the sealed reply, share only on audio) are contract C16; the fingerprint is the
sha256 of the file with its own 64 hex characters zeroed.

Not run: real phone camera, microphone and speaker, Android Chrome, handheld QR scanning, clipboard sync services,
iOS and Safari (unverified), `file://` camera behaviour (the harness serves `http://localhost`), a slow-phone Argon2
fallback.

## What remains for the maintainer (physical devices)

| Item | Needs |
|---|---|
| C4 handheld QR worst case (under 5 s at 80x24, five runs), C5 recovery after interruption | a phone with a camera |
| D3 Argon2 256 MiB, t=4 on a phone (under 3 s, five runs) | a phone |
| H3 over-the-air audio (8 of 10 under 5 s), H4 channel matrix, H5 channel order | phone speaker, laptop microphone |
| Phone page on Android Chrome (first run, enrollment, release) | a phone |
| E hardware key | not applicable: no key, feature removed |
| G iPhone | not applicable: no iPhone |
