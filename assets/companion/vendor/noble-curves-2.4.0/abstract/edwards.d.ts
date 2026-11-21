/**
 * Twisted Edwards curve. The formula is: ax² + y² = 1 + dx²y².
 * For design rationale of types / exports, see weierstrass module documentation.
 * Untwisted Edwards curves exist, but they aren't used in real-world protocols.
 * @module
 */
/*! noble-curves - MIT License (c) 2022 Paul Miller (paulmillr.com) */
import { type FHash, type TArg, type TRet } from '../utils.ts';
import { type AffinePoint, type CurveLengths, type CurvePoint, type CurvePointCons } from './curve.ts';
import { type IField } from './modular.ts';
/** Extended Edwards point with X/Y/Z/T coordinates. */
export interface EdwardsPoint extends CurvePoint<bigint, EdwardsPoint> {
    /** extended X coordinate. Different from affine x. */
    readonly X: bigint;
    /** extended Y coordinate. Different from affine y. */
    readonly Y: bigint;
    /** extended Z coordinate */
    readonly Z: bigint;
    /** extended T coordinate */
    readonly T: bigint;
}
/** Constructor and decoding helpers for extended Edwards points. */
export interface EdwardsPointCons extends CurvePointCons<EdwardsPoint> {
    /** Create a point from extended X/Y/Z/T coordinates without validation. */
    new (X: bigint, Y: bigint, Z: bigint, T: bigint): EdwardsPoint;
    /**
     * Return the curve parameters used by this point constructor.
     * @returns Curve parameters.
     */
    CURVE(): EdwardsOpts;
    /**
     * Decode a point from bytes, optionally using ZIP-215 rules.
     * @param bytes - Encoded point bytes.
     * @param zip215 - Whether to accept ZIP-215 encodings.
     * @returns Decoded Edwards point.
     */
    fromBytes(bytes: Uint8Array, zip215?: boolean): EdwardsPoint;
    /**
     * Decode a point from hex, optionally using ZIP-215 rules.
     * @param hex - Encoded point hex.
     * @param zip215 - Whether to accept ZIP-215 encodings.
     * @returns Decoded Edwards point.
     */
    fromHex(hex: string, zip215?: boolean): EdwardsPoint;
}
/**
 * Twisted Edwards curve options.
 *
 * * a: formula param
 * * d: formula param
 * * p: prime characteristic (order) of finite field, in which arithmetics is done
 * * n: order of prime subgroup a.k.a total amount of valid curve points
 * * h: cofactor. h*n is group order; n is subgroup order
 * * Gx: x coordinate of generator point a.k.a. base point
 * * Gy: y coordinate of generator point
 */
export type EdwardsOpts = Readonly<{
    /** Base-field modulus. */
    p: bigint;
    /** Prime subgroup order. */
    n: bigint;
    /** Curve cofactor. */
    h: bigint;
    /** Edwards curve parameter `a`. */
    a: bigint;
    /** Edwards curve parameter `d`. */
    d: bigint;
    /** Generator x coordinate. */
    Gx: bigint;
    /** Generator y coordinate. */
    Gy: bigint;
}>;
/**
 * Extra curve options for Twisted Edwards.
 *
 * * Fp: redefined Field over curve.p
 * * Fn: redefined Field over curve.n
 * * uvRatio: helper function for decompression, calculating √(u/v)
 */
export type EdwardsExtraOpts = Partial<{
    /** Optional base-field override. */
    Fp: IField<bigint>;
    /** Optional scalar-field override. */
    Fn: IField<bigint>;
    /** Whether field encodings are little-endian. */
    FpFnLE: boolean;
    /** Square-root ratio helper used during point decompression. */
    uvRatio: (u: bigint, v: bigint) => {
        isValid: boolean;
        value: bigint;
    };
    /** RNG override used for scalar blinding. */
    randomBytes: (bytesLength?: number) => TRet<Uint8Array>;
}>;
/**
 * EdDSA (Edwards Digital Signature algorithm) options.
 *
 * * hash: hash function used to hash secret keys and messages
 * * adjustScalarBytes: clears bits to get valid field element
 * * domain: Used for hashing
 * * mapToCurve: for hash-to-curve standard
 * * prehash: RFC 8032 pre-hashing of messages to sign() / verify()
 * * randomBytes: function generating random bytes, used for randomSecretKey
 */
export type EdDSAOpts = Partial<{
    /** Clamp or otherwise normalize secret-scalar bytes before reducing mod `n`. */
    adjustScalarBytes: (bytes: TArg<Uint8Array>) => TRet<Uint8Array>;
    /** Domain-separation helper for contexts and prehash mode. */
    domain: (data: TArg<Uint8Array>, ctx: TArg<Uint8Array>, phflag: boolean) => TRet<Uint8Array>;
    /** Optional hash-to-curve mapper for protocols like Ristretto hash-to-group. */
    mapToCurve: (scalar: bigint[]) => AffinePoint<bigint>;
    /** Optional conversion from this Edwards curve to a birational/isogenous Montgomery curve. */
    toMontgomery: (point: EdwardsPoint) => TRet<Uint8Array>;
    /** Optional secret-key conversion for the same Montgomery curve as `toMontgomery`. */
    toMontgomerySecret: (secretKey: TArg<Uint8Array>) => TRet<Uint8Array>;
    /** Optional prehash function used before signing or verifying messages. */
    prehash: FHash;
    /** Default verification decoding policy. ZIP-215 is more permissive than RFC 8032 / NIST. */
    zip215: boolean;
    /** RNG override used by helper constructors. */
    randomBytes: (bytesLength?: number) => TRet<Uint8Array>;
}>;
/**
 * EdDSA (Edwards Digital Signature algorithm) helper namespace.
 * Allows creating and verifying signatures, and deriving public keys.
 */
export interface EdDSA {
    /**
     * Generate a secret/public key pair.
     * @param seed - Optional seed material.
     * @returns Secret/public key pair.
     */
    keygen: (seed?: TArg<Uint8Array>) => {
        secretKey: TRet<Uint8Array>;
        publicKey: TRet<Uint8Array>;
    };
    /**
     * Derive the public key from a secret key.
     * @param secretKey - Secret key bytes.
     * @returns Encoded public key.
     */
    getPublicKey: (secretKey: TArg<Uint8Array>) => TRet<Uint8Array>;
    /**
     * Sign a message with an EdDSA secret key.
     * @param message - Message bytes.
     * @param secretKey - Secret key bytes.
     * @param options - Optional signature tweaks:
     *   - `context` (optional): Domain-separation context for Ed25519ctx/Ed448.
     * @returns Encoded signature bytes.
     */
    sign: (message: TArg<Uint8Array>, secretKey: TArg<Uint8Array>, options?: TArg<{
        context?: Uint8Array;
    }>) => TRet<Uint8Array>;
    /**
     * Verify a signature against a message and public key.
     * @param sig - Encoded signature bytes.
     * @param message - Message bytes.
     * @param publicKey - Encoded public key.
     * @param options - Optional verification tweaks:
     *   - `context` (optional): Domain-separation context for Ed25519ctx/Ed448.
     *   - `zip215` (optional): Whether to accept ZIP-215 encodings.
     * @throws Malformed argument or option types may throw; `false` means well-formed inputs
     *   failed verification. {@link Error}
     * @returns Whether the signature is valid.
     */
    verify: (sig: TArg<Uint8Array>, message: TArg<Uint8Array>, publicKey: TArg<Uint8Array>, options?: TArg<{
        context?: Uint8Array;
        zip215?: boolean;
    }>) => boolean;
    /** Point constructor used by this signature scheme. */
    Point: EdwardsPointCons;
    /** Helper utilities for key validation and Montgomery conversion. */
    utils: {
        /**
         * Generate a valid random secret key.
         * Optional seed bytes are only length-checked and returned unchanged.
         */
        randomSecretKey: (seed?: TArg<Uint8Array>) => TRet<Uint8Array>;
        /** Check whether a secret key has the expected encoding. */
        isValidSecretKey: (secretKey: TArg<Uint8Array>) => boolean;
        /** Check whether a public key decodes to a valid point. */
        isValidPublicKey: (publicKey: TArg<Uint8Array>, zip215?: boolean) => boolean;
        /**
         * Converts ed public key to x public key.
         * Throws when the Edwards curve has no supported Montgomery conversion.
         *
         * There is NO `fromMontgomery`:
         * - There are 2 valid ed25519 points for every x25519, with flipped coordinate
         * - Sometimes there are 0 valid ed25519 points, because x25519 *additionally*
         *   accepts inputs on the quadratic twist, which can't be moved to ed25519
         *
         * @example
         * Converts ed public key to x public key.
         *
         * ```js
         * const someonesPub_ed = ed25519.getPublicKey(ed25519.utils.randomSecretKey());
         * const someonesPub = ed25519.utils.toMontgomery(someonesPub);
         * const aPriv = x25519.utils.randomSecretKey();
         * const shared = x25519.getSharedSecret(aPriv, someonesPub)
         * ```
         */
        toMontgomery: (publicKey: TArg<Uint8Array>) => TRet<Uint8Array>;
        /**
         * Converts ed secret key to x secret key.
         * Throws when the Edwards curve has no supported Montgomery conversion.
         * @example
         * Converts ed secret key to x secret key.
         *
         * ```js
         * const someonesPub = x25519.getPublicKey(x25519.utils.randomSecretKey());
         * const aPriv_ed = ed25519.utils.randomSecretKey();
         * const aPriv = ed25519.utils.toMontgomerySecret(aPriv_ed);
         * const shared = x25519.getSharedSecret(aPriv, someonesPub)
         * ```
         */
        toMontgomerySecret: (secretKey: TArg<Uint8Array>) => TRet<Uint8Array>;
        /** Return the expanded private key components used by RFC8032 signing. */
        getExtendedPublicKey: (key: TArg<Uint8Array>) => {
            head: TRet<Uint8Array>;
            prefix: TRet<Uint8Array>;
            scalar: bigint;
            point: EdwardsPoint;
            pointBytes: TRet<Uint8Array>;
        };
    };
    /** Byte lengths for keys and signatures exposed by this scheme. */
    lengths: CurveLengths;
}
/**
 * @param params - Curve parameters. See {@link EdwardsOpts}.
 * @param extraOpts - Optional helpers and overrides. See {@link EdwardsExtraOpts}.
 * @returns Edwards point constructor. Generator validation here only checks
 *   that `(Gx, Gy)` satisfies the affine Edwards equation.
 *   RFC 8032 base-point constraints like `B != (0,1)` and `[L]B = 0`
 *   are left to the caller's chosen parameters, since eager subgroup
 *   validation here adds about 10-15ms to heavyweight imports like ed448.
 *   The returned constructor also eagerly marks `Point.BASE` for W=6
 *   precompute caching. Some code paths still assume
 *   `Fp.BYTES === Fn.BYTES`, so mismatched byte lengths are not fully audited here.
 * @throws If the curve parameters or Edwards overrides are invalid. {@link Error}
 * @example
 * ```ts
 * import { edwards } from '@noble/curves/abstract/edwards.js';
 * import { jubjub } from '@noble/curves/misc.js';
 * // Build a point constructor from explicit curve parameters, then use its base point.
 * const Point = edwards(jubjub.Point.CURVE());
 * Point.BASE.toHex();
 * ```
 */
export declare function edwards(params: TArg<EdwardsOpts>, extraOpts?: TArg<EdwardsExtraOpts>): EdwardsPointCons;
/**
 * Base class for prime-order points like Ristretto255 and Decaf448.
 * These points eliminate cofactor issues by representing equivalence classes
 * of Edwards curve points. Multiple Edwards representatives can describe the
 * same abstract wrapper element, so wrapper validity is not the same thing as
 * the hidden representative being torsion-free.
 * @param ep - Backing Edwards point.
 * @example
 * Base class for prime-order points like Ristretto255 and Decaf448.
 *
 * ```ts
 * import { ristretto255 } from '@noble/curves/ed25519.js';
 * const point = ristretto255.Point.BASE.multiply(2n);
 * ```
 */
export declare abstract class PrimeEdwardsPoint<T extends PrimeEdwardsPoint<T>> implements CurvePoint<bigint, T> {
    static BASE: PrimeEdwardsPoint<any>;
    static ZERO: PrimeEdwardsPoint<any>;
    static Fp: IField<bigint>;
    static Fn: IField<bigint>;
    protected readonly ep: EdwardsPoint;
    /**
     * Wrap one internal Edwards representative directly.
     * This is not a canonical encoding boundary: alternate Edwards
     * representatives may still describe the same abstract wrapper element.
     */
    constructor(ep: EdwardsPoint);
    abstract toBytes(): Uint8Array;
    abstract equals(other: T): boolean;
    static fromBytes(_bytes: Uint8Array): any;
    static fromHex(_hex: string): any;
    get x(): bigint;
    get y(): bigint;
    clearCofactor(): T;
    assertValidity(): void;
    /**
     * Return affine coordinates of the current internal Edwards representative.
     * This is a convenience helper, not a canonical Ristretto/Decaf encoding.
     * Equal abstract elements may expose different `x` / `y`; use
     * `toBytes()` / `fromBytes()` for canonical roundtrips.
     */
    toAffine(invertedZ?: bigint): AffinePoint<bigint>;
    toHex(): string;
    toString(): string;
    isTorsionFree(): boolean;
    isSmallOrder(): boolean;
    add(other: T): T;
    subtract(other: T): T;
    multiply(scalar: bigint): T;
    multiplyUnsafe(scalar: bigint): T;
    double(): T;
    negate(): T;
    precompute(windowSize?: number, isLazy?: boolean): T;
    abstract is0(): boolean;
    protected abstract assertSame(other: T): void;
    protected abstract init(ep: EdwardsPoint): T;
}
/**
 * Initializes EdDSA signatures over given Edwards curve.
 * @param Point - Edwards point constructor.
 * @param cHash - Hash function.
 * @param eddsaOpts - Optional signature helpers. See {@link EdDSAOpts}.
 * @returns EdDSA helper namespace.
 * @throws If the hash function, options, or derived point operations are invalid. {@link Error}
 * @example
 * Initializes EdDSA signatures over given Edwards curve.
 *
 * ```ts
 * import { eddsa } from '@noble/curves/abstract/edwards.js';
 * import { jubjub } from '@noble/curves/misc.js';
 * import { sha512 } from '@noble/hashes/sha2.js';
 * const sigs = eddsa(jubjub.Point, sha512);
 * const { secretKey, publicKey } = sigs.keygen();
 * const msg = new TextEncoder().encode('hello noble');
 * const sig = sigs.sign(msg, secretKey);
 * const isValid = sigs.verify(sig, msg, publicKey);
 * ```
 */
export declare function eddsa(Point: EdwardsPointCons, cHash: TArg<FHash>, eddsaOpts?: TArg<EdDSAOpts>): EdDSA;
