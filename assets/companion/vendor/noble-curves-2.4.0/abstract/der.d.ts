/**
 * ASN.1 DER (Distinguished Encoding Rules) helpers for ECDSA signatures.
 * Only implements the tiny subset needed for `SEQUENCE(INTEGER r, INTEGER s)`.
 * @module
 */
/*! noble-curves - MIT License (c) 2022 Paul Miller (paulmillr.com) */
import { type TArg, type TRet } from '../utils.ts';
/**
 * @param m - Error message.
 * @example
 * Throw a DER-specific error when signature parsing encounters invalid bytes.
 *
 * ```ts
 * new DERErr('bad der');
 * ```
 */
export declare class DERErr extends Error {
    constructor(m?: string);
}
/** DER helper namespace used by ECDSA signature parsing and encoding. */
export type IDER = {
    /**
     * DER-specific error constructor.
     * @param m - Error message.
     * @returns DER-specific error instance.
     */
    Err: typeof DERErr;
    /** Low-level tag-length-value helpers used by DER encoders. */
    _tlv: {
        /**
         * Encode one TLV record.
         * @param tag - ASN.1 tag byte.
         * @param data - Hex-encoded value payload.
         * @returns Encoded TLV string.
         */
        encode: (tag: number, data: string) => string;
        /**
         * Decode one TLV record and return the value plus leftover bytes.
         * @param tag - Expected ASN.1 tag byte.
         * @param data - Remaining DER bytes.
         * @returns Parsed value plus leftover bytes.
         */
        decode(tag: number, data: TArg<Uint8Array>): TRet<{
            v: Uint8Array;
            l: Uint8Array;
        }>;
    };
    /** Positive-integer DER helpers used by ECDSA signature encoding. */
    _int: {
        /**
         * Encode one positive bigint as a DER INTEGER.
         * @param num - Positive integer to encode.
         * @returns Encoded DER INTEGER.
         */
        encode(num: bigint): string;
        /**
         * Decode one DER INTEGER into a bigint.
         * @param data - DER INTEGER bytes.
         * @returns Decoded bigint.
         */
        decode(data: TArg<Uint8Array>): bigint;
    };
    /**
     * Parse a DER signature into `{ r, s }`.
     * @param bytes - DER signature bytes.
     * @param maxScalarBytes - Optional maximum encoded byte length of each INTEGER.
     * @returns Parsed signature components.
     */
    toSig(bytes: TArg<Uint8Array>, maxScalarBytes?: number): {
        r: bigint;
        s: bigint;
    };
    /**
     * Encode `{ r, s }` as a DER signature.
     * @param sig - Signature components.
     * @returns DER-encoded signature hex.
     */
    hexFromSig(sig: {
        r: bigint;
        s: bigint;
    }): string;
};
/**
 * ASN.1 DER encoding utilities. ASN is very complex & fragile. Format:
 *
 *     [0x30 (SEQUENCE), bytelength, 0x02 (INTEGER), intLength, R, 0x02 (INTEGER), intLength, S]
 *
 * Docs: {@link https://letsencrypt.org/docs/a-warm-welcome-to-asn1-and-der/ | Let's Encrypt ASN.1 guide} and
 * {@link https://luca.ntop.org/Teaching/Appunti/asn1.html | Luca Deri's ASN.1 notes}.
 * @example
 * ASN.1 DER encoding utilities.
 *
 * ```ts
 * const der = DER.hexFromSig({ r: 1n, s: 2n });
 * ```
 */
export declare const DER: IDER;
