// Builds a reshare of one credential to the holders [A, B, phone, D] with the vector-verified
// primitives of lib.mjs; shared by the node tests and the headless end to end run.
import { xchacha20poly1305 } from '../vendor/noble-ciphers-2.4.0/chacha.js';
import {
  bytesOf, canonicalJSON, chachaRngStream, concat, deriveKey, deviceIdOf, edSign, hexOf, hybridWrap, identityFromIkm, reshareContribute, splitAtIndicesSeeded, utf8,
} from '../src/lib.mjs';
import { b64enc } from '../src/ops.mjs';
import { randomBytes } from '../src/keys.mjs';

const h = (x) => bytesOf(x);
export const EPOCH = 1;
export const HOST = '0a'.repeat(16);
export const hostBytes = h(HOST);
const commit = (from, index, sub) => deriveKey('sshub/sync/reshare/commit/v1', concat(Uint8Array.of(from, index), sub));
const u64 = (n) => {
  const b = new Uint8Array(8);
  new DataView(b.buffer).setBigUint64(0, BigInt(n));
  return b;
};
const seal = (key, aad, plain) => {
  const nonce = randomBytes(24);
  return concat(nonce, xchacha20poly1305(key, nonce, aad).encrypt(plain));
};

// Two contributors (devices A and B, old shares 1 and 2 of a 3 way sharing) reshare one credential
// to the new holders [A, B, phone, D]. Returns signed ops and what the test needs to check them.
// oneAuthor: contributor B's batch is written and signed by device A (a single device posing as two contributors,
// with a different `from` byte, re-using A's session key).
export function scenario({ phone, lie = false, oneAuthor = false, secret = randomBytes(32) }) {
  const old = splitAtIndicesSeeded(secret, 2, [1, 2, 3], chachaRngStream(randomBytes(32), 32));
  const dev = (n) => identityFromIkm(new Uint8Array(32).fill(n), `sshub/test/contrib/${n}`);
  const A = dev(1);
  const B = dev(2);
  const ids = { A: deviceIdOf(A.edPub), B: deviceIdOf(B.edPub), D: 'dddddddddddddddd' };
  const holders = [ids.A, ids.B, phone.deviceId, ids.D].sort();
  const newIndices = holders.map((_, i) => i + 1);
  let lamport = 10;
  const signed = [];
  const emit = (author, kind, fields) => {
    const body = { [kind]: fields };
    const text = canonicalJSON({ device_id: deviceIdOf(author.edPub), seq: lamport, lamport, prev_hash: '00'.repeat(32), timestamp: 1893452400, epoch: 0, alg: [1, 1], body });
    lamport += 1;
    const canonical = utf8(text);
    signed.push({ canonical, signature: edSign(author.edSeed, canonical) });
  };
  let sessionKeyA = null;
  for (const [name, who0, oldShare] of [['A', A, old[0]], ['B', B, old[1]]]) {
    const posing = oneAuthor && name === 'B';
    const who = posing ? A : who0;
    const author = posing ? ids.A : ids[name];
    const c = reshareContribute(oldShare, newIndices, 2, chachaRngStream(randomBytes(32), 32));
    const sessionKey = posing ? sessionKeyA : randomBytes(32);
    if (name === 'A') sessionKeyA = sessionKey;
    if (!posing) emit(who, 'ShareReshareSession', { epoch: EPOCH, to: phone.deviceId, sealed: b64enc(hybridWrap({ x25519_pub: phone.xPub, mlkem_pub: phone.mlkemPub }, sessionKey, randomBytes(32), randomBytes(24), randomBytes(32))) });
    const sub_shares = holders.map((to, j) => {
      const [index, sub] = c.sub_shares[j];
      const plain = concat(u64(0), Uint8Array.of(c.from), sub);
      const key = to === phone.deviceId ? sessionKey : randomBytes(32);
      return [to, b64enc(seal(key, concat(hostBytes, u64(EPOCH), utf8(author), utf8(to)), plain))];
    });
    const commitments = holders.map((_, j) => {
      const [index, sub] = c.sub_shares[j];
      return b64enc(commit(c.from, index, lie && name === 'B' && holders[j] === phone.deviceId ? new Uint8Array(32) : sub));
    });
    emit(who, 'ShareReshareBatch', { epoch: EPOCH, new_holders: holders, items: [{ host_uid: HOST, sub_shares, commitments }] });
    ids[`c${name}`] = c;
  }
  const ring = { [ids.A]: { name: 'terminal A', ed25519_pub: hexOf(A.edPub) }, [ids.B]: { name: 'terminal B', ed25519_pub: hexOf(B.edPub) } };
  return { secret, signed, holders, ids, ring, newIndices };
}

