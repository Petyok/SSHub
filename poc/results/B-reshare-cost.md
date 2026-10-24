# PoC B: resharing and the cost of revocation

Branch `poc/shamir` (code: `src/sync/crypto/shamir.rs`, `src/sync/crypto/identity.rs`; bench and ring model in
`poc/reshare/`). Verdict: **B1 to B4 PASS; B5 time PASS, B5 size FAIL for the straightforward protocol and PASS
for the redesigned one; B6 (new claim) FAIL as designed in v4, PASS with the fix below, with named residuals.**

Environment (generic): Linux x86_64, 20 logical CPUs, rustc 1.99.1, release build for the bench, single thread
per run. Raw numbers: `poc/results/B-bench.json`. Reproduce: `cd poc/reshare && cargo test --no-default-features`
and `cargo run --release --no-default-features --bin bench-revocation`.

## Claims

| Claim | Criterion | Result | Verdict |
|---|---|---|---|
| B1 any two new shares recover the secret | all pairs, at least three holder-set changes | add one, remove one, replace one and all-new, every contributor pair, every pair of new shares, ciphertext opens each time (unit `b1_*`, ring level `b1_to_b4_at_ring_level`) | PASS |
| B2 a revoked holder gains nothing | old share plus any new share fails AEAD, on every combination | all 3 old x 3 new combinations, and the kept-index set, none opens (`b2_*`, ring level). Note the inverse: old share plus another OLD share still opens, see B6 | PASS |
| B3 the secret is never reconstructed | one contributor's sub-shares recombine to that contributor's share exactly and never to the secret | equality with the contributor's share asserted for every pair of sub-shares, inequality with the secret asserted; plus a static test that the resharing functions never mention `shamir_combine` (`b3_*`, `b3b_*`) | PASS |
| B4 a dropped contributor leaves the ring working | no credential becomes unopenable at any point | one contribution alone is refused, a damaged contribution is caught by its commitment, a consistently lying contributor yields a set that fails AEAD and is dropped while the old set keeps opening (`b4_*`) | PASS |
| B5 price of revocation | 200 credentials, 4 holders: under 10 s and under 2 MB of new ops (soft) | time: 0.3 s total, 0.12 s on the busiest holder, far inside 10 s. Size: 3.51 MB with the straightforward protocol (hybrid wrap per share), 0.78 MB with epoch envelopes, 0.37 MB with envelopes batched 200 per op (table below) | time PASS; size FAIL then PASS |
| B6 forward secrecy of wrapped shares (found here) | a revoked device's old share plus a later theft of ONE remaining holder's wrapping key must not open the credential | v4 as designed: the attack succeeds (`b6a`). With per-epoch wrap key rotation and retirement: fails (`b6b`). Holders that cannot rotate keep the exposure (`b6c`) | FAIL, then PASS with residuals |

## B5: the numbers

Columns: credentials x holders before the revocation; log size before (MB, base64 canonical JSON); reshare time
(ms, total over all roles / busiest single holder) and new op bytes (MB) for V1 = hybrid wrap per share, V2 =
epoch envelopes, V2b = envelopes batched 200 credentials per op; first sync of a fresh device over the whole log
(ms to parse, hash and verify every op, excluding storage and network, V1 / V2).

| credentials x holders | log before | V1 ms | V1 MB | V2 ms | V2 MB | V2b MB | first sync ms V1 / V2 |
|---|---|---|---|---|---|---|---|
| 50 x 3 (revoke 1, 2 remain) | 0.28 | 53 / 27 | 0.60 | 11 / 6 | 0.15 | 0.075 | 10 / 9 |
| 50 x 4 (revoke 1, 3 remain) | 0.36 | 75 / 31 | 0.88 | 11 / 5 | 0.21 | 0.109 | 12 / 11 |
| 50 x 6 (revoke 1, 5 remain) | 0.52 | 120 / 40 | 1.43 | 13 / 6 | 0.31 | 0.175 | 16 / 15 |
| 200 x 3 (revoke 1, 2 remain) | 1.10 | 211 / 106 | 2.40 | 39 / 19 | 0.58 | 0.260 | 39 / 35 |
| 200 x 4 (revoke 1, 3 remain) | 1.44 | 298 / 123 | 3.51 | 41 / 20 | 0.78 | 0.374 | 51 / 46 |
| 200 x 6 (revoke 1, 5 remain) | 2.10 | 481 / 161 | 5.73 | 45 / 21 | 1.17 | 0.603 | 71 / 62 |
| 1000 x 3 (revoke 1, 2 remain) | 5.52 | 1049 / 524 | 12.02 | 189 / 95 | 2.85 | 1.254 | 206 / 186 |
| 1000 x 4 (revoke 1, 3 remain) | 7.18 | 1464 / 605 | 17.56 | 196 / 95 | 3.81 | 1.800 | 253 / 225 |
| 1000 x 6 (revoke 1, 5 remain) | 10.49 | 2333 / 781 | 28.65 | 214 / 100 | 5.74 | 2.893 | 347 / 305 |

Two holders: revoking one is impossible (one remaining holder cannot supply k = 2 contributions); recovery needs
the paper share or the phone. A ring that wants revocation to be possible needs at least three holders.

Byte encoding matters as much as the protocol. The same V1 reshare of 200 credentials with 4 holders is 3.51 MB
with base64 byte fields, 4.98 MB with hex and 8.41 MB with JSON number arrays (the form the A5 vectors use for
32 byte values). Decision: `Vec<u8>` fields of ops are base64 strings (standard alphabet, padded) in the
canonical form; fixed 32 byte values stay as the rules in the contract say for hashes, and anything large
is base64. The encoder in stage 1 freezes this with vectors.

First sync parse, hash and verify cost is small (0.05 s for the 200 x 4 log) next to transfer: at 3.5 MB per
revocation a café connection moves a log of a few revocations in seconds, but a long-lived ring grows by
megabytes per revocation forever, so a checkpoint and prune chapter is still missing from the design (maintainer
decision, see below).

## B6: what resharing does not do, and the fix

Resharing re-splits shares of the SAME secret. The old wrapped shares stay in the append-only log, replicated
to every peer, and a holder's long-lived wrapping key still opens its old blob. So: revoked device R keeps its
old share; an attacker who later steals ONE remaining holder's key opens that holder's old blob from the log,
combines it with R's share and gets the DEK of every credential. `b6a` demonstrates it. The v4 statement
"revocation protects the future" does not hold for software holders unless the wrapping keys change.

Fix, implemented in `identity.rs` and measured here:

1. Before a reshare, every remaining holder rotates its wrapping keys (`rotate_wrap_keys`, new X25519 and ML-KEM
   pair, the signing key does not change) and publishes the new public half (`WrapKeyRotate` op).
2. The reshare wraps everything to the new keys. Old blobs stay readable only while the previous generation is held.
3. After the new sharing is adopted, every holder drops the previous generation (`retire_previous_wrap_keys`,
   zeroized, the key file stores only what is held). Adoption is lazy: a credential is confirmed by its next real
   unlock; retirement happens when every credential of the holder is confirmed or after
   `WRAP_KEY_RETIRE_AFTER_SECS` (7 days), whichever comes first.

Cost: one key generation per holder per revocation (milliseconds) and one small op per holder; the reshare of all
credentials is the same one already measured above.

Residual, stated exactly (goes into the design's residual-risk list):

- A copy of a holder's key file or disk image taken BEFORE retirement still opens the old blobs (asserted in `b6b`).
- Holders that cannot rotate keep the exposure: the paper phrase (its wrapping key is derived from the words), a
  hardware key holder, and a phone that has not contacted a terminal since the reshare. Revoked device plus theft
  of such a holder's key still yields every credential it shares. The phone rotates when it next contacts a
  terminal (it signs a `WrapKeyRotate` into its own chain).
- Retirement is deletion, not a guarantee: backups, SSD wear levelling, swap and snapshots may keep the old key.
- A contributor that lies consistently can make a credential unrecoverable (denial of service, not disclosure);
  if it is not confirmed within the retirement window the old shares are gone and the password must be re-entered.
- The revoked device keeps its own old share forever (revocation is not retroactive); with the rotation fix it is
  useless unless one of the residuals above supplies a second old share.

## Design consequences (contract delta, relayed to the op-log package)

1. New op `WrapKeyRotate { device_id, wrap_epoch: u64, x25519_pub, mlkem_pub }`; `DeviceRow` gains `wrap_epoch`;
   applying it replaces the device's current wrap public keys (history kept).
2. Reshare ops replace `ShareReshare` and `ShareReshareConfirm`: `ShareReshareSession`, `ShareStorageKey`,
   `ShareReshareBatch`, `ShareReshareConfirmBatch` (exact fields in `docs/host-sync-contract.md`).
3. New constants `WRAP_KEY_RETIRE_AFTER_SECS` and `RESHARE_ITEMS_PER_OP` in `src/sync/limits.rs`.
4. `CredUpdate` keeps hybrid wrapped shares (the creator does not know recipients' symmetric keys); the log
   holds about 1.2 KB per recipient per credential edit forever.
5. Canonical byte fields are base64 strings.

## What was NOT run

No network or storage cost (the bench measures CPU and bytes; a real round also pays SQLite and transport); no
multi-device concurrency (two simultaneous reshares); no phone-side rotation (PoC C/H territory); no measurement
on slow or battery-powered hardware; the batched variant is a size computation over real ciphertexts, the batched
op handling itself is not implemented yet (stage 3).

## Addendum: the log after a checkpoint

The bench also measured the live state a checkpoint would carry after the revocation (credentials only:
ciphertext, the remaining holders' envelope shares, their storage key blobs; hosts, groups and identities are not
modelled). Chapter "Checkpoints and pruning" of the design uses these numbers.

| credentials x holders | log before | after, naive wraps | after, batched envelopes | checkpoint state |
|---|---|---|---|---|
| 200 x 4 | 1.44 MB | 4.95 MB | 1.81 MB | 0.126 MB |
| 200 x 6 | 2.10 MB | 7.83 MB | 2.70 MB | 0.186 MB |
| 1000 x 4 | 7.18 MB | 24.74 MB | 8.98 MB | 0.611 MB |
| 1000 x 6 | 10.49 MB | 39.14 MB | 13.38 MB | 0.896 MB |

The state is smaller than the log before the revocation because the reshare converts 1.2 KB hybrid wraps into
73 byte envelopes. Credentials that were never resharded keep their 1.2 KB wraps in the state.
