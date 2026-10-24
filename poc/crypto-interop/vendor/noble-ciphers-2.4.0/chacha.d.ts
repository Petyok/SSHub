/**
 * ChaCha stream cipher, released
 * in 2008. Developed after Salsa20, ChaCha aims to increase diffusion per round.
 * It was standardized in
 * {@link https://www.rfc-editor.org/rfc/rfc8439 | RFC 8439} and
 * is now used in TLS 1.3.
 *
 * {@link https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-xchacha | XChaCha20}
 * extended-nonce variant is also provided. Similar to XSalsa, it's safe to use with
 * randomly-generated nonces.
 *
 * Check out
 * {@link http://cr.yp.to/chacha/chacha-20080128.pdf | PDF},
 * {@link https://en.wikipedia.org/wiki/Salsa20 | wiki}, and
 * {@link https://cr.yp.to/chacha.html | website}.
 *
 * @module
 */
import { type XorPRG } from './_arx.ts';
import { type ARXCipher, type CipherWithOutput, type TArg, type TRet, type XorStream } from './utils.ts';
/**
 * ChaCha core function. Uses an unrolled loop (chachaCore, hchacha) - 4x
 * faster than a simple loop, but larger & harder to read. A simple-loop
 * reference version lives in `test/misc/micro-ciphers.ts`;
 * `test/arx.test.ts` keeps the two aligned.
 * The specific implementation is selected in `createCipher` below.
 */
/** RFC 8439 §2.3 block core for `state = constants | key | counter | nonce`. */
declare function chachaCore(s: TArg<Uint32Array>, k: TArg<Uint32Array>, n: TArg<Uint32Array>, out: TArg<Uint32Array>, cnt: number, rounds?: number): void;
/**
 * hchacha hashes key and nonce into key' and nonce' for xchacha20.
 * Algorithmically identical to `hchacha_small` from `test/misc/micro-ciphers.ts`,
 * but this exported path normalizes word order on big-endian hosts.
 * Reuses `chachaCore` and subtracts its feed-forward, keeping the hot per-block
 * path untouched.
 * @param s - Sigma constants as 32-bit words.
 * @param k - Key words.
 * @param i - Nonce-prefix words.
 * @param out - Output buffer for the derived subkey.
 * @example
 * Derives the XChaCha subkey from sigma, key, and nonce-prefix words.
 *
 * ```ts
 * const sigma = new Uint32Array(4);
 * const key = new Uint32Array(8);
 * const nonce = new Uint32Array(4);
 * const out = new Uint32Array(8);
 * hchacha(sigma, key, nonce, out);
 * ```
 */
export declare function hchacha(s: TArg<Uint32Array>, k: TArg<Uint32Array>, i: TArg<Uint32Array>, out: TArg<Uint32Array>): void;
/**
 * Original, non-RFC chacha20 from DJB. 8-byte nonce, 8-byte counter.
 * The nonce/counter layout still reserves 8 counter bytes internally, but the shared public
 * `counter` argument follows noble's strict non-wrapping 32-bit policy. See `src/_arx.ts`
 * near `MAX_COUNTER` for the full counter-policy rationale.
 * @param key - 16-byte or 32-byte key.
 * @param nonce - 8-byte nonce.
 * @param data - Input bytes to xor with the keystream.
 * @param output - Optional destination buffer.
 * @param counter - Initial block counter.
 * @returns Encrypted or decrypted bytes.
 * @example
 * Encrypts bytes with the original 8-byte-nonce ChaCha variant and a fresh key/nonce.
 *
 * ```ts
 * import { chacha20orig } from '@noble/ciphers/chacha.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const key = randomBytes(32);
 * const nonce = randomBytes(8);
 * chacha20orig(key, nonce, new Uint8Array(4));
 * ```
 */
export declare const chacha20orig: TRet<XorStream>;
/**
 * ChaCha stream cipher. Conforms to RFC 8439 (IETF, TLS). 12-byte nonce, 4-byte counter.
 * With smaller nonce, it's not safe to make it random (CSPRNG), due to collision chance.
 * @param key - 32-byte key.
 * @param nonce - 12-byte nonce.
 * @param data - Input bytes to xor with the keystream.
 * @param output - Optional destination buffer.
 * @param counter - Initial block counter.
 * @returns Encrypted or decrypted bytes.
 * @example
 * Encrypts bytes with the RFC 8439 ChaCha20 stream cipher and a fresh key/nonce.
 *
 * ```ts
 * import { chacha20 } from '@noble/ciphers/chacha.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const key = randomBytes(32);
 * const nonce = randomBytes(12);
 * chacha20(key, nonce, new Uint8Array(4));
 * ```
 */
export declare const chacha20: TRet<XorStream>;
/**
 * XChaCha eXtended-nonce ChaCha. With 24-byte nonce, it's safe to make it random (CSPRNG).
 * See {@link https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-xchacha | the IRTF draft}.
 * The nonce/counter layout still reserves 8 counter bytes internally, but the shared public
 * `counter` argument follows noble's strict non-wrapping 32-bit policy. See `src/_arx.ts`
 * near `MAX_COUNTER` for the full counter-policy rationale.
 * @param key - 32-byte key.
 * @param nonce - 24-byte extended nonce.
 * @param data - Input bytes to xor with the keystream.
 * @param output - Optional destination buffer.
 * @param counter - Initial block counter.
 * @returns Encrypted or decrypted bytes.
 * @example
 * Encrypts bytes with XChaCha20 using a fresh key and random 24-byte nonce.
 *
 * ```ts
 * import { xchacha20 } from '@noble/ciphers/chacha.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const key = randomBytes(32);
 * const nonce = randomBytes(24);
 * xchacha20(key, nonce, new Uint8Array(4));
 * ```
 */
export declare const xchacha20: TRet<XorStream>;
/**
 * Reduced 8-round chacha, described in original paper.
 * @param key - 32-byte key.
 * @param nonce - 12-byte nonce.
 * @param data - Input bytes to xor with the keystream.
 * @param output - Optional destination buffer.
 * @param counter - Initial block counter.
 * @returns Encrypted or decrypted bytes.
 * @example
 * Uses the reduced 8-round variant for non-critical workloads with a fresh key/nonce.
 *
 * ```ts
 * import { chacha8 } from '@noble/ciphers/chacha.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const key = randomBytes(32);
 * const nonce = randomBytes(12);
 * chacha8(key, nonce, new Uint8Array(4));
 * ```
 */
export declare const chacha8: TRet<XorStream>;
/**
 * Reduced 12-round chacha, described in original paper.
 * @param key - 32-byte key.
 * @param nonce - 12-byte nonce.
 * @param data - Input bytes to xor with the keystream.
 * @param output - Optional destination buffer.
 * @param counter - Initial block counter.
 * @returns Encrypted or decrypted bytes.
 * @example
 * Uses the reduced 12-round variant for non-critical workloads with a fresh key/nonce.
 *
 * ```ts
 * import { chacha12 } from '@noble/ciphers/chacha.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const key = randomBytes(32);
 * const nonce = randomBytes(12);
 * chacha12(key, nonce, new Uint8Array(4));
 * ```
 */
export declare const chacha12: TRet<XorStream>;
export declare const __TESTS: {
    chachaCore: typeof chachaCore;
};
/**
 * AEAD algorithm from RFC 8439.
 * Salsa20 and chacha (RFC 8439) use poly1305 differently.
 * We could have composed them, but it's hard because of authKey:
 * In salsa20, authKey changes position in salsa stream.
 * In chacha, authKey can't be computed inside computeTag, it modifies the counter.
 */
export declare const _poly1305_aead: (xorStream: TArg<XorStream>) => (key: TArg<Uint8Array>, nonce: TArg<Uint8Array>, AAD?: TArg<Uint8Array>) => CipherWithOutput;
/**
 * ChaCha20-Poly1305 from RFC 8439.
 *
 * Unsafe to use random nonces under the same key, due to collision chance.
 * Prefer XChaCha instead.
 * @param key - 32-byte key.
 * @param nonce - 12-byte nonce.
 * @param AAD - Additional authenticated data.
 * @returns AEAD cipher instance.
 * @example
 * Encrypts and authenticates plaintext with a fresh key and nonce.
 *
 * ```ts
 * import { chacha20poly1305 } from '@noble/ciphers/chacha.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const key = randomBytes(32);
 * const nonce = randomBytes(12);
 * const aad = new TextEncoder().encode('session metadata');
 * const cipher = chacha20poly1305(key, nonce, aad);
 * cipher.encrypt(new Uint8Array([1, 2, 3]));
 * ```
 */
export declare const chacha20poly1305: TRet<ARXCipher & {
    withAAD: true;
}>;
/**
 * XChaCha20-Poly1305 extended-nonce chacha.
 *
 * Can be safely used with random nonces (CSPRNG).
 * See {@link https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-xchacha | the IRTF draft}.
 * @param key - 32-byte key.
 * @param nonce - 24-byte nonce.
 * @param AAD - Additional authenticated data.
 * @returns AEAD cipher instance.
 * @example
 * Encrypts and authenticates plaintext with a fresh key and random 24-byte nonce.
 *
 * ```ts
 * import { xchacha20poly1305 } from '@noble/ciphers/chacha.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const key = randomBytes(32);
 * const nonce = randomBytes(24);
 * const aad = new TextEncoder().encode('session metadata');
 * const cipher = xchacha20poly1305(key, nonce, aad);
 * cipher.encrypt(new Uint8Array([1, 2, 3]));
 * ```
 */
export declare const xchacha20poly1305: TRet<ARXCipher & {
    withAAD: true;
}>;
/**
 * Chacha20 CSPRNG (cryptographically secure pseudorandom number generator).
 * It's best to limit usage to non-production, non-critical cases: for example, test-only.
 * Compatible with libtomcrypt. It does not have a specification, so unclear how secure it is.
 * @param seed - Optional seed bytes mixed into the internal `key || nonce` state. When omitted,
 * only 32 random bytes are mixed into the 40-byte state.
 * @returns Seeded concrete `_XorStreamPRG` instance, including `clone()`.
 * @example
 * Seeds the test-only ChaCha20 DRBG from fresh entropy.
 *
 * ```ts
 * import { rngChacha20 } from '@noble/ciphers/chacha.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const seed = randomBytes(32);
 * const prg = rngChacha20(seed);
 * prg.randomBytes(8);
 * ```
 */
export declare const rngChacha20: TRet<XorPRG>;
/**
 * Chacha20/8 CSPRNG (cryptographically secure pseudorandom number generator).
 * It's best to limit usage to non-production, non-critical cases: for example, test-only.
 * Faster than `rngChacha20`.
 * @param seed - Optional seed bytes mixed into the internal `key || nonce` state. When omitted,
 * only 32 random bytes are mixed into the 44-byte state.
 * @returns Seeded concrete `_XorStreamPRG` instance, including `clone()`.
 * @example
 * Seeds the faster test-only ChaCha8 DRBG from fresh entropy.
 *
 * ```ts
 * import { rngChacha8 } from '@noble/ciphers/chacha.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const seed = randomBytes(32);
 * const prg = rngChacha8(seed);
 * prg.randomBytes(8);
 * ```
 */
export declare const rngChacha8: TRet<XorPRG>;
export {};
