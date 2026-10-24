/**
 * Friendly wrapper over elliptic curves from built-in WebCrypto. Experimental: API may change.

# WebCrypto issues

## No way to get public keys

- Export of raw secret key is prohibited by spec:
  - https://w3c.github.io/webcrypto/#ecdsa-operations-export-key
    -> "If format is "raw":" -> "If the [[type]] internal slot of key is not "public",
       then throw an InvalidAccessError."
- Import of raw secret keys is prohibited by spec:
  - https://w3c.github.io/webcrypto/#ecdsa-operations-import-key
    -> "If format is "raw":" -> "If usages contains a value which is not "verify"
       then throw a SyntaxError."
- SPKI (Simple public-key infrastructure) is public-key-only
- PKCS8 is secret-key-only
- No way to get public key from secret key, but we convert to JWK and then
  create it manually, since a JWK secret key includes both private and public
  parts.
- Noble supports generating keys for both sign, verify & getSharedSecret,
  but JWK key includes usage, which forces us to patch it (non-JWK is ok)
- We have import/export for 'raw', but it doesn't work in Firefox / Safari

## Point encoding

- Raw export of public points returns uncompressed points,
  but this is implementation specific and not much we can do there.
- `getSharedSecret` differs for p256, p384, p521:
  Noble returns 33-byte output (y-parity + x coordinate),
  while in WebCrypto returns 32-byte output (x coordinate).
  This is intentional: noble keeps the full encoded shared point, and x-only
  callers can slice it down themselves.
- `getSharedSecret` identical for X25519, X448

## Availability

Node.js additionally supports ed448.
There seems no reasonable way to check for availability, other than actually calling methods.

 * @module
 */
/*! noble-curves - MIT License (c) 2022 Paul Miller (paulmillr.com) */
import { type TArg, type TRet } from './utils.ts';
/** Raw type */
declare const TYPE_RAW = "raw";
declare const TYPE_JWK = "jwk";
declare const TYPE_SPKI = "spki";
declare const TYPE_PKCS = "pkcs8";
/** Key serialization formats supported by the WebCrypto wrappers. */
export type WebCryptoFormat = typeof TYPE_RAW | typeof TYPE_JWK | typeof TYPE_SPKI | typeof TYPE_PKCS;
/** WebCrypto keys can be in raw, jwk, pkcs8/spki formats. Raw is internal and fragile. */
export type WebCryptoOpts = {
    /** Preferred secret-key serialization format. */
    formatSec?: WebCryptoFormat;
    /** Preferred public-key serialization format. */
    formatPub?: WebCryptoFormat;
};
declare function hexToBytesLocal(hex: string): TRet<Uint8Array>;
export declare const __TEST: {
    hexToBytesLocal: typeof hexToBytesLocal;
};
type JsonWebKey = {
    crv?: string;
    d?: string;
    kty?: string;
    x?: string;
    y?: string;
    [key: string]: unknown;
};
type Key = JsonWebKey | Uint8Array;
type WebCryptoBaseCurve = {
    name: string;
    isSupported(): Promise<boolean>;
    keygen(): TRet<Promise<{
        secretKey: Uint8Array;
        publicKey: Uint8Array;
    }>>;
    getPublicKey(secretKey: TArg<Key>, opts?: TArg<WebCryptoOpts>): TRet<Promise<Key>>;
    utils: {
        randomSecretKey: (format?: WebCryptoFormat) => TRet<Promise<Key>>;
        convertSecretKey: (key: TArg<Key>, inFormat?: WebCryptoFormat, outFormat?: WebCryptoFormat) => TRet<Promise<Key>>;
        convertPublicKey: (key: TArg<Key>, inFormat?: WebCryptoFormat, outFormat?: WebCryptoFormat) => TRet<Promise<Key>>;
    };
};
/** WebCrypto signing interface shared by ECDSA and EdDSA helpers. */
export type WebCryptoSigner = {
    /**
     * Sign one message with a WebCrypto-backed private key.
     * @param message - Message bytes to sign.
     * @param secretKey - Secret key in one supported format.
     * @param opts - Optional key-format overrides. See {@link WebCryptoOpts}.
     * @returns Signature bytes.
     */
    sign(message: TArg<Uint8Array>, secretKey: TArg<Key>, opts?: TArg<WebCryptoOpts>): TRet<Promise<Uint8Array>>;
    /**
     * Verify one signature with a WebCrypto-backed public key.
     * @param signature - Signature bytes.
     * @param message - Signed message bytes.
     * @param publicKey - Public key in one supported format.
     * @param opts - Optional key-format overrides. See {@link WebCryptoOpts}.
     * @returns `true` when the signature is valid.
     */
    verify(signature: TArg<Uint8Array>, message: TArg<Uint8Array>, publicKey: TArg<Key>, opts?: TArg<WebCryptoOpts>): Promise<boolean>;
};
/** WebCrypto ECDH interface for shared-secret derivation. */
export type WebCryptoECDH = {
    /**
     * Derive one shared secret from a local secret key and peer public key.
     * Short-Weierstrass wrappers return the raw x-coordinate here, not noble's parity-prefixed
     * shared-point encoding. Runtime also accepts alternate key formats through `opts`, even though
     * this public type is still narrowed to byte arrays.
     * @param secA - Local secret key in one supported format.
     * @param pubB - Peer public key in one supported format.
     * @param opts - Optional key-format overrides. See {@link WebCryptoOpts}.
     * @returns Shared secret bytes.
     */
    getSharedSecret(secA: TArg<Uint8Array>, pubB: TArg<Uint8Array>, opts?: TArg<WebCryptoOpts>): TRet<Promise<Uint8Array>>;
};
/** WebCrypto ECDSA interface with keygen, signing, and ECDH helpers. */
export type WebCryptoECDSA = WebCryptoBaseCurve & WebCryptoSigner & WebCryptoECDH;
/** WebCrypto EdDSA interface with keygen and signing helpers. */
export type WebCryptoEdDSA = WebCryptoBaseCurve & WebCryptoSigner;
/** WebCrypto Montgomery interface with keygen and ECDH helpers. */
export type WebCryptoMontgomery = WebCryptoBaseCurve & WebCryptoECDH;
/**
 * Friendly wrapper over built-in WebCrypto NIST P-256 (secp256r1).
 * Inherits the generic WebCrypto ECDSA caveats: `isSupported()` only probes the sign-side API, and
 * the conversion/signing helpers keep the shared `createKeyUtils(...)` / `createSigner(...)` quirks,
 * including raw WebCrypto ECDSA signatures without low-S normalization.
 * @example
 * Check support, then sign and verify once with WebCrypto P-256.
 *
 * ```ts
 * if (await p256.isSupported()) {
 *   const { secretKey, publicKey } = await p256.keygen();
 *   const msg = new TextEncoder().encode('hello noble');
 *   const sig = await p256.sign(msg, secretKey);
 *   const isValid = await p256.verify(sig, msg, publicKey);
 * }
 * ```
 */
export declare const p256: TRet<WebCryptoECDSA>;
/**
 * Friendly wrapper over built-in WebCrypto NIST P-384 (secp384r1).
 * Inherits the generic WebCrypto ECDSA caveats around support probing and key/signing conversion.
 * @example
 * Check support, then sign and verify once with WebCrypto P-384.
 *
 * ```ts
 * if (await p384.isSupported()) {
 *   const { secretKey, publicKey } = await p384.keygen();
 *   const msg = new TextEncoder().encode('hello noble');
 *   const sig = await p384.sign(msg, secretKey);
 *   const isValid = await p384.verify(sig, msg, publicKey);
 * }
 * ```
 */
export declare const p384: TRet<WebCryptoECDSA>;
/**
 * Friendly wrapper over built-in WebCrypto NIST P-521 (secp521r1).
 * Inherits the generic WebCrypto ECDSA caveats around support probing and key/signing conversion.
 * @example
 * Check support, then sign and verify once with WebCrypto P-521.
 *
 * ```ts
 * if (await p521.isSupported()) {
 *   const { secretKey, publicKey } = await p521.keygen();
 *   const msg = new TextEncoder().encode('hello noble');
 *   const sig = await p521.sign(msg, secretKey);
 *   const isValid = await p521.verify(sig, msg, publicKey);
 * }
 * ```
 */
export declare const p521: TRet<WebCryptoECDSA>;
/**
 * Friendly wrapper over built-in WebCrypto ed25519.
 * Inherits the generic WebCrypto EdDSA caveats around JWK conversion metadata and extractability.
 * @example
 * Check support, then sign and verify once with WebCrypto Ed25519.
 *
 * ```ts
 * if (await ed25519.isSupported()) {
 *   const { secretKey, publicKey } = await ed25519.keygen();
 *   const msg = new TextEncoder().encode('hello noble');
 *   const sig = await ed25519.sign(msg, secretKey);
 *   const isValid = await ed25519.verify(sig, msg, publicKey);
 * }
 * ```
 */
export declare const ed25519: TRet<WebCryptoEdDSA>;
/**
 * Friendly wrapper over built-in WebCrypto ed448.
 * Inherits the generic WebCrypto EdDSA caveats around JWK conversion metadata and extractability.
 * @example
 * Check support, then sign and verify once with WebCrypto Ed448.
 *
 * ```ts
 * if (await ed448.isSupported()) {
 *   const { secretKey, publicKey } = await ed448.keygen();
 *   const msg = new TextEncoder().encode('hello noble');
 *   const sig = await ed448.sign(msg, secretKey);
 *   const isValid = await ed448.verify(sig, msg, publicKey);
 * }
 * ```
 */
export declare const ed448: TRet<WebCryptoEdDSA>;
/**
 * Friendly wrapper over built-in WebCrypto x25519 (ECDH over Curve25519).
 * Inherits the generic WebCrypto Montgomery caveat that runtime accepts more key formats than the
 * narrow public `Uint8Array` argument types suggest.
 * @example
 * Check support, then derive one shared secret with WebCrypto X25519.
 *
 * ```ts
 * if (await x25519.isSupported()) {
 *   const alice = await x25519.keygen();
 *   const bob = await x25519.keygen();
 *   const shared = await x25519.getSharedSecret(alice.secretKey, bob.publicKey);
 * }
 * ```
 */
export declare const x25519: TRet<WebCryptoMontgomery>;
/**
 * Friendly wrapper over built-in WebCrypto x448 (ECDH over Curve448).
 * Inherits the generic WebCrypto Montgomery caveat that runtime accepts more key formats than the
 * narrow public `Uint8Array` argument types suggest.
 * @example
 * Check support, then derive one shared secret with WebCrypto X448.
 *
 * ```ts
 * if (await x448.isSupported()) {
 *   const alice = await x448.keygen();
 *   const bob = await x448.keygen();
 *   const shared = await x448.getSharedSecret(alice.secretKey, bob.publicKey);
 * }
 * ```
 */
export declare const x448: TRet<WebCryptoMontgomery>;
export {};
