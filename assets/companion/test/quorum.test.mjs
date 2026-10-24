// Replays tests/vectors/quorum/vectors.json (produced by the Rust side) against the phone's
// quorum module. Every vector must pass; none is skipped. Run: node --test test/
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { xchacha20poly1305 } from '../vendor/noble-ciphers-2.4.0/chacha.js';
import { argon2idDerive, concat } from '../src/lib.mjs';
import {
  bytesToHex,
  crockfordEncode,
  decodeChallenge,
  decodePhoneMaterial,
  encodeShareResponse,
  FountainDecoder,
  fountainEncode,
  hexToBytes,
  normalizeCode,
  snapshotOpen,
  snapshotSeal,
} from '../src/quorum.mjs';

const root = new URL('../../../', import.meta.url);
const file = JSON.parse(readFileSync(new URL('tests/vectors/quorum/vectors.json', root), 'utf8'));
const vectors = file.vectors;
const EMAIL_MAGIC = new TextEncoder().encode('SSHE');

// Test-side seal for the email fragment (terminal writes it; the phone only reads it back).
// Blob: "SSHE" || 1 || salt(16) || nonce(24) || ct, header as associated data.
async function emailFragmentSeal(fragment, code, profile, salt, nonce) {
  const header = concat(EMAIL_MAGIC, Uint8Array.of(1), salt, nonce);
  const key = await argon2idDerive(normalizeCode(code).code, salt, profile);
  return concat(header, xchacha20poly1305(key, nonce, header).encrypt(fragment));
}

const profileOf = (p) => ({ mem_kib: p.mem_kib, iters: p.iters });

const replay = {
  async crockford_encode(v) {
    assert.equal(crockfordEncode(hexToBytes(v.inputs.bytes)), v.outputs.code);
  },
  async normalize_code(v) {
    const got = normalizeCode(v.inputs.input);
    assert.deepEqual({ code: got.code, ok: got.ok }, { code: v.outputs.code, ok: v.outputs.ok });
  },
  async snapshot_seal(v) {
    const blob = await snapshotSeal(
      v.inputs.share_index,
      hexToBytes(v.inputs.share_bytes),
      v.inputs.code,
      v.inputs.issued_at,
      profileOf(v.params.profile),
      hexToBytes(v.params.salt),
      hexToBytes(v.params.nonce),
    );
    assert.equal(bytesToHex(blob), v.outputs.blob);
  },
  async snapshot_open(v) {
    const got = await snapshotOpen(hexToBytes(v.inputs.blob), v.inputs.code, v.inputs.now);
    assert.equal(got.ok, v.outputs.ok);
    if (v.outputs.ok) {
      assert.equal(got.share_index, v.outputs.share_index);
      assert.equal(bytesToHex(got.share_bytes), v.outputs.share_bytes);
    }
  },
  async email_fragment_seal(v) {
    const blob = await emailFragmentSeal(
      hexToBytes(v.inputs.fragment),
      v.inputs.code,
      profileOf(v.params.profile),
      hexToBytes(v.params.salt),
      hexToBytes(v.params.nonce),
    );
    assert.equal(bytesToHex(blob), v.outputs.blob);
  },
  async wire_challenge(v) {
    const material = hexToBytes(v.inputs.phone_material);
    decodePhoneMaterial(material);
    const frame = hexToBytes(v.outputs.frame);
    const got = decodeChallenge(frame);
    assert.equal(bytesToHex(got.binding_preimage), v.outputs.binding_preimage);
    assert.equal(bytesToHex(got.material_hash), v.outputs.material_hash);
    assert.equal(bytesToHex(got.binding_hash), v.outputs.binding_hash);
    assert.equal(bytesToHex(got.nonce), v.inputs.nonce);
    assert.equal(got.expiry, v.inputs.expiry);
    assert.equal(bytesToHex(got.eph_x25519_pub), v.inputs.eph_x25519_pub);
    assert.equal(got.request_id, v.inputs.request_id);
    assert.equal(bytesToHex(got.op_canonical), v.inputs.op_canonical);
    assert.equal(bytesToHex(got.op_signature), v.inputs.op_signature);
    assert.equal(bytesToHex(got.material), v.inputs.phone_material);
  },
  async wire_share_response(v) {
    const frame = encodeShareResponse(v.inputs.request_id, hexToBytes(v.inputs.ciphertext));
    assert.equal(bytesToHex(frame), v.outputs.frame);
  },
  async fountain_encode(v) {
    const payload = hexToBytes(v.inputs.payload);
    const frames = fountainEncode(payload, v.params.frame_bytes, v.params.parity_percent);
    assert.deepEqual(frames.map(bytesToHex), v.outputs.frames);
    // Round trip with one data frame lost: the parity frame must repair it.
    const dec = new FountainDecoder();
    const k = Math.ceil(payload.length / (v.params.frame_bytes - 8));
    frames.forEach((f, i) => {
      if (!(k >= 2 && i === 0)) dec.push(f);
    });
    assert.deepEqual(dec.finish(), payload);
  },
};

for (const v of vectors) {
  test(`quorum vector ${v.id} (${v.primitive})`, async () => {
    const fn = replay[v.primitive];
    assert.ok(fn, `no replay for primitive ${v.primitive}`);
    await fn(v);
  });
}

test('fountain decoder reports other documents and redundant frames', () => {
  const a = fountainEncode(new Uint8Array(500).fill(7), 100, 25);
  const b = fountainEncode(new Uint8Array(500).fill(9), 100, 25);
  const dec = new FountainDecoder();
  assert.equal(dec.push(a[0]), 'progress');
  assert.equal(dec.push(a[0]), 'redundant');
  assert.equal(dec.push(b[1]), 'other');
});
