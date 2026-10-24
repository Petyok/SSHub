// The phone's identity: 32 random bytes (the secret), from which the Ed25519, X25519 and ML-KEM
// keys are derived with the same BLAKE3 derive_key construction as every other identity
// (lib.identityFromIkm). Context `sshub/sync/phone/v1`; the 24 BIP39 words ARE the secret, so
// writing them down is the backup and typing them into any phone restores the same device.
// At rest the secret is wrapped under the PIN: Argon2id -> XChaCha20-Poly1305, the profile in
// the header (and under AAD) like the key file of contract section 1:
//   "SSHP" || 1 || mem u32 BE || iters u32 BE || salt(16) || nonce(24) || ct(32 + 16)
import { xchacha20poly1305 } from '../vendor/noble-ciphers-2.4.0/chacha.js';
import { argon2idDerive, bip39Decode, bip39Encode, concat, deviceIdOf, identityFromIkm } from './lib.mjs';

export const PHONE_CONTEXT = 'sshub/sync/phone/v1';
// 64 MiB x 3: about a second on a phone. A short PIN is the weak link whatever the profile is;
// the profile only prices an offline guess against a stolen browser profile.
export const PIN_PROFILE = { mem_kib: 65536, iters: 3 };
export const PIN_MIN_CHARS = 6;
const MAGIC = new Uint8Array([0x53, 0x53, 0x48, 0x50]); // "SSHP"
const HEADER = 4 + 1 + 4 + 4 + 16 + 24;
const MAX_MEM_KIB = 512 * 1024;
const MAX_ITERS = 16;
const u32 = (n) => Uint8Array.of((n >>> 24) & 0xff, (n >>> 16) & 0xff, (n >>> 8) & 0xff, n & 0xff);
const rd32 = (b, at) => ((b[at] << 24) | (b[at + 1] << 16) | (b[at + 2] << 8) | b[at + 3]) >>> 0;

export const randomBytes = (n) => globalThis.crypto.getRandomValues(new Uint8Array(n));

export function identityOfSecret(secret32) {
  const id = identityFromIkm(secret32, PHONE_CONTEXT);
  return { ...id, secret: secret32, deviceId: deviceIdOf(id.edPub) };
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
