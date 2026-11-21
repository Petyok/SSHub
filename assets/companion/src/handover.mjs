// PhoneHandover on the page (design 5, contract 4j): the OLD phone signs the op that names its successor, and the NEW phone
// signs its own admission (RedPhoneSign). The eight compare words come from lib.mjs handoverWords (the same function as
// Rust crypto/sas.rs handover_words). The ring decides whether the handover counts; the page only authors its part.
import { bytesOf, deviceIdOf, edPointValid, handoverWords, mlkemEncapsulate } from './lib.mjs';
import { describeCert, issuedCerts } from './cert.mjs';
import { safeText } from './ops.mjs';
import { appendPhoneHandover, appendRedPhoneSign } from './state.mjs';

const MAX_NAME_BYTES = 64;
// This phone's OWN id (the OLD phone the words bind). state.enrolled only says "in a ring": it names the terminal.
const oldId8 = (state, identity) => {
  if (!state.enrolled) throw new Error('This phone is not part of a ring yet: there is nothing to hand over.');
  return bytesOf(identity.deviceId);
};

// The name rules of the enrollment (not empty, at most 64 bytes, no control characters) and no padding.
function checkName(name) {
  const t = name.trim();
  if (!t) throw new Error('Give this phone a name.');
  if (new TextEncoder().encode(t).length > MAX_NAME_BYTES) throw new Error('The name is longer than 64 bytes.');
  if (/\p{Cc}/u.test(t)) throw new Error('The name has control characters.');
  if (t !== name) throw new Error("The new phone's name has spaces around it: ask the terminal to show it again.");
}

function checkRequest(state, identity, req) {
  checkName(req.name);
  const p = req.new_phone;
  try {
    if (!edPointValid(p.ed25519_pub)) throw new Error('invalid Ed25519 public key');
    if (p.mlkem_pub.length !== 1184) throw new Error('ML-KEM-768 public key must be 1184 bytes');
    mlkemEncapsulate(p.mlkem_pub, new Uint8Array(32));
  } catch (e) {
    throw new Error(`The new phone's keys are not valid: ${e.message}`);
  }
  if (state.enrolled && identity.deviceId === deviceIdOf(p.ed25519_pub)) throw new Error('The request names this phone as its own successor.');
}

// HandoverView: what the old phone shows. { name, words, certs_carried }.
export function describePhoneHandover(state, identity, req) {
  if (req.op) throw new Error('This is the admission frame for the new phone, not a request to the old one.');
  const old = oldId8(state, identity);
  checkRequest(state, identity, req);
  const aliases = { ...Object.fromEntries(Object.entries(state.ring).map(([id, d]) => [id, d.name])), ...state.aliases };
  return {
    name: safeText(req.name),
    words: handoverWords(req.new_phone.ed25519_pub, req.new_phone.x25519_pub, req.new_phone.mlkem_pub, req.name, old),
    certs_carried: issuedCerts(state).filter((c) => !c.revoked).map((c) => `${safeText(describeCert(c, aliases))} [${c.id}]`),
  };
}

const hasHandover = (state) => state.chain.some((e) => /"body":\{"PhoneHandover"/.test(new TextDecoder().decode(bytesOf(e.canonical))));

// Signs the PhoneHandover op naming the new phone (the author proof). One handover per phone.
export function buildPhoneHandover(state, identity, req, now) {
  if (req.op) throw new Error('This is the admission frame for the new phone: only the new phone signs it.');
  oldId8(state, identity);
  checkRequest(state, identity, req);
  if (hasHandover(state)) throw new Error('This phone already signed a handover. Only one counts; ask the terminal to finish it or cancel it.');
  return appendPhoneHandover(state, identity, req.new_phone, req.name, now, null);
}

// The admission frame the NEW phone scans (contract 4l b): the request plus the hash and lamport of the ingested PhoneHandover op.
// Refused unless it names THIS phone's own keys, so a frame meant for another phone never gets this phone's signature.
// The lamport is unsigned: a forged huge one would push this phone's counters out of range, so anything a ring could not have reached (2^32 ops) is refused.
export const MAX_ADMISSION_OP_LAMPORT = 0xffffffffn;
function checkAdmission(identity, req) {
  if (!req.op) throw new Error('This request carries no handover operation: it is for the old phone.');
  const p = req.new_phone;
  const same = (a, b) => a.length === b.length && a.every((x, i) => x === b[i]);
  if (!same(p.ed25519_pub, identity.edPub) || !same(p.x25519_pub, identity.xPub) || !same(p.mlkem_pub, identity.mlkemPub)) throw new Error('This handover names another phone, not this one. Do not sign it.');
  checkName(req.name);
  if (req.op.lamport > MAX_ADMISSION_OP_LAMPORT) throw new Error('The handover operation names an impossible counter: ask the terminal to show it again.');
  return req.op;
}

// HandoverView for the new phone: the name and the operation it will sign over; no words, no certificates.
export function describeHandoverAdmission(identity, req) {
  const op = checkAdmission(identity, req);
  return { name: safeText(req.name), words: [], certs_carried: [], op_hash: op.hash, op_lamport: Number(op.lamport) };
}

// The NEW phone's own RedPhoneSign over the handover op. Its lamport is max(own next, op_lamport + 1): a proof written
// before the op it names would not count on the host.
export function signHandoverAdmission(state, identity, req, now) {
  const op = checkAdmission(identity, req);
  return appendRedPhoneSign(state, identity, op.hash, now, { lamport: op.lamport, epoch: 0n });
}
