/**
 * BLS != BLS.
 * The file implements BLS (Boneh-Lynn-Shacham) signatures.
 * Used in both BLS (Barreto-Lynn-Scott) and BN (Barreto-Naehrig)
 * families of pairing-friendly curves.
 * Consists of two curves: G1 and G2:
 * - G1 is a subgroup of (x, y) E(Fq) over y² = x³ + 4.
 * - G2 is a subgroup of ((x₁, x₂+i), (y₁, y₂+i)) E(Fq²) over y² = x³ + 4(1 + i) where i is √-1
 * - Gt, created by bilinear (ate) pairing e(G1, G2), consists of p-th roots of unity in
 *   Fq^k where k is embedding degree. Only degree 12 is currently supported, 24 is not.
 * Pairing is used to aggregate and verify signatures.
 * There are two modes of operation:
 * - Long signatures:  X-byte keys + 2X-byte sigs (G1 keys + G2 sigs).
 * - Short signatures: 2X-byte keys + X-byte sigs (G2 keys + G1 sigs).
 * @module
 **/
/*! noble-curves - MIT License (c) 2022 Paul Miller (paulmillr.com) */
import { type TArg, type TRet } from '../utils.ts';
import { type CurveLengths } from './curve.ts';
import { type H2CHasher, type H2COpts, type MapToCurve } from './hash-to-curve.ts';
import { type IField } from './modular.ts';
import type { Fp12, Fp12Bls, Fp2, Fp2Bls, Fp6Bls } from './tower.ts';
import { type WeierstrassPoint, type WeierstrassPointCons } from './weierstrass.ts';
type Fp = bigint;
/**
 * Twist convention used by the pairing formulas for a concrete curve family.
 * BLS12-381 uses a multiplicative twist, while BN254 uses a divisive one.
 */
export type BlsTwistType = 'multiplicative' | 'divisive';
/**
 * Codec exposed as `curve.shortSignatures.Signature`.
 * Use it to parse or serialize G1 signatures in short-signature mode.
 * In this mode, public keys live in G2.
 */
export type BlsShortSignatureCoder<Fp> = {
    /**
     * Parse a compressed signature from raw bytes.
     * @param bytes - Compressed signature bytes.
     * @returns Parsed signature point.
     */
    fromBytes(bytes: TArg<Uint8Array>): WeierstrassPoint<Fp>;
    /**
     * Parse a compressed signature from a hex string.
     * @param hex - Compressed signature hex string.
     * @returns Parsed signature point.
     */
    fromHex(hex: string): WeierstrassPoint<Fp>;
    /**
     * Encode a signature point into compressed bytes.
     * @param point - Signature point.
     * @returns Compressed signature bytes.
     */
    toBytes(point: WeierstrassPoint<Fp>): TRet<Uint8Array>;
    /**
     * Encode a signature point into a hex string.
     * @param point - Signature point.
     * @returns Compressed signature hex.
     */
    toHex(point: WeierstrassPoint<Fp>): string;
};
/**
 * Codec exposed as `curve.longSignatures.Signature`.
 * Use it to parse or serialize G2 signatures in long-signature mode.
 * In this mode, public keys live in G1.
 */
export type BlsLongSignatureCoder<Fp> = {
    /**
     * Parse a compressed signature from raw bytes.
     * @param bytes - Compressed signature bytes.
     * @returns Parsed signature point.
     */
    fromBytes(bytes: TArg<Uint8Array>): WeierstrassPoint<Fp>;
    /**
     * Parse a compressed signature from a hex string.
     * @param hex - Compressed signature hex string.
     * @returns Parsed signature point.
     */
    fromHex(hex: string): WeierstrassPoint<Fp>;
    /**
     * Encode a signature point into compressed bytes.
     * @param point - Signature point.
     * @returns Compressed signature bytes.
     */
    toBytes(point: WeierstrassPoint<Fp>): TRet<Uint8Array>;
    /**
     * Encode a signature point into a hex string.
     * @param point - Signature point.
     * @returns Compressed signature hex.
     */
    toHex(point: WeierstrassPoint<Fp>): string;
};
/** Tower fields needed by pairing code, hash-to-curve, and subgroup arithmetic. */
export type BlsFields = {
    /** Base field of G1 coordinates. */
    Fp: IField<Fp>;
    /** Scalar field used for secret scalars and subgroup order arithmetic. */
    Fr: IField<bigint>;
    /** Quadratic extension field used by G2. */
    Fp2: Fp2Bls;
    /** Sextic extension field used inside pairing arithmetic. */
    Fp6: Fp6Bls;
    /** Degree-12 extension field that contains the GT target group. */
    Fp12: Fp12Bls;
};
/**
 * Callback used by pairing post-processing hooks to add one more G2 point to the Miller-loop state.
 * @param Rx - Current projective X coordinate.
 * @param Ry - Current projective Y coordinate.
 * @param Rz - Current projective Z coordinate.
 * @param Qx - G2 affine x coordinate.
 * @param Qy - G2 affine y coordinate.
 * @returns Updated projective accumulator coordinates.
 */
export type BlsPostPrecomputePointAddFn = (Rx: Fp2, Ry: Fp2, Rz: Fp2, Qx: Fp2, Qy: Fp2) => {
    Rx: Fp2;
    Ry: Fp2;
    Rz: Fp2;
};
/**
 * Hook for curve-specific pairing cleanup after the Miller loop precomputes are built.
 * @param Rx - Current projective X coordinate.
 * @param Ry - Current projective Y coordinate.
 * @param Rz - Current projective Z coordinate.
 * @param Qx - G2 affine x coordinate.
 * @param Qy - G2 affine y coordinate.
 * @param pointAdd - Callback used to fold one more point into the accumulator.
 */
export type BlsPostPrecomputeFn = (Rx: Fp2, Ry: Fp2, Rz: Fp2, Qx: Fp2, Qy: Fp2, pointAdd: BlsPostPrecomputePointAddFn) => void;
/** Low-level pairing helpers shared by BLS curve bundles. */
export type BlsPairing = {
    /** Byte lengths for keys and signatures exposed by this pairing family. */
    lengths: CurveLengths;
    /** Scalar field used by the pairing and signing helpers. */
    Fr: IField<bigint>;
    /** Target field used for the GT result of pairings. */
    Fp12: Fp12Bls;
    /**
     * Build Miller-loop precomputes for one G2 point.
     * @param p - Valid non-ZERO G2 point to precompute.
     * @returns Pairing precompute table.
     */
    calcPairingPrecomputes: (p: WeierstrassPoint<Fp2>) => Precompute;
    /**
     * Evaluate a batch of Miller loops from precomputed line coefficients.
     * @param pairs - Precomputed Miller-loop inputs.
     * @returns Accumulated GT value before or after final exponentiation.
     */
    millerLoopBatch: (pairs: [Precompute, Fp, Fp][]) => Fp12;
    /**
     * Pair one G1 point with one G2 point.
     * @param P - G1 point.
     * @param Q - G2 point.
     * @param withFinalExponent - Whether to apply the final exponentiation step.
     * @returns GT pairing result.
     * @throws If either point is the point at infinity. {@link Error}
     */
    pairing: (P: WeierstrassPoint<Fp>, Q: WeierstrassPoint<Fp2>, withFinalExponent?: boolean) => Fp12;
    /**
     * Pair many G1/G2 pairs in one batch.
     * @param pairs - Point pairs to accumulate.
     * @param withFinalExponent - Whether to apply the final exponentiation step.
     * @returns GT pairing result. Empty input returns the multiplicative identity in GT.
     */
    pairingBatch: (pairs: {
        g1: WeierstrassPoint<Fp>;
        g2: WeierstrassPoint<Fp2>;
    }[], withFinalExponent?: boolean) => Fp12;
    /**
     * Generate a random secret key for this pairing family.
     * @param seed - Optional seed material.
     * @returns Secret key bytes.
     */
    randomSecretKey: (seed?: TArg<Uint8Array>) => TRet<Uint8Array>;
};
/**
 * Parameters that define the Miller-loop shape and twist handling
 * for a concrete pairing family.
 */
export type BlsPairingParams = {
    /** Signed loop parameter used by the Miller loop. */
    ateLoopSize: bigint;
    /** Whether the signed Miller-loop parameter is negative. */
    xNegative: boolean;
    /**
     * Twist convention used by the pairing formulas.
     * BLS12-381 is multiplicative; BN254 is divisive.
     */
    twistType: BlsTwistType;
    /**
     * Optional RNG override used by helper constructors.
     * @param len - Requested byte length.
     * @returns Random bytes.
     */
    randomBytes?: (len?: number) => TRet<Uint8Array>;
    /**
     * Optional hook for curve-specific untwisting after precomputation.
     * Used by BN254 after the Miller loop.
     */
    postPrecompute?: BlsPostPrecomputeFn;
};
/** Hash-to-curve settings shared by the G1 and G2 hashers inside a BLS curve bundle. */
export type BlsHasherParams = {
    /**
     * Optional map-to-curve override for G1.
     * Receives the hash-to-field tuple and returns one affine G1 point.
     */
    mapToG1?: MapToCurve<Fp>;
    /**
     * Optional map-to-curve override for G2.
     * Receives the hash-to-field tuple and returns one affine G2 point.
     */
    mapToG2?: MapToCurve<Fp2>;
    /** Shared baseline hash-to-curve options. */
    hasherOpts: H2COpts;
    /** G1-specific hash-to-curve options merged on top of `hasherOpts`. */
    hasherOptsG1: H2COpts;
    /** G2-specific hash-to-curve options merged on top of `hasherOpts`. */
    hasherOptsG2: H2COpts;
};
type PrecomputeSingle = [Fp2, Fp2, Fp2][];
type Precompute = PrecomputeSingle[];
/**
 * BLS consists of two curves: G1 and G2:
 * - G1 is a subgroup of (x, y) E(Fq) over y² = x³ + 4.
 * - G2 is a subgroup of ((x₁, x₂+i), (y₁, y₂+i)) E(Fq²) over y² = x³ + 4(1 + i) where i is √-1
 */
export interface BlsCurvePair {
    /** Byte lengths for keys and signatures exposed by this curve family. */
    lengths: CurveLengths;
    /**
     * Shared Miller-loop batch evaluator.
     * @param pairs - Precomputed Miller-loop inputs.
     * @returns Accumulated GT value.
     */
    millerLoopBatch: BlsPairing['millerLoopBatch'];
    /**
     * Pair one G1 point with one G2 point.
     * @param P - G1 point.
     * @param Q - G2 point.
     * @param withFinalExponent - Whether to apply the final exponentiation step.
     * @returns GT pairing result.
     * @throws If either point is the point at infinity. {@link Error}
     */
    pairing: BlsPairing['pairing'];
    /**
     * Pair many G1/G2 pairs in one batch.
     * @param pairs - Point pairs to accumulate.
     * @param withFinalExponent - Whether to apply the final exponentiation step.
     * @returns GT pairing result. Empty input returns the multiplicative identity in GT.
     */
    pairingBatch: BlsPairing['pairingBatch'];
    /** G1 point constructor for the base field subgroup. */
    G1: {
        Point: WeierstrassPointCons<Fp>;
    };
    /** G2 point constructor for the twist subgroup. */
    G2: {
        Point: WeierstrassPointCons<Fp2>;
    };
    /** Tower fields exposed by the pairing implementation. */
    fields: {
        Fp: IField<Fp>;
        Fp2: Fp2Bls;
        Fp6: Fp6Bls;
        Fp12: Fp12Bls;
        Fr: IField<bigint>;
    };
    /** Utility helpers shared by hashers and signers. */
    utils: {
        randomSecretKey: (seed?: TArg<Uint8Array>) => TRet<Uint8Array>;
        calcPairingPrecomputes: BlsPairing['calcPairingPrecomputes'];
    };
    /** Public pairing parameters exposed for introspection. */
    params: {
        ateLoopSize: bigint;
        xNegative: boolean;
        twistType: BlsTwistType;
        postPrecompute?: BlsPostPrecomputeFn;
    };
}
/** BLS curve bundle extended with hash-to-curve helpers for G1 and G2. */
export interface BlsCurvePairWithHashers extends BlsCurvePair {
    /** G1 hasher bundle with RFC 9380 helpers. */
    G1: H2CHasher<WeierstrassPointCons<Fp>>;
    /** G2 hasher bundle with RFC 9380 helpers. */
    G2: H2CHasher<WeierstrassPointCons<Fp2>>;
}
/** BLS curve bundle extended with both hashers and signature helpers. */
export interface BlsCurvePairWithSignatures extends BlsCurvePairWithHashers {
    /** Long-signature mode: G1 public keys and G2 signatures. */
    longSignatures: BlsSigs<bigint, Fp2>;
    /** Short-signature mode: G2 public keys and G1 signatures. */
    shortSignatures: BlsSigs<Fp2, bigint>;
}
type BLSInput = TArg<Uint8Array>;
/** BLS signer helpers for one signature mode. */
export interface BlsSigs<P, S> {
    /** Byte lengths for secret keys, public keys, and signatures. */
    lengths: CurveLengths;
    /**
     * Generate a secret/public key pair for this signature mode.
     * @param seed - Optional seed material.
     * @returns Secret and public key pair.
     */
    keygen(seed?: TArg<Uint8Array>): {
        secretKey: TRet<Uint8Array>;
        publicKey: WeierstrassPoint<P>;
    };
    /**
     * Derive the public key from a secret key.
     * @param secretKey - Secret key bytes.
     * @returns Public-key point.
     */
    getPublicKey(secretKey: TArg<Uint8Array>): WeierstrassPoint<P>;
    /**
     * Sign a message already hashed onto the signature subgroup.
     * @param hashedMessage - Message mapped to the signature subgroup.
     * @param secretKey - Secret key bytes.
     * @returns Signature point.
     */
    sign(hashedMessage: WeierstrassPoint<S>, secretKey: TArg<Uint8Array>): WeierstrassPoint<S>;
    /**
     * Verify one signature against one public key and hashed message.
     * Malformed encoded signatures or keys may throw during point decoding; `false` means
     * well-formed inputs failed the pairing equation.
     * @param signature - Signature point or encoded signature.
     * @param message - Hashed message point.
     * @param publicKey - Public-key point or encoded key.
     * @returns Whether the signature is valid.
     */
    verify(signature: WeierstrassPoint<S> | BLSInput, message: WeierstrassPoint<S>, publicKey: WeierstrassPoint<P> | BLSInput): boolean;
    /**
     * Verify one aggregated signature against many `(message, publicKey)` pairs.
     * @param signature - Aggregated signature.
     * @param items - Message/public-key pairs.
     * @returns Whether the aggregated signature is valid. Same-message aggregate verification still
     *   requires proof of possession or another rogue-key defense from the caller.
     */
    verifyBatch: (signature: WeierstrassPoint<S> | BLSInput, items: {
        message: WeierstrassPoint<S>;
        publicKey: WeierstrassPoint<P> | BLSInput;
    }[]) => boolean;
    /**
     * Add many public keys into one aggregate point.
     * Encoded inputs are decoded through `fromBytes()`; point instances are treated as already
     * validated caller-owned objects to keep aggregation linear in additions.
     * @param publicKeys - Public keys to aggregate.
     * @returns Aggregated public-key point. This is raw point addition and does not add proof of
     *   possession or rogue-key protection on its own.
     */
    aggregatePublicKeys(publicKeys: (WeierstrassPoint<P> | BLSInput)[]): WeierstrassPoint<P>;
    /**
     * Add many signatures into one aggregate point.
     * Encoded inputs are decoded through `fromBytes()`; point instances are treated as already
     * validated caller-owned objects to keep aggregation linear in additions.
     * @param signatures - Signatures to aggregate.
     * @returns Aggregated signature point. This is raw point addition and does not change the proof
     *   of possession requirements of the aggregate-verification scheme.
     */
    aggregateSignatures(signatures: (WeierstrassPoint<S> | BLSInput)[]): WeierstrassPoint<S>;
    /**
     * Hash an arbitrary message onto the signature subgroup.
     * @param message - Message bytes.
     * @param DST - Optional domain separation tag.
     * @returns Curve point on the signature subgroup.
     */
    hash(message: TArg<Uint8Array>, DST?: TArg<string | Uint8Array>): WeierstrassPoint<S>;
    /** Signature codec for this mode. */
    Signature: BlsLongSignatureCoder<S>;
}
type BlsSignatureCoders = Partial<{
    LongSignature: BlsLongSignatureCoder<Fp2>;
    ShortSignature: BlsShortSignatureCoder<Fp>;
}>;
/**
 * @param fields - Tower field implementations.
 * @param G1_Point - G1 point constructor.
 * @param G2_Point - G2 point constructor.
 * @param params - Pairing parameters. See {@link BlsPairingParams}.
 * @returns Pairing-only BLS helpers. The returned pairing surface rejects infinity inputs, while
 *   empty `pairingBatch(...)` calls return the multiplicative identity in GT. This keeps the
 *   low-level pairing API fail-closed for BLS-style callers, where identity points usually signal
 *   broken hash / wiring instead of an intentionally neutral pairing term. This also eagerly
 *   precomputes the G1 base-point table as a performance side effect.
 * @throws If the pairing parameters or underlying curve helpers are inconsistent. {@link Error}
 * @example
 * ```ts
 * import { blsBasic } from '@noble/curves/abstract/bls.js';
 * import { bn254 } from '@noble/curves/bn254.js';
 * // Rebuild the pairing-only helper from a concrete curve's public pieces.
 * const pair = blsBasic(bn254.fields, bn254.G1.Point, bn254.G2.Point, bn254.params);
 * const gt = pair.pairing(pair.G1.Point.BASE, pair.G2.Point.BASE);
 * ```
 */
export declare function blsBasic(fields: TArg<BlsFields>, G1_Point: WeierstrassPointCons<Fp>, G2_Point: WeierstrassPointCons<Fp2>, params: TArg<BlsPairingParams>): BlsCurvePair;
/**
 * @param fields - Tower field implementations.
 * @param G1_Point - G1 point constructor.
 * @param G2_Point - G2 point constructor.
 * @param params - Pairing parameters. See {@link BlsPairingParams}.
 * @param hasherParams - Hash-to-curve configuration. See {@link BlsHasherParams}.
 * @param signatureCoders - Signature codecs.
 * @returns BLS helpers with signers. The inherited pairing surface still rejects infinity inputs,
 *   and empty `pairingBatch(...)` calls still return the multiplicative identity in GT. Aggregate
 *   verification still requires proof of possession or another rogue-key defense from the caller.
 * @throws If the pairing, hashing, or signature helpers are configured inconsistently. {@link Error}
 * @example
 * ```ts
 * import { bls } from '@noble/curves/abstract/bls.js';
 * import { bls12_381 } from '@noble/curves/bls12-381.js';
 * // Rebuild a signer namespace from a concrete curve.
 * // Applications usually import bls12_381 directly.
 * const rebuilt = bls(
 *   bls12_381.fields,
 *   bls12_381.G1.Point,
 *   bls12_381.G2.Point,
 *   bls12_381.params,
 *   {
 *     hasherOpts: bls12_381.G2.defaults,
 *     hasherOptsG1: bls12_381.G1.defaults,
 *     hasherOptsG2: bls12_381.G2.defaults,
 *   },
 *   {}
 * );
 * const { secretKey, publicKey } = rebuilt.longSignatures.keygen();
 * ```
 */
export declare function bls(fields: TArg<BlsFields>, G1_Point: WeierstrassPointCons<Fp>, G2_Point: WeierstrassPointCons<Fp2>, params: TArg<BlsPairingParams>, hasherParams: TArg<BlsHasherParams>, signatureCoders: BlsSignatureCoders): BlsCurvePairWithSignatures;
export {};
