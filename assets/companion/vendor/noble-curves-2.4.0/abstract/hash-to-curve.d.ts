/**
 * hash-to-curve from RFC 9380.
 * Hashes arbitrary-length byte strings to a list of one or more elements of a finite field F.
 * https://www.rfc-editor.org/rfc/rfc9380
 * @module
 */
/*! noble-curves - MIT License (c) 2022 Paul Miller (paulmillr.com) */
import type { CHash, TArg, TRet } from '../utils.ts';
import type { AffinePoint, PC_ANY, PC_F, PC_P } from './curve.ts';
import { type IField } from './modular.ts';
/** ASCII domain-separation tag or raw bytes. */
export type AsciiOrBytes = string | Uint8Array;
type H2CDefaults = {
    DST: AsciiOrBytes;
    expand: 'xmd' | 'xof';
    hash: CHash;
    p: bigint;
    m: number;
    k: number;
    encodeDST?: AsciiOrBytes;
};
/**
 * * `DST` is a domain separation tag, defined in section 2.2.5
 * * `p` characteristic of F, where F is a finite field of characteristic p and order q = p^m
 * * `m` is extension degree (1 for prime fields)
 * * `k` is the target security target in bits (e.g. 128), from section 5.1
 * * `expand` is `xmd` (SHA2, SHA3, BLAKE) or `xof` (SHAKE, BLAKE-XOF)
 * * `hash` conforming to `utils.CHash` interface, with `outputLen` / `blockLen` props
 */
export type H2COpts = {
    /** Domain separation tag. */
    DST: AsciiOrBytes;
    /** Expander family used by RFC 9380. */
    expand: 'xmd' | 'xof';
    /** Hash or XOF implementation used by the expander. */
    hash: CHash;
    /** Base-field characteristic. */
    p: bigint;
    /** Extension degree (`1` for prime fields). */
    m: number;
    /** Target security level in bits. */
    k: number;
};
/** Hash-only subset of RFC 9380 options used by per-call overrides. */
export type H2CHashOpts = {
    /** Expander family used by RFC 9380. */
    expand: 'xmd' | 'xof';
    /** Hash or XOF implementation used by the expander. */
    hash: CHash;
};
/**
 * Map one hash-to-field output tuple onto affine curve coordinates.
 * Implementations receive the validated scalar tuple by reference for performance and MUST treat it
 * as read-only. Callers that need scratch space should copy before mutating.
 * @param scalar - Field-element tuple produced by `hash_to_field`.
 * @returns Affine point before subgroup clearing.
 */
export type MapToCurve<T> = (scalar: bigint[]) => AffinePoint<T>;
/** Per-call override for the domain-separation tag. */
export type H2CDSTOpts = {
    /** Domain-separation tag override. */
    DST: AsciiOrBytes;
};
/** Base hash-to-curve helpers shared by `hashToCurve` and `encodeToCurve`. */
export type H2CHasherBase<PC extends PC_ANY> = {
    /**
     * Hash arbitrary bytes to one curve point.
     * @param msg - Input message bytes.
     * @param options - Optional domain-separation override. See {@link H2CDSTOpts}.
     * @returns Curve point after hash-to-curve.
     */
    hashToCurve(msg: TArg<Uint8Array>, options?: TArg<H2CDSTOpts>): PC_P<PC>;
    /**
     * Hash arbitrary bytes to one scalar.
     * @param msg - Input message bytes.
     * @param options - Optional domain-separation override. See {@link H2CDSTOpts}.
     * @returns Scalar reduced into the target field.
     */
    hashToScalar(msg: TArg<Uint8Array>, options?: TArg<H2CDSTOpts>): bigint;
    /**
     * Derive one curve point from non-uniform bytes without the random-oracle
     * guarantees of `hashToCurve`.
     * Accepts the same arguments as `hashToCurve`, but runs the encode-to-curve
     * path instead of the random-oracle construction.
     */
    deriveToCurve?(msg: TArg<Uint8Array>, options?: TArg<H2CDSTOpts>): PC_P<PC>;
    /** Point constructor for the target curve. */
    Point: PC;
};
/**
 * RFC 9380 methods, with cofactor clearing. See {@link https://www.rfc-editor.org/rfc/rfc9380#section-3 | RFC 9380 section 3}.
 *
 * * hashToCurve: `map(hash(input))`, encodes RANDOM bytes to curve (WITH hashing)
 * * encodeToCurve: `map(hash(input))`, encodes NON-UNIFORM bytes to curve (WITH hashing)
 * * mapToCurve: `map(scalars)`, encodes NON-UNIFORM scalars to curve (NO hashing)
 */
export type H2CHasher<PC extends PC_ANY> = H2CHasherBase<PC> & {
    /**
     * Encode non-uniform bytes to one curve point.
     * @param msg - Input message bytes.
     * @param options - Optional domain-separation override. See {@link H2CDSTOpts}.
     * @returns Curve point after encode-to-curve.
     */
    encodeToCurve(msg: TArg<Uint8Array>, options?: TArg<H2CDSTOpts>): PC_P<PC>;
    /** Deterministic map from `hash_to_field` tuples into affine coordinates. */
    mapToCurve: MapToCurve<PC_F<PC>>;
    /** Default RFC 9380 options captured by this hasher bundle. */
    defaults: H2CDefaults;
};
/**
 * Produces a uniformly random byte string using a cryptographic hash
 * function H that outputs b bits.
 * See {@link https://www.rfc-editor.org/rfc/rfc9380#section-5.3.1 | RFC 9380 section 5.3.1}.
 * @param msg - Input message.
 * @param DST - Domain separation tag. This helper normalizes DST, rejects empty DSTs, and
 *   oversize-hashes DST when needed.
 * @param lenInBytes - Output length.
 * @param H - Hash function.
 * @returns Uniform byte string.
 * @throws If the message, DST, hash, or output length is invalid. {@link Error}
 * @example
 * Expand one message into uniform bytes with the XMD construction.
 *
 * ```ts
 * import { expand_message_xmd } from '@noble/curves/abstract/hash-to-curve.js';
 * import { sha256 } from '@noble/hashes/sha2.js';
 * const uniform = expand_message_xmd(new TextEncoder().encode('hello noble'), 'DST', 32, sha256);
 * ```
 */
export declare function expand_message_xmd(msg: TArg<Uint8Array>, DST: TArg<AsciiOrBytes>, lenInBytes: number, H: TArg<CHash>): TRet<Uint8Array>;
/**
 * Produces a uniformly random byte string using an extendable-output function (XOF) H.
 * 1. The collision resistance of H MUST be at least k bits.
 * 2. H MUST be an XOF that has been proved indifferentiable from
 *    a random oracle under a reasonable cryptographic assumption.
 * See {@link https://www.rfc-editor.org/rfc/rfc9380#section-5.3.2 | RFC 9380 section 5.3.2}.
 * @param msg - Input message.
 * @param DST - Domain separation tag. This helper normalizes DST, rejects empty DSTs, and
 *   oversize-hashes DST when needed.
 * @param lenInBytes - Output length.
 * @param k - Target security level.
 * @param H - XOF hash function.
 * @returns Uniform byte string.
 * @throws If the message, DST, XOF, or output length is invalid. {@link Error}
 * @example
 * Expand one message into uniform bytes with the XOF construction.
 *
 * ```ts
 * import { expand_message_xof } from '@noble/curves/abstract/hash-to-curve.js';
 * import { shake256 } from '@noble/hashes/sha3.js';
 * const uniform = expand_message_xof(
 *   new TextEncoder().encode('hello noble'),
 *   'DST',
 *   32,
 *   128,
 *   shake256
 * );
 * ```
 */
export declare function expand_message_xof(msg: TArg<Uint8Array>, DST: TArg<AsciiOrBytes>, lenInBytes: number, k: number, H: TArg<CHash>): TRet<Uint8Array>;
/**
 * Hashes arbitrary-length byte strings to a list of one or more elements of a finite field F.
 * See {@link https://www.rfc-editor.org/rfc/rfc9380#section-5.2 | RFC 9380 section 5.2}.
 * @param msg - Input message bytes.
 * @param count - Number of field elements to derive. Must be `>= 1`.
 * @param options - RFC 9380 options. See {@link H2COpts}. `m` must be `>= 1`.
 * @returns `[u_0, ..., u_(count - 1)]`, a list of field elements.
 * @throws If the expander choice or RFC 9380 options are invalid. {@link Error}
 * @example
 * Hash one message into field elements before mapping it onto a curve.
 *
 * ```ts
 * import { hash_to_field } from '@noble/curves/abstract/hash-to-curve.js';
 * import { sha256 } from '@noble/hashes/sha2.js';
 * const scalars = hash_to_field(new TextEncoder().encode('hello noble'), 2, {
 *   DST: 'DST',
 *   p: 17n,
 *   m: 1,
 *   k: 128,
 *   expand: 'xmd',
 *   hash: sha256,
 * });
 * ```
 */
export declare function hash_to_field(msg: TArg<Uint8Array>, count: number, options: TArg<H2COpts>): bigint[][];
type XY<T> = (x: T, y: T) => {
    x: T;
    y: T;
};
type XYRatio<T> = [T[], T[], T[], T[]];
/**
 * @param field - Field implementation.
 * @param map - Isogeny coefficients.
 * @returns Isogeny mapping helper.
 * @example
 * Build one rational isogeny map, then apply it to affine x/y coordinates.
 *
 * ```ts
 * import { isogenyMap } from '@noble/curves/abstract/hash-to-curve.js';
 * import { Field } from '@noble/curves/abstract/modular.js';
 * const Fp = Field(17n);
 * const iso = isogenyMap(Fp, [[0n, 1n], [1n], [1n], [1n]]);
 * const point = iso(3n, 5n);
 * ```
 */
export declare function isogenyMap<T, F extends IField<T>>(field: F, map: XYRatio<T>): XY<T>;
export declare const _DST_scalar: "HashToScalar-";
/**
 * Creates hash-to-curve methods from EC Point and mapToCurve function. See {@link H2CHasher}.
 * @param Point - Point constructor.
 * @param mapToCurve - Map-to-curve function.
 * @param defaults - Default hash-to-curve options. A frozen detached snapshot is reused as the
 *   shared defaults bundle for the returned helpers.
 * @returns Hash-to-curve helper namespace.
 * @throws If the map-to-curve callback or default hash-to-curve options are invalid. {@link Error}
 * @example
 * Bundle hash-to-curve, hash-to-scalar, and encode-to-curve helpers for one curve.
 *
 * ```ts
 * import { createHasher } from '@noble/curves/abstract/hash-to-curve.js';
 * import { p256 } from '@noble/curves/nist.js';
 * import { sha256 } from '@noble/hashes/sha2.js';
 * const hasher = createHasher(p256.Point, () => p256.Point.BASE.toAffine(), {
 *   DST: 'P256_XMD:SHA-256_SSWU_RO_',
 *   encodeDST: 'P256_XMD:SHA-256_SSWU_NU_',
 *   p: p256.Point.Fp.ORDER,
 *   m: 1,
 *   k: 128,
 *   expand: 'xmd',
 *   hash: sha256,
 * });
 * const point = hasher.encodeToCurve(new TextEncoder().encode('hello noble'));
 * ```
 */
export declare function createHasher<PC extends PC_ANY>(Point: PC, mapToCurve: MapToCurve<PC_F<PC>>, defaults: TArg<H2COpts & {
    encodeDST?: AsciiOrBytes;
}>): H2CHasher<PC>;
/**
 * Implementation of the Shallue and van de Woestijne method for any weierstrass curve.
 * TODO: check if there is a way to merge this with uvRatio in Edwards; move to modular.
 * b = True and y = sqrt(u / v) if (u / v) is square in F, and
 * b = False and y = sqrt(Z * (u / v)) otherwise.
 * RFC 9380 expects callers to provide `v != 0`; this helper does not enforce it.
 * @param Fp - Field implementation.
 * @param Z - Simplified SWU map parameter.
 * @returns Square-root ratio helper.
 * @example
 * Build the square-root ratio helper used by SWU map implementations.
 *
 * ```ts
 * import { SWUFpSqrtRatio } from '@noble/curves/abstract/hash-to-curve.js';
 * import { Field } from '@noble/curves/abstract/modular.js';
 * const Fp = Field(17n);
 * const sqrtRatio = SWUFpSqrtRatio(Fp, 3n);
 * const out = sqrtRatio(4n, 1n);
 * ```
 */
export declare function SWUFpSqrtRatio<T>(Fp: TArg<IField<T>>, Z: T): (u: T, v: T) => {
    isValid: boolean;
    value: T;
};
/**
 * Simplified Shallue-van de Woestijne-Ulas Method
 * See {@link https://www.rfc-editor.org/rfc/rfc9380#section-6.6.2 | RFC 9380 section 6.6.2}.
 * @param Fp - Field implementation.
 * @param opts - SWU parameters:
 *   - `A`: Curve parameter `A`.
 *   - `B`: Curve parameter `B`.
 *   - `Z`: Simplified SWU map parameter.
 * @returns Deterministic map-to-curve function.
 * @throws If the SWU parameters are invalid or the field lacks the required helpers. {@link Error}
 * @example
 * Map one field element to a Weierstrass curve point with the SWU recipe.
 *
 * ```ts
 * import { mapToCurveSimpleSWU } from '@noble/curves/abstract/hash-to-curve.js';
 * import { Field } from '@noble/curves/abstract/modular.js';
 * const Fp = Field(17n);
 * const map = mapToCurveSimpleSWU(Fp, { A: 1n, B: 2n, Z: 3n });
 * const point = map(5n);
 * ```
 */
export declare function mapToCurveSimpleSWU<T>(Fp: TArg<IField<T>>, opts: {
    A: T;
    B: T;
    Z: T;
}): (u: T) => {
    x: T;
    y: T;
};
export {};
