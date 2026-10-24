// Browser entry for the single-file build. The build injects the vector and wordlist
// text as globals (__VECTORS__, __OPS__, __EFF__); nothing is fetched at runtime.
import { replayVectors, replayOps } from './replay.mjs';

const failures = (recs) => recs.filter((r) => !r.ok).map((r) => `${r.id}: ${r.error}`);

async function run() {
  const t0 = performance.now();
  const crypto = await replayVectors(globalThis.__VECTORS__, globalThis.__EFF__);
  const ops = replayOps(globalThis.__OPS__);
  const result = {
    crypto: { total: crypto.length, expected: JSON.parse(globalThis.__VECTORS__).vectors.length, failures: failures(crypto) },
    ops: { total: ops.length, expected: JSON.parse(globalThis.__OPS__).ops.length, failures: failures(ops) },
    ms: Math.round(performance.now() - t0),
  };
  result.pass =
    result.crypto.total === result.crypto.expected &&
    result.ops.total === result.ops.expected &&
    result.crypto.failures.length === 0 &&
    result.ops.failures.length === 0;
  globalThis.__result = result;
  globalThis.__done = true;
}

run().catch((e) => {
  globalThis.__result = { pass: false, error: String(e && e.stack ? e.stack : e) };
  globalThis.__done = true;
});
