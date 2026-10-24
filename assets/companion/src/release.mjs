// Share release, phone side: evaluate a challenge (everything the screen shows and every refusal
// is decided HERE from the received bytes), then approve it. No DOM, no I/O: tests drive it
// directly, the UI only renders the result.
import { xchacha20poly1305 } from '../vendor/noble-ciphers-2.4.0/chacha.js';
import {
  bytesOf, concat, edSign, edVerify, hexOf, hybridUnwrap, sasWords, sealedBoxSeal, bip39Encode,
} from './lib.mjs';
import { decodeChallenge, decodePhoneMaterial, encodeShareResponse } from './quorum.mjs';
import { describeOp, parseOp } from './ops.mjs';
import { buildReplyPlain } from './frames.mjs';
import { appendHeadAttest, courierOps } from './state.mjs';
import { randomBytes } from './keys.mjs';

export const CHALLENGE_EXPIRY_SECS = 120; // limits.rs
export const TIMESTAMP_SKEW_GRACE_SECS = 600;

const u64be = (n) => {
  const b = new Uint8Array(8);
  new DataView(b.buffer).setBigUint64(0, BigInt(n));
  return b;
};

const refuse = (checks, id, text) => ({ ok: false, refusal: text, checks: [...checks, { id, ok: false, text }] });

// Opens the phone's own share. Hybrid (kind 0): host_uid(16) || index || share(32) with a
// MANDATORY host_uid comparison. Envelope (kind 1): storage key from the hybrid blob, then an
// AEAD envelope whose associated data names host_uid, epoch and the holder's own index; the
// index is not known to the phone beforehand, so every index is tried and the plaintext must
// name the index that opened it.
export function unwrapPhoneShare(identity, material, hostBytes) {
  if (material.kind === 'hybrid') {
    const plain = hybridUnwrap(identity.xSecret, identity.mlkemDk, material.share_blob);
    if (plain.length !== 16 + 33) throw new Error('wrapped share has the wrong length');
    if (hexOf(plain.slice(0, 16)) !== hexOf(hostBytes)) throw new Error('the share inside the material belongs to a different credential');
    if (plain[16] === 0) throw new Error('share index is zero');
    return plain.slice(16);
  }
  const storageKey = hybridUnwrap(identity.xSecret, identity.mlkemDk, material.storage_blob);
  if (storageKey.length !== 32) throw new Error('storage key has the wrong length');
  const b = material.share_blob;
  if (b.length < 24 + 16) throw new Error('envelope share is too short');
  for (let index = 1; index < 256; index++) {
    const aad = concat(hostBytes, u64be(material.epoch), Uint8Array.of(index));
    let plain;
    try {
      plain = xchacha20poly1305(storageKey, b.slice(0, 24), aad).decrypt(b.slice(24));
    } catch {
      continue;
    }
    if (plain.length !== 33 || plain[0] !== index) throw new Error('envelope share names a different index');
    return plain;
  }
  throw new Error('the envelope share does not belong to this credential or epoch');
}

// ctx: { now (seconds), identity, ring: {id: {name, ed25519_pub hex}}, aliases }
// Returns { ok, refusal, checks, ...details }. On ok, `share` (33 bytes) is live in memory.
export function evaluateChallenge(frame, ctx) {
  const checks = [];
  const pass = (id, text) => checks.push({ id, ok: true, text });
  let ch;
  try {
    ch = decodeChallenge(frame);
  } catch (e) {
    return refuse(checks, 'frame', `challenge is malformed: ${e.message}`);
  }
  pass('frame', 'challenge frame is well formed');

  const left = ch.expiry - ctx.now;
  if (left <= 0) return refuse(checks, 'expiry', `challenge expired ${-left} s ago`);
  if (left > CHALLENGE_EXPIRY_SECS + TIMESTAMP_SKEW_GRACE_SECS) return refuse(checks, 'expiry', `challenge claims to live ${left} s, more than the ${CHALLENGE_EXPIRY_SECS} s a terminal issues`);
  pass('expiry', `valid for another ${left} s`);

  let op;
  try {
    op = parseOp(ch.op_canonical);
  } catch (e) {
    return refuse(checks, 'op', `operation is malformed: ${e.message}`);
  }
  if (op.kind !== 'ShareRelease') return refuse(checks, 'kind', `this is a ${op.kind} operation, not a share release; this phone releases shares only`);
  pass('kind', 'operation is a share release');

  const f = op.fields;
  const names = Object.keys(f).join(',');
  if (names !== 'host_uid,request_id,requester' || !/^[0-9a-f]{32}$/.test(f.host_uid) || !/^[0-9a-f]{16}$/.test(f.requester)) {
    return refuse(checks, 'body', 'share release fields are malformed');
  }
  if (BigInt(f.request_id) !== BigInt(ch.request_id)) return refuse(checks, 'request', 'the operation asks for a different request id than the challenge carries');
  if (f.requester !== op.device_id) return refuse(checks, 'requester', `the operation is signed by ${op.device_id} but names ${f.requester} as the requester`);
  pass('request', 'request id and requester agree between the operation and the challenge');

  const known = ctx.ring[op.device_id];
  const warnings = [];
  if (known) {
    if (!edVerify(bytesOf(known.ed25519_pub), ch.op_canonical, ch.op_signature)) return refuse(checks, 'signature', `the signature of ${op.device_id} does not verify over these operation bytes`);
    pass('signature', `signature of ${known.name} (${op.device_id}) verifies`);
  } else {
    warnings.push(`this phone has never been introduced to ${op.device_id}; its signature cannot be checked here`);
    checks.push({ id: 'signature', ok: true, warn: true, text: warnings[0] });
  }

  let material;
  let share;
  const hostBytes = bytesOf(f.host_uid);
  try {
    material = decodePhoneMaterial(ch.material);
    share = unwrapPhoneShare(ctx.identity, material, hostBytes);
  } catch (e) {
    return refuse(checks, 'material', `this phone cannot release for this request: ${e.message}`);
  }
  pass('material', `own share unwrapped, credential id matches (${material.kind}, epoch ${material.epoch})`);

  const words = sasWords(ch.binding_hash);
  const aliases = { ...ctx.aliases, ...Object.fromEntries(Object.entries(ctx.ring).map(([id, d]) => [id, d.name])) };
  return {
    ok: true,
    refusal: null,
    checks,
    warnings,
    challenge: ch,
    op,
    hostHex: f.host_uid,
    requester: f.requester,
    description: describeOp(op, aliases),
    bindingHash: hexOf(ch.binding_hash),
    bindingWords: words,
    share,
    expiresIn: left,
  };
}

// Approve: chain op, approval signature, and the sealed replies. `snapshot` is the already
// issued { host_uid(16), blob } or null. Returns frames for every channel; the audio frame is
// the short reply (share only): the courier does not fit a 2 to 15 second burst.
export function approveRelease(ev, { identity, state, now, snapshot = null }) {
  if (!ev.ok) throw new Error('refusing to approve a challenge that failed its checks');
  const ch = ev.challenge;
  const op = appendHeadAttest(state, identity, now, { lamport: ev.op.lamport, epoch: ev.op.epoch });
  const approvalSig = edSign(identity.edSeed, ch.binding_preimage);
  const ext = { approvalSig, ops: courierOps(state), snapshot };
  const seal = (plain) => encodeShareResponse(ch.request_id, sealedBoxSeal(ch.eph_x25519_pub, plain, randomBytes(32), randomBytes(24)));
  return {
    full: seal(buildReplyPlain(ev.share, ext)),
    short: seal(buildReplyPlain(ev.share, null)),
    chainOp: op,
    manual: { index: ev.share[0], words: bip39Encode(ev.share.slice(1)) },
  };
}
