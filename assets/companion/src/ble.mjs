// Bluetooth Low Energy frame channel, the phone (Web Bluetooth central) side of contract C20.
// It moves whole frames ("SWR1" || version || kind || body, kinds 1 to 12) unchanged, in both directions. The link adds no
// security and assumes none: no pairing, and every frame is signed or sealed end to end as on a camera or a speaker.
// Three layers, each testable alone:
//   cutFrame / Reassembler   pure chunking and the receiver rules
//   BleSession               HELLO, windowed sending with ACK retries, PING, timers, over a transport object
//   connectBle               navigator.bluetooth -> a transport ({ write, setReceiver, setOnClose, close })
// The transport object is the only seam: { write(Uint8Array): Promise, setReceiver(fn), setOnClose(fn), close() }.

export const BLE = {
  SERVICE: 'b62f7d3f-0001-42dd-8371-6ffd96411583',
  RX: 'b62f7d3f-0002-42dd-8371-6ffd96411583', // phone to terminal
  TX: 'b62f7d3f-0003-42dd-8371-6ffd96411583', // terminal to phone (notify)
  INFO: 'b62f7d3f-0004-42dd-8371-6ffd96411583', // proto u8 || max_frame u32 BE || tag(4 ASCII)
  CHUNK_DEFAULT: 244,
  CHUNK_MIN: 20,
  CHUNK_MAX: 512,
  WINDOW: 8,
  MAX_FRAME_BYTES: 1 << 20,
  HELLO_MS: 10_000,
  ACK_MS: 5_000,
  RETRIES: 3,
  FRAME_MS: 120_000,
  PING_MS: 30_000,
  IDLE_MS: 90_000,
};

const T = { HELLO: 0x01, HELLO_ACK: 0x02, DATA: 0x10, ACK: 0x11, ABORT: 0x12, BYE: 0x20, PING: 0x22 };
export const ABORT = { CRC: 1, TOO_LARGE: 2, BUSY: 3, ORDER: 4, VERSION: 5, TIMEOUT: 6 };
export const BYE = { DONE: 0, TIMEOUT: 1, USER: 2, ERROR: 3, BUSY: 4 };

const MAGIC = [0x53, 0x57, 0x52, 0x31]; // "SWR1"
const TAG_RE = /^[0-9A-HJKMNP-TV-Z]{4}$/; // Crockford base32: no I L O U

// ---------------------------------------------------------------- crc32 (IEEE 802.3)
const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();
export function crc32(bytes) {
  let c = 0xffffffff;
  for (let i = 0; i < bytes.length; i++) c = CRC_TABLE[(c ^ bytes[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

const u16 = (n) => [(n >>> 8) & 0xff, n & 0xff];
const u32 = (n) => [(n >>> 24) & 0xff, (n >>> 16) & 0xff, (n >>> 8) & 0xff, n & 0xff];
const rd16 = (b, at) => (b[at] << 8) | b[at + 1];
const rd32 = (b, at) => ((b[at] << 24) | (b[at + 1] << 16) | (b[at + 2] << 8) | b[at + 3]) >>> 0;

// A whole-frame check shared by both directions: the first four bytes are "SWR1" and the kind is 1..=12.
export const frameLooksValid = (f) => f.length >= 7 && MAGIC.every((m, i) => f[i] === m) && f[6] >= 1 && f[6] <= 12;

// ---------------------------------------------------------------- chunking
// count = 1 + ceil(max(0, len - (chunk - 15)) / (chunk - 7))
export const chunkCount = (len, chunk) => 1 + Math.ceil(Math.max(0, len - (chunk - 15)) / (chunk - 7));

// The DATA messages of one frame. Chunk 0 carries len and crc32 after the 7 byte header.
export function cutFrame(frame, frameId, chunk) {
  if (!(chunk >= BLE.CHUNK_MIN && chunk <= BLE.CHUNK_MAX)) throw new Error(`chunk size ${chunk} is outside ${BLE.CHUNK_MIN}..${BLE.CHUNK_MAX}`);
  if (!frameLooksValid(frame)) throw new Error('not a SSHub frame (SWR1, kind 1 to 12)');
  if (frame.length > BLE.MAX_FRAME_BYTES) throw new Error('frame is larger than 1 MiB');
  const count = chunkCount(frame.length, chunk);
  if (count > 0xffff) throw new Error(`the frame needs ${count} chunks of ${chunk} bytes, more than the 65535 a frame may have`);
  const out = [];
  let at = 0;
  for (let idx = 0; idx < count; idx++) {
    const head = [T.DATA, ...u16(frameId), ...u16(idx), ...u16(count)];
    if (idx === 0) head.push(...u32(frame.length), ...u32(crc32(frame)));
    const take = Math.min(chunk - head.length, frame.length - at);
    const msg = new Uint8Array(head.length + take);
    msg.set(head);
    msg.set(frame.subarray(at, at + take), head.length);
    at += take;
    out.push(msg);
  }
  return out;
}

const ackMsg = (id, next) => Uint8Array.of(T.ACK, ...u16(id), ...u16(next));
const abortMsg = (id, reason) => Uint8Array.of(T.ABORT, ...u16(id), reason);
const byeMsg = (reason) => Uint8Array.of(T.BYE, reason);

// The receiver rules of C20, one direction. push(DATA message) -> { reply: [messages to send], frame?: whole frame }.
// Memory held is one frame, released on every abort.
export class Reassembler {
  constructor({ chunk = BLE.CHUNK_DEFAULT, window = BLE.WINDOW, maxFrame = BLE.MAX_FRAME_BYTES } = {}) {
    Object.assign(this, { chunk, window, maxFrame });
    this.reset();
  }

  reset() {
    this.cur = null; // { id, count, len, crc, next, parts, have }
    this.done = null; // { id, count } of the last completed frame (a repeated last chunk is acknowledged again)
  }

  abortCurrent() {
    this.cur = null;
  }

  push(m) {
    if (m.length < 7 || m[0] !== T.DATA) return { reply: [] };
    const id = rd16(m, 1);
    const idx = rd16(m, 3);
    const count = rd16(m, 5);
    const c = this.cur;
    if (c && id !== c.id) {
      return { reply: [abortMsg(id, ABORT.BUSY)] };
    }
    if (!c) {
      if (this.done && id === this.done.id) return { reply: [ackMsg(id, this.done.count)] };
      if (idx !== 0) return { reply: [ackMsg(id, 0)] };
      if (m.length < 15) return { reply: [abortMsg(id, ABORT.TOO_LARGE)] };
      const len = rd32(m, 7);
      const crc = rd32(m, 11);
      if (len > this.maxFrame || len < 7 || count !== chunkCount(len, this.chunk)) return { reply: [abortMsg(id, ABORT.TOO_LARGE)] };
      this.cur = { id, count, len, crc, next: 0, parts: [], have: 0 };
      this.done = null;
    }
    const cur = this.cur;
    if (idx !== cur.next || count !== cur.count) return { reply: [ackMsg(cur.id, cur.next)] };
    const payload = m.subarray(idx === 0 ? 15 : 7);
    if (m.length > this.chunk || cur.have + payload.length > cur.len) return this.fail(ABORT.CRC);
    cur.parts.push(payload.slice());
    cur.have += payload.length;
    cur.next += 1;
    const last = cur.next === cur.count;
    if (!last) return { reply: cur.next % Math.max(1, this.window >> 1) === 0 ? [ackMsg(cur.id, cur.next)] : [] };
    if (cur.have !== cur.len) return this.fail(ABORT.CRC);
    const frame = new Uint8Array(cur.len);
    let at = 0;
    for (const p of cur.parts) {
      frame.set(p, at);
      at += p.length;
    }
    if (crc32(frame) !== cur.crc) return this.fail(ABORT.CRC);
    if (!frameLooksValid(frame)) return this.fail(ABORT.BUSY);
    this.done = { id: cur.id, count: cur.count };
    this.cur = null;
    return { reply: [ackMsg(this.done.id, this.done.count)], frame };
  }

  fail(reason) {
    const id = this.cur.id;
    this.cur = null;
    return { reply: [abortMsg(id, reason)] };
  }
}

// ---------------------------------------------------------------- session
// One central session. Writes are serialised through one queue, never concurrent.
export class BleSession {
  constructor(transport, { chunk = BLE.CHUNK_DEFAULT, maxFrame = BLE.MAX_FRAME_BYTES, timing = {}, timers = globalThis } = {}) {
    this.t = transport;
    this.tm = { ...BLE, ...timing };
    this.set = (f, ms) => timers.setTimeout(f, ms);
    this.clear = (h) => timers.clearTimeout(h);
    this.proposal = Math.min(Math.max(chunk, BLE.CHUNK_MIN), BLE.CHUNK_MAX);
    this.maxFrame = Math.min(maxFrame, BLE.MAX_FRAME_BYTES);
    this.chunk = BLE.CHUNK_MIN; // until HELLO_ACK every message is at most 20 bytes
    this.window = BLE.WINDOW;
    this.state = 'new'; // new | hello | open | closed
    this.nextId = 1;
    this.out = null;
    this.inbox = null;
    this.onFrame = null;
    this.onClose = null;
    this.q = Promise.resolve();
    this.timers = {};
    transport.setReceiver((m) => this.receive(m));
    transport.setOnClose(() => this.end('the link went down', false));
  }

  send(msg) {
    this.arm('ping', this.tm.PING_MS, () => this.write(Uint8Array.of(T.PING)));
    const r = this.q.then(() => (this.state === 'closed' && msg[0] !== T.BYE && msg[0] !== T.ABORT ? undefined : this.t.write(msg)));
    this.q = r.catch(() => {});
    return r;
  }

  write(msg) {
    return this.send(msg).catch((e) => this.end(`write failed: ${e && e.message ? e.message : e}`, false));
  }

  arm(name, ms, fn) {
    this.clear(this.timers[name]);
    this.timers[name] = this.state === 'closed' ? null : this.set(fn, ms);
  }

  // HELLO, then wait for HELLO_ACK within the hello window. Resolves { chunk, window }.
  start() {
    if (this.state !== 'new') throw new Error('session already started');
    this.state = 'hello';
    return new Promise((resolve, reject) => {
      this.helloDone = { resolve, reject };
      this.arm('hello', this.tm.HELLO_MS, () => this.end('the terminal did not answer HELLO', true, BYE.TIMEOUT));
      this.arm('idle', this.tm.IDLE_MS, () => this.end('nothing heard from the terminal', true, BYE.TIMEOUT));
      this.send(Uint8Array.of(T.HELLO, 1, ...u16(this.proposal))).catch((e) => this.end(`write failed: ${e && e.message ? e.message : e}`, false));
    });
  }

  receive(m) {
    if (this.state === 'closed' || !m.length) return;
    this.arm('idle', this.tm.IDLE_MS, () => this.end('nothing heard from the terminal', true, BYE.TIMEOUT));
    switch (m[0]) {
      case T.HELLO_ACK: {
        if ((this.state !== 'hello' && this.state !== 'open') || m.length < 5) return;
        const [ver, chunk, window] = [m[1], rd16(m, 2), m[4]];
        if (ver !== 1) return this.end(`the terminal speaks version ${ver}`, true, BYE.ERROR);
        if (chunk < BLE.CHUNK_MIN || chunk > this.proposal || window < 1) return this.end('the terminal answered HELLO with unusable numbers', true, BYE.ERROR);
        const restart = this.state === 'open';
        this.chunk = chunk;
        this.window = window;
        this.inbox = new Reassembler({ chunk, window, maxFrame: this.maxFrame });
        this.clear(this.timers.frame);
        this.state = 'open';
        if (restart) {
          // C20: a HELLO_ACK while open restarts the session: the half-received frame is forgotten and the frame in
          // flight is queued again under a new frame_id.
          const o = this.out;
          if (o) {
            let msgs;
            try {
              o.id = this.nextId;
              this.nextId = o.id === 0xffff ? 1 : o.id + 1;
              msgs = cutFrame(o.frame, o.id, chunk);
            } catch (e) {
              return this.finishOut(e);
            }
            Object.assign(o, { msgs, acked: 0, sent: 0, high: 0, went: false, retries: 0 });
            this.pump();
          }
          return;
        }
        this.clear(this.timers.hello);
        this.helloDone.resolve({ chunk, window });
        this.helloDone = null;
        return;
      }
      case T.DATA: {
        if (this.state !== 'open') return;
        const had = !!this.inbox.cur;
        const r = this.inbox.push(m);
        // The frame window starts when the frame starts: a repeated chunk 0 does not re-arm it.
        if (!had && this.inbox.cur) this.arm('frame', this.tm.FRAME_MS, () => {
          const id = this.inbox.cur && this.inbox.cur.id;
          this.inbox.abortCurrent();
          if (id) this.write(abortMsg(id, ABORT.TIMEOUT));
        });
        for (const x of r.reply) this.write(x);
        if (r.frame) {
          this.clear(this.timers.frame);
          if (this.onFrame) this.onFrame(r.frame);
        } else if (!this.inbox.cur) this.clear(this.timers.frame);
        return;
      }
      case T.ACK:
        return m.length >= 5 ? this.gotAck(rd16(m, 1), rd16(m, 3)) : undefined;
      case T.ABORT: {
        const id = rd16(m, 1);
        if (this.out && this.out.id === id) this.finishOut(new Error(`the terminal aborted the frame (reason ${m[3]})`));
        else if (this.inbox && this.inbox.cur && this.inbox.cur.id === id) this.inbox.abortCurrent();
        return;
      }
      case T.BYE:
        return this.end(`the terminal ended the session (reason ${m[1]})`, false);
      case T.PING:
      default:
    }
  }

  // Sends one frame and resolves when the terminal acknowledged the last chunk.
  sendFrame(frame) {
    if (this.state !== 'open') return Promise.reject(new Error('the Bluetooth session is not open'));
    if (this.out) return Promise.reject(new Error('a frame is already being sent'));
    if (frame.length > this.maxFrame) return Promise.reject(new Error('the frame is larger than the terminal accepts'));
    const id = this.nextId;
    this.nextId = id === 0xffff ? 1 : id + 1;
    let msgs;
    try {
      msgs = cutFrame(frame, id, this.chunk);
    } catch (e) {
      return Promise.reject(e);
    }
    return new Promise((resolve, reject) => {
      this.out = { id, frame, msgs, acked: 0, sent: 0, high: 0, went: false, retries: 0, resolve, reject, pumping: false };
      this.pump();
    });
  }

  async pump() {
    const o = this.out;
    if (!o || o.pumping) return;
    o.pumping = true;
    try {
      while (this.out === o && o.sent < o.msgs.length && o.sent - o.acked < this.window) {
        const msg = o.msgs[o.sent];
        o.sent += 1;
        o.high = Math.max(o.high, o.sent);
        await this.send(msg);
        if (this.out === o) this.arm('ack', this.tm.ACK_MS, () => this.ackTimeout(o));
      }
    } catch (e) {
      if (this.out === o) this.finishOut(new Error(`write failed: ${e && e.message ? e.message : e}`));
    } finally {
      o.pumping = false;
    }
  }

  gotAck(id, next) {
    const o = this.out;
    // Any ACK up to the highest chunk ever sent counts (it may answer chunks sent before a go-back); beyond that the
    // peer claims chunks that never left this phone.
    if (!o || o.id !== id || next > o.high) return;
    if (next === o.acked) {
      // C20: an ACK equal to the last acknowledged next while chunks are outstanding means a gap: go back once, then
      // wait for progress before doing so again.
      if (o.high > o.acked && !o.went) {
        o.went = true;
        o.sent = o.acked;
        this.pump();
      }
      return;
    }
    if (next < o.acked) return;
    o.acked = next;
    o.retries = 0;
    o.went = false;
    if (next === o.msgs.length) return this.finishOut(null);
    this.arm('ack', this.tm.ACK_MS, () => this.ackTimeout(o));
    this.pump();
  }

  ackTimeout(o) {
    if (this.out !== o) return;
    o.retries += 1;
    if (o.retries > this.tm.RETRIES) {
      this.write(abortMsg(o.id, ABORT.TIMEOUT));
      return this.end('the terminal stopped acknowledging', true, BYE.TIMEOUT);
    }
    o.sent = o.acked; // resend from the last acknowledged chunk
    this.pump();
  }

  finishOut(err) {
    const o = this.out;
    this.out = null;
    this.clear(this.timers.ack);
    if (!o) return;
    if (err) o.reject(err);
    else o.resolve();
  }

  // Ends the session. say: send BYE(reason) first.
  end(why, say, reason = BYE.DONE) {
    if (this.state === 'closed') return Promise.resolve();
    const hello = this.helloDone;
    this.helloDone = null;
    const last = say ? this.send(byeMsg(reason)).catch(() => {}) : Promise.resolve();
    this.state = 'closed';
    for (const k of Object.keys(this.timers)) this.clear(this.timers[k]);
    const err = new Error(why);
    if (hello) hello.reject(err);
    if (this.out) this.finishOut(err);
    if (this.inbox) this.inbox.reset();
    return last.then(() => {
      try {
        this.t.close();
      } catch {
        /* already down */
      }
      if (this.onClose) this.onClose(why);
    });
  }

  // The phone page after its last step: BYE(0), disconnect.
  bye(reason = BYE.DONE) {
    return this.end('closed by this page', true, reason);
  }
}

// ---------------------------------------------------------------- Web Bluetooth
export const bleAvailable = (nav = globalThis.navigator) => !!(nav && nav.bluetooth && typeof nav.bluetooth.requestDevice === 'function');

const bytesOfView = (v) => new Uint8Array(v.buffer, v.byteOffset, v.byteLength).slice();

// Must run inside a user gesture. The browser's chooser lists devices that advertise the service; the user picks
// "SSHub-<TAG>". Returns { name, tag, maxFrame, transport }; the page shows the tag so the person compares it with the terminal.
export async function connectBle(nav = globalThis.navigator) {
  if (!bleAvailable(nav)) throw new Error('this browser has no Web Bluetooth');
  const dev = await nav.bluetooth.requestDevice({ filters: [{ services: [BLE.SERVICE] }] });
  const server = await dev.gatt.connect();
  try {
    const svc = await server.getPrimaryService(BLE.SERVICE);
    const info = bytesOfView(await (await svc.getCharacteristic(BLE.INFO)).readValue());
    if (info.length < 9 || info[0] !== 1) throw new Error('the terminal speaks another Bluetooth protocol version');
    const maxFrame = rd32(info, 1);
    const tag = String.fromCharCode(...info.slice(5, 9));
    if (!TAG_RE.test(tag)) throw new Error('the terminal sent an unusable tag');
    const rx = await svc.getCharacteristic(BLE.RX);
    const tx = await svc.getCharacteristic(BLE.TX);
    let recv = null;
    let closed = null;
    const early = [];
    tx.addEventListener('characteristicvaluechanged', (e) => {
      const m = bytesOfView(e.target.value);
      if (recv) recv(m);
      else early.push(m);
    });
    dev.addEventListener('gattserverdisconnected', () => closed && closed());
    await tx.startNotifications(); // before the first write to RX
    const noResponse = rx.properties && rx.properties.writeWithoutResponse && typeof rx.writeValueWithoutResponse === 'function';
    const transport = {
      write: (b) => (noResponse ? rx.writeValueWithoutResponse(b) : typeof rx.writeValueWithResponse === 'function' ? rx.writeValueWithResponse(b) : rx.writeValue(b)),
      setReceiver(fn) {
        recv = fn;
        for (const m of early.splice(0)) fn(m);
      },
      setOnClose(fn) {
        closed = fn;
      },
      close() {
        if (dev.gatt.connected) dev.gatt.disconnect();
      },
    };
    return { name: dev.name || '', tag, maxFrame, transport };
  } catch (e) {
    if (dev.gatt.connected) dev.gatt.disconnect();
    throw e;
  }
}
