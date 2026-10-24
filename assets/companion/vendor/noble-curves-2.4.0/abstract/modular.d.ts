/**
 * Utils for modular division and fields.
 * Field over 11 is a finite (Galois) field is integer number operations `mod 11`.
 * There is no division: it is replaced by modular multiplicative inverse.
 * @module
 */
/*! noble-curves - MIT License (c) 2022 Paul Miller (paulmillr.com) */
import { type TArg, type TRet } from '../utils.ts';
/**
 * @param a - Dividend value.
 * @param b - Positive modulus.
 * @returns Reduced value in `[0, b)` only when `b` is positive.
 * @throws If the modulus is not positive. {@link Error}
 * @example
 * Normalize a bigint into one field residue.
 *
 * ```ts
 * mod(-1n, 5n);
 * ```
 */
export declare function mod(a: bigint, b: bigint): bigint;
/**
 * Efficiently raise num to a power with modular reduction.
 * Unsafe in some contexts: uses ladder, so can expose bigint bits.
 * Low-level helper: callers that need canonical residues must pass a valid `num` for the chosen
 * modulus instead of relying on the `power===0/1` fast paths to normalize it.
 * @param num - Base value.
 * @param power - Exponent value.
 * @param modulo - Reduction modulus.
 * @returns Modular exponentiation result.
 * @throws If the modulus or exponent is invalid. {@link Error}
 * @example
 * Raise one bigint to a modular power.
 *
 * ```ts
 * pow(2n, 6n, 11n) // 64n % 11n == 9n
 * ```
 */
export declare function pow(num: bigint, power: bigint, modulo: bigint): bigint;
/**
 * Does `x^(2^power)` mod p. `pow2(30, 4)` == `30^(2^4)`.
 * Low-level helper: callers that need canonical residues must pass a valid `x` for the chosen
 * modulus; the `power===0` fast path intentionally returns the input unchanged.
 * @param x - Base value.
 * @param power - Number of squarings.
 * @param modulo - Reduction modulus.
 * @returns Repeated-squaring result.
 * @throws If the exponent is negative. {@link Error}
 * @example
 * Apply repeated squaring inside one field.
 *
 * ```ts
 * pow2(3n, 2n, 11n);
 * ```
 */
export declare function pow2(x: bigint, power: bigint, modulo: bigint): bigint;
/**
 * Inverses number over modulo.
 * Implemented using the {@link https://brilliant.org/wiki/extended-euclidean-algorithm/ | extended Euclidean algorithm}.
 * @param number - Value to invert.
 * @param modulo - Modulus greater than 1.
 * @returns Multiplicative inverse.
 * @throws If the modulus is invalid or the inverse does not exist. {@link Error}
 * @example
 * Compute one modular inverse with the extended Euclidean algorithm.
 *
 * ```ts
 * invert(3n, 11n);
 * ```
 */
export declare function invert(number: bigint, modulo: bigint): bigint;
/**
 * Inverses number over modulo using Fermat's little theorem: `a^(p-2) ≡ a⁻¹ (mod p)`.
 *
 * Unlike {@link invert} (extended Euclidean), the exponent `p-2` is a public constant, so the
 * underlying square-and-multiply has the same control flow for every secret `a`: there is no
 * data-dependent branching or loop count that could leak `a` through timing (e.g. Minerva-style
 * ECDSA nonce-inversion attacks). This is only "algorithmically" constant-time — JS bigint
 * multiplication/reduction is still value-dependent — and it is roughly 4x slower than
 * {@link invert}.
 *
 * REQUIRES a prime modulus; Fermat's theorem does not hold otherwise. The result is verified to be
 * a real inverse, so a non-prime modulus (or a non-invertible input) fails closed with an error
 * instead of returning a wrong value.
 * @param a - Value to invert.
 * @param prime - Prime modulus.
 * @returns Multiplicative inverse in `[1, prime)`.
 * @throws If the modulus is below 2, the input reduces to zero, or the inverse does not exist.
 *   {@link Error}
 * @example
 * Compute one modular inverse without secret-dependent branching.
 *
 * ```ts
 * invertCt(3n, 11n); // 4n, since 3 * 4 = 12 ≡ 1 (mod 11)
 * ```
 */
export declare function invertCt(a: bigint, prime: bigint): bigint;
/**
 * Tonelli-Shanks square root search algorithm.
 * This implementation is variable-time: it searches data-dependently for the first non-residue `Z`
 * and for the smallest `i` in the main loop, unlike RFC 9380 Appendix I.4's constant-time shape.
 * 1. {@link https://eprint.iacr.org/2012/685.pdf | eprint 2012/685}, page 12
 * 2. Square Roots from 1; 24, 51, 10 to Dan Shanks
 * @param P - field order
 * @returns function that takes field Fp (created from P) and number n
 * @throws If the field is too small, non-prime, or the square root does not exist. {@link Error}
 * @example
 * Construct a square-root helper for primes that need Tonelli-Shanks.
 *
 * ```ts
 * import { Field, tonelliShanks } from '@noble/curves/abstract/modular.js';
 * const Fp = Field(17n);
 * const sqrt = tonelliShanks(17n)(Fp, 4n);
 * ```
 */
export declare function tonelliShanks(P: bigint): TRet<(<T>(Fp: IField<T>, n: T) => T)>;
/**
 * Square root for a finite field. Will try optimized versions first:
 *
 * 1. P ≡ 3 (mod 4)
 * 2. P ≡ 5 (mod 8)
 * 3. P ≡ 9 (mod 16)
 * 4. Tonelli-Shanks algorithm
 *
 * Different algorithms can give different roots, it is up to user to decide which one they want.
 * For example there is FpSqrtOdd/FpSqrtEven to choose a root by oddness
 * (used for hash-to-curve).
 * @param P - Field order.
 * @returns Square-root helper. The generic fallback inherits Tonelli-Shanks' variable-time
 *   behavior and this selector assumes prime-field-style integer moduli.
 * @throws If the field is unsupported or the square root does not exist. {@link Error}
 * @example
 * Choose the square-root helper appropriate for one field modulus.
 *
 * ```ts
 * import { Field, FpSqrt } from '@noble/curves/abstract/modular.js';
 * const Fp = Field(17n);
 * const sqrt = FpSqrt(17n)(Fp, 4n);
 * ```
 */
export declare function FpSqrt(P: bigint): TRet<(<T>(Fp: IField<T>, n: T) => T)>;
/**
 * @param num - Value to inspect.
 * @param modulo - Field modulus.
 * @returns `true` when the least-significant little-endian bit is set.
 * @throws If the modulus is invalid for `mod(...)`. {@link Error}
 * @example
 * Inspect the low bit used by little-endian sign conventions.
 *
 * ```ts
 * isNegativeLE(3n, 11n);
 * ```
 */
export declare const isNegativeLE: (num: bigint, modulo: bigint) => boolean;
/** Generic field interface used by prime and extension fields alike.
 * Generic helpers treat field operations as pure functions: implementations MUST treat provided
 * values/byte buffers as read-only and return detached results instead of mutating arguments.
 */
export interface IField<T> {
    /** Field order `q`, which may be prime or a prime power. */
    ORDER: bigint;
    /** Canonical encoded byte length. */
    BYTES: number;
    /** Canonical encoded bit length. */
    BITS: number;
    /** Whether encoded field elements use little-endian bytes. */
    isLE: boolean;
    /** Additive identity. */
    ZERO: T;
    /** Multiplicative identity. */
    ONE: T;
    /**
     * Normalize one value into the field.
     * @param num - Input value.
     * @returns Normalized field value.
     */
    create: (num: T) => T;
    /**
     * Check whether one value already belongs to the field.
     * @param num - Input value.
     * Implementations may throw `TypeError` on malformed input types instead of returning `false`.
     * @returns Whether the value already belongs to the field.
     */
    isValid: (num: T) => boolean;
    /**
     * Check whether one value is zero.
     * @param num - Input value.
     * @returns Whether the value is zero.
     */
    is0: (num: T) => boolean;
    /**
     * Check whether one value is non-zero and belongs to the field.
     * @param num - Input value.
     * Implementations may throw `TypeError` on malformed input types instead of returning `false`.
     * @returns Whether the value is non-zero and valid.
     */
    isValidNot0: (num: T) => boolean;
    /**
     * Negate one value.
     * @param num - Input value.
     * @returns Negated value.
     */
    neg(num: T): T;
    /**
     * Invert one value multiplicatively.
     * @param num - Input value.
     * @returns Multiplicative inverse.
     */
    inv(num: T): T;
    /**
     * Compute one square root when it exists.
     * @param num - Input value.
     * @returns Square root.
     */
    sqrt(num: T): T;
    /**
     * Square one value.
     * @param num - Input value.
     * @returns Squared value.
     */
    sqr(num: T): T;
    /**
     * Compare two field values.
     * @param lhs - Left value.
     * @param rhs - Right value.
     * @returns Whether both values are equal.
     */
    eql(lhs: T, rhs: T): boolean;
    /**
     * Add two normalized field values.
     * @param lhs - Left value.
     * @param rhs - Right value.
     * @returns Sum value.
     */
    add(lhs: T, rhs: T): T;
    /**
     * Subtract two normalized field values.
     * @param lhs - Left value.
     * @param rhs - Right value.
     * @returns Difference value.
     */
    sub(lhs: T, rhs: T): T;
    /**
     * Multiply two field values.
     * @param lhs - Left value.
     * @param rhs - Right value or scalar.
     * @returns Product value.
     */
    mul(lhs: T, rhs: T | bigint): T;
    /**
     * Raise one field value to a power.
     * @param lhs - Base value.
     * @param power - Exponent.
     * @returns Power value.
     */
    pow(lhs: T, power: bigint): T;
    /**
     * Divide one field value by another.
     * @param lhs - Dividend.
     * @param rhs - Divisor or scalar.
     * @returns Quotient value.
     */
    div(lhs: T, rhs: T | bigint): T;
    /**
     * Add two values without re-normalizing the result.
     * @param lhs - Left value.
     * @param rhs - Right value.
     * @returns Non-normalized sum.
     */
    addN(lhs: T, rhs: T): T;
    /**
     * Subtract two values without re-normalizing the result.
     * @param lhs - Left value.
     * @param rhs - Right value.
     * @returns Non-normalized difference.
     */
    subN(lhs: T, rhs: T): T;
    /**
     * Multiply two values without re-normalizing the result.
     * @param lhs - Left value.
     * @param rhs - Right value or scalar.
     * @returns Non-normalized product.
     */
    mulN(lhs: T, rhs: T | bigint): T;
    /**
     * Square one value without re-normalizing the result.
     * @param num - Input value.
     * @returns Non-normalized square.
     */
    sqrN(num: T): T;
    /**
     * Return the RFC 9380 `sgn0`-style oddness bit when supported.
     * This uses oddness instead of evenness so extension fields like Fp2 can expose the same hook.
     * Returns whether the value is odd under the field encoding.
     */
    isOdd?(num: T): boolean;
    /**
     * Invert many field elements in one batch.
     * @param lst - Values to invert.
     * @returns Batch of inverses.
     */
    invertBatch: (lst: T[]) => T[];
    /**
     * Encode one field value into fixed-width bytes.
     * Callers that need canonical encodings MUST supply a valid field element.
     * Low-level protocols may also use this to serialize raw / non-canonical residues.
     * @param num - Input value.
     * @returns Fixed-width byte encoding.
     */
    toBytes(num: T): Uint8Array;
    /**
     * Decode one field value from fixed-width bytes.
     * @param bytes - Fixed-width byte encoding.
     * @param skipValidation - Whether to skip range validation.
     * Implementations MUST treat `bytes` as read-only.
     * @returns Decoded field value.
     */
    fromBytes(bytes: Uint8Array, skipValidation?: boolean): T;
    /**
     * Constant-time conditional move.
     * @param a - Value used when the condition is false.
     * @param b - Value used when the condition is true.
     * @param c - Selection bit.
     * @returns Selected value.
     */
    cmov(a: T, b: T, c: boolean): T;
}
/**
 * @param field - Field implementation.
 * @returns Validated field. This only checks the arithmetic subset needed by generic helpers; it
 *   does not guarantee full runtime-method coverage for serialization, batching, `cmov`, or
 *   field-specific extras beyond positive `BYTES` / `BITS`.
 * @throws If the field shape or numeric metadata are invalid. {@link Error}
 * @example
 * Check that a field implementation exposes the operations curve code expects.
 *
 * ```ts
 * import { Field, validateField } from '@noble/curves/abstract/modular.js';
 * const Fp = validateField(Field(17n));
 * ```
 */
export declare function validateField<T>(field: TArg<IField<T>>): TRet<IField<T>>;
/**
 * Same as `pow` but for Fp: non-constant-time.
 * Unsafe in some contexts: uses ladder, so can expose bigint bits.
 * @param Fp - Field implementation.
 * @param num - Base value.
 * @param power - Exponent value.
 * @returns Powered field element.
 * @throws If the exponent is negative. {@link Error}
 * @example
 * Raise one field element to a public exponent.
 *
 * ```ts
 * import { Field, FpPow } from '@noble/curves/abstract/modular.js';
 * const Fp = Field(17n);
 * const x = FpPow(Fp, 3n, 5n);
 * ```
 */
export declare function FpPow<T>(Fp: TArg<IField<T>>, num: T, power: bigint): T;
/**
 * Efficiently invert an array of Field elements.
 * Zero-valued inputs are not inverted: by default their slot stays `undefined` (hence the
 * `(T | undefined)[]` return type), or becomes `0` when `passZero` is enabled. Because of that the
 * batch never calls `inv` on a zero, so over a prime field it is exception-free. The single
 * `Fp.inv` of the accumulated product can still throw, but only for a non-invertible product, which
 * a prime `ORDER` cannot produce (it requires a composite / non-field `ORDER`).
 * @param Fp - Field implementation.
 * @param nums - Values to invert.
 * @param passZero - map 0 to 0 (instead of undefined)
 * @returns Inverted values; entries for zero inputs are `undefined` unless `passZero` is set.
 * @example
 * Invert several field elements with one shared inversion.
 *
 * ```ts
 * import { Field, FpInvertBatch } from '@noble/curves/abstract/modular.js';
 * const Fp = Field(17n);
 * const inv = FpInvertBatch(Fp, [1n, 2n, 4n]);
 * ```
 */
export declare function FpInvertBatch<T>(Fp: TArg<IField<T>>, nums: T[], passZero: true): T[];
export declare function FpInvertBatch<T>(Fp: TArg<IField<T>>, nums: T[], passZero?: boolean): (T | undefined)[];
/**
 * @param Fp - Field implementation.
 * @param lhs - Dividend value.
 * @param rhs - Divisor value.
 * @returns Division result.
 * @throws If the divisor is non-invertible. {@link Error}
 * @example
 * Divide one field element by another.
 *
 * ```ts
 * import { Field, FpDiv } from '@noble/curves/abstract/modular.js';
 * const Fp = Field(17n);
 * const x = FpDiv(Fp, 6n, 3n);
 * ```
 */
export declare function FpDiv<T>(Fp: TArg<IField<T>>, lhs: T, rhs: T | bigint): T;
/**
 * Legendre symbol.
 * Legendre constant is used to calculate Legendre symbol (a | p)
 * which denotes the value of a^((p-1)/2) (mod p).
 *
 * * (a | p) ≡ 1    if a is a square (mod p), quadratic residue
 * * (a | p) ≡ -1   if a is not a square (mod p), quadratic non residue
 * * (a | p) ≡ 0    if a ≡ 0 (mod p)
 * @param Fp - Field implementation.
 * @param n - Value to inspect.
 * @returns Legendre symbol.
 * @throws If the powered value does not match a valid Legendre symbol. {@link Error}
 * @example
 * Compute the Legendre symbol of one field element.
 *
 * ```ts
 * import { Field, FpLegendre } from '@noble/curves/abstract/modular.js';
 * const Fp = Field(17n);
 * const symbol = FpLegendre(Fp, 4n);
 * ```
 */
export declare function FpLegendre<T>(Fp: TArg<IField<T>>, n: T): -1 | 0 | 1;
/**
 * @param Fp - Field implementation.
 * @param n - Value to inspect.
 * @returns `true` when `Fp.sqrt(n)` exists. This includes `0`, even though strict "quadratic
 *   residue" terminology often reserves that name for the non-zero square class.
 * @throws If the field returns an invalid Legendre symbol value. {@link Error}
 * @example
 * Check whether one field element has a square root in the field.
 *
 * ```ts
 * import { Field, FpIsSquare } from '@noble/curves/abstract/modular.js';
 * const Fp = Field(17n);
 * const isSquare = FpIsSquare(Fp, 4n);
 * ```
 */
export declare function FpIsSquare<T>(Fp: TArg<IField<T>>, n: T): boolean;
/** Byte and bit lengths derived from one scalar order. */
export type NLength = {
    /** Canonical byte length. */
    nByteLength: number;
    /** Canonical bit length. */
    nBitLength: number;
};
/**
 * @param n - Curve order. Callers are expected to pass a positive order.
 * @param nBitLength - Optional cached bit length. Callers are expected to pass a positive cached
 *   value when overriding the derived bit length.
 * @returns Byte and bit lengths.
 * @throws If the order or cached bit length is invalid. {@link Error}
 * @example
 * Measure the encoding sizes needed for one modulus.
 *
 * ```ts
 * nLength(255n);
 * ```
 */
export declare function nLength(n: bigint, nBitLength?: number): NLength;
type FpField = IField<bigint> & Required<Pick<IField<bigint>, 'isOdd'>>;
type SqrtFn = (n: bigint) => bigint;
type FieldOpts = Partial<{
    isLE: boolean;
    BITS: number;
    sqrt: SqrtFn;
    allowedLengths?: readonly number[];
    modFromBytes: boolean;
}>;
/**
 * Creates a finite field. Major performance optimizations:
 * * 1. Denormalized operations like mulN instead of mul.
 * * 2. Identical object shape: never add or remove keys.
 * * 3. Frozen stable object shape; the lazy sqrt cache lives in a module-level `WeakMap`.
 * Fragile: always run a benchmark on a change.
 * Security note: operations and low-level serializers like `toBytes` don't check `isValid` for
 * all elements for performance and protocol-flexibility reasons; callers are responsible for
 * supplying valid elements when they need canonical field behavior.
 * This is low-level code, please make sure you know what you're doing.
 *
 * Note about field properties:
 * * CHARACTERISTIC p = prime number, number of elements in main subgroup.
 * * ORDER q = similar to cofactor in curves, may be composite `q = p^m`.
 *
 * @param ORDER - field order, probably prime, or could be composite
 * @param opts - Field options such as bit length or endianness. See {@link FieldOpts}.
 * @returns Frozen field instance with a stable object shape. This wrapper forwards `opts` straight
 *   into `_Field`, so it inherits `_Field`'s assumptions about cached sizes and `allowedLengths`.
 * @example
 * Construct one prime field with optional overrides.
 *
 * ```ts
 * Field(11n);
 * ```
 */
export declare function Field(ORDER: bigint, opts?: FieldOpts): TRet<Readonly<FpField>>;
/**
 * @param Fp - Field implementation.
 * @param elm - Value to square-root.
 * @returns Odd square root when two roots exist. The special case `elm = 0` still returns `0`,
 *   which is the only square root but is not odd.
 * @throws If the field lacks oddness checks or the square root does not exist. {@link Error}
 * @example
 * Select the odd square root when two roots exist.
 *
 * ```ts
 * import { Field, FpSqrtOdd } from '@noble/curves/abstract/modular.js';
 * const Fp = Field(17n);
 * const root = FpSqrtOdd(Fp, 4n);
 * ```
 */
export declare function FpSqrtOdd<T>(Fp: TArg<IField<T>>, elm: T): T;
/**
 * @param Fp - Field implementation.
 * @param elm - Value to square-root.
 * @returns Even square root.
 * @throws If the field lacks oddness checks or the square root does not exist. {@link Error}
 * @example
 * Select the even square root when two roots exist.
 *
 * ```ts
 * import { Field, FpSqrtEven } from '@noble/curves/abstract/modular.js';
 * const Fp = Field(17n);
 * const root = FpSqrtEven(Fp, 4n);
 * ```
 */
export declare function FpSqrtEven<T>(Fp: TArg<IField<T>>, elm: T): T;
/**
 * Returns total number of bytes consumed by the field element.
 * For example, 32 bytes for usual 256-bit weierstrass curve.
 * @param fieldOrder - number of field elements, usually CURVE.n. Callers are expected to pass an
 *   order greater than 1.
 * @returns byte length of field
 * @throws If the field order is not a bigint. {@link Error}
 * @example
 * Read the fixed-width byte length of one field.
 *
 * ```ts
 * getFieldBytesLength(255n);
 * ```
 */
export declare function getFieldBytesLength(fieldOrder: bigint): number;
/**
 * Returns minimal amount of bytes that can be safely reduced
 * by field order.
 * Should be 2^-128 for 128-bit curve such as P256.
 * This is the reduction / modulo-bias lower bound; higher-level helpers may still impose a larger
 * absolute floor for policy reasons.
 * @param fieldOrder - number of field elements greater than 1, usually CURVE.n.
 * @returns byte length of target hash
 * @throws If the field order is invalid. {@link Error}
 * @example
 * Compute the minimum hash length needed for field reduction.
 *
 * ```ts
 * getMinHashLength(255n);
 * ```
 */
export declare function getMinHashLength(fieldOrder: bigint): number;
/**
 * "Constant-time" private key generation utility.
 * Can take (n + n/2) or more bytes of uniform input e.g. from CSPRNG or KDF
 * and convert them into private scalar, with the modulo bias being negligible.
 * Needs at least 48 bytes of input for 32-byte private key. The implementation also keeps a hard
 * 16-byte minimum even when `getMinHashLength(...)` is smaller, so toy-small inputs do not look
 * accidentally acceptable for real scalar derivation.
 * See {@link https://research.kudelskisecurity.com/2020/07/28/the-definitive-guide-to-modulo-bias-and-how-to-avoid-it/ | Kudelski's modulo-bias guide},
 * {@link https://csrc.nist.gov/publications/detail/fips/186/5/final | FIPS 186-5 appendix A.2}, and
 * {@link https://www.rfc-editor.org/rfc/rfc9380#section-5 | RFC 9380 section 5}. Unlike RFC 9380
 * `hash_to_field`, this helper intentionally maps into the non-zero private-scalar range `1..n-1`.
 * @param key - Uniform input bytes.
 * @param fieldOrder - Size of subgroup.
 * @param isLE - interpret hash bytes as LE num
 * @returns valid private scalar
 * @throws If the hash length or field order is invalid for scalar reduction. {@link Error}
 * @example
 * Map hash output into a private scalar range.
 *
 * ```ts
 * mapHashToField(new Uint8Array(48).fill(1), 255n);
 * ```
 */
export declare function mapHashToField(key: TArg<Uint8Array>, fieldOrder: bigint, isLE?: boolean): TRet<Uint8Array>;
export {};
