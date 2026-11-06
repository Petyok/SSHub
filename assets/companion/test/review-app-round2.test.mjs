// Round 2 review reproductions for the phone page (host-sync-plan/review/app-page-round2.md).
// Every test marked FAILS asserts the safe behaviour and fails while the finding is open.
// Run in the tools container: cd /work/assets/companion && NODE_PATH=/opt/node/lib/node_modules node --test test/review-app-round2.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { xchacha20poly1305 } from '../vendor/noble-ciphers-2.4.0/chacha.js';
import {
  bytesOf, canonicalJSON, chachaRngStream, concat, deriveKey, deviceIdOf, edSign, hexOf, hybridWrap, identityFromIkm, parseJSONExact, reshareContribute, splitAtIndicesSeeded, utf8,
} from '../src/lib.mjs';
import { identityOfSecret, randomBytes } from '../src/keys.mjs';
import { b64enc, isU64, parseOp, safeText } from '../src/ops.mjs';
import { planReshare, publishReshare } from '../src/reshare.mjs';
import { certRequestProblems } from '../src/cert.mjs';
import { SAME_TERMINAL_MESSAGE, acceptHead, adoptOwnChain, appendCertExtend, appendHeadAttest, appendWrapKeyRotate, confirmNoChain, courierOps, emptyState, loadState, memoryBackend, restoredState, saveState } from '../src/state.mjs';
import { WRAP_GEN_MAX, pinProblem } from '../src/keys.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const rnd = (n, fill) => new Uint8Array(n).fill(fill);
const NOW = 1_900_000_000;

// ---------------------------------------------------------------- PF1: the head taken from a terminal can be stale
// Round 2 changed the contract: one list is never enough. Two lists must name the same newest op, then the user accepts.
test('PF1 a truncated own-ops list does not let the restored phone author a second op at a seq the ring holds', () => {
  const id = identityOfSecret(rnd(32, 11));
  const live = emptyState();
  for (let i = 0; i < 3; i++) appendHeadAttest(live, id, NOW + i, null);
  const ring3 = live.chain[2];
  const stale = courierOps(live, 8).slice(0, 2); // a courier (or a terminal behind on sync) hands over seq 1..2 only
  const fresh = restoredState();
  assert.equal(adoptOwnChain(fresh, id, stale, new Uint8Array(8).fill(1)).agreed, false);
  assert.throws(() => appendHeadAttest(fresh, id, NOW + 100, null), /must not sign yet/, 'one list adopts nothing');
  // a second, newer list disagrees with the first: still nothing, and the page says which seq the earlier one ended at
  const newer = adoptOwnChain(fresh, id, courierOps(live, 8), new Uint8Array(8).fill(2));
  assert.equal(newer.agreed, false);
  assert.equal(newer.previous_seq, '2');
  assert.throws(() => acceptHead(fresh), /No chain head/);
  // two lists naming the same newest op agree; acceptance continues after seq 3, no fork
  assert.equal(adoptOwnChain(fresh, id, courierOps(live, 8), new Uint8Array(8).fill(1)).agreed, true);
  assert.equal(acceptHead(fresh).head_seq, '3');
  const next = appendHeadAttest(fresh, id, NOW + 100, null);
  assert.equal(next.entry.seq, '4');
  assert.notEqual(next.entry.seq, ring3.seq);
});

test('PF1 the screen data names the adopted seq and the newest op time; "never signed anything" needs the device id typed', () => {
  const id = identityOfSecret(rnd(32, 16));
  const live = emptyState();
  appendHeadAttest(live, id, NOW, null);
  appendHeadAttest(live, id, NOW + 7, null);
  const r = adoptOwnChain(restoredState(), id, courierOps(live, 8));
  assert.equal(r.head_seq, '2');
  assert.equal(r.newest_at, String(NOW + 7));
  const s = restoredState();
  assert.throws(() => confirmNoChain(s, id, ''), /device id/);
  assert.throws(() => confirmNoChain(s, id, 'aaaaaaaaaaaaaaaa'), /device id/);
  assert.equal(s.headPending, true);
  confirmNoChain(s, id, id.deviceId);
  assert.equal(s.headPending, false);
});

test('PF2 a restore plus the delivered own chain derives wrap generation 2 from the WrapKeyRotate ops', () => {
  const id0 = identityOfSecret(rnd(32, 12));
  const live = emptyState();
  const id1 = identityOfSecret(id0.secret.slice(), 1);
  appendWrapKeyRotate(live, id0, id1, NOW, null);
  const id2 = identityOfSecret(id0.secret.slice(), 2);
  appendWrapKeyRotate(live, id1, id2, NOW + 1, null);
  assert.equal(live.wrapEpoch, 2);
  const fresh = restoredState();
  const me = identityOfSecret(rnd(32, 12));
  adoptOwnChain(fresh, me, courierOps(live, 8), new Uint8Array(8).fill(1));
  adoptOwnChain(fresh, me, courierOps(live, 8), new Uint8Array(8).fill(2));
  acceptHead(fresh);
  assert.equal(fresh.wrapEpoch, 2, 'the page opens shares wrapped to generation 2 only if it knows to derive it');
});

test('PF2 unwrap tries generations above the stored one, and a rotation never starts below one the ring uses', () => {
  const secret = rnd(32, 17);
  const key = rnd(32, 99);
  const g3 = identityOfSecret(secret.slice(), 3);
  const blob = hybridWrap({ x25519_pub: g3.xPub, mlkem_pub: g3.mlkemPub }, key, randomBytes(32), randomBytes(24), randomBytes(32));
  const restored = identityOfSecret(secret.slice(), 0); // a counter that was lost
  assert.deepEqual(Array.from(restored.unwrap(blob)), Array.from(key));
  assert.equal(restored.seenWrapEpoch, 3);
  const g40 = identityOfSecret(secret.slice(), 40);
  const far = hybridWrap({ x25519_pub: g40.xPub, mlkem_pub: g40.mlkemPub }, key, randomBytes(32), randomBytes(24), randomBytes(32));
  assert.throws(() => identityOfSecret(secret.slice(), 0).unwrap(far), undefined, `beyond ${WRAP_GEN_MAX} is not tried`);
});

// ---------------------------------------------------------------- PF3: two tabs fork the chain
test('PF3 two page instances on one storage: the stale one cannot overwrite a longer chain', async () => {
  const kv = memoryBackend();
  const id = identityOfSecret(rnd(32, 13));
  const s0 = emptyState();
  appendHeadAttest(s0, id, NOW, null);
  await saveState(kv, s0);
  const a = await loadState(kv);
  const b = await loadState(kv);
  appendHeadAttest(a, id, NOW + 1, null);
  appendHeadAttest(b, id, NOW + 2, null);
  await saveState(kv, a);
  await assert.rejects(() => saveState(kv, b), /Another copy of this page/, 'a stale instance must not overwrite a longer chain');
  assert.equal((await loadState(kv)).chain[1].hash, a.chain[1].hash);
  await saveState(kv, restoredState(), { replace: true }); // a deliberate restore starts a chain over
});

// ---------------------------------------------------------------- reshare scenario with a chosen epoch and a chosen last holder
const u64 = (n) => {
  const b = new Uint8Array(8);
  new DataView(b.buffer).setBigUint64(0, BigInt(n));
  return b;
};
function scenario({ phone, epoch, lastHolder = 'dddddddddddddddd' }) {
  const HOST = '0a'.repeat(16);
  const hostB = bytesOf(HOST);
  const commit = (from, index, sub) => deriveKey('sshub/sync/reshare/commit/v1', concat(Uint8Array.of(from, index), sub));
  const seal = (key, aad, plain) => {
    const nonce = randomBytes(24);
    return concat(nonce, xchacha20poly1305(key, nonce, aad).encrypt(plain));
  };
  const old = splitAtIndicesSeeded(randomBytes(32), 2, [1, 2, 3], chachaRngStream(randomBytes(32), 32));
  const dev = (n) => identityFromIkm(new Uint8Array(32).fill(n), `sshub/test/contrib/${n}`);
  const A = dev(1);
  const B = dev(2);
  const ids = { A: deviceIdOf(A.edPub), B: deviceIdOf(B.edPub) };
  const holders = [ids.A, ids.B, phone.deviceId, lastHolder].sort();
  const newIndices = holders.map((_, i) => i + 1);
  let lamport = 10;
  const signed = [];
  const emit = (author, kind, fields) => {
    const text = canonicalJSON({ device_id: deviceIdOf(author.edPub), seq: lamport, lamport, prev_hash: '00'.repeat(32), timestamp: 1893452400, epoch: 0, alg: [1, 1], body: { [kind]: fields } });
    lamport += 1;
    const canonical = utf8(text);
    signed.push({ canonical, signature: edSign(author.edSeed, canonical) });
  };
  for (const [name, who, oldShare] of [['A', A, old[0]], ['B', B, old[1]]]) {
    const author = ids[name];
    const c = reshareContribute(oldShare, newIndices, 2, chachaRngStream(randomBytes(32), 32));
    const sessionKey = randomBytes(32);
    emit(who, 'ShareReshareSession', { epoch, to: phone.deviceId, sealed: b64enc(hybridWrap({ x25519_pub: phone.xPub, mlkem_pub: phone.mlkemPub }, sessionKey, randomBytes(32), randomBytes(24), randomBytes(32))) });
    const sub_shares = holders.map((to, j) => {
      const [, sub] = c.sub_shares[j];
      const key = to === phone.deviceId ? sessionKey : randomBytes(32);
      return [to, b64enc(seal(key, concat(hostB, u64(epoch), utf8(author), utf8(to)), concat(u64(0), Uint8Array.of(c.from), sub)))];
    });
    const commitments = holders.map((_, j) => b64enc(commit(c.from, c.sub_shares[j][0], c.sub_shares[j][1])));
    emit(who, 'ShareReshareBatch', { epoch, new_holders: holders, items: [{ host_uid: HOST, sub_shares, commitments }] });
  }
  return { signed, holders, ring: {} }; // ring empty: the phone has met nobody (or the contributors are invented)
}

test('PF4 a reshare batch with epoch -1 is refused before the phone authors anything', () => {
  const phone = identityOfSecret(rnd(32, 14));
  const sc = scenario({ phone, epoch: -1n });
  const plan = planReshare(phone, sc.signed, -1n, { ring: sc.ring });
  assert.equal(plan.ok, false);
  assert.match(plan.error, /u64/);
  assert.equal(planReshare(phone, sc.signed, 2n ** 64n, { ring: sc.ring }).ok, false);
  const st = emptyState();
  const id = identityOfSecret(rnd(32, 18));
  const hash = '00'.repeat(32);
  for (const bad of [-1n, 2n ** 64n]) {
    assert.throws(() => appendCertExtend(st, id, hash, bad, NOW, null), undefined, `extra_hosts ${bad}`);
  }
  assert.equal(st.chain.length, 0, 'nothing was appended');
  appendCertExtend(st, id, hash, 2n ** 64n - 1n, NOW, null); // the edge is fine
  assert.equal(st.chain.length, 1);
});

test('PF4 an op whose body carries a non-u64 integer is never appended (the assertion before signing)', () => {
  const phone = identityOfSecret(rnd(32, 19));
  const plan = { ok: true, epoch: -1n, items: [{ host_uid: '0a'.repeat(16), sealed_share: rnd(40, 1) }], storageSealed: rnd(40, 2), seen: null };
  const st = emptyState();
  assert.throws(() => publishReshare(st, phone, plan, NOW), /u64/);
  assert.equal(st.chain.length, 0);
});

test('PF5 holder ids of a reshare plan must be 16 lower-case hex characters (a bidi id never reaches the summary row)', () => {
  const phone = identityOfSecret(rnd(32, 15));
  const evil = 'ab\u202edddddddddddd';
  const sc = scenario({ phone, epoch: 1n, lastHolder: evil });
  const plan = planReshare(phone, sc.signed, 1n, { ring: sc.ring });
  assert.equal(plan.ok, false);
  assert.match(plan.error, /holder id/);
});

// ---------------------------------------------------------------- PF6: PIN counter in a real browser
test('PF6 concurrent unlock taps are counted one after the other: three wrong PINs, then the delay refuses the rest', { timeout: 240000 }, async () => {
  const require = createRequire(import.meta.url);
  const { chromium } = require('playwright');
  const body = fs.readFileSync(path.resolve(here, '..', 'sshub-companion.html'));
  const server = http.createServer((req, res) => (req.url === '/' ? res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' }).end(body) : res.writeHead(404).end()));
  await new Promise((r) => server.listen(0, 'localhost', r));
  const url = `http://localhost:${server.address().port}/`;
  const browser = await chromium.launch();
  try {
    const page = await (await browser.newContext()).newPage();
    await page.goto(url);
    await page.waitForSelector('#create');
    await page.fill('#pin1', 'correct-pin-17');
    await page.fill('#pin2', 'correct-pin-17');
    await page.click('#create');
    await page.waitForSelector('#backup-words span');
    await page.check('#backup-ok');
    await page.click('#backup-done');
    await page.waitForSelector('#device-id');
    await page.reload();
    await page.waitForSelector('#unlock');
    await page.fill('#pin', 'wrong-pin-0000');
    await page.evaluate(() => { for (let i = 0; i < 5; i++) document.querySelector('#unlock').click(); });
    await page.waitForTimeout(25000);
    const fails = await page.evaluate(() => new Promise((res) => {
      const r = indexedDB.open('sshub-companion', 1);
      r.onsuccess = () => { const g = r.result.transaction('kv').objectStore('kv').get('unlock-fails'); g.onsuccess = () => res(g.result); };
    }));
    assert.equal(fails && fails.count, 3, `the persisted counter says ${JSON.stringify(fails)}: tries 1..3 count (not 5 taps as 1), tries 4 and 5 meet the 5 s delay and are refused`);
    assert.ok(fails.until > 0, 'the delay is stored');
  } finally {
    await browser.close();
    server.close();
  }
});

// ---------------------------------------------------------------- PF7: invisible characters outside \p{Cf}
test('PF7 blank-looking characters do not survive safeText', () => {
  for (const c of ['\u2800', '\uffa0', '\u17b4', '\u17b5', '\u3000', '\u00a0', '\u2009', '\u2003']) {
    assert.notEqual(safeText(`a${c}b`), `a${c}b`, `U+${c.codePointAt(0).toString(16)} passes unchanged`);
  }
  assert.equal(safeText('a b'), 'a b', 'the plain space stays');
});

test('SU3 trivially weak PINs are refused: one character repeated, ascending and descending runs, too short', () => {
  for (const bad of ['1111111111', '1234567890', '0987654321', '9876543210', 'abcdefghij', 'aaaaaaaaaaaa', '123456789']) assert.ok(pinProblem(bad), bad);
  for (const good of ['correct-pin-17', '1357924680', 'blue-tiger-ladder-9']) assert.equal(pinProblem(good), null, good);
});

// ---------------------------------------------------------------- sound checks (must stay green)
const op = (over = {}, body = '{"HeadAttest":{"device_id":"aaaaaaaaaaaaaaaa","seq":1,"chain_hash":"' + '0'.repeat(64) + '"}}') => {
  const h = { device_id: 'bbbbbbbbbbbbbbbb', seq: '1', lamport: '1', prev_hash: '0'.repeat(64), timestamp: String(NOW), epoch: '0', ...over };
  return utf8(`{"device_id":"${h.device_id}","seq":${h.seq},"lamport":${h.lamport},"prev_hash":"${h.prev_hash}","timestamp":${h.timestamp},"epoch":${h.epoch},"alg":[1,1],"body":${body}}`);
};
test('sound: parseOp range edges (u64 max ok, 2^64 and -1 refused, i64 timestamp edges)', () => {
  assert.doesNotThrow(() => parseOp(op({ seq: '18446744073709551615', lamport: '18446744073709551615', epoch: '18446744073709551615' })));
  for (const bad of [{ seq: '18446744073709551616' }, { lamport: '-1' }, { epoch: '18446744073709551616' }, { seq: '-0' }, { seq: '01' }, { seq: '1.0' }, { seq: '1e0' }, { timestamp: '9223372036854775808' }, { timestamp: '-9223372036854775809' }]) {
    assert.throws(() => parseOp(op(bad)), undefined, JSON.stringify(bad));
  }
  assert.doesNotThrow(() => parseOp(op({ timestamp: '-9223372036854775808' })));
  assert.doesNotThrow(() => parseOp(op({ timestamp: '9223372036854775807' })));
});
test('sound: parseJSONExact depth, duplicates, escapes, unterminated', () => {
  assert.throws(() => parseJSONExact('['.repeat(33) + ']'.repeat(33)));
  assert.doesNotThrow(() => parseJSONExact('['.repeat(32) + ']'.repeat(32)));
  assert.throws(() => parseOp(utf8('{"a":1,"a":1}')));
  for (const s of ['"\\ud800"', '"\\u00zz"', '"\\q"', '{"a":1', '{"__proto__":1}']) assert.throws(() => parseJSONExact(s), undefined, s);
});
test('sound: a 250 KiB integer token is refused quickly', () => {
  const t0 = Date.now();
  assert.throws(() => parseOp(op({ seq: '9'.repeat(250000) })));
  assert.ok(Date.now() - t0 < 1500, `took ${Date.now() - t0} ms`);
});
test('sound: certificate epoch range', () => {
  const req = (lo, hi) => ({ device_id: 'cccccccccccccccc', scope: { group: 'aa'.repeat(16) }, expiry: BigInt(NOW + 86400), max_new_hosts_per_day: 5, epoch_lo: lo, epoch_hi: hi });
  assert.equal(certRequestProblems(req(3n, 4n), NOW, 'dddddddddddddddd', '3').length, 0);
  assert.ok(certRequestProblems(req(0n, 18446744073709551615n), NOW, 'dddddddddddddddd', '0').length > 0);
  assert.ok(certRequestProblems(req(2n, 3n), NOW, 'dddddddddddddddd', '3').length > 0);
  assert.equal(certRequestProblems(req(18446744073709551615n, 18446744073709551615n), NOW, 'dddddddddddddddd', '18446744073709551615').length, 0);
});

test('C-M2 the same terminal twice never agrees, two terminals naming the same head do, a list without a sender never does', () => {
  const id = identityOfSecret(rnd(32, 14));
  const live = emptyState();
  for (let i = 0; i < 3; i++) appendHeadAttest(live, id, NOW + i, null);
  const ops = courierOps(live, 8);
  const a = new Uint8Array(8).fill(1);
  const b = new Uint8Array(8).fill(2);
  // the same frame twice: refused with the sentence, the first candidate stays
  const s = restoredState();
  assert.equal(adoptOwnChain(s, id, ops, a).agreed, false);
  assert.throws(() => adoptOwnChain(s, id, ops, a), (e) => e.message === SAME_TERMINAL_MESSAGE);
  assert.equal(s.headProposal, undefined);
  assert.ok(s.headCandidate);
  assert.equal(adoptOwnChain(s, id, ops, b).agreed, true);
  // a list without a sender can be the first candidate but never agrees, from either side, and never costs the id
  const t = restoredState();
  assert.equal(adoptOwnChain(t, id, ops).agreed, false);
  assert.equal(adoptOwnChain(t, id, ops).agreed, false);
  assert.equal(adoptOwnChain(t, id, ops, a).agreed, false, 'the first one named nobody');
  assert.equal(t.headProposal, undefined);
  assert.equal(t.headCandidate.sender, '0101010101010101');
  assert.equal(adoptOwnChain(t, id, ops).agreed, false, 'an anonymous list never completes a pair');
  assert.equal(t.headCandidate.sender, '0101010101010101');
  assert.equal(adoptOwnChain(t, id, ops, b).agreed, true);
  // the same terminal with a newer head is a new candidate, not a refusal
  const u = restoredState();
  adoptOwnChain(u, id, ops.slice(0, 2), a);
  const newer = adoptOwnChain(u, id, ops, a);
  assert.equal(newer.agreed, false);
  assert.equal(newer.previous_seq, '2');
});

test('C-M2 accepting a head is bound to the head that was shown', () => {
  const id = identityOfSecret(rnd(32, 13));
  const live = emptyState();
  for (let i = 0; i < 3; i++) appendHeadAttest(live, id, NOW + i, null);
  const fresh = restoredState();
  const ops = courierOps(live, 8);
  adoptOwnChain(fresh, id, ops, new Uint8Array(8).fill(1));
  const agreed = adoptOwnChain(fresh, id, ops, new Uint8Array(8).fill(2));
  assert.equal(agreed.agreed, true);
  assert.equal(agreed.head_hash, live.chain[2].hash);
  assert.throws(() => acceptHead(fresh, { seq: '2', hash: live.chain[1].hash }), /changed since it was shown/);
  assert.ok(fresh.headProposal, 'the agreed proposal stays');
  acceptHead(fresh, { seq: agreed.head_seq, hash: agreed.head_hash });
  assert.equal(fresh.headPending, false);
});
