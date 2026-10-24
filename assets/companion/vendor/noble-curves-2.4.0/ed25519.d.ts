import { type AffinePoint } from './abstract/curve.ts';
import { PrimeEdwardsPoint, type EdDSA, type EdwardsPoint, type EdwardsPointCons } from './abstract/edwards.ts';
import { type FROST } from './abstract/frost.ts';
import { type H2CHasher, type H2CHasherBase } from './abstract/hash-to-curve.ts';
import { type IField } from './abstract/modular.ts';
import { type MontgomeryECDH } from './abstract/montgomery.ts';
import { type OPRF } from './abstract/oprf.ts';
import { type TArg, type TRet } from './utils.ts';
/**
 * ed25519 curve with EdDSA signatures.
 * Seeded `keygen(seed)` / `utils.randomSecretKey(seed)` reuse the provided
 * 32-byte seed buffer instead of copying it.
 * @example
 * Generate one Ed25519 keypair, sign a message, and verify it.
 *
 * ```js
 * import { ed25519 } from '@noble/curves/ed25519.js';
 * const { secretKey, publicKey } = ed25519.keygen();
 * // const publicKey = ed25519.getPublicKey(secretKey);
 * const msg = new TextEncoder().encode('hello noble');
 * const sig = ed25519.sign(msg, secretKey);
 * const isValid = ed25519.verify(sig, msg, publicKey); // ZIP215
 * // RFC8032 / FIPS 186-5
 * const isValid2 = ed25519.verify(sig, msg, publicKey, { zip215: false });
 * ```
 */
export declare const ed25519: EdDSA;
/**
 * Context version of ed25519 (ctx for domain separation). See {@link ed25519}
 * Seeded `keygen(seed)` / `utils.randomSecretKey(seed)` reuse the provided
 * 32-byte seed buffer instead of copying it.
 * @example
 * Sign and verify with Ed25519ctx under one explicit context.
 *
 * ```ts
 * const context = new TextEncoder().encode('docs');
 * const { secretKey, publicKey } = ed25519ctx.keygen();
 * const msg = new TextEncoder().encode('hello noble');
 * const sig = ed25519ctx.sign(msg, secretKey, { context });
 * const isValid = ed25519ctx.verify(sig, msg, publicKey, { context });
 * ```
 */
export declare const ed25519ctx: EdDSA;
/**
 * Prehashed version of ed25519. See {@link ed25519}
 * Seeded `keygen(seed)` / `utils.randomSecretKey(seed)` reuse the provided
 * 32-byte seed buffer instead of copying it.
 * @example
 * Use the prehashed Ed25519 variant for one message.
 *
 * ```ts
 * const { secretKey, publicKey } = ed25519ph.keygen();
 * const msg = new TextEncoder().encode('hello noble');
 * const sig = ed25519ph.sign(msg, secretKey);
 * const isValid = ed25519ph.verify(sig, msg, publicKey);
 * ```
 */
export declare const ed25519ph: EdDSA;
/**
 * FROST threshold signatures over ed25519. RFC 9591.
 * @example
 * Create one trusted-dealer package for 2-of-3 ed25519 signing.
 *
 * ```ts
 * const alice = ed25519_FROST.Identifier.derive('alice@example.com');
 * const bob = ed25519_FROST.Identifier.derive('bob@example.com');
 * const carol = ed25519_FROST.Identifier.derive('carol@example.com');
 * const deal = ed25519_FROST.trustedDealer({ min: 2, max: 3 }, [alice, bob, carol]);
 * ```
 */
export declare const ed25519_FROST: TRet<FROST>;
/**
 * ECDH using curve25519 aka x25519.
 * `getSharedSecret()` rejects low-order peer inputs by default, and seeded
 * `keygen(seed)` reuses the provided 32-byte seed buffer instead of copying it.
 * @example
 * Derive one shared secret between two X25519 peers.
 *
 * ```js
 * import { x25519 } from '@noble/curves/ed25519.js';
 * const alice = x25519.keygen();
 * const bob = x25519.keygen();
 * const alicePublic = x25519.getPublicKey(alice.secretKey);
 * const shared = x25519.getSharedSecret(alice.secretKey, bob.publicKey);
 * ```
 */
export declare const x25519: TRet<MontgomeryECDH>;
/**
 * RFC 9380 method `map_to_curve_elligator2_curve25519`. Experimental name: may be renamed later.
 * @private
 */
export declare function _map_to_curve_elligator2_curve25519(u: bigint): {
    xMn: bigint;
    xMd: bigint;
    yMn: bigint;
    yMd: bigint;
};
/**
 * Hashing to ed25519 points / field. RFC 9380 methods.
 * Public `mapToCurve()` returns the cofactor-cleared subgroup point; the
 * internal map callback below consumes one field element bigint, not `[bigint]`.
 * @example
 * Hash one message onto the ed25519 curve.
 *
 * ```ts
 * const point = ed25519_hasher.hashToCurve(new TextEncoder().encode('hello noble'));
 * ```
 */
export declare const ed25519_hasher: H2CHasher<EdwardsPointCons>;
/**
 * Wrapper over Edwards Point for ristretto255.
 *
 * Each ed25519/EdwardsPoint has 8 different equivalent points. This can be
 * a source of bugs for protocols like ring signatures. Ristretto was created to solve this.
 * Ristretto point operates in X:Y:Z:T extended coordinates like EdwardsPoint,
 * but it should work in its own namespace: do not combine those two.
 * See [RFC9496](https://www.rfc-editor.org/rfc/rfc9496).
 */
declare class _RistrettoPoint extends PrimeEdwardsPoint<_RistrettoPoint> {
    static BASE: _RistrettoPoint;
    static ZERO: _RistrettoPoint;
    static Fp: IField<bigint>;
    static Fn: IField<bigint>;
    constructor(ep: EdwardsPoint);
    /**
     * Create one Ristretto255 point from affine Edwards coordinates.
     * This wraps the internal Edwards representative directly and is not a
     * canonical ristretto255 decoding path.
     * Use `toBytes()` / `fromBytes()` if canonical ristretto255 bytes matter.
     */
    static fromAffine(ap: AffinePoint<bigint>): _RistrettoPoint;
    protected assertSame(other: _RistrettoPoint): void;
    protected init(ep: EdwardsPoint): _RistrettoPoint;
    static fromBytes(bytes: TArg<Uint8Array>): _RistrettoPoint;
    /**
     * Converts ristretto-encoded string to ristretto point.
     * Described in [RFC9496](https://www.rfc-editor.org/rfc/rfc9496#name-decode).
     * @param hex - Ristretto-encoded 32 bytes. Not every 32-byte string is valid ristretto encoding
     */
    static fromHex(hex: string): _RistrettoPoint;
    /**
     * Encodes ristretto point to Uint8Array.
     * Described in [RFC9496](https://www.rfc-editor.org/rfc/rfc9496#name-encode).
     */
    toBytes(): TRet<Uint8Array>;
    /**
     * Compares two Ristretto points.
     * Described in [RFC9496](https://www.rfc-editor.org/rfc/rfc9496#name-equals).
     */
    equals(other: _RistrettoPoint): boolean;
    is0(): boolean;
}
/** Prime-order Ristretto255 group bundle. */
export declare const ristretto255: {
    Point: typeof _RistrettoPoint;
};
/**
 * Hashing to ristretto255 points / field. RFC 9380 methods.
 * `hashToCurve()` is RFC 9380 Appendix B, `deriveToCurve()` is the RFC 9496
 * §4.3.4 element-derivation building block, and `hashToScalar()` is a
 * library-specific helper for OPRF-style use.
 * @example
 * Hash one message onto ristretto255.
 *
 * ```ts
 * const point = ristretto255_hasher.hashToCurve(new TextEncoder().encode('hello noble'));
 * ```
 */
export declare const ristretto255_hasher: H2CHasherBase<typeof _RistrettoPoint>;
/**
 * ristretto255 OPRF/VOPRF/POPRF bundle, defined in RFC 9497.
 * @example
 * Run one blind/evaluate/finalize OPRF round over ristretto255.
 *
 * ```ts
 * const input = new TextEncoder().encode('hello noble');
 * const keys = ristretto255_oprf.oprf.generateKeyPair();
 * const blind = ristretto255_oprf.oprf.blind(input);
 * const evaluated = ristretto255_oprf.oprf.blindEvaluate(keys.secretKey, blind.blinded);
 * const output = ristretto255_oprf.oprf.finalize(input, blind.blind, evaluated);
 * ```
 */
export declare const ristretto255_oprf: TRet<OPRF>;
/**
 * FROST threshold signatures over ristretto255. RFC 9591.
 * @example
 * Create one trusted-dealer package for 2-of-3 ristretto255 signing.
 *
 * ```ts
 * const alice = ristretto255_FROST.Identifier.derive('alice@example.com');
 * const bob = ristretto255_FROST.Identifier.derive('bob@example.com');
 * const carol = ristretto255_FROST.Identifier.derive('carol@example.com');
 * const deal = ristretto255_FROST.trustedDealer({ min: 2, max: 3 }, [alice, bob, carol]);
 * ```
 */
export declare const ristretto255_FROST: TRet<FROST>;
/**
 * Weird / bogus points, useful for debugging.
 * All 8 ed25519 points of 8-torsion subgroup can be generated from the point
 * T = `26e8958fc2b227b045c3f489f2ef98f0d5dfac05d3c63339b13802886d53fc05`.
 * The subgroup generated by `T` is `{ O, T, 2T, 3T, 4T, 5T, 6T, 7T }`; the
 * array below is that set, not the powers in that exact index order.
 * @example
 * Decode one known torsion point for debugging.
 *
 * ```ts
 * import { ED25519_TORSION_SUBGROUP, ed25519 } from '@noble/curves/ed25519.js';
 * const point = ed25519.Point.fromHex(ED25519_TORSION_SUBGROUP[1]);
 * ```
 */
export declare const ED25519_TORSION_SUBGROUP: readonly string[];
export {};
