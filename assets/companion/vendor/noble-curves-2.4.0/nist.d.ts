import { type FROST } from './abstract/frost.ts';
import { type H2CHasher } from './abstract/hash-to-curve.ts';
import { type OPRF } from './abstract/oprf.ts';
import { type ECDSA, type WeierstrassPointCons } from './abstract/weierstrass.ts';
import { type TRet } from './utils.ts';
/**
 * NIST P256 (aka secp256r1, prime256v1) curve, ECDSA and ECDH methods.
 * Hashes inputs with sha256 by default.
 *
 * @example
 * Generate one P-256 keypair, sign a message, and verify it.
 *
 * ```js
 * import { p256 } from '@noble/curves/nist.js';
 * const { secretKey, publicKey } = p256.keygen();
 * const recovered = p256.getPublicKey(secretKey);
 * const peer = p256.keygen();
 * const shared = p256.getSharedSecret(secretKey, peer.publicKey);
 * const msg = new TextEncoder().encode('hello noble');
 * const sig = p256.sign(msg, secretKey, { lowS: true, prehash: true });
 * const isValid = p256.verify(sig, msg, publicKey);
 * // const sigKeccak = p256.sign(keccak256(msg), secretKey, { prehash: false });
 * ```
 */
export declare const p256: ECDSA;
/**
 * Hashing / encoding to p256 points / field. RFC 9380 methods.
 * @example
 * Hash one message onto the P-256 curve.
 *
 * ```ts
 * const point = p256_hasher.hashToCurve(new TextEncoder().encode('hello noble'));
 * ```
 */
export declare const p256_hasher: H2CHasher<WeierstrassPointCons<bigint>>;
/**
 * p256 OPRF, defined in RFC 9497.
 * @example
 * Run one blind/evaluate/finalize OPRF round over P-256.
 *
 * ```ts
 * const input = new TextEncoder().encode('hello noble');
 * const keys = p256_oprf.oprf.generateKeyPair();
 * const blind = p256_oprf.oprf.blind(input);
 * const evaluated = p256_oprf.oprf.blindEvaluate(keys.secretKey, blind.blinded);
 * const output = p256_oprf.oprf.finalize(input, blind.blind, evaluated);
 * ```
 */
export declare const p256_oprf: TRet<OPRF>;
/**
 * FROST threshold signatures over p256. RFC 9591.
 * @example
 * Create one trusted-dealer package for 2-of-3 p256 signing.
 *
 * ```ts
 * const alice = p256_FROST.Identifier.derive('alice@example.com');
 * const bob = p256_FROST.Identifier.derive('bob@example.com');
 * const carol = p256_FROST.Identifier.derive('carol@example.com');
 * const deal = p256_FROST.trustedDealer({ min: 2, max: 3 }, [alice, bob, carol]);
 * ```
 */
export declare const p256_FROST: TRet<FROST>;
/**
 * NIST P384 (aka secp384r1) curve, ECDSA and ECDH methods. Hashes inputs with sha384 by default.
 * @example
 * Generate one P-384 keypair, sign a message, and verify it.
 *
 * ```ts
 * const { secretKey, publicKey } = p384.keygen();
 * const msg = new TextEncoder().encode('hello noble');
 * const sig = p384.sign(msg, secretKey);
 * const isValid = p384.verify(sig, msg, publicKey);
 * ```
 */
export declare const p384: ECDSA;
/**
 * Hashing / encoding to p384 points / field. RFC 9380 methods.
 * @example
 * Hash one message onto the P-384 curve.
 *
 * ```ts
 * const point = p384_hasher.hashToCurve(new TextEncoder().encode('hello noble'));
 * ```
 */
export declare const p384_hasher: H2CHasher<WeierstrassPointCons<bigint>>;
/**
 * p384 OPRF, defined in RFC 9497.
 * @example
 * Run one blind/evaluate/finalize OPRF round over P-384.
 *
 * ```ts
 * const input = new TextEncoder().encode('hello noble');
 * const keys = p384_oprf.oprf.generateKeyPair();
 * const blind = p384_oprf.oprf.blind(input);
 * const evaluated = p384_oprf.oprf.blindEvaluate(keys.secretKey, blind.blinded);
 * const output = p384_oprf.oprf.finalize(input, blind.blind, evaluated);
 * ```
 */
export declare const p384_oprf: TRet<OPRF>;
/**
 * NIST P521 (aka secp521r1) curve, ECDSA and ECDH methods. Hashes inputs with sha512 by default.
 * Deterministic `keygen(seed)` expects 99 seed bytes here because the generic scalar-derivation
 * helper uses `getMinHashLength(n)`, not the 66-byte canonical secret-key width.
 * @example
 * Generate one P-521 keypair, sign a message, and verify it.
 *
 * ```ts
 * const { secretKey, publicKey } = p521.keygen();
 * const msg = new TextEncoder().encode('hello noble');
 * const sig = p521.sign(msg, secretKey);
 * const isValid = p521.verify(sig, msg, publicKey);
 * ```
 */
export declare const p521: ECDSA;
/**
 * Hashing / encoding to p521 points / field. RFC 9380 methods.
 * @example
 * Hash one message onto the P-521 curve.
 *
 * ```ts
 * const point = p521_hasher.hashToCurve(new TextEncoder().encode('hello noble'));
 * ```
 */
export declare const p521_hasher: H2CHasher<WeierstrassPointCons<bigint>>;
/**
 * p521 OPRF, defined in RFC 9497.
 * @example
 * Run one blind/evaluate/finalize OPRF round over P-521.
 *
 * ```ts
 * const input = new TextEncoder().encode('hello noble');
 * const keys = p521_oprf.oprf.generateKeyPair();
 * const blind = p521_oprf.oprf.blind(input);
 * const evaluated = p521_oprf.oprf.blindEvaluate(keys.secretKey, blind.blinded);
 * const output = p521_oprf.oprf.finalize(input, blind.blind, evaluated);
 * ```
 */
export declare const p521_oprf: TRet<OPRF>;
