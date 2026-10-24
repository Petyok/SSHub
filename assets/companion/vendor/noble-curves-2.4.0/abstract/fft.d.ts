/**
 * Experimental implementation of NTT / FFT (Fast Fourier Transform) over finite fields.
 * API may change at any time. The code has not been audited. Feature requests are welcome.
 * @module
 */
import { type TArg } from '../utils.ts';
import { type IField } from './modular.ts';
/** Array-like coefficient storage that can be mutated in place. */
export interface MutableArrayLike<T> {
    /** Element access by numeric index. */
    [index: number]: T;
    /** Current amount of stored coefficients. */
    length: number;
    /**
     * Return a sliced copy using the same storage shape.
     * @param start - Inclusive start index.
     * @param end - Exclusive end index.
     * @returns Sliced copy.
     */
    slice(start?: number, end?: number): this;
    /**
     * Iterate over stored coefficients in order.
     * @returns Coefficient iterator.
     */
    [Symbol.iterator](): Iterator<T>;
}
/**
 * Concrete polynomial containers accepted by the high-level `poly(...)` helpers.
 * Lower-level FFT helpers can work with structural `MutableArrayLike`, but `poly(...)`
 * intentionally keeps runtime dispatch on plain arrays and typed-array views.
 */
export type PolyStorage<T> = T[] | (MutableArrayLike<T> & ArrayBufferView);
/**
 * Checks if integer is in form of `1 << X`.
 * @param x - Integer to inspect.
 * @returns `true` when the value is a power of two.
 * @example
 * Validate that an FFT size is a power of two.
 *
 * ```ts
 * isPowerOfTwo(8);
 * ```
 */
export declare function isPowerOfTwo(x: number): boolean;
/**
 * @param n - Input value.
 * @returns Next power of two within the u32/array-length domain.
 * @throws If `n` is not a valid unsigned 32-bit integer. {@link Error}
 * @example
 * Round an integer up to the FFT size it needs.
 *
 * ```ts
 * nextPowerOfTwo(9);
 * ```
 */
export declare function nextPowerOfTwo(n: number): number;
/**
 * @param n - Value to reverse.
 * @param bits - Number of bits to use.
 * @returns Bit-reversed integer.
 * @throws If `n` is not a valid unsigned 32-bit integer. {@link Error}
 * @example
 * Reverse the low `bits` bits of one index.
 *
 * ```ts
 * reverseBits(3, 3);
 * ```
 */
export declare function reverseBits(n: number, bits: number): number;
/**
 * Similar to `bitLen(x)-1` but much faster for small integers, like indices.
 * @param n - Input value.
 * @returns Base-2 logarithm. For `n = 0`, the current implementation returns `-1`.
 * @example
 * Compute the radix-2 stage count for one transform size.
 *
 * ```ts
 * log2(8);
 * ```
 */
export declare function log2(n: number): number;
/**
 * Moves lowest bit to highest position, which at first step splits
 * array on even and odd indices, then it applied again to each part,
 * which is core of fft
 * @param values - Mutable coefficient array.
 * @returns Mutated input array.
 * @throws If the array length is not a positive power of two. {@link Error}
 * @example
 * Reorder coefficients into bit-reversed order in place.
 *
 * ```ts
 * const values = Uint8Array.from([0, 1, 2, 3]);
 * bitReversalInplace(values);
 * ```
 */
export declare function bitReversalInplace<T extends MutableArrayLike<any>>(values: T): T;
/**
 * @param values - Input values.
 * @returns Reordered copy.
 * @throws If the array length is not a positive power of two. {@link Error}
 * @example
 * Return a reordered copy instead of mutating the input in place.
 *
 * ```ts
 * const reordered = bitReversalPermutation([0, 1, 2, 3]);
 * ```
 */
export declare function bitReversalPermutation<T>(values: T[]): T[];
/** Cached roots-of-unity tables derived from one finite field. */
export type RootsOfUnity = {
    /** Generator and 2-adicity metadata for the cached field. */
    info: {
        G: bigint;
        oddFactor: bigint;
        powerOfTwo: number;
    };
    /**
     * Return the natural-order roots of unity for one radix-2 size.
     * @param bits - Transform size as `log2(N)`.
     * @returns Natural-order roots for that size.
     */
    roots: (bits: number) => bigint[];
    /**
     * Return the bit-reversal permutation of the roots for one radix-2 size.
     * @param bits - Transform size as `log2(N)`.
     * @returns Bit-reversed roots.
     */
    brp(bits: number): bigint[];
    /**
     * Return the inverse roots of unity for one radix-2 size.
     * @param bits - Transform size as `log2(N)`.
     * @returns Inverse roots.
     */
    inverse(bits: number): bigint[];
    /**
     * Return one primitive root used by a radix-2 stage.
     * @param bits - Transform size as `log2(N)`.
     * @returns Primitive root for that stage.
     */
    omega: (bits: number) => bigint;
    /**
     * Drop all cached root tables.
     * @returns Nothing.
     */
    clear: () => void;
};
/**
 * We limit roots up to 2**31, which is a lot: 2-billion polynomial should be rare.
 * @param field - Field implementation.
 * @param generator - Optional trusted non-quadratic-residue override for callers that already know the field.
 * @returns Roots-of-unity cache.
 * @example
 * Cache roots once, then ask for the omega table of one FFT size.
 *
 * ```ts
 * import { rootsOfUnity } from '@noble/curves/abstract/fft.js';
 * import { Field } from '@noble/curves/abstract/modular.js';
 * const roots = rootsOfUnity(Field(17n));
 * const omega = roots.omega(4);
 * ```
 */
export declare function rootsOfUnity(field: TArg<IField<bigint>>, generator?: bigint): RootsOfUnity;
/** Polynomial coefficient container used by the FFT helpers. */
export type Polynomial<T> = MutableArrayLike<T>;
/**
 * Arithmetic operations used by the generic FFT implementation.
 *
 * Maps great to Field<bigint>, but not to Group (EC points):
 * - inv from scalar field
 * - we need multiplyUnsafe here, instead of multiply for speed
 * - multiplyUnsafe is safe in the context: we do mul(rootsOfUnity), which are public and sparse
 */
export type FFTOpts<T, R> = {
    /**
     * Add two coefficients.
     * @param a - Left coefficient.
     * @param b - Right coefficient.
     * @returns Sum coefficient.
     */
    add: (a: T, b: T) => T;
    /**
     * Subtract two coefficients.
     * @param a - Left coefficient.
     * @param b - Right coefficient.
     * @returns Difference coefficient.
     */
    sub: (a: T, b: T) => T;
    /**
     * Multiply one coefficient by a scalar/root factor.
     * @param a - Coefficient value.
     * @param scalar - Scalar/root factor.
     * @returns Scaled coefficient.
     */
    mul: (a: T, scalar: R) => T;
    /**
     * Invert one scalar/root factor.
     * @param a - Scalar/root factor.
     * @returns Inverse factor.
     */
    inv: (a: R) => R;
};
/** Configuration for one low-level FFT loop. */
export type FFTCoreOpts<R> = {
    /** Transform size. Must be a power of two. */
    N: number;
    /** Stage roots for the selected transform size. */
    roots: Polynomial<R>;
    /** Whether to run the DIT variant instead of DIF. */
    dit: boolean;
    /** Whether to invert butterfly placement for decode-oriented layouts. */
    invertButterflies?: boolean;
    /** Number of initial stages to skip. */
    skipStages?: number;
    /** Whether to apply bit-reversal permutation at the boundary. */
    brp?: boolean;
};
/**
 * Callable low-level FFT loop over one polynomial storage shape.
 * @param values - Polynomial coefficients to transform in place.
 * @returns The mutated input polynomial.
 */
export type FFTCoreLoop<T> = <P extends Polynomial<T>>(values: P) => P;
/**
 * Constructs different flavors of FFT. radix2 implementation of low level mutating API. Flavors:
 *
 * - DIT (Decimation-in-Time): Bottom-Up (leaves to root), Cooley-Tukey
 * - DIF (Decimation-in-Frequency): Top-Down (root to leaves), Gentleman-Sande
 *
 * DIT takes brp input, returns natural output.
 * DIF takes natural input, returns brp output.
 *
 * The output is actually identical. Time / frequence distinction is not meaningful
 * for Polynomial multiplication in fields.
 * Which means if protocol supports/needs brp output/inputs, then we can skip this step.
 *
 * Cyclic NTT: Rq = Zq[x]/(x^n-1). butterfly_DIT+loop_DIT OR butterfly_DIF+loop_DIT, roots are omega
 * Negacyclic NTT: Rq = Zq[x]/(x^n+1). butterfly_DIT+loop_DIF, at least for mlkem / mldsa
 *
 * `invertButterflies` indexes roots by a per-butterfly-group counter (`grp`): forward
 * (`dit: false`) reads `roots[grp]` with grp = 1..; inverse (`dit: true`) reads `roots[N - grp]`
 * with grp restarting at 1. With `skipStages: 0` one table serves both directions (ωᴺ = 1 makes
 * the reversed walk self-inverse). With `skipStages > 0` the inverse walk starts at `N - 1`
 * instead of continuing where the skipped stages would have left off, so the caller must supply
 * a table shaped for that (ML-KEM: `ζ^BitRev7(i)` over all N=256 indices, whose aliased upper
 * half is exactly the FIPS 203 inverse walk).
 * @param F - Field operations.
 * @param coreOpts - FFT configuration. See {@link FFTCoreOpts}:
 *   - `N`: Transform size. Must be a power of two.
 *   - `roots`: Stage roots for the selected transform size.
 *   - `dit`: Whether to run the DIT variant instead of DIF.
 *   - `invertButterflies` (optional): Whether to invert butterfly placement.
 *   - `skipStages` (optional): Number of initial stages to skip.
 *   - `brp` (optional): Whether to apply bit-reversal permutation at the boundary.
 * @returns Low-level FFT loop.
 * @throws If the FFT options or cached roots are invalid for the requested size. {@link Error}
 * @example
 * Constructs different flavors of FFT.
 *
 * ```ts
 * import { FFTCore, rootsOfUnity } from '@noble/curves/abstract/fft.js';
 * import { Field } from '@noble/curves/abstract/modular.js';
 * const Fp = Field(17n);
 * const roots = rootsOfUnity(Fp).roots(2);
 * const loop = FFTCore(Fp, { N: 4, roots, dit: true });
 * const values = loop([1n, 2n, 3n, 4n]);
 * ```
 */
export declare const FFTCore: <T, R>(F: FFTOpts<T, R>, coreOpts: FFTCoreOpts<R>) => FFTCoreLoop<T>;
/** Forward and inverse FFT helpers for one coefficient domain. */
export type FFTMethods<T> = {
    /**
     * Apply the forward transform.
     * @param values - Polynomial coefficients to transform.
     * @param brpInput - Whether the input is already bit-reversed.
     * @param brpOutput - Whether to keep the output bit-reversed.
     * @returns Transformed copy.
     */
    direct<P extends Polynomial<T>>(values: P, brpInput?: boolean, brpOutput?: boolean): P;
    /**
     * Apply the inverse transform.
     * @param values - Polynomial coefficients to transform.
     * @param brpInput - Whether the input is already bit-reversed.
     * @param brpOutput - Whether to keep the output bit-reversed.
     * @returns Inverse-transformed copy.
     */
    inverse<P extends Polynomial<T>>(values: P, brpInput?: boolean, brpOutput?: boolean): P;
};
/**
 * NTT aka FFT over finite field (NOT over complex numbers).
 * Naming mirrors other libraries.
 * @param roots - Roots-of-unity cache.
 * @param opts - Field operations. See {@link FFTOpts}.
 * @returns Forward and inverse FFT helpers.
 * @example
 * NTT aka FFT over finite field (NOT over complex numbers).
 *
 * ```ts
 * import { FFT, rootsOfUnity } from '@noble/curves/abstract/fft.js';
 * import { Field } from '@noble/curves/abstract/modular.js';
 * const Fp = Field(17n);
 * const fft = FFT(rootsOfUnity(Fp), Fp);
 * const values = fft.direct([1n, 2n, 3n, 4n]);
 * ```
 */
export declare function FFT<T>(roots: RootsOfUnity, opts: FFTOpts<T, bigint>): FFTMethods<T>;
/**
 * Factory that allocates one polynomial storage container.
 * Callers must ensure `_create(len)` returns field-zero-filled storage when `elm` is omitted,
 * because the quadratic `mul()` / `convolve()` paths and the Kronecker-δ shortcut in
 * `lagrange.basis()` rely on that default instead of always passing `field.ZERO` explicitly.
 * @param len - Requested amount of coefficients.
 * @param elm - Optional fill value.
 * @returns Newly allocated polynomial container.
 */
export type CreatePolyFn<P extends PolyStorage<T>, T> = (len: number, elm?: T) => P;
/** High-level polynomial helpers layered on top of FFT and field arithmetic. */
export type PolyFn<P extends PolyStorage<T>, T> = {
    /** Roots-of-unity cache used by the helper namespace. */
    roots: RootsOfUnity;
    /** Factory used to allocate new polynomial containers. */
    create: CreatePolyFn<P, T>;
    /** Optional enforced polynomial length. */
    length?: number;
    /**
     * Compute the polynomial degree.
     * @param a - Polynomial coefficients.
     * @returns Polynomial degree.
     */
    degree: (a: P) => number;
    /**
     * Extend or truncate one polynomial to a requested length.
     * @param a - Polynomial coefficients.
     * @param len - Target length.
     * @returns Resized polynomial.
     */
    extend: (a: P, len: number) => P;
    /**
     * Add two polynomials coefficient-wise.
     * @param a - Left polynomial.
     * @param b - Right polynomial.
     * @returns Sum polynomial.
     */
    add: (a: P, b: P) => P;
    /**
     * Subtract two polynomials coefficient-wise.
     * @param a - Left polynomial.
     * @param b - Right polynomial.
     * @returns Difference polynomial.
     */
    sub: (a: P, b: P) => P;
    /**
     * Multiply by another polynomial or by one scalar.
     * @param a - Left polynomial.
     * @param b - Right polynomial or scalar.
     * @returns Product polynomial.
     */
    mul: (a: P, b: P | T) => P;
    /**
     * Multiply coefficients point-wise.
     * @param a - Left polynomial.
     * @param b - Right polynomial.
     * @returns Point-wise product polynomial.
     */
    dot: (a: P, b: P) => P;
    /**
     * Multiply two polynomials with convolution.
     * @param a - Left polynomial.
     * @param b - Right polynomial.
     * @returns Convolution product.
     */
    convolve: (a: P, b: P) => P;
    /**
     * Apply a point-wise coefficient shift by powers of one factor.
     * @param p - Polynomial coefficients.
     * @param factor - Shift factor.
     * @returns Shifted polynomial.
     */
    shift: (p: P, factor: bigint) => P;
    /**
     * Clone one polynomial container.
     * @param a - Polynomial coefficients.
     * @returns Cloned polynomial.
     */
    clone: (a: P) => P;
    /**
     * Evaluate one polynomial on a basis vector.
     * @param a - Polynomial coefficients.
     * @param basis - Basis vector.
     * @returns Evaluated field element.
     */
    eval: (a: P, basis: P) => T;
    /** Helpers for monomial-basis polynomials. */
    monomial: {
        /** Build the monomial basis vector for one evaluation point. */
        basis: (x: T, n: number) => P;
        /** Evaluate a polynomial in the monomial basis. */
        eval: (a: P, x: T) => T;
    };
    /** Helpers for Lagrange-basis polynomials. */
    lagrange: {
        /** Build the Lagrange basis vector for one evaluation point. */
        basis: (x: T, n: number, brp?: boolean) => P;
        /** Evaluate a polynomial in the Lagrange basis. */
        eval: (a: P, x: T, brp?: boolean) => T;
    };
    /**
     * Build the vanishing polynomial for a root set.
     * @param roots - Root set.
     * @returns Vanishing polynomial.
     */
    vanishing: (roots: P) => P;
};
/**
 * Poly wants a cracker.
 *
 * Polynomials are functions like `y=f(x)`, which means when we multiply two polynomials, result is
 * function `f3(x) = f1(x) * f2(x)`, we don't multiply values. Key takeaways:
 *
 * - **Polynomial** is an array of coefficients: `f(x) = sum(coeff[i] * basis[i](x))`
 * - **Basis** is array of functions
 * - **Monominal** is Polynomial where `basis[i](x) == x**i` (powers)
 * - **Array size** is domain size
 * - **Lattice** is matrix (Polynomial of Polynomials)
 * @param field - Field implementation.
 * @param roots - Roots-of-unity cache.
 * @param create - Optional polynomial factory. Runtime input validation accepts only plain `Array`
 *   and typed-array polynomial containers; arbitrary structural wrappers are intentionally rejected.
 * @param fft - Optional FFT implementation.
 * @param length - Optional fixed polynomial length.
 * @returns Polynomial helper namespace.
 * @example
 * Build polynomial helpers, then convolve two coefficient arrays.
 *
 * ```ts
 * import { poly, rootsOfUnity } from '@noble/curves/abstract/fft.js';
 * import { Field } from '@noble/curves/abstract/modular.js';
 * const Fp = Field(17n);
 * const poly17 = poly(Fp, rootsOfUnity(Fp));
 * const product = poly17.convolve([1n, 2n], [3n, 4n]);
 * ```
 */
export declare function poly<T>(field: TArg<IField<T>>, roots: RootsOfUnity, create?: undefined, fft?: FFTMethods<T>, length?: number): PolyFn<T[], T>;
export declare function poly<T, P extends PolyStorage<T>>(field: TArg<IField<T>>, roots: RootsOfUnity, create: CreatePolyFn<P, T>, fft?: FFTMethods<T>, length?: number): PolyFn<P, T>;
