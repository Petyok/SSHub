/**
 * GHash from AES-GCM and its little-endian "mirror image" Polyval from AES-SIV.
 *
 * Implemented in terms of GHash with conversion function for keys
 * GCM GHASH from
 * {@link https://nvlpubs.nist.gov/nistpubs/Legacy/SP/nistspecialpublication800-38d.pdf | NIST SP800-38d},
 * SIV from
 * {@link https://www.rfc-editor.org/rfc/rfc8452 | RFC 8452}.
 *
 * GHASH   modulo: x^128 + x^7   + x^2   + x     + 1
 * POLYVAL modulo: x^128 + x^127 + x^126 + x^121 + 1
 *
 * @module
 */
import { type CMac, type IHash2, type TArg, type TRet } from './utils.ts';
/**
 * `mulX_GHASH(ByteReverse(H))` from RFC 8452 Appendix A.
 * @param k mutated in place
 */
export declare function _toGHASHKey(k: TArg<Uint8Array>): TRet<Uint8Array>;
type Value = {
    s0: number;
    s1: number;
    s2: number;
    s3: number;
};
/**
 * Incremental GHASH state for AES-GCM.
 * @param key - 16-byte GHASH key.
 * @param expectedLength - Expected message length for table sizing.
 * Chunking is segment-based, not hash-streaming: every `update()` call is zero-padded
 * to the next 16-byte boundary before it is absorbed. This matches the internal AES/GCM
 * use where AAD, payload, and length block are separate padded segments.
 * @example
 * Feeds one ciphertext block into an incremental GHASH state with a fresh hash key.
 *
 * ```ts
 * import { GHASH } from '@noble/ciphers/_polyval.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const key = randomBytes(16);
 * const mac = new GHASH(key);
 * mac.update(new Uint8Array(16));
 * mac.digest();
 * ```
 */
export declare class GHASH implements IHash2 {
    readonly blockLen: number;
    readonly outputLen: number;
    protected s0: number;
    protected s1: number;
    protected s2: number;
    protected s3: number;
    protected finished: boolean;
    protected destroyed: boolean;
    protected t: Value[];
    private W;
    private windowSize;
    constructor(key: TArg<Uint8Array>, expectedLength?: number);
    protected _updateBlock(s0: number, s1: number, s2: number, s3: number): void;
    update(data: TArg<Uint8Array>): this;
    destroy(): void;
    digestInto(out: TArg<Uint8Array>): void;
    digest(): TRet<Uint8Array>;
}
/**
 * Incremental POLYVAL state for AES-SIV.
 * @param key - 16-byte POLYVAL key.
 * @param expectedLength - Expected message length for table sizing.
 * Inherits GHASH's segment-padded `update()` behavior: each call is padded
 * independently to a 16-byte boundary before absorption.
 * @example
 * Feeds one block into an incremental POLYVAL state with a fresh hash key.
 *
 * ```ts
 * import { Polyval } from '@noble/ciphers/_polyval.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const key = randomBytes(16);
 * const mac = new Polyval(key);
 * mac.update(new Uint8Array(16));
 * mac.digest();
 * ```
 */
export declare class Polyval extends GHASH {
    constructor(key: TArg<Uint8Array>, expectedLength?: number);
    update(data: TArg<Uint8Array>): this;
    digestInto(out: TArg<Uint8Array>): void;
}
/**
 * GHash MAC for AES-GCM.
 * @param msg - Message bytes to authenticate.
 * @param key - 16-byte GHASH key.
 * @returns 16-byte authentication tag.
 * @example
 * Authenticates a short message with GHASH and a fresh hash key.
 *
 * ```ts
 * import { ghash } from '@noble/ciphers/_polyval.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const key = randomBytes(16);
 * ghash(new Uint8Array(), key);
 * ```
 */
export declare const ghash: TRet<CMac<GHASH, [expectedLength?: number]>>;
/**
 * POLYVAL MAC for AES-SIV.
 * @param msg - Message bytes to authenticate.
 * @param key - 16-byte POLYVAL key.
 * @returns 16-byte authentication tag.
 * @example
 * Authenticates a short message with POLYVAL and a fresh hash key.
 *
 * ```ts
 * import { polyval } from '@noble/ciphers/_polyval.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const key = randomBytes(16);
 * polyval(new Uint8Array(), key);
 * ```
 */
export declare const polyval: TRet<CMac<Polyval, [expectedLength?: number]>>;
export {};
