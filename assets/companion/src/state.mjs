// What the phone keeps besides its key: the devices it enrolled with, names the user gave to
// other devices, its own op chain (courier cargo), and the short-code snapshots it has issued.
// All of it is public or low-value data; the secret lives only in the PIN-wrapped key blob.
import { edVerify, hexOf } from './lib.mjs';
import { blake3 } from '../vendor/noble-hashes-2.4.0/blake3.js';
import { encodeCertExtend, encodeCertIssue, encodeCertRenew, encodePhoneHandover, encodeCertRevoke, encodeHeadAttest, encodePhoneReenrollCancel, encodeRedPhoneSign, encodeShareReshareConfirmBatch, encodeShareStorageKey, encodeWrapKeyRotate, isU64, parseOp, signOp, ZERO32 } from './ops.mjs';
import { crockfordEncode, snapshotSeal } from './quorum.mjs';
import { WRAP_GEN_MAX, randomBytes } from './keys.mjs';

export const SNAPSHOT_TTL_SECS = 7 * 86400;
export const SNAPSHOT_PROFILE = { mem_kib: 256 * 1024, iters: 4 };
// Used only when the floor profile cannot run on this device; the blob records it and the
// terminal flags it (design: "a weakened profile from a slow phone is flagged").
export const SNAPSHOT_PROFILE_WEAK = { mem_kib: 64 * 1024, iters: 8 };
// The courier carries the newest ops of the phone's chain with every response; the ring dedups
// by seq. A longer gap than this needs the phone to touch another terminal.
export const COURIER_TAIL = 8;

export const HEAD_PENDING_MESSAGE = 'This phone was restored from its 24 words, so it does not know how far its own chain went and must not sign yet. Take its chain from a terminal first, or confirm that this key never signed anything.';

export const emptyState = () => ({ v: 1, enrolled: null, ring: {}, aliases: {}, chain: [], epoch: '0', lamport: '0', wrapEpoch: 0, snapshots: {} });

// A restored key has the same device id as before, so its next op would reuse a seq the ring already holds (a fork,
// which the ring treats as proof of compromise). Until the chain head comes back from a terminal (the phone's own signed
// ops) or the user confirms that the key never signed anything, nothing may be authored.
export const restoredState = () => ({ ...emptyState(), headPending: true });

// Reads a list of the phone's OWN ops as a terminal holds them. Every op must be authored by this key, verify under it
// and continue the previous one; the newest is the head. Throws a sentence on anything else.
function readOwnChain(identity, ops) {
  if (!ops.length) throw new Error('The list holds no op of this phone. If this key never signed anything, say so with the other button.');
  const parsed = ops.map((o) => ({ ...o, op: parseOp(o.canonical) })).sort((a, b) => (BigInt(a.op.seq) < BigInt(b.op.seq) ? -1 : 1));
  let prev = null;
  const chain = [];
  let wrapEpoch = 0;
  for (const p of parsed) {
    if (p.op.device_id !== identity.deviceId) throw new Error(`An op in the list is not by this phone (${p.op.device_id}).`);
    if (!edVerify(identity.edPub, p.canonical, p.signature)) throw new Error(`The signature of seq ${p.op.seq} does not verify under this phone's key.`);
    const hash = hexOf(blake3(p.canonical));
    if (prev) {
      if (BigInt(p.op.seq) !== BigInt(prev.seq) + 1n || p.op.prev_hash !== prev.hash) throw new Error('The ops in the list do not form one chain.');
    } else if (BigInt(p.op.seq) === 1n && p.op.prev_hash !== ZERO32) throw new Error('The first op of a chain starts from nothing.');
    // The wrap generation is read from the signed rotations (the page cannot keep it any other way after a restore).
    if (p.op.kind === 'WrapKeyRotate' && isU64(p.op.fields.wrap_epoch)) {
      const w = BigInt(p.op.fields.wrap_epoch);
      wrapEpoch = Math.max(wrapEpoch, Number(w > BigInt(WRAP_GEN_MAX) ? BigInt(WRAP_GEN_MAX) : w));
    }
    prev = { seq: String(p.op.seq), lamport: String(p.op.lamport), timestamp: String(p.op.timestamp), hash, canonical: hexOf(p.canonical), signature: hexOf(p.signature) };
    chain.push(prev);
  }
  const epoch = parsed.reduce((m, p) => maxBig(m, big(p.op.epoch)), 0n);
  return { chain, head: prev, epoch, wrapEpoch };
}

// A single list cannot show that it is the newest (a courier or a terminal behind on sync hands over an old tail, and
// signing on top of it forks the chain). So a head is adopted in two steps: two lists must name the same newest op
// (adoptOwnChain returns agreed: true only then) AND come from two DIFFERENT terminals (the kind 5 sender id: unsigned, it
// tells two lists apart and proves nothing), and the user then confirms what they see (acceptHead). A list without a
// sender id can be the first candidate but never agrees. A terminal-signed head would be stronger; see
// docs/host-sync-companion.md.
export const SAME_TERMINAL_MESSAGE = 'This list comes from the same terminal as the first one. Scan the list from another terminal.';
export const SINGLE_SOURCE_TICK_MESSAGE = 'This head comes from one list. Tick that you know the risk (the terminal may be behind, and the next approval would then fork your chain), or take a second list from another terminal.';
// The card text for a head from one list: the count the terminal reported, the risk, never "trusted".
export function singleSourceNote(headSeq, terminals) {
  const ring = terminals && terminals.count === 1 ? 'Your ring has one terminal' : terminals ? `Your ring has ${terminals.count} terminals and I can reach only this one` : 'One list';
  return `${ring}. This phone will continue after seq ${headSeq}. Risk: if that terminal is behind, or this phone signed something it never received, the next approval forks your chain and every device raises an alarm; you would then re-enroll this phone. The count is that terminal's own statement.`;
}
// One list is enough when its sender reports a ring (terminals.count >= 1, itself included): the user ticks the risk
// (acceptHead). The first list stays a candidate, so a second agreeing list from another terminal upgrades it to the
// two-list default that needs no tick. A dead or away terminal never blocks the restore.
export function adoptOwnChain(state, identity, ops, sender = null, terminals = null) {
  const r = readOwnChain(identity, ops);
  const from = sender ? hexOf(sender) : null;
  const info = { head_seq: r.head.seq, head_hash: r.head.hash, newest_at: r.head.timestamp, ops: r.chain.length, sender: from, single_source: false, terminals };
  const c = state.headCandidate;
  let keep = from;
  let previous = null;
  if (c) {
    if (c.seq === r.head.seq && c.hash === r.head.hash) {
      if (c.sender && from && c.sender === from) throw new Error(SAME_TERMINAL_MESSAGE);
      if (c.sender && from) {
        state.headProposal = { chain: r.chain, epoch: String(r.epoch), wrapEpoch: r.wrapEpoch, singleSource: false };
        delete state.headCandidate;
        return { ...info, agreed: true };
      }
      // a list that names no terminal never agrees, and never costs the first one its id
      keep = from || c.sender || null;
    } else {
      previous = c.seq;
    }
  }
  delete state.headProposal;
  state.headCandidate = { seq: r.head.seq, hash: r.head.hash, sender: keep };
  if (terminals && terminals.count >= 1) {
    state.headProposal = { chain: r.chain, epoch: String(r.epoch), wrapEpoch: r.wrapEpoch, singleSource: true };
    return { ...info, agreed: true, single_source: true, previous_seq: previous };
  }
  return { ...info, agreed: false, previous_seq: previous };
}

// The user looked at the agreed head (seq, newest op time) and accepted it. `shown` is { seq, hash } of the head the screen
// showed: a proposal that has changed since is refused, so what is accepted is what was approved.
export function acceptHead(state, shown = null, acknowledgedSingleSource = false) {
  const p = state.headProposal;
  if (!p) throw new Error('No chain head was agreed by two lists yet.');
  const last = p.chain[p.chain.length - 1];
  if (shown && (last.seq !== shown.seq || last.hash !== shown.hash)) throw new Error('The chain head changed since it was shown: take the chain again and look at it before accepting.');
  if (p.singleSource && !acknowledgedSingleSource) throw new Error(SINGLE_SOURCE_TICK_MESSAGE);
  state.chain = p.chain;
  state.lamport = String(p.chain.reduce((m, e) => maxBig(m, big(e.lamport)), big(state.lamport)));
  state.epoch = String(maxBig(big(state.epoch), big(p.epoch)));
  state.wrapEpoch = Math.max(state.wrapEpoch || 0, p.wrapEpoch);
  state.headPending = false;
  delete state.headProposal;
  delete state.headCandidate;
  return { head_seq: p.chain[p.chain.length - 1].seq, ops: p.chain.length };
}

// "This key never signed anything" is a claim nothing can check: the user types the device id to make it deliberate.
export function confirmNoChain(state, identity, typed) {
  if (String(typed).trim().toLowerCase() !== identity.deviceId) throw new Error(`Type this phone's device id (${identity.deviceId}) to confirm that this key never signed anything.`);
  state.headPending = false;
}

// ------------------------------------------------------------ storage backends

export function memoryBackend() {
  const m = new Map();
  return { get: async (k) => m.get(k), set: async (k, v) => void m.set(k, v), del: async (k) => void m.delete(k) };
}

export function indexedDbBackend(name = 'sshub-companion') {
  const open = () =>
    new Promise((res, rej) => {
      const r = indexedDB.open(name, 1);
      r.onupgradeneeded = () => r.result.createObjectStore('kv');
      r.onsuccess = () => res(r.result);
      r.onerror = () => rej(r.error);
    });
  const run = async (mode, fn) => {
    const db = await open();
    try {
      return await new Promise((res, rej) => {
        const tx = db.transaction('kv', mode);
        const req = fn(tx.objectStore('kv'));
        tx.oncomplete = () => res(req && req.result);
        tx.onerror = () => rej(tx.error);
        tx.onabort = () => rej(tx.error);
      });
    } finally {
      db.close();
    }
  };
  return {
    get: (k) => run('readonly', (s) => s.get(k)),
    set: (k, v) => run('readwrite', (s) => s.put(v, k)),
    del: (k) => run('readwrite', (s) => s.delete(k)),
  };
}

export async function loadState(kv) {
  const s = await kv.get('state');
  // A copy: the memory backend hands out the stored object itself, IndexedDB a clone; callers mutate their state in place.
  return s && s.v === 1 ? JSON.parse(JSON.stringify(s)) : emptyState();
}
// Two copies of the page on one storage (the installed page and a tab) would each append the same seq and fork the chain,
// or overwrite each other's short-code records, enrollments and names. Every stored state carries a version counter (`rev`);
// a save only goes through when the stored counter is the one this copy loaded, and then bumps it. `replace` is for the
// deliberate resets (new key, restore): it always writes, one above whatever is stored, so a stale copy refuses afterwards.
// The check and the write run under a Web Lock where there is one, so two tabs cannot interleave them.
// ponytail: without navigator.locks the read and the write are two storage calls; a very tight race is still possible.
export async function saveState(kv, state, { replace = false } = {}) {
  const write = async () => {
    const stored = await kv.get('state');
    const have = stored && stored.v === 1 ? Number(stored.rev || 0) : 0;
    if (!replace && have !== Number(state.rev || 0)) {
      throw new Error('Another copy of this page changed this phone since this one started. Close the other copy and reload this page.');
    }
    const next = have + 1;
    await kv.set('state', JSON.parse(JSON.stringify({ ...state, rev: next })));
    state.rev = next;
  };
  const locks = globalThis.navigator && globalThis.navigator.locks;
  return locks ? locks.request('sshub-companion-state', write) : write();
}

// ------------------------------------------------------------ the phone's own chain

const big = (s) => BigInt(s);
const maxBig = (a, b) => (a > b ? a : b);

// Header of the next op: continues the chain, lamport above anything seen (own and the op
// being answered), epoch = highest epoch seen.
const U64_MAX = (1n << 64n) - 1n;
// `seen` is { lamport, epoch } of an op whose signature the caller VERIFIED (never of an unverified one: a forged
// huge lamport would push the chain out of range). Every counter must stay inside the Rust u64.
function nextHeader(state, id, now, seen) {
  if (state.headPending) throw new Error(HEAD_PENDING_MESSAGE);
  const last = state.chain[state.chain.length - 1];
  const lamport = maxBig(big(last ? last.lamport : '0'), maxBig(big(state.lamport), big(seen?.lamport ?? 0))) + 1n;
  const seq = big(last ? last.seq : '0') + 1n;
  const epoch = maxBig(big(state.epoch), big(seen?.epoch ?? 0));
  if (lamport > U64_MAX || seq > U64_MAX || epoch > U64_MAX) throw new Error('a counter of this phone chain would leave the u64 range: refusing to sign');
  return {
    device_id: id.deviceId,
    seq,
    lamport,
    prev_hash: last ? last.hash : ZERO32,
    timestamp: BigInt(Math.floor(now)),
    epoch,
  };
}

// Every integer inside the body of an op this phone authors must be a Rust u64 (the header is checked by parseOp): an op
// a terminal cannot decode would cut the phone chain at that seq.
function assertU64Leaves(v) {
  if (typeof v === 'number' || typeof v === 'bigint') {
    if (!isU64(v)) throw new Error(`refusing to sign: the number ${v} is outside u64`);
  } else if (Array.isArray(v)) v.forEach(assertU64Leaves);
  else if (v && typeof v === 'object') Object.values(v).forEach(assertU64Leaves);
}

function appendCanonical(state, id, canonical, now) {
  const op = parseOp(canonical);
  assertU64Leaves(op.fields);
  const signature = signOp(id.edSeed, canonical);
  const entry = {
    seq: String(op.seq), lamport: String(op.lamport), timestamp: String(op.timestamp),
    hash: hexOf(blake3(canonical)), canonical: hexOf(canonical), signature: hexOf(signature),
  };
  state.chain.push(entry);
  state.lamport = entry.lamport;
  state.epoch = String(op.epoch);
  return { canonical, signature, entry };
}

// Self head attestation: "my head before this op was (seq, hash)" with a fresh timestamp. This
// is the phone's proof of recent contact (PHONE_FRESH_SECS) that rides every response.
export function appendHeadAttest(state, id, now, seen) {
  const last = state.chain[state.chain.length - 1];
  const h = nextHeader(state, id, now, seen);
  const canonical = encodeHeadAttest(h, { device_id: id.deviceId, seq: last ? last.seq : '0', chain_hash: last ? last.hash : ZERO32 });
  return appendCanonical(state, id, canonical, now);
}

export function appendReenrollCancel(state, id, reenrollHashHex, now, seen) {
  return appendCanonical(state, id, encodePhoneReenrollCancel(nextHeader(state, id, now, seen), reenrollHashHex), now);
}

// Announces the next wrap key generation (C14 step 1). The caller then rebuilds the identity at
// the new epoch; blobs wrapped to older generations stay readable (see keys.mjs).
export function appendWrapKeyRotate(state, id, newGen, now, seen) {
  const wrap_epoch = (state.wrapEpoch || 0) + 1;
  if (wrap_epoch > WRAP_GEN_MAX) throw new Error(`this phone cannot open wrap key generations above ${WRAP_GEN_MAX}`);
  const r = appendCanonical(state, id, encodeWrapKeyRotate(nextHeader(state, id, now, seen), { device_id: id.deviceId, wrap_epoch, x25519_pub: newGen.xPub, mlkem_pub: newGen.mlkemPub }), now);
  state.wrapEpoch = wrap_epoch;
  return r;
}

export const appendShareStorageKey = (state, id, epoch, sealed, now, seen) =>
  appendCanonical(state, id, encodeShareStorageKey(nextHeader(state, id, now, seen), { epoch, sealed }), now);

export const appendConfirmBatch = (state, id, epoch, items, now, seen) =>
  appendCanonical(state, id, encodeShareReshareConfirmBatch(nextHeader(state, id, now, seen), { epoch, items }), now);

export function appendRedPhoneSign(state, id, opHashHex, now, seen) {
  return appendCanonical(state, id, encodeRedPhoneSign(nextHeader(state, id, now, seen), opHashHex), now);
}

// Trust sessions (4c-2): the phone is the author of CertIssue, CertRevoke and CertExtend. The caller (cert.mjs)
// has already enforced the limits; these only chain, sign and return the new entry (entry.hash is the cert id).
export const appendCertIssue = (state, id, req, now, seen) => appendCanonical(state, id, encodeCertIssue(nextHeader(state, id, now, seen), req), now);
export const appendCertRevoke = (state, id, certIdHex, now, seen) => appendCanonical(state, id, encodeCertRevoke(nextHeader(state, id, now, seen), certIdHex), now);
export const appendCertRenew = (state, id, certIdHex, expiry, now, seen) => appendCanonical(state, id, encodeCertRenew(nextHeader(state, id, now, seen), certIdHex, expiry), now);
export const appendPhoneHandover = (state, id, newPhone, name, now, seen) => appendCanonical(state, id, encodePhoneHandover(nextHeader(state, id, now, seen), newPhone, name), now);
export const appendCertExtend = (state, id, certIdHex, extraHosts, now, seen) => appendCanonical(state, id, encodeCertExtend(nextHeader(state, id, now, seen), certIdHex, extraHosts), now);

// Newest ops of the chain as { canonical, signature } for the courier lists.
export function courierOps(state, count = COURIER_TAIL) {
  const hex = (s) => Uint8Array.from(s.match(/../g) ?? [], (b) => parseInt(b, 16));
  return state.chain.slice(-count).map((e) => ({ canonical: hex(e.canonical), signature: hex(e.signature) }));
}

// ------------------------------------------------------------ short-code snapshots

export const liveSnapshot = (state, hostHex, now) => {
  const s = state.snapshots[hostHex];
  return s && now < s.expires_at ? s : null;
};

// Dash-grouped Crockford code, 16 characters of 80 random bits: K7Q2-9FMX-C41T-8PWD.
export const newSnapshotCode = () => crockfordEncode(randomBytes(10)).replace(/(.{4})(?=.)/g, '$1-');

// One live snapshot per credential, enforced HERE (the phone), not by the peer. Throws when one
// is live. A device that cannot run the floor profile falls back to a weaker recorded one.
export async function issueSnapshot(state, hostHex, shareIndex, shareBytes, now, { code = newSnapshotCode(), profiles = [SNAPSHOT_PROFILE, SNAPSHOT_PROFILE_WEAK] } = {}) {
  const live = liveSnapshot(state, hostHex, now);
  if (live) throw new Error(`a short code for this credential is already live until ${new Date(live.expires_at * 1000).toISOString()}`);
  let blob;
  let profile;
  let lastErr;
  for (const p of profiles) {
    try {
      blob = await snapshotSeal(shareIndex, shareBytes, code, Math.floor(now), p, randomBytes(16), randomBytes(24), Uint8Array.from(hostHex.match(/../g), (b) => parseInt(b, 16)));
      profile = p;
      break;
    } catch (e) {
      lastErr = e;
    }
  }
  if (!blob) throw new Error(`no Argon2 profile fits this device: ${lastErr && lastErr.message}`);
  const rec = { issued_at: Math.floor(now), expires_at: Math.floor(now) + SNAPSHOT_TTL_SECS, profile: { mem_kib: profile.mem_kib, iters: profile.iters }, weak: profile !== profiles[0] };
  return { code, blob, record: rec };
}

export function recordSnapshot(state, hostHex, record) {
  state.snapshots[hostHex] = record;
}

// The user declares the old code dead (the credential's DEK was reissued, or the blob never
// reached the peer). Without this a failed delivery would block the credential for 7 days.
export const invalidateSnapshot = (state, hostHex) => void delete state.snapshots[hostHex];
