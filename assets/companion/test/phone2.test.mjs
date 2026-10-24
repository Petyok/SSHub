// Phone page, second round: Red co-signature, ops export, wrap key generations and the reshare
// receiver. The reshare scenario is built here with the vector-verified primitives of lib.mjs
// (Shamir, resharing, hybrid wrap); a Rust-made reshare fixture is requested separately so the op
// bodies are also pinned against `reissue_plan`.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { xchacha20poly1305 } from '../vendor/noble-ciphers-2.4.0/chacha.js';
import { blake3 } from '../vendor/noble-hashes-2.4.0/blake3.js';
import {
  bytesOf, canonicalJSON, chachaRngStream, concat, deriveKey, deviceIdOf, edSign, edVerify, hexOf, hybridWrap, identityFromIkm, reshareCombine, reshareContribute,
  shamirCombine, splitAtIndicesSeeded, utf8,
} from '../src/lib.mjs';
import {
  b64dec, b64enc, encodeRedPhoneSign, encodeShareReshareConfirmBatch, encodeShareStorageKey, encodeWrapKeyRotate, isRedOp, describeOp, parseOp, opHashHex,
} from '../src/ops.mjs';
import { identityOfSecret, randomBytes } from '../src/keys.mjs';
import {
  appendRedPhoneSign, appendWrapKeyRotate, courierOps, emptyState,
} from '../src/state.mjs';
import { decodePhoneOps, encodePhoneOps } from '../src/frames.mjs';
import { envelopeAad, ownStorageKey, planReshare, publishReshare } from '../src/reshare.mjs';
import { EPOCH, HOST, hostBytes, scenario as buildScenario } from './reshare-scenario.mjs';

const scenario = (o = {}) => buildScenario({ phone, ...o });

const here = new URL('./fixtures/', import.meta.url);
const opVectors = JSON.parse(readFileSync(new URL('ops.vectors.json', here), 'utf8'));
const h = (x) => bytesOf(x);
const vec = (id) => opVectors.vectors.find((v) => v.id === id);
const exact = (id) => parseOp(h(vec(id).canonical_hex));
const hdr = (o) => ({ device_id: o.device_id, seq: o.seq, lamport: o.lamport, prev_hash: o.prev_hash, timestamp: o.timestamp, epoch: o.epoch });
const phone = identityOfSecret(h('42'.repeat(32)));

test('the reshare ops the phone writes equal the canonical bytes of the op vectors', () => {
  const r = exact('wrap-key-rotate');
  assert.equal(hexOf(encodeWrapKeyRotate(hdr(r), { device_id: r.fields.device_id, wrap_epoch: r.fields.wrap_epoch, x25519_pub: h(r.fields.x25519_pub), mlkem_pub: b64dec(r.fields.mlkem_pub) })), vec('wrap-key-rotate').canonical_hex);
  const k = exact('share-storage-key-empty');
  assert.equal(hexOf(encodeShareStorageKey(hdr(k), { epoch: k.fields.epoch, sealed: b64dec(k.fields.sealed) })), vec('share-storage-key-empty').canonical_hex);
  const c = exact('share-reshare-confirm-batch');
  assert.equal(
    hexOf(encodeShareReshareConfirmBatch(hdr(c), { epoch: c.fields.epoch, items: c.fields.items.map((i) => ({ host_uid: i.host_uid, sealed_share: b64dec(i.sealed_share) })) })),
    vec('share-reshare-confirm-batch').canonical_hex,
  );
});

test('RedPhoneSign names the op hash, is signed by the phone and is described', () => {
  const notice = exact('device-add-holder');
  assert.equal(isRedOp(notice), true);
  assert.equal(isRedOp(exact('device-add-mirror')), false);
  assert.equal(isRedOp(exact('host-upsert-full')), false);
  for (const id of ['device-revoke', 'mode-change-some', 'cert-issue-hosts']) assert.equal(isRedOp(exact(id)), true, id);
  const hash = opHashHex(h(vec('device-add-holder').canonical_hex));
  const state = emptyState();
  const r = appendRedPhoneSign(state, phone, hash, 1893452500, { lamport: notice.lamport, epoch: notice.epoch });
  const op = parseOp(r.canonical);
  assert.equal(op.kind, 'RedPhoneSign');
  assert.deepEqual(op.fields, { op_hash: hash });
  assert.equal(op.device_id, phone.deviceId);
  assert.ok(op.lamport > notice.lamport);
  assert.ok(edVerify(phone.edPub, r.canonical, r.signature));
  assert.equal(encodeRedPhoneSign(hdr(op), hash).length, r.canonical.length);
  const d = describeOp(notice);
  assert.ok(d.dangerCount > 0, 'a holder admission is highlighted');
});

test('ops travel as an ops frame in both directions, capped by the courier', () => {
  const state = emptyState();
  for (let i = 0; i < 10; i++) appendRedPhoneSign(state, phone, '0'.repeat(64), 1893452500 + i, null);
  assert.equal(courierOps(state).length, 8);
  const frame = encodePhoneOps(courierOps(state));
  assert.deepEqual(decodePhoneOps(frame).map((o) => hexOf(o.canonical)), courierOps(state).map((o) => hexOf(o.canonical)));
  assert.throws(() => decodePhoneOps(Uint8Array.of(...frame, 0)), /trailing/);
});

test('wrap key generations: rotation derives new keys, every generation still unwraps', () => {
  const secret = h('42'.repeat(32));
  const g0 = identityOfSecret(secret, 0);
  const g1 = identityOfSecret(secret, 1);
  const g2 = identityOfSecret(secret, 2);
  assert.equal(g0.deviceId, g1.deviceId);
  assert.notEqual(hexOf(g0.xPub), hexOf(g1.xPub));
  assert.notEqual(hexOf(g1.mlkemPub), hexOf(g2.mlkemPub));
  assert.equal(hexOf(g0.edPub), hexOf(g2.edPub), 'the signing key never rotates');
  const wrapTo = (id, pt) => hybridWrap({ x25519_pub: id.xPub, mlkem_pub: id.mlkemPub }, pt, randomBytes(32), randomBytes(24), randomBytes(32));
  const old = wrapTo(g0, utf8('old'));
  const fresh = wrapTo(g2, utf8('fresh'));
  assert.equal(new TextDecoder().decode(g2.unwrap(old)), 'old');
  assert.equal(new TextDecoder().decode(g2.unwrap(fresh)), 'fresh');
  assert.throws(() => g0.unwrap(fresh));
  const state = emptyState();
  const r = appendWrapKeyRotate(state, g0, g1, 1893452500, null);
  const op = parseOp(r.canonical);
  assert.equal(op.kind, 'WrapKeyRotate');
  assert.equal(op.fields.x25519_pub, hexOf(g1.xPub));
  assert.equal(b64enc(g1.mlkemPub), op.fields.mlkem_pub);
  assert.equal(state.wrapEpoch, 1);
});

// ------------------------------------------------------------ reshare receiver

test('the phone adopts a reshare: its new share combines with another new holder to the secret', () => {
  const s = scenario();
  const plan = planReshare(phone, s.signed, EPOCH, { ring: s.ring });
  assert.equal(plan.ok, true, plan.error);
  assert.equal(plan.myIndex, s.holders.indexOf(phone.deviceId) + 1);
  assert.equal(plan.items.length, 1);
  assert.ok(plan.contributors.every((c) => c.verified && c.session));
  assert.ok(plan.storageSealed, 'a first adoption publishes a storage key');
  const state = emptyState();
  const out = publishReshare(state, phone, plan, 1893452500);
  assert.deepEqual(out.map((o) => parseOp(o.canonical).kind), ['ShareStorageKey', 'ShareReshareConfirmBatch']);
  for (const o of out) assert.ok(edVerify(phone.edPub, o.canonical, o.signature));
  // the sealed share opens with the storage key the chain holds, under the envelope aad
  const key = ownStorageKey(phone, state.chain, EPOCH);
  assert.equal(key.length, 32);
  const confirm = parseOp(out[1].canonical).fields.items[0];
  assert.equal(confirm.host_uid, HOST);
  const sealed = b64dec(confirm.sealed_share);
  const mine = xchacha20poly1305(key, sealed.slice(0, 24), envelopeAad(hostBytes, EPOCH, plan.myIndex)).decrypt(sealed.slice(24));
  assert.equal(mine[0], plan.myIndex);
  // another new holder's share from the same contributions (what its own receiver would compute)
  const otherIdx = s.holders.indexOf(s.ids.D) + 1;
  const other = {
    index: otherIdx,
    bytes: (() => {
      const cs = [s.ids.cA, s.ids.cB].map((c) => ({ from: c.from, sub_shares: [c.sub_shares.find(([x]) => x === otherIdx)] }));
      return reshareCombine(otherIdx, cs).bytes;
    })(),
  };
  assert.deepEqual(shamirCombine([{ index: mine[0], bytes: mine.slice(1) }, other]), s.secret);
  // a second pass reuses the published storage key
  const again = planReshare(phone, s.signed, EPOCH, { ring: s.ring, existingStorage: key });
  assert.equal(again.storageSealed, null);
});

test('a lying contributor, a bad signature and a foreign holder set are not adopted', () => {
  const lie = scenario({ lie: true });
  const p = planReshare(phone, lie.signed, EPOCH, { ring: lie.ring });
  assert.equal(p.ok, false);
  assert.match(p.error, /two readable contributions/);
  assert.equal(p.skipped.length, 1);

  const good = scenario();
  const forged = good.signed.map((o) => ({ ...o }));
  forged[1].signature = forged[1].signature.slice();
  forged[1].signature[3] ^= 1;
  const f = planReshare(phone, forged, EPOCH, { ring: good.ring });
  assert.equal(f.ok, false);
  assert.match(f.error, /does not verify/);

  const other = identityOfSecret(h('43'.repeat(32)));
  const o = planReshare(other, good.signed, EPOCH, { ring: good.ring });
  assert.equal(o.ok, false);
  assert.match(o.error, /not a holder/);
  assert.equal(planReshare(phone, good.signed, 9, { ring: good.ring }).ok, false);
  void blake3;
});

test('a delivery that needs more than 8 ops is refused with the reason', () => {
  const s = scenario();
  const plan = planReshare(phone, s.signed, EPOCH, { ring: s.ring });
  plan.items = Array.from({ length: 1500 }, () => plan.items[0]);
  assert.throws(() => publishReshare(emptyState(), phone, plan, 1893452500), /carries 8/);
});

// ------------------------------------------------------------ the op-log package's own vectors

const finalVectors = JSON.parse(readFileSync(new URL('../../../tests/vectors/op/ops.json', import.meta.url), 'utf8'));

test('every final op vector (all body variants incl. the Red ceremony ops) parses, verifies and is described', () => {
  const signer = h(finalVectors.meta.signer_ed25519_pub_hex);
  const kinds = new Set();
  for (const v of finalVectors.vectors) {
    const bytes = h(v.canonical_hex);
    const op = parseOp(bytes);
    kinds.add(op.kind);
    assert.ok(edVerify(signer, bytes, h(v.signature_hex)), `${v.id}: signature`);
    const d = describeOp(op);
    assert.ok(d.rows.length >= 3, v.id);
    const leafRows = d.rows.filter((r) => !['Op signed by', 'Op time', 'Op seq / epoch'].includes(r.label));
    if (['FactorRegister', 'DeviceRoleChange', 'BulkExport'].includes(op.kind)) assert.ok(leafRows.every((r) => r.danger), `${v.id}: Red body fully highlighted`);
    assert.equal(isRedOp(op), ['DeviceRevoke', 'CertIssue', 'ModeChange', 'FactorRegister', 'DeviceRoleChange', 'BulkExport', 'ExemptionSet'].includes(op.kind) || (op.kind === 'DeviceAdd' && op.fields.role === 'Holder'), v.id);
  }
  assert.ok(finalVectors.vectors.length >= 48);
  for (const k of ['RedApprove', 'RedPhoneSign', 'RedFactor', 'FactorRegister', 'DeviceRoleChange', 'BulkExport']) assert.ok(kinds.has(k), k);
  const v = finalVectors.vectors.find((x) => x.id === 'red-phone-sign');
  const o = parseOp(h(v.canonical_hex));
  assert.equal(hexOf(encodeRedPhoneSign(hdr(o), o.fields.op_hash)), v.canonical_hex, 'RedPhoneSign bytes equal the vector');
});
