// PoC A - A2 blob production (JS → Rust direction), shared by the browser
// page (emit-js-blobs.html) and the node producer (emit-js-blobs.node.mjs →
// blobs.json). Fixed key material comes from fixtures.mjs; the Rust
// verify-js-blobs bin opens/verifies these with the same values.

import * as lib from './lib.mjs';
import { FIX } from './fixtures.mjs';
const hx = lib.hexOf;

export function buildFixtureKeys() {
  return {
    requesterPub: lib.edPubOfSeed(FIX.requesterEdSeed),
    requesterXPub: lib.x25519Pub(FIX.requesterXSec),
    holderPub: lib.edPubOfSeed(FIX.holderEdSeed),
    recipXPub: lib.x25519Pub(FIX.recipXSec),
    recipEdPub: lib.edPubOfSeed(FIX.recipEdSeed),
    mlkemSeed: (() => {
      const s = new Uint8Array(64);
      s.set(FIX.mlkemD, 0);
      s.set(FIX.mlkemZ, 32);
      return s;
    })(),
    challengeEphPub: lib.x25519Pub(FIX.challengeEphSec),
    wrapEphPub: lib.x25519Pub(FIX.wrapEphSec),
    respEphPub: lib.x25519Pub(FIX.respEphSec),
  };
}

export function buildOp(keys) {
  // ShareRelease op by the requester (canonical form per stage-1).
  const op = {
    device_id: lib.deviceIdOf(keys.requesterPub),
    seq: FIX.opSeq,
    lamport: FIX.opLamport,
    prev_hash: FIX.opPrevHash,
    timestamp: FIX.opTimestamp,
    epoch: FIX.opEpoch,
    body: {
      ShareRelease: {
        host_id: FIX.opHostId,
        request_id: FIX.requestId,
        requester: lib.deviceIdOf(keys.requesterPub),
      },
    },
    signature: new Uint8Array(64),
  };
  op.signature = lib.edSign(FIX.requesterEdSeed, lib.opCanonicalBytes(op));
  return op;
}

// (the op above keeps raw bytes for hashing; the JSON form of an op uses hex for
// prev_hash and signature so Rust can parse it without guessing)

// Everything the A2 direction needs, as one structured object (hex strings).
export async function produceBlobs() {
  const k = buildFixtureKeys();
  const { publicKey: recipMlkemEk, secretKey: recipMlkemDk } = lib.mlkemKeygen(k.mlkemSeed);
  const op = buildOp(k);

  // ---- 1. hybrid_wrap to a Rust-held key ----------------------------------
  const wrapBlob = lib.hybridWrap(
    { x25519_pub: k.recipXPub, mlkem_pub: recipMlkemEk },
    FIX.wrapPlain,
    FIX.wrapEphSec,
    FIX.wrapNonce,
    FIX.wrapMlkemM,
  );

  // ---- 2. share response sealed to the challenge eph key (SWR1 frame) -----
  const frame = lib.shareResponse(k.challengeEphPub, FIX.sharePlain, FIX.respEphSec, FIX.respNonce, FIX.requestId);

  // ---- 3. signed approval over the binding preimage -----------------------
  const preimage = lib.approvalPreimage(op, FIX.challengeNonce, FIX.challengeExpiry, k.challengeEphPub, FIX.requestId);
  const approvalSig = lib.approvalSign(FIX.holderEdSeed, op, FIX.challengeNonce, FIX.challengeExpiry, k.challengeEphPub, FIX.requestId);
  const bindingHash = lib.bindingHash(op, FIX.challengeNonce, FIX.challengeExpiry, k.challengeEphPub, FIX.requestId);

  return {
    meta: {
      generated: new Date().toISOString(),
      proto_version: 1,
      domain_tag_challenge: 'sshub/sync/challenge/v1',
      sig_over: 'preimage',
      fixtures: 'test/fixtures.mjs',
      note: 'all key material is synthetic fixture data (byte patterns), not real keys',
      js: {
        '@noble/post-quantum': '0.7.1',
        '@noble/hashes': '2.4.0',
        '@noble/curves': '2.4.0',
        '@noble/ciphers': '2.4.0',
        '@scure/bip39': '2.4.0',
        'hash-wasm': '4.12.0',
        runtime: typeof process !== 'undefined' ? `node ${process.versions.node}` : 'browser',
      },
    },
    blobs: {
      hybrid_wrap: {
        // what Rust must reconstruct to OPEN the blob (verify-js-blobs):
        params: {
          recipient: {
            ed25519_pub: hx(k.recipEdPub),
            x25519_pub: hx(k.recipXPub),
            mlkem_pub: hx(recipMlkemEk),
          },
          recipient_secrets_for_rust: {
            x25519_secret: hx(FIX.recipXSec),
            mlkem_dk: hx(recipMlkemDk),
            ed25519_seed: hx(FIX.recipEdSeed),
          },
          eph_secret: hx(FIX.wrapEphSec),
          eph_pub: hx(k.wrapEphPub),
          nonce: hx(FIX.wrapNonce),
          mlkem_m: hx(FIX.wrapMlkemM),
          plaintext: hx(FIX.wrapPlain),
        },
        output: { blob_hex: hx(wrapBlob) },
      },
      share_response: {
        params: {
          challenge_eph_pub: hx(k.challengeEphPub),
          challenge_eph_secret_for_rust: hx(FIX.challengeEphSec),
          sealer_eph_pub: hx(k.respEphPub),
          sealer_eph_secret: hx(FIX.respEphSec),
          nonce: hx(FIX.respNonce),
          request_id: FIX.requestId,
          plaintext: hx(FIX.sharePlain),
        },
        output: { frame_hex: hx(frame) },
      },
      signed_approval: {
        params: {
          op: { ...op, prev_hash: hx(op.prev_hash), signature: hx(op.signature) },
          nonce: hx(FIX.challengeNonce),
          expiry: FIX.challengeExpiry,
          eph_x25519_pub: hx(k.challengeEphPub),
          request_id: FIX.requestId,
          holder_pub: hx(k.holderPub),
          holder_seed_for_rust: hx(FIX.holderEdSeed),
          requester_pub: hx(k.requesterPub),
        },
        output: {
          preimage_hex: hx(preimage),
          sig_hex: hx(approvalSig),
          binding_hash_hex: hx(bindingHash),
          op_hash_hex: hx(lib.opHash(op)),
        },
      },
    },
  };
}
