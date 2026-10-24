import { type CurveLengths } from './abstract/curve.ts';
import { type FROST, type FrostPublic, type FrostSecret } from './abstract/frost.ts';
import { type H2CHasher } from './abstract/hash-to-curve.ts';
import { type ECDSA, type WeierstrassPoint as PointType, type WeierstrassPointCons } from './abstract/weierstrass.ts';
import { type TArg, type TRet } from './utils.ts';
/**
 * secp256k1 curve: ECDSA and ECDH methods.
 *
 * Uses sha256 to hash messages. To use a different hash,
 * pass `{ prehash: false }` to sign / verify.
 *
 * @example
 * Generate one secp256k1 keypair, sign a message, and verify it.
 *
 * ```js
 * import { secp256k1 } from '@noble/curves/secp256k1.js';
 * const { secretKey, publicKey } = secp256k1.keygen();
 * // const publicKey = secp256k1.getPublicKey(secretKey);
 * const msg = new TextEncoder().encode('hello noble');
 * const sig = secp256k1.sign(msg, secretKey);
 * const isValid = secp256k1.verify(sig, msg, publicKey);
 * // const sigKeccak = secp256k1.sign(keccak256(msg), secretKey, { prehash: false });
 * ```
 */
export declare const secp256k1: ECDSA;
declare function taggedHash(tag: string, ...messages: TArg<Uint8Array[]>): TRet<Uint8Array>;
/**
 * lift_x from BIP340. Convert 32-byte x coordinate to elliptic curve point.
 * @returns valid point checked for being on-curve
 */
declare function lift_x(x: bigint): PointType<bigint>;
/** Schnorr public key is just `x` coordinate of Point as per BIP340. */
declare function schnorrGetPublicKey(secretKey: TArg<Uint8Array>): TRet<Uint8Array>;
/**
 * Creates Schnorr signature as per BIP340. Verifies itself before returning anything.
 * `auxRand` is optional and is not the sole source of `k` generation: bad CSPRNG output will not
 * be catastrophic, but BIP-340 still recommends fresh auxiliary randomness when available to harden
 * deterministic signing against side-channel and fault-injection attacks.
 */
declare function schnorrSign(message: TArg<Uint8Array>, secretKey: TArg<Uint8Array>, auxRand?: TArg<Uint8Array>): TRet<Uint8Array>;
/**
 * Verifies Schnorr signature.
 * Will swallow errors & return false except for initial type validation of arguments.
 */
declare function schnorrVerify(signature: TArg<Uint8Array>, message: TArg<Uint8Array>, publicKey: TArg<Uint8Array>): boolean;
export declare const __TEST: {
    lift_x: typeof lift_x;
    frostTweakPublic: typeof frostTweakPublic;
    frostTweakSecret: typeof frostTweakSecret;
};
/** Schnorr-specific secp256k1 API from BIP340. */
export type SecpSchnorr = {
    /**
     * Generate one Schnorr secret/public keypair.
     * @param seed - Optional seed for deterministic testing or custom randomness.
     * @returns Fresh secret/public keypair.
     */
    keygen: (seed?: TArg<Uint8Array>) => {
        secretKey: TRet<Uint8Array>;
        publicKey: TRet<Uint8Array>;
    };
    /**
     * Derive the x-only public key from a secret key.
     * @param secretKey - Secret key bytes.
     * @returns X-only public key bytes.
     */
    getPublicKey: typeof schnorrGetPublicKey;
    /**
     * Create one BIP340 Schnorr signature.
     * @param message - Message bytes to sign.
     * @param secretKey - Secret key bytes.
     * @param auxRand - Optional auxiliary randomness.
     * @returns Compact Schnorr signature bytes.
     */
    sign: typeof schnorrSign;
    /**
     * Verify one BIP340 Schnorr signature.
     * @param signature - Compact signature bytes.
     * @param message - Signed message bytes.
     * @param publicKey - X-only public key bytes.
     * @returns `true` when the signature is valid.
     */
    verify: typeof schnorrVerify;
    /** Underlying secp256k1 point constructor. */
    Point: WeierstrassPointCons<bigint>;
    /** Helper utilities for Schnorr-specific key handling and tagged hashing. */
    utils: {
        /** Generate one Schnorr secret key. */
        randomSecretKey: (seed?: TArg<Uint8Array>) => TRet<Uint8Array>;
        /** Convert one point into its x-only BIP340 byte encoding. */
        pointToBytes: (point: TArg<PointType<bigint>>) => TRet<Uint8Array>;
        /** Lift one x coordinate into the unique even-Y point. */
        lift_x: typeof lift_x;
        /** Compute a BIP340 tagged hash. */
        taggedHash: typeof taggedHash;
    };
    /** Public byte lengths for keys, signatures, and seeds. */
    lengths: CurveLengths;
};
/**
 * Schnorr signatures over secp256k1.
 * See {@link https://github.com/bitcoin/bips/blob/master/bip-0340.mediawiki | BIP 340}.
 * @example
 * Generate one BIP340 Schnorr keypair, sign a message, and verify it.
 *
 * ```js
 * import { schnorr } from '@noble/curves/secp256k1.js';
 * const { secretKey, publicKey } = schnorr.keygen();
 * // const publicKey = schnorr.getPublicKey(secretKey);
 * const msg = new TextEncoder().encode('hello');
 * const sig = schnorr.sign(msg, secretKey);
 * const isValid = schnorr.verify(sig, msg, publicKey);
 * ```
 */
export declare const schnorr: SecpSchnorr;
/**
 * Hashing / encoding to secp256k1 points / field. RFC 9380 methods.
 * @example
 * Hash one message onto secp256k1.
 *
 * ```ts
 * const point = secp256k1_hasher.hashToCurve(new TextEncoder().encode('hello noble'));
 * ```
 */
export declare const secp256k1_hasher: H2CHasher<WeierstrassPointCons<bigint>>;
/**
 * FROST threshold signatures over secp256k1. RFC 9591.
 * @example
 * Create one trusted-dealer package for 2-of-3 secp256k1 signing.
 *
 * ```ts
 * const alice = secp256k1_FROST.Identifier.derive('alice@example.com');
 * const bob = secp256k1_FROST.Identifier.derive('bob@example.com');
 * const carol = secp256k1_FROST.Identifier.derive('carol@example.com');
 * const deal = secp256k1_FROST.trustedDealer({ min: 2, max: 3 }, [alice, bob, carol]);
 * ```
 */
export declare const secp256k1_FROST: TRet<FROST>;
declare function frostTweakSecret(s: TArg<FrostSecret>, pub: TArg<FrostPublic>, merkleRoot?: TArg<Uint8Array>): TRet<FrostSecret>;
declare function frostTweakPublic(pub: TArg<FrostPublic>, merkleRoot?: TArg<Uint8Array>): TRet<FrostPublic>;
/**
 * FROST threshold signatures over secp256k1-schnorr-taproot. RFC 9591.
 * DKG outputs are auto-tweaked with the empty Taproot merkle root for compatibility, while
 * `trustedDealer()` outputs stay untweaked unless callers apply the Taproot tweak themselves.
 * @example
 * Create one trusted-dealer package for Taproot-compatible FROST signing.
 *
 * ```ts
 * const alice = schnorr_FROST.Identifier.derive('alice@example.com');
 * const bob = schnorr_FROST.Identifier.derive('bob@example.com');
 * const carol = schnorr_FROST.Identifier.derive('carol@example.com');
 * const deal = schnorr_FROST.trustedDealer({ min: 2, max: 3 }, [alice, bob, carol]);
 * ```
 */
export declare const schnorr_FROST: TRet<FROST>;
export {};
