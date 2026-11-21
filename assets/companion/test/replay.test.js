// Replays the shared Rust-produced vectors. Any vector that fails is a test failure;
// nothing is skipped. Run: node --test assets/companion/test/
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { replayVectors } from '../src/replay.mjs';

const root = new URL('../../../', import.meta.url);
const read = (p) => readFileSync(new URL(p, root), 'utf8');
const vectorsText = read('tests/vectors/crypto/vectors.json');
const effText = read('crates/sshub-sync-proto/assets/wordlists/eff_large.txt');

const failures = (recs) => recs.filter((r) => !r.ok).map((r) => `${r.id}: ${r.error}`);

test('crypto vectors (vectors.json) reproduce exactly', async () => {
  const recs = await replayVectors(vectorsText, effText);
  assert.equal(recs.length, JSON.parse(vectorsText).vectors.length, 'every vector in the file is replayed');
  assert.deepEqual(failures(recs), []);
});
