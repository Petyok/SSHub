/**
 * Towered extension fields.
 * Rather than implementing a massive 12th-degree extension directly, it is more efficient
 * to build it up from smaller extensions: a tower of extensions.
 *
 * For BLS12-381, the Fp12 field is implemented as a quadratic (degree two) extension,
 * on top of a cubic (degree three) extension, on top of a quadratic extension of Fp.
 *
 * For more info: "Pairings for beginners" by Costello, section 7.3.
 * @module
 */
/*! noble-curves - MIT License (c) 2022 Paul Miller (paulmillr.com) */
import { type TArg, type TRet } from '../utils.ts';
import * as mod from './modular.ts';
import type { WeierstrassPoint, WeierstrassPointCons } from './weierstrass.ts';
/** Pair of bigints used for quadratic-extension tuples. */
export type BigintTuple = [bigint, bigint];
/** Prime-field element. */
export type Fp = bigint;
/** Quadratic-extension field element `c0 + c1 * u`. */
export type Fp2 = {
    /** Real component. */
    c0: bigint;
    /** Imaginary component. */
    c1: bigint;
};
/** Six bigints used for sextic-extension tuples. */
export type BigintSix = [bigint, bigint, bigint, bigint, bigint, bigint];
/** Sextic-extension field element `c0 + c1 * v + c2 * v^2`. */
export type Fp6 = {
    /** Constant coefficient. */
    c0: Fp2;
    /** Linear coefficient. */
    c1: Fp2;
    /** Quadratic coefficient. */
    c2: Fp2;
};
/**
 * Degree-12 extension field element `c0 + c1 * w`.
 * Fp₁₂ = Fp₆² over Fp₂³, with Fp₆(w) / (w² - γ) where γ = v.
 */
export type Fp12 = {
    /** Constant coefficient. */
    c0: Fp6;
    /** Linear coefficient. */
    c1: Fp6;
};
/** Twelve bigints used for degree-12 extension tuples. */
export type BigintTwelve = [
    bigint,
    bigint,
    bigint,
    bigint,
    bigint,
    bigint,
    bigint,
    bigint,
    bigint,
    bigint,
    bigint,
    bigint
];
/** BLS-friendly helpers on top of the quadratic extension field. */
export type Fp2Bls = mod.IField<Fp2> & {
    /** Underlying prime field. */
    Fp: mod.IField<Fp>;
    /** Apply one Frobenius map. */
    frobeniusMap(num: Fp2, power: number): Fp2;
    /** Build one field element from a raw bigint tuple. */
    fromBigTuple(num: BigintTuple): Fp2;
    /** Multiply by the curve `b` constant. */
    mulByB: (num: Fp2) => Fp2;
    /** Multiply by the quadratic non-residue. */
    mulByNonresidue: (num: Fp2) => Fp2;
    /** Split one quadratic element into real and imaginary components. */
    reim: (num: Fp2) => {
        re: Fp;
        im: Fp;
    };
    /** Specialized helper used by sextic squaring formulas. */
    Fp4Square: (a: Fp2, b: Fp2) => {
        first: Fp2;
        second: Fp2;
    };
    /** Quadratic non-residue used by the extension. */
    NONRESIDUE: Fp2;
};
/** BLS-friendly helpers on top of the sextic extension field. */
export type Fp6Bls = mod.IField<Fp6> & {
    /** Underlying quadratic extension field. */
    Fp2: Fp2Bls;
    /** Apply one Frobenius map. */
    frobeniusMap(num: Fp6, power: number): Fp6;
    /** Build one field element from a raw six-bigint tuple. */
    fromBigSix: (tuple: BigintSix) => Fp6;
    /** Multiply by a sparse `(0, b1, 0)` sextic element. */
    mul1(num: Fp6, b1: Fp2): Fp6;
    /** Multiply by a sparse `(b0, b1, 0)` sextic element. */
    mul01(num: Fp6, b0: Fp2, b1: Fp2): Fp6;
    /** Multiply by one quadratic-extension element. */
    mulByFp2(lhs: Fp6, rhs: Fp2): Fp6;
    /** Multiply by the sextic non-residue. */
    mulByNonresidue: (num: Fp6) => Fp6;
};
/** BLS-friendly helpers on top of the degree-12 extension field. */
export type Fp12Bls = mod.IField<Fp12> & {
    /** Underlying sextic extension field. */
    Fp6: Fp6Bls;
    /** Apply one Frobenius map. */
    frobeniusMap(num: Fp12, power: number): Fp12;
    /** Build one field element from a raw twelve-bigint tuple. */
    fromBigTwelve: (t: BigintTwelve) => Fp12;
    /** Multiply by one quadratic-extension element. */
    mulByFp2(lhs: Fp12, rhs: Fp2): Fp12;
    /** Conjugate one degree-12 element. */
    conjugate(num: Fp12): Fp12;
    /** Apply the final exponentiation from pairing arithmetic. */
    finalExponentiate(num: Fp12): Fp12;
    /** Apply one cyclotomic square. */
    _cyclotomicSquare(num: Fp12): Fp12;
    /** Apply one cyclotomic exponentiation. */
    _cyclotomicExp(num: Fp12, n: bigint): Fp12;
};
declare function calcFrobeniusCoefficients<T>(Fp: TArg<mod.IField<T>>, nonResidue: T, modulus: bigint, degree: number, num?: number, divisor?: number): T[][];
export declare const __TEST: {
    calcFrobeniusCoefficients: typeof calcFrobeniusCoefficients;
};
/**
 * @param Fp - Base field implementation.
 * @param Fp2 - Quadratic extension field.
 * @param base - Twist-specific Frobenius base whose powers yield the `c1` / `c2` constants.
 * BLS12-381 uses `1 / NONRESIDUE`; BN254 uses `NONRESIDUE`.
 * @returns Frobenius endomorphism helpers.
 * @throws If the derived Frobenius constants are inconsistent for the tower. {@link Error}
 * @example
 * Build Frobenius endomorphism helpers for a BLS extension tower.
 *
 * ```ts
 * import { psiFrobenius } from '@noble/curves/abstract/tower.js';
 * import { bls12_381 } from '@noble/curves/bls12-381.js';
 * const Fp = bls12_381.fields.Fp;
 * const Fp2 = bls12_381.fields.Fp2;
 * const frob = psiFrobenius(Fp, Fp2, Fp2.div(Fp2.ONE, Fp2.NONRESIDUE));
 * const point = frob.G2psi(bls12_381.G2.Point, bls12_381.G2.Point.BASE);
 * ```
 */
export declare function psiFrobenius(Fp: TArg<mod.IField<Fp>>, Fp2: TArg<Fp2Bls>, base: TArg<Fp2>): {
    psi: (x: Fp2, y: Fp2) => [Fp2, Fp2];
    psi2: (x: Fp2, y: Fp2) => [Fp2, Fp2];
    G2psi: (c: WeierstrassPointCons<Fp2>, P: WeierstrassPoint<Fp2>) => WeierstrassPoint<Fp2>;
    G2psi2: (c: WeierstrassPointCons<Fp2>, P: WeierstrassPoint<Fp2>) => WeierstrassPoint<Fp2>;
    PSI_X: Fp2;
    PSI_Y: Fp2;
    PSI2_X: Fp2;
    PSI2_Y: Fp2;
};
/** Construction options for the BLS-style degree-12 tower. */
export type Tower12Opts = {
    /** Prime-field order. */
    ORDER: bigint;
    /** Bit length of the BLS parameter `x`. */
    X_LEN: number;
    /** Prime-field non-residue used by the quadratic extension. */
    NONRESIDUE?: Fp;
    /** Quadratic-extension non-residue used by the sextic tower. */
    FP2_NONRESIDUE: BigintTuple;
    /**
     * Optional custom quadratic square-root helper.
     * @param num - Quadratic-extension element.
     * @returns One square root.
     */
    Fp2sqrt?: (num: Fp2) => Fp2;
    /**
     * Multiply one quadratic element by the curve `b` constant.
     * @param num - Quadratic-extension element to scale.
     * @returns Product by the curve `b` constant.
     */
    Fp2mulByB: (num: Fp2) => Fp2;
    /**
     * Final exponentiation used by pairing arithmetic.
     * @param num - Degree-12 field element to exponentiate.
     * @returns Pairing result after final exponentiation.
     */
    Fp12finalExponentiate: (num: Fp12) => Fp12;
};
/**
 * @param opts - Tower construction options. See {@link Tower12Opts}.
 * @returns BLS tower fields.
 * @throws If the tower options or derived Frobenius helpers are invalid. {@link Error}
 * @example
 * Construct the Fp2/Fp6/Fp12 tower used by a pairing-friendly curve.
 *
 * ```ts
 * import { tower12, type Fp2, type Fp12 } from '@noble/curves/abstract/tower.js';
 * const fields = tower12({
 *   ORDER: 17n,
 *   X_LEN: 4,
 *   FP2_NONRESIDUE: [1n, 1n],
 *   Fp2mulByB: (num: Fp2) => num,
 *   Fp12finalExponentiate: (num: Fp12) => num,
 * });
 * const fp12 = fields.Fp12.ONE;
 * ```
 */
export declare function tower12(opts: TArg<Tower12Opts>): TRet<{
    Fp: Readonly<mod.IField<bigint> & Required<Pick<mod.IField<bigint>, 'isOdd'>>>;
    Fp2: Fp2Bls;
    Fp6: Fp6Bls;
    Fp12: Fp12Bls;
}>;
export {};
