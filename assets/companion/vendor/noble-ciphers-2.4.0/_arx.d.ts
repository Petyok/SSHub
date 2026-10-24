/**
 * Basic utils for ARX (add-rotate-xor) salsa and chacha ciphers.

RFC8439 requires multi-step cipher stream, where
authKey starts with counter: 0, actual msg with counter: 1.

For this, we need a way to re-use nonce / counter:

    const counter = new Uint8Array(4);
    chacha(..., counter, ...); // counter is now 1
    chacha(..., counter, ...); // counter is now 2

This is complicated:

- 32-bit counters are enough, no need for 64-bit: max ArrayBuffer size in JS is 4GB
- Original papers don't allow mutating counters
- Counter overflow is undefined [^1]
- Idea A: allow providing (nonce | counter) instead of just nonce, re-use it
- Caveat: Cannot be re-used through all cases:
- * chacha has (counter | nonce)
- * xchacha has (nonce16 | counter | nonce16)
- Idea B: separate nonce / counter and provide separate API for counter re-use
- Caveat: there are different counter sizes depending on an algorithm.
- salsa & chacha also differ in structures of key & sigma:
  salsa20:      s[0] | k(4) | s[1] | nonce(2) | cnt(2) | s[2] | k(4) | s[3]
  chacha:       s(4) | k(8) | cnt(1) | nonce(3)
  chacha20orig: s(4) | k(8) | cnt(2) | nonce(2)
- Idea C: helper method such as `setSalsaState(key, nonce, sigma, data)`
- Caveat: we can't re-use counter array

xchacha uses the subkey and remaining 8 byte nonce with ChaCha20 as normal
(prefixed by 4 NUL bytes, since RFC8439 specifies a 12-byte nonce).
Counter overflow is undefined; see {@link https://mailarchive.ietf.org/arch/msg/cfrg/gsOnTJzcbgG6OqD8Sc0GO5aR_tU/ | the CFRG thread}.
Current noble policy is strict non-wrap for the shared 32-bit counter path:
exported ARX ciphers reject initial `0xffffffff` and stop before any implicit
wrap back to zero.
See {@link https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-xchacha#appendix-A.2 | the XChaCha appendix} for the extended-nonce construction.

 * @module
 */
import { type PRG, type TArg, type TRet, type XorStream } from './utils.ts';
/**
 * Rotates a 32-bit word left.
 * @param a - Input word.
 * @param b - Rotation count in bits.
 * @returns Rotated 32-bit word.
 * @example
 * Moves the top byte of `0x12345678` into the low byte position.
 * ```ts
 * rotl(0x12345678, 8);
 * ```
 */
export declare function rotl(a: number, b: number): number;
/**
 * ARX core function operating on 32-bit words. Ciphers must use u32 for efficiency.
 * @param sigma - Sigma constants for the selected cipher layout.
 * @param key - Expanded key words.
 * @param nonce - Nonce and counter words prepared for the round function.
 * @param output - Output block written in place.
 * @param counter - Block counter value.
 * @param rounds - Optional round count override.
 */
export type CipherCoreFn = (sigma: TArg<Uint32Array>, key: TArg<Uint32Array>, nonce: TArg<Uint32Array>, output: TArg<Uint32Array>, counter: number, rounds?: number) => void;
/**
 * Nonce-extension function used by XChaCha and XSalsa.
 * @param sigma - Sigma constants for the selected cipher layout.
 * @param key - Expanded key words.
 * @param input - Input nonce words used for subkey derivation.
 * @param output - Output buffer written with the derived nonce words.
 */
export type ExtendNonceFn = (sigma: TArg<Uint32Array>, key: TArg<Uint32Array>, input: TArg<Uint32Array>, output: TArg<Uint32Array>) => void;
/** ARX cipher options.
 * * `allowShortKeys` for 16-byte keys
 * * `counterLength` in bytes
 * * `counterRight`: right: `nonce|counter`; left: `counter|nonce`
 * */
export type CipherOpts = {
    /** Whether 16-byte keys are accepted for legacy Salsa and ChaCha variants. */
    allowShortKeys?: boolean;
    /** Optional nonce-expansion hook used by extended-nonce variants. */
    extendNonceFn?: ExtendNonceFn;
    /** Counter length in bytes inside the nonce/counter layout. */
    counterLength?: number;
    /** Whether the layout is `nonce|counter` instead of `counter|nonce`. */
    counterRight?: boolean;
    /** Number of core rounds to execute. */
    rounds?: number;
};
/**
 * Creates an ARX stream cipher from a 32-bit core permutation.
 * Used internally to build the exported Salsa and ChaCha stream ciphers.
 * @param core - Core function that fills one keystream block.
 * @param opts - Cipher layout and nonce-extension options. See {@link CipherOpts}.
 * @returns Stream cipher function over byte arrays.
 * @throws If the core callback, key size, counter, or output sizing is invalid. {@link Error}
 */
export declare function createCipher(core: TArg<CipherCoreFn>, opts: TArg<CipherOpts>): TRet<XorStream>;
/** Internal class which wraps chacha20 or chacha8 to create CSPRNG. */
export declare class _XorStreamPRG implements PRG {
    readonly blockLen: number;
    readonly keyLen: number;
    readonly nonceLen: number;
    private state;
    private buf;
    private key;
    private nonce;
    private pos;
    private ctr;
    private cipher;
    private destroyed;
    constructor(cipher: TArg<XorStream>, blockLen: number, keyLen: number, nonceLen: number, seed: TArg<Uint8Array>);
    private reseed;
    addEntropy(seed: TArg<Uint8Array>): void;
    randomBytes(len: number): TRet<Uint8Array>;
    clone(): _XorStreamPRG;
    clean(): void;
}
/**
 * PRG constructor backed by an ARX stream cipher.
 * @param seed - Optional seed bytes mixed into the initial state. When omitted, exactly 32
 * random bytes are mixed in by default: larger states keep a zero tail, while smaller states
 * wrap those bytes through `reseed()`'s XOR schedule.
 * @returns Seeded concrete `_XorStreamPRG` instance, including `clone()`.
 */
export type XorPRG = (seed?: TArg<Uint8Array>) => TRet<_XorStreamPRG>;
/**
 * Creates a PRG constructor from a stream cipher.
 * @param cipher - Stream cipher used to fill output blocks.
 * @param blockLen - Keystream block length in bytes.
 * @param keyLen - Internal key length in bytes.
 * @param nonceLen - Internal nonce length in bytes.
 * @returns PRG factory for seeded concrete `_XorStreamPRG` instances.
 * @example
 * Builds a PRG from XChaCha20 and reads bytes from a randomly seeded instance.
 * ```ts
 * import { xchacha20 } from '@noble/ciphers/chacha.js';
 * import { createPRG } from '@noble/ciphers/_arx.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const seed = randomBytes(32);
 * const init = createPRG(xchacha20, 64, 32, 24);
 * const prg = init(seed);
 * prg.randomBytes(8);
 * ```
 */
export declare const createPRG: (cipher: TArg<XorStream>, blockLen: number, keyLen: number, nonceLen: number) => TRet<XorPRG>;
