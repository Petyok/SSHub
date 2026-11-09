// PhoneHandover on the page (design 5, contract 4j): the OLD phone signs the op that names its successor, and the NEW phone
// signs its own admission (RedPhoneSign). The eight compare words come from lib.mjs handoverWords (the same function as
// Rust crypto/sas.rs handover_words). The ring decides whether the handover counts; the page only authors its part.
import { bytesOf, deviceIdOf, edPointValid, handoverWords, mlkemEncapsulate } from './lib.mjs';
import { describeCert, issuedCerts } from './cert.mjs';
import { safeText } from './ops.mjs';
import { appendPhoneHandover, appendRedPhoneSign } from './state.mjs';

const MAX_NAME_BYTES = 64;
const oldId8 = (state) => {
  if (!state.enrolled) throw new Error('This phone is not part of a ring yet: there is nothing to hand over.');
  return bytesOf(state.enrolled.device_id);
};

// The name rules of the enrollment (not empty, at most 64 bytes, no control characters) and no padding.
function checkName(name) {
  const t = name.trim();
  if (!t) throw new Error('Give this phone a name.');
  if (new TextEncoder().encode(t).length > MAX_NAME_BYTES) throw new Error('The name is longer than 64 bytes.');
  if (/\p{Cc}/u.test(t)) throw new Error('The name has control characters.');
  if (t !== name) throw new Error("The new phone's name has spaces around it: ask the terminal to show it again.");
}

function checkRequest(state, req) {
  checkName(req.name);
  const p = req.new_phone;
  try {
    if (!edPointValid(p.ed25519_pub)) throw new Error('invalid Ed25519 public key');
    if (p.mlkem_pub.length !== 1184) throw new Error('ML-KEM-768 public key must be 1184 bytes');
    mlkemEncapsulate(p.mlkem_pub, new Uint8Array(32));
  } catch (e) {
    throw new Error(`The new phone's keys are not valid: ${e.message}`);
  }
  if (state.enrolled && state.enrolled.device_id === deviceIdOf(p.ed25519_pub)) throw new Error('The request names this phone as its own successor.');
}

// HandoverView: what the old phone shows. { name, words, certs_carried }.
export function describePhoneHandover(state, req) {
  const old = oldId8(state);
  checkRequest(state, req);
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
  oldId8(state);
  checkRequest(state, req);
  if (hasHandover(state)) throw new Error('This phone already signed a handover. Only one counts; ask the terminal to finish it or cancel it.');
  return appendPhoneHandover(state, identity, req.new_phone, req.name, now, null);
}

// The NEW phone's own RedPhoneSign over the handover op.
export const signHandoverAdmission = (state, identity, handoverOpHashHex, now) => appendRedPhoneSign(state, identity, handoverOpHashHex, now, null);
