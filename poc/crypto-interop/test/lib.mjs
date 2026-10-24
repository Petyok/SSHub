// PoC A - JS reproduction of the Rust stage-0/1/3 primitives.
// Byte-for-byte parity targets are in ../src/support/{primitives,shamir}.rs and
// .claude/host-sync/{stage-0-crypto,stage-1-oplog-store,stage-3-creds-quorum}.md.
// Every deviation from Rust behaviour is a bug: this file's comment "Rust:" marks
// the exact Rust source each block reproduces.

import { blake3 } from '../vendor/noble-hashes-2.4.0/blake3.js';
import { utf8ToBytes } from '../vendor/noble-hashes-2.4.0/utils.js';
import { ed25519, x25519 } from '../vendor/noble-curves-2.4.0/ed25519.js';
import {
  xchacha20poly1305,
  chacha20orig,
  hchacha,
} from '../vendor/noble-ciphers-2.4.0/chacha.js';
import { poly1305 } from '../vendor/noble-ciphers-2.4.0/_poly1305.js';
import { ml_kem768 } from '../vendor/noble-post-quantum-0.7.1/ml-kem.js';
import {
  entropyToMnemonic,
  mnemonicToEntropy,
} from '../vendor/scure-bip39-2.4.0/index.js';
import { wordlist as bip39Wordlist } from '../vendor/scure-bip39-2.4.0/wordlists/english.js';
import { argon2id } from '../vendor/hash-wasm-4.12.0/dist/index.esm.js';

// ---------------------------------------------------------------- byte helpers

export const utf8 = (s) => utf8ToBytes(s);
const HEX = '0123456789abcdef';
export const hexOf = (b) => {
  let s = '';
  for (let i = 0; i < b.length; i++) s += HEX[b[i] >> 4] + HEX[b[i] & 15];
  return s;
};
export const bytesEqual = (a, b) => {
  if (a.length !== b.length) return false;
  let d = 0;
  for (let i = 0; i < a.length; i++) d |= a[i] ^ b[i];
  return d === 0;
};
export const bytesOf = (h) => {
  if (typeof h !== 'string' || h.length % 2 !== 0 || /[^0-9a-fA-F]/.test(h)) {
    throw new Error(`bad hex: ${String(h).slice(0, 40)}`);
  }
  const out = new Uint8Array(h.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(h.slice(i * 2, i * 2 + 2), 16);
  return out;
};
export const concat = (...parts) => {
  const len = parts.reduce((n, p) => n + p.length, 0);
  const out = new Uint8Array(len);
  let o = 0;
  for (const p of parts) {
    out.set(p, o);
    o += p.length;
  }
  return out;
};
export const zeros = (n) => new Uint8Array(n);
const u16be = (n) => Uint8Array.of((n >> 8) & 0xff, n & 0xff);
const u64be = (n) => {
  const b = new DataView(new ArrayBuffer(8));
  b.setBigUint64(0, BigInt(n), false);
  return new Uint8Array(b.buffer);
};
const u16le = (n) => Uint8Array.of(n & 0xff, (n >> 8) & 0xff);
const u64le = (n) => {
  const b = new DataView(new ArrayBuffer(8));
  b.setBigUint64(0, BigInt(n), true);
  return new Uint8Array(b.buffer);
};
const i64le = (n) => {
  const b = new DataView(new ArrayBuffer(8));
  b.setBigInt64(0, BigInt(n), true);
  return new Uint8Array(b.buffer);
};
export const u64leHex = (n) => hexOf(u64le(n));
export const i64leHex = (n) => hexOf(i64le(n));
export const u16beHex = (n) => hexOf(u16be(n));
export const u64beHex = (n) => hexOf(u64be(n));

// ---------------------------------------------------------------- BLAKE3 (Rust: chain_hash, blake3::derive_key)

// Rust primitives.rs:22 chain_hash = blake3::hash(bytes), 32 bytes.
export const chainHash = (bytes) => blake3(bytes);

// Rust: blake3::derive_key(context, ikm) - the derive_key mode of BLAKE3.
// noble: blake3(ikm, { context }) - context is caller-encoded bytes.
export const deriveKey = (context, ikm) => blake3(ikm, { context: utf8(context) });

// The five context strings used by the specs (stage-0 §3c).
export const CONTEXTS = {
  wrap: 'sshub/sync/wrap/v1',
  x25519Seed: 'sshub/sync/seed/x25519/v1',
  vault: 'sshub/sync/vault/v1',
  paperUnwrap: 'sshub/sync/paper/unwrap/v1',
  paperRecover: 'sshub/sync/paper/recover/v1',
};

// ---------------------------------------------------------------- ed25519 / x25519 (Rust: dalek)

export const edPubOfSeed = (seed32) => ed25519.getPublicKey(seed32);
export const edSign = (seed32, msg) => ed25519.sign(msg, seed32);
export const edVerify = (pub32, msg, sig64) => ed25519.verify(sig64, msg, pub32);

export const x25519Pub = (sec32) => x25519.getPublicKey(sec32);
export const x25519DH = (sec32, pub32) => x25519.getSharedSecret(sec32, pub32);

// ---------------------------------------------------------------- ML-KEM-768 (Rust: ml-kem 0.2.3, FIPS 203 final)

export const MLKEM = ml_kem768;
// keygen(seed64): seed = d‖z (Rust: KemCore::generate_deterministic(d, z)).
export const mlkemKeygen = (seed64) => ml_kem768.keygen(seed64);
// encapsulate(ek1184, m32): m = recorded message randomness (Rust: encapsulate_deterministic).
export const mlkemEncapsulate = (ek, m) => ml_kem768.encapsulate(ek, m);
// decapsulate never throws on well-formed input - FIPS 203 implicit rejection.
export const mlkemDecapsulate = (ct, dk) => ml_kem768.decapsulate(ct, dk);

// ---------------------------------------------------------------- DEK seal (Rust: dek_seal_with_nonce, XChaCha20-Poly1305 RFC 8439)

// Rust primitives.rs:172: nonce(24) ‖ ciphertext(len+16); tag APPENDED.
export const dekSeal = (dek32, plaintext, nonce24) =>
  concat(nonce24, xchacha20poly1305(dek32, nonce24).encrypt(plaintext));

export const dekOpen = (dek32, sealed) => {
  if (sealed.length < 24 + 16) throw new Error('too short');
  const nonce = sealed.slice(0, 24);
  return xchacha20poly1305(dek32, nonce).decrypt(sealed.slice(24));
};

// ---------------------------------------------------------------- Argon2id (Rust: argon2::Argon2::hash_password_into)

// Rust primitives.rs:209-235 - bare numbers live here, nowhere else.
export const ARGON2_PROFILES = {
  TEST: { mem_kib: 32 * 1024, iters: 1 },
  SNAPSHOT: { mem_kib: 256 * 1024, iters: 4 },
  EMAIL: { mem_kib: 64 * 1024, iters: 3 },
  MASTER: { mem_kib: 1024 * 1024, iters: 1 },
};

// Rust argon2_key: Params::new(mem_kib, iters, 1, Some(32)), Algorithm::Argon2id,
// Version::V0x13 → raw 32-byte output (no PHC string). hash-wasm argon2id with
// outputType:'binary' is the same primitive.
export async function argon2idDerive(input, salt16, profile) {
  return argon2id({
    password: input,
    salt: salt16,
    parallelism: 1,
    iterations: profile.iters,
    memorySize: profile.mem_kib,
    hashLength: 32,
    outputType: 'binary',
  });
}

// ---------------------------------------------------------------- BIP39 (Rust: bip39 2.2.2, 24 words)

export const bip39Encode = (entropy32) => entropyToMnemonic(entropy32, bip39Wordlist);

export const bip39Decode = (phrase) => {
  // Rust: split_whitespace().collect::<Vec<_>>().join(" ") then parse_normalized.
  const normalized = phrase.trim().split(/\s+/).join(' ');
  const entropy = mnemonicToEntropy(normalized, bip39Wordlist); // validates checksum
  if (entropy.length !== 32) {
    throw new Error(`expected 32-byte entropy, got ${entropy.length}`);
  }
  return entropy;
};

// ---------------------------------------------------------------- EFF-7776 (Rust: stage-0 Task 12, u128 arithmetic)

// words: string[7776]. Both sides use wordlists/eff_large.txt.
const EFF_BASE = 7776n;
const EFF_LIMIT = EFF_BASE ** 6n; // v must be < 7776^6 (fits u128; 10 bytes BE)

export function effEncode(words, bytes10) {
  if (bytes10.length !== 10) throw new Error('need exactly 10 bytes');
  let v = 0n;
  for (const b of bytes10) v = (v << 8n) | BigInt(b);
  if (v >= EFF_LIMIT) throw new Error('entropy >= 7776^6, rejected by eff_generate');
  const out = [];
  let x = v;
  for (let i = 0; i < 6; i++) {
    out.push(words[Number(x % EFF_BASE)]); // least-significant word FIRST
    x /= EFF_BASE;
  }
  return out.join(' ');
}

export function effDecode(words, phrase) {
  const toks = phrase.trim().split(/\s+/);
  if (toks.length !== 6) throw new Error(`need exactly 6 words, got ${toks.length}`);
  // case-insensitive index lookup (Rust: eq_ignore_ascii_case)
  const lower = new Map(words.map((w, i) => [w.toLowerCase(), i]));
  let v = 0n;
  // encode emits the least-significant word first → accumulate tokens in REVERSE
  // (v = v*7776 + idx, starting from the most-significant = last token).
  for (const t of [...toks].reverse()) {
    const idx = lower.get(t.toLowerCase());
    if (idx === undefined) throw new Error(`unknown word: ${t}`);
    v = v * EFF_BASE + BigInt(idx);
  }
  if (v >= EFF_LIMIT) throw new Error('phrase encodes value >= 7776^6');
  const out = new Uint8Array(10);
  let x = v;
  for (let i = 9; i >= 0; i--) {
    out[i] = Number(x & 0xffn); // big-endian re-expansion
    x >>= 8n;
  }
  return out;
}

// ---------------------------------------------------------------- GF(2^8) (Rust: shamir.rs - poly 0x11b, gen 3)

function gfTables() {
  const log = new Uint8Array(256);
  const exp = new Uint8Array(512);
  let x = 1;
  for (let i = 0; i < 255; i++) {
    exp[i] = x;
    log[x] = i;
    const hi = (x & 0x80) !== 0;
    let t = (x << 1) & 0xff; // JS << is 32-bit: mask to u8 like Rust's wrapping x<<1
    if (hi) t ^= 0x1b;
    x = t ^ x;
  }
  for (let i = 255; i < 512; i++) exp[i] = exp[i - 255];
  return { log, exp };
}
const T = gfTables();
const gfMul = (a, b) => (a === 0 || b === 0 ? 0 : T.exp[T.log[a] + T.log[b]]);
const gfInv = (a) => {
  if (a === 0) throw new Error('0 has no inverse in GF(2^8)');
  return T.exp[255 - T.log[a]];
};

// Rust lagrange_at_zero: (0 - xj) == xj in GF(2^8).
function lagrangeAtZero(xs) {
  return xs.map((xi, i) => {
    let num = 1;
    let den = 1;
    xs.forEach((xj, j) => {
      if (i === j) return;
      num = gfMul(num, xj);
      den = gfMul(den, xi ^ xj);
    });
    return gfMul(num, gfInv(den));
  });
}

// Rust split_at_indices_with_rng: coeffs[0] = secret, coeffs[1..k) drawn from the
// RNG as fill_bytes(32) each - sequential stream consumption; Horner top-down.
export function splitAtIndicesSeeded(secret32, k, indices, rngStream) {
  if (k < 2) throw new Error('threshold below 2 is not secret sharing');
  if (indices.some((x) => x === 0)) throw new Error('x=0 is the secret itself');
  const coeffs = [Uint8Array.from(secret32)];
  for (let i = 1; i < k; i++) coeffs.push(rngStream.slice((i - 1) * 32, i * 32));
  return indices.map((x) => {
    const bytes = new Uint8Array(32);
    for (let b = 0; b < 32; b++) {
      let acc = 0;
      for (let d = k - 1; d >= 0; d--) acc = gfMul(acc, x) ^ coeffs[d][b];
      bytes[b] = acc;
    }
    return { index: x, bytes };
  });
}

// Rust shamir_combine: Lagrange at zero over the given subset.
export function shamirCombine(shares) {
  if (shares.length < 2) throw new Error('need at least 2 shares');
  if (shares.some((s) => s.bytes.length !== 32)) throw new Error('malformed share: expected 32 bytes');
  if (shares.some((s) => s.index === 0)) throw new Error('share index 0 is the secret itself');
  const xs = shares.map((s) => s.index);
  if (new Set(xs).size !== xs.length) throw new Error('duplicate share index');
  const l = lagrangeAtZero(xs);
  const out = new Uint8Array(32);
  shares.forEach((s, i) => {
    for (let b = 0; b < 32; b++) out[b] ^= gfMul(l[i], s.bytes[b]);
  });
  return out;
}

// Rust reshare_contribute_with_rng: re-split MY share bytes under new indices.
export function reshareContribute(my, newIndices, k, rngStream) {
  const sub = splitAtIndicesSeeded(Uint8Array.from(my.bytes), k, newIndices, rngStream);
  return { from: my.index, sub_shares: sub.map((s) => [s.index, s.bytes]) };
}

// Rust reshare_combine: my new share = Σ l[i] * contribution[my_index].
export function reshareCombine(myIndex, contributions) {
  if (contributions.length < 2) throw new Error('resharing needs at least k=2 contributors');
  const xs = contributions.map((c) => c.from);
  if (new Set(xs).size !== xs.length) throw new Error('duplicate contributor index');
  const l = lagrangeAtZero(xs);
  const bytes = new Uint8Array(32);
  contributions.forEach((c, i) => {
    const hit = c.sub_shares.find(([x]) => x === myIndex);
    if (!hit) throw new Error(`contribution from ${c.from} carries no sub-share for ${myIndex}`);
    const sub = hit[1];
    if (sub.length !== 32) throw new Error(`malformed sub-share from ${c.from}`);
    for (let b = 0; b < 32; b++) bytes[b] ^= gfMul(l[i], sub[b]);
  });
  return { index: myIndex, bytes };
}

// ---------------------------------------------------------------- rand_chacha ChaCha20Rng reproduction

// Rust rand_chacha 0.3.1 ChaCha20Rng::from_seed(seed): key = seed, 64-bit block
// counter starting at 0, stream/nonce = 0. Sequential fill_bytes consumes the
// plain keystream (djb layout = noble chacha20orig, 8-byte zero nonce). Verified
// byte-for-byte against the real crate in /tmp/jsref (see notes).
export const chachaRngStream = (seed32, length) =>
  chacha20orig(seed32, zeros(8), zeros(length));

// ---------------------------------------------------------------- crypto_secretbox over ChaCha20Legacy (Rust: ChaChaBox internals)

// Noble's hchacha works on u32 words; pack/unpack little-endian explicitly.
const packWords = (bytes) => {
  const w = new Uint32Array(bytes.length >> 2);
  for (let i = 0; i < w.length; i++) {
    w[i] = bytes[4 * i] | (bytes[4 * i + 1] << 8) | (bytes[4 * i + 2] << 16) | (bytes[4 * i + 3] << 24);
  }
  return w;
};
const unpackWords = (w) => {
  const b = new Uint8Array(4 * w.length);
  for (let i = 0; i < w.length; i++) {
    b[4 * i] = w[i] & 0xff;
    b[4 * i + 1] = (w[i] >>> 8) & 0xff;
    b[4 * i + 2] = (w[i] >>> 16) & 0xff;
    b[4 * i + 3] = (w[i] >>> 24) & 0xff;
  }
  return b;
};
const SIGMA = Uint32Array.of(0x61707865, 0x3320646e, 0x79622d32, 0x6b206574);

// HChaCha20(key32, input16) → 32-byte subkey (djb construction, no feed-forward).
const hchachaBytes = (key32, input16) => {
  const out = new Uint32Array(8);
  hchacha(SIGMA, packWords(key32), packWords(input16), out);
  return unpackWords(out);
};

// Rust crypto_secretbox 0.1.1 init_cipher_and_mac + encrypt_in_place_detached:
//   subkey   = HChaCha20(key, nonce[0..16])
//   cipher   = ChaCha20Legacy(subkey, nonce[16..24])
//   mac_key  = first 32 keystream bytes (same cipher instance)
//   ct       = plaintext XOR keystream[32..]
//   tag      = Poly1305(mac_key, ct)  (NaCl unpadded MAC, no AAD)
//   output   = tag PREPENDED ‖ ct
export function secretboxSeal(boxKey32, nonce24, plaintext) {
  const subkey = hchachaBytes(boxKey32, nonce24.slice(0, 16));
  const stream = chacha20orig(subkey, nonce24.slice(16, 24), zeros(32 + plaintext.length));
  const macKey = stream.slice(0, 32);
  const ct = new Uint8Array(plaintext.length);
  for (let i = 0; i < plaintext.length; i++) ct[i] = plaintext[i] ^ stream[32 + i];
  const tag = poly1305(ct, macKey);
  return concat(tag, ct);
}

export function secretboxOpen(boxKey32, nonce24, tagCt) {
  if (tagCt.length < 16) throw new Error('ciphertext too short');
  const subkey = hchachaBytes(boxKey32, nonce24.slice(0, 16));
  const len = tagCt.length - 16;
  const stream = chacha20orig(subkey, nonce24.slice(16, 24), zeros(32 + len));
  const macKey = stream.slice(0, 32);
  const ct = tagCt.slice(16);
  const expected = poly1305(ct, macKey);
  const tag = tagCt.slice(0, 16);
  if (!bytesEqual(tag, expected)) throw new Error('secretbox tag mismatch');
  const pt = new Uint8Array(len);
  for (let i = 0; i < len; i++) pt[i] = ct[i] ^ stream[32 + i];
  return pt;
}

// Rust ChaChaBox::new(pub, sec): key = HChaCha20(x25519(ss), [0u8;16]).
// Sealed box format (rig seal_to_parts): eph_pub(32) ‖ nonce(24) ‖ tag ‖ ct.
export function chachaBoxSeal(recipientPub32, plaintext, ephSec32, nonce24) {
  const ss = x25519DH(ephSec32, recipientPub32);
  const boxKey = hchachaBytes(ss, zeros(16));
  const ephPub = x25519Pub(ephSec32);
  return concat(ephPub, nonce24, secretboxSeal(boxKey, nonce24, plaintext));
}

export function chachaBoxOpen(recipientSec32, sealed) {
  if (sealed.length < 32 + 24 + 16) throw new Error('sealed box too short');
  const ephPub = sealed.slice(0, 32);
  const nonce = sealed.slice(32, 56);
  const body = sealed.slice(56);
  const ss = x25519DH(recipientSec32, ephPub);
  const boxKey = hchachaBytes(ss, zeros(16));
  return secretboxOpen(boxKey, nonce, body);
}

// ---------------------------------------------------------------- canonical Op serialisation (Rust: stage-1 serde_json)

// serde_json-compatible canonical text: compact, insertion-ordered keys, raw
// UTF-8 (no \uXXXX for non-ASCII), integers verbatim (BigInt emitted as digits).
export function canonicalJSON(value) {
  if (value === null) return 'null';
  const t = typeof value;
  if (t === 'boolean') return value ? 'true' : 'false';
  if (t === 'string') return JSON.stringify(value);
  if (t === 'bigint') return value.toString();
  if (t === 'number') {
    if (!Number.isInteger(value)) throw new Error(`non-integer in canonical JSON: ${value}`);
    if (!Number.isSafeInteger(value)) throw new Error(`unsafe integer ${value} - use BigInt`);
    return String(value);
  }
  if (Array.isArray(value)) return `[${value.map(canonicalJSON).join(',')}]`;
  if (t === 'object') {
    return `{${Object.entries(value).map(([k, v]) => `${JSON.stringify(k)}:${canonicalJSON(v)}`).join(',')}}`;
  }
  throw new Error(`unsupported type in canonical JSON: ${t}`);
}

// JSON.parse loses digits above 2^53 (u64::MAX!). Vectors.json is re-read through
// this parser so big integers survive as BigInt.
export function parseJSONExact(text) {
  let i = 0;
  const ws = () => {
    while (i < text.length && /[\s]/.test(text[i])) i++;
  };
  function value() {
    ws();
    const c = text[i];
    if (c === '{') return object();
    if (c === '[') return array();
    if (c === '"') return string();
    if (c === 't') return expect('true', true);
    if (c === 'f') return expect('false', false);
    if (c === 'n') return expect('null', null);
    if (c === '-' || (c >= '0' && c <= '9')) return number();
    throw new Error(`parseJSONExact: unexpected '${c}' at ${i}`);
  }
  function expect(lit, v) {
    if (text.startsWith(lit, i)) {
      i += lit.length;
      return v;
    }
    throw new Error(`parseJSONExact: bad literal at ${i}`);
  }
  function number() {
    const start = i;
    if (text[i] === '-') i++;
    while (i < text.length && text[i] >= '0' && text[i] <= '9') i++;
    const isInt = i >= text.length || !/[.eE]/.test(text[i]);
    if (!isInt) {
      while (i < text.length && /[0-9.eE+-]/.test(text[i])) i++;
      return Number(text.slice(start, i)); // floats never appear in canonical ops
    }
    const tok = text.slice(start, i);
    const n = Number(tok);
    if (Number.isSafeInteger(n) && String(n) === tok.replace(/^(-?)0+(?=\d)/, '$1')) {
      if (tok === '-0') throw new Error('parseJSONExact: -0 unsupported');
      return n;
    }
    return BigInt(tok); // u64::MAX & friends survive exactly
  }
  function string() {
    let out = '';
    i++; // opening quote
    while (true) {
      const c = text[i];
      if (c === '"') {
        i++;
        return out;
      }
      if (c === '\\') {
        const e = text[i + 1];
        if (e === 'u') {
          out += String.fromCharCode(parseInt(text.slice(i + 2, i + 6), 16));
          i += 6;
        } else {
          out += { n: '\n', r: '\r', t: '\t', b: '\b', f: '\f', '"': '"', '\\': '\\', '/': '/' }[e];
          i += 2;
        }
        continue;
      }
      out += c;
      i++;
    }
  }
  function array() {
    i++;
    const out = [];
    ws();
    if (text[i] === ']') {
      i++;
      return out;
    }
    while (true) {
      out.push(value());
      ws();
      if (text[i] === ',') {
        i++;
        continue;
      }
      if (text[i] === ']') {
        i++;
        return out;
      }
      throw new Error(`parseJSONExact: expected , or ] at ${i}`);
    }
  }
  function object() {
    i++;
    const out = {};
    ws();
    if (text[i] === '}') {
      i++;
      return out;
    }
    while (true) {
      ws();
      const k = string();
      ws();
      if (text[i] !== ':') throw new Error(`parseJSONExact: expected : at ${i}`);
      i++;
      out[k] = value();
      ws();
      if (text[i] === ',') {
        i++;
        continue;
      }
      if (text[i] === '}') {
        i++;
        return out;
      }
      throw new Error(`parseJSONExact: expected , or } at ${i}`);
    }
  }
  const v = value();
  ws();
  if (i !== text.length) throw new Error(`parseJSONExact: trailing data at ${i}`);
  return v;
}

export const canonicalBytes = (value) => utf8(canonicalJSON(value));

// Rust OpCanonical: fixed field order, prev_hash as lowercase hex, signature EXCLUDED.
export function opCanonical(op) {
  return {
    device_id: op.device_id,
    seq: op.seq,
    lamport: op.lamport,
    prev_hash: hexOf(op.prev_hash).toLowerCase(),
    timestamp: op.timestamp,
    epoch: op.epoch,
    body: op.body,
  };
}

export const opCanonicalBytes = (op) => canonicalBytes(opCanonical(op));
export const opHash = (op) => chainHash(opCanonicalBytes(op));
export const opVerify = (op, signerPub32) => edVerify(signerPub32, opCanonicalBytes(op), op.signature);

// Rust device_id_of: lowercase hex of the first 8 bytes of the ed25519 PUBLIC key.
export const deviceIdOf = (ed25519Pub32) => hexOf(ed25519Pub32.slice(0, 8)).toLowerCase();

// ---------------------------------------------------------------- binding_hash + approval (Rust: stage-3 Challenge)

export const DOMAIN_TAG_CHALLENGE = utf8('sshub/sync/challenge/v1');
export const PROTO_VERSION = 1;

// Rust Challenge::binding_hash: tag ‖ proto u16 LE ‖ op.canonical_bytes ‖
// nonce(32) ‖ expiry i64 LE ‖ eph_pub(32) ‖ request_id u64 LE → chain_hash.
export function bindingHashPreimage(op, nonce32, expiry, eph32, requestId) {
  return concat(
    DOMAIN_TAG_CHALLENGE,
    u16le(PROTO_VERSION),
    opCanonicalBytes(op),
    nonce32,
    i64le(expiry),
    eph32,
    u64le(requestId),
  );
}

export const bindingHash = (...args) => chainHash(bindingHashPreimage(...args));

// ---------------------------------------------------------------- hybrid_wrap (Rust: hybrid_wrap_seeded)

export const WRAP_MAGIC = utf8('SSHW');
export const WRAP_KDF_CONTEXT = CONTEXTS.wrap;

// Blob: SSHW(4) ‖ sig_u8=1 ‖ wrap_u8=1 ‖ eph_pub(32) ‖ kem_ct_len u16 BE ‖
// kem_ct ‖ nonce(24) ‖ ct(pt+16).
// ikm = x_ss(32) ‖ kem_ss(32) ‖ eph_pub(32) ‖ recipient_x25519_pub(32).
export function hybridWrap(recipient, plaintext, ephSec32, nonce24, m32) {
  const ephPub = x25519Pub(ephSec32);
  const x_ss = x25519DH(ephSec32, recipient.x25519_pub);
  const kem = ml_kem768.encapsulate(recipient.mlkem_pub, m32);
  const ikm = concat(x_ss, kem.sharedSecret, ephPub, recipient.x25519_pub);
  const key = deriveKey(WRAP_KDF_CONTEXT, ikm);
  const ct = xchacha20poly1305(key, nonce24).encrypt(plaintext);
  return concat(WRAP_MAGIC, Uint8Array.of(1, 1), ephPub, u16be(kem.cipherText.length), kem.cipherText, nonce24, ct);
}

export function hybridUnwrap(x25519Sec32, mlkemDk, blob) {
  const magic = blob.slice(0, 4);
  if (hexOf(magic) !== '53534857') throw new Error('not a wrap blob');
  if (blob[4] !== 1 || blob[5] !== 1) throw new Error('unknown algorithm generation');
  let p = 6;
  const ephPub = blob.slice(p, p + 32);
  p += 32;
  const kemLen = (blob[p] << 8) | blob[p + 1];
  p += 2;
  const kemCt = blob.slice(p, p + kemLen);
  p += kemLen;
  const nonce = blob.slice(p, p + 24);
  p += 24;
  const ct = blob.slice(p);
  const x_ss = x25519DH(x25519Sec32, ephPub);
  const kem_ss = ml_kem768.decapsulate(kemCt, mlkemDk);
  const ikm = concat(x_ss, kem_ss, ephPub, x25519Pub(x25519Sec32));
  const key = deriveKey(WRAP_KDF_CONTEXT, ikm);
  return xchacha20poly1305(key, nonce).decrypt(ct);
}

// ---------------------------------------------------------------- share response (Rust: stage-3 wire, SWR1 frame)

const WIRE_MAGIC = utf8('SWR1');

// Frame: SWR1(4) ‖ version u16 BE(=1) ‖ kind u8(=2) ‖ body.
// Body: request_id u64 BE ‖ ciphertext_len u64 BE ‖ ciphertext.
// ciphertext = ChaChaBox sealed to the challenge eph key (tag‖ct inside).
export function shareResponse(challengeEphPub32, plaintext, sealerEphSec32, nonce24, requestId) {
  const sealed = chachaBoxSeal(challengeEphPub32, plaintext, sealerEphSec32, nonce24);
  const body = concat(u64be(requestId), u64be(sealed.length), sealed);
  return concat(WIRE_MAGIC, u16be(1), Uint8Array.of(2), body);
}

// ---------------------------------------------------------------- signed approval (stage-0 §26-28 + stage-3 §457-472)

// sig_over = "preimage": Ed25519 over the binding_hash PREIMAGE bytes.
export const approvalPreimage = bindingHashPreimage;
export const approvalSign = (holderSeed32, op, nonce32, expiry, eph32, requestId) =>
  edSign(holderSeed32, approvalPreimage(op, nonce32, expiry, eph32, requestId));
export const approvalVerify = (holderPub32, op, nonce32, expiry, eph32, requestId, sig64) =>
  edVerify(holderPub32, approvalPreimage(op, nonce32, expiry, eph32, requestId), sig64);
