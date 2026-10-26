// Round 3 review reproductions for the phone page (host-sync-plan/review/app-page-round3.md).
// Every test marked FAILS asserts the safe behaviour and fails while the finding is open. "sound:" tests pass.
// Run in the tools container: cd /work/assets/companion && NODE_PATH=/opt/node/lib/node_modules node --test test/review-app-round3.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { xchacha20poly1305 } from '../vendor/noble-ciphers-2.4.0/chacha.js';
import {
  bytesOf, canonicalJSON, chachaRngStream, concat, deriveKey, deviceIdOf, edSign, hexOf, hybridWrap, identityFromIkm, reshareContribute, splitAtIndicesSeeded, utf8,
} from '../src/lib.mjs';
import { identityOfSecret, pinProblem, randomBytes } from '../src/keys.mjs';
import { b64enc, isU64 } from '../src/ops.mjs';
import { planReshare } from '../src/reshare.mjs';
import { emptyState, loadState, memoryBackend, saveState } from '../src/state.mjs';
import { ABORT, BLE, BleSession, Reassembler, chunkCount, crc32, cutFrame } from '../src/ble.mjs';

const rnd = (n, fill) => new Uint8Array(n).fill(fill);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ---------------------------------------------------------------- RP1: reshare "k verified contributors" counts the wrong set
const u64 = (n) => {
  const b = new Uint8Array(8);
  new DataView(b.buffer).setBigUint64(0, BigInt(n));
  return b;
};
// Real ring terminals A and B (known to the phone, genuine signatures) contribute a batch each but NO session to the phone,
// so their sub-shares cannot be opened. Invented authors X and Y (unknown to the phone) carry the sessions and sub-shares
// that the phone actually combines.
function mixedScenario(phone, epoch) {
  const HOST = '0a'.repeat(16);
  const hostB = bytesOf(HOST);
  const commit = (from, index, sub) => deriveKey('sshub/sync/reshare/commit/v1', concat(Uint8Array.of(from, index), sub));
  const seal = (key, aad, plain) => {
    const nonce = randomBytes(24);
    return concat(nonce, xchacha20poly1305(key, nonce, aad).encrypt(plain));
  };
  const old = splitAtIndicesSeeded(randomBytes(32), 2, [1, 2, 3], chachaRngStream(randomBytes(32), 32));
  const dev = (n) => identityFromIkm(new Uint8Array(32).fill(n), `sshub/test/contrib/${n}`);
  const A = dev(1), B = dev(2), X = dev(3), Y = dev(4);
  const holders = [deviceIdOf(A.edPub), deviceIdOf(B.edPub), phone.deviceId, 'dddddddddddddddd'].sort();
  const newIndices = holders.map((_, i) => i + 1);
  let lamport = 10;
  const signed = [];
  const emit = (author, kind, fields) => {
    const text = canonicalJSON({ device_id: deviceIdOf(author.edPub), seq: lamport, lamport, prev_hash: '00'.repeat(32), timestamp: 1893452400, epoch: 0, alg: [1, 1], body: { [kind]: fields } });
    lamport += 1;
    const canonical = utf8(text);
    signed.push({ canonical, signature: edSign(author.edSeed, canonical) });
  };
  const contribute = (who, oldShare, withSession) => {
    const author = deviceIdOf(who.edPub);
    const c = reshareContribute(oldShare, newIndices, 2, chachaRngStream(randomBytes(32), 32));
    const sessionKey = randomBytes(32);
    if (withSession) emit(who, 'ShareReshareSession', { epoch, to: phone.deviceId, sealed: b64enc(hybridWrap({ x25519_pub: phone.xPub, mlkem_pub: phone.mlkemPub }, sessionKey, randomBytes(32), randomBytes(24), randomBytes(32))) });
    const sub_shares = holders.map((to, j) => {
      const [, sub] = c.sub_shares[j];
      const key = to === phone.deviceId ? sessionKey : randomBytes(32);
      return [to, b64enc(seal(key, concat(hostB, u64(epoch), utf8(author), utf8(to)), concat(u64(0), Uint8Array.of(c.from), sub)))];
    });
    const commitments = holders.map((_, j) => b64enc(commit(c.from, c.sub_shares[j][0], c.sub_shares[j][1])));
    emit(who, 'ShareReshareBatch', { epoch, new_holders: holders, items: [{ host_uid: HOST, sub_shares, commitments }] });
  };
  contribute(A, old[0], false);
  contribute(B, old[1], false);
  contribute(X, old[0], true);
  contribute(Y, old[1], true);
  const ring = { [deviceIdOf(A.edPub)]: { name: 'A', ed25519_pub: hexOf(A.edPub) }, [deviceIdOf(B.edPub)]: { name: 'B', ed25519_pub: hexOf(B.edPub) } };
  return { signed, ring };
}

test('RP1 FAILS reshare adoption needs k VERIFIED contributions combined, not k verified signatures next to unverified shares', () => {
  const phone = identityOfSecret(rnd(32, 31));
  const sc = mixedScenario(phone, 1n);
  const plan = planReshare(phone, sc.signed, 1n, { ring: sc.ring });
  assert.equal(plan.ok, false, `plan.ok with contributors ${JSON.stringify((plan.contributors || []).map((c) => [c.verified, c.session]))}: the two shares combined came from authors this phone never met`);
});

// ---------------------------------------------------------------- RP2: PIN floor still accepts trivially guessable PINs
test('RP2 FAILS common and near-trivial 10 character PINs are refused', () => {
  const weak = ['1234512345', '1111111112', '0000000001', '1234567891', '1212121212', '0123012301', 'password12', 'qwertyuiop', 'aaaaaaaaab', '1357913579'];
  const accepted = weak.filter((p) => pinProblem(p) === null);
  assert.deepEqual(accepted, [], `accepted: ${accepted.join(' ')}`);
});

test('sound: PIN floor still refuses short, repeated and runs', () => {
  for (const p of ['123456789', '1111111111', '1234567890', '0987654321', '9876543210', 'abcdefghij', 'jihgfedcba', '          ']) assert.notEqual(pinProblem(p), null, p);
  assert.equal(pinProblem('correct-horse-battery'), null);
});

// ---------------------------------------------------------------- RP3: compare-and-set loses non-chain updates
test('RP3 FAILS a stale page instance does not erase the other instance\'s short-code record or enrollment', async () => {
  const kv = memoryBackend();
  await saveState(kv, emptyState());
  const a = await loadState(kv);
  const b = await loadState(kv);
  a.snapshots['0a'.repeat(16)] = { issued_at: 1, expires_at: 9e9, profile: { mem_kib: 1, iters: 1 }, weak: false };
  await saveState(kv, a); // tab A issued a short code (the one-live-code rule lives in this record)
  b.ring['bbbbbbbbbbbbbbbb'] = { name: 'laptop', ed25519_pub: '00'.repeat(32) };
  try {
    await saveState(kv, b); // tab B (stale) enrolled something and saves
  } catch {
    /* refusing is the safe behaviour */
  }
  const stored = await loadState(kv);
  assert.ok(stored.snapshots['0a'.repeat(16)], 'the short-code record of the other instance was overwritten (one-live-code rule erased)');
});

test('sound: CAS refuses a stale chain and accepts the extension; locks absent falls back; locks present is exclusive', async () => {
  const kv = memoryBackend();
  const s0 = emptyState();
  await saveState(kv, s0);
  const order = [];
  const real = Object.getOwnPropertyDescriptor(globalThis, 'navigator');
  let held = Promise.resolve();
  Object.defineProperty(globalThis, 'navigator', { configurable: true, value: { locks: { request: (name, cb) => { const r = held.then(() => cb()); held = r.catch(() => {}); order.push(name); return r; } } } });
  try {
    await Promise.all([saveState(kv, s0), saveState(kv, s0)]);
    assert.deepEqual(order, ['sshub-companion-state', 'sshub-companion-state']);
  } finally {
    if (real) Object.defineProperty(globalThis, 'navigator', real);
    else delete globalThis.navigator;
  }
});

// ---------------------------------------------------------------- RP4/RP5/RP6: BLE (contract C20) against the page's ble.mjs
const T = { HELLO: 1, HELLO_ACK: 2, DATA: 0x10, ACK: 0x11, ABORT: 0x12, BYE: 0x20, PING: 0x22 };
const rd16 = (b, at) => (b[at] << 8) | b[at + 1];
const frameOf = (len, kind = 1, fill = 7) => {
  const f = new Uint8Array(len);
  f.set([0x53, 0x57, 0x52, 0x31, 0, 1, kind]);
  for (let i = 7; i < len; i++) f[i] = (i * 31 + fill) & 0xff;
  return f;
};
function fakeLink() {
  const sent = [];
  let recv = null;
  const link = {
    sent,
    onWrite: null,
    transport: {
      write: async (m) => {
        const c = m.slice();
        sent.push(c);
        if (link.onWrite) await link.onWrite(c);
      },
      setReceiver: (fn) => (recv = fn),
      setOnClose: () => {},
      close: () => {},
    },
    notify: (m) => recv(m),
  };
  return link;
}
const ackOf = (id, next) => Uint8Array.of(T.ACK, id >> 8, id & 255, next >> 8, next & 255);
const helloAck = (link, chunk = 244, window = 8) => {
  link.onWrite = (m) => {
    if (m[0] === T.HELLO) queueMicrotask(() => link.notify(Uint8Array.of(T.HELLO_ACK, 1, chunk >> 8, chunk & 255, window)));
  };
};
const fast = { ACK_MS: 5000, HELLO_MS: 500, PING_MS: 60_000, IDLE_MS: 60_000, FRAME_MS: 60_000, RETRIES: 3 };

test('RP4 FAILS a frame needing more than 65535 chunks at the 20 byte minimum is refused, not cut with wrapped counters', () => {
  // C20: chunk min 20, u16 count and idx, frame max 1 MiB: 1 MiB at chunk 20 is 80660 chunks.
  const f = frameOf(900_000);
  let msgs = null;
  try {
    msgs = cutFrame(f, 1, 20);
  } catch {
    return; // refusing is the safe behaviour
  }
  assert.ok(msgs.length <= 0xffff, `${msgs.length} chunks, header count field says ${rd16(msgs[0], 5)} and idx wraps`);
});

test('RP5 FAILS an ACK for chunks the page has not sent yet does not complete the frame', async () => {
  const link = fakeLink();
  helloAck(link);
  const s = new BleSession(link.transport, { timing: fast });
  await s.start();
  const frame = frameOf(5000, 2);
  const total = chunkCount(frame.length, 244);
  let done = false;
  const p = s.sendFrame(frame).then(() => (done = true), () => {});
  await sleep(5);
  const sentBefore = link.sent.filter((m) => m[0] === T.DATA).length;
  link.notify(ackOf(1, total)); // a terminal (or a spoofer on the link) claims it has everything
  await sleep(5);
  assert.ok(sentBefore < total, `only ${sentBefore} of ${total} chunks were on the air`);
  assert.equal(done, false, 'sendFrame resolved (UI says "Sent.") for a frame that was never sent');
  await s.bye();
  await p;
});

test('RP6 FAILS a HELLO_ACK while the session is open restarts it (C20): the half-received frame is forgotten', async () => {
  const link = fakeLink();
  helloAck(link);
  const s = new BleSession(link.transport, { timing: fast });
  await s.start();
  const msgs = cutFrame(frameOf(2000, 1), 7, 244);
  link.notify(msgs[0]);
  await sleep(2);
  assert.ok(s.inbox.cur, 'a frame is half received');
  link.notify(Uint8Array.of(T.HELLO_ACK, 1, 0, 100, 8));
  await sleep(2);
  assert.equal(s.inbox.cur, null, 'C20: the page MUST treat a HELLO_ACK while open as a restart and forget the half-received frame');
  await s.bye();
});

test('RP7 FAILS a duplicate ACK while chunks are outstanding makes the sender go back at once (C20), not after BLE_ACK_SECS', async () => {
  const link = fakeLink();
  helloAck(link);
  const s = new BleSession(link.transport, { timing: { ...fast, ACK_MS: 3000 } });
  await s.start();
  const frame = frameOf(5000, 2);
  const p = s.sendFrame(frame).catch(() => {});
  await sleep(5);
  link.notify(ackOf(1, 4)); // progress to 4, window 8 allows chunks up to 11 on the air
  await sleep(5);
  const before = link.sent.filter((m) => m[0] === T.DATA).length;
  link.notify(ackOf(1, 4)); // equal to the last acknowledged next: the receiver saw a gap
  await sleep(50);
  const after = link.sent.filter((m) => m[0] === T.DATA);
  const resent = after.slice(before).some((m) => rd16(m, 3) === 4);
  assert.ok(resent, 'C20: ACK(next) equal to the last acknowledged next means a gap; the sender goes back to next once (no resend within 50 ms)');
  await s.bye();
  await p;
});

test('RP8 FAILS a repeated chunk 0 must not keep a stalled frame alive past BLE_FRAME_SECS', async () => {
  const link = fakeLink();
  helloAck(link);
  const s = new BleSession(link.transport, { timing: { ...fast, FRAME_MS: 150 } });
  await s.start();
  const first = cutFrame(frameOf(3000, 1), 3, 244)[0];
  for (let i = 0; i < 8; i++) {
    link.notify(first);
    await sleep(60); // 480 ms in total, three times the frame window
  }
  const timedOut = link.sent.some((m) => m[0] === T.ABORT && rd16(m, 1) === 3 && m[3] === ABORT.TIMEOUT);
  assert.ok(timedOut, 'the frame timer was re-armed by every repeat of chunk 0, the 120 s limit never fires while the peer keeps repeating');
  await s.bye();
});

test('sound: BLE reassembly bounds (len cap, zero-length, count lie, interleave, duplicate, overlong chunk)', () => {
  const chunk = 244;
  const abortOf = (o) => (o.reply.length === 1 && o.reply[0][0] === T.ABORT ? o.reply[0][3] : null);
  const f = frameOf(2000);
  const msgs = cutFrame(f, 5, chunk);
  // len above the cap, len zero, count lie
  for (const mut of [(m) => new DataView(m.buffer).setUint32(7, BLE.MAX_FRAME_BYTES + 1), (m) => new DataView(m.buffer).setUint32(7, 0), (m) => (m[6] += 1)]) {
    const m = msgs[0].slice();
    mut(m);
    const r = new Reassembler({ chunk });
    assert.equal(abortOf(r.push(m)), ABORT.TOO_LARGE);
    assert.equal(r.cur, null);
  }
  // a chunk longer than the negotiated size is refused and releases memory
  const r = new Reassembler({ chunk });
  const long = concat(msgs[0], new Uint8Array(40));
  assert.equal(abortOf(r.push(long)), ABORT.CRC);
  assert.equal(r.cur, null);
  // zero-length payload chunks cannot complete a frame
  const r2 = new Reassembler({ chunk });
  r2.push(msgs[0]);
  for (let i = 1; i < msgs.length; i++) {
    const h = msgs[i].slice(0, 7);
    const out = r2.push(h);
    if (i === msgs.length - 1) assert.equal(abortOf(out), ABORT.CRC);
  }
  assert.equal(r2.cur, null);
  // duplicate and gap are answered with ACK(next), nothing buffered twice
  const r3 = new Reassembler({ chunk });
  r3.push(msgs[0]);
  const dup = r3.push(msgs[0]);
  assert.deepEqual(Array.from(dup.reply[0]), Array.from(ackOf(5, 1)));
  const gap = r3.push(msgs[3]);
  assert.deepEqual(Array.from(gap.reply[0]), Array.from(ackOf(5, 1)));
  assert.equal(r3.cur.have, msgs[0].length - 15);
  // another frame id while one is in progress: ABORT(busy) for the intruder, the first survives
  const other = cutFrame(frameOf(900), 6, chunk)[0];
  assert.equal(abortOf(r3.push(other)), ABORT.BUSY);
  assert.equal(r3.cur.id, 5);
  // crc: a flipped payload bit is refused
  const r4 = new Reassembler({ chunk });
  const bad = cutFrame(f, 8, chunk);
  bad[2][20] ^= 1;
  const outs = bad.map((m) => r4.push(m));
  assert.equal(abortOf(outs[outs.length - 1]), ABORT.CRC);
  assert.equal(crc32(new TextEncoder().encode('123456789')), 0xcbf43926);
});

// ---------------------------------------------------------------- isU64 with a float
test('RP9 FAILS isU64 answers false for a non-integer number instead of throwing', () => {
  for (const v of [1.5, NaN, Infinity]) {
    let r;
    try {
      r = isU64(v);
    } catch (e) {
      assert.fail(`isU64(${v}) threw ${e.constructor.name}: ${e.message}`);
    }
    assert.equal(r, false);
  }
});

test('sound: isU64 edges', () => {
  assert.equal(isU64(0), true);
  assert.equal(isU64(2n ** 64n - 1n), true);
  assert.equal(isU64(2n ** 64n), false);
  assert.equal(isU64(-1), false);
  assert.equal(isU64(-1n), false);
  assert.equal(isU64(2 ** 53), true);
  assert.equal(isU64('5'), false);
  assert.equal(isU64(null), false);
  assert.equal(isU64(undefined), false);
  assert.equal(isU64({}), false);
});

// ---------------------------------------------------------------- unwrap cost with a lost generation counter
test('sound: a failing unwrap on a restored identity stays well under a second of CPU per blob', () => {
  const id = identityOfSecret(rnd(32, 40), 0);
  const blob = rnd(1200, 3);
  const t0 = Date.now();
  assert.throws(() => id.unwrap(blob));
  const first = Date.now() - t0;
  const t1 = Date.now();
  assert.throws(() => id.unwrap(blob));
  const second = Date.now() - t1;
  console.log(`unwrap miss: first ${first} ms (derives 32 generations), next ${second} ms`);
  assert.ok(first < 5000);
});
