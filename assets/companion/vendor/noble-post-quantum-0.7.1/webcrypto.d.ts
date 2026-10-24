/**
 * Friendly async wrappers over ML-KEM and ML-KEM-768 + X25519 from built-in WebCrypto.
 * Private keys use the same raw seed accepted by the synchronous implementations' `keygen(seed)`;
 * they are not expanded decapsulation keys.
 *
 * # WebCrypto quirks
 *
 * - The algorithms are experimental: a runtime can expose `encapsulateBits` and friends while
 *   implementing none of them, so support is probed with a full round-trip in `isSupported()`.
 * - `MLKEM768-X25519` accepts `raw-seed` on import, but has no `raw-seed` / `raw-public` export.
 *   Its key bytes are read out of the JWK `priv` / `pub` members instead.
 * - base64url is hand-rolled: scure-base's `base64urlnopad` would do, but this module must not add
 *   dependencies, and importing the synchronous implementations for four byte lengths would pull
 *   the whole lattice math into a WebCrypto-only entrypoint.
 * @module
 */
/*! noble-post-quantum - MIT License (c) 2024 Paul Miller (paulmillr.com) */
import { type TArg, type TRet } from './utils.ts';
type MLKEMName = 'ML-KEM-512' | 'ML-KEM-768' | 'ML-KEM-1024';
/** Byte lengths for a WebCrypto wrapper's serialized keys and ciphertexts. */
type KEMLengths = {
    /** Deterministic key-generation seed length. */
    seed: number;
    /**
     * Raw seed private-key length. Note this is the *seed*, not the expanded decapsulation key:
     * for ML-KEM the synchronous `lengths.secretKey` is much larger (1632 / 2400 / 3168 bytes), so
     * these private keys only fit the synchronous `keygen(seed)`, never its `decapsulate(ct, sk)`.
     */
    secretKey: number;
    /** Serialized public-key length. */
    publicKey: number;
    /** Encapsulated ciphertext length. */
    cipherText: number;
};
/** Async KEM interface backed by the current runtime's WebCrypto implementation. */
export type WebCryptoKEM = {
    /** WebCrypto algorithm name passed to `crypto.subtle`. */
    webCryptoName: string;
    /** Byte lengths for this WebCrypto wrapper's serialized keys and ciphertexts. */
    lengths: KEMLengths;
    /**
     * Checks whether the runtime implements the complete WebCrypto surface used by this wrapper.
     * Probes with a real key generation and encapsulation round-trip, and memoizes the result.
     * @returns Whether key generation, serialization, encapsulation, and decapsulation are supported.
     */
    isSupported(): Promise<boolean>;
    /**
     * Generates a KEM key pair.
     * @param seed - Optional raw seed for deterministic key generation.
     * @returns Raw seed private key and serialized public key.
     */
    keygen(seed?: TArg<Uint8Array>): TRet<Promise<{
        secretKey: Uint8Array;
        publicKey: Uint8Array;
    }>>;
    /**
     * Derives a serialized public key from a raw seed private key.
     * @param secretKey - Raw seed private key.
     * @returns Serialized public key.
     */
    getPublicKey(secretKey: TArg<Uint8Array>): TRet<Promise<Uint8Array>>;
    /**
     * Encapsulates a new random shared secret to a serialized public key.
     * @param publicKey - Recipient public key.
     * @returns Ciphertext and 32-byte shared secret.
     */
    encapsulate(publicKey: TArg<Uint8Array>): TRet<Promise<{
        cipherText: Uint8Array;
        sharedSecret: Uint8Array;
    }>>;
    /**
     * Decapsulates a ciphertext with a raw seed private key.
     * @param cipherText - Encapsulated ciphertext bytes.
     * @param secretKey - Private key in WebCrypto `raw-seed` format.
     * @returns Decapsulated 32-byte shared secret.
     */
    decapsulate(cipherText: TArg<Uint8Array>, secretKey: TArg<Uint8Array>): TRet<Promise<Uint8Array>>;
};
/** Async ML-KEM interface backed by the current runtime's WebCrypto implementation. */
export type WebCryptoMLKEM = WebCryptoKEM & {
    webCryptoName: MLKEMName;
};
/** WebCrypto ML-KEM-512 wrapper. */
export declare const ml_kem512: TRet<WebCryptoMLKEM>;
/** WebCrypto ML-KEM-768 wrapper. */
export declare const ml_kem768: TRet<WebCryptoMLKEM>;
/** WebCrypto ML-KEM-1024 wrapper. */
export declare const ml_kem1024: TRet<WebCryptoMLKEM>;
/** WebCrypto ML-KEM-768 + X25519 (X-Wing) wrapper. */
export declare const ml_kem768_x25519: TRet<WebCryptoKEM>;
export {};
