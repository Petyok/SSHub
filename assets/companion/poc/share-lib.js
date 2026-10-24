// Shared harness for PoC C (QR budget) and PoC H (share channels).
// Plain script; everything hangs off window.ShareLib. No network, no state on disk.
// Vendored libraries (see vendor/VENDORED.md): qrcode-generator (MIT), jsQR (Apache-2.0).
(function (global) {
  "use strict";

  // ---- CRC-16/CCITT-FALSE (poly 0x1021, init 0xFFFF). Check value of "123456789" is 0x29B1.
  function crc16(bytes) {
    let crc = 0xffff;
    for (let i = 0; i < bytes.length; i++) {
      crc ^= bytes[i] << 8;
      for (let b = 0; b < 8; b++) {
        crc = crc & 0x8000 ? ((crc << 1) ^ 0x1021) & 0xffff : (crc << 1) & 0xffff;
      }
    }
    return crc;
  }

  function randomBytes(n) {
    const out = new Uint8Array(n);
    crypto.getRandomValues(out);
    return out;
  }

  // Deterministic bytes from a 32-bit seed (xorshift32). Sender and checker derive the same payload.
  // Not a secret: used only so both ends can compare against a known expected value.
  function seededBytes(n, seed) {
    const out = new Uint8Array(n);
    let x = seed >>> 0 || 1;
    for (let i = 0; i < n; i++) {
      x ^= x << 13; x >>>= 0; x ^= x >>> 17; x ^= x << 5; x >>>= 0;
      out[i] = x & 0xff;
    }
    return out;
  }

  async function sha256Hex(bytes) {
    return hex(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)));
  }

  function hex(bytes) {
    return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
  }

  function equalBytes(a, b) {
    if (!a || !b || a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
    return true;
  }

  // ---- framing: [LEN:1][payload][CRC16 BE over LEN+payload]
  function frame(payload) {
    if (payload.length > 255) throw new Error("frame: payload over 255 bytes");
    const out = new Uint8Array(payload.length + 3);
    out[0] = payload.length;
    out.set(payload, 1);
    const c = crc16(out.subarray(0, payload.length + 1));
    out[payload.length + 1] = c >> 8;
    out[payload.length + 2] = c & 0xff;
    return out;
  }

  // Returns { ok: true, payload } or { ok: false, error }.
  function deframe(bytes) {
    if (bytes.length < 3) return { ok: false, error: "short" };
    const len = bytes[0];
    if (bytes.length < len + 3) return { ok: false, error: "truncated" };
    const want = (bytes[len + 1] << 8) | bytes[len + 2];
    if (crc16(bytes.subarray(0, len + 1)) !== want) return { ok: false, error: "crc" };
    return { ok: true, payload: bytes.slice(1, len + 1) };
  }

  // ---- indexed chunks, looped by the sender. Header: [idx:2][total:2][chunkCRC:2] BE, then data.
  // NOTE: this is indexed repetition, not a rateless fountain code. A missing index is recovered
  // only when the sender loops back to it.
  const CHUNK_HDR = 6;

  function fountainChunks(bytes, chunkSize) {
    const total = Math.ceil(bytes.length / chunkSize);
    if (total > 0xffff) throw new Error("fountainChunks: too many chunks");
    const frames = [];
    for (let idx = 0; idx < total; idx++) {
      const data = bytes.subarray(idx * chunkSize, Math.min(bytes.length, (idx + 1) * chunkSize));
      const f = new Uint8Array(CHUNK_HDR + data.length);
      const c = crc16(data);
      f[0] = idx >> 8; f[1] = idx & 0xff;
      f[2] = total >> 8; f[3] = total & 0xff;
      f[4] = c >> 8; f[5] = c & 0xff;
      f.set(data, CHUNK_HDR);
      frames.push(f);
    }
    return frames;
  }

  // Stateful reassembler: tolerant of order, duplicates, gaps and corrupt frames.
  class Reassembler {
    constructor() { this.total = -1; this.parts = new Map(); this.bad = 0; this.dupes = 0; this.bytes = null; }
    add(f) {
      if (this.bytes) return this.status();
      if (f.length < CHUNK_HDR) { this.bad++; return this.status(); }
      const idx = (f[0] << 8) | f[1];
      const total = (f[2] << 8) | f[3];
      const want = (f[4] << 8) | f[5];
      const data = f.slice(CHUNK_HDR);
      if (crc16(data) !== want || idx >= total || total === 0) { this.bad++; return this.status(); }
      if (this.total === -1) this.total = total;
      else if (this.total !== total) { this.bad++; return this.status(); }
      if (this.parts.has(idx)) { this.dupes++; return this.status(); }
      this.parts.set(idx, data);
      if (this.parts.size === this.total) {
        let n = 0;
        for (let i = 0; i < this.total; i++) n += this.parts.get(i).length;
        const out = new Uint8Array(n);
        let o = 0;
        for (let i = 0; i < this.total; i++) { out.set(this.parts.get(i), o); o += this.parts.get(i).length; }
        this.bytes = out;
      }
      return this.status();
    }
    status() {
      return { have: this.parts.size, total: this.total, complete: this.bytes !== null, bytes: this.bytes };
    }
  }

  function fountainReassemble(frames) {
    const r = new Reassembler();
    for (const f of frames) r.add(f);
    return r.bytes;
  }

  // ---- MFSK audio modem: 16 tones (4 bits/symbol), 1600 Hz + i*100 Hz, raised-cosine edges,
  // 1000 Hz sync tone as preamble and timing anchor. Tone spacing is 100 Hz, an integer multiple
  // of 1/symbol for both profiles (60 ms and 30 ms), so tones stay orthogonal over a symbol.
  const FSK = {
    sampleRate: 48000, base: 1600, step: 100, nTones: 16,
    symMs: 60, rampMs: 5, preambleMs: 300, syncHz: 1000, amp: 0.5, hopMs: 10,
  };
  // Second profile: half-length symbols, shorter ramps. Same tones and sync.
  const FSK_FAST = Object.assign({}, FSK, { symMs: 30, rampMs: 3 });

  function ramped(n, N, R) {
    // raised-cosine envelope over R samples at each end of an N-sample symbol
    if (n < R) return 0.5 * (1 - Math.cos((Math.PI * n) / R));
    if (n >= N - R) return 0.5 * (1 - Math.cos((Math.PI * (N - 1 - n)) / R));
    return 1;
  }

  function toneInto(out, off, freq, N, R) {
    const w = (2 * Math.PI * freq) / FSK.sampleRate;
    for (let n = 0; n < N; n++) out[off + n] = FSK.amp * ramped(n, N, R) * Math.sin(w * n);
  }

  function fskEncode(bytes, F = FSK) {
    const sr = FSK.sampleRate;
    const N = Math.round((sr * F.symMs) / 1000);
    const R = Math.round((sr * F.rampMs) / 1000);
    const P = Math.round((sr * FSK.preambleMs) / 1000);
    const nibbles = [];
    for (const b of bytes) { nibbles.push(b >> 4, b & 0x0f); }
    const out = new Float32Array(P + nibbles.length * N);
    toneInto(out, 0, FSK.syncHz, P, R);
    nibbles.forEach((s, i) => toneInto(out, P + i * N, FSK.base + FSK.step * s, N, R));
    return out;
  }

  // Goertzel power of `freq` over samples[from .. from+n).
  function goertzel(samples, from, n, freq) {
    const w = (2 * Math.PI * freq) / FSK.sampleRate;
    const coeff = 2 * Math.cos(w);
    let s1 = 0, s2 = 0;
    for (let i = 0; i < n; i++) {
      const s0 = samples[from + i] + coeff * s1 - s2;
      s2 = s1; s1 = s0;
    }
    return s1 * s1 + s2 * s2 - coeff * s1 * s2;
  }

  // Returns { ok: true, payload, startMs, nibbles } or { ok: false, error }.
  function fskDecode(samples, F = FSK) {
    const sr = FSK.sampleRate;
    const N = Math.round((sr * F.symMs) / 1000);
    const P = Math.round((sr * FSK.preambleMs) / 1000);
    const hop = Math.round((sr * FSK.hopMs) / 1000);
    const win = hop * 4;
    // 1. locate the sync tone: first hop whose sync power reaches half the maximum
    const power = [];
    let maxP = 0;
    for (let k = 0; k + win <= samples.length; k += hop) {
      const p = goertzel(samples, k, win, FSK.syncHz);
      power.push(p);
      if (p > maxP) maxP = p;
    }
    if (maxP === 0) return { ok: false, error: "no-sync" };
    let onset = -1;
    for (let k = 0; k < power.length; k++) if (power[k] >= 0.5 * maxP) { onset = k * hop; break; }
    // the 300 ms preamble nominally ends at onset + P
    // coarse start is within one analysis window of the true preamble end; refine by
    // maximising tone energy over the first four symbols (1 ms steps)
    const coarse = onset + P;
    const energyAt = (s, count) => {
      let e = 0;
      for (let i = 0; i < count; i++) {
        let bestP = 0;
        for (let t = 0; t < FSK.nTones; t++) {
          const p = goertzel(samples, s + i * N, N, FSK.base + FSK.step * t);
          if (p > bestP) bestP = p;
        }
        e += bestP;
      }
      return e;
    };
    let start = coarse, bestE = -1;
    for (let s = coarse - win - hop; s <= coarse + hop; s += 48) {
      if (s < 0 || s + 4 * N > samples.length) continue;
      const e = energyAt(s, 4);
      if (e > bestE) { bestE = e; start = s; }
    }
    const symbolsAvail = Math.floor((samples.length - start) / N);
    const readNibbles = (count) => {
      if (count > symbolsAvail) return null;
      const out = [];
      for (let i = 0; i < count; i++) {
        const off = start + i * N;
        let best = -1, bestP = -1;
        for (let t = 0; t < FSK.nTones; t++) {
          const p = goertzel(samples, off, N, FSK.base + FSK.step * t);
          if (p > bestP) { bestP = p; best = t; }
        }
        out.push(best);
      }
      return out;
    };
    const head = readNibbles(2);
    if (!head) return { ok: false, error: "short-signal" };
    const len = (head[0] << 4) | head[1];
    const total = (1 + len + 2) * 2;
    const all = readNibbles(total);
    if (!all) return { ok: false, error: "short-signal" };
    const bytes = new Uint8Array(1 + len + 2);
    for (let i = 0; i < bytes.length; i++) bytes[i] = (all[2 * i] << 4) | all[2 * i + 1];
    const d = deframe(bytes);
    return d.ok ? { ok: true, payload: d.payload, startMs: (onset * 1000) / sr, nibbles: all.length } : { ok: false, error: d.error };
  }

  // Additive white Gaussian noise at a given SNR relative to the buffer's mean power.
  function addNoise(samples, snrDb) {
    let p = 0;
    for (let i = 0; i < samples.length; i++) p += samples[i] * samples[i];
    p /= samples.length;
    const sigma = Math.sqrt(p / Math.pow(10, snrDb / 10));
    const out = new Float32Array(samples.length);
    for (let i = 0; i < samples.length; i++) {
      const u = 1 - Math.random(), v = Math.random();
      const g = Math.sqrt(-2 * Math.log(u)) * Math.cos(2 * Math.PI * v);
      out[i] = samples[i] + sigma * g;
    }
    return out;
  }

  // ---- QR helpers. qrcode-generator 'Byte' mode keeps the low 8 bits of each char, so a
  // latin1 string built from raw bytes is a binary-safe payload.
  function qrFor(bytes, version, ecl) {
    const qr = qrcode(version, ecl || "M");
    qr.addData(String.fromCharCode.apply(null, Array.from(bytes)), "Byte");
    qr.make();
    return qr;
  }

  // Largest byte payload that fits a fixed QR version at the given EC level.
  function byteCapacity(version, ecl) {
    let lo = 0, hi = 3000;
    const fits = (n) => {
      try {
        const q = qrcode(version, ecl || "M");
        q.addData(String.fromCharCode.apply(null, new Array(n).fill(65)), "Byte");
        q.make();
        return true;
      } catch (e) { return false; }
    };
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (fits(mid)) lo = mid; else hi = mid - 1;
    }
    return lo;
  }

  // Render a QR to a canvas: `scale` px per module, `quiet` modules of margin.
  function drawQR(canvas, qr, scale, quiet) {
    const n = qr.getModuleCount();
    const size = (n + 2 * quiet) * scale;
    canvas.width = size; canvas.height = size;
    const ctx = canvas.getContext("2d");
    ctx.fillStyle = "#fff";
    ctx.fillRect(0, 0, size, size);
    ctx.fillStyle = "#000";
    for (let r = 0; r < n; r++) for (let c = 0; c < n; c++) {
      if (qr.isDark(r, c)) ctx.fillRect((c + quiet) * scale, (r + quiet) * scale, scale, scale);
    }
    return canvas;
  }

  // Decode a canvas in memory with jsQR. Returns the raw bytes or null.
  function scanCanvas(canvas) {
    const ctx = canvas.getContext("2d");
    const img = ctx.getImageData(0, 0, canvas.width, canvas.height);
    const r = jsQR(img.data, img.width, img.height, { inversionAttempts: "attemptBoth" });
    return r && r.binaryData ? Uint8Array.from(r.binaryData) : null;
  }

  global.ShareLib = {
    crc16, randomBytes, seededBytes, sha256Hex, hex, equalBytes,
    frame, deframe,
    CHUNK_HDR, fountainChunks, Reassembler, fountainReassemble,
    FSK, FSK_FAST, fskEncode, fskDecode, addNoise,
    qrFor, byteCapacity, drawQR, scanCanvas,
  };
})(window);
