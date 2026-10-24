/**
 * Poly1305 ({@link https://cr.yp.to/mac/poly1305-20050329.pdf | PDF},
 * {@link https://en.wikipedia.org/wiki/Poly1305 | wiki})
 * is a fast and parallel secret-key message-authentication code suitable for
 * a wide variety of applications. It was standardized in
 * {@link https://www.rfc-editor.org/rfc/rfc8439 | RFC 8439} and is now used in TLS 1.3.
 *
 * Polynomial MACs are not perfect for every situation:
 * they lack Random Key Robustness: the MAC can be forged, and can't be used in PAKE schemes.
 * See {@link https://keymaterial.net/2020/09/07/invisible-salamanders-in-aes-gcm-siv/ | the invisible salamanders attack writeup}.
 * To combat invisible salamanders, `hash(key)` can be included in ciphertext,
 * however, this would violate ciphertext indistinguishability:
 * an attacker would know which key was used - so `HKDF(key, i)`
 * could be used instead.
 *
 * Check out the {@link https://cr.yp.to/mac.html | original website}.
 * Based on public-domain {@link https://github.com/floodyberry/poly1305-donna | poly1305-donna}.
 * @module
 */
import { type CMac, type IHash2, type TArg, type TRet } from './utils.ts';
/**
 * Incremental Poly1305 MAC state.
 * Prefer `poly1305()` for one-shot use.
 * @param key - 32-byte Poly1305 one-time key.
 * @example
 * Feeds one chunk into an incremental Poly1305 state with a fresh one-time key.
 *
 * ```ts
 * import { Poly1305 } from '@noble/ciphers/_poly1305.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const key = randomBytes(32);
 * const mac = new Poly1305(key);
 * mac.update(new Uint8Array([1, 2, 3]));
 * mac.digest();
 * ```
 */
export declare class Poly1305 implements IHash2 {
    readonly blockLen = 16;
    readonly outputLen = 16;
    private buffer;
    private r;
    private h;
    private pad;
    private pos;
    protected finished: boolean;
    protected destroyed: boolean;
    constructor(key: TArg<Uint8Array>);
    private process;
    private finalize;
    update(data: TArg<Uint8Array>): this;
    destroy(): void;
    digestInto(out: TArg<Uint8Array>): void;
    digest(): TRet<Uint8Array>;
}
/** One-shot keyed hash helper with `.create()`. */
export type CHash = CMac<Poly1305>;
/**
 * Poly1305 MAC from RFC 8439.
 * @param msg - Message bytes to authenticate.
 * @param key - 32-byte Poly1305 one-time key.
 * @returns 16-byte authentication tag.
 * @example
 * Authenticates one message with a one-shot Poly1305 call and a fresh key.
 *
 * ```ts
 * import { poly1305 } from '@noble/ciphers/_poly1305.js';
 * import { randomBytes } from '@noble/ciphers/utils.js';
 * const key = randomBytes(32);
 * poly1305(new Uint8Array(), key);
 * ```
 */
export declare const poly1305: TRet<CHash>;
