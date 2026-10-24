// PoC A: browser entry. Runs inside the single-file page (no network).
//  1. produceBlobs(): the A2 direction (JS produces, Rust verifies).
//  2. replayVectors(): the A1 direction (Rust vectors, JS reproduces), with the
//     vectors and the EFF word list embedded at build time.
// Results are exposed on window.__result for the headless test and rendered
// into the page. The 1 GiB Argon2 profile only runs when the URL has ?master=1.
import { produceBlobs } from './test/blobs.mjs';
import { replayVectors } from './test/replay.mjs';
import vectorsText from './vectors.json';
import effText from './wordlists/eff_large.txt';

const t0 = performance.now();
const status = document.getElementById('status');
const out = document.getElementById('out');
const master = new URLSearchParams(location.search).has('master');

async function main() {
  const tb = performance.now();
  const blobs = await produceBlobs();
  const blobMs = performance.now() - tb;
  const tr = performance.now();
  const replay = await replayVectors(vectorsText, effText, { skipProfiles: master ? [] : ['MASTER'] });
  const replayMs = performance.now() - tr;
  const failed = replay.filter((r) => !r.ok);
  window.__result = {
    ok: failed.length === 0,
    secureContext: window.isSecureContext,
    blobMs,
    replayMs,
    totalMs: performance.now() - t0,
    master,
    vectors: replay.length,
    skipped: replay.filter((r) => r.skipped).length,
    failed: failed.map((r) => `${r.id}: ${r.error}`),
    replay,
    blobs,
  };
  status.textContent = `${replay.length - failed.length}/${replay.length} vectors reproduced, blobs in ${blobMs.toFixed(0)} ms, replay in ${replayMs.toFixed(0)} ms (offline, no network used)`;
  if (failed.length) status.className = 'err';
  for (const [kind, b] of Object.entries(blobs.blobs)) {
    for (const [name, hexv] of Object.entries(b.output)) {
      const title = document.createElement('div');
      title.className = 'title';
      title.textContent = `${kind} / ${name} / ${hexv.length / 2} bytes`;
      const pre = document.createElement('pre');
      pre.textContent = hexv;
      out.append(title, pre);
    }
  }
  window.__done = true;
}

main().catch((e) => {
  status.textContent = `ERROR: ${e.message}`;
  status.className = 'err';
  window.__result = { ok: false, error: String(e.stack || e) };
  window.__done = true;
});
