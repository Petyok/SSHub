// Phase B, U13-4 revised: PhoneHandover on the phone page (design 5, contract 4j). Mirrors crates/sshub-phone-core/src/handover.rs.
//   cd /work/assets/companion && NODE_PATH=/opt/node/lib/node_modules node --test test/phone-handover.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { bytesOf, handoverWords } from '../src/lib.mjs';
import { encodePhoneHandover, parseOp } from '../src/ops.mjs';
import { identityOfSecret } from '../src/keys.mjs';
import { emptyState, restoredState } from '../src/state.mjs';
import { decodePhoneHandoverRequest, encodePhoneHandoverRequest } from '../src/frames.mjs';
import { issueCert } from '../src/cert.mjs';
import { buildPhoneHandover, describePhoneHandover, signHandoverAdmission } from '../src/handover.mjs';

const finalVectors = JSON.parse(readFileSync(new URL('../../../tests/vectors/op/ops.json', import.meta.url), 'utf8'));
const h = (x) => bytesOf(x);
const NOW = 1893452400;
const oldId = identityOfSecret(h('42'.repeat(32)));
const newId = identityOfSecret(h('55'.repeat(32)));
const newPhone = { ed25519_pub: newId.edPub, x25519_pub: newId.xPub, mlkem_pub: newId.mlkemPub };
const req = (name, over = {}) => ({ new_phone: newPhone, name, terminal: '07'.repeat(8), ...over });
const enrolledState = () => ({ ...emptyState(), enrolled: { device_id: oldId.deviceId, name: 'old phone', at: NOW } });
const kindOf = (s) => parseOp(h(s.chain.at(-1).canonical)).kind;

test('kind 12 round trip, truncation and the PhoneHandover encoder reproduces the final vector byte for byte', () => {
  const r = req('new phone');
  const back = decodePhoneHandoverRequest(encodePhoneHandoverRequest(r));
  assert.deepEqual([back.name, back.terminal], ['new phone', r.terminal]);
  assert.deepEqual(back.new_phone, newPhone);
  const bytes = encodePhoneHandoverRequest(r);
  assert.throws(() => decodePhoneHandoverRequest(bytes.slice(0, bytes.length - 1)));
  assert.throws(() => decodePhoneHandoverRequest(Uint8Array.of(...bytes, 0)));
  const v = finalVectors.vectors.find((x) => x.id === 'phone-handover');
  const op = parseOp(h(v.canonical_hex));
  const header = { device_id: op.device_id, seq: op.seq, lamport: op.lamport, prev_hash: op.prev_hash, timestamp: op.timestamp, epoch: op.epoch };
  const np = { ed25519_pub: h(op.fields.new_phone.ed25519_pub), x25519_pub: h(op.fields.new_phone.x25519_pub), mlkem_pub: Uint8Array.from(Buffer.from(op.fields.new_phone.mlkem_pub, 'base64')) };
  assert.equal(Buffer.from(encodePhoneHandover(header, np, op.fields.name)).toString('hex'), v.canonical_hex);
});

test('the view shows the shared words, the name and the certificates that stay valid', () => {
  const s = enrolledState();
  issueCert(s, oldId, { device_id: 'cccccccccccccccc', scope: { hosts: ['01'.repeat(16)] }, expiry: BigInt(NOW + 86400), max_new_hosts_per_day: 2, epoch_lo: 0n, epoch_hi: 1n }, NOW);
  const v = describePhoneHandover(s, req('new phone'));
  assert.deepEqual(v.words, handoverWords(newId.edPub, newId.xPub, newId.mlkemPub, 'new phone', h(oldId.deviceId)));
  assert.equal(v.words.length, 8);
  assert.equal(v.name, 'new phone');
  assert.equal(v.certs_carried.length, 1);
  assert.match(v.certs_carried[0], /cccccccccccccccc/);
  assert.notDeepEqual(describePhoneHandover(s, req('new phonf')).words, v.words);
});

test('a request that is not right is refused before anything is shown or signed', () => {
  const s = enrolledState();
  for (const bad of ['', '  ', ' padded ', 'tab\there', 'x'.repeat(65)]) {
    assert.throws(() => describePhoneHandover(s, req(bad)), undefined, JSON.stringify(bad));
    assert.throws(() => buildPhoneHandover(s, oldId, req(bad), NOW), undefined, JSON.stringify(bad));
  }
  const broken = req('new phone', { new_phone: { ...newPhone, mlkem_pub: newPhone.mlkem_pub.slice(1) } });
  assert.throws(() => describePhoneHandover(s, broken), /keys are not valid/);
  assert.throws(() => buildPhoneHandover(s, oldId, req('new phone', { new_phone: { ed25519_pub: oldId.edPub, x25519_pub: oldId.xPub, mlkem_pub: oldId.mlkemPub } }), NOW), /its own successor/);
  assert.throws(() => buildPhoneHandover(emptyState(), oldId, req('new phone'), NOW), /not part of a ring/);
  const r = restoredState();
  r.enrolled = s.enrolled;
  assert.throws(() => buildPhoneHandover(r, oldId, req('new phone'), NOW), /must not sign/);
  assert.equal(s.chain.length, 0, 'refusals sign nothing');
});

test('the old phone signs one handover and no second; the new phone signs its admission over the op hash', () => {
  const s = enrolledState();
  buildPhoneHandover(s, oldId, req('new phone'), NOW);
  const op = parseOp(h(s.chain[0].canonical));
  assert.equal(op.kind, 'PhoneHandover');
  assert.equal(op.fields.name, 'new phone');
  assert.throws(() => buildPhoneHandover(s, oldId, req('another'), NOW + 1), /already signed a handover/);
  const n = emptyState();
  signHandoverAdmission(n, newId, '09'.repeat(32), NOW);
  assert.equal(kindOf(n), 'RedPhoneSign');
  assert.equal(parseOp(h(n.chain[0].canonical)).fields.op_hash, '09'.repeat(32));
});
