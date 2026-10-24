import { type CHash } from '@noble/hashes/utils.js';
import { type Signer, type TArg, type TRet } from './utils.ts';
/**
 * * N: Security parameter (in bytes). W: Winternitz parameter
 * * H: Hypertree height. D: Hypertree layers
 * * K: FORS trees numbers. A: FORS trees height
 */
export type SphincsOpts = {
    /** Security parameter in bytes. */
    N: number;
    /** Winternitz parameter. */
    W: number;
    /** Total hypertree height. */
    H: number;
    /** Number of hypertree layers. */
    D: number;
    /** Number of FORS trees. */
    K: number;
    /** Height of each FORS tree. */
    A: number;
    /** Target security level in bits. */
    securityLevel: number;
};
/** Hash customization options for SLH-DSA context creation. */
export type SphincsHashOpts = {
    /** Whether to use the compressed-address variant from the standard. */
    isCompressed?: boolean;
    /** Factory that binds one parameter set to one per-key hash context generator. */
    getContext: GetContext;
};
/** Winternitz signature params. */
/**
 * Built-in SLH-DSA Table 2 subset keyed by strength/profile.
 * SHA2 and SHAKE pairs share the same numeric rows here, so the hash family is chosen separately.
 * `securityLevel` stores 128/192/256-bit strengths for `checkHash(...)`,
 * not Table 2's category labels 1/3/5.
 * Other Table 2 columns such as `m`, public-key bytes, and signature bytes
 * stay derived at the export layer.
 */
export declare const PARAMS: Record<string, SphincsOpts>;
/** Address byte array of size `ADDR_BYTES`. */
export type ADRS = Uint8Array;
/** Hash and tweakable-hash callbacks bound to one SLH-DSA keypair context.
 * Buffer-aliasing contract: `PRFaddr`, `thash1` and `thashN` return views into per-context
 * scratch buffers (one per lane), so callers must consume or copy a result before the next
 * call on the same lane. `clean()` wipes the scratch buffers along with the hash states.
 */
export type Context = {
    /**
     * Derive a PRF output for one address.
     * @param addr - Address bytes.
     * @returns PRF output bytes (scratch view; copy to retain).
     */
    PRFaddr: (addr: TArg<ADRS>) => TRet<Uint8Array>;
    /**
     * Derive the randomized message hash prefix.
     * @param skPRF - Secret PRF seed.
     * @param random - Per-signature randomness.
     * @param msg - Message bytes.
     * @returns PRF output bytes.
     */
    PRFmsg: (skPRF: TArg<Uint8Array>, random: TArg<Uint8Array>, msg: TArg<Uint8Array>) => TRet<Uint8Array>;
    /**
     * Hash one randomized message transcript.
     * @param R - Randomized message prefix.
     * @param pk - Public key bytes.
     * @param m - Message bytes.
     * @param outLen - Output length in bytes.
     * @returns Transcript hash bytes.
     */
    Hmsg: (R: TArg<Uint8Array>, pk: TArg<Uint8Array>, m: TArg<Uint8Array>, outLen: number) => TRet<Uint8Array>;
    /**
     * Tweakable hash over one input block.
     * @param input - Input block.
     * @param addr - Address bytes.
     * @returns Hash output bytes.
     */
    thash1: (input: TArg<Uint8Array>, addr: TArg<ADRS>) => TRet<Uint8Array>;
    /**
     * Tweakable hash over multiple input blocks.
     * @param blocks - Number of input blocks.
     * @param input - Concatenated input bytes.
     * @param addr - Address bytes.
     * @returns Hash output bytes.
     */
    thashN: (blocks: number, input: TArg<Uint8Array>, addr: TArg<ADRS>) => TRet<Uint8Array>;
    /** Wipe any buffered hash state for the current context. */
    clean: () => void;
};
/** Factory that creates a context generator for one SLH-DSA parameter set. */
export type GetContext = (opts: SphincsOpts) => (pub_seed: TArg<Uint8Array>, sk_seed?: TArg<Uint8Array>) => TRet<Context>;
/** Public SLH-DSA signer with prehash customization. */
export type SphincsSigner = Signer & {
    internal: TRet<Signer>;
    securityLevel: number;
    prehash: (hash: TArg<CHash>) => TRet<Signer>;
};
/**
 * SLH-DSA-SHAKE-128f: Table 2 row `n=16, h=66, d=22, h'=3, a=6, k=33, lg w=4, m=34`;
 * lengths `publicKey=32`, `secretKey=64`, `signature=17088`, `seed=48`, `signRand=16`.
 * Also exposes `.prehash(...)`.
 */
export declare const slh_dsa_shake_128f: TRet<SphincsSigner>;
/**
 * SLH-DSA-SHAKE-128s: Table 2 row `n=16, h=63, d=7, h'=9, a=12, k=14, lg w=4, m=30`;
 * lengths `publicKey=32`, `secretKey=64`, `signature=7856`, `seed=48`, `signRand=16`.
 * Also exposes `.prehash(...)`.
 */
export declare const slh_dsa_shake_128s: TRet<SphincsSigner>;
/**
 * SLH-DSA-SHAKE-192f: Table 2 row `n=24, h=66, d=22, h'=3, a=8, k=33, lg w=4, m=42`;
 * lengths `publicKey=48`, `secretKey=96`, `signature=35664`, `seed=72`, `signRand=24`.
 * Also exposes `.prehash(...)`.
 */
export declare const slh_dsa_shake_192f: TRet<SphincsSigner>;
/**
 * SLH-DSA-SHAKE-192s: Table 2 row `n=24, h=63, d=7, h'=9, a=14, k=17, lg w=4, m=39`;
 * lengths `publicKey=48`, `secretKey=96`, `signature=16224`, `seed=72`, `signRand=24`.
 * Also exposes `.prehash(...)`.
 */
export declare const slh_dsa_shake_192s: TRet<SphincsSigner>;
/**
 * SLH-DSA-SHAKE-256f: Table 2 row `n=32, h=68, d=17, h'=4, a=9, k=35, lg w=4, m=49`;
 * lengths `publicKey=64`, `secretKey=128`, `signature=49856`, `seed=96`, `signRand=32`.
 * Also exposes `.prehash(...)`.
 */
export declare const slh_dsa_shake_256f: TRet<SphincsSigner>;
/**
 * SLH-DSA-SHAKE-256s: Table 2 row `n=32, h=64, d=8, h'=8, a=14, k=22, lg w=4, m=47`;
 * lengths `publicKey=64`, `secretKey=128`, `signature=29792`, `seed=96`, `signRand=32`.
 * Also exposes `.prehash(...)`.
 */
export declare const slh_dsa_shake_256s: TRet<SphincsSigner>;
/**
 * SLH-DSA-SHA2-128f: Table 2 row `n=16, h=66, d=22, h'=3, a=6, k=33, lg w=4, m=34`;
 * lengths `publicKey=32`, `secretKey=64`, `signature=17088`, `seed=48`, `signRand=16`.
 * Also exposes `.prehash(...)`.
 * @example
 * Generate deterministic SLH-DSA keys, sign one message, and verify the signature.
 * ```ts
 * import { sha256 } from '@noble/hashes/sha2.js';
 * import { slh_dsa_sha2_128f } from '@noble/post-quantum/slh-dsa.js';
 * const seed = new Uint8Array(slh_dsa_sha2_128f.lengths.seed!);
 * const { secretKey, publicKey } = slh_dsa_sha2_128f.keygen(seed);
 * const msg = new TextEncoder().encode('hello noble');
 * const sig = slh_dsa_sha2_128f.sign(msg, secretKey);
 * const isValid = slh_dsa_sha2_128f.verify(sig, msg, publicKey);
 * const recovered = slh_dsa_sha2_128f.getPublicKey(secretKey);
 * const context = new Uint8Array([1, 2, 3]);
 * const prehash = slh_dsa_sha2_128f.prehash(sha256);
 * const preSig = prehash.sign(msg, secretKey, { context });
 * const preValid = prehash.verify(preSig, msg, publicKey, { context });
 * const internalSig = slh_dsa_sha2_128f.internal.sign(msg, secretKey);
 * ```
 */
export declare const slh_dsa_sha2_128f: TRet<SphincsSigner>;
/**
 * SLH-DSA-SHA2-128s: Table 2 row `n=16, h=63, d=7, h'=9, a=12, k=14, lg w=4, m=30`;
 * lengths `publicKey=32`, `secretKey=64`, `signature=7856`, `seed=48`, `signRand=16`.
 * Also exposes `.prehash(...)`.
 */
export declare const slh_dsa_sha2_128s: TRet<SphincsSigner>;
/**
 * SLH-DSA-SHA2-192f: Table 2 row `n=24, h=66, d=22, h'=3, a=8, k=33, lg w=4, m=42`;
 * lengths `publicKey=48`, `secretKey=96`, `signature=35664`, `seed=72`, `signRand=24`.
 * Also exposes `.prehash(...)`.
 */
export declare const slh_dsa_sha2_192f: TRet<SphincsSigner>;
/**
 * SLH-DSA-SHA2-192s: Table 2 row `n=24, h=63, d=7, h'=9, a=14, k=17, lg w=4, m=39`;
 * lengths `publicKey=48`, `secretKey=96`, `signature=16224`, `seed=72`, `signRand=24`.
 * Also exposes `.prehash(...)`.
 */
export declare const slh_dsa_sha2_192s: TRet<SphincsSigner>;
/**
 * SLH-DSA-SHA2-256f: Table 2 row `n=32, h=68, d=17, h'=4, a=9, k=35, lg w=4, m=49`;
 * lengths `publicKey=64`, `secretKey=128`, `signature=49856`, `seed=96`, `signRand=32`.
 * Also exposes `.prehash(...)`.
 */
export declare const slh_dsa_sha2_256f: TRet<SphincsSigner>;
/**
 * SLH-DSA-SHA2-256s: Table 2 row `n=32, h=64, d=8, h'=8, a=14, k=22, lg w=4, m=47`;
 * lengths `publicKey=64`, `secretKey=128`, `signature=29792`, `seed=96`, `signRand=32`.
 * Also exposes `.prehash(...)`.
 */
export declare const slh_dsa_sha2_256s: TRet<SphincsSigner>;
