// Browser entry for the single-file build. The build injects the vector and wordlist
// text as globals (__VECTORS__, __EFF__) and then calls start(); nothing is fetched at runtime.
import { replayVectors } from './replay.mjs';

const failures = (recs) => recs.filter((r) => !r.ok).map((r) => `${r.id}: ${r.error}`);

async function run() {
  const t0 = performance.now();
  const crypto = await replayVectors(globalThis.__VECTORS__, globalThis.__EFF__);
  const result = {
    crypto: { total: crypto.length, expected: JSON.parse(globalThis.__VECTORS__).vectors.length, failures: failures(crypto) },
    ms: Math.round(performance.now() - t0),
  };
  result.pass = result.crypto.total === result.crypto.expected && result.crypto.failures.length === 0;
  globalThis.__result = result;
  globalThis.__done = true;
}

export function start() {
  run().catch((e) => {
    globalThis.__result = { pass: false, error: String(e && e.stack ? e.stack : e) };
    globalThis.__done = true;
  });
}
