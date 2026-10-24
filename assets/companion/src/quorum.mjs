// Quorum wire pieces the phone needs: Crockford short codes, snapshot open, SWR1 frames,
// channel binding, phone material and the animated-QR fountain. Byte formats follow
// docs/host-sync-contract.md and the Rust modules they mirror (codes.rs, snapshot.rs,
// wire.rs, fountain.rs). Every function here is replayed against tests/vectors/quorum.
import { blake3 } from '../vendor/noble-hashes-2.4.0/blake3.js';
import { xchacha20poly1305 } from '../vendor/noble-ciphers-2.4.0/chacha.js';
import { argon2idDerive, concat } from './lib.mjs';

const ALPHABET = '0123456789ABCDEFGHJKMNPQRSTVWXYZ';
const ARGON2_MAX_MEM_KIB = 2 * 1048576;
const ARGON2_MAX_ITERS = 16;
const SNAPSHOT_TTL_SECS = 7 * 86400;
const TIMESTAMP_SKEW_GRACE_SECS = 600;
const SNAPSHOT_MAGIC = [0x53, 0x53, 0x48, 0x4e]; // "SSHN"
const SNAPSHOT_HEADER = 4 + 1 + 4 + 4 + 8 + 16;
const WIRE_MAGIC = [0x53, 0x57, 0x52, 0x31]; // "SWR1"
const WIRE_VERSION = 1;
const KIND_CHALLENGE = 1;
const KIND_SHARE_RESPONSE = 2;
const DOMAIN_TAG = new TextEncoder().encode('sshub/sync/challenge/v1');
const FOUNTAIN_MAGIC = 0x51;
export const FOUNTAIN_HEADER = 8;

const u16be = (n) => Uint8Array.of((n >>> 8) & 0xff, n & 0xff);
const u16le = (n) => Uint8Array.of(n & 0xff, (n >>> 8) & 0xff);
const u64be = (n) => beBytes(BigInt(n), 8);
const u64le = (n) => beBytes(BigInt(n), 8).reverse();
const i64le = (n) => beBytes(BigInt.asUintN(64, BigInt(n)), 8).reverse();
function beBytes(v, len) {
  const out = new Uint8Array(len);
  for (let i = len - 1; i >= 0; i--) {
    out[i] = Number(v & 0xffn);
    v >>= 8n;
  }
  return out;
}
const readU16be = (b, at) => (b[at] << 8) | b[at + 1];
const readU32be = (b, at) => ((b[at] << 24) | (b[at + 1] << 16) | (b[at + 2] << 8) | b[at + 3]) >>> 0;
const view = (b, at) => new DataView(b.buffer, b.byteOffset + at, 8);
const readU64be = (b, at) => Number(view(b, at).getBigUint64(0));
const readI64be = (b, at) => Number(view(b, at).getBigInt64(0));

export const hexToBytes = (hex) => {
  if (hex.length % 2) throw new Error('odd hex length');
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.substr(i * 2, 2), 16);
  return out;
};
export const bytesToHex = (b) => Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('');

// Crockford base32, 5-bit groups, big-endian, no padding (codes.rs crockford_encode).
export function crockfordEncode(bytes) {
  let out = '';
  let acc = 0;
  let bits = 0;
  for (const b of bytes) {
    acc = ((acc << 8) | b) & 0xffffff;
    bits += 8;
    while (bits >= 5) {
      bits -= 5;
      out += ALPHABET[(acc >>> bits) & 31];
    }
  }
  if (bits > 0) out += ALPHABET[(acc << (5 - bits)) & 31];
  return out;
}

// Uppercases, drops spaces, tabs and hyphens, maps O to 0 and I/L to 1. Anything outside the
// alphabet (U included) is an error. Returns code '' on error so vectors compare plainly.
export function normalizeCode(input) {
  let out = '';
  for (const raw of input) {
    let c = raw.toUpperCase();
    if (c === ' ' || c === '-' || c === '\t') continue;
    if (c === 'O') c = '0';
    else if (c === 'I' || c === 'L') c = '1';
    if (!ALPHABET.includes(c)) return { code: '', ok: false };
    out += c;
  }
  return { code: out, ok: true };
}

// snapshot.rs snapshot_open. Returns { ok, share_index, share_bytes }. TTL and profile bounds
// are checked before any Argon2 work; any failure reports ok false (never a partial share).
export async function snapshotOpen(blob, code, now) {
  const fail = { ok: false, share_index: 0, share_bytes: new Uint8Array(0) };
  if (blob.length < SNAPSHOT_HEADER + 24 + 16 || !SNAPSHOT_MAGIC.every((m, i) => blob[i] === m)) return fail;
  if (blob[4] !== 1) return fail;
  const memKib = readU32be(blob, 5);
  const iters = readU32be(blob, 9);
  const issuedAt = readI64be(blob, 13);
  const salt = blob.slice(21, 37);
  if (issuedAt > now + TIMESTAMP_SKEW_GRACE_SECS) return fail;
  if (now - issuedAt > SNAPSHOT_TTL_SECS) return fail;
  if (memKib < 8 || memKib > ARGON2_MAX_MEM_KIB || iters < 1 || iters > ARGON2_MAX_ITERS) return fail;
  const norm = normalizeCode(code);
  if (!norm.ok || norm.code.length !== 16) return fail;
  const key = await argon2idDerive(norm.code, salt, { mem_kib: memKib, iters });
  const header = blob.slice(0, SNAPSHOT_HEADER);
  const nonce = blob.slice(SNAPSHOT_HEADER, SNAPSHOT_HEADER + 24);
  const ct = blob.slice(SNAPSHOT_HEADER + 24);
  let plain;
  try {
    plain = xchacha20poly1305(key, nonce, header).decrypt(ct);
  } catch {
    return fail;
  }
  if (plain.length !== 33 || plain[0] === 0) return fail;
  return { ok: true, share_index: plain[0], share_bytes: plain.slice(1) };
}

const u32be = (n) => Uint8Array.of((n >>> 24) & 0xff, (n >>> 16) & 0xff, (n >>> 8) & 0xff, n & 0xff);
const i64be = (n) => beBytes(BigInt.asUintN(64, BigInt(n)), 8);

// snapshot.rs seal_with: the phone issues a short-code snapshot of its own share. Blob header
// (magic, version, Argon2 profile, issued_at, salt) is the AAD; sealed = nonce || ciphertext.
export async function snapshotSeal(shareIndex, shareBytes, code, issuedAt, profile, salt, nonce) {
  const header = concat(
    Uint8Array.from(SNAPSHOT_MAGIC),
    Uint8Array.of(1),
    u32be(profile.mem_kib),
    u32be(profile.iters),
    i64be(issuedAt),
    salt,
  );
  const key = await argon2idDerive(normalizeCode(code).code, salt, profile);
  const ct = xchacha20poly1305(key, nonce, header).encrypt(concat(Uint8Array.of(shareIndex), shareBytes));
  return concat(header, nonce, ct);
}

// Wire frame header: "SWR1" || version u16 BE || kind u8.
const frameHeader = (kind) => concat(Uint8Array.from(WIRE_MAGIC), u16be(WIRE_VERSION), Uint8Array.of(kind));

// PhoneMaterial: kind u8 (0 hybrid, 1 envelope) || epoch u64 BE || share_len u16 BE ||
// share_blob || storage_len u16 BE || storage_blob. Mirrors wire.rs PhoneMaterial::decode.
export function decodePhoneMaterial(b) {
  if (b.length < 13) throw new Error('phone material too short');
  const kind = b[0] === 0 ? 'hybrid' : b[0] === 1 ? 'envelope' : null;
  if (!kind) throw new Error(`unknown phone material kind ${b[0]}`);
  const epoch = readU64be(b, 1);
  const shareLen = readU16be(b, 9);
  const storageAt = 11 + shareLen;
  if (b.length < storageAt + 2) throw new Error('phone material truncated');
  const storageLen = readU16be(b, storageAt);
  if (b.length !== storageAt + 2 + storageLen) throw new Error('phone material length does not match its fields');
  if (kind === 'hybrid' && storageLen !== 0) throw new Error('a hybrid share carries no storage key blob');
  if (kind === 'envelope' && storageLen === 0) throw new Error('an envelope share needs its storage key blob');
  return {
    kind,
    epoch,
    share_blob: b.slice(11, storageAt),
    storage_blob: b.slice(storageAt + 2),
  };
}

// Challenge frame (kind 1). Returns the fields plus the binding hash the phone must match.
export function decodeChallenge(frame) {
  if (frame.length < 7 + 32 + 8 + 32 + 8 + 4) throw new Error('challenge too short');
  if (!WIRE_MAGIC.every((m, i) => frame[i] === m)) throw new Error('bad wire magic');
  if (readU16be(frame, 4) !== WIRE_VERSION) throw new Error('unsupported wire version');
  if (frame[6] !== KIND_CHALLENGE) throw new Error('not a challenge frame');
  const body = frame.subarray(7);
  let at = 0;
  const nonce = body.slice(at, (at += 32));
  const expiry = readI64be(body, at);
  at += 8;
  const eph = body.slice(at, (at += 32));
  const requestId = readU64be(body, at);
  at += 8;
  const opLen = readU32be(body, at);
  at += 4;
  const op = body.slice(at, (at += opLen));
  const sig = body.slice(at, (at += 64));
  const matLen = readU32be(body, at);
  at += 4;
  if (body.length !== at + matLen) throw new Error('challenge length does not match its fields');
  const material = body.slice(at, at + matLen);
  const materialHash = blake3(material);
  const preimage = concat(DOMAIN_TAG, u16le(WIRE_VERSION), op, nonce, i64le(expiry), eph, u64le(requestId), materialHash);
  return {
    nonce,
    expiry,
    eph_x25519_pub: eph,
    request_id: requestId,
    op_canonical: op,
    op_signature: sig,
    material,
    material_hash: materialHash,
    binding_preimage: preimage,
    binding_hash: blake3(preimage),
  };
}

// Share response frame (kind 2): request_id u64 BE || len u64 BE || ciphertext.
export function encodeShareResponse(requestId, ciphertext) {
  return concat(frameHeader(KIND_SHARE_RESPONSE), u64be(requestId), u64be(ciphertext.length), ciphertext);
}

// Frames for one lap of the animated QR (fountain.rs fountain_encode). `frameBytes` is the QR
// byte budget; the block is what remains after the 8 byte header.
export function fountainEncode(payload, frameBytes, parityPercent) {
  if (payload.length < 1 || payload.length > 65535) throw new Error('payload must be 1..=65535 bytes');
  const block = frameBytes - FOUNTAIN_HEADER;
  if (block < 1) throw new Error('frame too small');
  const k = Math.ceil(payload.length / block);
  const groups = k < 2 || parityPercent === 0 ? 0 : Math.max(1, Math.ceil((k * parityPercent) / 100));
  const id = blake3(payload).slice(0, 2);
  const head = (seq) => concat(Uint8Array.of(FOUNTAIN_MAGIC), id, u16be(payload.length), Uint8Array.of(groups), u16be(seq));
  const blocks = [];
  for (let i = 0; i < k; i++) {
    const chunk = new Uint8Array(block);
    chunk.set(payload.subarray(i * block, Math.min((i + 1) * block, payload.length)));
    blocks.push(chunk);
  }
  const frames = blocks.map((chunk, i) => concat(head(i), chunk));
  for (let g = 0; g < groups; g++) {
    const parity = new Uint8Array(block);
    for (let i = g; i < k; i += groups) for (let j = 0; j < block; j++) parity[j] ^= blocks[i][j];
    frames.push(concat(head(k + g), parity));
  }
  return frames;
}

// Receiver side (fountain.rs FountainDecoder). push() returns 'progress' | 'redundant' |
// 'other' (a frame of another document); finish() returns the payload once every block is held.
export class FountainDecoder {
  constructor() {
    this.head = null;
    this.blocks = [];
    this.parity = [];
  }

  push(frame) {
    if (frame.length <= FOUNTAIN_HEADER || frame[0] !== FOUNTAIN_MAGIC) throw new Error('not a fountain frame');
    const id = frame.slice(1, 3);
    const total = readU16be(frame, 3);
    const groups = frame[5];
    const seq = readU16be(frame, 6);
    const block = frame.length - FOUNTAIN_HEADER;
    if (total === 0) throw new Error('zero total length');
    if (!this.head) {
      const k = Math.ceil(total / block);
      if (groups > k) throw new Error('more groups than blocks');
      this.head = { id, total, groups, k, block };
      this.blocks = new Array(k).fill(null);
      this.parity = new Array(groups).fill(null);
    }
    const h = this.head;
    if (!equalBytes(id, h.id) || total !== h.total || groups !== h.groups || block !== h.block) return 'other';
    if (seq >= h.k + h.groups) throw new Error('sequence out of range');
    const data = frame.slice(FOUNTAIN_HEADER);
    let fresh;
    if (seq < h.k) {
      fresh = this.blocks[seq] === null;
      if (fresh) this.blocks[seq] = data;
    } else {
      const g = seq - h.k;
      fresh = this.parity[g] === null;
      if (fresh) this.parity[g] = data;
    }
    this.repair();
    return fresh ? 'progress' : 'redundant';
  }

  // Recovers one missing block per group when its parity frame is held.
  repair() {
    const h = this.head;
    if (!h) return;
    for (let g = 0; g < h.groups; g++) {
      const p = this.parity[g];
      if (p === null) continue;
      const members = [];
      for (let i = g; i < h.k; i += h.groups) members.push(i);
      const missing = members.filter((i) => this.blocks[i] === null);
      if (missing.length !== 1) continue;
      const x = p.slice();
      for (const i of members) {
        if (i === missing[0]) continue;
        for (let j = 0; j < x.length; j++) x[j] ^= this.blocks[i][j];
      }
      this.blocks[missing[0]] = x;
    }
  }

  finish() {
    const h = this.head;
    if (!h || this.blocks.some((b) => b === null)) return null;
    const joined = concat(...this.blocks).slice(0, h.total);
    if (!equalBytes(blake3(joined).slice(0, 2), h.id)) {
      throw new Error('reassembled payload does not match its document id');
    }
    return joined;
  }
}

function equalBytes(a, b) {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}
