/**
 * Hex, bytes and number utilities.
 * @module
 */
/*! noble-curves - MIT License (c) 2022 Paul Miller (paulmillr.com) */
import { anumber as anumber_, bytesToHex as bytesToHex_, isBytes as isBytes_ } from '@noble/hashes/utils.js';
/**
 * Bytes API type helpers for old + new TypeScript.
 *
 * TS 5.6 has `Uint8Array`, while TS 5.9+ made it generic `Uint8Array<ArrayBuffer>`.
 * We can't use specific return type, because TS 5.6 will error.
 * We can't use generic return type, because most TS 5.9 software will expect specific type.
 *
 * Maps typed-array input leaves to broad forms.
 * These are compatibility adapters, not ownership guarantees.
 *
 * - `TArg` keeps byte inputs broad.
 * - `TRet` marks byte outputs for TS 5.6 and TS 5.9+ compatibility.
 */
export type TypedArg<T> = T extends BigInt64Array ? BigInt64Array : T extends BigUint64Array ? BigUint64Array : T extends Float32Array ? Float32Array : T extends Float64Array ? Float64Array : T extends Int16Array ? Int16Array : T extends Int32Array ? Int32Array : T extends Int8Array ? Int8Array : T extends Uint16Array ? Uint16Array : T extends Uint32Array ? Uint32Array : T extends Uint8ClampedArray ? Uint8ClampedArray : T extends Uint8Array ? Uint8Array : never;
/** Maps typed-array output leaves to narrow TS-compatible forms. */
export type TypedRet<T> = T extends BigInt64Array ? ReturnType<typeof BigInt64Array.of> : T extends BigUint64Array ? ReturnType<typeof BigUint64Array.of> : T extends Float32Array ? ReturnType<typeof Float32Array.of> : T extends Float64Array ? ReturnType<typeof Float64Array.of> : T extends Int16Array ? ReturnType<typeof Int16Array.of> : T extends Int32Array ? ReturnType<typeof Int32Array.of> : T extends Int8Array ? ReturnType<typeof Int8Array.of> : T extends Uint16Array ? ReturnType<typeof Uint16Array.of> : T extends Uint32Array ? ReturnType<typeof Uint32Array.of> : T extends Uint8ClampedArray ? ReturnType<typeof Uint8ClampedArray.of> : T extends Uint8Array ? ReturnType<typeof Uint8Array.of> : never;
/** Recursively adapts byte-carrying API input types. See {@link TypedArg}. */
export type TArg<T> = T | ([TypedArg<T>] extends [never] ? T extends (...args: infer A) => infer R ? ((...args: {
    [K in keyof A]: TRet<A[K]>;
}) => TArg<R>) & {
    [K in keyof T]: T[K] extends (...args: any) => any ? T[K] : TArg<T[K]>;
} : T extends [infer A, ...infer R] ? [TArg<A>, ...{
    [K in keyof R]: TArg<R[K]>;
}] : T extends readonly [infer A, ...infer R] ? readonly [TArg<A>, ...{
    [K in keyof R]: TArg<R[K]>;
}] : T extends (infer A)[] ? TArg<A>[] : T extends readonly (infer A)[] ? readonly TArg<A>[] : T extends Promise<infer A> ? Promise<TArg<A>> : T extends object ? {
    [K in keyof T]: TArg<T[K]>;
} : T : TypedArg<T>);
/** Recursively adapts byte-carrying API output types. See {@link TypedArg}. */
export type TRet<T> = T extends unknown ? T & ([TypedRet<T>] extends [never] ? T extends (...args: infer A) => infer R ? ((...args: {
    [K in keyof A]: TArg<A[K]>;
}) => TRet<R>) & {
    [K in keyof T]: T[K] extends (...args: any) => any ? T[K] : TRet<T[K]>;
} : T extends [infer A, ...infer R] ? [TRet<A>, ...{
    [K in keyof R]: TRet<R[K]>;
}] : T extends readonly [infer A, ...infer R] ? readonly [TRet<A>, ...{
    [K in keyof R]: TRet<R[K]>;
}] : T extends (infer A)[] ? TRet<A>[] : T extends readonly (infer A)[] ? readonly TRet<A>[] : T extends Promise<infer A> ? Promise<TRet<A>> : T extends object ? {
    [K in keyof T]: TRet<T[K]>;
} : T : TypedRet<T>) : never;
/**
 * Validates that a value is an array, optionally validating each element.
 * @param item - Value to validate.
 * @param title - Label included in thrown errors.
 * @param inner - Optional per-element validator, called with the element and its label.
 * @returns The validated array.
 * @example
 * Validate an array of points before batch processing.
 *
 * ```ts
 * aarray([1n, 2n], 'scalars');
 * ```
 */
export declare function aarray<T>(item: unknown, title: string, inner?: (elm: T, title: string) => void): T[];
/**
 * Validates that a value is a byte array.
 * @param value - Value to validate.
 * @param length - Optional exact byte length.
 * @param title - Optional field name.
 * @returns Original byte array.
 * @example
 * Reject non-byte input before passing data into curve code.
 *
 * ```ts
 * abytes(new Uint8Array(1));
 * ```
 */
export declare const abytes: <T extends TArg<Uint8Array>>(value: T, length?: number, title?: string) => T;
/**
 * Validates that a value is a non-negative safe integer.
 * @param n - Value to validate.
 * @param title - Optional field name.
 * @returns The validated number.
 * @example
 * Validate a numeric length before allocating buffers.
 *
 * ```ts
 * anumber(1);
 * ```
 */
export declare const anumber: typeof anumber_;
/**
 * Asserts something is a string.
 * @param value - Value to validate.
 * @param title - Label included in thrown errors.
 * @returns The validated string.
 * @throws On wrong argument types. {@link TypeError}
 * @example
 * Validate a label string.
 *
 * ```ts
 * astring('example', 'label');
 * ```
 */
export declare function astring(value: unknown, title?: string): string;
/**
 * Asserts something is a plain object-ish value, not null or array.
 * @param value - Value to validate.
 * @param title - Label included in thrown errors.
 * @returns The validated object.
 * @throws On wrong argument types. {@link TypeError}
 * @example
 * Validate an options object before checking fields.
 *
 * ```ts
 * aobject({ flag: true });
 * ```
 */
export declare function aobject<T extends Record<string, any>>(value: T, title?: string): T;
/**
 * Asserts something is a function.
 * @param value - Value to validate.
 * @param title - Label included in thrown errors.
 * @returns The validated function.
 * @throws On wrong argument types. {@link TypeError}
 * @example
 * Validate a required method before calling it.
 *
 * ```ts
 * afunction(() => true, 'predicate');
 * ```
 */
export declare function afunction<T extends (...args: any[]) => any>(value: T, title: string): T;
/**
 * Encodes bytes as lowercase hex.
 * @param bytes - Bytes to encode.
 * @returns Lowercase hex string.
 * @example
 * Serialize bytes as hex for logging or fixtures.
 *
 * ```ts
 * bytesToHex(Uint8Array.of(1, 2, 3));
 * ```
 */
export declare const bytesToHex: typeof bytesToHex_;
/**
 * Concatenates byte arrays.
 * @param arrays - Byte arrays to join.
 * @returns Concatenated bytes.
 * @example
 * Join domain-separated chunks into one buffer.
 *
 * ```ts
 * concatBytes(Uint8Array.of(1), Uint8Array.of(2));
 * ```
 */
export declare const concatBytes: (...arrays: TArg<Uint8Array[]>) => TRet<Uint8Array>;
/**
 * Decodes lowercase or uppercase hex into bytes.
 * @param hex - Hex string to decode.
 * @returns Decoded bytes.
 * @example
 * Parse fixture hex into bytes before hashing.
 *
 * ```ts
 * hexToBytes('0102');
 * ```
 */
export declare const hexToBytes: (hex: string) => TRet<Uint8Array>;
/**
 * Checks whether a value is a Uint8Array.
 * @param a - Value to inspect.
 * @returns `true` when `a` is a Uint8Array.
 * @example
 * Branch on byte input before decoding it.
 *
 * ```ts
 * isBytes(new Uint8Array(1));
 * ```
 */
export declare const isBytes: typeof isBytes_;
/**
 * Reads random bytes from the platform CSPRNG.
 * @param bytesLength - Number of random bytes to read.
 * @returns Fresh random bytes.
 * @example
 * Generate a random seed for a keypair.
 *
 * ```ts
 * randomBytes(2);
 * ```
 */
export declare const randomBytes: (bytesLength?: number) => TRet<Uint8Array>;
/** Callable hash interface with metadata and optional extendable output support. */
export type CHash = {
    /**
     * Hash one message.
     * @param message - Message bytes to hash.
     * @returns Digest bytes.
     */
    (message: TArg<Uint8Array>): TRet<Uint8Array>;
    /** Hash block length in bytes. */
    blockLen: number;
    /** Default output length in bytes. */
    outputLen: number;
    /** Whether `.create()` can be used as an XOF stream. */
    canXOF: boolean;
    /**
     * Create one stateful hash or XOF instance, for example SHAKE with a custom output length.
     * @param opts - Optional extendable-output configuration:
     *   - `dkLen` (optional): Optional output length for XOF-style hashes.
     * @returns Hash instance.
     */
    create(opts?: {
        dkLen?: number;
    }): any;
};
/** Plain callable hash interface. */
export type FHash = (message: TArg<Uint8Array>) => TRet<Uint8Array>;
/** HMAC callback signature. */
export type HmacFn = (key: TArg<Uint8Array>, message: TArg<Uint8Array>) => TRet<Uint8Array>;
/**
 * Validates that a flag is boolean.
 * @param value - Value to validate.
 * @param title - Optional field name.
 * @returns Original value.
 * @throws On wrong argument types. {@link TypeError}
 * @example
 * Reject non-boolean option flags early.
 *
 * ```ts
 * abool(true);
 * ```
 */
export declare function abool(value: boolean, title?: string): boolean;
/**
 * Validates that a value is a non-negative bigint or safe integer.
 * @param n - Value to validate.
 * @returns The same validated value.
 * @throws On wrong argument ranges or values. {@link RangeError}
 * @example
 * Validate one integer-like value before serializing it.
 *
 * ```ts
 * abignumber(1n);
 * ```
 */
export declare function abignumber<T extends number | bigint>(n: T): T;
/**
 * Validates that a value is a safe integer.
 * @param value - Integer to validate.
 * @param title - Optional field name.
 * @throws On wrong argument types. {@link TypeError}
 * @throws On wrong argument ranges or values. {@link RangeError}
 * @example
 * Validate a window size before scalar arithmetic uses it.
 *
 * ```ts
 * asafenumber(1);
 * ```
 */
export declare function asafenumber(value: number, title?: string): void;
/**
 * Encodes a bigint into even-length big-endian hex.
 * The historical "unpadded" name only means "no fixed-width field padding"; odd-length hex still
 * gets one leading zero nibble so the result always represents whole bytes.
 * @param num - Number to encode.
 * @returns Big-endian hex string.
 * @throws On wrong argument ranges or values. {@link RangeError}
 * @example
 * Encode a scalar into hex without a `0x` prefix.
 *
 * ```ts
 * numberToHexUnpadded(255n);
 * ```
 */
export declare function numberToHexUnpadded(num: number | bigint): string;
/**
 * Parses a big-endian hex string into bigint.
 * Accepts odd-length hex through the native `BigInt('0x' + hex)` parser and currently surfaces the
 * same native `SyntaxError` for malformed hex instead of wrapping it in a library-specific error.
 * @param hex - Hex string without `0x`.
 * @returns Parsed bigint value.
 * @throws On wrong argument types. {@link TypeError}
 * @example
 * Parse a scalar from fixture hex.
 *
 * ```ts
 * hexToNumber('ff');
 * ```
 */
export declare function hexToNumber(hex: string): bigint;
/**
 * Parses big-endian bytes into bigint.
 * @param bytes - Bytes in big-endian order.
 * @returns Parsed bigint value.
 * @throws On wrong argument types. {@link TypeError}
 * @example
 * Read a scalar encoded in network byte order.
 *
 * ```ts
 * bytesToNumberBE(Uint8Array.of(1, 0));
 * ```
 */
export declare function bytesToNumberBE(bytes: TArg<Uint8Array>): bigint;
/**
 * Parses little-endian bytes into bigint.
 * @param bytes - Bytes in little-endian order.
 * @returns Parsed bigint value.
 * @throws On wrong argument types. {@link TypeError}
 * @example
 * Read a scalar encoded in little-endian form.
 *
 * ```ts
 * bytesToNumberLE(Uint8Array.of(1, 0));
 * ```
 */
export declare function bytesToNumberLE(bytes: TArg<Uint8Array>): bigint;
/**
 * Encodes a bigint into fixed-length big-endian bytes.
 * @param n - Number to encode.
 * @param len - Output length in bytes. Must be greater than zero.
 * @returns Big-endian byte array.
 * @throws On wrong argument ranges or values. {@link RangeError}
 * @throws If a documented runtime validation or state check fails. {@link Error}
 * @example
 * Serialize a scalar into a 32-byte field element.
 *
 * ```ts
 * numberToBytesBE(255n, 2);
 * ```
 */
export declare function numberToBytesBE(n: number | bigint, len: number): TRet<Uint8Array>;
/**
 * Encodes a bigint into fixed-length little-endian bytes.
 * @param n - Number to encode.
 * @param len - Output length in bytes.
 * @returns Little-endian byte array.
 * @throws On wrong argument ranges or values. {@link RangeError}
 * @throws If a documented runtime validation or state check fails. {@link Error}
 * @example
 * Serialize a scalar for little-endian protocols.
 *
 * ```ts
 * numberToBytesLE(255n, 2);
 * ```
 */
export declare function numberToBytesLE(n: number | bigint, len: number): TRet<Uint8Array>;
/**
 * Encodes a bigint into variable-length big-endian bytes.
 * @param n - Number to encode.
 * @returns Variable-length big-endian bytes.
 * @throws On wrong argument ranges or values. {@link RangeError}
 * @example
 * Serialize a bigint without fixed-width padding.
 *
 * ```ts
 * numberToVarBytesBE(255n);
 * ```
 */
export declare function numberToVarBytesBE(n: number | bigint): TRet<Uint8Array>;
/**
 * Compares two byte arrays in constant-ish time.
 * @param a - Left byte array.
 * @param b - Right byte array.
 * @returns `true` when bytes match.
 * @example
 * Compare two encoded points without early exit.
 *
 * ```ts
 * equalBytes(Uint8Array.of(1), Uint8Array.of(1));
 * ```
 */
export declare function equalBytes(a: TArg<Uint8Array>, b: TArg<Uint8Array>): boolean;
/**
 * Copies Uint8Array. We can't use u8a.slice(), because u8a can be Buffer,
 * and Buffer#slice creates mutable copy. Never use Buffers!
 * @param bytes - Bytes to copy.
 * @returns Detached copy.
 * @example
 * Make an isolated copy before mutating serialized bytes.
 *
 * ```ts
 * copyBytes(Uint8Array.of(1, 2, 3));
 * ```
 */
export declare function copyBytes(bytes: TArg<Uint8Array>): TRet<Uint8Array>;
/**
 * Decodes 7-bit ASCII string to Uint8Array, throws on non-ascii symbols
 * Should be safe to use for things expected to be ASCII.
 * Returns exact same result as `TextEncoder` for ASCII or throws.
 * @param ascii - ASCII input text.
 * @returns Encoded bytes.
 * @throws On wrong argument types. {@link TypeError}
 * @example
 * Encode an ASCII domain-separation tag.
 *
 * ```ts
 * asciiToBytes('ABC');
 * ```
 */
export declare function asciiToBytes(ascii: string): TRet<Uint8Array>;
/**
 * Checks whether n is non-negative bigint. Historical name.
 * @param n - candidate value
 * @returns `true` when the value is bigint and 0 or larger
 * @example
 * Check a candidate scalar before range validation.
 *
 * ```ts
 * isPosBig(2n);
 * ```
 */
export declare function isPosBig(n: bigint): boolean;
/**
 * Checks whether a bigint lies inside a half-open range.
 * @param n - Candidate value.
 * @param min - Inclusive lower bound.
 * @param max - Exclusive upper bound.
 * @returns `true` when the value is inside the range.
 * @example
 * Check whether a candidate scalar fits the field order.
 *
 * ```ts
 * inRange(2n, 1n, 3n);
 * ```
 */
export declare function inRange(n: bigint, min: bigint, max: bigint): boolean;
/**
 * Asserts `min <= n < max`. NOTE: upper bound is exclusive.
 * @param title - Value label for error messages.
 * @param n - Candidate value.
 * @param min - Inclusive lower bound.
 * @param max - Exclusive upper bound.
 * Wrong-type inputs are not separated from out-of-range values here: they still flow through the
 * shared `RangeError` path because this is only a throwing wrapper around `inRange(...)`.
 * @throws On wrong argument ranges or values. {@link RangeError}
 * @example
 * Assert that a bigint stays within one half-open range.
 *
 * ```ts
 * aInRange('x', 2n, 1n, 256n);
 * ```
 */
export declare function aInRange(title: string, n: bigint, min: bigint, max: bigint): void;
/**
 * Calculates amount of bits in a bigint.
 * Same as `n.toString(2).length`
 * TODO: merge with nLength in modular
 * @param n - Value to inspect.
 * @returns Bit length.
 * @throws If the value is negative. {@link Error}
 * @example
 * Measure the bit length of a scalar before serialization.
 *
 * ```ts
 * bitLen(8n);
 * ```
 */
export declare function bitLen(n: bigint): number;
/**
 * Gets single bit at position.
 * NOTE: first bit position is 0 (same as arrays)
 * Same as `!!+Array.from(n.toString(2)).reverse()[pos]`
 * @param n - Source value.
 * @param pos - Bit position. Negative positions are passed through to raw
 *   bigint shift semantics; because the mask is built as `1n << pos`,
 *   they currently collapse to `0n` and make the helper a no-op.
 * @returns Bit as bigint.
 * @example
 * Gets single bit at position.
 *
 * ```ts
 * bitGet(5n, 0);
 * ```
 */
export declare function bitGet(n: bigint, pos: number): bigint;
/**
 * Sets single bit at position.
 * @param n - Source value.
 * @param pos - Bit position. Negative positions are passed through to raw bigint shift semantics,
 *   so they currently behave like left shifts.
 * @param value - Whether the bit should be set.
 * @returns Updated bigint.
 * @example
 * Sets single bit at position.
 *
 * ```ts
 * bitSet(0n, 1, true);
 * ```
 */
export declare function bitSet(n: bigint, pos: number, value: boolean): bigint;
/**
 * Calculate mask for N bits. Not using ** operator with bigints because of old engines.
 * Same as BigInt(`0b${Array(i).fill('1').join('')}`)
 * @param n - Number of bits. Negative widths are currently passed through to raw bigint shift
 *   semantics and therefore produce `-1n`.
 * @returns Bitmask value.
 * @example
 * Calculate mask for N bits.
 *
 * ```ts
 * bitMask(4);
 * ```
 */
export declare const bitMask: (n: number) => bigint;
type Pred<T> = (v: TArg<Uint8Array>) => T | undefined;
/**
 * Minimal HMAC-DRBG from NIST 800-90 for RFC6979 sigs.
 * @param hashLen - Hash output size in bytes. Callers are expected to pass a positive length; `0`
 *   is not rejected here and would make the internal generate loop non-progressing.
 * @param qByteLen - Requested output size in bytes. Callers are expected to pass a positive length.
 * @param hmacFn - HMAC implementation.
 * @returns Function that will call DRBG until the predicate returns anything
 *   other than `undefined`.
 * @throws On wrong argument types. {@link TypeError}
 * @example
 * Build a deterministic nonce generator for RFC6979-style signing.
 *
 * ```ts
 * import { createHmacDrbg } from '@noble/curves/utils.js';
 * import { hmac } from '@noble/hashes/hmac.js';
 * import { sha256 } from '@noble/hashes/sha2.js';
 * const hmacFn = (key: Uint8Array, msg: Uint8Array) => hmac(sha256, key, msg);
 * const drbg = createHmacDrbg(32, 32, hmacFn);
 * const seed = new Uint8Array(32);
 * drbg(seed, (bytes) => bytes);
 * ```
 */
export declare function createHmacDrbg<T>(hashLen: number, qByteLen: number, hmacFn: TArg<HmacFn>): TRet<(seed: Uint8Array, predicate: Pred<T>) => T>;
/**
 * Validates declared required and optional field types on a plain object.
 * Extra keys are intentionally ignored because many callers validate only the subset they use from
 * richer option bags or runtime objects.
 * This walks field schemas and formats detailed errors, so avoid it on hot paths; use direct
 * one-line guards such as `aobject()`, `afunction()`, `abool()`, or `asafenumber()` instead.
 * @param object - Object to validate.
 * @param fields - Required field types.
 * @param optFields - Optional field types.
 * @param title - Object label included in thrown errors.
 * @throws On wrong argument types. {@link TypeError}
 * @example
 * Check user options before building a curve helper.
 *
 * ```ts
 * validateObject({ flag: true }, { flag: 'boolean' });
 * ```
 */
export declare function validateObject(object: Record<string, any>, fields?: Record<string, string>, optFields?: Record<string, string>, title?: string): void;
/**
 * Throws not implemented error.
 * @returns Never returns.
 * @throws If the unfinished code path is reached. {@link Error}
 * @example
 * Surface the placeholder error from an unfinished code path.
 *
 * ```ts
 * try {
 *   notImplemented();
 * } catch {}
 * ```
 */
export declare const notImplemented: () => never;
/** Generic keygen/getPublicKey interface shared by curve helpers. */
export interface CryptoKeys {
    /** Public byte lengths for keys and optional seeds. */
    lengths: {
        seed?: number;
        public?: number;
        secret?: number;
    };
    /**
     * Generate one secret/public keypair.
     * @param seed - Optional seed bytes for deterministic key generation.
     * @returns Fresh secret/public keypair.
     */
    keygen: (seed?: Uint8Array) => {
        secretKey: Uint8Array;
        publicKey: Uint8Array;
    };
    /**
     * Derive one public key from a secret key.
     * @param secretKey - Secret key bytes.
     * @returns Public key bytes.
     */
    getPublicKey: (secretKey: Uint8Array) => Uint8Array;
}
/** Generic interface for signatures. Has keygen, sign and verify. */
export interface Signer extends CryptoKeys {
    /** Public byte lengths for keys, signatures, and optional signing randomness. */
    lengths: {
        seed?: number;
        public?: number;
        secret?: number;
        signRand?: number;
        signature?: number;
    };
    /**
     * Sign one message.
     * @param msg - Message bytes to sign.
     * @param secretKey - Secret key bytes.
     * @returns Signature bytes.
     */
    sign: (msg: Uint8Array, secretKey: Uint8Array) => Uint8Array;
    /**
     * Verify one signature.
     * @param sig - Signature bytes.
     * @param msg - Signed message bytes.
     * @param publicKey - Public key bytes.
     * @returns `true` when the signature is valid.
     */
    verify: (sig: Uint8Array, msg: Uint8Array, publicKey: Uint8Array) => boolean;
}
export {};
