// Phone page, third round (P1): trust-session certificates authored by the phone (CertIssue, CertRevoke,
// CertExtend with their kind 9 / kind 10 requests), the courier caps of C16, and the reshare receiver pinned
// to the Rust-made fixture (test/fixtures/reshare.json, not re-emitted here).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { xchacha20poly1305 } from '../vendor/noble-ciphers-2.4.0/chacha.js';
import { bytesOf, concat, edVerify, hexOf } from '../src/lib.mjs';
import { b64dec, describeOp, encodeCertExtend, encodeCertIssue, encodeCertRevoke, opHashHex, parseOp } from '../src/ops.mjs';
import { identityOfSecret } from '../src/keys.mjs';
import { courierOps, emptyState } from '../src/state.mjs';
import {
  KIND, MAX_COURIER_OPS, decodeCertExtendRequest, decodeCertRequest, decodeFrame, decodePhoneOps, encodeCertExtendRequest, encodeCertRequest, encodeFrame, encodePhoneOps,
} from '../src/frames.mjs';
import {
  certRequestProblems, describeCertRequest, extendCert, extendProblems, issueCert, issuedCerts, revokeCert,
} from '../src/cert.mjs';
import { envelopeAad, planReshare } from '../src/reshare.mjs';
import { DocumentCollector, SWITCH_AFTER_FRAMES } from '../src/scan.mjs';
import { fountainEncode } from '../src/quorum.mjs';

const here = new URL('./fixtures/', import.meta.url);
const opVectors = JSON.parse(readFileSync(new URL('ops.vectors.json', here), 'utf8'));
const reshareFx = JSON.parse(readFileSync(new URL('reshare.json', here), 'utf8'));
const h = (x) => bytesOf(x);
const vec = (id) => opVectors.vectors.find((v) => v.id === id);
const exact = (id) => parseOp(h(vec(id).canonical_hex));
const hdr = (o) => ({ device_id: o.device_id, seq: o.seq, lamport: o.lamport, prev_hash: o.prev_hash, timestamp: o.timestamp, epoch: o.epoch });
const phone = identityOfSecret(h('42'.repeat(32)));
const DELEGATE = identityOfSecret(h('71'.repeat(32))).deviceId;
const NOW = 1893452400;
const DAY = 86400;
const HOST_A = '0a'.repeat(16);
const HOST_B = '0b'.repeat(16);
const i64be = (n) => {
  const b = new Uint8Array(8);
  new DataView(b.buffer).setBigInt64(0, BigInt(n));
  return b;
};

const request = (over = {}) => ({
  device_id: DELEGATE, scope: { hosts: [HOST_A, HOST_B] }, expiry: BigInt(NOW + 7 * DAY), max_new_hosts_per_day: 2, epoch_lo: 0n, epoch_hi: 1n, ...over,
});
// What the page receives: the request through the kind 9 wire, as a terminal would show it.
const wire = (r) => decodeCertRequest(encodeCertRequest(r));
const MAGIC = Uint8Array.of(0x53, 0x57, 0x52, 0x31, 0, 1);

// ------------------------------------------------------------ vectors

test('the op-log vectors of the three certificate ops are reproduced byte for byte by the phone encoders', () => {
  for (const id of ['cert-issue-hosts', 'cert-issue-group-extremes']) {
    const o = exact(id);
    const f = o.fields;
    const req = { device_id: f.device_id, scope: f.scope.Group ? { group: f.scope.Group } : { hosts: f.scope.Hosts }, expiry: f.expiry, max_new_hosts_per_day: f.max_new_hosts_per_day, epoch_lo: f.epoch_lo, epoch_hi: f.epoch_hi };
    assert.equal(hexOf(encodeCertIssue(hdr(o), req)), vec(id).canonical_hex, id);
  }
  const r = exact('cert-revoke');
  assert.equal(hexOf(encodeCertRevoke(hdr(r), r.fields.cert_id)), vec('cert-revoke').canonical_hex);
  const e = exact('cert-extend');
  assert.equal(hexOf(encodeCertExtend(hdr(e), e.fields.cert_id, e.fields.extra_hosts)), vec('cert-extend').canonical_hex);
});

test('CertExtend is described with every row highlighted', () => {
  const d = describeOp(exact('cert-extend'));
  const leaves = d.rows.filter((r) => !['Op signed by', 'Op time', 'Op seq / epoch'].includes(r.label));
  assert.equal(leaves.length, 2);
  assert.ok(leaves.every((r) => r.danger));
});

// ------------------------------------------------------------ wire kinds 9 and 10

test('kind 9 and kind 10 have the byte layout of wire.rs', () => {
  const r = request({ scope: { group: '06'.repeat(16) } });
  const expected = concat(
    MAGIC, Uint8Array.of(9), h(DELEGATE), Uint8Array.of(0), h('06'.repeat(16)),
    i64be(NOW + 7 * DAY), Uint8Array.of(0, 0, 0, 2), new Uint8Array(8), Uint8Array.of(0, 0, 0, 0, 0, 0, 0, 1),
  );
  assert.equal(hexOf(encodeCertRequest(r)), hexOf(expected));
  assert.deepEqual(decodeCertRequest(expected), r);
  const hosts = wire(request());
  assert.deepEqual(hosts.scope, { hosts: [HOST_A, HOST_B] });
  assert.equal(hosts.device_id, DELEGATE);
  const ext = encodeCertExtendRequest({ cert_id: 'ab'.repeat(32), extra_hosts: 7 });
  assert.equal(hexOf(ext), hexOf(concat(MAGIC, Uint8Array.of(10), h('ab'.repeat(32)), Uint8Array.of(0, 0, 0, 7))));
  assert.deepEqual(decodeCertExtendRequest(ext), { cert_id: 'ab'.repeat(32), extra_hosts: 7 });
});

test('the request decoders refuse what wire.rs refuses', () => {
  const uids = (n) => Array.from({ length: n }, (_, i) => i.toString(16).padStart(2, '0').repeat(16));
  const ok = encodeCertRequest(request());
  assert.throws(() => decodeCertRequest(encodeCertRequest(request({ scope: { hosts: [] } }))), /1\.\.=64 hosts/);
  assert.throws(() => decodeCertRequest(encodeCertRequest(request({ scope: { hosts: uids(65) } }))), /1\.\.=64 hosts/);
  assert.equal(decodeCertRequest(encodeCertRequest(request({ scope: { hosts: uids(64) } }))).scope.hosts.length, 64);
  assert.throws(() => decodeCertRequest(concat(ok, Uint8Array.of(0))), /trailing or missing/);
  assert.throws(() => decodeCertRequest(ok.slice(0, ok.length - 1)), /trailing or missing/);
  assert.throws(() => decodeCertRequest(ok.slice(0, 12)), /truncated/);
  const badTag = ok.slice();
  badTag[15] = 7;
  assert.throws(() => decodeCertRequest(badTag), /unknown scope tag/);
  assert.throws(() => decodeCertRequest(encodeFrame(KIND.CHALLENGE, ok.slice(7))), /not a trust-session request/);
  const ext = (extra) => encodeCertExtendRequest({ cert_id: 'ab'.repeat(32), extra_hosts: extra });
  assert.throws(() => decodeCertExtendRequest(ext(0)), /1 to 50/);
  assert.throws(() => decodeCertExtendRequest(ext(51)), /1 to 50/);
  assert.equal(decodeCertExtendRequest(ext(50)).extra_hosts, 50);
  assert.throws(() => decodeCertExtendRequest(concat(ext(1), Uint8Array.of(0))), /36 bytes/);
});

test('courier lists: at most 8 ops, each at most 256 KiB, the list at most 1 MiB, both ways', () => {
  const op = (n) => ({ canonical: new Uint8Array(n), signature: new Uint8Array(64) });
  assert.equal(decodePhoneOps(encodePhoneOps(Array.from({ length: 8 }, () => op(10)))).length, MAX_COURIER_OPS);
  assert.throws(() => encodePhoneOps(Array.from({ length: 9 }, () => op(10))), /at most 8/);
  assert.equal(decodePhoneOps(encodePhoneOps([op(256 * 1024)]))[0].canonical.length, 256 * 1024);
  assert.throws(() => encodePhoneOps([op(256 * 1024 + 1)]), /does not fit/);
  assert.throws(() => encodePhoneOps(Array.from({ length: 5 }, () => op(256 * 1024))), /over 1 MiB/);
  // a decoder refuses a hostile frame the encoder would never write
  assert.throws(() => decodePhoneOps(encodeFrame(KIND.PHONE_OPS, Uint8Array.of(9))), /exceeds the cap of 8/);
  const big = encodeFrame(KIND.PHONE_OPS, Uint8Array.of(1, 0, 4, 0, 1), new Uint8Array(256 * 1024 + 1 + 64));
  assert.throws(() => decodePhoneOps(big), /oversize or truncated/);
  assert.throws(() => decodePhoneOps(encodeFrame(KIND.PHONE_OPS, new Uint8Array(1024 * 1024 + 1))), /frame cap/);
});

// ------------------------------------------------------------ CertIssue

test('the phone authors CertIssue with the request fields UNCHANGED, chained on its own chain', () => {
  const state = emptyState();
  const first = issueCert(state, phone, wire(request()), NOW);
  const op = parseOp(first.canonical);
  assert.equal(op.kind, 'CertIssue');
  assert.equal(op.device_id, phone.deviceId, 'the phone is the author');
  assert.equal(op.seq, 1);
  assert.deepEqual(op.fields, {
    device_id: DELEGATE, scope: { Hosts: [HOST_A, HOST_B] }, expiry: NOW + 7 * DAY, max_new_hosts_per_day: 2, epoch_lo: 0, epoch_hi: 1,
  });
  assert.ok(edVerify(phone.edPub, first.canonical, first.signature));
  assert.equal(first.entry.hash, opHashHex(first.canonical), 'the certificate id is the op hash');
  const second = issueCert(state, phone, wire(request({ scope: { group: '06'.repeat(16) } })), NOW + 5);
  const op2 = parseOp(second.canonical);
  assert.equal(op2.seq, 2);
  assert.equal(op2.prev_hash, first.entry.hash, 'chained on the first op');
  assert.deepEqual(op2.fields.scope, { Group: '06'.repeat(16) });
  assert.deepEqual(issuedCerts(state).map((c) => c.id), [first.entry.hash, second.entry.hash]);
  // it travels in a kind 5 list
  assert.deepEqual(decodePhoneOps(encodePhoneOps(courierOps(state))).map((o) => parseOp(o.canonical).kind), ['CertIssue', 'CertIssue']);
});

test('the request is described from its bytes: device, scope, term, budget', () => {
  const d = describeCertRequest(wire(request()), NOW, { [DELEGATE]: 'work laptop' });
  const all = d.rows.map((r) => `${r.label}=${r.value}`).join('\n');
  assert.ok(all.includes(`work laptop [${DELEGATE}]`), all);
  assert.ok(all.includes(HOST_A) && all.includes(HOST_B), 'every listed host is shown');
  assert.ok(all.includes('2 hosts only'));
  assert.ok(all.includes('7 days'), all);
  assert.ok(all.includes('New hosts per day=2'));
  assert.equal(d.delegateKnown, true);
  assert.equal(describeCertRequest(wire(request()), NOW, {}).delegateKnown, false);
  assert.ok(describeCertRequest(wire(request({ scope: { group: '06'.repeat(16) } })), NOW).rows.some((r) => r.value === '06'.repeat(16) && r.danger));
  assert.ok(d.rows.filter((r) => ['Trusted device', 'Term', 'New hosts per day'].includes(r.label)).every((r) => r.danger));
});

test('every limit of CertIssue is enforced before signing, and a refusal writes nothing', () => {
  const cases = [
    ['expiry is now', request({ expiry: BigInt(NOW) }), /not in the future/],
    ['expiry in the past', request({ expiry: BigInt(NOW - 5) }), /not in the future/],
    ['90 days and a second', request({ expiry: BigInt(NOW + 90 * DAY + 1) }), /longer than 90 days/],
    ['budget 0', request({ max_new_hosts_per_day: 0 }), /between 1 and 50/],
    ['budget 51', request({ max_new_hosts_per_day: 51 }), /between 1 and 50/],
    ['epochs inverted', request({ epoch_lo: 6n, epoch_hi: 5n }), /inverted/],
    ['names the phone itself', request({ device_id: phone.deviceId }), /does not delegate to itself/],
  ];
  for (const [name, r, re] of cases) {
    const state = emptyState();
    const req = wire(r);
    assert.match(certRequestProblems(req, NOW, phone.deviceId).join(' '), re, name);
    assert.throws(() => issueCert(state, phone, req, NOW), re, name);
    assert.equal(state.chain.length, 0, `${name}: nothing was signed`);
  }
  // the hand-built shapes the wire decoder never lets through are refused by the phone too
  const state = emptyState();
  for (const scope of [{ hosts: [] }, { hosts: Array.from({ length: 65 }, () => HOST_A) }]) {
    const r = { ...wire(request()), scope };
    assert.match(certRequestProblems(r, NOW, phone.deviceId).join(' '), /1 to 64 hosts/);
    assert.throws(() => issueCert(state, phone, r, NOW), /1 to 64 hosts/);
  }
  assert.equal(state.chain.length, 0);
  // the edges themselves are accepted
  for (const r of [request({ expiry: BigInt(NOW + 1) }), request({ expiry: BigInt(NOW + 90 * DAY) }), request({ max_new_hosts_per_day: 1 }), request({ max_new_hosts_per_day: 50 }), request({ epoch_lo: 0n, epoch_hi: 0n })]) {
    const s = emptyState();
    assert.doesNotThrow(() => issueCert(s, phone, wire(r), NOW));
    assert.equal(s.chain.length, 1);
  }
});

// ------------------------------------------------------------ CertRevoke

test('a trust session is revoked by a phone-signed CertRevoke', () => {
  const state = emptyState();
  const cert = issueCert(state, phone, wire(request()), NOW);
  const r = revokeCert(state, phone, cert.entry.hash, NOW + 100);
  const op = parseOp(r.canonical);
  assert.equal(op.kind, 'CertRevoke');
  assert.deepEqual(op.fields, { cert_id: cert.entry.hash });
  assert.equal(op.device_id, phone.deviceId);
  assert.ok(edVerify(phone.edPub, r.canonical, r.signature));
  assert.equal(issuedCerts(state)[0].revoked, true);
  assert.throws(() => revokeCert(state, phone, cert.entry.hash, NOW + 200), /already revoked/);
  assert.throws(() => revokeCert(state, phone, 'ee'.repeat(32), NOW), /did not issue/);
  assert.equal(state.chain.length, 2);
});

// ------------------------------------------------------------ CertExtend

test('CertExtend: phone-authored, one per certificate per rolling 24 hours, none after expiry', () => {
  const state = emptyState();
  const cert = issueCert(state, phone, wire(request()), NOW);
  const id = cert.entry.hash;
  const req = decodeCertExtendRequest(encodeCertExtendRequest({ cert_id: id, extra_hosts: 5 }));
  const e = extendCert(state, phone, req, NOW + 3600);
  const op = parseOp(e.canonical);
  assert.equal(op.kind, 'CertExtend');
  assert.deepEqual(op.fields, { cert_id: id, extra_hosts: 5 });
  assert.equal(op.device_id, phone.deviceId);
  assert.ok(edVerify(phone.edPub, e.canonical, e.signature));
  assert.equal(issuedCerts(state)[0].extensions.length, 1);
  // a second extension inside 24 hours is refused, with the time it becomes possible
  const again = extendProblems(state, req, NOW + 3600 + DAY - 1);
  assert.equal(again.ok, false);
  assert.match(again.problems.join(' '), /one per 24 hours/);
  assert.throws(() => extendCert(state, phone, req, NOW + 7200), /one per 24 hours/);
  assert.equal(state.chain.length, 2, 'the refusal wrote nothing');
  // 24 hours later it is possible again (rolling window)
  assert.equal(extendProblems(state, req, NOW + 3600 + DAY).ok, true);
  assert.doesNotThrow(() => extendCert(state, phone, req, NOW + 3600 + DAY));
  // not after expiry, not for an unknown or revoked certificate, not outside 1..=50
  assert.match(extendProblems(state, req, NOW + 7 * DAY).problems.join(' '), /expired/);
  assert.match(extendProblems(state, { cert_id: 'ee'.repeat(32), extra_hosts: 1 }, NOW).problems.join(' '), /did not issue/);
  assert.match(extendProblems(state, { cert_id: id, extra_hosts: 51 }, NOW + 5 * DAY).problems.join(' '), /1 to 50/);
  assert.match(extendProblems(state, { cert_id: id, extra_hosts: 0 }, NOW + 5 * DAY).problems.join(' '), /1 to 50/);
  revokeCert(state, phone, id, NOW + 5 * DAY);
  assert.match(extendProblems(state, req, NOW + 6 * DAY).problems.join(' '), /revoked/);
});

// ------------------------------------------------------------ reshare pinned to the Rust fixture

test('planReshare on the committed Rust reshare fixture reproduces the committed phone shares', () => {
  assert.equal(phone.deviceId, reshareFx.phone_pub.device_id, 'the fixture phone is the 0x42 phone');
  const ops = reshareFx.ops.map((o) => ({ canonical: h(o.canonical_hex), signature: h(o.sig_hex) }));
  const ring = Object.fromEntries(reshareFx.contributors.map((c) => [c.device_id, { name: `terminal ${c.device_id.slice(0, 4)}`, ed25519_pub: c.ed25519_pub }]));
  const plan = planReshare(phone, ops, reshareFx.epoch, { ring });
  assert.equal(plan.ok, true, plan.error);
  assert.deepEqual(plan.holders, reshareFx.new_holders.sorted_ids);
  assert.equal(plan.myIndex, reshareFx.new_holders.sorted_ids.indexOf(phone.deviceId) + 1);
  assert.equal(plan.items.length, Object.keys(reshareFx.expected_phone_shares).length);
  assert.deepEqual(plan.contributors.map((c) => [c.session, c.verified]), [[true, true], [true, true]]);
  const key = phone.unwrap(plan.storageSealed);
  for (const item of plan.items) {
    const want = reshareFx.expected_phone_shares[item.host_uid];
    assert.ok(want, `a committed share for ${item.host_uid}`);
    const plain = xchacha20poly1305(key, item.sealed_share.slice(0, 24), envelopeAad(h(item.host_uid), reshareFx.epoch, plan.myIndex)).decrypt(item.sealed_share.slice(24));
    assert.equal(plain[0], want.index);
    assert.equal(hexOf(plain.slice(1)), want.bytes_hex, `the new share of ${item.host_uid}`);
  }
  // the courier subset (what a terminal sends in one kind 5 list: the batches and this phone's sessions)
  const subset = ops.filter((o) => {
    const p = parseOp(o.canonical);
    return p.kind === 'ShareReshareBatch' || (p.kind === 'ShareReshareSession' && p.fields.to === phone.deviceId);
  });
  assert.ok(subset.length <= MAX_COURIER_OPS);
  assert.equal(decodeFrame(encodePhoneOps(subset)).kind, KIND.PHONE_OPS);
  assert.equal(planReshare(phone, subset, reshareFx.epoch, { ring }).ok, true);
  const batch = subset.map((o) => parseOp(o.canonical)).find((p) => p.kind === 'ShareReshareBatch');
  assert.equal(b64dec(batch.fields.items[0].commitments[0]).length, 32);
});

// ------------------------------------------------------------ the scan collector follows a document switch

test('the collector ignores a stray foreign frame, but follows another document that keeps arriving', () => {
  const doc = (n, seed) => Uint8Array.from({ length: n }, (_, i) => (i * 7 + seed) & 255);
  const A = doc(1300, 1);
  const B = doc(900, 2);
  const fa = fountainEncode(A, 84, 25);
  const fb = fountainEncode(B, 84, 25);
  assert.ok(fa.length > 12 && fb.length > 12);
  const have = (c) => c.progress().have;

  // one stray frame of B in the middle of A: progress is kept and A completes
  let c = new DocumentCollector();
  for (let i = 0; i < 5; i++) assert.equal(c.push(fa[i]), null);
  const kept = have(c);
  assert.equal(c.push(fb[0]), null);
  assert.equal(have(c), kept, 'a stray foreign frame does not reset progress');
  let got = null;
  for (let i = 5; i < fa.length && !got; i++) got = c.push(fa[i]);
  assert.deepEqual(got, A);

  // B alternating with A never counts as a switch (the frames are not consecutive)
  c = new DocumentCollector();
  got = null;
  for (let i = 0; i < fa.length && !got; i++) {
    got = c.push(fa[i]);
    if (!got) c.push(fb[i % fb.length]);
  }
  assert.deepEqual(got, A, 'alternating frames keep following the first document');

  // the terminal switches to B while the phone holds part of A: after SWITCH_AFTER_FRAMES of B it follows B
  c = new DocumentCollector();
  for (let i = 0; i < 8; i++) c.push(fa[i]);
  assert.ok(have(c) > 0);
  for (let i = 0; i < SWITCH_AFTER_FRAMES - 1; i++) assert.equal(c.push(fb[i]), null);
  assert.ok(have(c) > 0, 'still holding the first document before the threshold');
  got = null;
  for (let i = SWITCH_AFTER_FRAMES - 1; i < fb.length * 2 && !got; i++) got = c.push(fb[i % fb.length]);
  assert.deepEqual(got, B, 'the new document is followed and completes');

  // a completed document followed by another one: the next document is taken up
  c = new DocumentCollector();
  got = null;
  for (let i = 0; i < fa.length && !got; i++) got = c.push(fa[i]);
  assert.deepEqual(got, A);
  got = null;
  for (let i = 0; i < fb.length * 2 && !got; i++) got = c.push(fb[i % fb.length]);
  assert.deepEqual(got, B);

  // a short foreign document (one lap shorter than the threshold) is taken up after one lap of its frames
  const small = fountainEncode(doc(100, 3), 84, 25);
  assert.ok(small.length < SWITCH_AFTER_FRAMES);
  c = new DocumentCollector();
  for (let i = 0; i < 4; i++) c.push(fa[i]);
  got = null;
  for (let i = 0; i < small.length * 2 && !got; i++) got = c.push(small[i % small.length]);
  assert.deepEqual(got, doc(100, 3), 'a one-frame document is read even while another is partial');
});
