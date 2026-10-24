import type { AffinePoint } from './abstract/curve.ts';
import { PrimeEdwardsPoint, type EdDSA, type EdwardsPoint, type EdwardsPointCons } from './abstract/edwards.ts';
import { type FROST } from './abstract/frost.ts';
import { type H2CHasher, type H2CHasherBase } from './abstract/hash-to-curve.ts';
import { type IField } from './abstract/modular.ts';
import { type MontgomeryECDH } from './abstract/montgomery.ts';
import { type OPRF } from './abstract/oprf.ts';
import { type TArg, type TRet } from './utils.ts';
/**
 * ed448 EdDSA curve and methods.
 * @example
 * Generate one Ed448 keypair, sign a message, and verify it.
 *
 * ```js
 * import { ed448 } from '@noble/curves/ed448.js';
 * const { secretKey, publicKey } = ed448.keygen();
 * // const publicKey = ed448.getPublicKey(secretKey);
 * const msg = new TextEncoder().encode('hello noble');
 * const sig = ed448.sign(msg, secretKey);
 * const isValid = ed448.verify(sig, msg, publicKey);
 * ```
 */
export declare const ed448: EdDSA;
/**
 * Prehashed version of ed448. See {@link ed448}
 * @example
 * Use the prehashed Ed448 variant for one message.
 *
 * ```ts
 * const { secretKey, publicKey } = ed448ph.keygen();
 * const msg = new TextEncoder().encode('hello noble');
 * const sig = ed448ph.sign(msg, secretKey);
 * const isValid = ed448ph.verify(sig, msg, publicKey);
 * ```
 */
export declare const ed448ph: EdDSA;
/**
 * E448 here is NIST SP 800-186 §3.2.3.3 E448, the Edwards representation of
 * Curve448, not RFC 8032 edwards448 / Goldilocks.
 * Goldilocks is the separate 4-isogenous curve exposed as `ed448`.
 * We keep the corrected prime-order base here; RFC 7748's literal Edwards
 * point / map are wrong for this curve model, and the literal point is the
 * wrong-sign order-2*n variant.
 * @param X - Projective X coordinate.
 * @param Y - Projective Y coordinate.
 * @param Z - Projective Z coordinate.
 * @param T - Projective T coordinate.
 * @example
 * Reconstruct and validate the E448 base point from its projective coordinates.
 *
 * ```ts
 * import { E448 } from '@noble/curves/ed448.js';
 * const point = new E448(E448.BASE.X, E448.BASE.Y, E448.BASE.Z, E448.BASE.T);
 * point.assertValidity();
 * ```
 */
export declare const E448: EdwardsPointCons;
/**
 * ECDH using curve448 aka x448.
 * The wrapper aborts on all-zero shared secrets by default, and seeded
 * `keygen(seed)` reuses the provided 56-byte seed buffer instead of copying it.
 *
 * @example
 * Derive one shared secret between two X448 peers.
 *
 * ```js
 * import { x448 } from '@noble/curves/ed448.js';
 * const alice = x448.keygen();
 * const bob = x448.keygen();
 * const shared = x448.getSharedSecret(alice.secretKey, bob.publicKey);
 * ```
 */
export declare const x448: TRet<MontgomeryECDH>;
/**
 * Hashing / encoding to ed448 points / field. RFC 9380 methods.
 * Public `mapToCurve()` consumes one field element bigint for `m = 1`, and RFC
 * Appendix J vectors use the special `QUUX-V01-*` test DST overrides rather
 * than the default suite IDs below.
 * @example
 * Hash one message onto the ed448 curve.
 *
 * ```ts
 * const point = ed448_hasher.hashToCurve(new TextEncoder().encode('hello noble'));
 * ```
 */
export declare const ed448_hasher: H2CHasher<EdwardsPointCons>;
/**
 * FROST threshold signatures over ed448. RFC 9591.
 * @example
 * Create one trusted-dealer package for 2-of-3 ed448 signing.
 *
 * ```ts
 * const alice = ed448_FROST.Identifier.derive('alice@example.com');
 * const bob = ed448_FROST.Identifier.derive('bob@example.com');
 * const carol = ed448_FROST.Identifier.derive('carol@example.com');
 * const deal = ed448_FROST.trustedDealer({ min: 2, max: 3 }, [alice, bob, carol]);
 * ```
 */
export declare const ed448_FROST: TRet<FROST>;
/**
 * Each ed448/EdwardsPoint has 4 different equivalent points. This can be
 * a source of bugs for protocols like ring signatures. Decaf was created to solve this.
 * Decaf point operates in X:Y:Z:T extended coordinates like EdwardsPoint,
 * but it should work in its own namespace: do not combine those two.
 * See [RFC9496](https://www.rfc-editor.org/rfc/rfc9496).
 */
declare class _DecafPoint extends PrimeEdwardsPoint<_DecafPoint> {
    static BASE: _DecafPoint;
    static ZERO: _DecafPoint;
    static Fp: IField<bigint>;
    static Fn: IField<bigint>;
    constructor(ep: EdwardsPoint);
    /**
     * Create one Decaf448 point from affine Edwards coordinates.
     * This wraps the internal Edwards representative directly and is not a
     * canonical decaf448 decoding path.
     * Use `toBytes()` / `fromBytes()` if canonical decaf448 bytes matter.
     */
    static fromAffine(ap: AffinePoint<bigint>): _DecafPoint;
    protected assertSame(other: _DecafPoint): void;
    protected init(ep: EdwardsPoint): _DecafPoint;
    static fromBytes(bytes: TArg<Uint8Array>): _DecafPoint;
    /**
     * Converts decaf-encoded string to decaf point.
     * Described in [RFC9496](https://www.rfc-editor.org/rfc/rfc9496#name-decode-2).
     * @param hex - Decaf-encoded 56 bytes. Not every 56-byte string is valid decaf encoding
     */
    static fromHex(hex: string): _DecafPoint;
    /**
     * Encodes decaf point to Uint8Array.
     * Described in [RFC9496](https://www.rfc-editor.org/rfc/rfc9496#name-encode-2).
     */
    toBytes(): TRet<Uint8Array>;
    /**
     * Compare one point to another.
     * Described in [RFC9496](https://www.rfc-editor.org/rfc/rfc9496#name-equals-2).
     */
    equals(other: _DecafPoint): boolean;
    is0(): boolean;
}
/** Prime-order Decaf448 group bundle. */
export declare const decaf448: {
    Point: typeof _DecafPoint;
};
/**
 * Hashing to decaf448 points / field. RFC 9380 methods.
 * `hashToCurve()` is RFC 9380 `hash_to_decaf448`, `deriveToCurve()` is RFC
 * 9496 element derivation, and `hashToScalar()` is a library helper layered on
 * top of RFC 9496 scalar reduction.
 * @example
 * Hash one message onto decaf448.
 *
 * ```ts
 * const point = decaf448_hasher.hashToCurve(new TextEncoder().encode('hello noble'));
 * ```
 */
export declare const decaf448_hasher: H2CHasherBase<typeof _DecafPoint>;
/**
 * decaf448 OPRF, defined in RFC 9497.
 * @example
 * Run one blind/evaluate/finalize OPRF round over decaf448.
 *
 * ```ts
 * const input = new TextEncoder().encode('hello noble');
 * const keys = decaf448_oprf.oprf.generateKeyPair();
 * const blind = decaf448_oprf.oprf.blind(input);
 * const evaluated = decaf448_oprf.oprf.blindEvaluate(keys.secretKey, blind.blinded);
 * const output = decaf448_oprf.oprf.finalize(input, blind.blind, evaluated);
 * ```
 */
export declare const decaf448_oprf: TRet<OPRF>;
/**
 * Weird / bogus points, useful for debugging.
 * Unlike ed25519, there is no ed448 generator point which can produce full T subgroup.
 * Instead, the torsion subgroup here is cyclic of order 4, generated by
 * `(1, 0)`, and the array below lists that subgroup set.
 * @example
 * Decode one known torsion point for debugging.
 *
 * ```ts
 * import { ED448_TORSION_SUBGROUP, ed448 } from '@noble/curves/ed448.js';
 * const point = ed448.Point.fromHex(ED448_TORSION_SUBGROUP[1]);
 * ```
 */
export declare const ED448_TORSION_SUBGROUP: readonly string[];
export {};
