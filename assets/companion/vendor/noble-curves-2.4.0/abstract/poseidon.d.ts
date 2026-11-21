/**
 * Implements [Poseidon](https://www.poseidon-hash.info) ZK-friendly hash.
 *
 * There are many poseidon variants with different constants.
 * We don't provide them: you should construct them manually.
 * Check out [micro-starknet](https://github.com/paulmillr/micro-starknet) package for a proper example.
 * @module
 */
/*! noble-curves - MIT License (c) 2022 Paul Miller (paulmillr.com) */
import { type TArg, type TRet } from '../utils.ts';
import { type IField } from './modular.ts';
/** Core Poseidon permutation parameters shared by all variants. */
export type PoseidonBasicOpts = {
    /** Prime field used by the permutation. */
    Fp: IField<bigint>;
    /** Poseidon width `t = rate + capacity`. */
    t: number;
    /** Number of full S-box rounds. */
    roundsFull: number;
    /** Number of partial S-box rounds. */
    roundsPartial: number;
    /** Whether to use the inverse S-box variant. */
    isSboxInverse?: boolean;
};
/** Poseidon settings used by the Grain-LFSR constant generator. */
export type PoseidonGrainOpts = PoseidonBasicOpts & {
    /** S-box power used while generating constants. */
    sboxPower?: number;
};
type PoseidonConstants = {
    mds: bigint[][];
    roundConstants: bigint[][];
};
/**
 * @param opts - Poseidon grain options. See {@link PoseidonGrainOpts}.
 * @param skipMDS - Number of MDS samples to skip.
 * @returns Generated constants.
 * @throws If the generated MDS matrix contains a zero denominator. {@link Error}
 * @example
 * Generate Poseidon round constants and an MDS matrix from the Grain LFSR.
 *
 * ```ts
 * import { grainGenConstants } from '@noble/curves/abstract/poseidon.js';
 * import { Field } from '@noble/curves/abstract/modular.js';
 * const Fp = Field(17n);
 * const constants = grainGenConstants({ Fp, t: 2, roundsFull: 8, roundsPartial: 8 });
 * ```
 */
export declare function grainGenConstants(opts: TArg<PoseidonGrainOpts>, skipMDS?: number): PoseidonConstants;
/** Fully specified Poseidon permutation options with explicit constants. */
export type PoseidonOpts = PoseidonBasicOpts & PoseidonConstants & {
    /** S-box power used by the permutation. */
    sboxPower?: number;
    /** Whether to reverse the partial-round S-box index. */
    reversePartialPowIdx?: boolean;
};
/**
 * @param opts - Poseidon options. See {@link PoseidonOpts}.
 * @returns Normalized poseidon options.
 * @throws If the Poseidon options, constants, or MDS matrix are invalid. {@link Error}
 * @example
 * Validate generated constants before constructing a permutation.
 *
 * ```ts
 * import { grainGenConstants, validateOpts } from '@noble/curves/abstract/poseidon.js';
 * import { Field } from '@noble/curves/abstract/modular.js';
 * const Fp = Field(17n);
 * const constants = grainGenConstants({ Fp, t: 2, roundsFull: 8, roundsPartial: 8 });
 * const opts = validateOpts({ ...constants, Fp, t: 2, roundsFull: 8, roundsPartial: 8, sboxPower: 3 });
 * ```
 */
export declare function validateOpts(opts: TArg<PoseidonOpts>): TRet<Readonly<{
    rounds: number;
    sboxFn: (n: bigint) => bigint;
    roundConstants: bigint[][];
    mds: bigint[][];
    Fp: IField<bigint>;
    t: number;
    roundsFull: number;
    roundsPartial: number;
    sboxPower?: number;
    reversePartialPowIdx?: boolean;
}>>;
/**
 * @param rc - Flattened round constants.
 * @param t - Poseidon width.
 * @returns Constants grouped by round.
 * @throws If the width or flattened constant array is invalid. {@link Error}
 * @example
 * Regroup a flat constant list into per-round chunks.
 *
 * ```ts
 * const rounds = splitConstants([1n, 2n, 3n, 4n], 2);
 * ```
 */
export declare function splitConstants(rc: bigint[], t: number): bigint[][];
/**
 * Poseidon permutation callable.
 * @param values - Poseidon state vector. Non-canonical bigints are normalized with `Fp.create(...)`.
 * @returns Permuted state vector.
 */
export type PoseidonFn = {
    (values: bigint[]): bigint[];
    /** Round constants captured by the permutation instance. */
    roundConstants: bigint[][];
};
/** Poseidon NTT-friendly hash. */
/**
 * @param opts - Poseidon options. See {@link PoseidonOpts}.
 * @returns Poseidon permutation.
 * @throws If the Poseidon options or state vector are invalid. {@link Error}
 * @example
 * Build a Poseidon permutation from validated parameters and constants.
 *
 * ```ts
 * import { grainGenConstants, poseidon } from '@noble/curves/abstract/poseidon.js';
 * import { Field } from '@noble/curves/abstract/modular.js';
 * const Fp = Field(17n);
 * const constants = grainGenConstants({ Fp, t: 2, roundsFull: 8, roundsPartial: 8 });
 * const hash = poseidon({ ...constants, Fp, t: 2, roundsFull: 8, roundsPartial: 8, sboxPower: 3 });
 * const state = hash([1n, 2n]);
 * ```
 */
export declare function poseidon(opts: TArg<PoseidonOpts>): PoseidonFn;
/**
 * @param Fp - Field implementation.
 * @param rate - Sponge rate.
 * @param capacity - Sponge capacity.
 * @param hash - Poseidon permutation.
 * @example
 * Wrap one Poseidon permutation in a sponge interface.
 *
 * ```ts
 * import { PoseidonSponge, grainGenConstants, poseidon } from '@noble/curves/abstract/poseidon.js';
 * import { Field } from '@noble/curves/abstract/modular.js';
 * const Fp = Field(17n);
 * const constants = grainGenConstants({ Fp, t: 2, roundsFull: 8, roundsPartial: 8 });
 * const hash = poseidon({ ...constants, Fp, t: 2, roundsFull: 8, roundsPartial: 8, sboxPower: 3 });
 * const sponge = new PoseidonSponge(Fp, 1, 1, hash);
 * sponge.absorb([1n]);
 * const out = sponge.squeeze(1);
 * ```
 */
export declare class PoseidonSponge {
    private Fp;
    readonly rate: number;
    readonly capacity: number;
    readonly hash: PoseidonFn;
    private state;
    private pos;
    private isAbsorbing;
    constructor(Fp: IField<bigint>, rate: number, capacity: number, hash: PoseidonFn);
    private process;
    absorb(input: bigint[]): void;
    squeeze(count: number): bigint[];
    clean(): void;
    clone(): PoseidonSponge;
}
/** Options for the non-standard but commonly used Poseidon sponge wrapper. */
export type PoseidonSpongeOpts = Omit<PoseidonOpts, 't'> & {
    /** Sponge rate. */
    rate: number;
    /** Sponge capacity. */
    capacity: number;
};
/**
 * The method is not defined in spec, but nevertheless used often.
 * Check carefully for compatibility: there are many edge cases, like absorbing an empty array.
 * We cross-test against:
 * - {@link https://github.com/ProvableHQ/snarkVM/tree/staging/algorithms | snarkVM algorithms}
 * - {@link https://github.com/arkworks-rs/crypto-primitives/tree/main | arkworks crypto-primitives}
 * @param opts - Sponge options. See {@link PoseidonSpongeOpts}.
 * @returns Factory for sponge instances.
 * @throws If the sponge dimensions or backing permutation options are invalid. {@link Error}
 * @example
 * Use the sponge helper to absorb several field elements and squeeze one digest.
 *
 * ```ts
 * import { grainGenConstants, poseidonSponge } from '@noble/curves/abstract/poseidon.js';
 * import { Field } from '@noble/curves/abstract/modular.js';
 * const Fp = Field(17n);
 * const constants = grainGenConstants({ Fp, t: 2, roundsFull: 8, roundsPartial: 8 });
 * const makeSponge = poseidonSponge({
 *   ...constants,
 *   Fp,
 *   rate: 1,
 *   capacity: 1,
 *   roundsFull: 8,
 *   roundsPartial: 8,
 *   sboxPower: 3,
 * });
 * const sponge = makeSponge();
 * sponge.absorb([1n]);
 * const out = sponge.squeeze(1);
 * ```
 */
export declare function poseidonSponge(opts: TArg<PoseidonSpongeOpts>): TRet<() => PoseidonSponge>;
export {};
