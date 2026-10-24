/**
 * WebCrypto-based AES gcm/ctr/cbc: async wrappers over `crypto.subtle` with the
 * same constructor shape as the sync `aes.ts` modes. Combine with `managedNonce`
 * and `randomBytes` from `utils.ts` for automatic nonce handling.
 * We use WebCrypto aka globalThis.crypto, which exists in browsers and node.js 16+.
 * @module
 */
import { abytes, anumber, clean, copyBytes, } from "./utils.js";
function getWebcryptoSubtle() {
    const cr = typeof globalThis !== 'undefined' && globalThis.crypto;
    if (cr && typeof cr.subtle === 'object' && cr.subtle != null)
        return cr.subtle;
    throw new Error('crypto.subtle must be defined');
}
const BUFFER_SOURCE_PARAMS = ['iv', 'counter', 'additionalData'];
function snapshotCryptParams(cryptParams) {
    const params = { ...cryptParams };
    const buffers = [];
    for (const name of BUFFER_SOURCE_PARAMS) {
        if (params[name] === undefined)
            continue;
        const snapshot = copyBytes(params[name]);
        params[name] = snapshot;
        buffers.push(snapshot);
    }
    return { params, buffers };
}
/**
 * Internal webcrypto utils. Can be overridden if crypto.subtle is not present,
 * for example in React Native.
 * Raw keys are re-imported on every call; this wrapper intentionally does not
 * cache `CryptoKey` objects between operations.
 */
export const utils = {
    async encrypt(key, keyParams, cryptParams, plaintext) {
        const cr = getWebcryptoSubtle();
        // Snapshot the small operation state before the first await. Otherwise key import leaves a
        // window where callers can accidentally collapse several operations onto a reused nonce.
        const opKey = copyBytes(key);
        let buffers = [];
        try {
            const snapshot = snapshotCryptParams(cryptParams);
            buffers = snapshot.buffers;
            // Non-extractable: the ephemeral CryptoKey is only used for this operation,
            // so there is no reason to allow exportKey() on it.
            const iKey = await cr.importKey('raw', opKey, keyParams, false, ['encrypt']);
            const ciphertext = await cr.encrypt(snapshot.params, iKey, plaintext);
            return new Uint8Array(ciphertext);
        }
        finally {
            clean(opKey, ...buffers);
        }
    },
    async decrypt(key, keyParams, cryptParams, ciphertext) {
        const cr = getWebcryptoSubtle();
        const opKey = copyBytes(key);
        let buffers = [];
        try {
            const snapshot = snapshotCryptParams(cryptParams);
            buffers = snapshot.buffers;
            // Non-extractable, same as encrypt() above.
            const iKey = await cr.importKey('raw', opKey, keyParams, false, ['decrypt']);
            const plaintext = await cr.decrypt(snapshot.params, iKey, ciphertext);
            return new Uint8Array(plaintext);
        }
        finally {
            clean(opKey, ...buffers);
        }
    },
};
const mode = {
    CBC: 'AES-CBC',
    CTR: 'AES-CTR',
    GCM: 'AES-GCM',
};
function getCryptParams(algo, nonce, AAD) {
    if (algo === mode.CBC)
        return { name: mode.CBC, iv: nonce };
    // WebCrypto allows 1..128 counter bits; use the full block to match sync ctr() / Node CTR wrap.
    if (algo === mode.CTR)
        return { name: mode.CTR, counter: nonce, length: 128 };
    if (algo === mode.GCM) {
        // Rely on the backend default tag length (128 bits) instead of setting it explicitly.
        if (AAD)
            return { name: mode.GCM, iv: nonce, additionalData: AAD };
        else
            return { name: mode.GCM, iv: nonce };
    }
    throw new Error('unknown aes block mode');
}
function generate(algo, nonceLength, withAAD = false) {
    anumber(nonceLength);
    const res = (key, nonce, AAD) => {
        abytes(key, undefined, 'key');
        abytes(nonce, undefined, 'nonce');
        // Reject falsy non-byte AAD locally; otherwise false/0/''/null silently become "no AAD".
        if (AAD !== undefined) {
            if (!withAAD)
                throw new Error('AAD not supported');
            abytes(AAD, undefined, 'AAD');
        }
        // Only GCM consumes AAD. CBC/CTR wrappers are typed without it; any runtime
        // third argument is outside their API and is not passed to WebCrypto params.
        // Exact nonce-length enforcement and WebCrypto-specific AAD normalization are
        // delegated to the backend; locally we only require byte-array inputs here.
        // Keep caller key/nonce/AAD by reference; mutating them after
        // construction changes later operations.
        const keyParams = { name: algo, length: key.length * 8 };
        const cryptParams = getCryptParams(algo, nonce, AAD);
        let consumed = false;
        return {
            // keyLength,
            encrypt(plaintext) {
                abytes(plaintext, undefined, 'data');
                if (consumed)
                    throw new Error('cannot encrypt() twice with same key + nonce');
                consumed = true;
                return utils.encrypt(key, keyParams, cryptParams, plaintext);
            },
            decrypt(ciphertext) {
                abytes(ciphertext, undefined, 'data');
                return utils.decrypt(key, keyParams, cryptParams, ciphertext);
            },
        };
    };
    res.nonceLength = nonceLength;
    res.blockSize = 16; // always for AES
    if (withAAD)
        res.withAAD = true;
    return res;
}
/**
 * AES-CBC implemented with WebCrypto.
 * Uses WebCrypto's built-in PKCS padding behavior; exact IV-length checks are
 * delegated to the backend instead of local `abytes(..., 16)` validation.
 * @param key - AES key bytes.
 * @param iv - 16-byte initialization vector.
 * @returns Async cipher instance.
 * @example
 * Encrypts a block with the browser or Node WebCrypto backend.
 *
 * ```ts
 * import { cbc } from '@noble/ciphers/webcrypto.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const key = randomBytes(16);
 * const iv = randomBytes(16);
 * const cipher = cbc(key, iv);
 * await cipher.encrypt(new Uint8Array(16));
 * ```
 */
export const cbc = /* @__PURE__ */ (() => generate(mode.CBC, 16))();
/**
 * AES-CTR implemented with WebCrypto.
 * Uses WebCrypto's full 128-bit counter-length setting so the whole
 * 16-byte counter block is incremented, matching sync `aes.ts:ctr`.
 * @param key - AES key bytes.
 * @param nonce - 16-byte counter block incremented as a full big-endian AES counter block.
 * @returns Async cipher instance.
 * @example
 * Encrypts a short payload with WebCrypto AES-CTR.
 *
 * ```ts
 * import { ctr } from '@noble/ciphers/webcrypto.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const key = randomBytes(16);
 * const nonce = randomBytes(16);
 * const cipher = ctr(key, nonce);
 * await cipher.encrypt(new Uint8Array([1, 2, 3]));
 * ```
 */
export const ctr = /* @__PURE__ */ (() => generate(mode.CTR, 16))();
/**
 * AES-GCM implemented with WebCrypto.
 * AAD type normalization and nonce-shape enforcement beyond raw bytes are left
 * to the backend WebCrypto implementation.
 * @param key - AES key bytes.
 * @param nonce - Nonce bytes.
 * @param AAD - Additional authenticated data.
 * @returns Async cipher instance.
 * @example
 * Encrypts and authenticates plaintext with WebCrypto AES-GCM.
 *
 * ```ts
 * import { gcm } from '@noble/ciphers/webcrypto.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const key = randomBytes(16);
 * const nonce = randomBytes(12);
 * const cipher = gcm(key, nonce);
 * await cipher.encrypt(new Uint8Array([1, 2, 3]));
 * ```
 */
export const gcm = /* @__PURE__ */ (() => generate(mode.GCM, 12, true))();
// // Type tests
// import { siv, gcm, ctr, ecb, cbc } from '../aes.ts';
// import { xsalsa20poly1305 } from '../salsa.ts';
// import { chacha20poly1305, xchacha20poly1305 } from '../chacha.ts';
// const wsiv = managedNonce(siv);
// const wgcm = managedNonce(gcm);
// const wctr = managedNonce(ctr);
// const wcbc = managedNonce(cbc);
// const wsalsapoly = managedNonce(xsalsa20poly1305);
// const wchacha = managedNonce(chacha20poly1305);
// const wxchacha = managedNonce(xchacha20poly1305);
// // should fail
// const wcbc2 = managedNonce(managedNonce(cbc));
// const wctr = managedNonce(ctr);
// import { gcm as gcmSync } from '../aes.ts';
// const x1 = managedNonce(gcmSync);
// // const x1: (key: Uint8Array, AAD?: Uint8Array | undefined) => Cipher
// const x2 = managedNonce(gcm);
// // const x2: (key: Uint8Array, AAD?: Uint8Array | undefined) => AsyncCipher
