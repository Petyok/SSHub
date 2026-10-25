// The phone as a RECEIVER of a reshare (C14), mirroring `reissue_receive` of src/sync/reshare.rs:
// the ops of the epoch arrive as a list of signed ops (the terminal pastes or shows them), the
// phone opens the session keys sealed to it, opens its sub-share of every credential from at least
// two contributors, checks each against its commitment, combines them into its NEW share and seals
// that under an epoch storage key. It never reconstructs a secret and holds no share afterwards:
// the sealed result is what the ring replicates, and the next release challenge hands it back.
//
// Sub-share plaintext is old_epoch u64 BE || from index || 32 bytes, sealed with XChaCha20-Poly1305
// under the session key with associated data host_uid || epoch u64 BE || contributor id || phone id.
// Commitment of a sub-share: derive_key("sshub/sync/reshare/commit/v1", from || index || sub).
// Confirm item: aead_seal(storage key, host_uid || epoch u64 BE || own index, index || share).
import { xchacha20poly1305 } from '../vendor/noble-ciphers-2.4.0/chacha.js';
import { bytesOf, concat, deriveKey, edVerify, hexOf, hybridWrap, reshareCombine, utf8 } from './lib.mjs';
import { b64dec, isU64, parseOp } from './ops.mjs';
import { randomBytes } from './keys.mjs';
import { COURIER_TAIL, appendConfirmBatch, appendShareStorageKey, courierOps } from './state.mjs';

export const RESHARE_ITEMS_PER_OP = 200;
const COMMIT_CTX = 'sshub/sync/reshare/commit/v1';
const SHARE_K = 2;

const u64be = (n) => {
  const b = new Uint8Array(8);
  new DataView(b.buffer).setBigUint64(0, BigInt(n));
  return b;
};
const subAad = (host, epoch, author, to) => concat(host, u64be(epoch), utf8(author), utf8(to));
export const envelopeAad = (host, epoch, index) => concat(host, u64be(epoch), Uint8Array.of(index));
const commitOf = (from, index, sub) => deriveKey(COMMIT_CTX, concat(Uint8Array.of(from, index), sub));
const aeadSeal = (key, aad, plain) => {
  const nonce = randomBytes(24);
  return concat(nonce, xchacha20poly1305(key, nonce, aad).encrypt(plain));
};

// ops: [{ canonical, signature }]; ring: { deviceId: { name, ed25519_pub hex } }.
// `existingStorage`: the storage key this phone already published for this epoch (opened from its
// own chain by the caller) or null; then no new ShareStorageKey is needed.
// Returns { ok, error, epoch, holders, myIndex, contributors, items, skipped, storageSealed, storageKey }.
// Remote ops of any shape end on a refusal, never on an exception out of the camera callback.
export function planReshare(identity, ops, epoch, opts = {}) {
  try {
    return planReshareChecked(identity, ops, epoch, opts);
  } catch (e) {
    return { ok: false, error: `these ops are malformed: ${e.message}` };
  }
}

function planReshareChecked(identity, ops, epoch, { ring = {}, existingStorage = null } = {}) {
  const me = identity.deviceId;
  const parsed = ops.map((o) => ({ ...o, op: parseOp(o.canonical) }));
  const sig = (p) => {
    const known = Object.prototype.hasOwnProperty.call(ring, p.op.device_id) ? ring[p.op.device_id] : null;
    return known ? edVerify(bytesOf(known.ed25519_pub), p.canonical, p.signature) : null;
  };
  const batches = parsed.filter((p) => p.op.kind === 'ShareReshareBatch' && BigInt(p.op.fields.epoch) === BigInt(epoch));
  const sessions = parsed.filter((p) => p.op.kind === 'ShareReshareSession' && BigInt(p.op.fields.epoch) === BigInt(epoch) && p.op.fields.to === me);
  const fail = (error) => ({ ok: false, error });
  // Every number this phone later writes into its own chain comes from here: a u64, like the Rust field.
  if (!isU64(epoch)) return fail('the reshare epoch is not a number a terminal can read (u64)');
  if (!batches.length) return fail(`no reshare batches for epoch ${epoch} in these ops`);
  for (const p of [...batches, ...sessions]) if (sig(p) === false) return fail(`the signature of ${p.op.device_id} does not verify over its reshare op`);
  const newHolders = batches[0].op.fields.new_holders;
  // A holder id is the 16 lower-case hex characters of a device id; anything else never reaches a screen.
  if (!batches.every((b) => Array.isArray(b.op.fields.new_holders) && b.op.fields.new_holders.every((id) => typeof id === 'string' && /^[0-9a-f]{16}$/.test(id)))) return fail('a holder id in the reshare is not a device id');
  const holders = [...newHolders].sort();
  if (!batches.every((b) => [...b.op.fields.new_holders].sort().join() === holders.join())) return fail('the contributors disagree on the new holder set');
  const at = holders.indexOf(me);
  if (at < 0) return fail('this phone is not a holder at that epoch');
  const myIndex = at + 1;

  const keys = new Map();
  for (const s of sessions) {
    try {
      const k = identity.unwrap(b64dec(s.op.fields.sealed));
      if (k.length === 32) keys.set(s.op.device_id, k);
    } catch {
      /* sealed to another generation or damaged: that contributor is skipped */
    }
  }
  const authors = [...new Set(batches.map((b) => b.op.device_id))];
  const contributors = authors.map((a) => ({ id: a, session: keys.has(a), verified: sig(batches.find((b) => b.op.device_id === a)) === true, known: Object.prototype.hasOwnProperty.call(ring, a) }));
  // Contributors this phone has not met prove nothing: at least SHARE_K of them must be devices it enrolled with, with a signature that verifies.
  if (contributors.filter((c) => c.verified).length < SHARE_K) return { ...fail(`fewer than ${SHARE_K} contributors have a signature this phone could verify (it only knows the terminals it enrolled with)`), contributors };

  const hosts = [...new Set(batches.flatMap((b) => b.op.fields.items.map((i) => i.host_uid)))].sort();
  let storage = existingStorage;
  const items = [];
  const skipped = [];
  const planned = [];
  for (const host of hosts) {
    const hostBytes = bytesOf(host);
    const groups = new Map(); // old epoch -> contributions of distinct contributors
    for (const b of batches) {
      const key = keys.get(b.op.device_id);
      const item = b.op.fields.items.find((i) => i.host_uid === host);
      if (!key || !item) continue;
      const pos = item.sub_shares.findIndex(([to]) => to === me);
      if (pos < 0 || !item.commitments[pos]) continue;
      let plain;
      try {
        plain = xchacha20poly1305(key, b64dec(item.sub_shares[pos][1]).slice(0, 24), subAad(hostBytes, epoch, b.op.device_id, me)).decrypt(b64dec(item.sub_shares[pos][1]).slice(24));
      } catch {
        continue;
      }
      if (plain.length !== 41) continue;
      const oldEpoch = new DataView(plain.buffer, plain.byteOffset, 8).getBigUint64(0);
      const from = plain[8];
      const sub = plain.slice(9);
      if (hexOf(commitOf(from, myIndex, sub)) !== hexOf(b64dec(item.commitments[pos]))) continue; // a lying contributor
      const g = groups.get(oldEpoch) ?? [];
      // Contributors are counted by the DEVICE that authored the batch, not by the byte inside the sealed text: one
      // author cannot pose as two by writing two `from` values.
      if (!g.some((c) => c.from === from || c.author === b.op.device_id)) g.push({ from, author: b.op.device_id, sub_shares: [[myIndex, sub]] });
      groups.set(oldEpoch, g);
    }
    const usable = [...groups.entries()].filter(([, g]) => g.length >= SHARE_K).sort((a, b) => (a[0] < b[0] ? 1 : -1))[0];
    if (!usable) {
      skipped.push(host);
      continue;
    }
    const share = reshareCombine(myIndex, usable[1].slice(0, SHARE_K));
    planned.push({ host, hostBytes, share });
  }
  if (!planned.length) return { ...fail('no credential has two readable contributions yet'), skipped, contributors };
  let storageSealed = null;
  if (!storage) {
    storage = randomBytes(32);
    storageSealed = hybridWrap({ x25519_pub: identity.xPub, mlkem_pub: identity.mlkemPub }, storage, randomBytes(32), randomBytes(24), randomBytes(32));
  }
  for (const p of planned) {
    items.push({ host_uid: p.host, sealed_share: aeadSeal(storage, envelopeAad(p.hostBytes, epoch, myIndex), concat(Uint8Array.of(myIndex), p.share.bytes)) });
  }
  // Counters are adopted only from ops whose signature verified under a key this phone knows.
  const trusted = parsed.filter((p) => sig(p) === true);
  const seenLamport = trusted.reduce((m, p) => (BigInt(p.op.lamport) > m ? BigInt(p.op.lamport) : m), 0n);
  const seenEpoch = trusted.reduce((m, p) => (BigInt(p.op.epoch) > m ? BigInt(p.op.epoch) : m), 0n);
  storage.fill(0);
  return { ok: true, epoch, holders, myIndex, contributors, items, skipped, storageSealed, seen: { lamport: seenLamport, epoch: seenEpoch } };
}

// The storage key this phone already published for `epoch`, opened from its own chain, or null.
export function ownStorageKey(identity, chain, epoch) {
  for (const e of chain) {
    const hex = e.canonical;
    const bytes = Uint8Array.from(hex.match(/../g) ?? [], (b) => parseInt(b, 16));
    const op = parseOp(bytes);
    if (op.kind === 'ShareStorageKey' && BigInt(op.fields.epoch) === BigInt(epoch)) {
      try {
        const k = identity.unwrap(b64dec(op.fields.sealed));
        if (k.length === 32) return k;
      } catch {
        /* an older generation's key: keep looking */
      }
    }
  }
  return null;
}

// Appends the ops of a plan to the phone chain (ShareStorageKey when new, then confirm batches)
// and returns the list a terminal must take. A delivery carries at most COURIER_TAIL ops.
export function publishReshare(state, identity, plan, now) {
  const chunks = [];
  for (let i = 0; i < plan.items.length; i += RESHARE_ITEMS_PER_OP) chunks.push(plan.items.slice(i, i + RESHARE_ITEMS_PER_OP));
  const count = chunks.length + (plan.storageSealed ? 1 : 0);
  if (count > COURIER_TAIL) throw new Error(`${plan.items.length} credentials need ${count} ops; a delivery carries ${COURIER_TAIL}. Ask the terminal to send them in parts.`);
  if (plan.storageSealed) appendShareStorageKey(state, identity, plan.epoch, plan.storageSealed, now, plan.seen);
  for (const c of chunks) appendConfirmBatch(state, identity, plan.epoch, c, now, plan.seen);
  return courierOps(state, count);
}
