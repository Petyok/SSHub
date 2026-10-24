import { type Cipher, type TArg, type TRet } from './utils.ts';
declare function getFF1RadixBytes(radix: number, v: number): number;
export declare const __TESTS: {
    getFF1RadixBytes: typeof getFF1RadixBytes;
};
/**
 * FPE-FF1 format-preserving encryption.
 * @param radix - Alphabet size for each input digit.
 * @param key - AES key bytes.
 * @param tweak - Optional tweak bytes.
 * @returns Encrypt/decrypt helpers over digit arrays.
 * @example
 * Encrypts decimal digits without changing their format, using a fresh AES key.
 *
 * ```ts
 * import { FF1 } from '@noble/ciphers/ff1.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const key = randomBytes(16);
 * const ff1 = FF1(10, key);
 * ff1.encrypt([1, 2, 3]);
 * ```
 */
export declare function FF1(radix: number, key: TArg<Uint8Array>, tweak?: TArg<Uint8Array>): {
    encrypt(x: number[]): number[];
    decrypt(x: number[]): number[];
};
/**
 * Binary FPE-FF1 wrapper over byte arrays.
 * @param key - AES key bytes.
 * @param tweak - Optional tweak bytes.
 * @returns Encrypt/decrypt helpers over byte arrays.
 * @example
 * Encrypts raw bytes through FF1's binary alphabet wrapper with a fresh AES key.
 *
 * ```ts
 * import { BinaryFF1 } from '@noble/ciphers/ff1.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const key = randomBytes(16);
 * const ff1 = BinaryFF1(key);
 * ff1.encrypt(new Uint8Array([1, 2, 3]));
 * ```
 */
export declare function BinaryFF1(key: TArg<Uint8Array>, tweak?: TArg<Uint8Array>): TRet<Cipher>;
export {};
