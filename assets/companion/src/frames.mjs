// Phone-side wire frames beyond the challenge and the share response (those live in quorum.mjs).
// Layouts are mirrored in src/sync/wire.rs (kinds 3, 5..8) and src/sync/share_release.rs (the
// reply extension); docs/host-sync-contract.md C16 is the single description.
//
//   frame      = "SWR1" || version u16 BE || kind u8 || body
//   kind 3     = recovery notice (terminal -> phone): op_len u32 BE || op_canonical || op_signature(64)
//   kind 5     = phone ops (phone -> terminal): n u8 || n * (op_len u32 BE || op_canonical || sig(64))
//   kind 6     = enroll commit:  role u8 || commit(32)
//   kind 7     = enroll reveal:  role u8 || reveal bytes (enroll.rs Reveal::bytes)
//   kind 8     = enroll confirm: role u8 || ok u8 || signature(64)
//   reply      = share(33) [|| 0x01 || approval_sig(64) || n u8 || n * (len u32 BE || op || sig(64))
//                           || snap u8 [|| host_uid(16) || blob_len u16 BE || blob]]
import { blake3 } from '../vendor/noble-hashes-2.4.0/blake3.js';
import { concat, utf8 } from './lib.mjs';

export const KIND = { CHALLENGE: 1, RESPONSE: 2, RECOVERY_NOTICE: 3, PHONE_OPS: 5, ENROLL_COMMIT: 6, ENROLL_REVEAL: 7, ENROLL_CONFIRM: 8 };
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
  if (ops.length > 255) throw new Error('too many ops');
  return concat(Uint8Array.of(ops.length), ...ops.flatMap((o) => [u32(o.canonical.length), o.canonical, o.signature]));
}

// Returns { ops, end } for a list starting at body[at].
function decodeOpList(body, at) {
  if (body.length < at + 1) throw new Error('op list truncated');
  const n = body[at++];
  const ops = [];
  for (let i = 0; i < n; i++) {
    if (body.length < at + 4) throw new Error('op list truncated');
    const len = rd32(body, at);
    at += 4;
    if (body.length < at + len + 64) throw new Error('op list truncated');
    ops.push({ canonical: body.slice(at, at + len), signature: body.slice(at + len, at + len + 64) });
    at += len + 64;
  }
  return { ops, end: at };
}

export const encodePhoneOps = (ops) => encodeFrame(KIND.PHONE_OPS, encodeOpList(ops));

export function decodeRecoveryNotice(frame) {
  const { kind, body } = decodeFrame(frame);
  if (kind !== KIND.RECOVERY_NOTICE) throw new Error('not a recovery notice');
  if (body.length < 4 + 64) throw new Error('recovery notice too short');
  const len = rd32(body, 0);
  if (body.length !== 4 + len + 64) throw new Error('recovery notice length does not match its fields');
  return { op_canonical: body.slice(4, 4 + len), op_signature: body.slice(4 + len) };
}

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
