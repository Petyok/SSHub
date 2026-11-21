// Scratch: PCM from the phone's own encoders. Usage: node media-gen.mjs <response-short.bin> <outdir>
import fs from 'node:fs';
import vm from 'node:vm';
import { fskEncode, audioFrame, FSK, FSK_FAST } from './src/channels.mjs';
const c = { console, crypto: globalThis.crypto };
c.window = c;
vm.createContext(c);
vm.runInContext(fs.readFileSync('poc/share-lib.js', 'utf8'), c);
const L = c.ShareLib;
const short = fs.readFileSync(process.argv[2]);
const out = process.argv[3];
const wav = (samples) => {
  const b = Buffer.alloc(44 + 2 * samples.length);
  b.write('RIFF', 0); b.writeUInt32LE(b.length - 8, 4); b.write('WAVEfmt ', 8); b.writeUInt32LE(16, 16); b.writeUInt16LE(1, 20); b.writeUInt16LE(1, 22);
  b.writeUInt32LE(48000, 24); b.writeUInt32LE(96000, 28); b.writeUInt16LE(2, 32); b.writeUInt16LE(16, 34); b.write('data', 36); b.writeUInt32LE(b.length - 44, 40);
  samples.forEach((v, i) => b.writeInt16LE(Math.max(-32767, Math.min(32767, Math.round(v * 32767))), 44 + 2 * i));
  return b;
};
const eq = (a, b) => a.length === b.length && a.every((v, i) => v === b[i]);
for (const [name, lib, chan] of [['slow', L.FSK, FSK], ['fast', L.FSK_FAST, FSK_FAST]]) {
  console.log(`share-lib ${name} == channels ${name}:`, eq(L.fskEncode(L.frame(short), lib), fskEncode(audioFrame(short), chan)));
}
const slow = fskEncode(audioFrame(short.subarray(0, 48)), FSK);
fs.writeFileSync(`${out}/slow-48.wav`, wav(slow));
const page = fs.readFileSync(`${out}/fake-mic-fast.wav`);
const all = new Int16Array(page.buffer.slice(page.byteOffset + 44, page.byteOffset + page.length));
fs.writeFileSync(`${out}/fast-128.wav`, wav(Array.from(all.subarray(48000, all.length - 96000), (v) => v / 32767)));
