/**
 * WebCrypto-based AES gcm/ctr/cbc: async wrappers over `crypto.subtle` with the
 * same constructor shape as the sync `aes.ts` modes. Combine with `managedNonce`
 * and `randomBytes` from `utils.ts` for automatic nonce handling.
 * We use WebCrypto aka globalThis.crypto, which exists in browsers and node.js 16+.
 * @module
 */
import { type AsyncCipher, type TArg, type TRet } from './utils.ts';
type WebcryptoUtils = {
    encrypt(key: TArg<Uint8Array>, keyParams: unknown, cryptParams: unknown, plaintext: TArg<Uint8Array>): Promise<TRet<Uint8Array>>;
    decrypt(key: TArg<Uint8Array>, keyParams: unknown, cryptParams: unknown, ciphertext: TArg<Uint8Array>): Promise<TRet<Uint8Array>>;
};
/**
 * Internal webcrypto utils. Can be overridden if crypto.subtle is not present,
 * for example in React Native.
 * Raw keys are re-imported on every call; this wrapper intentionally does not
 * cache `CryptoKey` objects between operations.
 */
export declare const utils: TRet<WebcryptoUtils>;
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
export declare const cbc: TRet<((key: TArg<Uint8Array>, iv: TArg<Uint8Array>) => AsyncCipher) & {
    blockSize: number;
    nonceLength: number;
}>;
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
export declare const ctr: TRet<((key: TArg<Uint8Array>, nonce: TArg<Uint8Array>) => AsyncCipher) & {
    blockSize: number;
    nonceLength: number;
}>;
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
export declare const gcm: TRet<((key: TArg<Uint8Array>, nonce: TArg<Uint8Array>, AAD?: TArg<Uint8Array>) => AsyncCipher) & {
    blockSize: number;
    nonceLength: number;
    withAAD: true;
}>;
export {};
