import { type EdDSA, type EdwardsPoint } from './abstract/edwards.ts';
import { type ECDSA } from './abstract/weierstrass.ts';
import { type TArg } from './utils.ts';
/**
 * Generic EdDSA-over-Jubjub convenience wrapper with `sha512`.
 * This is not the Zcash RedJubjub / Sapling signature scheme.
 * @example
 * Generate one Jubjub keypair, sign a message, and verify it.
 *
 * ```ts
 * const { secretKey, publicKey } = jubjub.keygen();
 * const msg = new TextEncoder().encode('hello noble');
 * const sig = jubjub.sign(msg, secretKey);
 * const isValid = jubjub.verify(sig, msg, publicKey);
 * ```
 */
export declare const jubjub: EdDSA;
/**
 * Curve over scalar field of bn254. babyjubjub Fp = bn254 n
 * This is a working generic EdDSA-over-BabyJubJub wrapper that uses `blake512` for the 64-byte
 * secret expansion required by the shared EdDSA helper.
 * It is not the BabyJubJub stack used by iden3/circomlib, `babyjubjub-rs`, or
 * `@zk-kit/eddsa-poseidon`: those pair the subgroup base B/B8 with Blake-style secret expansion
 * plus dedicated Poseidon / MiMC / Pedersen transcript hashing. This wrapper stays generic and is
 * not meant as an interoperability target for those BabyJubJub signing stacks.
 * @example
 * Access the BabyJubJub base point and round-trip it through the point codec.
 *
 * ```ts
 * import { babyjubjub } from '@noble/curves/misc.js';
 * const base = babyjubjub.Point.BASE;
 * const encoded = base.toBytes();
 * const decoded = babyjubjub.Point.fromBytes(encoded);
 * ```
 */
export declare const babyjubjub: EdDSA;
/**
 * @param tag - Hash input.
 * @param personalization - BLAKE2 personalization bytes.
 * @returns Prime-order Jubjub point.
 * @throws If the digest does not decode to a Jubjub point, or if the
 *   cofactor-cleared point has small order. {@link Error}
 * @example
 * Hash a tag into a prime-order Jubjub point.
 *
 * ```ts
 * import { jubjub_groupHash } from '@noble/curves/misc.js';
 * import { asciiToBytes } from '@noble/curves/utils.js';
 * const tag = Uint8Array.of(2);
 * const personalization = asciiToBytes('Zcash_G_');
 * const point = jubjub_groupHash(tag, personalization);
 * ```
 */
export declare function jubjub_groupHash(tag: TArg<Uint8Array>, personalization: TArg<Uint8Array>): EdwardsPoint;
/**
 * No secret data is leaked here at all.
 * It operates over public data.
 * @param m - Message prefix.
 * @param personalization - 8-byte BLAKE2 personalization bytes.
 * @returns First non-zero group hash.
 * @throws If the personalization is invalid, or if no non-zero Jubjub group
 *   hash can be found. {@link Error}
 * @example
 * Derive the first non-zero Jubjub group hash for one personalization tag.
 *
 * ```ts
 * import { jubjub_findGroupHash } from '@noble/curves/misc.js';
 * import { asciiToBytes } from '@noble/curves/utils.js';
 * const msg = Uint8Array.of();
 * const personalization = asciiToBytes('Zcash_G_');
 * const point = jubjub_findGroupHash(msg, personalization);
 * ```
 */
export declare function jubjub_findGroupHash(m: TArg<Uint8Array>, personalization: TArg<Uint8Array>): EdwardsPoint;
/**
 * Brainpool P256r1 with sha256, from RFC 5639.
 * @example
 * Generate one Brainpool P256r1 keypair, sign a message, and verify it.
 *
 * ```ts
 * const { secretKey, publicKey } = brainpoolP256r1.keygen();
 * const msg = new TextEncoder().encode('hello noble');
 * const sig = brainpoolP256r1.sign(msg, secretKey);
 * const isValid = brainpoolP256r1.verify(sig, msg, publicKey);
 * ```
 */
export declare const brainpoolP256r1: ECDSA;
/**
 * Brainpool P384r1 with sha384, from RFC 5639.
 * @example
 * Generate one Brainpool P384r1 keypair, sign a message, and verify it.
 *
 * ```ts
 * const { secretKey, publicKey } = brainpoolP384r1.keygen();
 * const msg = new TextEncoder().encode('hello noble');
 * const sig = brainpoolP384r1.sign(msg, secretKey);
 * const isValid = brainpoolP384r1.verify(sig, msg, publicKey);
 * ```
 */
export declare const brainpoolP384r1: ECDSA;
/**
 * Brainpool P512r1 with sha512, from RFC 5639.
 * @example
 * Generate one Brainpool P512r1 keypair, sign a message, and verify it.
 *
 * ```ts
 * const { secretKey, publicKey } = brainpoolP512r1.keygen();
 * const msg = new TextEncoder().encode('hello noble');
 * const sig = brainpoolP512r1.sign(msg, secretKey);
 * const isValid = brainpoolP512r1.verify(sig, msg, publicKey);
 * ```
 */
export declare const brainpoolP512r1: ECDSA;
