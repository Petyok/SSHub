import { type ARXCipher, type TArg, type TRet, type XorStream } from './utils.ts';
/**
 * Salsa20 core function. Uses an unrolled loop (salsaCore, hsalsa) - 4x
 * faster than a simple loop, but larger & harder to read. A simple-loop
 * reference version lives in `test/misc/micro-ciphers.ts`;
 * `test/arx.test.ts` keeps the two aligned.
 * The specific implementation is selected in `createCipher` below.
 * Performance numbers for 1MB inputs:
 * * default x 779 ops/sec @ 1ms/op
 * * if salsa+hsalsa are merged x 459 ops/sec @ 2ms/op
 * * small x 132 ops/sec @ 7ms/op
 */
/** Uses only the low 32 bits of Salsa20's 64-bit counter state. */
declare function salsaCore(s: TArg<Uint32Array>, k: TArg<Uint32Array>, n: TArg<Uint32Array>, out: TArg<Uint32Array>, cnt: number, rounds?: number, cntHi?: number): void;
/**
 * hsalsa hashes key and nonce-prefix words into the 32-byte subkey used by XSalsa20.
 * Algorithmically identical to `hsalsa_small` from `test/misc/micro-ciphers.ts`,
 * but this exported path normalizes word order on big-endian hosts.
 * Reuses `salsaCore` and subtracts its feed-forward, keeping the hot per-block
 * path untouched.
 * @param s - Sigma constants as 32-bit words.
 * @param k - Key words.
 * @param i - Nonce-prefix words.
 * @param out - Output buffer for the derived subkey.
 * @example
 * Derives the XSalsa20 subkey from sigma, key, and nonce-prefix words.
 *
 * ```ts
 * const sigma = new Uint32Array(4);
 * const key = new Uint32Array(8);
 * const nonce = new Uint32Array(4);
 * const out = new Uint32Array(8);
 * hsalsa(sigma, key, nonce, out);
 * ```
 */
export declare function hsalsa(s: TArg<Uint32Array>, k: TArg<Uint32Array>, i: TArg<Uint32Array>, out: TArg<Uint32Array>): void;
/**
 * Salsa20 from original paper. 8-byte nonce.
 * With smaller nonce, it's not safe to make it random (CSPRNG), due to collision chance.
 * @param key - 16-byte or 32-byte key.
 * @param nonce - 8-byte nonce.
 * @param data - Input bytes to xor with the keystream.
 * @param output - Optional destination buffer.
 * @param counter - Initial block counter.
 * Only the low 32 bits of Salsa20's 64-bit counter state are exposed here;
 * the high word stays zero and the implementation still caps the public
 * value to 32 bits.
 * @returns Encrypted or decrypted bytes.
 * @example
 * Encrypts bytes with the original 8-byte-nonce Salsa20 stream cipher.
 *
 * ```ts
 * import { salsa20 } from '@noble/ciphers/salsa.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const key = randomBytes(32);
 * const nonce = randomBytes(8);
 * salsa20(key, nonce, new Uint8Array([1, 2, 3, 4]));
 * ```
 */
export declare const salsa20: TRet<XorStream>;
/**
 * XSalsa20 extended-nonce salsa.
 * With 24-byte nonce, it's safe to make it random (CSPRNG).
 * @param key - 32-byte key.
 * This XSalsa20 wrapper does not enable Salsa20's 16-byte legacy key mode.
 * @param nonce - 24-byte nonce.
 * @param data - Input bytes to xor with the keystream.
 * @param output - Optional destination buffer.
 * @param counter - Initial block counter.
 * @returns Encrypted or decrypted bytes.
 * @example
 * Encrypts bytes with XSalsa20 and a random 24-byte nonce.
 *
 * ```ts
 * import { xsalsa20 } from '@noble/ciphers/salsa.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const key = randomBytes(32);
 * const nonce = randomBytes(24);
 * xsalsa20(key, nonce, new Uint8Array([1, 2, 3, 4]));
 * ```
 */
export declare const xsalsa20: TRet<XorStream>;
export declare const __TESTS: {
    salsaCore: typeof salsaCore;
};
/**
 * xsalsa20-poly1305 eXtended-nonce (24 bytes) salsa.
 * With 24-byte nonce, it's safe to make it random (CSPRNG).
 * Also known as `secretbox` from libsodium / nacl.
 * No AAD input is supported here. Caller-provided `output` buffers for
 * `encrypt()` / `decrypt()` must be `input.length + 32` bytes because the
 * implementation uses a 32-byte leading scratch area before returning `+16`.
 * @param key - 32-byte key.
 * @param nonce - 24-byte nonce.
 * @param AAD - Must be omitted; XSalsa20-Poly1305 secretbox does not support associated data.
 * @returns AEAD cipher instance.
 * @example
 * Encrypts and authenticates plaintext with XSalsa20-Poly1305.
 *
 * ```ts
 * import { xsalsa20poly1305 } from '@noble/ciphers/salsa.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const key = randomBytes(32);
 * const nonce = randomBytes(24);
 * const cipher = xsalsa20poly1305(key, nonce);
 * cipher.encrypt(new Uint8Array([1, 2, 3]));
 * ```
 */
export declare const xsalsa20poly1305: TRet<ARXCipher>;
/**
 * Alias to `xsalsa20poly1305`, for compatibility with libsodium / nacl.
 * Check out {@link https://github.com/serenity-kit/noble-sodium | noble-sodium}
 * for `crypto_box`.
 * @param key - 32-byte key.
 * @param nonce - 24-byte nonce.
 * @returns Wrapper with `seal()` and `open()` helpers.
 * @example
 * Uses the libsodium-style `seal()` and `open()` wrapper.
 *
 * ```ts
 * import { secretbox } from '@noble/ciphers/salsa.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const key = randomBytes(32);
 * const nonce = randomBytes(24);
 * const box = secretbox(key, nonce);
 * box.seal(new Uint8Array([1, 2, 3]));
 * ```
 */
export declare function secretbox(key: TArg<Uint8Array>, nonce: TArg<Uint8Array>): TRet<{
    seal: (plaintext: TArg<Uint8Array>, output?: TArg<Uint8Array>) => TRet<Uint8Array>;
    open: (ciphertext: TArg<Uint8Array>, output?: TArg<Uint8Array>) => TRet<Uint8Array>;
}>;
export {};
