/**
 * Methods for elliptic curve multiplication by scalars.
 * Contains wNAF-based ScalarMultiplier, pippenger.
 * @module
 */
/*! noble-curves - MIT License (c) 2022 Paul Miller (paulmillr.com) */
import { type Signer, type TArg, type TRet } from '../utils.ts';
import { type IField } from './modular.ts';
/** Affine point coordinates without projective fields. */
export type AffinePoint<T> = {
    /** Affine x coordinate. */
    x: T;
    /** Affine y coordinate. */
    y: T;
} & {
    Z?: never;
};
/** Base interface for all elliptic-curve point instances. */
export interface CurvePoint<F, P extends CurvePoint<F, P>> {
    /** Affine x coordinate. Different from projective / extended X coordinate. */
    x: F;
    /** Affine y coordinate. Different from projective / extended Y coordinate. */
    y: F;
    /** Projective Z coordinate when the point keeps projective state. */
    Z?: F;
    /**
     * Double the point.
     * @returns Doubled point.
     */
    double(): P;
    /**
     * Negate the point.
     * @returns Negated point.
     */
    negate(): P;
    /**
     * Add another point from the same curve.
     * @param other - Point to add.
     * @returns Sum point.
     */
    add(other: P): P;
    /**
     * Subtract another point from the same curve.
     * @param other - Point to subtract.
     * @returns Difference point.
     */
    subtract(other: P): P;
    /**
     * Compare two points for equality.
     * @param other - Point to compare.
     * @returns Whether the points are equal.
     */
    equals(other: P): boolean;
    /**
     * Multiply the point by a scalar in constant time.
     * Implementations keep the subgroup-scalar contract strict and may reject
     * `0` instead of returning the identity point.
     * @param scalar - Scalar multiplier.
     * @returns Product point.
     */
    multiply(scalar: bigint): P;
    /** Assert that the point satisfies the curve equation and subgroup checks. */
    assertValidity(): void;
    /**
     * Map the point into the prime-order subgroup when the curve requires it.
     * @returns Prime-order point.
     */
    clearCofactor(): P;
    /**
     * Check whether the point is the point at infinity.
     * @returns Whether the point is zero.
     */
    is0(): boolean;
    /**
     * Check whether the point belongs to the prime-order subgroup.
     * @returns Whether the point is torsion-free.
     */
    isTorsionFree(): boolean;
    /**
     * Check whether the point lies in a small torsion subgroup.
     * @returns Whether the point has small order.
     */
    isSmallOrder(): boolean;
    /**
     * Multiply the point by a scalar without constant-time guarantees.
     * Public-scalar callers that need `0` should use this method instead of
     * relying on `multiply(...)` to return the identity point.
     * @param scalar - Scalar multiplier.
     * @returns Product point.
     */
    multiplyUnsafe(scalar: bigint): P;
    /**
     * Massively speeds up `p.multiply(n)` by using precompute tables (caching).
     * See {@link ScalarMultiplier}.
     * Cache state lives in internal WeakMaps keyed by point identity, not on the point object.
     * Repeating `precompute(...)` for the same point identity replaces the remembered window size
     * and forces table regeneration for that point.
     * @param windowSize - Precompute window size.
     * @param isLazy - calculate cache now. Default (true) ensures it's deferred to first `multiply()`
     * @returns Same point instance with precompute tables attached.
     */
    precompute(windowSize?: number, isLazy?: boolean): P;
    /**
     * Converts point to 2D xy affine coordinates.
     * @param invertedZ - Optional inverted Z coordinate for batch normalization.
     * @returns Affine x/y coordinates.
     */
    toAffine(invertedZ?: F): AffinePoint<F>;
    /**
     * Encode the point into the curve's canonical byte form.
     * @returns Encoded point bytes.
     */
    toBytes(): Uint8Array;
    /**
     * Encode the point into the curve's canonical hex form.
     * @returns Encoded point hex.
     */
    toHex(): string;
}
/** Base interface for elliptic-curve point constructors. */
export interface CurvePointCons<P extends CurvePoint<any, P>> {
    /**
     * Runtime brand check for points created by this constructor.
     * @param item - Value to test.
     * @returns Whether the value is a point from this constructor.
     */
    [Symbol.hasInstance]: (item: unknown) => boolean;
    /** Canonical subgroup generator. */
    BASE: P;
    /** Point at infinity. */
    ZERO: P;
    /** Field for basic curve math */
    Fp: IField<P_F<P>>;
    /** Scalar field, for scalars in multiply and others */
    Fn: IField<bigint>;
    /**
     * Create one point from affine coordinates.
     * Does NOT validate curve, subgroup, or wrapper invariants.
     * Use `.assertValidity()` on adversarial inputs.
     * @param p - Affine point coordinates.
     * @returns Point instance.
     */
    fromAffine(p: AffinePoint<P_F<P>>): P;
    /**
     * Decode a point from the canonical byte encoding.
     * @param bytes - Encoded point bytes.
     * Implementations MUST treat `bytes` as read-only.
     * @returns Point instance.
     */
    fromBytes(bytes: Uint8Array): P;
    /**
     * Decode a point from the canonical hex encoding.
     * @param hex - Encoded point hex.
     * @returns Point instance.
     */
    fromHex(hex: string): P;
}
/** Returns the affine field type for a point instance (`P_F<P> == P.F`). */
export type P_F<P extends CurvePoint<any, P>> = P extends CurvePoint<infer F, P> ? F : never;
/** Returns the affine field type for a point constructor (`PC_F<PC> == PC.P.F`). */
export type PC_F<PC extends CurvePointCons<CurvePoint<any, any>>> = PC['Fp']['ZERO'];
/** Returns the point instance type for a point constructor (`PC_P<PC> == PC.P`). */
export type PC_P<PC extends CurvePointCons<CurvePoint<any, any>>> = PC['ZERO'];
/** Wide point-constructor type used when the concrete curve is not important. */
export type PC_ANY = CurvePointCons<CurvePoint<any, CurvePoint<any, CurvePoint<any, CurvePoint<any, CurvePoint<any, CurvePoint<any, CurvePoint<any, CurvePoint<any, CurvePoint<any, CurvePoint<any, any>>>>>>>>>>>;
/**
 * Validates the static surface of a point constructor.
 * This is only a cheap sanity check for the constructor hooks and fields consumed by generic
 * factories; it does not certify `BASE`/`ZERO` semantics or prove the curve implementation itself.
 * @param Point - Runtime point constructor.
 * @throws On missing constructor hooks or malformed field metadata. {@link TypeError}
 * @example
 * Check that one point constructor exposes the static hooks generic helpers need.
 *
 * ```ts
 * import { ed25519 } from '@noble/curves/ed25519.js';
 * import { validatePointCons } from '@noble/curves/abstract/curve.js';
 * validatePointCons(ed25519.Point);
 * ```
 */
export declare function validatePointCons<P extends CurvePoint<any, P>>(Point: CurvePointCons<P>): void;
/** Byte lengths used by one curve implementation. */
export interface CurveLengths {
    /** Secret-key length in bytes. */
    secretKey?: number;
    /** Compressed public-key length in bytes. */
    publicKey?: number;
    /** Uncompressed public-key length in bytes. */
    publicKeyUncompressed?: number;
    /** Whether public-key encodings include a format prefix byte. */
    publicKeyHasPrefix?: boolean;
    /** Signature length in bytes. */
    signature?: number;
    /** Seed length in bytes when the curve exposes deterministic keygen from seed. */
    seed?: number;
}
/** Reorders or otherwise remaps a batch while preserving its element type. */
export type Mapper<T> = (i: T[]) => T[];
/**
 * Takes a bunch of Projective Points but executes only one
 * inversion on all of them. Inversion is very slow operation,
 * so this improves performance massively.
 * Optimization: converts a list of projective points to a list of identical points with Z=1.
 * Input points are left unchanged; the normalized points are returned as fresh instances.
 * @param c - Point constructor.
 * @param points - Projective points.
 * @returns Fresh projective points reconstructed from normalized affine coordinates.
 * @example
 * Batch-normalize projective points with a single shared inversion.
 *
 * ```ts
 * import { normalizeZ } from '@noble/curves/abstract/curve.js';
 * import { p256 } from '@noble/curves/nist.js';
 * const points = normalizeZ(p256.Point, [p256.Point.BASE, p256.Point.BASE.double()]);
 * ```
 */
export declare function normalizeZ<P extends CurvePoint<any, P>, PC extends CurvePointCons<P>>(c: PC, points: P[]): P[];
/** RNG interface used for scalar / nonce blinding. */
export type RandomBytes = (bytesLength?: number) => TRet<Uint8Array>;
/**
 * Probes an RNG once, at construction time: returns `undefined` when it is unavailable —
 * throws or returns malformed bytes — so callers can downgrade to their unblinded /
 * deterministic constant-time fallback. Blinding is defense-in-depth (DPA/template
 * hardening), not a correctness or key-secrecy requirement, so availability-based
 * downgrade is acceptable.
 *
 * The downgrade decision is deliberately static. After a successful probe the RNG becomes
 * part of the trusted contract: later misbehavior must fail closed in per-call validation
 * (throw), never downgrade — a dynamic fallback would let a tampered RNG silently strip
 * blinding on demand. A probe can only ever classify broken environments, not adversarial
 * RNGs: a stateful RNG can always behave while probed and misbehave later.
 * @param randomBytes - RNG to probe, or `undefined` when the environment provides none.
 * @param length - Byte length requested from the probe call.
 * @returns The RNG when the probe produced `length` valid bytes; `undefined` otherwise.
 * @example
 * Probe an RNG once before enabling scalar blinding.
 *
 * ```ts
 * import { probeRandomBytes } from '@noble/curves/abstract/curve.js';
 * import { randomBytes } from '@noble/hashes/utils.js';
 * const rng = probeRandomBytes(randomBytes, 16);
 * ```
 */
export declare function probeRandomBytes(randomBytes: TArg<RandomBytes | undefined>, length: number): TRet<RandomBytes | undefined>;
/** Result of a constant-time multiply: real point `p`, fake accumulator `f` (discarded). */
type MulResult<P> = {
    p: P;
    f: P;
};
/**
 * Elliptic curve multiplication of Point by scalar.
 * Routes between cached-table, fixed-window, and one-shot wNAF paths; entry points validate
 * their own scalars (`mulCT`/`mulCTBlinded`: `1 <= s < Fn.ORDER`; `mulUnsafe`: up to the
 * `Fn.ORDER^4` DoS cap via {@link mulAddUnsafe}).
 * Table generation is expensive and happens on first call of `multiply()`
 * (or eagerly via `precompute(W, false)`). By default, `BASE` point is precomputed.
 *
 * Cached algorithm is signed fixed-window wNAF:
 * - table stores, for every window w, the multiples `[1..2^(W−1)]⋅2^(w⋅W)⋅P` — all doublings
 *   are baked in, so a multiplication is exactly one table addition per window
 * - window count is fixed (`ceil(bits/W) + 1`), so the point-operation count is scalar-independent
 *   (basis of the constant-time path)
 * - for a 256-bit curve and W=6: 44⋅32 = 1408 table points, 44 additions per multiply
 * - secret scalars are additionally blinded (see {@link ScalarMultiplier.mulCTBlinded}), which
 *   widens tables by 128 bits
 * @param Point - Point constructor.
 * @param randomBytes - RNG used for scalar blinding; required by the blinded secret path.
 * @example
 * Elliptic curve multiplication of Point by scalar.
 *
 * ```ts
 * import { ScalarMultiplier } from '@noble/curves/abstract/curve.js';
 * import { p256 } from '@noble/curves/nist.js';
 * const mul = new ScalarMultiplier(p256.Point);
 * ```
 */
export declare class ScalarMultiplier<PC extends PC_ANY> {
    private readonly Point;
    private readonly BASE;
    private readonly ZERO;
    private readonly randomBytes?;
    private readonly wnafPrecomputes;
    private baseCanBeBlinded;
    readonly bits: number;
    constructor(Point: PC, randomBytes?: RandomBytes);
    /**
     * Creates a signed fixed-window wNAF precomputation table: for every window w, the
     * multiples `[1..2^(W−1)]⋅2^(w⋅W)⋅P`, flattened. All doublings are baked into the table,
     * so cached multiplication is additions-only. `windows = ceil(bits/W) + 1`: the extra
     * window absorbs the final carry of signed-digit recoding.
     * For a 256-bit curve and W=6, the table is 44⋅32 = 1408 points.
     * @param point - Point instance
     * @param W - window size
     * @param bits - scalar bitlength the table must cover
     */
    private buildWnafTable;
    /**
     * Implements ec multiplication using precomputed signed fixed-window wNAF tables.
     * Constant-time: fixed window count with one table addition per window — zero digits feed
     * the fake accumulator — and no doublings; the lookup scans the whole window slice.
     * Scalar bounds are validated by the public entry points ({@link ScalarMultiplier.mulCT},
     * {@link ScalarMultiplier.mulCTBlinded}, {@link ScalarMultiplier.mulUnsafe});
     * signedWindowDigits throws if `n` exceeds the table.
     * @returns real and fake (for const-time) points
     */
    private wnafCachedCT;
    private getWnafPrecomputes;
    private assertPoint;
    private validateMulInput;
    private runCT;
    mulCT(point: PC_P<PC>, scalar: bigint, transform?: Mapper<PC_P<PC>>): MulResult<PC_P<PC>>;
    mulCTBlinded(point: PC_P<PC>, scalar: bigint, transform?: Mapper<PC_P<PC>>): MulResult<PC_P<PC>>;
    /**
     * Constant-time multiplication `n*point` for an un-precomputed point, via a small fixed window.
     * A cached wNAF table only pays off when reused; a flat 2^FW_WINDOW table (`size-1` adds) is
     * far cheaper to build for a single use. The point-operation sequence is independent of `n`:
     * build the table, then per window exactly FW_WINDOW doublings, a data-oblivious scan over
     * every table entry, and one addition (adds the identity when the window digit is 0 — never
     * skipped).
     *
     * `n` must be `< 2^bits`. Assumes complete addition (adding the identity costs the same as any
     * add), which holds for the Weierstrass/Edwards point types used here. The table is left in
     * projective form (no normalizeZ): normalizing this small a table costs more than the
     * mixed-add savings it would buy for a single multiply.
     * @returns real point `p`; `f` duplicates it only to match {@link wnafCachedCT}'s return shape
     * (this path needs no fake accumulator — its op-count is already scalar-independent).
     */
    private fixedWindowCT;
    private shouldBlind;
    mulSecret(point: PC_P<PC>, scalar: bigint, cofactor: bigint, transform?: Mapper<PC_P<PC>>): MulResult<PC_P<PC>>;
    mulUnsafe(point: PC_P<PC>, scalar: bigint, transform?: Mapper<PC_P<PC>>): PC_P<PC>;
    setWindowSize(point: PC_P<PC>, W: number): void;
    hasWindowSize(point: PC_P<PC>): boolean;
}
/**
 * Combined multi-scalar multiplication `Σ scalars[i]⋅points[i]` via interleaved width-4 wNAF
 * (Strauss–Shamir). Every input gets its own table of odd multiples `[1P, 3P, 5P, 7P]` and
 * signed-digit recoding, but all walks share one doubling chain, so total cost is
 * `~bits` doublings + `L⋅bits/5` additions instead of `L⋅bits` doublings for separate
 * multiplications. Intended for the 2-4 point shapes of signature verification
 * (`R = u1⋅G + u2⋅P`); use {@link pippenger} for larger batches.
 *
 * Not constant-time: only for public inputs. Scalars must satisfy `0 <= s < Fn.ORDER`;
 * fold negative signs into the points before calling.
 * @param c - Point constructor.
 * @param points - Array of curve points.
 * @param scalars - Array of non-negative scalars, same length as points.
 * @param allowOversized - Replace the `s < Fn.ORDER` scalar check with a `Fn.ORDER^4` DoS cap.
 *   Off by default. For scalars that must NOT be reduced mod ORDER: torsion checks
 *   (`Fn.ORDER⋅P ≟ O`) and cofactor-clearing multiples. Walk length grows with `bitLen(s)`.
 * @returns Combined multiplication result; identity for empty input.
 * @throws If the point set or scalar set is invalid. {@link Error}
 * @example
 * Combined multi-scalar multiplication via Strauss–Shamir.
 *
 * ```ts
 * import { mulAddUnsafe } from '@noble/curves/abstract/curve.js';
 * import { p256 } from '@noble/curves/nist.js';
 * const G = p256.Point.BASE;
 * const R = mulAddUnsafe(p256.Point, [G, G.double()], [2n, 3n]); // 2⋅G + 3⋅(2⋅G)
 * ```
 */
export declare function mulAddUnsafe<P extends CurvePoint<any, P>, PC extends CurvePointCons<P>>(c: PC, points: P[], scalars: bigint[], allowOversized?: boolean): P;
/**
 * Pippenger algorithm for multi-scalar multiplication (MSM, Pa + Qb + Rc + ...).
 * 30x faster vs naive addition on L=4096, 10x faster than precomputes.
 * For N=254bit, L=1, it does: 1024 ADD + 254 DBL. For L=5: 1536 ADD + 254 DBL.
 * Point-operation count is scalar-independent (for same L), even when 1 point + scalar, or when
 * scalar = 0 — but bucket indices are scalar windows, so the memory-access pattern is
 * scalar-dependent: do not rely on this for secret scalars.
 *
 * A repaired LFG bucket-set variant from ePrint 2024/750 was benchmarked on BLS12-381 G1
 * against this implementation: ~1.4x faster at 2048 points and ~1.1-1.25x faster at
 * 4096-32768 points, at the cost of extra recoding and multiplier-table complexity.
 * @param c - Curve Point constructor
 * @param points - array of L curve points
 * @param scalars - array of L scalars (aka secret keys / bigints)
 * @returns MSM result point. Empty input is accepted and returns the identity.
 * @throws If the point set, scalar set, or MSM sizing is invalid. {@link Error}
 * @example
 * Pippenger algorithm for multi-scalar multiplication (MSM, Pa + Qb + Rc + ...).
 *
 * ```ts
 * import { pippenger } from '@noble/curves/abstract/curve.js';
 * import { p256 } from '@noble/curves/nist.js';
 * const point = pippenger(p256.Point, [p256.Point.BASE, p256.Point.BASE.double()], [2n, 3n]);
 * ```
 */
export declare function pippenger<P extends CurvePoint<any, P>, PC extends CurvePointCons<P>>(c: PC, points: P[], scalars: bigint[]): P;
/**
 * Interleaved wNAF multi-scalar multiplication (MSM, Pa + Qb + Rc + ...) over a FIXED set
 * of points: each point gets a one-time table of odd multiples
 * `[1P, 3P, ..., (2^(W−1)−1)P]`, and the returned closure evaluates MSMs against those
 * tables. All scalars share one doubling chain (Straus 1964) — one doubling per scalar bit
 * plus one signed table addition per nonzero width-W wNAF digit (density ~1/(W+1)) — the
 * "interleaving" method of Möller, "Algorithms for multi-exponentiation" (SAC 2001).
 *
 * Table memory is `L⋅2^(W−2)` points, capped at ~2 GiB. Prefer this over {@link pippenger}
 * when the same points are reused across many MSMs (fixed-base commitments etc.) and up to a
 * few hundred points; prefer pippenger for one-shot MSMs or thousands of points, where
 * bucketing beats per-point tables.
 *
 * Not constant-time (zero digits are skipped): public inputs only.
 * @param c - Curve Point constructor
 * @param points - array of L curve points, captured by the returned closure
 * @param windowSize - window width W in bits, from 2 to Fn.BITS; also capped so the
 *   per-closure tables stay under ~2 GiB
 * @returns Function which multiplies points with scalars. The closure accepts at most
 *   `points.length` scalars, and omitted trailing scalars are treated as zero.
 * @throws If the point set or precompute window is invalid. {@link Error}
 * @example
 * Interleaved wNAF multi-scalar multiplication (MSM, Pa + Qb + Rc + ...).
 *
 * ```ts
 * import { interleavedMSMUnsafe } from '@noble/curves/abstract/curve.js';
 * import { p256 } from '@noble/curves/nist.js';
 * const msm = interleavedMSMUnsafe(p256.Point, [p256.Point.BASE], 4);
 * const point = msm([3n]);
 * ```
 */
export declare function interleavedMSMUnsafe<P extends CurvePoint<any, P>, PC extends CurvePointCons<P>>(c: PC, points: P[], windowSize: number): (scalars: bigint[]) => P;
/** Minimal curve parameters needed to construct a Weierstrass or Edwards curve. */
export type ValidCurveParams<T> = {
    /** Base-field modulus. */
    p: bigint;
    /** Prime subgroup order. */
    n: bigint;
    /** Cofactor. */
    h: bigint;
    /** Curve parameter `a`. */
    a: T;
    /** Weierstrass curve parameter `b`. */
    b?: T;
    /** Edwards curve parameter `d`. */
    d?: T;
    /** Generator x coordinate. */
    Gx: T;
    /** Generator y coordinate. */
    Gy: T;
};
/** Pair of fields used by curve constructors. */
export type FpFn<T> = {
    /** Base field used for curve coordinates. */
    Fp: IField<T>;
    /** Scalar field used for secret scalars and subgroup arithmetic. */
    Fn: IField<bigint>;
};
/**
 * Validates basic CURVE shape and field membership, then creates fields.
 * This does not prove that the generator is on-curve, that subgroup/order data are consistent, or
 * that the curve equation itself is otherwise sane.
 * @param type - Curve family.
 * @param CURVE - Curve parameters.
 * @param curveOpts - Optional field overrides. See {@link FpFn}:
 *   - `Fp` (optional): Optional base-field override.
 *   - `Fn` (optional): Optional scalar-field override.
 * @param FpFnLE - Whether field encoding is little-endian.
 * @returns Frozen curve parameters and fields.
 * @throws If the curve parameters or field overrides are invalid. {@link Error}
 * @example
 * Build curve fields from raw constants before constructing a curve instance.
 *
 * ```ts
 * const curve = createCurveFields('weierstrass', {
 *   p: 17n,
 *   n: 19n,
 *   h: 1n,
 *   a: 2n,
 *   b: 2n,
 *   Gx: 5n,
 *   Gy: 1n,
 * });
 * ```
 */
export declare function createCurveFields<T>(type: 'weierstrass' | 'edwards', CURVE: ValidCurveParams<T>, curveOpts?: TArg<Partial<FpFn<T>>>, FpFnLE?: boolean): TRet<FpFn<T> & {
    CURVE: ValidCurveParams<T>;
}>;
type KeygenFn = (seed?: Uint8Array) => {
    secretKey: Uint8Array;
    publicKey: Uint8Array;
};
/**
 * @param randomSecretKey - Secret-key generator.
 * @param getPublicKey - Public-key derivation helper.
 * @returns Keypair generator.
 * @example
 * Build a `keygen()` helper from existing secret-key and public-key primitives.
 *
 * ```ts
 * import { createKeygen } from '@noble/curves/abstract/curve.js';
 * import { p256 } from '@noble/curves/nist.js';
 * const keygen = createKeygen(p256.utils.randomSecretKey, p256.getPublicKey);
 * const pair = keygen();
 * ```
 */
export declare function createKeygen(randomSecretKey: Function, getPublicKey: TArg<Signer['getPublicKey']>): TRet<KeygenFn>;
export {};
