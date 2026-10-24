import { type Cipher, type CipherWithOutput, type CMac, type IHash2, type PRG, type TArg, type TRet, type Uint8ArrayBuffer } from './utils.ts';
/**
 * Increments a counter block with wrap around.
 * AES call sites here currently use the big-endian branch, but the helper supports both layouts.
 * NIST SP 800-38A Appendix B.1 and SP 800-38D §6.2 increment the
 * least-significant/rightmost bits.
 * `isLE=false` matches that standard counter-block layout, while `isLE=true`
 * is a generic extension for non-AES callers.
 * The implementation keeps a 32-bit bitwise carry path, so `carry` is capped at `0xffffff00`;
 * larger values throw instead of silently overflowing before the next-byte propagation step.
 */
declare const incBytes: (data: TArg<Uint8Array>, isLE: boolean, carry?: number) => void;
/** Forward AES key expansion used across ECB/CBC/CTR/GCM/CMAC/KW-style paths. */
declare function expandKeyLE(key: TArg<Uint8Array>): TRet<Uint32Array>;
declare function expandKeyDecLE(key: TArg<Uint8Array>): TRet<Uint32Array>;
declare function encrypt(xk: TArg<Uint32Array>, s0: number, s1: number, s2: number, s3: number): {
    s0: number;
    s1: number;
    s2: number;
    s3: number;
};
declare function decrypt(xk: TArg<Uint32Array>, s0: number, s1: number, s2: number, s3: number): {
    s0: number;
    s1: number;
    s2: number;
    s3: number;
};
declare function ctrCounter(xk: TArg<Uint32Array>, nonce: TArg<Uint8Array>, src: TArg<Uint8Array>, dst?: TArg<Uint8Array>): TRet<Uint8Array>;
declare function ctr32(xk: TArg<Uint32Array>, isLE: boolean, nonce: TArg<Uint8Array>, src: TArg<Uint8Array>, dst?: TArg<Uint8Array>): TRet<Uint8Array>;
/**
 * **CTR** (Counter Mode): turns a block cipher into a stream cipher using a
 * full 16-byte counter block.
 * Efficient and parallelizable. Requires a unique nonce per encryption. Unauthenticated: needs MAC.
 * @param key - AES key bytes.
 * @param nonce - 16-byte counter block, incremented as a full AES block.
 * @returns Cipher instance with `encrypt()` and `decrypt()`.
 * @example
 * Encrypts a short payload with a fresh AES key and counter block.
 *
 * ```ts
 * import { ctr } from '@noble/ciphers/aes.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const key = randomBytes(16);
 * const nonce = randomBytes(16);
 * const cipher = ctr(key, nonce);
 * cipher.encrypt(new Uint8Array([1, 2, 3]));
 * ```
 */
export declare const ctr: TRet<((key: TArg<Uint8Array>, nonce: TArg<Uint8Array>) => CipherWithOutput) & {
    blockSize: number;
    nonceLength: number;
}>;
/** Options for ECB and CBC. */
export type BlockOpts = {
    /**
     * Disable the library's PKCS#7 padding/unpadding layer and require exact-block inputs.
     * Use this when a higher-level authenticated protocol handles padding itself.
     */
    disablePadding?: boolean;
};
/**
 * **ECB** (Electronic Codebook): Deterministic encryption; identical plaintext blocks yield
 * identical ciphertexts. Not secure due to pattern leakage.
 * See {@link https://words.filippo.io/the-ecb-penguin/ | the AES Penguin}.
 * @param key - AES key bytes.
 * @param opts - Padding options. See {@link BlockOpts}.
 * @returns Cipher instance with `encrypt()` and `decrypt()`.
 * @example
 * Shows the basic ECB encrypt call shape with a fresh key; avoid ECB in new designs.
 *
 * ```ts
 * import { ecb } from '@noble/ciphers/aes.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const key = randomBytes(16);
 * const cipher = ecb(key);
 * cipher.encrypt(new Uint8Array([1, 2, 3]));
 * ```
 */
export declare const ecb: TRet<((key: TArg<Uint8Array>, opts?: BlockOpts) => CipherWithOutput) & {
    blockSize: number;
}>;
/**
 * **CBC** (Cipher Block Chaining): Each plaintext block is XORed with the
 * previous block of ciphertext before encryption.
 * Hard to use: requires proper padding and an unpredictable IV. Unauthenticated: needs MAC.
 * Bad padding is reported as decrypt failure, which can be a padding oracle if exposed.
 * @param key - AES key bytes.
 * @param iv - 16-byte unpredictable initialization vector.
 * @param opts - Padding options. See {@link BlockOpts}.
 * @returns Cipher instance with `encrypt()` and `decrypt()`.
 * @example
 * Encrypts a padded message with a fresh key and 16-byte IV.
 *
 * ```ts
 * import { cbc } from '@noble/ciphers/aes.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const key = randomBytes(16);
 * const iv = randomBytes(16);
 * const cipher = cbc(key, iv);
 * cipher.encrypt(new Uint8Array([1, 2, 3]));
 * ```
 */
export declare const cbc: TRet<((key: TArg<Uint8Array>, iv: TArg<Uint8Array>, opts?: BlockOpts) => CipherWithOutput) & {
    blockSize: number;
    nonceLength: number;
}>;
/**
 * CFB (CFB-128): Cipher Feedback Mode with 128-bit segments. The input for the
 * block cipher is the previous cipher output.
 * Unauthenticated: needs MAC.
 * @param key - AES key bytes.
 * @param iv - 16-byte unpredictable initialization vector.
 * @returns Cipher instance with `encrypt()` and `decrypt()`.
 * @example
 * Encrypts a short message with feedback mode and a fresh key/IV pair.
 *
 * ```ts
 * import { cfb } from '@noble/ciphers/aes.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const key = randomBytes(16);
 * const iv = randomBytes(16);
 * const cipher = cfb(key, iv);
 * cipher.encrypt(new Uint8Array([1, 2, 3]));
 * ```
 */
export declare const cfb: TRet<((key: TArg<Uint8Array>, iv: TArg<Uint8Array>) => CipherWithOutput) & {
    blockSize: number;
    nonceLength: number;
}>;
/**
 * **GCM** (Galois/Counter Mode): Combines CTR mode with polynomial MAC. Efficient and widely used.
 * Not perfect:
 * a) conservative key wear-out is `2**32` (4B) msgs.
 * b) key wear-out under random nonces is even smaller: `2**23` (8M) messages for `2**-50` chance.
 * c) MAC can be forged: see Poly1305 documentation.
 * @param key - AES key bytes.
 * @param nonce - Nonce bytes (12 recommended, minimum 8; other lengths use GHASH J0 derivation).
 * @param AAD - Additional authenticated data.
 * @returns AEAD cipher instance with a fixed 16-byte tag.
 * @example
 * Encrypts and authenticates plaintext with a fresh key and 12-byte nonce.
 *
 * ```ts
 * import { gcm } from '@noble/ciphers/aes.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const key = randomBytes(16);
 * const nonce = randomBytes(12);
 * const aad = new TextEncoder().encode('session metadata');
 * const cipher = gcm(key, nonce, aad);
 * cipher.encrypt(new Uint8Array([1, 2, 3]));
 * ```
 */
export declare const gcm: TRet<((key: TArg<Uint8Array>, nonce: TArg<Uint8Array>, AAD?: TArg<Uint8Array>) => Cipher) & {
    blockSize: number;
    nonceLength: number;
    tagLength: number;
    withAAD: true;
    varSizeNonce: true;
}>;
/**
 * **SIV** (Synthetic IV): GCM with nonce-misuse resistance.
 * Repeating nonces reveal only the fact plaintexts are identical.
 * Also suffers from GCM issues: key wear-out limits & MAC forging.
 * See {@link https://www.rfc-editor.org/rfc/rfc8452 | RFC 8452}.
 * RFC 8452 defines 16-byte and 32-byte AES keys for this mode.
 * This implementation also accepts 24-byte AES-192 keys as a local
 * extension; see the inline comment next to `validateKeyLength(key)` below
 * for the exact scope note.
 * @param key - AES key bytes.
 * @param nonce - 12-byte nonce.
 * @param AAD - Additional authenticated data.
 * @returns AEAD cipher instance.
 * @example
 * Encrypts and authenticates plaintext with a fresh key and nonce, while tolerating reuse.
 *
 * ```ts
 * import { gcmsiv } from '@noble/ciphers/aes.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const key = randomBytes(16);
 * const nonce = randomBytes(12);
 * const aad = new TextEncoder().encode('session metadata');
 * const cipher = gcmsiv(key, nonce, aad);
 * cipher.encrypt(new Uint8Array([1, 2, 3]));
 * ```
 */
export declare const gcmsiv: TRet<((key: TArg<Uint8Array>, nonce: TArg<Uint8Array>, AAD?: TArg<Uint8Array>) => Cipher) & {
    blockSize: number;
    nonceLength: number;
    tagLength: number;
    withAAD: true;
    varSizeNonce: true;
}>;
declare function encryptBlock(xk: TArg<Uint32Array>, block: TArg<Uint8Array>): TRet<Uint8Array>;
declare function decryptBlock(xk: TArg<Uint32Array>, block: TArg<Uint8Array>): TRet<Uint8Array>;
/**
 * AES-KW (key-wrap). Injects static IV into plaintext, adds counter, encrypts 6 times.
 * Reduces block size from 16 to 8 bytes.
 * Plaintext must be a non-empty multiple of 8 bytes with minimum 16 bytes.
 * 8-byte inputs use aeskwp.
 * Wrapped ciphertext must be a multiple of 8 bytes with minimum 24 bytes.
 * For padded version, use aeskwp.
 * See {@link https://www.rfc-editor.org/rfc/rfc3394/ | RFC 3394} and
 * {@link https://nvlpubs.nist.gov/nistpubs/SpecialPublications/NIST.SP.800-38F.pdf | NIST SP 800-38F}.
 * @param kek - AES key-encryption key.
 * @returns Key-wrap cipher instance.
 * As with other `wrapCipher(...)` wrappers, `encrypt()` is single-use per
 * instance.
 * @example
 * Wraps a 128-bit content-encryption key with a fresh key-encryption key.
 *
 * ```ts
 * import { aeskw } from '@noble/ciphers/aes.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const kek = randomBytes(16);
 * const cek = randomBytes(16);
 * const wrap = aeskw(kek);
 * wrap.encrypt(cek);
 * ```
 */
export declare const aeskw: TRet<((kek: TArg<Uint8Array>) => Cipher) & {
    blockSize: number;
}>;
/**
 * AES-KW, but with padding and allows random keys.
 * Uses the RFC 5649 alternative initial value; the second u32 stores the
 * 32-bit MLI in network order.
 * Wrapped ciphertext must be at least 16 bytes; malformed lengths are
 * rejected during AIV/padding checks.
 * See {@link https://www.rfc-editor.org/rfc/rfc5649 | RFC 5649}.
 * @param kek - AES key-encryption key.
 * @returns Padded key-wrap cipher instance.
 * @example
 * Wraps a short key blob using the padded variant and a fresh key-encryption key.
 *
 * ```ts
 * import { aeskwp } from '@noble/ciphers/aes.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const kek = randomBytes(16);
 * const wrap = aeskwp(kek);
 * wrap.encrypt(new Uint8Array([1, 2, 3]));
 * ```
 */
export declare const aeskwp: TRet<((kek: TArg<Uint8Array>) => Cipher) & {
    blockSize: number;
}>;
declare class _AesCtrDRBG implements PRG {
    readonly blockLen: number;
    private key;
    private nonce;
    private state;
    private reseedCnt;
    private destroyed;
    constructor(keyLen: number, seed: TArg<Uint8Array>, personalization?: TArg<Uint8Array>);
    private update;
    addEntropy(seed: TArg<Uint8Array>, info?: TArg<Uint8Array>): void;
    randomBytes(len: number, info?: TArg<Uint8Array>): TRet<Uint8Array>;
    clean(): void;
}
/**
 * Factory for AES-CTR DRBG instances.
 * @param seed - Initial entropy input.
 * @param personalization - Optional personalization string mixed into the state.
 * @returns Seeded AES-CTR DRBG instance.
 */
export type AesCtrDrbg = (seed: TArg<Uint8Array>, personalization?: TArg<Uint8Array>) => TRet<_AesCtrDRBG>;
/**
 * AES-CTR DRBG 128-bit - CSPRNG (cryptographically secure pseudorandom number generator).
 * It's best to limit usage to non-production, non-critical cases: for example, test-only.
 * @param seed - Initial 32-byte entropy input.
 * @param personalization - Optional personalization string.
 * @returns Seeded DRBG instance. The concrete methods also accept optional additional-input bytes.
 * @example
 * Seeds the test-only AES-CTR DRBG from fresh entropy and reads bytes from it.
 *
 * ```ts
 * import { rngAesCtrDrbg128 } from '@noble/ciphers/aes.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const seed = randomBytes(32);
 * const prg = rngAesCtrDrbg128(seed);
 * prg.randomBytes(8);
 * ```
 */
export declare const rngAesCtrDrbg128: TRet<AesCtrDrbg>;
/**
 * AES-CTR DRBG 256-bit - CSPRNG (cryptographically secure pseudorandom number generator).
 * It's best to limit usage to non-production, non-critical cases: for example, test-only.
 * @param seed - Initial 48-byte entropy input.
 * @param personalization - Optional personalization string.
 * @returns Seeded DRBG instance. The concrete methods also accept optional additional-input bytes.
 * @example
 * Seeds the test-only AES-CTR DRBG from fresh entropy and reads bytes from it.
 *
 * ```ts
 * import { rngAesCtrDrbg256 } from '@noble/ciphers/aes.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const seed = randomBytes(48);
 * const prg = rngAesCtrDrbg256(seed);
 * prg.randomBytes(8);
 * ```
 */
export declare const rngAesCtrDrbg256: TRet<AesCtrDrbg>;
/**
 * Left-shift by one bit and conditionally XOR with 0x87:
 * ```
 * if MSB(L) is equal to 0
 * then    K1 := L << 1;
 * else    K1 := (L << 1) XOR const_Rb;
 * ```
 *
 * Specs:
 * {@link https://www.rfc-editor.org/rfc/rfc4493.html#section-2.3 | RFC 4493 Section 2.3},
 * {@link https://datatracker.ietf.org/doc/html/rfc5297.html#section-2.3 | RFC 5297 Section 2.3}
 *
 * @returns modified `block` (for chaining)
 */
declare function dbl<T extends Uint8Array>(block: T): T;
/**
 * `a XOR b`, running in-place on `a`.
 * @param a left operand and output
 * @param b right operand
 * @returns `a` (for chaining)
 */
declare function xorBlock<T extends TArg<Uint8Array>>(a: T, b: TArg<Uint8Array>): T;
/**
 * xorend as defined in
 * {@link https://datatracker.ietf.org/doc/html/rfc5297.html#section-2.1 | RFC 5297 Section 2.1}.
 *
 * ```
 * leftmost(A, len(A)-len(B)) || (rightmost(A, len(B)) xor B)
 * ```
 *
 * Mutates `a` in place so the left prefix stays untouched and only the
 * rightmost `len(B)` bytes are xored with `b`.
 */
declare function xorend<T extends TArg<Uint8Array>>(a: T, b: TArg<Uint8Array>): T;
/**
 * Internal CMAC class.
 */
declare class _CMAC implements IHash2 {
    readonly blockLen: number;
    readonly outputLen: number;
    private buffer;
    private pos;
    protected finished: boolean;
    protected destroyed: boolean;
    private k1;
    private k2;
    private x;
    private x32;
    private xk;
    constructor(key: TArg<Uint8Array>);
    private process;
    update(data: TArg<Uint8Array>): this;
    digestInto(out: TArg<Uint8Array>): void;
    digest(): Uint8ArrayBuffer;
    destroy(): void;
}
/**
 * AES-CMAC (Cipher-based Message Authentication Code).
 * Specs: {@link https://www.rfc-editor.org/rfc/rfc4493.html | RFC 4493}.
 * @param msg - Message bytes to authenticate.
 * @param key - AES key bytes.
 * @returns 16-byte authentication tag. `cmac.create(...)` follows the same incremental MAC shape as
 * the other keyed helpers in this repo, including `blockLen`,
 * `outputLen`, `digestInto()` and `destroy()`.
 * @example
 * Authenticates a message with AES-CMAC and a fresh key.
 *
 * ```ts
 * import { cmac } from '@noble/ciphers/aes.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const key = randomBytes(16);
 * cmac(new Uint8Array(), key);
 * ```
 */
export declare const cmac: TRet<CMac<_CMAC>>;
/**
 * S2V (Synthetic Initialization Vector) function as described in
 * {@link https://datatracker.ietf.org/doc/html/rfc5297.html#section-2.4 | RFC 5297 Section 2.4}.
 *
 * ```
 * S2V(K, S1, ..., Sn) {
 *   if n = 0 then
 *     return V = AES-CMAC(K, <one>)
 *   fi
 *   D = AES-CMAC(K, <zero>)
 *   for i = 1 to n-1 do
 *     D = dbl(D) xor AES-CMAC(K, Si)
 *   done
 *   if len(Sn) >= 128 then
 *     T = Sn xorend D
 *   else
 *     T = dbl(D) xor pad(Sn)
 *   fi
 *   return V = AES-CMAC(K, T)
 * }
 * ```
 *
 * S2V takes a key and a vector of strings S1, S2, ..., Sn and returns a 128-bit string.
 * The S2V function is used to generate a synthetic IV for AES-SIV.
 *
 * @param key - AES key (128, 192, or 256 bits)
 * @param strings - Array of byte arrays to process
 * @returns 128-bit synthetic IV
 */
declare function s2v(key: TArg<Uint8Array>, strings: TArg<Uint8Array[]>): TRet<Uint8Array>;
/**
 * Use `gcmsiv` or `aessiv`.
 * @returns Never; always throws with the migration hint.
 * @throws If called; `siv()` is a removed v1 alias. {@link Error}
 * @example
 * `siv()` was removed in v2; use `gcmsiv()` for nonce-based SIV instead.
 *
 * ```ts
 * import { gcmsiv } from '@noble/ciphers/aes.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const key = randomBytes(16);
 * const nonce = randomBytes(12);
 * const cipher = gcmsiv(key, nonce);
 * cipher.encrypt(new Uint8Array([1, 2, 3]));
 * ```
 */
export declare const siv: () => never;
/**
 * **SIV**: Synthetic Initialization Vector (SIV) Authenticated Encryption
 * Nonce is derived from the plaintext and AAD using the S2V function.
 * Supports at most 126 AAD components. RFC 5297 nonce-based use is expressed by
 * passing the nonce as the final AAD component before the plaintext.
 * See {@link https://datatracker.ietf.org/doc/html/rfc5297.html | RFC 5297}.
 * @param key - 32-byte, 48-byte, or 64-byte key.
 * @param AAD - Additional authenticated data chunks (up to 126).
 * @returns AEAD cipher instance.
 * @example
 * Authenticates and encrypts plaintext with a fresh key without requiring unique nonces.
 *
 * ```ts
 * import { aessiv } from '@noble/ciphers/aes.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const key = randomBytes(32);
 * const cipher = aessiv(key);
 * cipher.encrypt(new Uint8Array([1, 2, 3]));
 * ```
 */
export declare const aessiv: TRet<((key: TArg<Uint8Array>, ...AAD: TArg<Uint8Array[]>) => Cipher) & {
    blockSize: number;
    tagLength: number;
    withAAD: true;
}>;
/**
 * Unsafe low-level internal methods. May change at any time.
 * Callers are expected to use reviewed expanded-key outputs, pass mutable and
 * aligned 16-byte blocks where required, and treat several helpers as in-place
 * mutations of their input buffers or counters.
 */
export declare const unsafe: {
    expandKeyLE: typeof expandKeyLE;
    expandKeyDecLE: typeof expandKeyDecLE;
    encrypt: typeof encrypt;
    decrypt: typeof decrypt;
    encryptBlock: typeof encryptBlock;
    decryptBlock: typeof decryptBlock;
    ctrCounter: typeof ctrCounter;
    ctr32: typeof ctr32;
    dbl: typeof dbl;
    xorBlock: typeof xorBlock;
    xorend: typeof xorend;
    s2v: typeof s2v;
};
export declare const __TESTS: {
    incBytes: typeof incBytes;
};
export {};
