// PoC A - node automation path for the A2 direction: produces the exact same
// blobs as the browser page and writes poc/crypto-interop/blobs.json.
// Run: node emit-js-blobs.node.mjs

import { produceBlobs } from './test/blobs.mjs';
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';

const ROOT = fileURLToPath(new URL('.', import.meta.url));
const t0 = performance.now();
const data = await produceBlobs();
const ms = performance.now() - t0;
fs.writeFileSync(`${ROOT}blobs.json`, JSON.stringify(data, null, 2) + '\n');
console.log(`blobs.json written in ${ms.toFixed(0)} ms`);
for (const [kind, b] of Object.entries(data.blobs)) {
  const out = Object.values(b.output)[0];
  console.log(`  ${kind.padEnd(16)} ${out.length / 2} bytes  ${out.slice(0, 24)}…`);
}
