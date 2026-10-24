/**
 * Post-Quantum Hybrid Cryptography
 *
 * The current implementation is flawed and likely redundant. We should offer
 * a small, generic API to compose hybrid schemes instead of reimplementing
 * protocol-specific logic (SSH, GPG, etc.) with ad hoc encodings.
 *
 * 1. Core Issues
 *    - sign/verify: implemented as two separate operations with different keys.
 *    - EC getSharedSecret: could be refactored into a proper KEM.
 *    - Multiple calls: keys, signatures, and shared secrets could be
 *      concatenated to reduce the number of API invocations.
 *    - Reinvention: most libraries add strange domain separations and
 *      encodings instead of simple byte concatenation.
 *
 * 2. API Goals
 *    - Provide primitives to build hybrids generically.
 *    - Avoid embedding SSH- or GPG-specific formats in the core API.
 *
 * 3. Edge Cases
 *    • Variable-length signatures:
 *      - DER-encoded (Weierstrass curves).
 *      - Falcon (unpadded).
 *      - Concatenation works only if length is fixed; otherwise a length
 *        prefix is required (but that breaks compatibility).
 *
 *    • getSharedSecret:
 *      - Default: non-KEM (authenticated ECDH).
 *      - KEM conversion: generate a random SK to remove implicit auth.
 *
 * 4. Common Pitfalls
 *    - Seed expansion:
 *      • Expanding a small seed into multiple keys reduces entropy.
 *      • API should allow identity mapping (no expansion).
 *
 *    - Skipping full point encoding:
 *      • Some omit the compression byte (parity) for WebCrypto compatibility.
 *      • Better: hash the raw secret; coordinate output is already non-uniform.
 *      • Some curves (e.g., X448) produce secrets that must be re-hashed to match
 *        symmetric-key lengths.
 *
 *    - Combiner inconsistencies:
 *      • Different domain separations and encodings across libraries.
 *      • Should live at the application layer, since key lengths vary.
 *
 * 5. Protocol Examples
 *    - SSH:
 *      • Concatenate keys.
 *      • Combiner: SHA-512.
 *
 *    - GPG:
 *      • Concatenate keys.
 *      • Combiner:
 *        SHA3-256(kemShare || ecdhShare || ciphertext || pubKey || algId || domSep || len(domSep))
 *
 *    - TLS:
 *      • Transcript-based derivation (HKDF).
 *
 * 6. Relevant Specs & Implementations
 *    - IETF Hybrid KEM drafts:
 *      • draft-irtf-cfrg-hybrid-kems
 *      • draft-connolly-cfrg-xwing-kem
 *      • draft-westerbaan-tls-xyber768d00
 *
 *    - PQC Libraries:
 *      • superdilithium (cyph/pqcrypto.js) – low adoption.
 *      • hybrid-pqc (DogeProtocol, quantumcoinproject) – complex encodings.
 *
 * 7. Signatures
 *    - Ed25519: fixed-size, easy to support.
 *    - Variable-size: introduces custom format requirements; best left to
 *      higher-level code.
 *
 * @module
 */
/*! noble-post-quantum - MIT License (c) 2024 Paul Miller (paulmillr.com) */
import { type EdDSA } from '@noble/curves/abstract/edwards.js';
import { type MontgomeryECDH } from '@noble/curves/abstract/montgomery.js';
import { type ECDSA } from '@noble/curves/abstract/weierstrass.js';
import { type CHash, type CHashXOF } from '@noble/hashes/utils.js';
import { type KEM, type Signer, type TArg, type TRet } from './utils.ts';
type CurveECDH = ECDSA | MontgomeryECDH;
type CurveSign = ECDSA | EdDSA;
/**
 * Wraps an ECDH-capable curve as a KEM.
 * Shared secrets stay in the wrapped curve's raw ECDH byte format with no built-in KDF.
 *
 * SECURITY: this is a low-level component adapter, not a standalone IND-CCA-secure KEM. It does
 * not bind the encapsulation or recipient public key into the secret, so distinct accepted point
 * encodings can produce the same output. Use it only inside a construction whose specified
 * combiner binds those values, or use a standardized DHKEM with labeled extract-and-expand.
 *
 * On SEC 1 / Weierstrass curves, that means the compressed shared-point body without the
 * 1-byte `0x02` / `0x03` prefix.
 * The X25519 path also leaves RFC 7748's optional all-zero shared-secret check to callers.
 * @param curve - Curve with `getSharedSecret`.
 * @param allowZeroKey - Legacy vector-matching toggle for Weierstrass keygen.
 * On Weierstrass curves this removes the usual post-reduction `+1` shift, changing seeded scalar
 * reduction from `[1, ORDER)` to direct reduction into `[0, ORDER)`. It does not make scalar zero
 * valid: an all-zero seed still derives scalar `0` and throws in `curve.getPublicKey(...)`.
 * Only supported on Weierstrass/ECDSA curves.
 * @returns KEM wrapper over the curve.
 * @throws If the curve does not expose `getSharedSecret`. {@link Error}
 * @example
 * Wrap an ECDH-capable curve as a generic KEM.
 * ```ts
 * import { x25519 } from '@noble/curves/ed25519.js';
 * import { _ecdhKem } from '@noble/post-quantum/hybrid.js';
 * const kem = _ecdhKem(x25519);
 * const publicKeyLen = kem.lengths.publicKey;
 * ```
 */
export declare function _ecdhKem(curve: CurveECDH, allowZeroKey?: boolean): TRet<KEM>;
/**
 * Wraps a curve signer as a generic `Signer`.
 * Signatures stay in the wrapped curve's native byte encoding.
 * This wrapper does not normalize or document which per-curve signing options are meaningful.
 * @param curve - Curve with `sign` and `verify`.
 * @param allowZeroKey - Legacy vector-matching toggle for Weierstrass keygen.
 * On Weierstrass curves this removes the usual post-reduction `+1` shift, changing seeded scalar
 * reduction from `[1, ORDER)` to direct reduction into `[0, ORDER)`. It does not make scalar zero
 * valid: an all-zero seed still derives scalar `0` and throws in `curve.getPublicKey(...)`.
 * Only supported on Weierstrass/ECDSA curves.
 * @returns Signer wrapper over the curve.
 * @throws If the curve does not expose `sign` and `verify`. {@link Error}
 * @example
 * Wrap a curve signer as a generic signer.
 * ```ts
 * import { ed25519 } from '@noble/curves/ed25519.js';
 * import { ecSigner } from '@noble/post-quantum/hybrid.js';
 * const signer = ecSigner(ed25519);
 * const sigLen = signer.lengths.signature;
 * ```
 */
export declare function ecSigner(curve: CurveSign, allowZeroKey?: boolean): TRet<Signer>;
/** Seed-expansion callback used by the hybrid combiners. */
export type ExpandSeed = (seed: TArg<Uint8Array>, len: number) => TRet<Uint8Array>;
type XOF = CHashXOF<any, {
    dkLen: number;
}>;
/**
 * Adapts an XOF into an `ExpandSeed` callback.
 * The returned callback interprets its second argument as an output byte length passed as `dkLen`.
 * @param xof - Extendable-output hash function.
 * @returns Seed expander using `dkLen`.
 * @example
 * Adapt an XOF into a seed expander.
 * ```ts
 * import { shake256 } from '@noble/hashes/sha3.js';
 * import { expandSeedXof } from '@noble/post-quantum/hybrid.js';
 * const expandSeed = expandSeedXof(shake256);
 * const seed = expandSeed(new Uint8Array([1]), 4);
 * ```
 */
export declare function expandSeedXof(xof: TArg<XOF>): TRet<ExpandSeed>;
/** Combines public keys, ciphertexts, and shared secrets into one shared secret. */
export type Combiner = (publicKeys: TArg<Uint8Array[]>, cipherTexts: TArg<Uint8Array[]>, sharedSecrets: TArg<Uint8Array[]>) => TRet<Uint8Array>;
/**
 * Combines multiple KEMs into one composite KEM.
 * @param realSeedLen - Positive input seed length expected by `expandSeed`, or `undefined` to use
 * the sum of component seed lengths. Callers remain responsible for choosing a security-appropriate
 * size.
 * @param realMsgLen - Positive shared-secret length returned by `combiner`, or `undefined` to use
 * the sum of component message lengths.
 * @param expandSeed - Seed expander used to derive per-KEM seeds.
 * @param combiner - Combines the per-KEM outputs into one shared secret.
 * @param kems - At least one KEM implementation. A construction advertised as hybrid normally
 * supplies two or more.
 * @returns Composite KEM.
 * @throws On wrong argument types. {@link TypeError}
 * @throws If there are no components or any required length resolves to zero. {@link RangeError}
 * @example
 * Combine multiple KEMs into one composite KEM.
 * ```ts
 * import { shake256 } from '@noble/hashes/sha3.js';
 * import { combineKEMS, expandSeedXof } from '@noble/post-quantum/hybrid.js';
 * import { ml_kem768 } from '@noble/post-quantum/ml-kem.js';
 * const hybrid = combineKEMS(
 *   32,
 *   32,
 *   expandSeedXof(shake256),
 *   (_pk, _ct, sharedSecrets) => sharedSecrets[0],
 *   ml_kem768,
 *   ml_kem768
 * );
 * const { publicKey } = hybrid.keygen();
 * ```
 */
export declare function combineKEMS(realSeedLen: number | undefined, // how much bytes expandSeed expects
realMsgLen: number | undefined, // how much bytes combiner returns
expandSeed: TArg<ExpandSeed>, combiner: TArg<Combiner>, ...kems: TArg<KEM[]>): TRet<KEM>;
/**
 * Combines multiple signers into one composite signer.
 * @param realSeedLen - Positive input seed length expected by `expandSeed`, or `undefined` to use
 * the sum of component seed lengths. Callers remain responsible for choosing a security-appropriate
 * size.
 * @param expandSeed - Seed expander used to derive per-signer seeds.
 * @param signers - At least one signer. A construction advertised as hybrid normally supplies two
 * or more.
 * @returns Composite signer.
 * @throws On wrong argument types. {@link TypeError}
 * @throws If there are no components or any required length resolves to zero. {@link RangeError}
 * @example
 * Combine multiple signers into one composite signer.
 * ```ts
 * import { shake256 } from '@noble/hashes/sha3.js';
 * import { combineSigners, expandSeedXof } from '@noble/post-quantum/hybrid.js';
 * import { ml_dsa44 } from '@noble/post-quantum/ml-dsa.js';
 * const hybrid = combineSigners(32, expandSeedXof(shake256), ml_dsa44, ml_dsa44);
 * const seed = new Uint8Array(hybrid.lengths.seed!).fill(1);
 * const { secretKey, publicKey } = hybrid.keygen(seed);
 * const msg = new TextEncoder().encode('hello noble');
 * const sig = hybrid.sign(msg, secretKey);
 * const isValid = hybrid.verify(sig, msg, publicKey);
 * ```
 */
export declare function combineSigners(realSeedLen: number | undefined, expandSeed: TArg<ExpandSeed>, ...signers: TArg<Signer[]>): TRet<Signer>;
/**
 * Builds a QSF hybrid KEM preset from a PQ KEM and an elliptic-curve KEM.
 * The combined shared-secret length follows `kdf.outputLen`; the built-in presets use 32-byte
 * SHA3-256 output, while custom `kdf` choices inherit their own digest size.
 * Its combiner hashes `ss0 || ss1 || ct1 || pk1 || label`, not the full
 * `(c1, c2, ek1, ek2)` example input shape from SP 800-227 equation (15).
 * Labels are encoded with `asciiToBytes()`, so non-ASCII labels are rejected.
 * @param label - Domain-separation label.
 * @param pqc - Post-quantum KEM.
 * @param curveKEM - Classical curve KEM.
 * @param xof - XOF used for seed expansion.
 * @param kdf - Hash used for the final combiner.
 * @returns Hybrid KEM.
 * @throws On wrong argument types. {@link TypeError}
 * @throws On wrong argument ranges or values. {@link RangeError}
 * @example
 * Build a QSF hybrid KEM preset from a PQ KEM and an elliptic-curve KEM.
 * ```ts
 * import { p256 } from '@noble/curves/nist.js';
 * import { sha3_256, shake256 } from '@noble/hashes/sha3.js';
 * import { QSF, _ecdhKem } from '@noble/post-quantum/hybrid.js';
 * import { ml_kem768 } from '@noble/post-quantum/ml-kem.js';
 * const kem = QSF('example', ml_kem768, _ecdhKem(p256, true), shake256, sha3_256);
 * const publicKeyLen = kem.lengths.publicKey;
 * ```
 */
export declare function QSF(label: string, pqc: TArg<KEM>, curveKEM: TArg<KEM>, xof: TArg<XOF>, kdf: CHash): TRet<KEM>;
/** QSF preset combining ML-KEM-768 with P-256. */
export declare const QSF_ml_kem768_p256: TRet<KEM>;
/** QSF preset combining ML-KEM-1024 with P-384. */
export declare const QSF_ml_kem1024_p384: TRet<KEM>;
/**
 * Builds the "KitchenSink" hybrid KEM combiner.
 * The current builder always derives a fixed 32-byte output,
 * regardless of the hash's native output size.
 * Its HKDF extract step uses implicit zero salt with IKM
 * `hybrid_prk || ss0 || ss1 || ct0 || pk0 || ct1 || pk1 || label`.
 * Its HKDF expand step fixes `info` to `len || 'shared_secret' || ''`.
 * Labels are encoded with `asciiToBytes()`, so non-ASCII labels are rejected.
 * @param label - Domain-separation label.
 * @param pqc - Post-quantum KEM.
 * @param curveKEM - Classical curve KEM.
 * @param xof - XOF used for seed expansion.
 * @param hash - Hash used for HKDF extraction and expansion.
 * @returns Hybrid KEM.
 * @throws On wrong argument types. {@link TypeError}
 * @throws On wrong argument ranges or values. {@link RangeError}
 * @example
 * Build the "KitchenSink" hybrid KEM combiner.
 * ```ts
 * import { sha256 } from '@noble/hashes/sha2.js';
 * import { shake256 } from '@noble/hashes/sha3.js';
 * import { createKitchenSink, _ecdhKem } from '@noble/post-quantum/hybrid.js';
 * import { ml_kem768 } from '@noble/post-quantum/ml-kem.js';
 * import { x25519 } from '@noble/curves/ed25519.js';
 * const kem = createKitchenSink('example', ml_kem768, _ecdhKem(x25519), shake256, sha256);
 * const publicKeyLen = kem.lengths.publicKey;
 * ```
 */
export declare function createKitchenSink(label: string, pqc: TArg<KEM>, curveKEM: TArg<KEM>, xof: TArg<XOF>, hash: CHash): TRet<KEM>;
/** KitchenSink preset combining ML-KEM-768 with X25519.
 * Caller randomness splits into 32 ML-KEM coins plus a 32-byte X25519 ephemeral-secret seed.
 */
export declare const KitchenSink_ml_kem768_x25519: TRet<KEM>;
/** X25519 + ML-KEM-768 hybrid preset.
 * Uses the hard-coded domain-separation label `\\.//^\\` and hashes only `ct1 || pk1`
 * from the X25519 side in addition to the two component shared secrets.
 */
export declare const ml_kem768_x25519: TRet<KEM>;
/** P-256 + ML-KEM-768 hybrid preset. */
export declare const ml_kem768_p256: TRet<KEM>;
/** P-384 + ML-KEM-1024 hybrid preset. */
export declare const ml_kem1024_p384: TRet<KEM>;
export {};
