import { type KEM, type TRet } from './utils.ts';
/** FIPS 203: 7. Parameter Sets */
/** Public ML-KEM parameter-set description. */
export type KEMParam = {
    /** Polynomial size. */
    N: number;
    /** Module rank. */
    K: number;
    /** Prime modulus. */
    Q: number;
    /** CBD parameter used for secret-key noise. */
    ETA1: number;
    /** CBD parameter used for error noise. */
    ETA2: number;
    /** Compression width for the `u` vector. */
    du: number;
    /** Compression width for the `v` polynomial. */
    dv: number;
    /** Required strength of the randomness source in bits. */
    RBGstrength: number;
};
/** Internal params of ML-KEM versions */
/** Built-in ML-KEM parameter presets keyed by the public export names
 * `ml_kem512` / `ml_kem768` / `ml_kem1024`.
 * `RBGstrength` is Table 2's required randomness-source strength in bits,
 * not a generic security label.
 */
export declare const PARAMS: Record<string, KEMParam>;
/**
 * Prepared (pre-expanded) ML-KEM public key. Experimental prototype.
 * Caches only public data: packed ek, the expanded matrix Â, decoded t̂ and H(ek). No secret
 * material is retained between calls; secret keys passed to `decapsulate` are decoded and wiped
 * per call, exactly like the one-shot API. `clean()` wipes the expanded Â/t̂ cache; the packed
 * public key and H(ek) are public and are not wiped. The object must not be used afterwards.
 */
export type KEMPrepared = {
    /**
     * Detached copy of the source public key. Treat as read-only while the prepared object is in use.
     * Callers may wipe it after final use; any mutation invalidates subsequent operations.
     */
    publicKey: Uint8Array;
    /** Same as `KEM.encapsulate`, minus per-call ek re-validation and Â re-expansion. */
    encapsulate: (msg?: Uint8Array) => {
        cipherText: Uint8Array;
        sharedSecret: Uint8Array;
    };
    /**
     * Same as `KEM.decapsulate`; throws if `secretKey` does not embed this public key.
     * The embedded-ek byte comparison plus stored-hash comparison is equivalent to the
     * FIPS 203 §7.3 hash input check.
     */
    decapsulate: (cipherText: Uint8Array, secretKey: Uint8Array) => Uint8Array;
    /** Wipe cached (public) data. */
    clean: () => void;
};
/** KEM with prepared-key support. */
export type MLKEM = KEM & {
    prepare: (publicKey: Uint8Array) => KEMPrepared;
};
/**
 * ML-KEM-512: Table 2 row `k=2, η1=3, η2=2, du=10, dv=4`; Table 3 sizes `800/1632/768/32`.
 * The ASD lifecycle note here is external policy guidance, not a FIPS 203 requirement.
 * @example
 * Generate deterministic ML-KEM-512 keys, encapsulate a shared secret, and decapsulate it.
 * ```ts
 * import { ml_kem512 } from '@noble/post-quantum/ml-kem.js';
 * const seed = new Uint8Array(ml_kem512.lengths.seed!);
 * const { secretKey, publicKey } = ml_kem512.keygen(seed);
 * const msg = new Uint8Array(ml_kem512.lengths.msgRand!);
 * const { cipherText, sharedSecret } = ml_kem512.encapsulate(publicKey, msg);
 * const recovered = ml_kem512.decapsulate(cipherText, secretKey);
 * const publicKey2 = ml_kem512.getPublicKey(secretKey);
 * ```
 */
export declare const ml_kem512: TRet<MLKEM>;
/**
 * ML-KEM-768: Table 2 row `k=3, η1=2, η2=2, du=10, dv=4`; Table 3 sizes `1184/2400/1088/32`.
 * The ASD lifecycle note here is external policy guidance, not a FIPS 203 requirement.
 */
export declare const ml_kem768: TRet<MLKEM>;
/**
 * ML-KEM-1024: Table 2 row `k=4, η1=2, η2=2, du=11, dv=5`; Table 3 sizes `1568/3168/1568/32`.
 * The ASD lifecycle note here is external policy guidance, not a FIPS 203 requirement.
 */
export declare const ml_kem1024: TRet<MLKEM>;
export declare const __tests: any;
