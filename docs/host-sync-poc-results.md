# Host sync: PoC and stage results

This document consolidates every proof-of-concept and stage result of epic #13 (P2P encrypted sync of hosts and
credentials). It replaces the per-branch `poc/results/*` files. Design: `docs/host-sync-design.md`; byte formats:
`docs/host-sync-contract.md`; phone page: `docs/host-sync-companion.md`.

Environment names are generic. "Container" means the Linux x86_64 build and test container (rustc 1.99.1, node
22.23.4, headless Chromium 156, Playwright 1.64.1, Python 3.11 oracle venv). "Laptop" means a 4 core, 8 GB x86_64
developer laptop. "Phone" means one Android phone, a Samsung SM-A536E (Galaxy A53 5G class) with Android 16 and
Chrome 155, connected over USB; no serial numbers or personal data appear in the results. No iPhone was available.

## Summary

| PoC | Question | Verdict |
|---|---|---|
| A | Do Rust and browser crypto agree byte for byte? | PASS, four findings changed stage 0 |
| B | What does revocation (resharing) cost, and does it protect the future? | PASS after redesign, one new risk found and fixed with named residuals |
| C | How much fits in an animated QR on a terminal? | PARTIAL: offline part done; C4 measured on a real phone (four blocks); C5 NOT run |
| D | Are the Argon2 profiles affordable natively and in a browser? | FAIL on the strict 2 s master bound (kept, mitigated); D3 on a phone: median PASS, worst of five FAIL at 256 MiB |
| E | Can a hardware key be a share holder? | NOT RUN, feature removed |
| F | Is the SSH transport sound, fast and bounded when a peer is offline? | PASS |
| G | iPhone behaviour | NOT APPLICABLE, no iPhone |
| H | Do audio and QR carry a share back to the terminal? | PARTIAL: offline part done; reverse QR to the laptop webcam 10 of 10 on a real phone; audio (H3, H4, H5) NOT run |
| Stage 2 | Protocol, ratchet, SAS enrollment | tests pass |
| Stage 3 | Credential quorum, recovery, certificates, modes | tests pass |
| Stage 3.5 | Hub | tests pass, no daemon needed |
| Stage 5 | Phone page end to end | PASS in headless Chromium; page flows 23 of 23 on a real Android phone (no camera, sound or clipboard); trust sessions PASS in the container |

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

Verdict: **PARTIAL.** C1 to C3 measured offline; C4 measured on a real phone (four blocks, below); C5 not run.

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

### Over the air (C4, real phone)

Setup: Android phone as above, rear camera (480 px wide preview in the page), held still by the maintainer; the
laptop screen showed the animated code in one kitty window (truecolor half blocks, black on white, quiet zone 4, 8
fps, indexed looped chunks of the PoC H scheme). Quiet room, normal indoor light. The scanner page ran jsQR on every
camera frame (about 20 Hz); 5 runs per block; "elapsed" is from the first decoded frame of the run to a matching
sha256. The window is the size of the code's terminal: a tiled window is only 701x825 px (66x35 cells), so the v13
blocks ran in a floating 800x840 px window (95x44 cells at 10 pt), and the player refuses to draw a code wider than
its tty.

| Block | Frames | Distance | Runs ok | Worst | Median |
|---|---|---|---|---|---|
| version 5 (80x24), 1714 B (the real challenge size) | 22 | about 30 cm | 5 of 5 | 7.47 s | 4.07 s |
| version 5, 4096 B | 53 | about 30 cm | 5 of 5 | 21.78 s | 18.72 s |
| version 13 (120x40), 1714 B | 6 | about 50 to 60 cm | 5 of 5 | 1.19 s | 0.66 s |
| version 13, 4096 B | 13 | about 50 to 60 cm | 5 of 5 | 4.59 s | 2.80 s |

Against the criterion (C4: under 5 s at 80x24, five runs) the real challenge at 80x24 is NOT under 5 s in every run:
worst 7.5 s, median 4.1 s; the first runs were slower while the aim settled, the last two took one lap (22 frames at 8
fps is 2.75 s). A missed frame costs a whole lap because the PoC scheme has no parity (the product adds 25 percent
parity frames). At 120x40 the challenge is 6 frames and takes about a second.

Findings: a half-block QR wider than the terminal wraps into horizontal stripes; Android Chrome asks for the camera
only through a collapsed permission chip in the address bar, and the first request stayed pending until Chrome was
restarted after Android's camera permission was granted.

Not run (scheduled for the next device session): **C5** recovery after covering the screen mid-scan, the front
camera, handheld movement and other lighting. The reverse QR direction is measured under PoC H.

## PoC D: Argon2 profiles

Question: are the master, snapshot and email profiles affordable natively and in a browser?

Verdict: **FAIL** on D1 (strict wall time bound missed by about 0.2 s, criterion not rewritten, mitigated), D2
PASS, D3 run on a real phone (median PASS, worst of five FAIL at 256 MiB), D4 deferred.

| Claim | Criterion | Measured |
|---|---|---|
| D1 1 GiB, t=1, p=1, whole process (laptop, median of 5) | under 2 s | 2.21 s (2.16 to 2.35): FAIL |
| D1 hash only / allocation | informational | 1.27 s / 0.85 to 0.90 s |
| D1 peak RSS | informational | about 1.05 GB |
| D1 under `ulimit -v` 512 MiB | clean error, no abort | clean error, exit 3 |
| D2 container Chromium 64 / 256 / 512 MiB, t=4 | success | 174 / 728 / 1,528 ms |
| D2 laptop Chromium 64 / 256 / 512 MiB, t=4 | success, digest equal | 701 / 2,840 / 5,772 ms |
| D3 phone, 256 MiB t=4 | under 3 s, five runs | median 2,445 ms, worst 3,139 ms, 4 of 5 under 3 s: FAIL on the strict worst case, PASS on the median |

Digests match across node, container Chromium and laptop Chromium. The laptop browser is about 4 times slower
than the container for the same profile; that is the order a user machine shows.

Changed in the design: `ARGON2_MASTER` stays 1 GiB, t=1; the Vault key is derived once per process in a
background thread with an "unlocking" state, kept in zeroized memory for the session and never written to disk.
The snapshot profile (256 MiB, t=4 floor) rides in the blob and a weakened profile is flagged.

### D3: real phone (Android)

Samsung SM-A536E (Galaxy A53 5G class), Android 16, Chrome 155, over USB; the same `measure.html` and hash-wasm build
as D2, driven in one tab of the phone's Chrome. The output digests equal the container and laptop digests for all
three profiles. Battery 60 percent, 34 C before, between and after the runs.

| Profile | Runs (ms) | Median | Worst |
|---|---|---|---|
| 64 MiB, t=4 | 1034, 933, 665, 468, 516 | 665 | 1034 |
| 256 MiB, t=4 (snapshot) | 2026, 1862, 2445, 3139, 2911 | 2445 | 3139 |
| 512 MiB, t=4 | 6163 (one run) | 6163 | 6163 |

The snapshot profile is 2 to 3 s on this phone, with a rising trend over five back-to-back runs. The strict criterion
(under 3 s in every one of five runs) is missed by one run (3.14 s); the median and four of five runs meet it. The
criterion is not rewritten. In the product the snapshot is issued once per release, the page shows that it is
sealing the short code, and a phone that cannot afford the floor profile falls back to 64 MiB, t=8, recorded in the
blob and flagged. One phone only; no second model, no slow phone.

Not run: allocation profiling of the 1 GiB cost, browser peak RSS, non-headless laptop window, **D4** profile round
trip, and the slow-phone fallback.

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

Verdict: **PARTIAL.** H1 and H2 measured offline; the reverse QR (phone screen to the laptop webcam) measured on a real phone, 10 of 10 (below); H3, H4, H5 (audio) NOT run, scheduled for the next device session.

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

### Reverse QR: phone screen to laptop webcam (measured)

Conditions: Android phone as above, screen brightness at maximum, held by the maintainer at roughly 13 to 25 cm from
the laptop webcam (an estimate), quiet room, normal indoor light. Receiver: a throwaway Chromium profile on the
laptop, 1280x720 fixed-focus webcam, jsQR on every frame (about 7 frames per second), live preview with an aiming
overlay. Payload 160 B, error correction M, 4 fps, 4 module quiet zone, black on white.

Finding: a first attempt failed because the test page had no viewport meta tag. Chrome laid it out at 980 CSS px and
the code was 26 percent of the screen width (about 1.8 cm, 0.28 mm per module). With a viewport tag and a code as
wide as the screen (an integer number of device pixels per module) it works.

| code | modules | phone px per module | share of screen width | frames | runs ok | median | worst |
|---|---|---|---|---|---|---|---|
| version 6, level M, 4 fps | 41 (49 with quiet zone) | 22 | 99.8 percent | 2 | 10 of 10 | 0.67 s | 4.90 s |

The two slow runs lost a frame and waited for the next lap (indexed looped chunks). The first valid chunk was read 644
ms after aiming. Over a 20 s survey while the holder moved nearer and farther, 80 of 102 frames carried a valid
chunk. The code measured 6.6 to 11.8 webcam pixels per module on decoded frames (1.43 mm per module on the phone); the
smallest module that was still read is 6.6 px, and the far limit was not reached. Estimated distance 13 to 24 cm,
assuming a 67.7 mm screen and a webcam field of view of 1.33 times the distance (not measured with a ruler).

Consequence: the reverse QR must be as wide as the screen with whole device pixels per module and the page must
declare the viewport. The companion page was changed accordingly (version 6 frames, full width, 4 fps, contract
table); the changed page has not been read by the webcam yet.

Not run (scheduled for the next device session): **H3** over-the-air audio (10 attempts, under 5 s), **H4** channel
matrix (10 attempts per cell), **H5** channel order decision, and the terminal's own microphone and camera receivers
against real hardware. The noise model of H1 is simulated white noise, not room acoustics, speaker or microphone
response.

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

Verdict: PASS in the container (headless Chromium, fake camera and microphone, Rust-generated fixtures); the page flows
also pass on a real Android phone (23 of 23) without camera, microphone, speaker or clipboard.

| Claim | Measured |
|---|---|
| End to end run (first run, backup, restore, enrollment with SAS, release, refusals, short code, recovery cancel, Red notice, reshare, trust sessions, fingerprint) | 102 checks, 0 failed |
| Node suites | 71 of 71 (58 crypto vectors, 21 quorum vectors, 51 op-log vectors parsed and described, trust sessions, reshare fixture) |
| Rust verifier of what the page sent | 6 tests pass (replies, reshare ops, trust-session chain) |
| Crypto vectors replayed inside headless Chromium | 58 of 58, 0 external requests |
| Build reproducible, committed file equals the container build | identical |
| Page size | 342,744 bytes |
| Scan of a Rust challenge (84 byte frames, 8 fps loop) to the approval screen | 4.5 s including PIN unlock |
| Reverse QR reply (711 bytes with courier and short code blob) | 10 frames of version 6 (106 B each, 25% parity), lap 2.5 s at 4 fps; clipboard identical |
| Audio reply, fast profile, 128 bytes | 8.16 s; decoded from a fake microphone WAV by the PoC H decoder |
| Rust opens what the page sent | share equals the fixture, approval signature over the binding preimage verifies, courier ops verify, snapshot blob opens with the shown code |
| Refusals | expired, expiry too far ahead, wrong host in the material (hybrid and envelope), tampered op, bad signature, request id mismatch, requester mismatch, other op kind, material wrapped to another device |

Earlier harness result: 58 of 58 crypto vectors in node and Chromium, single file reproducible, cold start 4.2 s
including the full replay, 0 external requests.

Trust sessions (P1, contract 4c-2): the phone authors `CertIssue`, `CertRevoke` and `CertExtend` from kind 9 and kind
10 requests. Refused before signing: a term outside 1 s to 90 days, a budget outside 1 to 50, a host list outside 1
to 64, an inverted epoch range, a request that names the phone; an extension is refused for an unknown, revoked or
expired certificate and when the phone extended it less than 24 hours ago. Rust validates the page's chain with
`validate_cert_issue`, `cert_allows` (the third new host is refused for the budget and allowed with the phone's
extension, each extension covers its own day) and the revocation. The reshare receiver is pinned to the committed
Rust fixture: the page's new shares equal the shares the Rust receiver committed. The courier caps are 8 ops, 256
KiB per op, 1 MiB per list.

### Real phone (Android)

The page was served from the laptop through `adb reverse`, opened in one tab of the phone's Chrome and driven over
the DevTools protocol; site data of the test origin was cleared and the tab closed at the end. Camera, microphone,
speaker and clipboard were not used (the driver refuses them inside the page); challenges came from the Rust
fixtures through the paste box. 23 checks, 0 failed, battery 60 percent, 34 C throughout.

| Claim | Measured |
|---|---|
| Crypto vectors replayed on the phone (the 1 GiB MASTER vector and one more MASTER vector skipped) | 56 of 58, 227 ms |
| Key created under a PIN with 24 backup words | 943 ms including Argon2id 64 MiB x 3 |
| Reload asks for the PIN, a wrong PIN is refused, the right one unlocks | 697 ms |
| Approve with a short code (Argon2id 256 MiB x 4 included) | 3,418 ms |
| Reverse QR reply read back from the page canvas | opens to the phone share (711 bytes) |
| Binding words, approval signature, courier op, short-code blob, refusals, Red notice with the phone proof | as in the container |

Not run on the phone: camera scanning from the page, microphone, speaker, the clipboard, enrollment with SAS (needs
the camera), other browsers, iOS.

## Phone runs so far, and what is scheduled for the next device session

One Android phone has been used (Samsung SM-A536E, Galaxy A53 5G class, Android 16, Chrome 155, USB), driven from the
laptop with the scripts in `assets/companion/scripts/` (`phone-device.mjs`, `phone-ota.mjs`). Done on it: D3 Argon2,
23 page checks, C4 handheld QR (four blocks), the reverse QR from the phone screen to the laptop webcam (10 of 10).

Scheduled for the next device session (not run):

| Item | Needs |
|---|---|
| H3 over-the-air audio (8 of 10 under 5 s), H4 channel matrix, H5 channel order | phone speaker, laptop microphone, media volume up |
| sshub's own receivers against real hardware: `mic.rs` (audio) and `cam.rs` (camera), through `examples/media-probe` of the `media` package | laptop microphone and webcam, the phone page playing and showing its reply |
| The real page's reply QR (version 6 frames, full width, 4 fps, since the page was changed) read by the laptop webcam, with the distance range | phone screen toward the webcam |
| The real sshub unlock modal QR (TUI QR widget) decoded by a camera in a real terminal, not only in tests | a build that contains the widget, a terminal window, the phone or a webcam |
| C5 recovery after covering the screen mid-scan; the front camera; handheld movement; other lighting; the v10 reverse QR for comparison | a phone with a camera |
| Android page on the real hardware beyond the paste path: enrollment with SAS by camera, sound and clipboard of the real page; Brave and Samsung Internet | a phone |
| A slow phone for the Argon2 fallback (64 MiB, t=8), and a second phone model | another phone |
| E hardware key | not applicable: no key, feature removed |
| G iPhone | not applicable: no iPhone, iOS and Safari stay unverified |
