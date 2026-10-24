// PoC A - JS side of the Rust ↔ JS vector rig. Run: node --test test/
//
// Three layers:
//   1. Known-vector self-tests (BLAKE3 KAT, Trezor BIP39, RFC 8439 via node's
//      native chacha20-poly1305, round-trips).
//   2. Replay of test/rust-ref.txt - real Rust outputs produced by the scratch
//      reference crate (/tmp/jsref, crates identical to the rig's Cargo.lock:
//      blake3 1.8.7, dalek 2.x, crypto_box 0.9.1, chacha20poly1305 0.10.1,
//      ml-kem 0.2.3, argon2 0.5.3, rand_chacha 0.3.1, serde_json 1). This is a
//      stand-in until vectors.json lands; vectors.json supersedes it.
//   3. Full replay of ../vectors.json when present (A1: zero mismatches).

import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';

import * as lib from './lib.mjs';
import { FIX } from './fixtures.mjs';
import { chacha20poly1305 } from '../vendor/noble-ciphers-2.4.0/chacha.js';

const ROOT = fileURLToPath(new URL('..', import.meta.url));

// ---------------------------------------------------------------- rust-ref.txt

const rustRef = new Map();
for (const line of fs.readFileSync(`${ROOT}/test/rust-ref.txt`, 'utf8').split('\n')) {
  const eq = line.indexOf('=');
  if (eq > 0) rustRef.set(line.slice(0, eq), line.slice(eq + 1).trim());
}
const ref = (name) => {
  const v = rustRef.get(name);
  if (!v) throw new Error(`rust-ref.txt missing ${name}`);
  return v;
};

// ---------------------------------------------------------------- wordlist

const EFF_WORDS = fs
  .readFileSync(`${ROOT}/wordlists/eff_large.txt`, 'utf8')
  .split('\n')
  .map((s) => s.trim())
  .filter(Boolean);

// ---------------------------------------------------------------- helpers

const bytes = lib.bytesOf;
const hx = lib.hexOf;
const eqHex = (a, b, what) => {
  if (a !== b) {
    throw new Error(`MISMATCH ${what}\n  js  = ${a}\n  rust= ${b}`);
  }
};

// ================================================================ self-tests

test('blake3 KAT (empty input) - official known-answer vector', () => {
  eqHex(hx(lib.chainHash(new Uint8Array(0))), 'af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262', 'blake3 empty');
});

test('chain_hash + derive_key vs rust blake3 1.8.7', () => {
  eqHex(hx(lib.chainHash(new Uint8Array(0))), ref('blake3_empty'), 'chain_hash empty');
  eqHex(hx(lib.chainHash(lib.utf8('abc'))), ref('blake3_abc'), 'chain_hash abc');
  const ikm = new Uint8Array(32).fill(1);
  for (const [rname, ctx] of [
    ['wrap', lib.CONTEXTS.wrap],
    ['x25519seed', lib.CONTEXTS.x25519Seed],
    ['vault', lib.CONTEXTS.vault],
    ['paper_unwrap', lib.CONTEXTS.paperUnwrap],
    ['paper_recover', lib.CONTEXTS.paperRecover],
  ]) {
    eqHex(hx(lib.deriveKey(ctx, ikm)), ref(`derive_key_${rname}`), `derive_key ${ctx}`);
  }
});

test('rand_chacha ChaCha20Rng byte-stream reproduction (rand_chacha 0.3.1)', () => {
  for (const [name, byte] of [['zero', 0], ['01', 1], ['11', 0x11]]) {
    const seed = new Uint8Array(32).fill(byte);
    const stream = lib.chachaRngStream(seed, 256);
    eqHex(hx(stream), ref(`rng_${name}`), `chacha20rng fill_bytes seed=0x${byte.toString(16).padStart(2, '0')}*32`);
    // the u32-draw path must agree with the byte path (same keystream)
    eqHex(hx(stream.slice(0, 16)), ref(`rng_${name}_u32`), 'chacha20rng next_u32 words');
  }
});

test('ed25519 + x25519 vs rust dalek 2.x', () => {
  const seed = new Uint8Array(32).fill(0x11);
  eqHex(hx(lib.edPubOfSeed(seed)), ref('ed25519_pub_11'), 'ed25519 pub');
  const sig = lib.edSign(seed, lib.utf8('canonical test message'));
  eqHex(hx(sig), ref('ed25519_sig_11'), 'ed25519 sig');
  assert.equal(lib.edVerify(hx(sig) === ref('ed25519_sig_11') ? lib.edPubOfSeed(seed) : seed, lib.utf8('canonical test message'), sig), true, 'ed25519 self-verify');
  const a = new Uint8Array(32).fill(0x22);
  const bSec = new Uint8Array(32).fill(0x55);
  eqHex(hx(lib.x25519Pub(a)), ref('x25519_pub_22'), 'x25519 pub a');
  eqHex(hx(lib.x25519Pub(bSec)), ref('x25519_pub_55'), 'x25519 pub b');
  eqHex(hx(lib.x25519DH(a, lib.x25519Pub(bSec))), ref('x25519_dh_22_55'), 'x25519 dh');
});

test('dek_seal (RFC 8439 XChaCha20-Poly1305) vs rust chacha20poly1305 0.10.1', () => {
  const dek = new Uint8Array(32).fill(1);
  const nonce = new Uint8Array(24).fill(2);
  const plain = lib.utf8('hello dek seal plaintext');
  eqHex(hx(lib.dekSeal(dek, plain, nonce)), ref('dek_seal'), 'dek_seal frame');
  assert.deepEqual(lib.dekOpen(dek, lib.dekSeal(dek, plain, nonce)), plain, 'dek_open round-trip');
  assert.throws(() => lib.dekOpen(dek, new Uint8Array(39)), 'len < 40 must be rejected');
});

test('RFC 8439 ChaCha20-Poly1305 core vs node native OpenSSL', () => {
  // 12-byte-nonce AEAD: cross-check noble against node's native implementation.
  const key = new Uint8Array(32).fill(0x01);
  const iv = new Uint8Array(12).fill(0x03);
  const aad = new Uint8Array(8).fill(0x04);
  const plain = lib.utf8("Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.");
  const nobleCt = chacha20poly1305(key, iv).encrypt(plain);
  const nobleCtAad = chacha20poly1305(key, iv, aad).encrypt(plain);
  const c = crypto.createCipheriv('chacha20-poly1305', Buffer.from(key), Buffer.from(iv), { authTagLength: 16 });
  c.setAAD(Buffer.from(aad));
  const nativeCt = Buffer.concat([c.update(Buffer.from(plain)), c.final(), c.getAuthTag()]);
  eqHex(hx(nobleCtAad), hx(nativeCt), 'chacha20-poly1305 with AAD');
  // no-AAD path agrees with the no-AAD noble call
  const c2 = crypto.createCipheriv('chacha20-poly1305', Buffer.from(key), Buffer.from(iv), { authTagLength: 16 });
  const nativeCt2 = Buffer.concat([c2.update(Buffer.from(plain)), c2.final(), c2.getAuthTag()]);
  eqHex(hx(nobleCt), hx(nativeCt2), 'chacha20-poly1305 no AAD');
});
test('ChaChaBox sealed box vs rust crypto_box 0.9.1 (crypto_secretbox over ChaCha20Legacy)', () => {
  const recipPub = lib.x25519Pub(new Uint8Array(32).fill(0x55));
  const ephSec = new Uint8Array(32).fill(0xcc);
  const nonce = new Uint8Array(24).fill(0xdd);
  const plain = new Uint8Array(32).fill(0x77);
  const sealed = lib.chachaBoxSeal(recipPub, plain, ephSec, nonce);
  eqHex(hx(sealed), ref('chachabox_seal'), 'chachabox sealed frame');
  assert.deepEqual(lib.chachaBoxOpen(new Uint8Array(32).fill(0x55), sealed), plain, 'chachabox open round-trip');
  // wrong key must fail the Poly1305 tag
  assert.throws(() => lib.chachaBoxOpen(new Uint8Array(32).fill(0x88), sealed), 'wrong-key open must fail');
});

test('ML-KEM-768 keygen/encapsulate/decapsulate vs rust ml-kem 0.2.3 (FIPS 203 final)', () => {
  const seed = new Uint8Array(64);
  seed.set(new Uint8Array(32).fill(0x66), 0);
  seed.set(new Uint8Array(32).fill(0x77), 32);
  const { publicKey: ek, secretKey: dk } = lib.mlkemKeygen(seed);
  eqHex(hx(ek), ref('mlkem_ek_6677'), 'ml-kem ek (1184B)');
  eqHex(hx(dk), ref('mlkem_dk_6677'), 'ml-kem dk (2400B)');
  assert.equal(ek.length, 1184, 'ek size');
  assert.equal(dk.length, 2400, 'dk size');

  const m = new Uint8Array(32).fill(0x1f);
  const enc = lib.mlkemEncapsulate(ek, m);
  assert.equal(enc.cipherText.length, 1088, 'ct size');
  assert.equal(enc.sharedSecret.length, 32, 'ss size');
  eqHex(hx(enc.cipherText), ref('mlkem_encaps_m1f_ct'), 'ml-kem ct for recorded m');
  eqHex(hx(enc.sharedSecret), ref('mlkem_encaps_m1f_ss'), 'ml-kem ss for recorded m');
  eqHex(hx(lib.mlkemDecapsulate(enc.cipherText, dk)), ref('mlkem_decaps_m1f'), 'ml-kem decapsulate');

  // hybrid_wrap's encapsulate(rng) path: m = first 32 bytes of the ChaCha20Rng
  // stream (verified true on the Rust side: mlkem_encaps_probe_*_eq=true).
  const rngM = lib.chachaRngStream(new Uint8Array(32).fill(0x99), 32);
  const encR = lib.mlkemEncapsulate(ek, rngM);
  eqHex(hx(encR.cipherText), ref('mlkem_encaps_rng99_ct'), 'ml-kem ct via rng-derived m');
  eqHex(hx(encR.sharedSecret), ref('mlkem_encaps_rng99_ss'), 'ml-kem ss via rng-derived m');
});

test('argon2id vs rust argon2 0.5.3 (raw hash_password_into, lanes=1)', async () => {
  const salt = new Uint8Array(16).fill(0x0a);
  for (const [name, profile] of [
    ['TEST', lib.ARGON2_PROFILES.TEST],
    ['SNAPSHOT', lib.ARGON2_PROFILES.SNAPSHOT],
    ['EMAIL', lib.ARGON2_PROFILES.EMAIL],
  ]) {
    const t0 = performance.now();
    const key = await lib.argon2idDerive('test-input', salt, profile);
    eqHex(hx(key), ref(`argon2_${name}`), `argon2id ${name} (${profile.mem_kib} KiB, t=${profile.iters})`);
    console.log(`  argon2id ${name}: ${(performance.now() - t0).toFixed(1)} ms`);
  }
});

test('argon2id MASTER profile (1 GiB) runs in node', async () => {
  const salt = new Uint8Array(16).fill(0x0a);
  const t0 = performance.now();
  const key = await lib.argon2idDerive('test-input', salt, lib.ARGON2_PROFILES.MASTER);
  const ms = performance.now() - t0;
  console.log(`  argon2id MASTER: ${ms.toFixed(0)} ms`);
  assert.equal(key.length, 32, 'MASTER output size');
});

test('bip39 24-word Trezor vectors + checksum/wordlist rejection', () => {
  const zeros = new Uint8Array(32);
  const ones = new Uint8Array(32).fill(0xff);
  const phrase0 = lib.bip39Encode(zeros);
  const phraseF = lib.bip39Encode(ones);
  eqHex(phrase0, `${'abandon '.repeat(23)}art`, '32x00 mnemonic');
  eqHex(phraseF, `${'zoo '.repeat(23)}vote`, '32xff mnemonic');
  assert.deepEqual(lib.bip39Decode(phrase0), zeros, 'decode(encode) round-trip zeros');
  assert.deepEqual(lib.bip39Decode(phraseF), ones, 'decode(encode) round-trip ff');
  // unknown word named
  assert.throws(() => lib.bip39Decode(phrase0.replace('abandon', 'notaword')), 'unknown word must throw');
  // checksum: swap two words with valid dictionary words
  assert.throws(() => lib.bip39Decode(phrase0.replace(/art$/, 'zoo')), 'checksum mismatch must throw');
});

test('EFF-7776 encode/decode (u128 BE, least-significant word first)', () => {
  assert.equal(EFF_WORDS.length, 7776, 'wordlist size');
  assert.equal(EFF_WORDS[0], 'abacus', 'first word');
  assert.equal(EFF_WORDS[7775], 'zoom', 'last word');
  const inRange = new Uint8Array(10).fill(1); // 0x0101..01 < 7776^6
  const phrase = lib.effEncode(EFF_WORDS, inRange);
  assert.equal(phrase.split(' ').length, 6, '6 words');
  assert.deepEqual(lib.effDecode(EFF_WORDS, phrase), inRange, 'decode(encode) round-trip');
  // case-insensitive decode
  assert.deepEqual(lib.effDecode(EFF_WORDS, phrase.toUpperCase()), inRange, 'case-insensitive decode');
  // out-of-range rejection (rust eff_generate rejection-samples; encode must reject)
  assert.throws(() => lib.effEncode(EFF_WORDS, new Uint8Array(10).fill(0xff)), 'v >= 7776^6 must be rejected');
  // 6 words decode is strict
  assert.throws(() => lib.effDecode(EFF_WORDS, phrase + ' abacus'), '7 words must throw');
});

test('canonical Op serialisation vs serde_json (incl. A5 knobs)', () => {
  // parse the REAL serde_json bytes, re-serialise with my canonical rules -
  // must be byte-identical. Covers: fixed field order, prev_hash lowercase hex,
  // hash fields as number arrays, Option None→null, empty Vecs, u64::MAX,
  // Cyrillic + emoji raw UTF-8.
  for (const name of [
    'op_canonical_host_upsert',
    'op_canonical_cred_update_empty',
    'op_canonical_share_release',
    'op_canonical_reenroll_cancel_none',
    'op_canonical_reenroll_cancel_some',
    'op_canonical_head_atttest_u64max',
  ]) {
    const rustHex = ref(name);
    const text = Buffer.from(bytes(rustHex)).toString('utf8');
    const value = lib.parseJSONExact(text);
    eqHex(hx(lib.canonicalBytes(value)), rustHex, `canonical re-serialisation ${name}`);
  }
  const u64max = lib.parseJSONExact('{"seq":18446744073709551615}');
  assert.equal(u64max.seq, 18446744073709551615n, 'u64::MAX survives as BigInt');
  eqHex(lib.canonicalJSON(u64max), '{"seq":18446744073709551615}', 'u64::MAX verbatim digits');
});

test('op canonical structure (field order + prev_hash hex rule)', () => {
  const op = {
    device_id: 'aaaaaaaaaaaaaaaa',
    seq: 1,
    lamport: 1,
    prev_hash: new Uint8Array(32),
    timestamp: 1893452400,
    epoch: 0,
    body: { ShareRelease: { host_id: 7, request_id: 42, requester: 'aaaaaaaaaaaaaaaa' } },
  };
  eqHex(hx(lib.opCanonicalBytes(op)), ref('op_canonical_share_release'), 'opCanonicalBytes');
  assert.equal(lib.deviceIdOf(lib.edPubOfSeed(new Uint8Array(32).fill(0x11))), hx(lib.edPubOfSeed(new Uint8Array(32).fill(0x11)).slice(0, 8)), 'device_id = first 8 pub bytes hex');
});

test('shamir split/combine/reshare round-trips over GF(2^8)', () => {
  const secret = new Uint8Array(32).fill(0x77);
  const rng = lib.chachaRngStream(new Uint8Array(32).fill(0x11), 32 * 20);
  const n = 12;
  const shares = lib.splitAtIndicesSeeded(secret, 3, Array.from({ length: n }, (_, i) => i + 1), rng);
  // any 3 of 12 reconstruct
  for (const [i, j, k] of [[1, 5, 9], [2, 4, 12], [3, 7, 11]]) {
    const subset = [shares[i - 1], shares[j - 1], shares[k - 1]];
    assert.deepEqual(lib.shamirCombine(subset), secret, `combine shares ${i},${j},${k}`);
  }
  // fewer than k shares yield an unrelated value (AEAD layer catches it)
  const wrong = lib.shamirCombine([shares[0], shares[1]]);
  assert.notEqual(hx(wrong), hx(secret), 'k-1 shares must not reconstruct');
  assert.throws(() => lib.shamirCombine([shares[0], shares[0]]), 'duplicate index must throw');

  // reshare: k=3 holders (x=2, x=4, x=6) re-split their shares under new
  // indices 10..12 (independent random sub-coefficients). With ALL k
  // contributors the assembled shares form a fresh k-of-n sharing of the SAME
  // secret: combine(new10,new11,new12) == secret. Fewer than k contributors
  // yields a well-formed but UNRELATED polynomial (same AEAD-layer rejection
  // semantics as a wrong-subset combine - rust reshare_combine only errors
  // below 2 contributors).
  const c2 = lib.reshareContribute(shares[1], [10, 11, 12], 3, rng.slice(32 * 12, 32 * 14));
  const c4 = lib.reshareContribute(shares[3], [10, 11, 12], 3, rng.slice(32 * 14, 32 * 16));
  const c6 = lib.reshareContribute(shares[5], [10, 11, 12], 3, rng.slice(32 * 16, 32 * 18));
  const newShares = [10, 11, 12].map((idx) => lib.reshareCombine(idx, [c2, c4, c6]));
  assert.equal(newShares[1].index, 11, 'reshared share index');
  assert.deepEqual(lib.shamirCombine(newShares), secret, 'reshared set re-shares the same secret');
  // 2-of-3 contributors: well-formed frame, unrelated secret - must NOT equal
  const twoOnly = [10, 11, 12].map((idx) => lib.reshareCombine(idx, [c2, c4]));
  assert.notEqual(hx(lib.shamirCombine(twoOnly)), hx(secret), 'partial reshare must not leak the secret');
});

test('hybrid_wrap blob reproduction vs rust (fixtures + rng-derived m)', () => {
  const recipient = {
    x25519_pub: lib.x25519Pub(new Uint8Array(32).fill(0x55)),
    mlkem_pub: bytes(ref('mlkem_ek_6677')),
  };
  const m = lib.chachaRngStream(new Uint8Array(32).fill(0x99), 32); // encapsulate(rng) m
  const blob = lib.hybridWrap(
    recipient,
    new Uint8Array(64).fill(0xee),
    new Uint8Array(32).fill(0x99),
    new Uint8Array(24).fill(0xbb),
    m,
  );
  eqHex(hx(blob), ref('hybrid_wrap'), 'hybrid_wrap blob (1232 bytes)');
  assert.deepEqual(lib.hybridUnwrap(new Uint8Array(32).fill(0x55), bytes(ref('mlkem_dk_6677')), blob), new Uint8Array(64).fill(0xee), 'hybridUnwrap round-trip');
});

test('binding_hash + signed approval self-consistency', () => {
  const op = {
    device_id: lib.deviceIdOf(lib.edPubOfSeed(FIX.requesterEdSeed)),
    seq: FIX.opSeq,
    lamport: FIX.opLamport,
    prev_hash: FIX.opPrevHash,
    timestamp: FIX.opTimestamp,
    epoch: FIX.opEpoch,
    body: {
      ShareRelease: {
        host_id: FIX.opHostId,
        request_id: FIX.requestId,
        requester: lib.deviceIdOf(lib.edPubOfSeed(FIX.requesterEdSeed)),
      },
    },
    signature: new Uint8Array(64),
  };
  op.signature = lib.edSign(FIX.requesterEdSeed, lib.opCanonicalBytes(op));
  assert.equal(lib.opVerify(op, lib.edPubOfSeed(FIX.requesterEdSeed)), true, 'op signature verifies');

  const preimage = lib.bindingHashPreimage(op, FIX.challengeNonce, FIX.challengeExpiry, lib.x25519Pub(FIX.challengeEphSec), FIX.requestId);
  eqHex(hx(lib.bindingHash(op, FIX.challengeNonce, FIX.challengeExpiry, lib.x25519Pub(FIX.challengeEphSec), FIX.requestId)), hx(lib.chainHash(preimage)), 'binding_hash = chain_hash(preimage)');
  // framing: tag ‖ 0x0100 (u16 LE 1) at the head
  eqHex(hx(preimage.slice(0, 23 + 2)), '73736875622f73796e632f6368616c6c656e67652f76310100', 'preimage head = tag ‖ proto u16 LE');
  const sig = lib.approvalSign(FIX.holderEdSeed, op, FIX.challengeNonce, FIX.challengeExpiry, lib.x25519Pub(FIX.challengeEphSec), FIX.requestId);
  assert.equal(lib.approvalVerify(lib.edPubOfSeed(FIX.holderEdSeed), op, FIX.challengeNonce, FIX.challengeExpiry, lib.x25519Pub(FIX.challengeEphSec), FIX.requestId, sig), true, 'approval verifies');
  // tamper: flipped request_id must break the approval
  assert.equal(lib.approvalVerify(lib.edPubOfSeed(FIX.holderEdSeed), op, FIX.challengeNonce, FIX.challengeExpiry, lib.x25519Pub(FIX.challengeEphSec), 43n, sig), false, 'tampered request_id rejected');
});

test('share_response SWR1 frame + sealed box self-consistency', () => {
  const frame = lib.shareResponse(
    lib.x25519Pub(FIX.challengeEphSec),
    FIX.sharePlain,
    FIX.respEphSec,
    FIX.respNonce,
    FIX.requestId,
  );
  // SWR1 ‖ 0001 ‖ 02 ‖ request_id u64 BE ‖ len u64 BE ‖ ciphertext
  eqHex(hx(frame.slice(0, 4)), '53575231', 'magic SWR1');
  eqHex(hx(frame.slice(4, 6)), '0001', 'version 1 BE');
  assert.equal(frame[6], 2, 'kind 2 = share_response');
  assert.equal(Number(new DataView(frame.buffer, frame.byteOffset + 7).getBigUint64(0, false)), FIX.requestId, 'request_id u64 BE');
  const ctLen = Number(new DataView(frame.buffer, frame.byteOffset + 15).getBigUint64(0, false));
  assert.equal(ctLen, 32 + 32 + 24 + 16, 'ciphertext len = eph(32)+nonce(24)+tag(16)+pt(32)');
  const sealed = frame.slice(23);
  assert.equal(sealed.length, ctLen, 'ciphertext length field matches');
  assert.deepEqual(lib.chachaBoxOpen(FIX.challengeEphSec, sealed), FIX.sharePlain, 'challenge key opens the sealed share');
});

// ================================================================ vectors.json replay

const vectorsPath = `${ROOT}/vectors.json`;
const vectorsExist = fs.existsSync(vectorsPath);

test('A1: full vectors.json replay', { skip: vectorsExist ? false : 'vectors.json not present yet - rust worker still building' }, async (t) => {
  const data = JSON.parse(fs.readFileSync(vectorsPath, 'utf8'), (k, v) => v);
  const raw = fs.readFileSync(vectorsPath, 'utf8');
  // exact-number parse for u64::MAX style values inside op structures
  const vectors = lib.parseJSONExact(raw).vectors;
  void data;

  const results = [];
  const run = async (v) => {
    const p = v.params ?? {};
    const i = v.inputs ?? {};
    const o = v.outputs ?? {};
    const h2b = (x) => (typeof x === 'string' ? bytes(x) : x);
    const opIn = (op) => ({ ...op, prev_hash: h2b(op.prev_hash), signature: h2b(op.signature) });
    switch (v.primitive) {
      case 'chain_hash':
        return eqHex(hx(lib.chainHash(h2b(i.bytes))), o.hash, v.id);
      case 'derive_key':
        return eqHex(hx(lib.deriveKey(p.context, h2b(i.ikm))), o.key, v.id);
      case 'ed25519_sign': {
        const pubk = lib.edPubOfSeed(h2b(i.seed));
        const sig = lib.edSign(h2b(i.seed), h2b(i.msg));
        eqHex(hx(pubk), o.pub, `${v.id} pub`);
        eqHex(hx(sig), o.sig, `${v.id} sig`);
        return;
      }
      case 'x25519_dh':
        eqHex(hx(lib.x25519Pub(h2b(i.a_sec))), o.a_pub, `${v.id} a_pub`);
        return eqHex(hx(lib.x25519DH(h2b(i.a_sec), h2b(i.b_pub))), o.shared, `${v.id} shared`);
      case 'mlkem_encaps': {
        const enc = lib.mlkemEncapsulate(h2b(i.ek), h2b(p.m));
        eqHex(hx(enc.cipherText), o.ct, `${v.id} ct`);
        return eqHex(hx(enc.sharedSecret), o.ss, `${v.id} ss`);
      }
      case 'mlkem_decaps':
        return eqHex(hx(lib.mlkemDecapsulate(h2b(i.ct), h2b(i.dk))), o.ss, `${v.id}`);
      case 'dek_seal':
        return eqHex(hx(lib.dekSeal(h2b(i.dek), h2b(i.plaintext), h2b(p.nonce))), o.ct, v.id);
      case 'argon2id':
        return eqHex(hx(await lib.argon2idDerive(i.input, h2b(i.salt), lib.ARGON2_PROFILES[p.profile])), o.key, v.id);
      case 'bip39':
        eqHex(lib.bip39Encode(h2b(i.entropy)), o.phrase, `${v.id} phrase`);
        if (o.decode_ok !== undefined && o.decode_ok) lib.bip39Decode(o.phrase);
        return;
      case 'eff7776':
        return eqHex(lib.effEncode(EFF_WORDS, h2b(i.entropy)), o.phrase, v.id);
      case 'shamir_split': {
        const rng = lib.chachaRngStream(h2b(p.rng_seed), 32 * Math.max(0, (p.k | 0) - 1));
        const shares = lib.splitAtIndicesSeeded(h2b(i.secret), p.k, Array.from({ length: p.n }, (_, idx) => idx + 1), rng);
        const rustShares = o.shares.map((s) => `${String(s.index).padStart(2, '0')}${s.bytes_hex}`);
        eqHex(shares.map((s) => `${String(s.index).padStart(2, '0')}${hx(s.bytes)}`).join(''), rustShares.join(''), `${v.id} shares`);
        return;
      }
      case 'shamir_combine':
        return eqHex(hx(lib.shamirCombine(i.shares.map((s) => ({ index: s.index, bytes: bytes(s.bytes_hex) })))), o.secret, v.id);
      case 'reshare': {
        const rng = lib.chachaRngStream(h2b(p.rng_seed), 32 * Math.max(0, (p.k | 0) - 1));
        const c = lib.reshareContribute({ index: i.share.index, bytes: bytes(i.share.bytes_hex) }, p.new_indices, p.k, rng);
        eqHex(String(c.from), String(o.from), `${v.id} from`);
        return eqHex(
          c.sub_shares.map(([x, b]) => `${String(x).padStart(2, '0')}${hx(b)}`).join(''),
          o.sub_shares.map((s) => `${String(s.index).padStart(2, '0')}${s.bytes_hex}`).join(''),
          `${v.id} sub_shares`,
        );
      }
      case 'op_canonical':
        eqHex(hx(lib.opCanonicalBytes(opIn(i.op))), o.canonical_hex, v.id);
        // the canonical text of a Rust-produced op must also survive a parse and re-serialise round trip
        return eqHex(lib.canonicalJSON(lib.parseJSONExact(Buffer.from(bytes(o.canonical_hex)).toString('utf8'))), Buffer.from(bytes(o.canonical_hex)).toString('utf8'), `${v.id} reparse`);
      case 'op_hash':
        return eqHex(hx(lib.opHash(opIn(i.op))), o.hash, v.id);
      case 'op_verify':
        return assert.equal(lib.opVerify(opIn(i.op), h2b(i.pub)), o.valid, v.id);
      case 'binding_hash': {
        const pre = lib.bindingHashPreimage(opIn(i.op), h2b(i.nonce), i.expiry, h2b(i.eph), i.request_id);
        eqHex(hx(pre), o.preimage, `${v.id} preimage`);
        return eqHex(hx(lib.bindingHash(opIn(i.op), h2b(i.nonce), i.expiry, h2b(i.eph), i.request_id)), o.binding_hash, v.id);
      }
      case 'hybrid_wrap': {
        const m = p.m !== undefined ? h2b(p.m) : lib.chachaRngStream(h2b(p.rng_seed), 32);
        const blob = lib.hybridWrap({ x25519_pub: h2b(i.recipient.x25519_pub), mlkem_pub: h2b(i.recipient.mlkem_pub) }, h2b(i.plaintext), h2b(p.eph_secret), h2b(p.nonce), m);
        return eqHex(hx(blob), o.blob, v.id);
      }
      case 'sealed_box': {
        const sealed = lib.chachaBoxSeal(h2b(i.recipient_x25519_pub), h2b(i.plaintext), h2b(p.sealer_eph_secret), h2b(p.nonce));
        eqHex(hx(sealed), o.sealed, `${v.id} seal`);
        return assert.deepEqual(lib.chachaBoxOpen(h2b(i.recipient_x25519_secret), sealed), h2b(i.plaintext), `${v.id} open`);
      }
      case 'signed_approval': {
        const preimage = lib.approvalPreimage(opIn(i.op), h2b(i.nonce), i.expiry, h2b(i.eph), i.request_id);
        eqHex(hx(preimage), o.preimage, `${v.id} preimage`);
        eqHex(hx(lib.approvalSign(h2b(p.holder_seed ?? i.holder_seed), opIn(i.op), h2b(i.nonce), i.expiry, h2b(i.eph), i.request_id)), o.sig, `${v.id} sig`);
        return assert.equal(lib.approvalVerify(h2b(i.holder_pub), opIn(i.op), h2b(i.nonce), i.expiry, h2b(i.eph), i.request_id, h2b(o.sig)), true, `${v.id} verify`);
      }
      case 'share_response':
        return; // A2 direction: JS produces, Rust opens - nothing to replay
      default:
        throw new Error(`unknown primitive ${v.primitive} (vector ${v.id})`);
    }
  };

  for (const v of vectors) {
    const t0 = performance.now();
    try {
      await run(v);
      results.push(`${v.primitive.padEnd(18)} PASS  ${v.id}${v.params && v.params.profile ? ` [${v.params.profile}]` : ''}  ${(performance.now() - t0).toFixed(0)} ms`);
    } catch (e) {
      results.push(`${v.primitive.padEnd(18)} FAIL  ${v.id}: ${e.message.split('\n')[0]}`);
      throw e;
    }
  }
  console.log(`\n=== vectors.json replay: ${vectors.length} vectors ===`);
  for (const line of results) console.log('  ' + line);
});
