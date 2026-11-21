// Phone-side wire frames beyond the challenge and the share response (those live in quorum.mjs).
// Layouts are mirrored in src/sync/wire.rs (kinds 3, 5..8) and src/sync/share_release.rs (the
// reply extension); docs/host-sync-contract.md C16 is the single description.
//
//   frame      = "SWR1" || version u16 BE || kind u8 || body
//   kind 3     = recovery notice (terminal -> phone): op_len u32 BE || op_canonical || op_signature(64)
//   kind 5     = ops list (both directions): n u8 || n * (op_len u32 BE || op_canonical || sig(64)); at most 8 ops,
//                each at most 256 KiB, the list at most 1 MiB (wire.rs MAX_COURIER_OPS, MAX_COURIER_OP_BYTES)
//   kind 6     = enroll commit:  role u8 || commit(32)
//   kind 7     = enroll reveal:  role u8 || reveal bytes (enroll.rs Reveal::bytes)
//   kind 8     = enroll confirm: role u8 || ok u8 || signature(64)
//   kind 9     = trust-session request (terminal -> phone): device_id(8) || scope tag u8 [0: group uid(16) |
//                1: n u16 BE (1..=64) || n * uid(16)] || expiry i64 BE || max_new_hosts_per_day u32 BE ||
//                epoch_lo u64 BE || epoch_hi u64 BE   (wire.rs CertRequestWire)
//   kind 10    = trust-session budget extension request: cert_id(32) || extra_hosts u32 BE (CertExtendRequestWire)
//   reply      = share(33) [|| 0x01 || approval_sig(64) || n u8 || n * (len u32 BE || op || sig(64))
//                           || snap u8 [|| host_uid(16) || blob_len u16 BE || blob]]
import { blake3 } from '../vendor/noble-hashes-2.4.0/blake3.js';
import { concat, utf8 } from './lib.mjs';

export const KIND = { CHALLENGE: 1, RESPONSE: 2, RECOVERY_NOTICE: 3, PHONE_OPS: 5, ENROLL_COMMIT: 6, ENROLL_REVEAL: 7, ENROLL_CONFIRM: 8, CERT_REQUEST: 9, CERT_EXTEND_REQUEST: 10, CERT_RENEW_REQUEST: 11, PHONE_HANDOVER_REQUEST: 12 };
export const MAX_COURIER_OPS = 8;
export const MAX_COURIER_OP_BYTES = 256 * 1024;
export const MAX_COURIER_LIST_BYTES = 1024 * 1024;
export const MAX_CERT_REQUEST_HOSTS = 64;
export const CERT_NEW_HOSTS_PER_DAY = 50; // limits.rs
export const ROLE_JOINING = 0; // the phone: the joining device (enroll.rs INITIATOR)
export const ROLE_EXISTING = 1; // the terminal that enrols it (enroll.rs RESPONDER)
const MAGIC = Uint8Array.of(0x53, 0x57, 0x52, 0x31);
const MAX_NAME_BYTES = 64;

const u16 = (n) => Uint8Array.of((n >>> 8) & 0xff, n & 0xff);
const u32 = (n) => Uint8Array.of((n >>> 24) & 0xff, (n >>> 16) & 0xff, (n >>> 8) & 0xff, n & 0xff);
const rd16 = (b, at) => (b[at] << 8) | b[at + 1];
const rd32 = (b, at) => ((b[at] << 24) | (b[at + 1] << 16) | (b[at + 2] << 8) | b[at + 3]) >>> 0;

export const encodeFrame = (kind, ...body) => concat(MAGIC, u16(1), Uint8Array.of(kind), ...body);

// Returns { kind, body } or throws.
export function decodeFrame(frame) {
  if (frame.length < 7 || !MAGIC.every((m, i) => frame[i] === m)) throw new Error('not a sshub frame');
  if (rd16(frame, 4) !== 1) throw new Error('unsupported wire version');
  return { kind: frame[6], body: frame.subarray(7) };
}

// ------------------------------------------------------------ ops lists

function encodeOpList(ops) {
  if (ops.length > MAX_COURIER_OPS) throw new Error(`at most ${MAX_COURIER_OPS} ops per list`);
  for (const o of ops) if (o.canonical.length > MAX_COURIER_OP_BYTES) throw new Error(`an op over ${MAX_COURIER_OP_BYTES} bytes does not fit a list`);
  const out = concat(Uint8Array.of(ops.length), ...ops.flatMap((o) => [u32(o.canonical.length), o.canonical, o.signature]));
  if (out.length > MAX_COURIER_LIST_BYTES) throw new Error('the list is over 1 MiB');
  return out;
}

// Returns { ops, end } for a list starting at body[at]; the caps are those of wire.rs decode_op_list.
function decodeOpList(body, at) {
  if (body.length < at + 1) throw new Error('op list truncated');
  if (body.length - at > MAX_COURIER_LIST_BYTES) throw new Error('op list exceeds the frame cap');
  const n = body[at++];
  if (n > MAX_COURIER_OPS) throw new Error(`op list of ${n} ops exceeds the cap of ${MAX_COURIER_OPS}`);
  const ops = [];
  for (let i = 0; i < n; i++) {
    if (body.length < at + 4) throw new Error('op list truncated');
    const len = rd32(body, at);
    at += 4;
    if (len > MAX_COURIER_OP_BYTES || body.length < at + len + 64) throw new Error('op list entry is oversize or truncated');
    ops.push({ canonical: body.slice(at, at + len), signature: body.slice(at + len, at + len + 64) });
    at += len + 64;
  }
  return { ops, end: at };
}

// Optional trailing block of a kind 5 frame: 0x01 || terminal device id (8 bytes). UNSIGNED: a phone restored from its words
// has no ring to check a signature against, so the id only tells two lists apart (an honest terminal that is behind, or one
// animation scanned twice); it is not proof of who made the list. Phone-to-terminal exports carry none.
export const PHONE_OPS_SENDER_TAG = 1;
// Second optional trailing block, after the sender block: 0x02 || count u8 || newest_op_ts i64 BE. The terminal's own count
// of the non-revoked devices of kind Terminal in its view (itself included) and the author stamp of the newest op in the
// list. UNSIGNED and informational: it only decides whether one list may be accepted with the user's tick.
export const PHONE_OPS_TERMINALS_TAG = 2;

export function encodePhoneOps(ops, sender = null, terminals = null) {
  const list = encodeOpList(ops);
  const parts = [list];
  if (sender != null) {
    if (sender.length !== 8) throw new Error('the sender id is 8 bytes');
    parts.push(Uint8Array.of(PHONE_OPS_SENDER_TAG), sender);
  }
  if (terminals != null) parts.push(Uint8Array.of(PHONE_OPS_TERMINALS_TAG, terminals.count), i64(terminals.newestOpTs));
  return encodeFrame(KIND.PHONE_OPS, concat(...parts));
}

// Kind 5 carries ops in both directions: the phone's own ops to a terminal, and the reshare ops
// of an epoch (sessions sealed to the phone, batches) from a terminal to the phone.
export function decodePhoneOps(frame) {
  const { kind, body } = decodeFrame(frame);
  if (kind !== KIND.PHONE_OPS) throw new Error('not an ops frame');
  const { ops, end } = decodeOpList(body, 0);
  let at = end;
  let sender = null;
  let terminals = null;
  if (at < body.length && body[at] === PHONE_OPS_SENDER_TAG) {
    if (body.length - at < 9) throw new Error('ops frame: the sender block is truncated or has trailing bytes');
    sender = body.slice(at + 1, at + 9);
    at += 9;
  }
  if (at < body.length && body[at] === PHONE_OPS_TERMINALS_TAG) {
    if (body.length - at !== 10) throw new Error('ops frame: the terminals block is truncated or has trailing bytes');
    terminals = { count: body[at + 1], newestOpTs: rd64(body, at + 2).getBigInt64(0) };
    at += 10;
  }
  if (at !== body.length) throw new Error(end === at ? 'ops frame has trailing bytes' : 'ops frame: the sender block is truncated or has trailing bytes');
  // the ops array itself stays what every caller expects; the sender and terminals (or null) ride along, not enumerable
  Object.defineProperty(ops, 'sender', { value: sender, enumerable: false });
  Object.defineProperty(ops, 'terminals', { value: terminals, enumerable: false });
  return ops;
}

export function decodeRecoveryNotice(frame) {
  const { kind, body } = decodeFrame(frame);
  if (kind !== KIND.RECOVERY_NOTICE) throw new Error('not a recovery notice');
  if (body.length < 4 + 64) throw new Error('recovery notice too short');
  const len = rd32(body, 0);
  if (body.length !== 4 + len + 64) throw new Error('recovery notice length does not match its fields');
  return { op_canonical: body.slice(4, 4 + len), op_signature: body.slice(4 + len) };
}

// ------------------------------------------------------------ trust sessions (kinds 9 and 10)

const rd64 = (b, at) => new DataView(b.buffer, b.byteOffset + at, 8);
const hexU = (b) => Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('');
const u64 = (n) => {
  const b = new Uint8Array(8);
  new DataView(b.buffer).setBigUint64(0, BigInt(n));
  return b;
};
const i64 = (n) => {
  const b = new Uint8Array(8);
  new DataView(b.buffer).setBigInt64(0, BigInt(n));
  return b;
};

// { device_id: hex(16), scope: { group } | { hosts: [hex32] }, expiry: BigInt, max_new_hosts_per_day: number,
//   epoch_lo: BigInt, epoch_hi: BigInt }. Throws on anything wire.rs CertRequestWire::decode refuses.
export function decodeCertRequest(frame) {
  const { kind, body } = decodeFrame(frame);
  if (kind !== KIND.CERT_REQUEST) throw new Error('not a trust-session request');
  const short = () => new Error('the trust-session request is truncated');
  if (body.length < 9) throw short();
  const device_id = hexU(body.subarray(0, 8));
  let at = 9;
  let scope;
  if (body[8] === 0) {
    if (body.length < at + 16) throw short();
    scope = { group: hexU(body.subarray(at, at + 16)) };
    at += 16;
  } else if (body[8] === 1) {
    if (body.length < at + 2) throw short();
    const n = rd16(body, at);
    at += 2;
    if (n === 0 || n > MAX_CERT_REQUEST_HOSTS) throw new Error(`a trust-session request lists 1..=${MAX_CERT_REQUEST_HOSTS} hosts, got ${n}`);
    const hosts = [];
    for (let i = 0; i < n; i++) {
      if (body.length < at + 16) throw short();
      hosts.push(hexU(body.subarray(at, at + 16)));
      at += 16;
    }
    scope = { hosts };
  } else throw new Error(`unknown scope tag ${body[8]}`);
  if (body.length !== at + 28) throw new Error('the trust-session request has trailing or missing bytes');
  return {
    device_id,
    scope,
    expiry: rd64(body, at).getBigInt64(0),
    max_new_hosts_per_day: rd32(body, at + 8),
    epoch_lo: rd64(body, at + 12).getBigUint64(0),
    epoch_hi: rd64(body, at + 20).getBigUint64(0),
  };
}

// The terminal side (tests only): the inverse of decodeCertRequest.
export function encodeCertRequest(r) {
  const hex = (s) => Uint8Array.from(s.match(/../g), (b) => parseInt(b, 16));
  const scope = r.scope.group ? concat(Uint8Array.of(0), hex(r.scope.group)) : concat(Uint8Array.of(1), u16(r.scope.hosts.length), ...r.scope.hosts.map(hex));
  return encodeFrame(KIND.CERT_REQUEST, hex(r.device_id), scope, i64(r.expiry), u32(r.max_new_hosts_per_day), u64(r.epoch_lo), u64(r.epoch_hi));
}

// { cert_id: hex(32), extra_hosts }; extra_hosts 1..=CERT_NEW_HOSTS_PER_DAY like wire.rs.
export function decodeCertExtendRequest(frame) {
  const { kind, body } = decodeFrame(frame);
  if (kind !== KIND.CERT_EXTEND_REQUEST) throw new Error('not a budget extension request');
  if (body.length !== 36) throw new Error(`a budget extension request is 36 bytes, got ${body.length}`);
  const extra_hosts = rd32(body, 32);
  if (extra_hosts === 0 || extra_hosts > CERT_NEW_HOSTS_PER_DAY) throw new Error(`an extension is 1 to ${CERT_NEW_HOSTS_PER_DAY} hosts`);
  return { cert_id: hexU(body.subarray(0, 32)), extra_hosts };
}

// Kind 12 (contract 4j): ed25519(32) || x25519(32) || mlkem_len u16 BE || mlkem || name_len u8 || name utf8 || terminal(8).
// { new_phone: { ed25519_pub, x25519_pub, mlkem_pub: Uint8Array }, name, terminal: hex(8) }.
export function decodePhoneHandoverRequest(frame) {
  const { kind, body } = decodeFrame(frame);
  if (kind !== KIND.PHONE_HANDOVER_REQUEST) throw new Error('not a handover request');
  const short = () => new Error('a handover request is truncated');
  if (body.length < 67) throw short();
  const mlen = (body[64] << 8) | body[65];
  const nameAt = 66 + mlen;
  if (body.length < nameAt + 1) throw short();
  const nlen = body[nameAt];
  const termAt = nameAt + 1 + nlen;
  if (body.length < termAt + 8) throw short();
  // contract 4l b: the admission frame shown to the NEW phone carries 0x01 || op_hash(32) || op_lamport u64 BE after the terminal id
  let op = null;
  const rest = body.length - (termAt + 8);
  if (rest === 41 && body[termAt + 8] === 1) op = { hash: hexU(body.subarray(termAt + 9, termAt + 41)), lamport: rd64(body, termAt + 41).getBigUint64(0) };
  else if (rest !== 0) throw new Error('a handover request does not match its lengths');
  let name;
  try {
    name = new TextDecoder('utf-8', { fatal: true }).decode(body.subarray(nameAt + 1, termAt));
  } catch {
    throw new Error('a handover name is not UTF-8');
  }
  return { new_phone: { ed25519_pub: body.slice(0, 32), x25519_pub: body.slice(32, 64), mlkem_pub: body.slice(66, nameAt) }, name, terminal: hexU(body.subarray(termAt, termAt + 8)), op };
}

// The terminal side (tests only): the inverse of decodePhoneHandoverRequest.
export function encodePhoneHandoverRequest(r) {
  const nm = utf8(r.name);
  return encodeFrame(KIND.PHONE_HANDOVER_REQUEST, r.new_phone.ed25519_pub, r.new_phone.x25519_pub, u16(r.new_phone.mlkem_pub.length), r.new_phone.mlkem_pub, Uint8Array.of(nm.length), nm, Uint8Array.from(r.terminal.match(/../g), (b) => parseInt(b, 16)), ...(r.op ? [Uint8Array.of(1), Uint8Array.from(r.op.hash.match(/../g), (b) => parseInt(b, 16)), u64(r.op.lamport)] : []));
}

// Kind 11 (contract 4j): cert_id(32) || expiry i64 BE || n u8 || n x (terminal(8) || head_seq u64 BE || head_ts i64 BE), n <= 32.
// { cert_id: hex(32), expiry: BigInt, evidence: [{ terminal: hex(8), head_seq: BigInt, head_ts: BigInt }] }.
export const MAX_RENEW_EVIDENCE = 32;
export const MAX_RENEW_OPENED = 8;
export const MAX_RENEW_OPENED_NAME_BYTES = 48;
export function decodeCertRenewRequest(frame) {
  const { kind, body } = decodeFrame(frame);
  if (kind !== KIND.CERT_RENEW_REQUEST) throw new Error('not a certificate renewal request');
  if (body.length < 41) throw new Error('a certificate renewal request is truncated');
  const n = body[40];
  if (n > MAX_RENEW_EVIDENCE) throw new Error(`a renewal request carries at most ${MAX_RENEW_EVIDENCE} evidence entries`);
  if (body.length < 41 + n * 24) throw new Error('a certificate renewal request does not match its evidence count');
  const evidence = [];
  for (let i = 0; i < n; i++) {
    const at = 41 + i * 24;
    evidence.push({ terminal: hexU(body.subarray(at, at + 8)), head_seq: rd64(body, at + 8).getBigUint64(0), head_ts: rd64(body, at + 16).getBigInt64(0) });
  }
  // contract 4l: optional opened-hosts block m u8 (<= 8) || m x (len u8 || name utf8, 1..=48) || total u16 BE; absent = no list sent
  let opened = [];
  let opened_total = 0;
  let at = 41 + n * 24;
  if (at < body.length) {
    const bad = () => new Error('the opened-hosts block of a renewal request is malformed');
    const m = body[at++];
    if (m > MAX_RENEW_OPENED) throw new Error(`a renewal request names at most ${MAX_RENEW_OPENED} opened hosts`);
    for (let i = 0; i < m; i++) {
      if (at >= body.length) throw bad();
      const len = body[at++];
      if (len < 1 || len > MAX_RENEW_OPENED_NAME_BYTES || at + len > body.length) throw bad();
      try {
        opened.push(new TextDecoder('utf-8', { fatal: true }).decode(body.subarray(at, at + len)));
      } catch {
        throw bad();
      }
      at += len;
    }
    if (body.length - at !== 2) throw bad();
    opened_total = (body[at] << 8) | body[at + 1];
    if (opened_total < m || (m === 0 && opened_total === 0)) throw bad();
  }
  return { cert_id: hexU(body.subarray(0, 32)), expiry: rd64(body, 32).getBigInt64(0), evidence, opened, opened_total };
}

// The terminal side (tests only): the inverse of decodeCertRenewRequest.
export function encodeCertRenewRequest(r) {
  const hex = (s) => Uint8Array.from(s.match(/../g), (b) => parseInt(b, 16));
  const opened = r.opened || [];
  const block = opened.length || r.opened_total ? [Uint8Array.of(opened.length), ...opened.map((nm) => concat(Uint8Array.of(utf8(nm).length), utf8(nm))), u16(r.opened_total)] : [];
  return encodeFrame(KIND.CERT_RENEW_REQUEST, hex(r.cert_id), i64(r.expiry), Uint8Array.of(r.evidence.length), ...r.evidence.map((e) => concat(hex(e.terminal), u64(e.head_seq), i64(e.head_ts))), ...block);
}

export const encodeCertExtendRequest = (r) => encodeFrame(KIND.CERT_EXTEND_REQUEST, Uint8Array.from(r.cert_id.match(/../g), (b) => parseInt(b, 16)), u32(r.extra_hosts));

// ------------------------------------------------------------ share reply plaintext

// share: 33 bytes (index || share). ext optional: { approvalSig, ops, snapshot: {host_uid(16), blob} | null }
export function buildReplyPlain(share33, ext) {
  if (share33.length !== 33) throw new Error('a share is 33 bytes');
  if (!ext) return share33;
  const snap = ext.snapshot
    ? concat(Uint8Array.of(1), ext.snapshot.host_uid, u16(ext.snapshot.blob.length), ext.snapshot.blob)
    : Uint8Array.of(0);
  return concat(share33, Uint8Array.of(1), ext.approvalSig, encodeOpList(ext.ops), snap);
}

export function parseReplyPlain(b) {
  if (b.length < 33) throw new Error('reply too short');
  const share = b.slice(0, 33);
  if (b.length === 33) return { share, ext: null };
  if (b[33] !== 1) throw new Error('unknown reply extension');
  const approvalSig = b.slice(34, 98);
  const { ops, end } = decodeOpList(b, 98);
  const flag = b[end];
  let at = end + 1;
  let snapshot = null;
  if (flag === 1) {
    const host_uid = b.slice(at, at + 16);
    const len = rd16(b, at + 16);
    snapshot = { host_uid, blob: b.slice(at + 18, at + 18 + len) };
    at += 18 + len;
  } else if (flag !== 0) throw new Error('bad snapshot flag');
  if (at !== b.length) throw new Error('reply has trailing bytes');
  return { share, ext: { approvalSig, ops, snapshot } };
}

// ------------------------------------------------------------ enrollment (enroll.rs byte layouts)

const COMMIT_CTX = utf8('sshub/sync/enroll/commit/v1');
const TRANSCRIPT_CTX = utf8('sshub/sync/enroll/transcript/v1');
const CONFIRM_CTX = utf8('sshub/sync/enroll/confirm/v1');

// Reveal = { ed25519_pub, x25519_pub, mlkem_pub, eph, nonce, name, attended }, enroll.rs Reveal::bytes.
export function revealBytes(r) {
  const name = utf8(r.name);
  if (name.length > MAX_NAME_BYTES) throw new Error('device name too long');
  return concat(r.ed25519_pub, r.x25519_pub, u16(r.mlkem_pub.length), r.mlkem_pub, r.eph, r.nonce, u16(name.length), name, Uint8Array.of(r.attended ? 1 : 0));
}

export function parseReveal(b) {
  if (b.length < 32 + 32 + 2) throw new Error('reveal truncated');
  let at = 64;
  const mlen = rd16(b, at);
  at += 2;
  if (b.length < at + mlen + 64 + 2) throw new Error('reveal truncated');
  const mlkem_pub = b.slice(at, at + mlen);
  at += mlen;
  const eph = b.slice(at, at + 32);
  const nonce = b.slice(at + 32, at + 64);
  at += 64;
  const nlen = rd16(b, at);
  at += 2;
  if (nlen > MAX_NAME_BYTES || b.length !== at + nlen + 1) throw new Error('reveal length does not match its fields');
  let name;
  try {
    name = new TextDecoder('utf-8', { fatal: true }).decode(b.slice(at, at + nlen));
  } catch {
    throw new Error('reveal name is not UTF-8');
  }
  if (/[\u0000-\u001f\u007f]/.test(name)) throw new Error('device name has control characters');
  // enroll.rs Reveal::bytes writes 0 or 1 only: another byte would give a second encoding of the same reveal.
  if (b[at + nlen] > 1) throw new Error('reveal attended flag is not 0 or 1');
  return { ed25519_pub: b.slice(0, 32), x25519_pub: b.slice(32, 64), mlkem_pub, eph, nonce, name, attended: b[at + nlen] === 1 };
}

export const commitOf = (role, reveal) => blake3(concat(COMMIT_CTX, Uint8Array.of(role), revealBytes(reveal)));
export const transcriptHash = (initiator, responder) => blake3(concat(TRANSCRIPT_CTX, revealBytes(initiator), revealBytes(responder)));
export const confirmMessage = (role, ok, th) => concat(CONFIRM_CTX, Uint8Array.of(role, ok ? 1 : 0), th);

export const encodeEnrollCommit = (role, commit) => encodeFrame(KIND.ENROLL_COMMIT, Uint8Array.of(role), commit);
export const encodeEnrollReveal = (role, reveal) => encodeFrame(KIND.ENROLL_REVEAL, Uint8Array.of(role), revealBytes(reveal));
export const encodeEnrollConfirm = (role, ok, sig) => encodeFrame(KIND.ENROLL_CONFIRM, Uint8Array.of(role, ok ? 1 : 0), sig);

export function decodeEnroll(frame) {
  const { kind, body } = decodeFrame(frame);
  if (body.length < 1) throw new Error('enrollment frame too short');
  const role = body[0];
  if (kind === KIND.ENROLL_COMMIT) {
    if (body.length !== 33) throw new Error('enroll commit must be 33 bytes');
    return { kind: 'commit', role, commit: body.slice(1) };
  }
  if (kind === KIND.ENROLL_REVEAL) return { kind: 'reveal', role, reveal: parseReveal(body.subarray(1)) };
  if (kind === KIND.ENROLL_CONFIRM) {
    if (body.length !== 66) throw new Error('enroll confirm must be 66 bytes');
    return { kind: 'confirm', role, ok: body[1] === 1, sig: body.slice(2) };
  }
  throw new Error('not an enrollment frame');
}
