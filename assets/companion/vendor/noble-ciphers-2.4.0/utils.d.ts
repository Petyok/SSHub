/**
 * Utilities for hex, bytes, CSPRNG.
 * @module
 */
/*! noble-ciphers - MIT License (c) 2023 Paul Miller (paulmillr.com) */
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
export declare function aarray<T>(item: unknown, title: string, inner?: (elm: T, title: string) => void): T[];
/**
 * Checks if something is Uint8Array. Be careful: nodejs Buffer will return true.
 * @param a - Value to inspect.
 * @returns `true` when the value is a Uint8Array view, including Node's `Buffer`.
 * @example
 * Guards a value before treating it as raw key material.
 *
 * ```ts
 * isBytes(new Uint8Array());
 * ```
 */
export declare function isBytes(a: unknown): a is Uint8Array;
/**
 * Asserts something is boolean.
 * @param value - Value to validate.
 * @returns The validated boolean.
 * @throws On wrong argument types. {@link TypeError}
 * @example
 * Validates a boolean option before branching on it.
 *
 * ```ts
 * abool(true);
 * ```
 */
export declare function abool(value: boolean, title?: string): boolean;
/**
 * Asserts something is a non-negative safe integer.
 * @param n - Value to validate.
 * @returns The validated number.
 * @throws On wrong argument types. {@link TypeError}
 * @throws On wrong argument ranges or values. {@link RangeError}
 * @example
 * Validates a non-negative length or counter.
 *
 * ```ts
 * anumber(1);
 * ```
 */
export declare function anumber(n: number, title?: string): number;
/**
 * Asserts something is Uint8Array.
 * @param value - Value to validate.
 * @param length - Expected byte length.
 * @param title - Optional label used in error messages.
 * @returns The validated byte array.
 * On Node, `Buffer` is accepted too because it is a Uint8Array view.
 * @throws On wrong argument types. {@link TypeError}
 * @throws On wrong argument lengths. {@link RangeError}
 * @example
 * Validates a fixed-length nonce or key buffer.
 *
 * ```ts
 * abytes(new Uint8Array([1, 2]), 2);
 * ```
 */
export declare function abytes(value: TArg<Uint8Array>, length?: number, title?: string): TRet<Uint8Array>;
/**
 * Asserts a hash- or MAC-like instance has not been destroyed or finished.
 * @param instance - Stateful instance to validate.
 * @param checkFinished - Whether to reject finished instances.
 * When `false`, only `destroyed` is checked.
 * @throws If the hash instance has already been destroyed or finalized. {@link Error}
 * @example
 * Guards against calling `update()` or `digest()` on a finished hash.
 *
 * ```ts
 * aexists({ destroyed: false, finished: false });
 * ```
 */
export declare function aexists(instance: any, checkFinished?: boolean): void;
/**
 * Asserts output is a sufficiently-sized byte array.
 * @param out - Output buffer to validate.
 * @param instance - Hash-like instance providing `outputLen`.
 * This is the relaxed `digestInto()`-style contract: output must be at least `outputLen`,
 * unlike one-shot cipher helpers elsewhere in the repo that often require exact lengths.
 * @throws On wrong argument types. {@link TypeError}
 * @throws On wrong output buffer lengths. {@link RangeError}
 * @example
 * Verifies that a caller-provided output buffer is large enough.
 *
 * ```ts
 * aoutput(new Uint8Array(16), { outputLen: 16 });
 * ```
 */
export declare function aoutput(out: any, instance: any): void;
/**
 * Asserts output is a sufficiently-sized, 4-byte-aligned byte array.
 * {@link aoutput} plus an {@link isAligned32} check, for `digestInto()` paths
 * that write through zero-allocation `u32` word views.
 * @param out - Output buffer to validate.
 * @param instance - Hash-like instance providing `outputLen`.
 * @throws On wrong argument types. {@link TypeError}
 * @throws On wrong output buffer lengths. {@link RangeError}
 * @throws On wrong output buffer alignment. {@link Error}
 * @example
 * Verifies that a caller-provided output buffer is large enough and aligned.
 *
 * ```ts
 * aoutput32(new Uint8Array(16), { outputLen: 16 });
 * ```
 */
export declare function aoutput32(out: any, instance: any): void;
/** One-shot hash helper with `.create()`. */
export type IHash = {
    (data: string | TArg<Uint8Array>): TRet<Uint8Array>;
    /** Input block size in bytes. */
    blockLen: number;
    /** Digest size in bytes. */
    outputLen: number;
    /** Creates a fresh incremental hash instance of the same algorithm. */
    create: any;
};
/** One-shot MAC helper with `.create()`. */
export type CMac<H extends IHash2 = IHash2, A extends any[] = []> = {
    (msg: TArg<Uint8Array>, key: TArg<Uint8Array>): TRet<Uint8Array>;
    /** Input block size in bytes. */
    blockLen: number;
    /** Digest size in bytes. */
    outputLen: number;
    /**
     * Creates a fresh incremental MAC instance of the same algorithm.
     * @param key - MAC key bytes.
     * @param args - Additional constructor arguments, when the MAC wrapper needs them.
     * @returns Fresh incremental MAC instance.
     */
    create(key: TArg<Uint8Array>, ...args: A): H;
};
/** Generic type encompassing 8/16/32-bit typed arrays, but not 64-bit. */
export type TypedArray = Int8Array | Uint8ClampedArray | Uint8Array | Uint16Array | Int16Array | Uint32Array | Int32Array;
/**
 * Casts a typed-array view to Uint8Array.
 * @param arr - Typed-array view to reinterpret.
 * @returns Uint8Array view over the same bytes.
 * @example
 * Views 32-bit words as raw bytes without copying.
 *
 * ```ts
 * u8(new Uint32Array([1]));
 * ```
 */
export declare function u8(arr: TArg<TypedArray>): TRet<Uint8Array>;
/**
 * Casts a typed-array view to Uint32Array.
 * @param arr - Typed-array view to reinterpret.
 * @returns Uint32Array view over the same bytes. Callers are expected to provide a
 * 4-byte-aligned offset; trailing `1..3` bytes are silently dropped.
 * @example
 * Views a byte buffer as 32-bit words for block processing.
 *
 * ```ts
 * u32(new Uint8Array(4));
 * ```
 */
export declare function u32(arr: TArg<TypedArray>): TRet<Uint32Array>;
/**
 * Zeroizes typed arrays in place.
 * Warning: JS provides no guarantees.
 * @param arrays - Arrays to wipe.
 * @example
 * Wipes a temporary key buffer after use.
 *
 * ```ts
 * const bytes = new Uint8Array([1]);
 * clean(bytes);
 * ```
 */
export declare function clean(...arrays: TArg<TypedArray[]>): void;
/**
 * Creates a DataView for byte-level manipulation.
 * @param arr - Typed-array view to wrap.
 * @returns DataView over the same bytes.
 * @example
 * Creates an endian-aware view for length encoding.
 *
 * ```ts
 * createView(new Uint8Array(4));
 * ```
 */
export declare function createView(arr: TArg<TypedArray>): DataView;
/**
 * Whether the current platform is little-endian.
 * Most are; some IBM systems are not.
 */
export declare const isLE: boolean;
/**
 * Reverses byte order of one 32-bit word.
 * @param word - Unsigned 32-bit word to swap.
 * @returns The same word with bytes reversed.
 * @example
 * Swaps a big-endian word into little-endian byte order.
 *
 * ```ts
 * byteSwap(0x11223344);
 * ```
 */
export declare function byteSwap(word: number): number;
/**
 * Normalizes one 32-bit word to the little-endian representation expected by cipher cores.
 * @param n - Unsigned 32-bit word to normalize.
 * @returns Little-endian normalized word on big-endian hosts, else the input word unchanged.
 * @example
 * Normalizes a host-endian word before passing it into an ARX/AES core.
 *
 * ```ts
 * swap8IfBE(0x11223344);
 * ```
 */
export declare const swap8IfBE: (n: number) => number;
/**
 * Byte-swaps every word of a Uint32Array in place.
 * @param arr - Uint32Array whose words should be swapped.
 * @returns The same array after in-place byte swapping.
 * @example
 * Swaps every 32-bit word in a word-view buffer.
 *
 * ```ts
 * byteSwap32(new Uint32Array([0x11223344]));
 * ```
 */
export declare function byteSwap32(arr: TArg<Uint32Array>): TRet<Uint32Array>;
/**
 * Normalizes a Uint32Array view to the little-endian representation expected by cipher cores.
 * @param u - Word view to normalize in place.
 * @returns Little-endian normalized word view.
 * @example
 * Normalizes a word-view buffer before block processing.
 *
 * ```ts
 * swap32IfBE(new Uint32Array([0x11223344]));
 * ```
 */
export declare const swap32IfBE: (u: TArg<Uint32Array>) => TRet<Uint32Array>;
/**
 * Convert byte array to hex string. Uses built-in function, when available.
 * @param bytes - Bytes to encode.
 * @returns Lowercase hexadecimal string.
 * @throws On wrong argument types. {@link TypeError}
 * @example
 * Formats ciphertext bytes for logs or test vectors.
 *
 * ```ts
 * bytesToHex(Uint8Array.from([0xca, 0xfe, 0x01, 0x23])); // 'cafe0123'
 * ```
 */
export declare function bytesToHex(bytes: TArg<Uint8Array>): string;
/**
 * Convert hex string to byte array. Uses built-in function, when available.
 * @param hex - hexadecimal string to decode
 * @returns Decoded bytes.
 * @throws On wrong argument types. {@link TypeError}
 * @throws On wrong argument ranges or values. {@link RangeError}
 * @example
 * Decode lowercase hexadecimal into bytes.
 * ```ts
 * hexToBytes('cafe0123'); // Uint8Array.from([0xca, 0xfe, 0x01, 0x23])
 * ```
 */
export declare function hexToBytes(hex: string): TRet<Uint8Array>;
/**
 * Converts a big-endian hex string into bigint.
 * @param hex - Hexadecimal string without `0x`.
 * @returns Parsed bigint value. The empty string is treated as `0n`.
 * @throws On wrong argument types. {@link TypeError}
 * @example
 * Parses a big-endian field element or counter from hex.
 *
 * ```ts
 * hexToNumber('ff');
 * ```
 */
export declare function hexToNumber(hex: string): bigint;
/**
 * Converts big-endian bytes into bigint.
 * @param bytes - Big-endian bytes.
 * @returns Parsed bigint value. Empty input is treated as `0n`.
 * @throws On invalid byte input passed to the internal hex conversion. {@link TypeError}
 * @example
 * Reads a big-endian integer from serialized bytes.
 *
 * ```ts
 * bytesToNumberBE(new Uint8Array([1, 0]));
 * ```
 */
export declare function bytesToNumberBE(bytes: TArg<Uint8Array>): bigint;
/**
 * Converts a number into big-endian bytes of fixed length.
 * @param n - Number to encode.
 * @param len - Output length in bytes. Must be greater than zero.
 * @returns Big-endian bytes padded to `len`.
 * Negative values, `len = 0`, and values that do not fit are rejected before
 * downstream hex parsing.
 * @throws On wrong argument ranges or values. {@link RangeError}
 * @throws If a documented runtime validation or state check fails. {@link Error}
 * @example
 * Encodes a counter as fixed-width big-endian bytes.
 *
 * ```ts
 * numberToBytesBE(1, 2);
 * ```
 */
export declare function numberToBytesBE(n: number | bigint, len: number): TRet<Uint8Array>;
/**
 * Converts string to bytes using UTF8 encoding.
 * @param str - String to encode.
 * @returns UTF-8 bytes in a detached fresh Uint8Array copy.
 * @throws On wrong argument types. {@link TypeError}
 * @example
 * Encodes application text before encryption or MACing.
 *
 * ```ts
 * utf8ToBytes('abc'); // new Uint8Array([97, 98, 99])
 * ```
 */
export declare function utf8ToBytes(str: string): TRet<Uint8Array>;
/**
 * Converts bytes to string using UTF8 encoding.
 * @param bytes - UTF-8 bytes.
 * @returns Decoded string. Input validation is delegated to `TextDecoder`, and malformed
 * UTF-8 is replacement-decoded instead of rejected.
 * @example
 * Decodes UTF-8 plaintext back into a string.
 *
 * ```ts
 * bytesToUtf8(new Uint8Array([97, 98, 99])); // 'abc'
 * ```
 */
export declare function bytesToUtf8(bytes: TArg<Uint8Array>): string;
/**
 * Checks if two U8A use same underlying buffer and overlaps.
 * This is invalid and can corrupt data.
 * @param a - First byte view.
 * @param b - Second byte view.
 * @returns `true` when the views overlap in memory.
 * @example
 * Detects whether two slices alias the same backing buffer.
 *
 * ```ts
 * overlapBytes(new Uint8Array(4), new Uint8Array(4));
 * ```
 */
export declare function overlapBytes(a: TArg<Uint8Array>, b: TArg<Uint8Array>): boolean;
/**
 * If input and output overlap and input starts before output, we will overwrite end of input before
 * we start processing it, so this is not supported by forward-processing ciphers.
 * @param input - Input bytes.
 * @param output - Output bytes.
 * @throws If the output view would overwrite unread input bytes. {@link Error}
 * @example
 * Rejects an in-place layout that would overwrite unread input bytes.
 *
 * ```ts
 * const buffer = new Uint8Array(8);
 * complexOverlapBytes(buffer.subarray(0, 4), buffer.subarray(2, 6));
 * ```
 */
export declare function complexOverlapBytes(input: TArg<Uint8Array>, output: TArg<Uint8Array>): void;
/**
 * Copies several Uint8Arrays into one.
 * @param arrays - Byte arrays to concatenate.
 * @returns Combined byte array.
 * @throws On wrong argument types inside the byte-array list. {@link TypeError}
 * @example
 * Builds a `nonce || ciphertext` style buffer.
 *
 * ```ts
 * concatBytes(new Uint8Array([1]), new Uint8Array([2]));
 * ```
 */
export declare function concatBytes(...arrays: TArg<Uint8Array[]>): TRet<Uint8Array>;
type EmptyObj = {};
/**
 * Merges user options into defaults.
 * @param defaults - Default option values.
 * @param opts - User-provided overrides.
 * @returns Combined options object.
 * `defaults` is a library-owned mutable object; user-provided `opts` only need to be
 * object-shaped, since "plain object" checks reject valid proxy/cross-realm containers.
 * The merge mutates `defaults` in place and returns the same object, so direct callers
 * should pass a fresh defaults object unless they intentionally want shared state updated.
 * @throws If options are missing or not an object. {@link Error}
 * @example
 * Applies user overrides to the default cipher options.
 *
 * ```ts
 * checkOpts({ rounds: 20 }, { rounds: 8 });
 * ```
 */
export declare function checkOpts<T1 extends EmptyObj, T2 extends EmptyObj>(defaults: T1, opts: T2): T1 & T2;
/**
 * Compares two byte arrays in kinda constant time once lengths already match.
 * @param a - First byte array.
 * @param b - Second byte array.
 * @returns `true` when the arrays contain the same bytes. Different lengths still return early.
 * @example
 * Compares an expected authentication tag with the received one.
 *
 * ```ts
 * equalBytes(new Uint8Array([1]), new Uint8Array([1]));
 * ```
 */
export declare function equalBytes(a: TArg<Uint8Array>, b: TArg<Uint8Array>): boolean;
/** Incremental hash interface used internally. */
export interface IHash2 {
    /** Bytes processed per compression block. */
    blockLen: number;
    /** Bytes produced by the final digest. */
    outputLen: number;
    /**
     * Absorbs one more chunk into the hash state.
     * @param buf - Data chunk to hash.
     * @returns The same hash instance for chaining.
     */
    update(buf: string | TArg<Uint8Array>): this;
    /**
     * Writes the final digest into a caller-provided buffer.
     * @param buf - Destination buffer for the digest bytes.
     * @returns Nothing. Implementations write into `buf` in place.
     */
    digestInto(buf: TArg<Uint8Array>): void;
    /**
     * Finalizes the hash and returns a fresh digest buffer.
     * @returns Digest bytes.
     */
    digest(): TRet<Uint8Array>;
    /**
     * Resets internal state. Makes Hash instance unusable.
     * Reset is impossible for keyed hashes if key is consumed into state. If digest is not consumed
     * by user, they will need to manually call `destroy()` when zeroing is necessary.
     */
    destroy(): void;
}
/**
 * Wraps a keyed MAC constructor into a one-shot helper with `.create()`.
 * @param keyLen - Valid probe-key length used to read static metadata once.
 * The probe key is only used for `outputLen` / `blockLen`, so callers with several valid key sizes
 * can pass any representative size as long as those values stay fixed.
 * @param macCons - Keyed MAC constructor or factory.
 * @param fromMsg - Optional adapter that derives extra constructor args from the one-shot message.
 * @returns Callable MAC helper with `.create()`.
 */
export declare function wrapMacConstructor<H extends IHash2, A extends any[] = []>(keyLen: number, macCons: TArg<(key: Uint8Array, ...args: A) => H>, fromMsg?: TArg<(msg: Uint8Array) => A>): TRet<CMac<H, A>>;
/** Sync cipher: takes byte array and returns byte array. */
export type Cipher = {
    /**
     * Encrypts plaintext bytes.
     * @param plaintext - Data to encrypt.
     * @returns Ciphertext bytes.
     */
    encrypt(plaintext: TArg<Uint8Array>): TRet<Uint8Array>;
    /**
     * Decrypts ciphertext bytes.
     * @param ciphertext - Data to decrypt.
     * @returns Plaintext bytes.
     */
    decrypt(ciphertext: TArg<Uint8Array>): TRet<Uint8Array>;
};
/** Async cipher e.g. from built-in WebCrypto. */
export type AsyncCipher = {
    /**
     * Encrypts plaintext bytes.
     * @param plaintext - Data to encrypt.
     * @returns Promise resolving to ciphertext bytes.
     */
    encrypt(plaintext: TArg<Uint8Array>): Promise<TRet<Uint8Array>>;
    /**
     * Decrypts ciphertext bytes.
     * @param ciphertext - Data to decrypt.
     * @returns Promise resolving to plaintext bytes.
     */
    decrypt(ciphertext: TArg<Uint8Array>): Promise<TRet<Uint8Array>>;
};
/** Cipher with `output` argument which can optimize by doing 1 less allocation. */
export type CipherWithOutput = Cipher & {
    /**
     * Encrypts plaintext bytes into an optional caller-provided buffer.
     * @param plaintext - Data to encrypt.
     * @param output - Optional destination buffer.
     * @returns Ciphertext bytes.
     */
    encrypt(plaintext: TArg<Uint8Array>, output?: TArg<Uint8Array>): TRet<Uint8Array>;
    /**
     * Decrypts ciphertext bytes into an optional caller-provided buffer.
     * @param ciphertext - Data to decrypt.
     * @param output - Optional destination buffer.
     * @returns Plaintext bytes.
     */
    decrypt(ciphertext: TArg<Uint8Array>, output?: TArg<Uint8Array>): TRet<Uint8Array>;
};
/**
 * Params are outside of return type, so it is accessible before calling constructor.
 * If function support multiple nonceLength's, we return the best one.
 */
export type CipherParams = {
    /** Cipher block size in bytes. */
    blockSize: number;
    /** Nonce length in bytes when the cipher uses a fixed nonce size. */
    nonceLength?: number;
    /** Authentication-tag length in bytes for AEAD modes. */
    tagLength?: number;
    /** Whether constructor accepts associated data after nonce/options args. */
    withAAD?: true;
    /** Whether nonce length is variable at runtime. */
    varSizeNonce?: boolean;
};
/**
 * ARX AEAD cipher, like salsa or chacha.
 * @param key - Secret key bytes.
 * @param nonce - Nonce bytes.
 * @param AAD - Optional associated data.
 * @returns Cipher instance with caller-managed output buffers.
 */
export type ARXCipher = ((key: TArg<Uint8Array>, nonce: TArg<Uint8Array>, AAD?: TArg<Uint8Array>) => CipherWithOutput) & {
    blockSize: number;
    nonceLength: number;
    tagLength: number;
};
/**
 * Cipher constructor signature.
 * @param key - Secret key bytes.
 * @param args - Additional constructor arguments, such as nonce or IV.
 * @returns Cipher instance.
 */
export type CipherCons<T extends any[]> = (key: TArg<Uint8Array>, ...args: T) => Cipher;
/**
 * Wraps a cipher: validates args, ensures encrypt() can only be called once.
 * Used internally by the exported cipher constructors.
 * Output-buffer support is inferred from the wrapped `encrypt` / `decrypt`
 * arity (`fn.length === 2`), so wrapped output-capable methods must use a normal
 * second parameter, not a default/rest parameter. AAD support is explicit in
 * `params.withAAD`; optional AAD starts after the nonce slot when one is present.
 * @__NO_SIDE_EFFECTS__
 * @param params - Static cipher metadata. See {@link CipherParams}.
 * @param constructor - Cipher constructor.
 * @returns Wrapped constructor with validation.
 */
export declare const wrapCipher: <C extends CipherCons<any>, P extends CipherParams>(params: P, constructor: C) => C & P;
/**
 * Represents a Salsa or ChaCha xor stream.
 * @param key - Secret key bytes.
 * @param nonce - Nonce bytes.
 * @param data - Input bytes to xor with the keystream.
 * @param output - Optional destination buffer.
 * @param counter - Optional starting block counter.
 * @returns Output bytes.
 */
export type XorStream = (key: TArg<Uint8Array>, nonce: TArg<Uint8Array>, data: TArg<Uint8Array>, output?: TArg<Uint8Array>, counter?: number) => TRet<Uint8Array>;
/**
 * By default, returns u8a of length.
 * When out is available, it checks it for validity and uses it.
 * @param expectedLength - Required output length.
 * @param out - Optional destination buffer.
 * @param onlyAligned - Whether `out` must be 4-byte aligned.
 * @returns Output buffer ready for writing.
 * @throws On wrong argument types. {@link TypeError}
 * @throws If the provided output buffer has the wrong size. {@link RangeError}
 * @throws If the provided output buffer has the wrong alignment. {@link Error}
 * @example
 * Reuses a caller-provided output buffer when lengths match.
 *
 * ```ts
 * getOutput(16, new Uint8Array(16));
 * ```
 */
export declare function getOutput(expectedLength: number, out?: TArg<Uint8Array>, onlyAligned?: boolean): TRet<Uint8Array>;
/**
 * Encodes data and AAD lengths into a 16-byte buffer.
 * @param dataLength - Data length. Units are caller-defined: GCM passes bit
 * lengths, ChaCha20-Poly1305 passes byte lengths — the helper writes the raw values.
 * @param aadLength - AAD length, same unit convention as `dataLength`.
 * The serialized block is still `aadLength || dataLength`, matching GCM/Poly1305
 * conventions even though the helper parameter order is `(dataLength, aadLength)`.
 * @param isLE - Whether to encode lengths as little-endian.
 * @returns 16-byte length block.
 * @throws On wrong argument types passed to the endian validator. {@link TypeError}
 * @throws On wrong argument ranges or values. {@link RangeError}
 * @example
 * Builds the length block appended by GCM and Poly1305.
 *
 * ```ts
 * u64Lengths(16, 8, true);
 * ```
 */
export declare function u64Lengths(dataLength: number, aadLength: number, isLE: boolean): TRet<Uint8Array>;
/**
 * Checks whether a byte array is aligned to a 4-byte offset.
 * @param bytes - Byte array to inspect.
 * @returns `true` when the view is 4-byte aligned.
 * @example
 * Checks whether a buffer can be safely viewed as Uint32Array.
 *
 * ```ts
 * isAligned32(new Uint8Array(4));
 * ```
 */
export declare function isAligned32(bytes: TArg<Uint8Array>): boolean;
/**
 * Copies bytes into a new Uint8Array.
 * @param bytes - Bytes to copy.
 * @returns Copied byte array.
 * @throws On wrong argument types. {@link TypeError}
 * @example
 * Copies input into an aligned Uint8Array before block processing.
 *
 * ```ts
 * copyBytes(new Uint8Array([1, 2]));
 * ```
 */
export declare function copyBytes(bytes: TArg<Uint8Array>): TRet<Uint8Array>;
/**
 * Cryptographically secure PRNG backed by `crypto.getRandomValues`.
 * @param bytesLength - number of random bytes to generate
 * @returns Random bytes.
 * The platform `getRandomValues()` implementation still defines any
 * single-call length cap, and this helper rejects oversize requests
 * with a stable library `RangeError` instead of host-specific errors.
 * @throws On wrong argument types. {@link TypeError}
 * @throws On wrong argument ranges or values. {@link RangeError}
 * @throws If the current runtime does not provide `crypto.getRandomValues`. {@link Error}
 * @example
 * Generate a fresh random key or nonce.
 * ```ts
 * const key = randomBytes(16);
 * ```
 */
export declare function randomBytes(bytesLength?: number): TRet<Uint8Array>;
/**
 * The pseudorandom number generator doesn't wipe current state:
 * instead, it generates new one based on previous state + entropy.
 * Not reseed/rekey, since AES CTR DRBG does rekey on each randomBytes,
 * which is in fact `reseed`, since it changes counter too.
 */
export interface PRG {
    /**
     * Mixes fresh entropy into the current generator state.
     * @param seed - Entropy bytes to absorb.
     */
    addEntropy(seed: TArg<Uint8Array>): void;
    /**
     * Produces a requested number of pseudorandom bytes.
     * @param bytesLength - Number of bytes to generate.
     * @returns Random byte array.
     */
    randomBytes(bytesLength: number): TRet<Uint8Array>;
    /** Destroys the generator state. */
    clean(): void;
}
/** Removes the nonce argument from a cipher constructor type. */
export type RemoveNonce<T extends (...args: any) => any> = T extends (arg0: any, arg1: any, ...rest: infer R) => infer Ret ? (key: TArg<Uint8Array>, ...args: R) => Ret : never;
/**
 * Cipher constructor that requires a nonce argument.
 * @param key - Secret key bytes.
 * @param nonce - Nonce bytes.
 * @param args - Additional cipher-specific arguments.
 * @returns Cipher instance.
 */
export type CipherWithNonce = ((key: TArg<Uint8Array>, nonce: TArg<Uint8Array>, ...args: any[]) => Cipher | AsyncCipher) & {
    nonceLength: number;
};
/**
 * Uses CSPRNG for nonce, nonce injected in ciphertext.
 * For `encrypt`, a `nonceBytes`-length buffer is fetched from CSPRNG and
 * prepended to encrypted ciphertext. For `decrypt`, first `nonceBytes` of ciphertext
 * are treated as nonce. The wrapper always allocates a fresh `nonce || ciphertext`
 * buffer on encrypt and intentionally does not support caller-provided destination buffers.
 * Too-short decrypt inputs are split into short/empty nonce views and then delegated
 * to the wrapped cipher instead of being rejected here first.
 *
 * NOTE: Under the same key, using random nonces (e.g. `managedNonce`) with AES-GCM and ChaCha
 * should be limited to `2**23` (8M) messages to get a collision chance of
 * `2**-50`. Stretching to `2**32` (4B) messages would raise that chance to
 * `2**-33`, still negligible but creeping up.
 * @param fn - Cipher constructor that expects a nonce.
 * @param randomBytes_ - Random-byte source used for nonce generation.
 * @returns Cipher constructor that prepends the nonce to ciphertext.
 * @throws On wrong argument types. {@link TypeError}
 * @throws On invalid nonce lengths observed at wrapper construction or use. {@link RangeError}
 * @example
 * Prepends a fresh random nonce to every ciphertext.
 *
 * ```ts
 * import { gcm } from '@noble/ciphers/aes.js';
 * import { managedNonce, randomBytes } from '@noble/ciphers/utils.js';
 * const wrapped = managedNonce(gcm);
 * const key = randomBytes(16);
 * const ciphertext = wrapped(key).encrypt(new Uint8Array([1, 2, 3]));
 * wrapped(key).decrypt(ciphertext);
 * ```
 */
export declare function managedNonce<T extends CipherWithNonce>(fn: T, randomBytes_?: typeof randomBytes): TRet<RemoveNonce<T>>;
/** `Uint8Array.of()` return type helper for TS 5.9. */
export type Uint8ArrayBuffer = TRet<Uint8Array>;
export {};
