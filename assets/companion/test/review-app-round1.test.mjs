// Review round 1 of the phone page (host-sync-plan/review/app-page-round1.md): the reviewer's reproductions, kept as
// permanent tests (each failed while its finding was open), plus the checks of the fixes and of the suspicions.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { bytesOf, concat, edSign, hexOf, hybridWrap, parseJSONExact, utf8 } from '../src/lib.mjs';
import { PIN_MIN_CHARS, identityOfSecret, pinDelaySecs, unwrapSecret, wrapSecret } from '../src/keys.mjs';
import { approveRelease, evaluateChallenge } from '../src/release.mjs';
import { certRequestProblems, describeCertRequest, issueCert } from '../src/cert.mjs';
import { describeOp, parseOp, safeText } from '../src/ops.mjs';
import { planReshare } from '../src/reshare.mjs';
import { HEAD_PENDING_MESSAGE, adoptOwnChain, appendHeadAttest, confirmNoChain, courierOps, emptyState, issueSnapshot, recordSnapshot, restoredState } from '../src/state.mjs';
import { parseReveal, revealBytes } from '../src/frames.mjs';
import { snapshotOpen } from '../src/quorum.mjs';
import { scenario as buildScenario } from './reshare-scenario.mjs';

const rnd = (n, fill) => new Uint8Array(n).fill(fill);
const u16 = (n) => Uint8Array.of((n >> 8) & 255, n & 255);
const u32 = (n) => Uint8Array.of((n >>> 24) & 255, (n >>> 16) & 255, (n >>> 8) & 255, n & 255);
const be = (n, len) => {
  const b = new Uint8Array(len);
  let v = BigInt(n);
  for (let i = len - 1; i >= 0; i--) {
    b[i] = Number(v & 255n);
    v >>= 8n;
  }
  return b;
};
const NOW = 1_900_000_000;
const HOST = 'ab'.repeat(16);
const phone = identityOfSecret(rnd(32, 7));
const requester = identityOfSecret(rnd(32, 9));

// A challenge frame the way share_release.rs / wire.rs lay it out (kind 1).
function challenge({ op, opSig = new Uint8Array(64), expiry = NOW + 100, requestId = 7, hostHex = HOST, share = rnd(33, 5) }) {
  const plain = concat(Uint8Array.from(hostHex.match(/../g), (b) => parseInt(b, 16)), Uint8Array.of(1), share.slice(1));
  const blob = hybridWrap({ x25519_pub: phone.xPub, mlkem_pub: phone.mlkemPub }, plain, rnd(32, 3), rnd(24, 4), rnd(32, 6));
  const material = concat(Uint8Array.of(0), be(3, 8), u16(blob.length), blob, u16(0));
  const opBytes = typeof op === 'string' ? utf8(op) : op;
  return concat(
    utf8('SWR1'), u16(1), Uint8Array.of(1),
    rnd(32, 1), be(expiry, 8), rnd(32, 2), be(requestId, 8), u32(opBytes.length), opBytes, opSig, u32(material.length), material,
  );
}
const shareReleaseOp = ({ device = requester.deviceId, lamport = 5, epoch = 3, ts = NOW - 10, reqId = '7', req = device } = {}) =>
  `{"device_id":"${device}","seq":1,"lamport":${lamport},"prev_hash":"${'0'.repeat(64)}","timestamp":${ts},"epoch":${epoch},"alg":[1,1],"body":{"ShareRelease":{"host_uid":"${HOST}","request_id":${reqId},"requester":"${req}"}}}`;
const ctx = (ring = {}) => ({ now: NOW, identity: phone, ring, aliases: {} });

// ---------------------------------------------------------------- F1: the parser terminates on every malformed input
test('F1 parseOp terminates on a malformed op (child process, 3 s budget)', () => {
  const code = `
    import { parseOp } from './src/ops.mjs';
    try { parseOp(new TextEncoder().encode('{x}')); } catch (e) { console.log('threw', e.message); }
  `;
  const r = spawnSync(process.execPath, ['--max-old-space-size=256', '--input-type=module', '-e', code], { cwd: new URL('..', import.meta.url).pathname, timeout: 3000, encoding: 'utf8' });
  assert.notEqual(r.error && r.error.code, 'ETIMEDOUT', 'parseOp must return');
  assert.equal(r.status, 0);
  assert.match(r.stdout, /threw/);
});

test('F1 parseJSONExact refuses every malformed input, each in a bounded time', () => {
  const bad = [
    '{x}', '{"a":"', '{"a', '{', '[', '[1,', '{"a":', '{"a":1', '{"a":1,', '"', '"\\', '"\\u12', '"\\u12G4"', '"\\x"', '"\\ud800"', '"\\udc00"', '"\\ud800\\u0041"',
    '{"a":1,}', '{,}', '{"a" 1}', '-', '-x', '1.', 'tru', 'nul', '', '   ', '{"a":1}{', '{"__proto__":{}}', '"\u0001"',
    '[' .repeat(40) + ']'.repeat(40), '18446744073709551616', '-9223372036854775809',
  ];
  for (const b of bad) {
    const t0 = Date.now();
    assert.throws(() => parseJSONExact(b), undefined, JSON.stringify(b));
    assert.ok(Date.now() - t0 < 500, 'bounded');
  }
  assert.equal(parseJSONExact('"\\ud83d\\ude00"'), '\u{1f600}');
  assert.equal(parseJSONExact('18446744073709551615'), 18446744073709551615n);
  assert.equal(parseJSONExact('-9223372036854775808'), -9223372036854775808n);
});

test('F1 parseOp caps the op at MAX_COURIER_OP_BYTES', () => {
  const base = `{"device_id":"${requester.deviceId}","seq":1,"lamport":1,"prev_hash":"${'0'.repeat(64)}","timestamp":1,"epoch":1,"alg":[1,1],"body":{"HostUpsert":{"host_uid":"${HOST}","fields":{"notes":"`;
  assert.throws(() => parseOp(utf8(`${base}${'x'.repeat(256 * 1024)}"}}}}`)), /larger than/);
});

// ---------------------------------------------------------------- F2: integers stay inside the Rust field types
test('F2 a lamport at u64::MAX from an unknown requester is not adopted into the phone chain', () => {
  const op = shareReleaseOp({ lamport: '18446744073709551615', epoch: '18446744073709551615' });
  const ev = evaluateChallenge(challenge({ op }), ctx());
  assert.equal(ev.ok, true);
  assert.equal(ev.signatureVerified, false);
  const state = emptyState();
  const out = approveRelease(ev, { identity: phone, state, now: NOW });
  const mine = parseOp(out.chainOp.canonical);
  assert.ok(BigInt(mine.lamport) <= 18446744073709551615n);
  assert.equal(String(mine.lamport), '1', 'the unverified op moved nothing');
  assert.equal(String(mine.epoch), '0');
});

test('F2 a VERIFIED op at u64::MAX cannot push the chain over u64: the phone refuses to sign', () => {
  const ring = { [requester.deviceId]: { name: 'laptop', ed25519_pub: hexOf(requester.edPub) } };
  const opText = shareReleaseOp({ lamport: '18446744073709551615' });
  const ev = evaluateChallenge(challenge({ op: opText, opSig: edSign(requester.edSeed, utf8(opText)) }), ctx(ring));
  assert.equal(ev.ok, true);
  assert.equal(ev.signatureVerified, true);
  const state = emptyState();
  assert.throws(() => approveRelease(ev, { identity: phone, state, now: NOW }), /u64/);
  assert.equal(state.chain.length, 0);
});

test('F2b parseOp rejects integers outside the Rust field types', () => {
  const mk = (k, v) => utf8(`{"device_id":"${requester.deviceId}","seq":${k === 'seq' ? v : 1},"lamport":${k === 'lamport' ? v : 1},"prev_hash":"${'0'.repeat(64)}","timestamp":${k === 'timestamp' ? v : 1},"epoch":${k === 'epoch' ? v : 0},"alg":[1,1],"body":{"HeadAttest":{"device_id":"${requester.deviceId}","seq":1,"chain_hash":"${'0'.repeat(64)}"}}}`);
  for (const k of ['seq', 'lamport', 'epoch']) for (const bad of ['-1', '18446744073709551616']) assert.throws(() => parseOp(mk(k, bad)), undefined, `${k} ${bad}`);
  for (const bad of ['9223372036854775808', '-9223372036854775809']) assert.throws(() => parseOp(mk('timestamp', bad)), undefined, `timestamp ${bad}`);
  parseOp(mk('seq', '18446744073709551615'));
  parseOp(mk('timestamp', '-9223372036854775808'));
});

test('S6 parseOp refuses lone surrogates and integers above u64 anywhere in a body', () => {
  const op = (notes) => utf8(`{"device_id":"${requester.deviceId}","seq":1,"lamport":1,"prev_hash":"${'0'.repeat(64)}","timestamp":1,"epoch":1,"alg":[1,1],"body":{"HostUpsert":{"host_uid":"${HOST}","fields":{"notes":${notes}}}}}`);
  assert.throws(() => parseOp(op('"\\ud800"')));
  assert.throws(() => parseOp(op('18446744073709551616')));
});

// ---------------------------------------------------------------- F3: a restored key does not fork its chain
test('F3 a restored key does not author until it has its chain head; the head comes from a terminal', () => {
  const secret = rnd(32, 11);
  const id = identityOfSecret(secret);
  const before = emptyState();
  appendHeadAttest(before, id, NOW, null);
  appendHeadAttest(before, id, NOW + 1, null);
  const restored = restoredState();
  assert.throws(() => appendHeadAttest(restored, identityOfSecret(secret), NOW + 5, null), /restored from its 24 words/);
  assert.equal(restored.chain.length, 0, 'nothing was signed');
  assert.match(HEAD_PENDING_MESSAGE, /must not sign yet/);
  // the terminal hands back this phone's own ops: the chain continues after them
  const ops = courierOps(before);
  assert.throws(() => adoptOwnChain(restored, identityOfSecret(secret), []), /holds no op/);
  const other = identityOfSecret(rnd(32, 12));
  assert.throws(() => adoptOwnChain(restoredState(), other, ops), /not by this phone/);
  const forged = ops.map((o) => ({ ...o, signature: new Uint8Array(64) }));
  assert.throws(() => adoptOwnChain(restoredState(), id, forged), /does not verify/);
  assert.throws(() => adoptOwnChain(restoredState(), id, [ops[1]].concat(ops[1])), /one chain|nothing/);
  const r = adoptOwnChain(restored, identityOfSecret(secret), ops);
  assert.equal(r.head_seq, '2');
  const next = appendHeadAttest(restored, identityOfSecret(secret), NOW + 9, null);
  const op = parseOp(next.canonical);
  assert.equal(String(op.seq), '3');
  assert.equal(op.prev_hash, before.chain[1].hash, 'continues the old chain: no fork');
});

test('F3 a key that never signed anything can say so, and then authors from seq 1', () => {
  const s = restoredState();
  const id = identityOfSecret(rnd(32, 13));
  assert.throws(() => appendHeadAttest(s, id, NOW, null), /must not sign yet/);
  confirmNoChain(s);
  assert.equal(String(parseOp(appendHeadAttest(s, id, NOW, null).canonical).seq), '1');
});

// ---------------------------------------------------------------- F4: the blob names its credential
test('F4 the short-code blob carries the host_uid in its header (associated data); the phone rule stays', async () => {
  const profiles = [{ mem_kib: 64, iters: 1 }];
  const s = emptyState();
  const first = await issueSnapshot(s, HOST, 1, rnd(32, 5), NOW, { profiles });
  recordSnapshot(s, HOST, first.record);
  assert.equal(first.blob[4], 2, 'header version 2');
  assert.equal(hexOf(first.blob.slice(37, 53)), HOST, 'host_uid(16) follows the salt');
  await assert.rejects(() => issueSnapshot(s, HOST, 1, rnd(32, 5), NOW + 1, { profiles }), /already live/);
  const ok = await snapshotOpen(first.blob, first.code, NOW + 2, bytesOf(HOST));
  assert.equal(ok.ok, true);
  assert.equal((await snapshotOpen(first.blob, first.code, NOW + 2, bytesOf('cd'.repeat(16)))).ok, false, 'a blob does not open for another credential');
  const relabelled = first.blob.slice();
  relabelled[40] ^= 1; // the host_uid is inside the associated data
  assert.equal((await snapshotOpen(relabelled, first.code, NOW + 2)).ok, false);
});

// ---------------------------------------------------------------- F5: hostile input ends on a refusal
test('F5a evaluateChallenge returns a refusal for a non-numeric request_id', () => {
  const op = shareReleaseOp({ reqId: '"abc"' });
  const ev = evaluateChallenge(challenge({ op }), ctx());
  assert.equal(ev.ok, false);
});

test('F5a2 a string request_id is not accepted as a u64', () => {
  const ev = evaluateChallenge(challenge({ op: shareReleaseOp({ reqId: '""' }), requestId: 0 }), ctx());
  assert.equal(ev.ok, false);
});

test('F5b describeCertRequest survives a far-future expiry', () => {
  const req = { device_id: 'aa'.repeat(8), scope: { group: 'cd'.repeat(16) }, expiry: 9_000_000_000_000_000n, max_new_hosts_per_day: 5, epoch_lo: 0n, epoch_hi: 0n };
  assert.ok(certRequestProblems(req, NOW, phone.deviceId).length > 0);
  assert.doesNotThrow(() => describeCertRequest(req, NOW, {}));
});

test('F5c planReshare refuses (does not throw) on malformed ops and malformed batch fields', () => {
  const hdr = (body) => utf8(`{"device_id":"${requester.deviceId}","seq":1,"lamport":1,"prev_hash":"${'0'.repeat(64)}","timestamp":1,"epoch":1,"alg":[1,1],"body":${body}}`);
  const cases = {
    'unknown kind': hdr('{"Nope":{}}'),
    'items not a list': hdr(`{"ShareReshareBatch":{"epoch":1,"new_holders":["${phone.deviceId}"],"items":5}}`),
    'new_holders not a list': hdr('{"ShareReshareBatch":{"epoch":1,"new_holders":5,"items":[]}}'),
    'unterminated': utf8('{"a":"'),
  };
  for (const [name, canonical] of Object.entries(cases)) {
    const r = planReshare(phone, [{ canonical, signature: new Uint8Array(64) }], 1n, { ring: {} });
    assert.equal(r.ok, false, name);
  }
});

// ---------------------------------------------------------------- F6: the signed op's age is judged
test('F6 a year-old signed ShareRelease inside a fresh challenge is refused', () => {
  const ring = { [requester.deviceId]: { name: 'laptop', ed25519_pub: hexOf(requester.edPub) } };
  const opText = shareReleaseOp({ ts: NOW - 365 * 86400 });
  const ev = evaluateChallenge(challenge({ op: opText, opSig: edSign(requester.edSeed, utf8(opText)) }), ctx(ring));
  assert.equal(ev.ok, false);
  assert.match(ev.refusal, /minutes old/);
});

test('F6 the expiry must sit 0 to 720 s after the signed op, a future op is refused, a stale one is flagged', () => {
  const ring = { [requester.deviceId]: { name: 'laptop', ed25519_pub: hexOf(requester.edPub) } };
  const run = (opText, expiry) => evaluateChallenge(challenge({ op: opText, opSig: edSign(requester.edSeed, utf8(opText)), expiry }), ctx(ring));
  assert.equal(run(shareReleaseOp({ ts: NOW + 50 }), NOW + 100).ok, true);
  assert.equal(run(shareReleaseOp({ ts: NOW + 101 }), NOW + 100).ok, false, 'expiry before the op');
  assert.equal(run(shareReleaseOp({ ts: NOW + 601 }), NOW + 700).ok, false, 'dated in the future');
  const stale = run(shareReleaseOp({ ts: NOW - 300 }), NOW + 100);
  assert.equal(stale.ok, true, 'inside the skew window');
  assert.equal(stale.description.rows.find((r) => r.label === 'Op time').danger, true, 'but flagged');
  assert.equal(run(shareReleaseOp({ ts: NOW - 10 }), NOW + 100).description.rows.find((r) => r.label === 'Op time').danger, false);
  assert.equal(run(shareReleaseOp({ ts: NOW - 10 }), NOW + 100 + 0).ok, true);
  assert.equal(run(shareReleaseOp({ ts: NOW - 700 }), NOW + 20).ok, true, 'exactly 720 s from op to expiry');
  assert.equal(run(shareReleaseOp({ ts: NOW - 701 }), NOW + 20).ok, false, 'more than 120 s plus the skew from op to expiry');
  assert.equal(run(shareReleaseOp({ ts: NOW - 721 }), NOW + 1).ok, false, 'more than 120 s plus the skew old');
});

// ---------------------------------------------------------------- F7: the epoch range is judged and flagged
test('F7 an unbounded epoch range is refused and the row is highlighted', () => {
  const req = { device_id: 'aa'.repeat(8), scope: { hosts: ['cd'.repeat(16)] }, expiry: BigInt(NOW + 86400), max_new_hosts_per_day: 5, epoch_lo: 0n, epoch_hi: 18446744073709551615n };
  const problems = certRequestProblems(req, NOW, phone.deviceId, '0');
  assert.ok(problems.some((p) => /wider than a terminal asks for/.test(p)));
  assert.equal(describeCertRequest(req, NOW, {}).rows.find((r) => r.label.startsWith('Phone epochs')).danger, true);
});

test('F7 the range must start at this phone\'s own epoch and be at most one wider', () => {
  const req = (lo, hi) => ({ device_id: 'aa'.repeat(8), scope: { hosts: ['cd'.repeat(16)] }, expiry: BigInt(NOW + 86400), max_new_hosts_per_day: 5, epoch_lo: lo, epoch_hi: hi });
  assert.deepEqual(certRequestProblems(req(3n, 4n), NOW, phone.deviceId, '3'), []);
  assert.deepEqual(certRequestProblems(req(3n, 3n), NOW, phone.deviceId, '3'), []);
  assert.match(certRequestProblems(req(0n, 1n), NOW, phone.deviceId, '3').join(' '), /this phone is at epoch 3/);
  assert.match(certRequestProblems(req(3n, 5n), NOW, phone.deviceId, '3').join(' '), /wider/);
  const s = emptyState();
  s.epoch = '3';
  assert.throws(() => issueCert(s, phone, req(0n, 1n), NOW), /epoch/);
  assert.equal(s.chain.length, 0);
});

// ---------------------------------------------------------------- F8: labels are sanitised, unknown keys are dangerous
test('F8 a bidi or invisible character in an op key does not reach a row label raw', () => {
  const op = utf8(`{"device_id":"${requester.deviceId}","seq":1,"lamport":1,"prev_hash":"${'0'.repeat(64)}","timestamp":1,"epoch":1,"alg":[1,1],"body":{"HostUpsert":{"host_uid":"${HOST}","fields":{"add\u2060ress":"evil.example","\u202eaddress":"x","name":"ok"}}}}`);
  const d = describeOp(parseOp(op), {});
  const labels = d.rows.map((r) => r.label);
  assert.ok(labels.every((l) => !/[\u2060\u202e]/.test(l)), JSON.stringify(labels));
  assert.ok(labels.some((l) => l.includes('\\u2060')));
  const byLabel = Object.fromEntries(d.rows.map((r) => [r.label, r.danger]));
  assert.equal(byLabel['fields.add\\u2060ress'], true, 'an unknown key is dangerous');
  assert.equal(byLabel['fields.\\u202eaddress'], true);
  assert.equal(byLabel['fields.name'], false, 'a known harmless field is not');
});

test('F8 safeText covers the Cc, Cf, Zl and Zp classes', () => {
  for (const c of ['\u0000', '\u007f', '\u00ad', '\u2060', '\u2061', '\u200b', '\u202e', '\u2066', '\ufeff', '\u2028', '\u2029', '\u180e', '\ufe0f', '\u034f', '\u{e0041}']) {
    const out = safeText(`a${c}b`);
    assert.ok(!out.includes(c), JSON.stringify(c));
    assert.match(out, /^a\\u/);
  }
  assert.equal(safeText('plain text 1-2'), 'plain text 1-2');
});

// ---------------------------------------------------------------- F9: PIN floor, wipe, attempt delay
test('F9 the PIN floor is 10 characters, every secret is zeroed on wipe, attempts get a growing delay', async () => {
  assert.ok(PIN_MIN_CHARS >= 10);
  assert.deepEqual([1, 2, 3, 4, 5, 9, 20].map(pinDelaySecs), [0, 0, 5, 10, 20, 320, 3600]);
  const secret = rnd(32, 4);
  const id = identityOfSecret(secret.slice(), 2);
  const pieces = [id.secret, id.edSeed, id.xSecret, id.mlkemDk];
  assert.ok(pieces.every((p) => p.some((b) => b !== 0)));
  id.wipe();
  assert.ok(pieces.every((p) => p.every((b) => b === 0)), 'secret, signing seed, current wrap secrets are zero');
  const blob = await wrapSecret(rnd(32, 4), '123456abcdef', { mem_kib: 64, iters: 1 });
  await assert.rejects(unwrapSecret(blob, 'x'.repeat(12)), /wrong PIN/);
});

// ---------------------------------------------------------------- F11: the committed page equals its fingerprint
test('F11 the fingerprint is the sha256 of the committed page with the fingerprint zeroed', () => {
  const html = readFileSync(new URL('../sshub-companion.html', import.meta.url), 'utf8');
  const fp = readFileSync(new URL('../FINGERPRINT', import.meta.url), 'utf8').trim();
  assert.match(fp, /^[0-9a-f]{64}$/);
  assert.equal(html.split(fp).length - 1, 1, 'the fingerprint appears exactly once');
  assert.equal(createHash('sha256').update(html.replace(fp, '0'.repeat(64))).digest('hex'), fp);
});

// ---------------------------------------------------------------- S1, S2, S5
test('S1 an unknown requester is flagged for the typed confirmation', () => {
  const ev = evaluateChallenge(challenge({ op: shareReleaseOp() }), ctx());
  assert.equal(ev.ok, true);
  assert.equal(ev.unknownRequester, true);
  assert.equal(ev.requester, requester.deviceId);
  const ring = { [requester.deviceId]: { name: 'laptop', ed25519_pub: hexOf(requester.edPub) } };
  const op = shareReleaseOp();
  assert.equal(evaluateChallenge(challenge({ op, opSig: edSign(requester.edSeed, utf8(op)) }), ctx(ring)).unknownRequester, false);
});

test('S2 two batches written by ONE device are not two contributors', () => {
  const sc = buildScenario({ phone, oneAuthor: true });
  const r = planReshare(phone, sc.signed, 1, { ring: sc.ring });
  assert.equal(r.ok, false, 'one author with two from-bytes must not combine a share');
  const honest = buildScenario({ phone });
  assert.equal(planReshare(phone, honest.signed, 1, { ring: honest.ring }).ok, true);
  const plan = planReshare(phone, honest.signed, 1, { ring: {} });
  assert.equal(plan.ok, true);
  assert.ok(plan.contributors.every((c) => c.known === false), 'contributors the phone does not know are marked');
  assert.equal(plan.seen.lamport, 0n, 'counters are not adopted from unverified ops');
});

test('S5 parseReveal accepts only 0 and 1 for the attended flag', () => {
  const rev = { ed25519_pub: rnd(32, 1), x25519_pub: rnd(32, 2), mlkem_pub: rnd(1184, 3), eph: rnd(32, 4), nonce: rnd(32, 5), name: 'n', attended: true };
  const b = revealBytes(rev);
  assert.equal(parseReveal(b).attended, true);
  const two = b.slice();
  two[two.length - 1] = 2;
  assert.throws(() => parseReveal(two), /attended/);
});

// ---------------------------------------------------------------- sound checks from the review (must stay green)
test('sound: duplicate keys, trailing bytes, escapes, whitespace, proto keys, unknown kind are refused', () => {
  const base = `{"device_id":"${requester.deviceId}","seq":1,"lamport":1,"prev_hash":"${'0'.repeat(64)}","timestamp":1,"epoch":1,"alg":[1,1],"body":`;
  const ok = `${base}{"HeadAttest":{"device_id":"${requester.deviceId}","seq":1,"chain_hash":"${'0'.repeat(64)}"}}}`;
  parseOp(utf8(ok));
  const bad = [
    ok.replace('"seq":1,"lamport"', '"seq":1,"seq":2,"lamport"'),
    `${ok} `,
    `${ok}\n`,
    ok.replace('"seq":1,', '"seq": 1,'),
    ok.replace('"alg":[1,1]', '"alg":[1,1.0]'),
    ok.replace('"timestamp":1', '"timestamp":01'),
    ok.replace('"timestamp":1', '"timestamp":1e0'),
    ok.replace('"epoch":1', '"epoch":1,"__proto__":{"x":1}'),
    ok.replace('HeadAttest', 'H\\u0065adAttest'),
    ok.replace('"chain_hash"', '"chain_hash":"x","chain_hash"'),
    ok.replace('"body":{"HeadAttest"', '"body":{"Nope":{},"HeadAttest"'),
  ];
  for (const b of bad) assert.throws(() => parseOp(utf8(b)), undefined, b.slice(0, 80));
  assert.throws(() => parseOp(Uint8Array.of(0xff, 0xfe)));
});

test('sound: expiry window, kind, requester mismatch, wrong material host', () => {
  const op = shareReleaseOp();
  assert.equal(evaluateChallenge(challenge({ op, expiry: NOW - 1 }), ctx()).ok, false);
  assert.equal(evaluateChallenge(challenge({ op, expiry: NOW + 721 }), ctx()).ok, false);
  assert.equal(evaluateChallenge(challenge({ op: shareReleaseOp({ req: 'ee'.repeat(8) }) }), ctx()).ok, false);
  const frame = challenge({ op });
  // flip the host inside the op: material was wrapped for HOST
  const swapped = challenge({ op: op.replace(HOST, 'cd'.repeat(16)) });
  assert.equal(evaluateChallenge(swapped, ctx()).ok, false);
  assert.equal(evaluateChallenge(frame, ctx()).ok, true);
});

