// Phase B, U13-3 revised: CertRenew on the phone page (contract 4j, design 6.2). Mirrors the renew tests in
// crates/sshub-phone-core/src/cert/tests.rs. Run in the tools container:
//   cd /work/assets/companion && NODE_PATH=/opt/node/lib/node_modules node --test test/cert-renew.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { bytesOf } from '../src/lib.mjs';
import { encodeCertRenew, parseOp } from '../src/ops.mjs';
import { identityOfSecret } from '../src/keys.mjs';
import { appendCertRenew, courierOps, emptyState, restoredState } from '../src/state.mjs';
import { decodeCertRenewRequest, encodeCertRenewRequest } from '../src/frames.mjs';
import { buildCertRenew, describeCert, describeCertRenew, extendProblems, issueCert, issuedCerts, revokeCert } from '../src/cert.mjs';

const finalVectors = JSON.parse(readFileSync(new URL('../../../tests/vectors/op/ops.json', import.meta.url), 'utf8'));
const h = (x) => bytesOf(x);
const phone = identityOfSecret(h('42'.repeat(32)));
const DELEGATE = identityOfSecret(h('71'.repeat(32))).deviceId;
const NOW = 1893452400;
const DAY = 86400;
const TERM = '33'.repeat(8);

function issued(termDays) {
  const s = emptyState();
  issueCert(s, phone, { device_id: DELEGATE, scope: { hosts: ['0a'.repeat(16), '0b'.repeat(16)] }, expiry: BigInt(NOW + termDays * DAY), max_new_hosts_per_day: 2, epoch_lo: 0n, epoch_hi: 1n }, NOW);
  return s;
}
const req = (s, expiry, ev) => ({ cert_id: issuedCerts(s)[0].id, expiry: BigInt(expiry), evidence: ev.map(([t, ts]) => ({ terminal: t, head_seq: 5n, head_ts: BigInt(ts) })) });
const renew = (s, expiry, ev, now) => buildCertRenew(s, phone, req(s, expiry, ev), now);
const lastKind = (s) => parseOp(h(s.chain[s.chain.length - 1].canonical)).kind;

test('kind 11 frame round trip and refusals; the CertRenew encoder reproduces the final vector byte for byte', () => {
  const r = { cert_id: 'ab'.repeat(32), expiry: 1800000000n, evidence: [{ terminal: TERM, head_seq: 7n, head_ts: 1799999000n }] };
  assert.deepEqual(decodeCertRenewRequest(encodeCertRenewRequest(r)), r);
  const bytes = encodeCertRenewRequest(r);
  assert.throws(() => decodeCertRenewRequest(bytes.slice(0, bytes.length - 1)));
  assert.throws(() => decodeCertRenewRequest(Uint8Array.of(...bytes, 0)));
  const v = finalVectors.vectors.find((x) => x.id === 'cert-renew');
  const op = parseOp(h(v.canonical_hex));
  const header = { device_id: op.device_id, seq: op.seq, lamport: op.lamport, prev_hash: op.prev_hash, timestamp: op.timestamp, epoch: op.epoch };
  assert.equal(Buffer.from(encodeCertRenew(header, op.fields.cert_id, op.fields.expiry)).toString('hex'), v.canonical_hex);
});

test('a renewal with fresh evidence extends the end that counts', () => {
  const s = issued(7);
  const now = NOW + 6 * DAY;
  const r = req(s, NOW + 14 * DAY, [[TERM, now - 3600]]);
  const v = describeCertRenew(s, r, now);
  assert.equal(v.refusal, null);
  assert.deepEqual([v.term_secs, v.budget, v.renewals_left, v.new_end, v.evidence_age_secs], [7 * DAY, 2, 2, NOW + 14 * DAY, 3600]);
  assert.match(v.delegate, new RegExp(DELEGATE));
  assert.equal(v.scope, '2 listed hosts');
  buildCertRenew(s, phone, r, now);
  assert.equal(lastKind(s), 'CertRenew');
  const c = issuedCerts(s)[0];
  assert.deepEqual([c.effectiveExpiry, c.renewals.length], [BigInt(NOW + 14 * DAY), 1]);
  assert.match(describeCert(c), /renewed 1x/);
  assert.equal(extendProblems(s, { cert_id: c.id, extra_hosts: 1 }, NOW + 10 * DAY).ok, true, 'alive after the first end date');
  assert.equal(describeCertRenew(s, req(s, NOW + 21 * DAY, [[TERM, now]]), now).renewals_left, 1);
});

test('stale evidence, or a list that came only through the delegate, is refused', () => {
  const now = NOW + 6 * DAY;
  const end = NOW + 14 * DAY;
  let s = issued(7);
  assert.throws(() => renew(s, end, [[TERM, now - 6 * 3600 - 1]], now), /fresher than 6 h/);
  renew(s, end, [[TERM, now - 6 * 3600]], now);
  s = issued(7);
  assert.throws(() => renew(s, end, [], now), /only through the trusted device/);
  assert.throws(() => renew(s, end, [[DELEGATE, now]], now), /only through the trusted device/);
  assert.equal(describeCertRenew(s, req(s, end, [[TERM, now - 30 * DAY], ['34'.repeat(8), now - 60]]), now).evidence_age_secs, 60);
  assert.equal(s.chain.length, 1, 'refusals sign nothing');
});

test('the caps hold: two renewals, the first term, 180 days', () => {
  let s = issued(7);
  const now = NOW + DAY;
  const ev = [[TERM, now]];
  assert.throws(() => renew(s, NOW + 14 * DAY + 1, ev, now), /at most as long as the first term/);
  assert.throws(() => renew(s, NOW + 7 * DAY, ev, now), /not later than the current one/);
  assert.throws(() => renew(s, NOW + DAY, ev, now), /not later than the current one/);
  renew(s, NOW + 14 * DAY, ev, now);
  renew(s, NOW + 21 * DAY, ev, now);
  assert.throws(() => renew(s, NOW + 28 * DAY, ev, now), /renewed 2 times already/);
  assert.equal(describeCertRenew(s, req(s, NOW + 28 * DAY, ev), now).renewals_left, 0);
  s = issued(90);
  renew(s, NOW + 180 * DAY, ev, now);
  assert.throws(() => renew(s, NOW + 180 * DAY + 1, ev, now), /180 days|first term/);
});

test('a renewal the host would not count is not counted by the page either', () => {
  const s = issued(7);
  const now = NOW;
  // a modified phone wrote a renewal far past the first term: it does not move the end date
  buildCertRenewUnchecked(s, NOW + 30 * DAY, now);
  const c = issuedCerts(s)[0];
  assert.deepEqual([c.effectiveExpiry, c.renewals.length], [BigInt(NOW + 7 * DAY), 0]);
});

function buildCertRenewUnchecked(s, expiry, now) {
  appendCertRenew(s, phone, issuedCerts(s)[0].id, expiry, now, null);
}

test('a revoked, unknown or unusable state is refused; a past epoch_hi is shown refused at use', () => {
  const now = NOW + DAY;
  const ev = [[TERM, now]];
  let s = issued(7);
  const r = req(s, NOW + 14 * DAY, ev);
  revokeCert(s, phone, r.cert_id, now);
  assert.throws(() => buildCertRenew(s, phone, r, now), /revoked/);
  s = issued(7);
  const unknown = { ...req(s, NOW + 14 * DAY, ev), cert_id: '09'.repeat(32) };
  assert.match(describeCertRenew(s, unknown, now).refusal, /did not issue/);
  assert.throws(() => buildCertRenew(s, phone, unknown, now));
  s = issued(7);
  const r2 = req(s, NOW + 14 * DAY, ev);
  s.headPending = true;
  assert.throws(() => buildCertRenew(s, phone, r2, now), /chain/);
  s = issued(7);
  s.epoch = '5';
  assert.match(describeCertRenew(s, req(s, NOW + 14 * DAY, ev), now).refusal, /refused at use/);
  assert.ok(courierOps(restoredState()).length === 0);
});
