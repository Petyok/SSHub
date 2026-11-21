import { type CHash, type HmacFn, type TArg, type TRet } from '../utils.ts';
import { type AffinePoint, type CurveLengths, type CurvePoint, type CurvePointCons } from './curve.ts';
import { type IField } from './modular.ts';
/** Shared affine point shape used by Weierstrass helpers. */
export type { AffinePoint };
export { DER, DERErr, type IDER } from './der.ts';
type EndoBasis = [[bigint, bigint], [bigint, bigint]];
/**
 * When Weierstrass curve has `a=0`, it becomes Koblitz curve.
 * Koblitz curves allow using **efficiently-computable GLV endomorphism ψ**.
 * Endomorphism speeds up un-precomputed public-scalar multiplication (verification / key
 * recovery) by splitting a scalar into two half-width halves that share doublings.
 *
 * Endomorphism consists of beta, lambda and basises:
 *
 * 1. GLV endomorphism ψ transforms a point: `P = (x, y) ↦ ψ(P) = (β·x mod p, y)`
 * 2. GLV scalar decomposition transforms a scalar: `k ≡ k₁ + k₂·λ (mod n)`
 * 3. Then these are combined: `k·P = k₁·P + k₂·ψ(P)`
 * 4. Two 128-bit point-by-scalar multiplications + one point addition is faster than
 *    one 256-bit multiplication.
 *
 * where
 * * beta: β ∈ Fₚ with β³ = 1, β ≠ 1
 * * lambda: λ ∈ Fₙ with λ³ = 1, λ ≠ 1
 * * `_splitEndoScalar` decomposes k ↦ k₁, k₂, by using reduced basis vectors.
 *   Gauss lattice reduction calculates them from initial basis vectors `(n, 0), (-λ, 0)`
 *
 * Check out `test/misc/endomorphism.js` and
 * {@link https://gist.github.com/paulmillr/eb670806793e84df628a7c434a873066 | this endomorphism gist}.
 */
export type EndomorphismOpts = {
    /** Cube root of unity used by the GLV endomorphism. */
    beta: bigint;
    /** Reduced lattice basis used for scalar splitting. */
    basises?: EndoBasis;
};
/** Two half-sized scalar components returned by endomorphism splitting. */
export type ScalarEndoParts = {
    /** Whether the first split scalar should be negated. */
    k1neg: boolean;
    /** Absolute value of the first split scalar. */
    k1: bigint;
    /** Whether the second split scalar should be negated. */
    k2neg: boolean;
    /** Absolute value of the second split scalar. */
    k2: bigint;
};
/** Splits scalar for GLV endomorphism. */
export declare function _splitEndoScalar(k: bigint, basis: EndoBasis, n: bigint): ScalarEndoParts;
/**
 * Option to enable hedged signatures with improved security.
 *
 * * Randomly generated k is bad, because broken CSPRNG would leak private keys.
 * * Deterministic k (RFC6979) is better; but is suspectible to fault attacks.
 *
 * We allow using technique described in RFC6979 3.6: additional k', a.k.a. adding randomness
 * to deterministic sig. If CSPRNG is broken & randomness is weak, it would STILL be as secure
 * as ordinary sig without ExtraEntropy.
 *
 * * `true` means "fetch data, from CSPRNG, incorporate it into k generation"
 * * `false` means "disable extra entropy, use purely deterministic k"
 * * `Uint8Array` passed means "incorporate following data into k generation"
 *
 * See {@link https://paulmillr.com/posts/deterministic-signatures/ | deterministic signatures}.
 */
export type ECDSAExtraEntropy = boolean | Uint8Array;
/**
 * - `compact` is the default format
 * - `recovered` is the same as compact, but with an extra byte indicating recovery byte
 * - `der` is ASN.1 DER encoding
 */
export type ECDSASignatureFormat = 'compact' | 'recovered' | 'der';
/**
 * - `prehash`: (default: true) indicates whether to do sha256(message).
 *   When a custom hash is used, it must be set to `false`.
 */
export type ECDSARecoverOpts = {
    /** Whether to hash the message before signature recovery. */
    prehash?: boolean;
};
/**
 * - `prehash`: (default: true) indicates whether to do sha256(message).
 *   When a custom hash is used, it must be set to `false`.
 * - `lowS`: (default: true) prohibits signatures with `sig.s >= CURVE.n/2n`.
 *   Compatible with BTC/ETH. Setting `lowS: false` allows to create malleable signatures,
 *   which is default openssl behavior.
 *   Non-malleable signatures can still be successfully verified in openssl.
 * - `format`: (default: 'compact') 'compact' or 'recovered' with recovery byte
 */
export type ECDSAVerifyOpts = {
    /** Whether to hash the message before verification. */
    prehash?: boolean;
    /** Whether to reject high-S signatures. */
    lowS?: boolean;
    /** Signature encoding to accept. */
    format?: ECDSASignatureFormat;
};
/**
 * - `prehash`: (default: true) indicates whether to do sha256(message).
 *   When a custom hash is used, it must be set to `false`.
 * - `lowS`: (default: true) prohibits signatures with `sig.s >= CURVE.n/2n`.
 *   Compatible with BTC/ETH. Setting `lowS: false` allows to create malleable signatures,
 *   which is default openssl behavior.
 *   Non-malleable signatures can still be successfully verified in openssl.
 * - `format`: (default: 'compact') 'compact' or 'recovered' with recovery byte
 * - `extraEntropy`: (default: false) creates signatures with increased
 *   security, see {@link ECDSAExtraEntropy}
 */
export type ECDSASignOpts = {
    /** Whether to hash the message before signing. */
    prehash?: boolean;
    /** Whether to normalize signatures into the low-S half-order. */
    lowS?: boolean;
    /** Signature encoding to produce. */
    format?: ECDSASignatureFormat;
    /** Optional hedging input for deterministic k generation. */
    extraEntropy?: ECDSAExtraEntropy;
};
/** Projective XYZ point used by short Weierstrass curves. */
export interface WeierstrassPoint<T> extends CurvePoint<T, WeierstrassPoint<T>> {
    /** projective X coordinate. Different from affine x. */
    readonly X: T;
    /** projective Y coordinate. Different from affine y. */
    readonly Y: T;
    /** projective z coordinate */
    readonly Z: T;
    /** affine x coordinate. Different from projective X. */
    get x(): T;
    /** affine y coordinate. Different from projective Y. */
    get y(): T;
    /**
     * Encode the point into compressed or uncompressed SEC1 bytes.
     * @param isCompressed - Whether to use the compressed form.
     * @returns Encoded point bytes.
     */
    toBytes(isCompressed?: boolean): TRet<Uint8Array>;
    /**
     * Encode the point into compressed or uncompressed SEC1 hex.
     * @param isCompressed - Whether to use the compressed form.
     * @returns Encoded point hex.
     */
    toHex(isCompressed?: boolean): string;
    /**
     * Double-scalar multiplication `a⋅this + b⋅other` via Strauss–Shamir: both scalar walks
     * share one doubling chain, and GLV endomorphism (when the curve has one) halves the chain
     * again by splitting each scalar. 1.3-1.7x faster than two `multiplyUnsafe()` calls.
     * Not constant-time: only for public scalars, e.g. ECDSA verification's `u1⋅G + u2⋅P`.
     * @param a - Scalar for this point.
     * @param other - Second point.
     * @param b - Scalar for the second point.
     * @returns Combined product point.
     */
    mulAddUnsafe(a: bigint, other: WeierstrassPoint<T>, b: bigint): WeierstrassPoint<T>;
}
/** Constructor and metadata helpers for Weierstrass points. */
export interface WeierstrassPointCons<T> extends CurvePointCons<WeierstrassPoint<T>> {
    /** Does NOT validate if the point is valid. Use `.assertValidity()`. */
    new (X: T, Y: T, Z: T): WeierstrassPoint<T>;
    /**
     * Return the curve parameters captured by this point constructor.
     * @returns Curve parameters.
     */
    CURVE(): WeierstrassOpts<T>;
}
/**
 * Weierstrass curve options.
 *
 * * p: prime characteristic (order) of finite field, in which arithmetics is done
 * * n: order of prime subgroup a.k.a total amount of valid curve points
 * * h: cofactor, usually 1. h*n is group order; n is subgroup order
 * * a: formula param, must be in field of p
 * * b: formula param, must be in field of p
 * * Gx: x coordinate of generator point a.k.a. base point
 * * Gy: y coordinate of generator point
 */
export type WeierstrassOpts<T> = Readonly<{
    /** Base-field modulus. */
    p: bigint;
    /** Prime subgroup order. */
    n: bigint;
    /** Curve cofactor. */
    h: bigint;
    /** Weierstrass curve parameter `a`. */
    a: T;
    /** Weierstrass curve parameter `b`. */
    b: T;
    /** Generator x coordinate. */
    Gx: T;
    /** Generator y coordinate. */
    Gy: T;
}>;
/**
 * Optional helpers and overrides for a Weierstrass point constructor.
 *
 * When a cofactor != 1, there can be effective methods to:
 * 1. Determine whether a point is torsion-free
 * 2. Clear torsion component
 */
export type WeierstrassExtraOpts<T> = Partial<{
    /** Optional base-field override. */
    Fp: IField<T>;
    /** Optional scalar-field override. */
    Fn: IField<bigint>;
    /**
     * Whether the point at infinity is a public value of this point type. When unset, infinity
     * fails `assertValidity()` and has no byte encoding at all: encoding throws and `0x00` does
     * not decode, which keeps `Point.fromBytes()` usable as a strict key boundary because
     * infinity cannot be expressed in bytes. When set, infinity validates and uses the SEC 1
     * v2.0 §2.3.3 / §2.3.4 single-octet `0x00` form, unless `fromBytes` / `toBytes` override it.
     */
    allowInfinityPoint: boolean;
    /** Optional GLV endomorphism data. */
    endo: EndomorphismOpts;
    /** RNG override used for scalar blinding. */
    randomBytes: (bytesLength?: number) => TRet<Uint8Array>;
    /** Optional torsion-check override. */
    isTorsionFree: (c: WeierstrassPointCons<T>, point: WeierstrassPoint<T>) => boolean;
    /** Optional cofactor-clearing override. */
    clearCofactor: (c: WeierstrassPointCons<T>, point: WeierstrassPoint<T>) => WeierstrassPoint<T>;
    /** Optional custom point decoder. */
    fromBytes: (bytes: TArg<Uint8Array>) => AffinePoint<T>;
    /** Optional custom point encoder. */
    toBytes: (c: WeierstrassPointCons<T>, point: WeierstrassPoint<T>, isCompressed: boolean) => TRet<Uint8Array>;
}>;
/**
 * Options for ECDSA signatures over a Weierstrass curve.
 *
 * * lowS: (default: true) whether produced or verified signatures occupy the
 *   low half of `ecdsaOpts.n`. Prevents malleability.
 * * hmac: (default: noble-hashes hmac) function, would be used to init hmac-drbg for k generation.
 * * randomBytes: (default: webcrypto os-level CSPRNG) custom method for fetching secure randomness.
 * * bits2int, bits2int_modN: used in sigs, sometimes overridden by curves. Custom hooks are
 *   treated as pure functions over validated bytes and MUST NOT mutate caller-owned buffers or
 *   closure-captured option bags. `bits2int_modN` must also return a canonical scalar in
 *   `[0..Point.Fn.ORDER-1]`.
 */
export type ECDSAOpts = Partial<{
    /** Default low-S policy for this ECDSA instance. */
    lowS: boolean;
    /** HMAC implementation used by RFC6979 DRBG. */
    hmac: HmacFn;
    /** RNG override used by helper constructors. */
    randomBytes: (bytesLength?: number) => TRet<Uint8Array>;
    /** Hash-to-integer conversion override. */
    bits2int: (bytes: TArg<Uint8Array>) => bigint;
    /** Hash-to-integer-mod-n conversion override. Returns a canonical scalar in `[0..Fn.ORDER-1]`. */
    bits2int_modN: (bytes: TArg<Uint8Array>) => bigint;
}>;
/** Elliptic Curve Diffie-Hellman helper namespace. */
export interface ECDH {
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
     * @param isCompressed - Whether to emit compressed SEC1 bytes.
     * @returns Encoded public key.
     */
    getPublicKey: (secretKey: TArg<Uint8Array>, isCompressed?: boolean) => TRet<Uint8Array>;
    /**
     * Compute the shared secret point from a secret key and peer public key.
     * @param secretKeyA - Local secret key bytes.
     * @param publicKeyB - Peer public key bytes.
     * @param isCompressed - Whether to emit compressed SEC1 bytes.
     * @returns Encoded shared point.
     */
    getSharedSecret: (secretKeyA: TArg<Uint8Array>, publicKeyB: TArg<Uint8Array>, isCompressed?: boolean) => TRet<Uint8Array>;
    /** Point constructor used by this ECDH instance. */
    Point: WeierstrassPointCons<bigint>;
    /** Validation and random-key helpers. */
    utils: {
        /** Check whether a secret key has the expected encoding. */
        isValidSecretKey: (secretKey: TArg<Uint8Array>) => boolean;
        /** Check whether a public key decodes to a valid point. */
        isValidPublicKey: (publicKey: TArg<Uint8Array>, isCompressed?: boolean) => boolean;
        /** Generate a valid random secret key. */
        randomSecretKey: (seed?: TArg<Uint8Array>) => TRet<Uint8Array>;
    };
    /** Byte lengths for keys and signatures exposed by this curve. */
    lengths: CurveLengths;
}
/**
 * ECDSA interface.
 * Only supported for prime fields, not Fp2 (extension fields).
 */
export interface ECDSA extends ECDH {
    /**
     * Sign a message with the given secret key.
     * @param message - Message bytes.
     * @param secretKey - Secret key bytes.
     * @param opts - Optional signing tweaks. See {@link ECDSASignOpts}.
     * @returns Encoded signature bytes.
     */
    sign: (message: TArg<Uint8Array>, secretKey: TArg<Uint8Array>, opts?: TArg<ECDSASignOpts>) => TRet<Uint8Array>;
    /**
     * Verify a signature against a message and public key.
     * @param signature - Encoded signature bytes.
     * @param message - Message bytes.
     * @param publicKey - Encoded public key.
     * @param opts - Optional verification tweaks. See {@link ECDSAVerifyOpts}.
     * @returns Whether the signature is valid.
     */
    verify: (signature: TArg<Uint8Array>, message: TArg<Uint8Array>, publicKey: TArg<Uint8Array>, opts?: TArg<ECDSAVerifyOpts>) => boolean;
    /**
     * Recover the public key encoded into a recoverable signature.
     * @param signature - Recoverable signature bytes.
     * @param message - Message bytes.
     * @param opts - Optional recovery tweaks. See {@link ECDSARecoverOpts}.
     * @returns Encoded recovered public key.
     */
    recoverPublicKey(signature: TArg<Uint8Array>, message: TArg<Uint8Array>, opts?: TArg<ECDSARecoverOpts>): TRet<Uint8Array>;
    /** Signature constructor and parser helpers. */
    Signature: ECDSASignatureCons;
}
/**
 * Creates weierstrass Point constructor, based on specified curve options.
 *
 * See {@link WeierstrassOpts}.
 * @param params - Curve parameters. See {@link WeierstrassOpts}.
 * @param extraOpts - Optional helpers and overrides. See {@link WeierstrassExtraOpts}.
 * @returns Weierstrass point constructor.
 * @throws If the curve parameters, overrides, or point codecs are invalid. {@link Error}
 *
 * @example
 * Construct a point type from explicit Weierstrass curve parameters.
 *
 * ```js
 * const opts = {
 *   p: 0xfffffffffffffffffffffffffffffffeffffac73n,
 *   n: 0x100000000000000000001b8fa16dfab9aca16b6b3n,
 *   h: 1n,
 *   a: 0n,
 *   b: 7n,
 *   Gx: 0x3b4c382ce37aa192a4019e763036f4f5dd4d7ebbn,
 *   Gy: 0x938cf935318fdced6bc28286531733c3f03c4feen,
 * };
 * const secp160k1_Point = weierstrass(opts);
 * ```
 */
export declare function weierstrass<T>(params: WeierstrassOpts<T>, extraOpts?: WeierstrassExtraOpts<T>): WeierstrassPointCons<T>;
/** Parsed ECDSA signature with helpers for recovery and re-encoding. */
export interface ECDSASignature {
    /** Signature component `r`. */
    readonly r: bigint;
    /** Signature component `s`. */
    readonly s: bigint;
    /** Optional recovery bit for recoverable signatures. */
    readonly recovery?: number;
    /**
     * Return a copy of the signature with a recovery bit attached.
     * @param recovery - Recovery bit to attach.
     * @returns Signature with an attached recovery bit.
     */
    addRecoveryBit(recovery: number): ECDSASignature & {
        readonly recovery: number;
    };
    /**
     * Check whether the signature uses the high-S half-order.
     * @returns Whether the signature uses the high-S half-order.
     */
    hasHighS(): boolean;
    /**
     * Recover the public key from the hashed message and recovery bit.
     * @param messageHash - Hashed message bytes.
     * @returns Recovered public-key point.
     */
    recoverPublicKey(messageHash: TArg<Uint8Array>): WeierstrassPoint<bigint>;
    /**
     * Encode the signature into bytes.
     * @param format - Signature encoding to produce.
     * @returns Encoded signature bytes.
     */
    toBytes(format?: string): TRet<Uint8Array>;
    /**
     * Encode the signature into hex.
     * @param format - Signature encoding to produce.
     * @returns Encoded signature hex.
     */
    toHex(format?: string): string;
}
/** Constructor and decoding helpers for ECDSA signatures. */
export type ECDSASignatureCons = {
    /** Create a signature from `r`, `s`, and an optional recovery bit. */
    new (r: bigint, s: bigint, recovery?: number): ECDSASignature;
    /**
     * Decode a signature from bytes.
     * @param bytes - Encoded signature bytes.
     * @param format - Signature encoding to parse.
     * @returns Parsed signature.
     */
    fromBytes(bytes: TArg<Uint8Array>, format?: ECDSASignatureFormat): ECDSASignature;
    /**
     * Decode a signature from hex.
     * @param hex - Encoded signature hex.
     * @param format - Signature encoding to parse.
     * @returns Parsed signature.
     */
    fromHex(hex: string, format?: ECDSASignatureFormat): ECDSASignature;
};
/**
 * Sometimes users only need getPublicKey, getSharedSecret, and secret key handling.
 * This helper ensures no signature functionality is present. Less code, smaller bundle size.
 * @param Point - Weierstrass point constructor.
 * @param ecdhOpts - Optional randomness helpers:
 *   - `randomBytes` (optional): Optional RNG override.
 * @returns ECDH helper namespace.
 * @example
 * Sometimes users only need getPublicKey, getSharedSecret, and secret key handling.
 *
 * ```ts
 * import { ecdh } from '@noble/curves/abstract/weierstrass.js';
 * import { p256 } from '@noble/curves/nist.js';
 * const dh = ecdh(p256.Point);
 * const alice = dh.keygen();
 * const shared = dh.getSharedSecret(alice.secretKey, alice.publicKey);
 * ```
 */
export declare function ecdh(Point: WeierstrassPointCons<bigint>, ecdhOpts?: TArg<{
    randomBytes?: (bytesLength?: number) => TRet<Uint8Array>;
}>): ECDH;
/**
 * Creates ECDSA signing interface for given elliptic curve `Point` and `hash` function.
 *
 * @param Point - created using {@link weierstrass} function
 * @param hash - used for 1) message prehash-ing 2) k generation in `sign`, using hmac_drbg(hash)
 * @param ecdsaOpts - rarely needed, see {@link ECDSAOpts}:
 *   - `lowS`: Default low-S policy.
 *   - `hmac`: HMAC implementation used by RFC6979 DRBG.
 *   - `randomBytes`: Optional RNG override.
 *   - `bits2int`: Optional hash-to-int conversion override.
 *   - `bits2int_modN`: Optional hash-to-int-mod-n conversion override.
 *
 * @returns ECDSA helper namespace.
 * @example
 * Create an ECDSA signer/verifier bundle for one curve implementation.
 *
 * ```ts
 * import { ecdsa } from '@noble/curves/abstract/weierstrass.js';
 * import { p256 } from '@noble/curves/nist.js';
 * import { sha256 } from '@noble/hashes/sha2.js';
 * const p256ecdsa = ecdsa(p256.Point, sha256);
 * const { secretKey, publicKey } = p256ecdsa.keygen();
 * const msg = new TextEncoder().encode('hello noble');
 * const sig = p256ecdsa.sign(msg, secretKey);
 * const isValid = p256ecdsa.verify(sig, msg, publicKey);
 * ```
 */
export declare function ecdsa(Point: WeierstrassPointCons<bigint>, hash: TArg<CHash>, ecdsaOpts?: TArg<ECDSAOpts>): ECDSA;
