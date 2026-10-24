// Shared fixed test material for PoC A (Rust ↔ JS interop).
//
// This is the contract between the JS side (test/, emit-js-blobs.html, blobs.json)
// and the Rust side (emit-vectors, verify-js-blobs). Every value is synthetic,
// deterministic, and recorded here so either side can hard-code expectations.
// Byte patterns are simple repeats; the Rust side reproduces these exact bytes.
//
// All values hex-encoded; `pat` = byte repeated N times.

const pat = (byte, len) => new Uint8Array(len).fill(byte);
const hexOf = (bytes) => Buffer.from(bytes).toString('hex');

export const FIX = {
  // --- identities -----------------------------------------------------------
  // requester: device that creates the ShareRelease op (and its own challenge
  // eph key is below). holder: device holding the share, signs the approval.
  requesterEdSeed: pat(0x11, 32), // ed25519 seed -> signer of the op
  requesterXSec: pat(0x22, 32),   // x25519 secret (unused in approval, recorded for completeness)
  holderEdSeed: pat(0x33, 32),    // ed25519 seed -> signs the approval
  // recipient: a Rust-held machine (hybrid_wrap target). Its private material
  // is included so verify-js-blobs can open our blob without further handoff.
  recipEdSeed: pat(0x44, 32),
  recipXSec: pat(0x55, 32),
  mlkemD: pat(0x66, 32),          // ML-KEM keygen seed d
  mlkemZ: pat(0x77, 32),          // ML-KEM keygen seed z

  // --- challenge (stage-3 Challenge fields) ---------------------------------
  challengeEphSec: pat(0x88, 32), // eph_x25519_pub's secret; Rust needs it to OPEN the share response
  challengeNonce: pat(0xaa, 32),  // Challenge.nonce (32)
  challengeExpiry: 1893456000,    // i64 (2030-01-01T00:00:00Z)
  requestId: 42,                  // request_id (u64; same in op body and challenge)

  // --- hybrid_wrap determinism (params of the A2 blob) ----------------------
  wrapEphSec: pat(0x99, 32),      // blob eph x25519 secret -> eph_pub in the blob
  wrapNonce: pat(0xbb, 24),       // XChaCha20-Poly1305 nonce in the blob

  // --- share response determinism (sealed box params) -----------------------
  respEphSec: pat(0xcc, 32),      // sealer-side eph secret (recorded; NOT the challenge key)
  respNonce: pat(0xdd, 24),       // ChaChaBox nonce

  // --- plaintexts -----------------------------------------------------------
  wrapPlain: pat(0xee, 64),       // hybrid_wrap plaintext (a 32B share padded to 64B shape)
  sharePlain: pat(0x77, 32),      // share_response plaintext (the 32B share itself)

  // --- ML-KEM encaps message randomness for the A2 hybrid_wrap blob --------
  // (recorded so blobs.json is reproducible; Rust only opens the blob, it
  // never needs this value)
  wrapMlkemM: pat(0x1f, 32),
  // --- op header constants --------------------------------------------------
  opSeq: 1,
  opLamport: 1,
  opPrevHash: pat(0x00, 32),      // genesis prev_hash for the PoC (Q6)
  opTimestamp: 1893452400,        // i64, expiry - 1h
  opEpoch: 0,
  opHostId: 7,
};


// Derived public material is computed by each side from the seeds/secrets above.
// For convenience the JS side also exports the derived values once computed:
export const hex = hexOf;
