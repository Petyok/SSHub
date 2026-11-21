// Output channels of a response and the reverse-QR renderer. The FSK modem is the PoC H design
// (poc/share-lib.js): 16 tones from 1600 Hz in 100 Hz steps, 1000 Hz sync preamble, frame =
// LEN(1) || payload || CRC16-CCITT-FALSE(2). poc/share-receive.html decodes it, so the terminal
// side and the benches stay interchangeable. Two symbol lengths; fast is the default.
import qrcode from '../vendor/qrcode-generator-1.4.4/qrcode.cjs';
import { fountainEncode } from './quorum.mjs';

// Reply QR frame size: the byte capacity of QR version 6, level M (106 B), so a 68 mm phone screen
// gives modules of 1.4 mm, the size a fixed-focus laptop webcam read in the over-the-air test (PoC H
// "Reverse QR"). The fountain format is unchanged (the decoder takes the block size from the frame
// length); only the number of frames grows. The terminal's own challenge frames keep limits.rs.
export const QR_FRAME_BYTES = 106;
export const QR_FPS = 4;
export const QR_PARITY_PERCENT = 25;

export function crc16(bytes) {
  let crc = 0xffff;
  for (let i = 0; i < bytes.length; i++) {
    crc ^= bytes[i] << 8;
    for (let b = 0; b < 8; b++) crc = crc & 0x8000 ? ((crc << 1) ^ 0x1021) & 0xffff : (crc << 1) & 0xffff;
  }
  return crc;
}

export function audioFrame(payload) {
  if (payload.length > 255) throw new Error('audio frame: payload over 255 bytes');
  const out = new Uint8Array(payload.length + 3);
  out[0] = payload.length;
  out.set(payload, 1);
  const c = crc16(out.subarray(0, payload.length + 1));
  out[payload.length + 1] = c >> 8;
  out[payload.length + 2] = c & 0xff;
  return out;
}

export const FSK = { sampleRate: 48000, base: 1600, step: 100, nTones: 16, symMs: 60, rampMs: 5, preambleMs: 300, syncHz: 1000, amp: 0.5 };
export const FSK_FAST = { ...FSK, symMs: 30, rampMs: 3 };
export const AUDIO_PROFILES = { fast: FSK_FAST, slow: FSK };

function toneInto(out, off, freq, N, R) {
  const w = (2 * Math.PI * freq) / FSK.sampleRate;
  for (let n = 0; n < N; n++) {
    const env = n < R ? 0.5 * (1 - Math.cos((Math.PI * n) / R)) : n >= N - R ? 0.5 * (1 - Math.cos((Math.PI * (N - 1 - n)) / R)) : 1;
    out[off + n] = FSK.amp * env * Math.sin(w * n);
  }
}

export function fskEncode(bytes, F = FSK_FAST) {
  const sr = FSK.sampleRate;
  const N = Math.round((sr * F.symMs) / 1000);
  const R = Math.round((sr * F.rampMs) / 1000);
  const P = Math.round((sr * FSK.preambleMs) / 1000);
  const nibbles = [];
  for (const b of bytes) nibbles.push(b >> 4, b & 0x0f);
  const out = new Float32Array(P + nibbles.length * N);
  toneInto(out, 0, FSK.syncHz, P, R);
  nibbles.forEach((s, i) => toneInto(out, P + i * N, FSK.base + FSK.step * s, N, R));
  return out;
}

// The burst the terminal hears for a response frame: sync, then the framed bytes.
export const responseBurst = (responseFrame, profile = 'fast') => fskEncode(audioFrame(responseFrame), AUDIO_PROFILES[profile]);

let playing = null;
export function playSamples(samples) {
  const Ctx = globalThis.AudioContext || globalThis.webkitAudioContext;
  if (!Ctx) throw new Error('this browser has no Web Audio');
  if (!playing) playing = new Ctx();
  const ctx = playing;
  const buf = ctx.createBuffer(1, samples.length, FSK.sampleRate);
  buf.copyToChannel(samples, 0);
  const src = ctx.createBufferSource();
  src.buffer = buf;
  src.connect(ctx.destination);
  if (ctx.state === 'suspended') ctx.resume();
  return new Promise((res) => {
    src.onended = res;
    src.start();
  });
}

// ------------------------------------------------------------ QR

export const latin1 = (bytes) => {
  let s = '';
  for (let i = 0; i < bytes.length; i += 4096) s += String.fromCharCode.apply(null, bytes.subarray(i, i + 4096));
  return s;
};

export function qrFor(bytes) {
  const qr = qrcode(0, 'M');
  qr.addData(latin1(bytes), 'Byte');
  qr.make();
  return qr;
}

// Black modules on a white field with a 4 module quiet zone, whatever the page theme is. The code is
// as wide as the screen: an integer number of DEVICE pixels per module (scale = floor(width * dpr /
// (modules + 8))), CSS size = canvas / dpr so the browser maps one canvas pixel to one device pixel
// (image-rendering: pixelated). The width is the smaller screen side so a rotated phone still shows
// the whole code; position:relative lets it leave the padded page column.
export function drawQr(canvas, qr, dpr = globalThis.devicePixelRatio || 1) {
  const n = qr.getModuleCount();
  const quiet = 4;
  const widthCss = Math.min(globalThis.innerWidth, globalThis.innerHeight);
  const scale = Math.max(1, Math.floor((widthCss * dpr) / (n + 2 * quiet)));
  const size = (n + 2 * quiet) * scale;
  canvas.width = size;
  canvas.height = size;
  canvas.style.width = `${size / dpr}px`;
  canvas.style.height = `${size / dpr}px`;
  canvas.style.left = `calc(50% - ${size / dpr / 2}px)`;
  const ctx = canvas.getContext('2d');
  ctx.fillStyle = '#fff';
  ctx.fillRect(0, 0, size, size);
  ctx.fillStyle = '#000';
  for (let r = 0; r < n; r++) for (let c = 0; c < n; c++) if (qr.isDark(r, c)) ctx.fillRect((c + quiet) * scale, (r + quiet) * scale, scale, scale);
}

// Frames of one document (fountain format, same as the terminal's challenge) as QR matrices.
export const reverseQrFrames = (payload) => fountainEncode(payload, QR_FRAME_BYTES, QR_PARITY_PERCENT).map(qrFor);

// Loops the frames at QR_FPS onto a canvas. Returns { stop(), frameIndex() }.
export function loopQr(canvas, qrs, fps = QR_FPS) {
  let i = 0;
  const show = () => drawQr(canvas, qrs[i % qrs.length]);
  show();
  const timer = qrs.length > 1 ? setInterval(() => { i++; show(); canvas.dataset.frame = String(i % qrs.length); }, 1000 / fps) : null;
  return { stop: () => timer && clearInterval(timer), frameIndex: () => i % qrs.length };
}

export function toBase64(bytes) {
  return btoa(latin1(bytes));
}
