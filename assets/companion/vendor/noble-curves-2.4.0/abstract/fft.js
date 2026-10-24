/**
 * Experimental implementation of NTT / FFT (Fast Fourier Transform) over finite fields.
 * API may change at any time. The code has not been audited. Feature requests are welcome.
 * @module
 */
import { aarray, validateObject } from "../utils.js";
import { validateField } from "./modular.js";
function checkU32(n, title = 'n') {
    // 0xff_ff_ff_ff
    if (typeof n !== 'number')
        throw new TypeError(`wrong u32 integer "${title}": expected number, got type=${typeof n}`);
    if (!Number.isSafeInteger(n) || n < 0 || n > 0xffffffff)
        throw new RangeError(`wrong u32 integer "${title}": expected 0..4294967295, got ${n}`);
    return n;
}
/**
 * Checks if integer is in form of `1 << X`.
 * @param x - Integer to inspect.
 * @returns `true` when the value is a power of two.
 * @example
 * Validate that an FFT size is a power of two.
 *
 * ```ts
 * isPowerOfTwo(8);
 * ```
 */
export function isPowerOfTwo(x) {
    checkU32(x, 'x');
    return (x & (x - 1)) === 0 && x !== 0;
}
/**
 * @param n - Input value.
 * @returns Next power of two within the u32/array-length domain.
 * @throws If `n` is not a valid unsigned 32-bit integer. {@link Error}
 * @example
 * Round an integer up to the FFT size it needs.
 *
 * ```ts
 * nextPowerOfTwo(9);
 * ```
 */
export function nextPowerOfTwo(n) {
    checkU32(n);
    if (n <= 1)
        return 1;
    // FFT sizes here are used as JS array lengths, so `2^32` is not a meaningful result:
    // keep the fast u32 bit-twiddling path and fail explicitly instead of wrapping to 1.
    if (n > 0x8000_0000)
        throw new Error('nextPowerOfTwo overflow: result does not fit u32');
    return (1 << (log2(n - 1) + 1)) >>> 0;
}
/**
 * @param n - Value to reverse.
 * @param bits - Number of bits to use.
 * @returns Bit-reversed integer.
 * @throws If `n` is not a valid unsigned 32-bit integer. {@link Error}
 * @example
 * Reverse the low `bits` bits of one index.
 *
 * ```ts
 * reverseBits(3, 3);
 * ```
 */
export function reverseBits(n, bits) {
    checkU32(n);
    if (typeof bits !== 'number')
        throw new TypeError('"bits" expected number, got type=' + typeof bits);
    if (!Number.isSafeInteger(bits) || bits < 0 || bits > 32)
        throw new Error(`expected integer 0 <= bits <= 32, got ${bits}`);
    let reversed = 0;
    for (let i = 0; i < bits; i++, n >>>= 1)
        reversed = (reversed << 1) | (n & 1);
    // JS bitwise ops are signed i32; cast back so 32-bit reversals stay in the unsigned u32 domain.
    return reversed >>> 0;
}
/**
 * Similar to `bitLen(x)-1` but much faster for small integers, like indices.
 * @param n - Input value.
 * @returns Base-2 logarithm. For `n = 0`, the current implementation returns `-1`.
 * @example
 * Compute the radix-2 stage count for one transform size.
 *
 * ```ts
 * log2(8);
 * ```
 */
export function log2(n) {
    checkU32(n);
    return 31 - Math.clz32(n);
}
/**
 * Moves lowest bit to highest position, which at first step splits
 * array on even and odd indices, then it applied again to each part,
 * which is core of fft
 * @param values - Mutable coefficient array.
 * @returns Mutated input array.
 * @throws If the array length is not a positive power of two. {@link Error}
 * @example
 * Reorder coefficients into bit-reversed order in place.
 *
 * ```ts
 * const values = Uint8Array.from([0, 1, 2, 3]);
 * bitReversalInplace(values);
 * ```
 */
export function bitReversalInplace(values) {
    if (!values ||
        typeof values !== 'object' ||
        typeof values.length !== 'number')
        throw new TypeError('"values" expected array-like, got type=' + typeof values);
    const n = values.length;
    // Size-1 FFT is the identity, so bit-reversal must stay a no-op there instead of rejecting it.
    if (!isPowerOfTwo(n))
        throw new Error('expected positive power-of-two length, got ' + n);
    const bits = log2(n);
    for (let i = 0; i < n; i++) {
        const j = reverseBits(i, bits);
        if (i < j) {
            const tmp = values[i];
            values[i] = values[j];
            values[j] = tmp;
        }
    }
    return values;
}
/**
 * @param values - Input values.
 * @returns Reordered copy.
 * @throws If the array length is not a positive power of two. {@link Error}
 * @example
 * Return a reordered copy instead of mutating the input in place.
 *
 * ```ts
 * const reordered = bitReversalPermutation([0, 1, 2, 3]);
 * ```
 */
export function bitReversalPermutation(values) {
    aarray(values, 'values');
    return bitReversalInplace(values.slice());
}
const _1n = /** @__PURE__ */ BigInt(1);
function findGenerator(field) {
    let G = BigInt(2);
    for (; field.eql(field.pow(G, field.ORDER >> _1n), field.ONE); G++)
        ;
    return G;
}
/**
 * We limit roots up to 2**31, which is a lot: 2-billion polynomial should be rare.
 * @param field - Field implementation.
 * @param generator - Optional trusted non-quadratic-residue override for callers that already know the field.
 * @returns Roots-of-unity cache.
 * @example
 * Cache roots once, then ask for the omega table of one FFT size.
 *
 * ```ts
 * import { rootsOfUnity } from '@noble/curves/abstract/fft.js';
 * import { Field } from '@noble/curves/abstract/modular.js';
 * const roots = rootsOfUnity(Field(17n));
 * const omega = roots.omega(4);
 * ```
 */
export function rootsOfUnity(field, generator) {
    validateField(field);
    if (generator !== undefined && typeof generator !== 'bigint')
        throw new TypeError('"generator" expected bigint, got type=' + typeof generator);
    // Factor field.ORDER-1 as oddFactor * 2^powerOfTwo
    let oddFactor = field.ORDER - _1n;
    let powerOfTwo = 0;
    for (; (oddFactor & _1n) !== _1n; powerOfTwo++, oddFactor >>= _1n)
        ;
    // Find non quadratic residue
    let G = generator !== undefined ? BigInt(generator) : findGenerator(field);
    // Powers of generator
    const omegas = new Array(powerOfTwo + 1);
    omegas[powerOfTwo] = field.pow(G, oddFactor);
    for (let i = powerOfTwo; i > 0; i--)
        omegas[i - 1] = field.sqr(omegas[i]);
    // Compute all roots of unity for powers up to maxPower
    const rootsCache = [];
    const checkBits = (bits) => {
        checkU32(bits, 'bits');
        if (bits > 31 || bits > powerOfTwo)
            throw new Error('rootsOfUnity: wrong bits ' + bits + ' powerOfTwo=' + powerOfTwo);
        return bits;
    };
    const precomputeRoots = (maxPower) => {
        checkBits(maxPower);
        for (let power = maxPower; power >= 0; power--) {
            if (rootsCache[power])
                continue; // Skip if we've already computed roots for this power
            const above = rootsCache[power + 1];
            const rootsAtPower = [];
            if (above) {
                // ω_{2^p} = ω_{2^{p+1}}², so the smaller table is the even-index stride of the bigger
                // one: only the largest requested power pays for the multiplication chain.
                for (let j = 0; j < 2 ** power; j++)
                    rootsAtPower.push(above[2 * j]);
            }
            else {
                for (let j = 0, cur = field.ONE; j < 2 ** power; j++, cur = field.mul(cur, omegas[power]))
                    rootsAtPower.push(cur);
            }
            rootsCache[power] = rootsAtPower;
        }
        return rootsCache[maxPower];
    };
    const brpCache = new Map();
    const inverseCache = new Map();
    // roots()/brp()/inverse() expose shared cached arrays by reference for speed; callers must treat them as read-only.
    // NOTE: we use bits instead of power, because power = 2**bits,
    // but power is not necessarily isPowerOfTwo(power)!
    return {
        info: { G, powerOfTwo, oddFactor },
        roots: (bits) => {
            const b = checkBits(bits);
            return precomputeRoots(b);
        },
        brp(bits) {
            const b = checkBits(bits);
            if (brpCache.has(b))
                return brpCache.get(b);
            else {
                const res = bitReversalPermutation(this.roots(b));
                brpCache.set(b, res);
                return res;
            }
        },
        inverse(bits) {
            const b = checkBits(bits);
            if (inverseCache.has(b))
                return inverseCache.get(b);
            else {
                // ωᴺ = 1, so inv(ωᵏ) = ωᴺ⁻ᵏ: the inverse table is the reversed roots table.
                // Value-identical to field.invertBatch(roots), but skips its 3N muls + inversion.
                const r = this.roots(b);
                const res = [r[0]].concat(r.slice(1).reverse());
                inverseCache.set(b, res);
                return res;
            }
        },
        omega: (bits) => omegas[checkBits(bits)],
        clear: () => {
            rootsCache.splice(0, rootsCache.length);
            brpCache.clear();
            inverseCache.clear();
        },
    };
}
/**
 * Constructs different flavors of FFT. radix2 implementation of low level mutating API. Flavors:
 *
 * - DIT (Decimation-in-Time): Bottom-Up (leaves to root), Cooley-Tukey
 * - DIF (Decimation-in-Frequency): Top-Down (root to leaves), Gentleman-Sande
 *
 * DIT takes brp input, returns natural output.
 * DIF takes natural input, returns brp output.
 *
 * The output is actually identical. Time / frequence distinction is not meaningful
 * for Polynomial multiplication in fields.
 * Which means if protocol supports/needs brp output/inputs, then we can skip this step.
 *
 * Cyclic NTT: Rq = Zq[x]/(x^n-1). butterfly_DIT+loop_DIT OR butterfly_DIF+loop_DIT, roots are omega
 * Negacyclic NTT: Rq = Zq[x]/(x^n+1). butterfly_DIT+loop_DIF, at least for mlkem / mldsa
 *
 * `invertButterflies` indexes roots by a per-butterfly-group counter (`grp`): forward
 * (`dit: false`) reads `roots[grp]` with grp = 1..; inverse (`dit: true`) reads `roots[N - grp]`
 * with grp restarting at 1. With `skipStages: 0` one table serves both directions (ωᴺ = 1 makes
 * the reversed walk self-inverse). With `skipStages > 0` the inverse walk starts at `N - 1`
 * instead of continuing where the skipped stages would have left off, so the caller must supply
 * a table shaped for that (ML-KEM: `ζ^BitRev7(i)` over all N=256 indices, whose aliased upper
 * half is exactly the FIPS 203 inverse walk).
 * @param F - Field operations.
 * @param coreOpts - FFT configuration. See {@link FFTCoreOpts}:
 *   - `N`: Transform size. Must be a power of two.
 *   - `roots`: Stage roots for the selected transform size.
 *   - `dit`: Whether to run the DIT variant instead of DIF.
 *   - `invertButterflies` (optional): Whether to invert butterfly placement.
 *   - `skipStages` (optional): Number of initial stages to skip.
 *   - `brp` (optional): Whether to apply bit-reversal permutation at the boundary.
 * @returns Low-level FFT loop.
 * @throws If the FFT options or cached roots are invalid for the requested size. {@link Error}
 * @example
 * Constructs different flavors of FFT.
 *
 * ```ts
 * import { FFTCore, rootsOfUnity } from '@noble/curves/abstract/fft.js';
 * import { Field } from '@noble/curves/abstract/modular.js';
 * const Fp = Field(17n);
 * const roots = rootsOfUnity(Fp).roots(2);
 * const loop = FFTCore(Fp, { N: 4, roots, dit: true });
 * const values = loop([1n, 2n, 3n, 4n]);
 * ```
 */
export const FFTCore = (F, coreOpts) => {
    validateObject(coreOpts, { N: 'number', roots: 'object', dit: 'boolean' }, { invertButterflies: 'boolean', skipStages: 'number', brp: 'boolean' }, 'coreOpts');
    const { N, roots, dit, invertButterflies = false, skipStages = 0, brp = true } = coreOpts;
    checkU32(N, 'coreOpts.N');
    const bits = log2(N);
    if (!isPowerOfTwo(N))
        throw new Error('FFT: Polynomial size should be power of two');
    checkU32(skipStages, 'coreOpts.skipStages');
    const maxSkipStages = bits === 0 ? 0 : bits - 1;
    // Skipping every stage leaves only boundary layout changes, not a valid FFT loop shape.
    if (skipStages > maxSkipStages)
        throw new Error(`FFT: wrong skipStages: expected 0 <= skipStages <= ${maxSkipStages}`);
    // Wrong-sized root tables can stay in-bounds for some loop shapes and silently compute nonsense.
    if (roots.length !== N)
        throw new Error(`FFT: wrong roots length: expected ${N}, got ${roots.length}`);
    const isDit = dit !== invertButterflies;
    return (values) => {
        if (values.length !== N)
            throw new Error('FFT: wrong Polynomial length');
        if (dit && brp)
            bitReversalInplace(values);
        for (let i = 0, g = 1; i < bits - skipStages; i++) {
            // For each stage s (sub-FFT length m = 2^s)
            const s = dit ? i + 1 + skipStages : bits - i;
            const m = 1 << s;
            const m2 = m >> 1;
            const stride = N >> s;
            // Loop over each subarray of length m
            for (let k = 0; k < N; k += m) {
                // Loop over each butterfly within the subarray
                for (let j = 0, grp = g++; j < m2; j++) {
                    const rootPos = invertButterflies ? (dit ? N - grp : grp) : j * stride;
                    const i0 = k + j;
                    const i1 = k + j + m2;
                    const omega = roots[rootPos];
                    const b = values[i1];
                    const a = values[i0];
                    // Inlining gives us 10% perf in kyber vs functions
                    if (isDit) {
                        const t = F.mul(b, omega); // Standard DIT butterfly
                        values[i0] = F.add(a, t);
                        values[i1] = F.sub(a, t);
                    }
                    else if (invertButterflies) {
                        values[i0] = F.add(b, a); // DIT loop + inverted butterflies (Kyber decode)
                        values[i1] = F.mul(F.sub(b, a), omega);
                    }
                    else {
                        values[i0] = F.add(a, b); // Standard DIF butterfly
                        values[i1] = F.mul(F.sub(a, b), omega);
                    }
                }
            }
        }
        if (!dit && brp)
            bitReversalInplace(values);
        return values;
    };
};
/**
 * NTT aka FFT over finite field (NOT over complex numbers).
 * Naming mirrors other libraries.
 * @param roots - Roots-of-unity cache.
 * @param opts - Field operations. See {@link FFTOpts}.
 * @returns Forward and inverse FFT helpers.
 * @example
 * NTT aka FFT over finite field (NOT over complex numbers).
 *
 * ```ts
 * import { FFT, rootsOfUnity } from '@noble/curves/abstract/fft.js';
 * import { Field } from '@noble/curves/abstract/modular.js';
 * const Fp = Field(17n);
 * const fft = FFT(rootsOfUnity(Fp), Fp);
 * const values = fft.direct([1n, 2n, 3n, 4n]);
 * ```
 */
export function FFT(roots, opts) {
    // Loops are cached per (size, direction, brp flags): FFTCore construction validates options
    // and allocates closures, which costs more than a small transform itself. The cached loop
    // closes over the root table active at first use; `roots.clear()` rebuilds value-identical
    // tables, so a stale reference stays correct.
    const loops = new Map();
    const getLoop = (N, rootsTable, key) => {
        const cached = loops.get(key);
        if (cached)
            return cached;
        const brpInput = !!(key & 2);
        const brpOutput = !!(key & 1);
        let loop;
        if (brpInput && brpOutput) {
            // we cannot optimize this case, but lets support it anyway
            const core = FFTCore(opts, { N, roots: rootsTable, dit: false, brp: false });
            loop = (values) => core(bitReversalInplace(values));
        }
        else if (brpInput)
            loop = FFTCore(opts, { N, roots: rootsTable, dit: true, brp: false });
        else if (brpOutput)
            loop = FFTCore(opts, { N, roots: rootsTable, dit: false, brp: false });
        else
            loop = FFTCore(opts, { N, roots: rootsTable, dit: true, brp: true }); // all natural
        loops.set(key, loop);
        return loop;
    };
    const loopKey = (bits, isInverse, brpInput, brpOutput) => (bits << 3) | (isInverse ? 4 : 0) | (brpInput ? 2 : 0) | (brpOutput ? 1 : 0);
    return {
        direct(values, brpInput = false, brpOutput = false) {
            const N = values.length;
            if (!isPowerOfTwo(N))
                throw new Error('FFT: Polynomial size should be power of two');
            const bits = log2(N);
            const key = loopKey(bits, false, brpInput, brpOutput);
            return getLoop(N, roots.roots(bits), key)(values.slice());
        },
        inverse(values, brpInput = false, brpOutput = false) {
            const N = values.length;
            if (!isPowerOfTwo(N))
                throw new Error('FFT: Polynomial size should be power of two');
            const bits = log2(N);
            const key = loopKey(bits, true, brpInput, brpOutput);
            const res = getLoop(N, roots.inverse(bits), key)(values.slice());
            const ivm = opts.inv(BigInt(values.length)); // scale
            // we can get brp output if we use dif instead of dit!
            for (let i = 0; i < res.length; i++)
                res[i] = opts.mul(res[i], ivm);
            // Allows to re-use non-inverted roots, but is VERY fragile
            // return [res[0]].concat(res.slice(1).reverse());
            // inverse calculated as pow(-1), which transforms into ω^{-kn} (-> reverses indices)
            return res;
        },
    };
}
export function poly(field, roots, create, fft, length) {
    validateField(field);
    const F = field;
    const _create = create ||
        ((len, elm) => new Array(len).fill(elm ?? F.ZERO));
    // `poly.mul(a, b)` distinguishes polynomial-vs-scalar at runtime, so keep accepted
    // polynomial containers concrete instead of trying to support arbitrary wrappers.
    const isPoly = (x) => {
        if (Array.isArray(x))
            return true;
        if (!ArrayBuffer.isView(x))
            return false;
        const v = x;
        return (typeof v.length === 'number' &&
            typeof v.slice === 'function' &&
            typeof v[Symbol.iterator] === 'function');
    };
    const checkPoly = (title, value) => {
        if (!isPoly(value))
            throw new TypeError(`"${title}" expected polynomial, got type=${typeof value}`);
    };
    const checkLength = (a, b) => {
        checkPoly('a', a);
        const L = a.length;
        if (b !== undefined) {
            checkPoly('b', b);
            if (b.length !== L)
                throw new Error(`poly: mismatched lengths ${L} vs ${b.length}`);
        }
        if (length !== undefined && L !== length)
            throw new Error(`poly: expected fixed length ${length}, got ${L}`);
        return L;
    };
    function findOmegaIndex(x, n, brp = false, weights) {
        if (!isPowerOfTwo(n))
            throw new Error('poly.lagrange: expected power of two length, got ' + n);
        // Explicit weights define the interpolation domain, including the Kronecker-δ shortcut.
        const omega = weights || (brp ? roots.brp(log2(n)) : roots.roots(log2(n)));
        for (let i = 0; i < n; i++)
            if (F.eql(x, omega[i]))
                return i;
        return -1;
    }
    // TODO: mutating versions for mlkem/mldsa
    return {
        roots,
        create: _create,
        length,
        extend: (a, len) => {
            checkLength(a);
            const out = _create(len, F.ZERO);
            // Plain arrays grow when writing past `out.length`, so cap the copy explicitly to keep
            // `extend()` consistent with typed arrays and with its documented truncate behavior.
            for (let i = 0; i < Math.min(a.length, len); i++)
                out[i] = a[i];
            return out;
        },
        degree: (a) => {
            checkLength(a);
            for (let i = a.length - 1; i >= 0; i--)
                if (!F.is0(a[i]))
                    return i;
            return -1;
        },
        add: (a, b) => {
            const len = checkLength(a, b);
            const out = _create(len);
            for (let i = 0; i < len; i++)
                out[i] = F.add(a[i], b[i]);
            return out;
        },
        sub: (a, b) => {
            const len = checkLength(a, b);
            const out = _create(len);
            for (let i = 0; i < len; i++)
                out[i] = F.sub(a[i], b[i]);
            return out;
        },
        dot: (a, b) => {
            const len = checkLength(a, b);
            const out = _create(len);
            for (let i = 0; i < len; i++)
                out[i] = F.mul(a[i], b[i]);
            return out;
        },
        mul: (a, b) => {
            if (isPoly(b)) {
                const len = checkLength(a, b);
                if (fft) {
                    const A = fft.direct(a, false, true);
                    const B = fft.direct(b, false, true);
                    for (let i = 0; i < A.length; i++)
                        A[i] = F.mul(A[i], B[i]);
                    return fft.inverse(A, true, false);
                }
                else {
                    // NOTE: this is quadratic and mostly for compat tests with FFT
                    const res = _create(len);
                    for (let i = 0; i < len; i++) {
                        for (let j = 0; j < len; j++) {
                            const k = (i + j) % len; // wrap mod length
                            res[k] = F.add(res[k], F.mul(a[i], b[j]));
                        }
                    }
                    return res;
                }
            }
            else {
                const out = _create(checkLength(a));
                for (let i = 0; i < out.length; i++)
                    out[i] = F.mul(a[i], b);
                return out;
            }
        },
        convolve(a, b) {
            checkPoly('a', a);
            checkPoly('b', b);
            const len = nextPowerOfTwo(a.length + b.length - 1);
            return this.mul(this.extend(a, len), this.extend(b, len));
        },
        shift(p, factor) {
            checkPoly('p', p);
            const out = _create(p.length);
            if (length !== undefined && p.length !== length)
                throw new Error(`poly: expected fixed length ${length}, got ${p.length}`);
            if (!p.length)
                return out;
            out[0] = p[0];
            for (let i = 1, power = F.ONE; i < p.length; i++) {
                power = F.mul(power, factor);
                out[i] = F.mul(p[i], power);
            }
            return out;
        },
        clone: (a) => {
            checkLength(a);
            const out = _create(a.length);
            for (let i = 0; i < a.length; i++)
                out[i] = a[i];
            return out;
        },
        eval: (a, basis) => {
            checkLength(a, basis);
            let acc = F.ZERO;
            for (let i = 0; i < a.length; i++)
                acc = F.add(acc, F.mul(a[i], basis[i]));
            return acc;
        },
        monomial: {
            basis: (x, n) => {
                const out = _create(n);
                let pow = F.ONE;
                for (let i = 0; i < n; i++) {
                    out[i] = pow;
                    pow = F.mul(pow, x);
                }
                return out;
            },
            eval: (a, x) => {
                checkLength(a);
                // Same as eval(a, monomialBasis(x, a.length)), but it is faster this way
                let acc = F.ZERO;
                for (let i = a.length - 1; i >= 0; i--)
                    acc = F.add(F.mul(acc, x), a[i]);
                return acc;
            },
        },
        lagrange: {
            basis: (x, n, brp = false, weights) => {
                if (!isPowerOfTwo(n))
                    throw new Error('poly.lagrange: expected power of two length, got ' + n);
                const bits = log2(n);
                const cache = weights || (brp ? roots.brp(bits) : roots.roots(bits)); // [ω⁰, ω¹, ..., ωⁿ⁻¹]
                const out = _create(n);
                // Fast Kronecker-δ shortcut
                const idx = findOmegaIndex(x, n, brp, weights);
                if (idx !== -1) {
                    out[idx] = F.ONE;
                    return out;
                }
                const tm = F.pow(x, BigInt(n));
                const c = F.mul(F.sub(tm, F.ONE), F.inv(BigInt(n))); // c = (xⁿ - 1)/n
                const denom = _create(n);
                for (let i = 0; i < n; i++)
                    denom[i] = F.sub(x, cache[i]);
                const inv = F.invertBatch(denom);
                for (let i = 0; i < n; i++)
                    out[i] = F.mul(c, F.mul(cache[i], inv[i]));
                return out;
            },
            eval(a, x, brp = false) {
                checkLength(a);
                const idx = findOmegaIndex(x, a.length, brp);
                if (idx !== -1)
                    return a[idx]; // fast path
                const L = this.basis(x, a.length, brp); // Lᵢ(x)
                let acc = F.ZERO;
                for (let i = 0; i < a.length; i++)
                    if (!F.is0(a[i]))
                        acc = F.add(acc, F.mul(a[i], L[i]));
                return acc;
            },
        },
        vanishing(roots) {
            checkPoly('roots', roots);
            if (length !== undefined && roots.length !== length)
                throw new Error(`poly: expected fixed length ${length}, got ${roots.length}`);
            const out = _create(roots.length + 1, F.ZERO);
            out[0] = F.ONE;
            for (const r of roots) {
                const neg = F.neg(r);
                for (let j = out.length - 1; j > 0; j--)
                    out[j] = F.add(F.mul(out[j], neg), out[j - 1]);
                out[0] = F.mul(out[0], neg);
            }
            return out;
        },
    };
}
