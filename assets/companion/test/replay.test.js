// Replays the shared Rust-produced vectors. Any vector that fails is a test failure;
// nothing is skipped. Run: node --test assets/companion/test/
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { replayVectors, replayOps } from '../src/replay.mjs';

const root = new URL('../../../', import.meta.url);
const read = (p) => readFileSync(new URL(p, root), 'utf8');
const vectorsText = read('tests/vectors/crypto/vectors.json');
const opsText = read('tests/vectors/op-rules/ops.json');
const effText = read('assets/wordlists/eff_large.txt');

const failures = (recs) => recs.filter((r) => !r.ok).map((r) => `${r.id}: ${r.error}`);

test('crypto vectors (vectors.json): 0 mismatches, none skipped', async () => {
  const recs = await replayVectors(vectorsText, effText);
  assert.equal(recs.length, 45, 'expected 45 vectors');
  assert.deepEqual(failures(recs), []);
});

test('op-rules vectors (ops.json): 0 mismatches, none skipped', () => {
  const recs = replayOps(opsText);
  assert.equal(recs.length, 13, 'expected 13 ops');
  assert.deepEqual(failures(recs), []);
});
