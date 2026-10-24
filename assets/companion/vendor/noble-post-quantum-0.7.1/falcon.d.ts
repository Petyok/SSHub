import { type CryptoKeys, type Signer, type SigOpts, type TRet, type VerOpts } from './utils.ts';
type FalconRandom = (bytesLength?: number) => TRet<Uint8Array>;
type FalconSigOpts = SigOpts & {
    random?: FalconRandom;
};
/** Falcon attached-signature API. */
export type FalconAttached = CryptoKeys & {
    /** Key lengths plus the 48-byte sampler-seed hook for signing. */
    lengths: CryptoKeys['lengths'] & {
        signRand?: number;
    };
    /**
     * Signs a message and appends it to the returned attached signature.
     * @param msg Message bytes to sign.
     * @param secretKey Falcon secret key bytes.
     * @param opts Optional Falcon signing options.
     * @returns Attached signature containing both the message and signature.
     */
    seal(msg: Uint8Array, secretKey: Uint8Array, opts?: FalconSigOpts): Uint8Array;
    /**
     * Verifies an attached signature and returns the embedded message.
     * @param sig Attached Falcon signature bytes.
     * @param publicKey Falcon public key bytes.
     * @param opts Optional verification options.
     * @returns Fresh message bytes that do not alias either input when the signature is valid.
     */
    open(sig: Uint8Array, publicKey: Uint8Array, opts?: VerOpts): Uint8Array;
};
/** Falcon detached-signature API with an attached-signature helper. */
export type Falcon = Signer & {
    /** Attached-signature helper for the same Falcon parameter set. */
    attached: FalconAttached;
};
/**
 * Falcon-512 detached-signature API with the attached helper exposed as `.attached`.
 * @example
 * Generate a Falcon-512 keypair and verify one detached signature.
 * ```ts
 * const { secretKey, publicKey } = falcon512.keygen();
 * const msg = new Uint8Array([1, 2, 3]);
 * const sig = falcon512.sign(msg, secretKey);
 * falcon512.verify(sig, msg, publicKey);
 * ```
 */
export declare const falcon512: TRet<Falcon>;
/**
 * Falcon-512 padded detached-signature API with the attached helper exposed as `.attached`.
 * @example
 * Generate a Falcon-512 padded keypair and verify one detached signature.
 * ```ts
 * const { secretKey, publicKey } = falcon512padded.keygen();
 * const msg = new Uint8Array([1, 2, 3]);
 * const sig = falcon512padded.sign(msg, secretKey);
 * falcon512padded.verify(sig, msg, publicKey);
 * ```
 */
export declare const falcon512padded: TRet<Falcon>;
/**
 * Falcon-1024 detached-signature API with the attached helper exposed as `.attached`.
 * @example
 * Generate a Falcon-1024 keypair and verify one detached signature.
 * ```ts
 * const { secretKey, publicKey } = falcon1024.keygen();
 * const msg = new Uint8Array([1, 2, 3]);
 * const sig = falcon1024.sign(msg, secretKey);
 * falcon1024.verify(sig, msg, publicKey);
 * ```
 */
export declare const falcon1024: TRet<Falcon>;
/**
 * Falcon-1024 padded detached-signature API with the attached helper exposed as `.attached`.
 * @example
 * Generate a Falcon-1024 padded keypair and verify one detached signature.
 * ```ts
 * const { secretKey, publicKey } = falcon1024padded.keygen();
 * const msg = new Uint8Array([1, 2, 3]);
 * const sig = falcon1024padded.sign(msg, secretKey);
 * falcon1024padded.verify(sig, msg, publicKey);
 * ```
 */
export declare const falcon1024padded: TRet<Falcon>;
export declare const __tests: any;
export {};
