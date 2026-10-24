import type { TypedArray } from '@noble/hashes/utils.js';
import { type BytesCoderLen, type Coder, type TRet } from './utils.ts';
/** Extendable-output reader used by the CRYSTALS implementations. */
export type XOF = (seed: Uint8Array, blockLen?: number) => {
    /**
     * Read diagnostic counters for the current XOF session.
     * @returns Current call and XOF block counters.
     */
    stats: () => {
        calls: number;
        xofs: number;
    };
    /**
     * Select one `(x, y)` coordinate pair and get a block reader for it.
     * Only one coordinate stream is live at a time: a later `get(...)` call rebinds the shared
     * SHAKE state and invalidates older readers.
     * Each squeeze aliases one mutable internal output buffer, so callers must copy blocks they
     * want to retain before the next read.
     * @param x - First matrix coordinate.
     * @param y - Second matrix coordinate.
     * @returns Lazy block reader for that coordinate pair.
     */
    get: (x: number, y: number) => () => Uint8Array;
    /** Wipe any buffered state once the reader is no longer needed. */
    clean: () => void;
};
/** CRYSTALS (ml-kem, ml-dsa) options */
/** Shared polynomial and NTT parameters for CRYSTALS algorithms. */
export type CrystalOpts<T extends TypedArray> = {
    /**
     * Allocate one zeroed polynomial/vector container.
     * @param n - Number of coefficients to allocate.
     * @returns Fresh typed container.
     */
    newPoly: TypedCons<T>;
    /** Polynomial size, typically `256`. */
    N: number;
    /** Prime modulus used for all coefficient arithmetic. */
    Q: number;
    /** Inverse transform normalization factor:
     * `256**-1 mod q` for Dilithium, `128**-1 mod q` for Kyber.
     */
    F: number;
    /** Principal root of unity for the transform domain. */
    ROOT_OF_UNITY: number;
    /** Number of bits used for bit-reversal ordering. */
    brvBits: number;
    /** `true` for Kyber/ML-KEM mode, `false` for Dilithium/ML-DSA mode. */
    isKyber: boolean;
};
/** Constructor function for typed polynomial containers. */
export type TypedCons<T extends TypedArray> = (n: number) => T;
type Crystals<T extends TypedArray> = {
    mod: (a: number, modulo?: number) => number;
    smod: (a: number, modulo?: number) => number;
    nttZetas: T;
    NTT: {
        /**
         * Forward transform in place. Mutates and returns `r`.
         * Kyber-mode input coefficients must already use canonical representatives in `[0, Q)`.
         */
        encode: (r: T) => T;
        /**
         * Inverse transform in place. Mutates and returns `r`.
         * Kyber-mode input coefficients must already use canonical representatives in `[0, Q)`.
         */
        decode: (r: T) => T;
    };
    bitsCoder: (d: number, c: Coder<number, number>) => BytesCoderLen<T>;
};
/**
 * Creates shared modular arithmetic, NTT, and packing helpers for CRYSTALS schemes.
 * @param opts - Polynomial and transform parameters. See {@link CrystalOpts}.
 * @returns CRYSTALS arithmetic and encoding helpers.
 * @example
 * Create shared modular arithmetic and NTT helpers for a CRYSTALS parameter set.
 * ```ts
 * const crystals = genCrystals({
 *   newPoly: (n) => new Uint16Array(n),
 *   N: 256,
 *   Q: 3329,
 *   F: 3303,
 *   ROOT_OF_UNITY: 17,
 *   brvBits: 7,
 *   isKyber: true,
 * });
 * const reduced = crystals.mod(-1);
 * ```
 */
export declare const genCrystals: <T extends TypedArray>(opts: CrystalOpts<T>) => TRet<Crystals<T>>;
/**
 * SHAKE128-based extendable-output reader factory used by ML-KEM.
 * `get(x, y)` selects one coordinate pair at a time; calling it again invalidates previously
 * returned readers, and each squeeze reuses one mutable internal output buffer.
 * @param seed - Seed bytes for the reader.
 * @param blockLen - Optional output block length.
 * @returns Stateful XOF reader.
 * @example
 * Build the ML-KEM SHAKE128 matrix expander and read one block.
 * ```ts
 * import { randomBytes } from '@noble/post-quantum/utils.js';
 * import { XOF128 } from '@noble/post-quantum/_crystals.js';
 * const reader = XOF128(randomBytes(32));
 * const block = reader.get(0, 0)();
 * ```
 */
export declare const XOF128: TRet<XOF>;
/**
 * SHAKE256-based extendable-output reader factory used by ML-DSA.
 * `get(x, y)` appends raw one-byte coordinates to the seed, invalidates previously returned
 * readers, and reuses one mutable internal output buffer for each squeeze.
 * @param seed - Seed bytes for the reader.
 * @param blockLen - Optional output block length.
 * @returns Stateful XOF reader.
 * @example
 * Build the ML-DSA SHAKE256 coefficient expander and read one block.
 * ```ts
 * import { randomBytes } from '@noble/post-quantum/utils.js';
 * import { XOF256 } from '@noble/post-quantum/_crystals.js';
 * const reader = XOF256(randomBytes(32));
 * const block = reader.get(0, 0)();
 * ```
 */
export declare const XOF256: TRet<XOF>;
export {};
