/**
 * Montgomery curve methods. It's not really whole montgomery curve,
 * just bunch of very specific methods for X25519 / X448 from
 * [RFC 7748](https://www.rfc-editor.org/rfc/rfc7748)
 * @module
 */
/*! noble-curves - MIT License (c) 2022 Paul Miller (paulmillr.com) */
import { type TArg, type TRet } from '../utils.ts';
import { type CurveLengths } from './curve.ts';
/** Curve-specific hooks required to build one X25519/X448 helper. */
export type MontgomeryOpts = {
    /** Prime field modulus. */
    P: bigint;
    /** RFC 7748 variant name. */
    type: 'x25519' | 'x448';
    /**
     * Clamp or otherwise normalize one scalar byte string before use.
     * @param bytes - Raw secret scalar bytes.
     * @returns Adjusted scalar bytes ready for Montgomery multiplication.
     */
    adjustScalarBytes: (bytes: TArg<Uint8Array>) => TRet<Uint8Array>;
    /**
     * Invert one field element with exponentiation by `p - 2`.
     * @param x - Field element to invert.
     * @returns Multiplicative inverse of `x`.
     */
    powPminus2: (x: bigint) => bigint;
    /**
     * Optional randomness source for `keygen()` and `utils.randomSecretKey()`.
     * @param bytesLength - Requested byte length.
     * @returns Random bytes.
     */
    randomBytes?: (bytesLength?: number) => TRet<Uint8Array>;
    /**
     * Optional fast fixed-base multiplication, replacing the Montgomery ladder in
     * `scalarMultBase()` / `getPublicKey()` only. Standard implementation computes `[k]B` on the
     * equivalent Edwards curve with cached base-point tables and maps the result back to a
     * Montgomery `u` coordinate (libsodium does the same for X25519); ~3x faster than the ladder.
     * @param k - Decoded, clamped scalar; guaranteed to be in the RFC 7748 clamped range.
     * @returns `u([k]G)` as an integer. Must return `0` when `[k]G` is the point at infinity
     *   (`k ≡ 0 mod n`) so the caller can reject it exactly like the ladder path does.
     */
    scalarMultBase?: (k: bigint) => bigint;
};
/** Public X25519/X448 ECDH API built on a Montgomery ladder. */
export type MontgomeryECDH = {
    /**
     * Multiply one scalar by one Montgomery `u` coordinate.
     * @param scalar - Secret scalar bytes.
     * @param u - Public Montgomery `u` coordinate.
     * @returns Shared point encoded as bytes.
     */
    scalarMult: (scalar: TArg<Uint8Array>, u: TArg<Uint8Array>) => TRet<Uint8Array>;
    /**
     * Multiply one scalar by the curve base point.
     * @param scalar - Secret scalar bytes.
     * @returns Public key bytes.
     */
    scalarMultBase: (scalar: TArg<Uint8Array>) => TRet<Uint8Array>;
    /**
     * Derive a shared secret from a local secret key and peer public key.
     * @param secretKeyA - Local secret key bytes.
     * @param publicKeyB - Peer public key bytes.
     * Rejects low-order public inputs instead of returning the all-zero shared secret.
     * @returns Shared secret bytes.
     */
    getSharedSecret: (secretKeyA: TArg<Uint8Array>, publicKeyB: TArg<Uint8Array>) => TRet<Uint8Array>;
    /**
     * Derive one public key from a secret key.
     * @param secretKey - Secret key bytes.
     * @returns Public key bytes.
     */
    getPublicKey: (secretKey: TArg<Uint8Array>) => TRet<Uint8Array>;
    /** Utility helpers for secret-key generation. */
    utils: {
        /** Generate one random secret key with the curve's expected byte length. */
        randomSecretKey: () => TRet<Uint8Array>;
    };
    /** Encoded Montgomery base point `u`. */
    GuBytes: TRet<Uint8Array>;
    /** Public lengths for keys and seeds. */
    lengths: CurveLengths;
    /**
     * Generate one random secret/public keypair.
     * @param seed - Optional seed bytes to use instead of random generation.
     * @returns Fresh secret/public keypair.
     */
    keygen: (seed?: TArg<Uint8Array>) => {
        secretKey: TRet<Uint8Array>;
        publicKey: TRet<Uint8Array>;
    };
};
/**
 * Selector for cswap(): `P` to keep, `P + 1` to swap, chosen by the low bit of `swap`.
 * Higher bits are ignored, and `swap` is passed in whole rather than as a {0n, 1n} bit on
 * purpose: `P + (swap & _1n)` would short-circuit the addition whenever the bit is clear, which
 * is the very leak this construction avoids, one round-trip further down. Subtracting `swap`
 * with its low bit cleared keeps every operand full-width instead.
 * @param P - Field modulus.
 * @param swap - Value whose low bit selects; ignored above that bit.
 * @returns `P` when the low bit is clear, `P + 1` when it is set.
 */
declare function cmask(P: bigint, swap: bigint): bigint;
/**
 * Swap two field elements when `mask` is `P + 1`, keep them when it is `P`:
 *
 *   d    = 6P + x_3 - x_2
 *   x_2' = d * mask + x_2   (mod P)      x_3' = (x_2 + x_3) - x_2'
 *
 * The extra `6P * mask` vanishes modulo P, so `mask === P` leaves x_2 and `mask === P + 1`
 * leaves x_3. Without the offset, the reduction dividend changes sign with input order and crosses
 * BigInt limb boundaries; those classes measured differently on the tested Node/V8 build. For
 * canonical inputs, the deliberately left-associative `offset + x_3 - x_2` is between 5P and 7P,
 * keeping the dividend positive and in one word-count band for both RFC fields and masks. Six is
 * the smallest coefficient `c` for which the shared offset `cP` has that property.
 *
 * This reduced the tested sign/size timing ratios, but JavaScript BigInt has no constant-time
 * contract and the contents of the multiply and remainder still vary. Valid ladder states can
 * contain genuine zero coordinates; this construction does not mask those value-shape effects.
 * Computing `x_3'` independently as `((6P + x_2 - x_3) * mask + x_3) % P` is more symmetric.
 * On the tested Node/V8 build, it reduced the timing difference between keeping `(0, v)` and
 * swapping `(v, 0)`—both return `(0, v)`—from about 10%/13% for X25519/X448 to about 3%.
 * Successful calls cannot reach that zero-in-the-first-output case. For the case they can reach,
 * swapping `(0, v)` and keeping `(v, 0)` both return `(v, 0)`; the difference instead grew from
 * about 0.7%/1.1% to 2.7%/2.8%. The extra multiply/remainder also made public
 * `getSharedSecret()` about 16% slower. The retained one-remainder form measured about 2.5%
 * slower than the prior helper for public X25519 `getSharedSecret()` in the same environment.
 * x_3' falls out of the sum, which a swap leaves invariant: no second multiply or reduction is
 * needed. Bind `6P` once per field so production and the timing regression exercise the same
 * configured helper without paying for the multiplication in every ladder round.
 *
 * The returned function is called twice per ladder round, so it validates nothing. Both elements
 * MUST already be reduced mod P; unreduced input silently corrupts the kept-side output.
 * @param P - Field modulus.
 * @returns A field-bound swap function taking mask, x_2, and x_3.
 */
declare function cswap(P: bigint): (mask: bigint, x_2: bigint, x_3: bigint) => {
    x_2: bigint;
    x_3: bigint;
};
/** Internal helpers, exported for tests only. Not part of the public API. */
export declare const __TEST: {
    cmask: typeof cmask;
    cswap: typeof cswap;
};
/**
 * @param curveDef - Montgomery curve definition.
 * @returns ECDH helper namespace.
 * @throws If the curve definition or derived shared point is invalid. {@link Error}
 * @example
 * Build an X25519 helper from curve parameters, then derive one public key.
 *
 * ```ts
 * import { montgomery } from '@noble/curves/abstract/montgomery.js';
 * const P = 2n ** 255n - 19n;
 * const mod = (num: bigint) => {
 *   const out = num % P;
 *   return out >= 0n ? out : out + P;
 * };
 * const pow = (num: bigint, power: bigint) => {
 *   let res = 1n;
 *   for (; power > 0n; power >>= 1n) {
 *     if (power & 1n) res = mod(res * num);
 *     num = mod(num * num);
 *   }
 *   return res;
 * };
 * const x25519 = montgomery({
 *   P,
 *   type: 'x25519',
 *   adjustScalarBytes(bytes: Uint8Array) {
 *     bytes[0] &= 248;
 *     bytes[31] &= 127;
 *     bytes[31] |= 64;
 *     return bytes;
 *   },
 *   powPminus2(x) {
 *     return pow(x, P - 2n);
 *   },
 * });
 * const publicKey = x25519.getPublicKey(new Uint8Array(32).fill(1));
 * ```
 */
export declare function montgomery(curveDef: TArg<MontgomeryOpts>): TRet<MontgomeryECDH>;
export {};
