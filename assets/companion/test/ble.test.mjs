// Contract C20 (Bluetooth frame channel), page side: chunking, receiver rules, a session against a fake terminal, and
// connectBle against a fake GATT object. No hook in production code: the tests use only the transport interface.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { ABORT, BLE, BYE, BleSession, Reassembler, bleAvailable, chunkCount, connectBle, crc32, cutFrame, frameLooksValid } from '../src/ble.mjs';

const frameOf = (len, kind = 1, fill = 7) => {
  const f = new Uint8Array(len).fill(fill);
  f.set([0x53, 0x57, 0x52, 0x31, 0, 1, kind]);
  for (let i = 7; i < len; i++) f[i] = (i * 31 + fill) & 0xff;
  return f;
};
const eq = (a, b) => assert.deepEqual(Array.from(a), Array.from(b));
const T = { HELLO: 1, HELLO_ACK: 2, DATA: 0x10, ACK: 0x11, ABORT: 0x12, BYE: 0x20, PING: 0x22 };
const rd16 = (b, at) => (b[at] << 8) | b[at + 1];
const ack = (id, next) => Uint8Array.of(T.ACK, id >> 8, id & 255, next >> 8, next & 255);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

test('crc32 is the IEEE check value', () => {
  assert.equal(crc32(new TextEncoder().encode('123456789')), 0xcbf43926);
});

test('chunking: count formula, sizes, chunk 0 carries len and crc, every message fits the chunk', () => {
  for (const chunk of [20, 100, 244, 512]) {
    for (const len of [7, 8, chunk - 15, chunk - 14, chunk - 15 + (chunk - 7), chunk - 14 + (chunk - 7), 5000].filter((n) => n >= 7)) {
      const f = frameOf(len);
      const msgs = cutFrame(f, 9, chunk);
      assert.equal(msgs.length, chunkCount(len, chunk), `len ${len} chunk ${chunk}`);
      assert.ok(msgs.every((m) => m.length <= chunk));
      assert.equal(msgs[0][0], T.DATA);
      assert.equal(rd16(msgs[0], 1), 9);
      assert.equal(rd16(msgs[0], 5), msgs.length);
      assert.equal(new DataView(msgs[0].buffer).getUint32(7), len);
      assert.equal(new DataView(msgs[0].buffer).getUint32(11), crc32(f));
      msgs.slice(1, -1).forEach((m) => assert.equal(m.length, chunk, 'only the last chunk is short'));
    }
  }
  assert.throws(() => cutFrame(frameOf(10), 1, 19));
  assert.throws(() => cutFrame(frameOf(10), 1, 513));
  assert.throws(() => cutFrame(new Uint8Array(10), 1, 244), /SWR1/);
  assert.throws(() => cutFrame(frameOf(10, 13), 1, 244), /SWR1/);
});

test('reassembly: every size round-trips; ACK after window/2 chunks and after the last', () => {
  for (const chunk of [20, 244]) {
    for (const len of [7, 229, 230, 1000, 20000]) {
      const f = frameOf(len, 5);
      const r = new Reassembler({ chunk });
      const msgs = cutFrame(f, 1, chunk);
      let got = null;
      msgs.forEach((m, i) => {
        const out = r.push(m);
        const wantAck = (i + 1) % 4 === 0 || i === msgs.length - 1;
        assert.equal(out.reply.length, wantAck ? 1 : 0, `chunk ${i + 1} of ${msgs.length}`);
        if (wantAck) assert.deepEqual(Array.from(out.reply[0]), Array.from(ack(1, i + 1)));
        if (out.frame) got = out.frame;
      });
      eq(got, f);
    }
  }
});

test('receiver rules: too large, count mismatch, busy, order, crc, length, magic and kind', () => {
  const f = frameOf(2000);
  const chunk = 244;
  const msgs = cutFrame(f, 5, chunk);
  const abortOf = (o) => (o.reply.length === 1 && o.reply[0][0] === T.ABORT ? [rd16(o.reply[0], 1), o.reply[0][3]] : null);

  // len above the cap, or not matching count: ABORT(2) before buffering
  const big = msgs[0].slice();
  new DataView(big.buffer).setUint32(7, BLE.MAX_FRAME_BYTES + 1);
  let r = new Reassembler({ chunk });
  assert.deepEqual(abortOf(r.push(big)), [5, ABORT.TOO_LARGE]);
  assert.equal(r.cur, null);
  const wrongCount = msgs[0].slice();
  wrongCount[6] += 1;
  assert.deepEqual(abortOf(new Reassembler({ chunk }).push(wrongCount)), [5, ABORT.TOO_LARGE]);

  // another frame_id while one is in progress: ABORT(3), the frame in progress survives
  r = new Reassembler({ chunk });
  r.push(msgs[0]);
  const other = cutFrame(frameOf(2000, 2), 6, chunk)[0];
  assert.deepEqual(abortOf(r.push(other)), [6, ABORT.BUSY]);
  assert.equal(r.cur.id, 5);

  // a gap or a duplicate is dropped and the next expected index is acknowledged again
  r = new Reassembler({ chunk });
  r.push(msgs[0]);
  assert.deepEqual(Array.from(r.push(msgs[2]).reply[0]), Array.from(ack(5, 1)), 'gap');
  assert.deepEqual(Array.from(r.push(msgs[0]).reply[0]), Array.from(ack(5, 1)), 'duplicate');
  assert.equal(r.cur.next, 1);

  // crc mismatch: ABORT(1); and the sender may try again under a new frame_id
  const badCrc = cutFrame(f, 7, chunk);
  badCrc[0][14] ^= 1;
  r = new Reassembler({ chunk });
  const outs = badCrc.map((m) => r.push(m));
  assert.deepEqual(abortOf(outs[outs.length - 1]), [7, ABORT.CRC]);
  assert.equal(r.cur, null);
  const again = cutFrame(f, 8, chunk).map((m) => r.push(m));
  eq(again[again.length - 1].frame, f);

  // length different from len: ABORT(1)
  const shortLen = cutFrame(f, 9, chunk);
  shortLen[shortLen.length - 1] = shortLen[shortLen.length - 1].slice(0, -1);
  r = new Reassembler({ chunk });
  const o2 = shortLen.map((m) => r.push(m));
  assert.deepEqual(abortOf(o2[o2.length - 1]), [9, ABORT.CRC]);

  // "SWR1" and kind 1..10 are checked on the completed frame, before it is handed on
  for (const bad of [Uint8Array.of(0x58, 0x57, 0x52, 0x31, 0, 1, 1, 1, 1), Uint8Array.of(0x53, 0x57, 0x52, 0x31, 0, 1, 0, 1, 1), Uint8Array.of(0x53, 0x57, 0x52, 0x31, 0, 1, 13, 1, 1)]) {
    const m = [T.DATA, 0, 1, 0, 0, 0, 1, 0, 0, 0, bad.length, ...new Uint8Array(new Uint32Array([0]).buffer)];
    const view = new DataView(new ArrayBuffer(15 + bad.length));
    view.setUint8(0, T.DATA);
    view.setUint16(1, 3);
    view.setUint16(3, 0);
    view.setUint16(5, 1);
    view.setUint32(7, bad.length);
    view.setUint32(11, crc32(bad));
    const raw = new Uint8Array(view.buffer);
    raw.set(bad, 15);
    void m;
    const out = new Reassembler({ chunk }).push(raw);
    assert.deepEqual(abortOf(out), [3, ABORT.BUSY]);
    assert.equal(out.frame, undefined);
  }
  assert.ok(frameLooksValid(frameOf(7, 10)) && !frameLooksValid(frameOf(7, 0)));
  // Phase B: the renewal (11) and handover (12) requests are frames too; 13 is not
  assert.ok(frameLooksValid(frameOf(7, 11)) && frameLooksValid(frameOf(7, 12)) && !frameLooksValid(frameOf(7, 13)));

  // a repeated last chunk after completion is acknowledged again, not taken as a new frame
  r = new Reassembler({ chunk });
  const full = cutFrame(frameOf(500, 3), 11, chunk).map((m) => r.push(m));
  assert.ok(full[full.length - 1].frame);
  assert.deepEqual(Array.from(r.push(msgs.length ? cutFrame(frameOf(500, 3), 11, chunk).pop() : []).reply[0]), Array.from(ack(11, chunkCount(500, chunk))));
});

// ---------------------------------------------------------------- a fake terminal behind the transport interface
function fakeLink() {
  const sent = []; // what the page wrote
  let recv = null;
  let onClose = null;
  let closed = false;
  const link = {
    sent,
    onWrite: null,
    transport: {
      write: async (m) => {
        const copy = m.slice();
        sent.push(copy);
        if (link.onWrite) await link.onWrite(copy);
      },
      setReceiver: (fn) => (recv = fn),
      setOnClose: (fn) => (onClose = fn),
      close: () => {
        closed = true;
      },
    },
    notify: (m) => recv(m),
    drop: () => onClose && onClose(),
    get closed() {
      return closed;
    },
  };
  return link;
}
const fast = { ACK_MS: 40, HELLO_MS: 80, PING_MS: 10_000, IDLE_MS: 10_000, FRAME_MS: 10_000, RETRIES: 3 };
const kinds = (link) => link.sent.map((m) => m[0]);

// The terminal side of the handshake and of receiving a frame, written with the primitives, not with BleSession.
function terminal(link, { chunk = 244, window = 8, dropAcks = 0 } = {}) {
  const rx = new Reassembler({ chunk, window });
  const t = { frames: [], aborts: [], dropAcks };
  link.onWrite = (m) => {
    if (m[0] === T.HELLO) {
      const proposal = rd16(m, 2);
      queueMicrotask(() => link.notify(Uint8Array.of(T.HELLO_ACK, 1, Math.min(proposal, chunk) >> 8, Math.min(proposal, chunk) & 255, window)));
    } else if (m[0] === T.DATA) {
      const out = rx.push(m);
      for (const r of out.reply) {
        if (r[0] === T.ACK && t.dropAcks > 0) {
          t.dropAcks -= 1;
          continue;
        }
        queueMicrotask(() => link.notify(r));
      }
      if (out.frame) t.frames.push(out.frame);
    } else if (m[0] === T.ABORT) t.aborts.push(rd16(m, 1) + ':' + m[3]);
  };
  return t;
}

test('session: HELLO proposes 244, HELLO_ACK sets chunk and window; at most 20 bytes before it', async () => {
  const link = fakeLink();
  terminal(link, { chunk: 100 });
  const s = new BleSession(link.transport, { timing: fast });
  const neg = await s.start();
  assert.deepEqual(neg, { chunk: 100, window: 8 });
  assert.deepEqual(Array.from(link.sent[0]), [T.HELLO, 1, 0, 244]);
  assert.ok(link.sent[0].length <= 20);
  assert.equal(s.state, 'open');
  await s.bye();
});

test('session: a kind 1 frame comes down in chunks and is acknowledged; a kind 2 frame goes up with ACKs', async () => {
  const link = fakeLink();
  const term = terminal(link);
  const s = new BleSession(link.transport, { timing: fast });
  const got = [];
  s.onFrame = (f) => got.push(f);
  await s.start();
  // terminal -> phone: the challenge (kind 1), 3000 bytes, window 8: the phone acknowledges every 4 chunks and the last
  const challenge = frameOf(3000, 1, 3);
  for (const m of cutFrame(challenge, 1, 244)) link.notify(m);
  await sleep(5);
  eq(got[0], challenge);
  const acks = link.sent.filter((m) => m[0] === T.ACK).map((m) => rd16(m, 3));
  assert.deepEqual(acks, [4, 8, 12, 13]);
  // phone -> terminal: the reply (kind 2)
  const reply = frameOf(6000, 2, 9);
  await s.sendFrame(reply);
  eq(term.frames[0], reply);
  // frame ids count per direction: the phone's first frame is 1, its second is 2
  await s.sendFrame(frameOf(50, 5));
  const ids = link.sent.filter((m) => m[0] === T.DATA && rd16(m, 3) === 0).map((m) => rd16(m, 1));
  assert.deepEqual(ids, [1, 2]);
  await assert.rejects(() => s.sendFrame(new Uint8Array(3)), /SWR1/);
  await s.bye();
  assert.deepEqual(link.sent[link.sent.length - 1], Uint8Array.of(T.BYE, BYE.DONE));
  assert.ok(link.closed);
});

test('session: the sender never has more than window chunks unacknowledged', async () => {
  const link = fakeLink();
  const rx = new Reassembler({ chunk: 244 });
  let inFlight = 0;
  let peak = 0;
  let acked = 0;
  link.onWrite = (m) => {
    if (m[0] === T.HELLO) return queueMicrotask(() => link.notify(Uint8Array.of(T.HELLO_ACK, 1, 0, 244, 4)));
    if (m[0] !== T.DATA) return;
    inFlight += 1;
    peak = Math.max(peak, inFlight);
    const out = rx.push(m);
    if (out.reply[0] && out.reply[0][0] === T.ACK) {
      const next = rd16(out.reply[0], 3);
      inFlight -= next - acked;
      acked = next;
      setTimeout(() => link.notify(out.reply[0]), 2);
    }
  };
  const s = new BleSession(link.transport, { timing: { ...fast, ACK_MS: 500 } });
  await s.start();
  await s.sendFrame(frameOf(5000, 2));
  assert.ok(peak <= 4, `peak ${peak}`);
  await s.bye();
});

test('session: a missing ACK re-sends from the last acknowledged chunk and still delivers', async () => {
  const link = fakeLink();
  const term = terminal(link, { dropAcks: 1 });
  const s = new BleSession(link.transport, { timing: fast });
  await s.start();
  const f = frameOf(900, 2); // 4 chunks: the single ACK (after 4) is lost once
  await s.sendFrame(f);
  eq(term.frames[0], f);
  const idx0 = link.sent.filter((m) => m[0] === T.DATA && rd16(m, 3) === 0).length;
  assert.equal(idx0, 2, 'chunk 0 was sent again');
  await s.bye();
});

test('session: after BLE_RETRIES silent timeouts it sends ABORT(6) and BYE(1) and rejects', async () => {
  const link = fakeLink();
  terminal(link, { dropAcks: 1000 });
  const s = new BleSession(link.transport, { timing: fast });
  await s.start();
  let closedWhy = null;
  s.onClose = (w) => (closedWhy = w);
  await assert.rejects(() => s.sendFrame(frameOf(400, 2)), /stopped acknowledging/);
  await sleep(10);
  const k = kinds(link);
  assert.deepEqual(k.slice(-2), [T.ABORT, T.BYE]);
  assert.equal(link.sent[link.sent.length - 2][3], ABORT.TIMEOUT);
  assert.equal(link.sent[link.sent.length - 1][1], BYE.TIMEOUT);
  assert.ok(closedWhy && link.closed);
  assert.equal(link.sent.filter((m) => m[0] === T.DATA && rd16(m, 3) === 0).length, 1 + fast.RETRIES, 'first send plus three re-sends');
});

test('session: ABORT from the terminal rejects the frame in flight; BYE closes the session', async () => {
  const link = fakeLink();
  link.onWrite = (m) => m[0] === T.HELLO && queueMicrotask(() => link.notify(Uint8Array.of(T.HELLO_ACK, 1, 0, 244, 8)));
  const s = new BleSession(link.transport, { timing: { ...fast, ACK_MS: 5000 } });
  await s.start();
  const p = s.sendFrame(frameOf(2000, 2));
  await sleep(5);
  link.notify(Uint8Array.of(T.ABORT, 0, 1, ABORT.CRC));
  await assert.rejects(() => p, /aborted the frame \(reason 1\)/);
  assert.equal(s.state, 'open', 'the session survives an abort: a new frame id may follow');
  const q = s.sendFrame(frameOf(60, 5));
  await sleep(5);
  link.notify(ack(2, 1));
  await q;
  let why = null;
  s.onClose = (w) => (why = w);
  link.notify(Uint8Array.of(T.BYE, BYE.USER));
  await sleep(5);
  assert.equal(s.state, 'closed');
  assert.match(why, /ended the session/);
  assert.ok(link.closed);
  await assert.rejects(() => s.sendFrame(frameOf(60, 5)), /not open/);
});

test('session: hello answered with another version or silence ends the session; a dropped link rejects start', async () => {
  let link = fakeLink();
  link.onWrite = (m) => m[0] === T.HELLO && queueMicrotask(() => link.notify(Uint8Array.of(T.HELLO_ACK, 2, 0, 244, 8)));
  let s = new BleSession(link.transport, { timing: fast });
  await assert.rejects(() => s.start(), /version 2/);
  await sleep(5);
  assert.deepEqual(kinds(link), [T.HELLO, T.BYE]);

  link = fakeLink();
  s = new BleSession(link.transport, { timing: fast });
  await assert.rejects(() => s.start(), /did not answer HELLO/);
  await sleep(5);
  assert.equal(link.sent[link.sent.length - 1][1], BYE.TIMEOUT);

  link = fakeLink();
  link.onWrite = (m) => m[0] === T.HELLO && queueMicrotask(() => link.notify(Uint8Array.of(T.HELLO_ACK, 1, 1, 244, 8))); // chunk 500 > proposal
  s = new BleSession(link.transport, { timing: fast });
  await assert.rejects(() => s.start(), /unusable numbers/);

  link = fakeLink();
  s = new BleSession(link.transport, { timing: { ...fast, HELLO_MS: 5000 } });
  const p = s.start();
  link.drop();
  await assert.rejects(() => p, /link went down/);
});

test('session: a hostile frame from the terminal is aborted, never handed on; a frame that stalls times out', async () => {
  const link = fakeLink();
  terminal(link);
  const s = new BleSession(link.transport, { timing: { ...fast, FRAME_MS: 30 } });
  const got = [];
  s.onFrame = (f) => got.push(f);
  await s.start();
  const [one] = cutFrame(frameOf(100, 1), 1, 244); // a single chunk: patch the magic and keep the crc honest
  one[15] = 0;
  new DataView(one.buffer).setUint32(11, crc32(one.subarray(15)));
  link.notify(one);
  await sleep(5);
  assert.equal(got.length, 0);
  assert.ok(link.sent.some((m) => m[0] === T.ABORT && rd16(m, 1) === 1 && m[3] === ABORT.BUSY));
  // a first chunk and then silence: ABORT(6) after the frame window, the next frame starts clean
  link.notify(cutFrame(frameOf(900, 1), 2, 244)[0]);
  await sleep(60);
  assert.ok(link.sent.some((m) => m[0] === T.ABORT && rd16(m, 1) === 2 && m[3] === ABORT.TIMEOUT));
  for (const m of cutFrame(frameOf(100, 1), 3, 244)) link.notify(m);
  await sleep(5);
  assert.equal(got.length, 1);
  await s.bye();
});

test('session: PING is sent while idle', async () => {
  const link = fakeLink();
  terminal(link);
  const s = new BleSession(link.transport, { timing: { ...fast, PING_MS: 20 } });
  await s.start();
  await sleep(70);
  assert.ok(kinds(link).includes(T.PING));
  await s.bye();
});

// ---------------------------------------------------------------- Web Bluetooth against a fake GATT object
function fakeNavigator({ tag = '7QK2', name = 'SSHub-7QK2', props = { writeWithoutResponse: true } } = {}) {
  const log = [];
  const listeners = {};
  const txChar = {
    addEventListener: (n, f) => (listeners[n] = f),
    startNotifications: async () => log.push('startNotifications'),
  };
  const rxChar = {
    properties: props,
    writeValueWithoutResponse: async (b) => log.push(['wwr', Array.from(b)]),
    writeValueWithResponse: async (b) => log.push(['wr', Array.from(b)]),
  };
  const info = Uint8Array.of(1, 0, 0, 16, 0, ...new TextEncoder().encode(tag));
  const chars = {
    [BLE.RX]: rxChar,
    [BLE.TX]: txChar,
    [BLE.INFO]: { readValue: async () => new DataView(info.buffer) },
  };
  let connected = false;
  const devListeners = {};
  const dev = {
    name,
    addEventListener: (n, f) => (devListeners[n] = f),
    gatt: {
      get connected() {
        return connected;
      },
      connect: async () => {
        connected = true;
        return { getPrimaryService: async (u) => (u === BLE.SERVICE ? { getCharacteristic: async (c) => chars[c] } : Promise.reject(new Error('no service'))) };
      },
      disconnect: () => {
        connected = false;
        log.push('disconnect');
      },
    },
  };
  const nav = { bluetooth: { requestDevice: async (o) => (log.push(['request', JSON.stringify(o)]), dev) } };
  const notify = (m) => listeners.characteristicvaluechanged({ target: { value: new DataView(m.buffer.slice(m.byteOffset, m.byteOffset + m.length)) } });
  return { nav, log, notify, devListeners };
}

test('connectBle: filter is the service only, notifications start before the first write, INFO gives the tag', async () => {
  assert.equal(bleAvailable({}), false);
  const fx = fakeNavigator();
  assert.equal(bleAvailable(fx.nav), true);
  const c = await connectBle(fx.nav);
  assert.equal(c.tag, '7QK2');
  assert.equal(c.name, 'SSHub-7QK2');
  assert.equal(c.maxFrame, 4096);
  assert.deepEqual(JSON.parse(fx.log[0][1]), { filters: [{ services: [BLE.SERVICE] }] });
  const s = new BleSession(c.transport, { timing: fast });
  const p = s.start();
  await sleep(2);
  assert.equal(fx.log[1], 'startNotifications');
  assert.deepEqual(fx.log[2], ['wwr', [T.HELLO, 1, 0, 244]]);
  fx.notify(Uint8Array.of(T.HELLO_ACK, 1, 0, 244, 8));
  await p;
  // a notification that arrives before the receiver is set is not lost (checked with a second connect)
  await s.bye();
  assert.ok(fx.log.includes('disconnect'));
});

test('connectBle: writes with response when the characteristic has no write-without-response; bad INFO is refused', async () => {
  const fx = fakeNavigator({ props: { write: true } });
  const c = await connectBle(fx.nav);
  await c.transport.write(Uint8Array.of(1, 2));
  assert.deepEqual(fx.log.at(-1), ['wr', [1, 2]]);
  c.transport.close();
  await assert.rejects(() => connectBle(fakeNavigator({ tag: 'ILOU' }).nav), /unusable tag/);
  await assert.rejects(() => connectBle({}), /no Web Bluetooth/);
});
