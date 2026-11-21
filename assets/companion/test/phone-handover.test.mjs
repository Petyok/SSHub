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
import { buildPhoneHandover, describeHandoverAdmission, describePhoneHandover, signHandoverAdmission } from '../src/handover.mjs';

const wireText = readFileSync(new URL('../../../tests/vectors/wire/phone_handover_request.json', import.meta.url), 'utf8');
const wireVectors = JSON.parse(wireText);
// u64 lamports exceed 2^53, so they are read from the text, not from the parsed JSON
const wireLamports = [...wireText.matchAll(/"op_lamport": (\d+|null)/g)].map((m) => m[1]);
const finalVectors = JSON.parse(readFileSync(new URL('../../../tests/vectors/op/ops.json', import.meta.url), 'utf8'));
const h = (x) => bytesOf(x);
const NOW = 1893452400;
const oldId = identityOfSecret(h('42'.repeat(32)));
const newId = identityOfSecret(h('55'.repeat(32)));
const newPhone = { ed25519_pub: newId.edPub, x25519_pub: newId.xPub, mlkem_pub: newId.mlkemPub };
const req = (name, over = {}) => ({ new_phone: newPhone, name, terminal: '07'.repeat(8), op: null, ...over });
const termId = identityOfSecret(h('66'.repeat(32))); // the terminal the old phone enrolled with, NOT the phone's own id
const TERMINAL_ID = termId.deviceId;
const enrolledState = () => ({ ...emptyState(), enrolled: { terminal_id: TERMINAL_ID, name: 'old phone', at: NOW } });
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
  const v = describePhoneHandover(s, oldId, req('new phone'));
  assert.deepEqual(v.words, handoverWords(newId.edPub, newId.xPub, newId.mlkemPub, 'new phone', h(oldId.deviceId)));
  assert.equal(v.words.length, 8);
  assert.equal(v.name, 'new phone');
  assert.equal(v.certs_carried.length, 1);
  assert.match(v.certs_carried[0], /cccccccccccccccc/);
  assert.notDeepEqual(describePhoneHandover(s, oldId, req('new phonf')).words, v.words);
});

test('a request that is not right is refused before anything is shown or signed', () => {
  const s = enrolledState();
  for (const bad of ['', '  ', ' padded ', 'tab\there', 'x'.repeat(65)]) {
    assert.throws(() => describePhoneHandover(s, oldId, req(bad)), undefined, JSON.stringify(bad));
    assert.throws(() => buildPhoneHandover(s, oldId, req(bad), NOW), undefined, JSON.stringify(bad));
  }
  const broken = req('new phone', { new_phone: { ...newPhone, mlkem_pub: newPhone.mlkem_pub.slice(1) } });
  assert.throws(() => describePhoneHandover(s, oldId, broken), /keys are not valid/);
  assert.throws(() => buildPhoneHandover(s, oldId, req('new phone', { new_phone: { ed25519_pub: oldId.edPub, x25519_pub: oldId.xPub, mlkem_pub: oldId.mlkemPub } }), NOW), /its own successor/);
  assert.throws(() => buildPhoneHandover(emptyState(), oldId, req('new phone'), NOW), /not part of a ring/);
  const r = restoredState();
  r.enrolled = s.enrolled;
  assert.throws(() => buildPhoneHandover(r, oldId, req('new phone'), NOW), /must not sign/);
  assert.equal(s.chain.length, 0, 'refusals sign nothing');
});

test('the old phone signs one handover and no second', () => {
  const s = enrolledState();
  buildPhoneHandover(s, oldId, req('new phone'), NOW);
  const op = parseOp(h(s.chain[0].canonical));
  assert.equal(op.kind, 'PhoneHandover');
  assert.equal(op.fields.name, 'new phone');
  assert.throws(() => buildPhoneHandover(s, oldId, req('another'), NOW + 1), /already signed a handover/);
});

test('kind 12 wire vectors (contract 4l b): decode and encode reproduce the request and both admission frames', () => {
  for (const [n, v] of wireVectors.vectors.entries()) {
    const i = v.inputs;
    const r = { new_phone: { ed25519_pub: h(i.ed), x25519_pub: h(i.x), mlkem_pub: h(i.mlkem) }, name: i.name, terminal: i.terminal, op: i.op_hash ? { hash: i.op_hash, lamport: BigInt(wireLamports[n]) } : null };
    assert.deepEqual(decodePhoneHandoverRequest(h(v.outputs.frame)), r, v.id);
    assert.equal(Buffer.from(encodePhoneHandoverRequest(r)).toString('hex'), v.outputs.frame, v.id);
  }
  const frame = encodePhoneHandoverRequest(req('new phone', { op: { hash: '0a'.repeat(32), lamport: 5n } }));
  assert.throws(() => decodePhoneHandoverRequest(frame.slice(0, frame.length - 1)));
  assert.throws(() => decodePhoneHandoverRequest(Uint8Array.of(...frame, 0)));
});

test('the new phone signs its admission after the op it names, and only for its own keys', () => {
  const n = emptyState();
  const r = req('new phone', { op: { hash: '09'.repeat(32), lamport: 700n } });
  const v = describeHandoverAdmission(newId, r);
  assert.deepEqual([v.op_hash, v.op_lamport, v.words.length, v.certs_carried.length], ['09'.repeat(32), 700, 0, 0]);
  signHandoverAdmission(n, newId, r, NOW);
  const op = parseOp(h(n.chain[0].canonical));
  assert.equal(op.kind, 'RedPhoneSign');
  assert.equal(op.fields.op_hash, '09'.repeat(32));
  assert.equal(Number(op.lamport), 701, 'after the op it names');
  signHandoverAdmission(n, newId, req('new phone', { op: { hash: '08'.repeat(32), lamport: 3n } }), NOW + 1);
  assert.equal(Number(parseOp(h(n.chain[1].canonical)).lamport), 702, 'a phone far ahead keeps its own next lamport');
  // another phone's keys, one key of three, no op, a forged huge lamport, a restored phone
  const strangerPub = identityOfSecret(h('66'.repeat(32)));
  const other = req('new phone', { op: r.op, new_phone: { ed25519_pub: strangerPub.edPub, x25519_pub: strangerPub.xPub, mlkem_pub: strangerPub.mlkemPub } });
  assert.throws(() => describeHandoverAdmission(newId, other), /another phone/);
  assert.throws(() => signHandoverAdmission(emptyState(), newId, other, NOW), /another phone/);
  const flipped = { ...newPhone, x25519_pub: Uint8Array.from(newPhone.x25519_pub, (b, i) => (i === 0 ? b ^ 1 : b)) };
  assert.throws(() => signHandoverAdmission(emptyState(), newId, req('new phone', { op: r.op, new_phone: flipped }), NOW), /another phone/);
  assert.throws(() => signHandoverAdmission(emptyState(), newId, req('new phone'), NOW), /no handover operation/);
  assert.throws(() => signHandoverAdmission(emptyState(), newId, req('new phone', { op: { hash: '09'.repeat(32), lamport: 2n ** 64n - 1n } }), NOW), /impossible counter/);
  assert.throws(() => signHandoverAdmission(restoredState(), newId, r, NOW), /must not sign/);
  // the old phone refuses the admission frame
  assert.throws(() => describePhoneHandover(enrolledState(), oldId, r), /admission frame/);
  assert.throws(() => buildPhoneHandover(enrolledState(), oldId, r, NOW), /admission frame/);
});

test('the words bind the phone\'s own id, not the terminal it enrolled with', () => {
  const s = enrolledState();
  const v = describePhoneHandover(s, oldId, req('new phone'));
  assert.deepEqual(v.words, handoverWords(newId.edPub, newId.xPub, newId.mlkemPub, 'new phone', h(oldId.deviceId)));
  assert.notDeepEqual(v.words, handoverWords(newId.edPub, newId.xPub, newId.mlkemPub, 'new phone', h(TERMINAL_ID)));
});

test('the own-successor check fires for the phone\'s own key and not for the terminal\'s', () => {
  const s = enrolledState();
  const naming = (id) => req('new phone', { new_phone: { ed25519_pub: id.edPub, x25519_pub: id.xPub, mlkem_pub: id.mlkemPub } });
  assert.throws(() => describePhoneHandover(s, oldId, naming(oldId)), /its own successor/);
  assert.doesNotThrow(() => describePhoneHandover(s, oldId, naming(termId)));
});
