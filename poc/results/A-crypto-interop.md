# PoC A: Rust and browser crypto interoperability

Branch `poc/crypto`, rig in `poc/crypto-interop/`. Verdict: **PASS** (A1, A2, A3, A4, A5), with four findings that
change stage 0 (section "Findings").

Environment (generic): Linux x86_64 build container, rustc 1.99.1, node 22.23.4, headless Chromium 156.0.8078.4
(Playwright 1.64.1), Python 3.11 oracle venv. One command reproduces everything: `sh poc/crypto-interop/run-all.sh`
(steps print `@@@ <name> PASS|FAIL`). Raw browser numbers: `poc/results/A-browser-report.json`.

## Claims

| Claim | Criterion | Measured | Verdict |
|---|---|---|---|
| A1 Rust to JS | zero mismatches over every primitive | 84 vectors, 21 primitives (chain_hash, derive_key, ed25519, x25519, ML-KEM-768 encaps and decaps, XChaCha20-Poly1305, Argon2id at TEST, SNAPSHOT, EMAIL and MASTER, BIP39, EFF-7776, Shamir split and combine, reshare, canonical op, op hash, op verify, binding hash, signed approval, hybrid wrap, sealed box, share response frame). JS reproduces all 84 in node (3.8 s) and in headless Chromium (84/84; 83 plus MASTER in a second run), 0 mismatches. | PASS |
| A1 oracle | vectors independently reproducible | `oracles/oracle_vectors.py` recomputes every vector from its inputs with libraries that are neither the Rust crates nor noble: 142 checks, 0 failed. Mutation test: flipping one output byte in 8 different primitives makes the oracle exit 1 each time. | PASS |
| A2 JS to Rust | Rust opens and verifies what the page produced | `verify-js-blobs`: 11 checks PASS on blobs produced by node and, separately, on blobs produced inside Chromium (hybrid wrap opens, public keys equal, Rust re-wrap with the recorded randomness is byte-identical, flipped bit rejected, SWR1 frame decodes and opens, op signature and hash, binding preimage and hash byte-identical, holder signature verifies, wrong requester id fails). The Python oracle also opens both sets (blobs.json and blobs-browser.json). | PASS |
| A3 ML-KEM named | both sides named with version, licence, provenance, vendored, passing A1 and A2 | Rust: `ml-kem` 0.2.3 (RustCrypto, Apache-2.0 OR MIT, from crates.io, locked in `Cargo.lock`; depends on `kem` 0.3.0-pre.0). JS: `@noble/post-quantum` 0.7.1 (MIT, vendored in `vendor/` with tarball sha256 in `vendor/VENDORED.md`). Two more implementations agree on ek, ct and ss for fixed seeds: kyber-py 1.2.0 and OpenSSL via `cryptography` 50.0.2. Four independent ML-KEM implementations, byte-identical. | PASS |
| A4 one file | page at most 8 MB, cold start under 3 s, works offline | `dist/emit-js-blobs.html` 337,515 bytes (0.32 MiB, 102,769 bytes gzip) with noble, hash-wasm (Argon2 wasm inlined), scure-bip39, vectors and word list. Cold start (fresh browser, navigate, blobs produced, 84 vectors replayed minus MASTER): median 1,012 ms over 5 runs (1,089 ms including browser launch). Document requests 1, external requests attempted 0 (all non-local routes were blocked), `window.isSecureContext` true on `http://localhost`. | PASS |
| A5 canonical edges | empty vec, empty byte field, u64::MAX, Cyrillic, emoji, Option None vs Some | 12 op vectors, each with canonical bytes, hash and signature verification in Rust, JS and Python: empty `Vec` and empty `Vec<WrappedShare>`, empty strings, `seq` and `lamport` = u64::MAX with `timestamp` = i64::MIN, `timestamp` = i64::MAX, negative `host_id`, Cyrillic plus emoji name, control characters (U+0001), quote, backslash, newline, DEL, U+2028, `Option::None` and `Some` for `third_factor` and `cosigner`, unit and newtype enum variants. A tampered op does not verify in any implementation. | PASS |

Information (PoC D owns performance): the 1 GiB Argon2id MASTER profile ran inside a headless Chromium tab (the replay
including it took 1,969 ms in total). Not a phone measurement.

## Findings that change the plan

1. **Canonical bytes: the serde_json form works, with one rule.** Integers above 2^53 appear as plain digits in the
   canonical text (u64::MAX is `18446744073709551615`). Native `JSON.parse` rounds that number, so any consumer in the
   browser must either verify over the received bytes (preferred: hash and verify the exact bytes that arrived, never
   re-serialise) or use an exact-integer parser (`lib.parseJSONExact`, tested). Contract amendment C2 is updated: integers
   are plain JSON number literals, and the phone verifies over received bytes. No second format is needed.
2. **The hybrid wrap header is not authenticated.** The blob starts with `SSHW | sig suite | wrap suite | eph_pub |
   kem_ct_len | kem_ct | nonce` and only the plaintext is under the AEAD. The algorithm tag can be flipped in transit
   without breaking decryption. Harmless with one suite, a downgrade lever with two. Stage 0 binds the whole header as
   AEAD associated data (new vector).
3. **The sealed box is a non-standard construction.** `crypto_box` 0.9 `ChaChaBox` is crypto_secretbox over the original
   djb ChaCha20 with a custom HChaCha20 key schedule. It works (JS assembles it from `hchacha`, `chacha20orig`,
   `poly1305`; the Python oracle needed a hand written HChaCha20 and a library ChaCha20 for the stream), but it is the
   only primitive in the stack that is not RFC 8439 or a plain KDF. Stage 0 replaces it with `eph_pub | nonce |
   XChaCha20-Poly1305(key = BLAKE3 derive_key(ctx, dh | eph_pub | recipient_pub))`, the same shape as the hybrid wrap.
   This drops the `crypto_box` crate and the legacy ChaCha path, and the phone needs no hand-assembled cipher. New
   vector, same oracle approach (PyNaCl XChaCha20-Poly1305 plus blake3).
4. **The Rust ML-KEM crate is unaudited.** `ml-kem` 0.2.3 states in its README: "The implementation contained in this
   crate has never been independently audited". It also pulls the pre-release crate `kem` 0.3.0-pre.0. Mitigations:
   the wrap is hybrid, so confidentiality of long-lived material survives a defect in the ML-KEM half as long as
   X25519 holds; the exact version is pinned (`=0.2.3`); the four-way byte agreement above removes encoding and
   implementation divergence but is not an audit; the limitation goes into the user facing risk list. Stage 0 re-checks
   `ml-kem` 0.3.x and `libcrux-ml-kem` and keeps 0.2.3 unless one is clearly better and compatible with the pinned
   `rand_core` 0.6 generation.

Smaller notes: `bincode` is only needed for the two-field share response frame (`request_id u64 BE | len u64 BE |
bytes`); stage 0 and 3 hand-encode it and drop the dependency and the bincode 1 versus 2 confusion. The rig's
`ARGON2_EMAIL` (64 MiB, t=3) predates the chosen default (19,456 KiB, t=2); stage 0 uses the contract value.

## What was NOT run

No physical phone or phone browser (Chromium 156 desktop engine only, no Safari, no Firefox); no constant-time or
side-channel analysis; no fuzzing of the decoders; no independent audit of any library (the noble audit claims in
`vendor/VENDORED.md` come from the library authors and were not re-verified here); the Python oracle shares
libsodium and OpenSSL C code with other users, which is independent of this repository but not of the wider
ecosystem; Argon2 timings on a phone (PoC D, hardware).
