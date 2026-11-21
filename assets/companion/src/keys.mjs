// The phone's identity: 32 random bytes (the secret), from which the Ed25519, X25519 and ML-KEM
// keys are derived with the same BLAKE3 derive_key construction as every other identity
// (lib.identityFromIkm). Context `sshub/sync/phone/v1`; the 24 BIP39 words ARE the secret, so
// writing them down is the backup and typing them into any phone restores the same device.
// At rest the secret is wrapped under the PIN: Argon2id -> XChaCha20-Poly1305, the profile in
// the header (and under AAD) like the key file of contract section 1:
//   "SSHP" || 1 || mem u32 BE || iters u32 BE || salt(16) || nonce(24) || ct(32 + 16)
import { xchacha20poly1305 } from '../vendor/noble-ciphers-2.4.0/chacha.js';
import { argon2idDerive, bip39Decode, bip39Encode, concat, deviceIdOf, deriveXof, hybridUnwrap, identityFromIkm, mlkemKeygen, x25519Pub } from './lib.mjs';

export const PHONE_CONTEXT = 'sshub/sync/phone/v1';
// 64 MiB x 3: about a second on a phone. A short PIN is the weak link whatever the profile is;
// the profile only prices an offline guess against a stolen browser profile.
export const PIN_PROFILE = { mem_kib: 65536, iters: 3 };
export const PIN_MIN_CHARS = 10;
// Refused as a PIN: fewer than 5 different characters (1111111112, 1212121212), a short block repeated (1234512345), a run
// of 7 or more consecutive characters up or down (1234567890, 1234567891, abcdefghij; digits wrap 9 to 0) and a few
// worst-known substrings (password, qwerty). Not a big list on purpose: the length floor and the Argon2id profile are the
// real price, and the setup screen says to use a passphrase.
const WEAK_PARTS = ['password', 'passw0rd', 'qwerty', 'asdfgh', 'zxcvbn', 'letmein', 'iloveyou', 'admin', 'welcome', 'abc123', '123abc'];
export function pinProblem(pin) {
  const cps = Array.from(pin, (c) => c.codePointAt(0));
  if (cps.length < PIN_MIN_CHARS) return `The PIN needs at least ${PIN_MIN_CHARS} characters.`;
  const weak = 'That PIN is too easy to guess. Use a passphrase of several unrelated words instead.';
  if (new Set(cps).size < 5) return weak;
  for (let p = 1; p <= 5; p++) if (cps.every((c, i) => i < p || c === cps[i - p])) return weak;
  const digits = cps.every((c) => c >= 48 && c <= 57);
  const step = (a, b) => (digits ? (b - a + 10) % 10 : b - a);
  const up = digits ? 1 : 1;
  const down = digits ? 9 : -1;
  let run = 1;
  let dir = 0;
  for (let i = 1; i < cps.length; i++) {
    const s = step(cps[i - 1], cps[i]);
    if (s === up || s === down) {
      run = s === dir ? run + 1 : 2;
      dir = s;
    } else {
      run = 1;
      dir = 0;
    }
    if (run >= 7) return weak;
  }
  const lower = pin.toLowerCase();
  if (WEAK_PARTS.some((w) => lower.includes(w))) return weak;
  return null;
}
// Seconds to wait after the n-th wrong PIN in a row: none for two, then 5 s doubling up to an hour.
export const pinDelaySecs = (n) => (n < 3 ? 0 : Math.min(3600, 5 * 2 ** (n - 3)));
const MAGIC = new Uint8Array([0x53, 0x53, 0x48, 0x50]); // "SSHP"
const HEADER = 4 + 1 + 4 + 4 + 16 + 24;
const MAX_MEM_KIB = 512 * 1024;
const MAX_ITERS = 16;
const u32 = (n) => Uint8Array.of((n >>> 24) & 0xff, (n >>> 16) & 0xff, (n >>> 8) & 0xff, n & 0xff);
const rd32 = (b, at) => ((b[at] << 24) | (b[at + 1] << 16) | (b[at + 2] << 8) | b[at + 3]) >>> 0;

export const randomBytes = (n) => globalThis.crypto.getRandomValues(new Uint8Array(n));

// Wrap-key generations (C14). Generation 0 is the identity's own X25519 and ML-KEM pair; generation
// e > 0 (published by a WrapKeyRotate op) is derived from the secret and e, so the 24 words still
// restore every generation. Unlike a key file the phone therefore cannot truly retire an old
// generation: a residual the design lists for phones.
const WRAP_GEN_CONTEXT = 'sshub/sync/phone/wrap/v1';
function wrapGeneration(secret32, epoch) {
  if (epoch === 0) {
    const id = identityFromIkm(secret32, PHONE_CONTEXT);
    return { xSecret: id.xSecret, xPub: id.xPub, mlkemPub: id.mlkemPub, mlkemDk: id.mlkemDk };
  }
  const e = new Uint8Array(8);
  new DataView(e.buffer).setBigUint64(0, BigInt(epoch));
  const m = deriveXof(WRAP_GEN_CONTEXT, concat(secret32, e), 96);
  const xSecret = m.slice(0, 32);
  const kem = mlkemKeygen(m.slice(32, 96));
  return { xSecret, xPub: x25519Pub(xSecret), mlkemPub: kem.publicKey, mlkemDk: kem.secretKey };
}

// The identity at wrap epoch `wrapEpoch`: signing key fixed, wrap keys of the newest generation,
// `unwrap(blob)` tries every generation from the newest down.
// A restored phone does not know its wrap generation unless the adopted chain says so, so unwrap also tries the
// generations above the one it was built for, up to this bound (derived once, then kept).
export const WRAP_GEN_MAX = 32;
export function identityOfSecret(secret32, wrapEpoch = 0) {
  const base = identityFromIkm(secret32, PHONE_CONTEXT);
  const gens = [];
  for (let e = 0; e <= wrapEpoch; e++) gens.push(wrapGeneration(secret32, e));
  const cur = gens[gens.length - 1];
  const tryGen = (i, blob) => hybridUnwrap(gens[i].xSecret, gens[i].mlkemDk, blob);
  const ident = {
    ...base,
    ...cur,
    secret: secret32,
    deviceId: deviceIdOf(base.edPub),
    wrapEpoch,
    // Highest generation an unwrap actually needed above `wrapEpoch`: a generation the ring uses, so a rotation must start above it.
    seenWrapEpoch: wrapEpoch,
    // Zeroes every secret this identity holds (the secret, the signing seed, every wrap generation).
    wipe() {
      secret32.fill(0);
      base.edSeed.fill(0);
      for (const g of gens) {
        g.xSecret.fill(0);
        g.mlkemDk.fill(0);
      }
    },
    unwrap(blob) {
      let last = new Error('no wrap key generation');
      for (let i = gens.length - 1; i >= 0; i--) {
        try {
          const k = tryGen(i, blob);
          ident.seenWrapEpoch = Math.max(ident.seenWrapEpoch, i);
          return k;
        } catch (e) {
          last = e;
        }
      }
      for (let i = wrapEpoch + 1; i <= WRAP_GEN_MAX; i++) {
        if (!gens[i]) gens[i] = wrapGeneration(secret32, i);
        try {
          const k = tryGen(i, blob);
          ident.seenWrapEpoch = Math.max(ident.seenWrapEpoch, i);
          return k;
        } catch (e) {
          last = e;
        }
      }
      throw last;
    },
  };
  return ident;
}

export const generateSecret = () => randomBytes(32);
export const secretToWords = (secret32) => bip39Encode(secret32);
export const secretFromWords = (phrase) => bip39Decode(phrase);

export async function wrapSecret(secret32, pin, profile = PIN_PROFILE, salt = randomBytes(16), nonce = randomBytes(24)) {
  const header = concat(MAGIC, Uint8Array.of(1), u32(profile.mem_kib), u32(profile.iters), salt, nonce);
  const key = await argon2idDerive(pin, salt, profile);
  return concat(header, xchacha20poly1305(key, nonce, header).encrypt(secret32));
}

// Throws 'wrong PIN' on an AEAD failure; the profile is bounded BEFORE any Argon2 work.
export async function unwrapSecret(blob, pin) {
  if (blob.length !== HEADER + 32 + 16 || !MAGIC.every((m, i) => blob[i] === m) || blob[4] !== 1) throw new Error('stored key is damaged');
  const profile = { mem_kib: rd32(blob, 5), iters: rd32(blob, 9) };
  if (profile.mem_kib < 8 || profile.mem_kib > MAX_MEM_KIB || profile.iters < 1 || profile.iters > MAX_ITERS) throw new Error('stored key has an unreasonable profile');
  const key = await argon2idDerive(pin, blob.slice(13, 29), profile);
  try {
    return xchacha20poly1305(key, blob.slice(29, 53), blob.slice(0, HEADER)).decrypt(blob.slice(HEADER));
  } catch {
    throw new Error('wrong PIN');
  }
}
