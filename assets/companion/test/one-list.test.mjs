// Phase B, U13-5 revised: the one-list rule (design 7). Mirrors crates/sshub-phone-core/src/chain.rs tests.
// Run in the tools container: cd /work/assets/companion && NODE_PATH=/opt/node/lib/node_modules node --test test/one-list.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { identityOfSecret } from '../src/keys.mjs';
import { decodePhoneOps, encodePhoneOps } from '../src/frames.mjs';
import { SAME_TERMINAL_MESSAGE, SINGLE_SOURCE_TICK_MESSAGE, acceptHead, adoptOwnChain, appendHeadAttest, courierOps, emptyState, restoredState, singleSourceNote } from '../src/state.mjs';

const NOW = 1_900_000_000;
const t = (count) => ({ count, newestOpTs: BigInt(NOW) });
const sid = (n) => new Uint8Array(8).fill(n);

function setup() {
  const id = identityOfSecret(new Uint8Array(32).fill(21));
  const live = emptyState();
  for (let i = 0; i < 3; i++) appendHeadAttest(live, id, NOW + i, null);
  return { id, ops: courierOps(live, 8), shown: { seq: '3', hash: live.chain[2].hash } };
}

test('n=2 with one terminal unreachable accepts one list with the tick', () => {
  const { id, ops, shown } = setup();
  const s = restoredState();
  const r = adoptOwnChain(s, id, ops, sid(1), t(2));
  assert.equal(r.agreed, true);
  assert.equal(r.single_source, true);
  assert.equal(r.terminals.count, 2);
  assert.throws(() => acceptHead(s, shown), (e) => e.message === SINGLE_SOURCE_TICK_MESSAGE);
  assert.throws(() => acceptHead(s, shown, false), (e) => e.message === SINGLE_SOURCE_TICK_MESSAGE);
  assert.ok(s.headProposal, 'a refusal keeps the proposal');
  assert.equal(acceptHead(s, shown, true).head_seq, '3');
  assert.equal(s.headPending, false);
});

test('a Hub plus one laptop: two agreeing lists need no tick', () => {
  const { id, ops, shown } = setup();
  const s = restoredState();
  assert.equal(adoptOwnChain(s, id, ops, sid(1), t(2)).single_source, true);
  const second = adoptOwnChain(s, id, ops, sid(2), t(2));
  assert.equal(second.agreed, true);
  assert.equal(second.single_source, false);
  assert.equal(acceptHead(s, shown).head_seq, '3');
});

test('no terminals block, or a count of zero, keeps the two-list rule', () => {
  const { id, ops } = setup();
  const s = restoredState();
  const r = adoptOwnChain(s, id, ops, sid(1));
  assert.equal(r.agreed, false);
  assert.equal(s.headProposal, undefined);
  const z = adoptOwnChain(s, id, ops, sid(3), t(0));
  assert.equal(z.agreed, true);
  assert.equal(z.single_source, false);
});

test('the same terminal twice never upgrades a one-list proposal', () => {
  const { id, ops, shown } = setup();
  const s = restoredState();
  adoptOwnChain(s, id, ops, sid(1), t(1));
  assert.throws(() => adoptOwnChain(s, id, ops, sid(1), t(1)), (e) => e.message === SAME_TERMINAL_MESSAGE);
  assert.equal(s.headProposal.singleSource, true);
  assert.throws(() => acceptHead(s, shown, false));
});

test('the card says the count and never "trusted"', () => {
  assert.match(singleSourceNote(3, t(1)), /^Your ring has one terminal\./);
  assert.match(singleSourceNote(3, t(4)), /^Your ring has 4 terminals and I can reach only this one\./);
  assert.doesNotMatch(singleSourceNote(3, t(1)), /trusted/);
});

test('kind 5 terminals block: round trip with both blocks, block order, truncation, unknown tag, old frames', () => {
  const { ops } = setup();
  const both = decodePhoneOps(encodePhoneOps(ops, sid(7), t(5)));
  assert.equal(both.terminals.count, 5);
  assert.equal(both.terminals.newestOpTs, BigInt(NOW));
  assert.deepEqual([...both.sender], [...sid(7)]);
  const plain = decodePhoneOps(encodePhoneOps(ops));
  assert.equal(plain.terminals, null);
  assert.equal(plain.sender, null);
  assert.equal(decodePhoneOps(encodePhoneOps(ops, sid(7))).terminals, null);
  const frame = encodePhoneOps(ops, sid(7), t(5));
  for (const cut of [1, 5, 9]) assert.throws(() => decodePhoneOps(frame.slice(0, frame.length - cut)));
  assert.throws(() => decodePhoneOps(Uint8Array.of(...frame, 0)), /trailing/);
  const swapped = Uint8Array.from(frame);
  swapped[frame.length - 10] = 9; // an unknown tag where the terminals block starts
  assert.throws(() => decodePhoneOps(swapped));
});
