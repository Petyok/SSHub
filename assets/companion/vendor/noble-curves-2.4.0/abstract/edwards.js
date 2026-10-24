/**
 * Twisted Edwards curve. The formula is: ax² + y² = 1 + dx²y².
 * For design rationale of types / exports, see weierstrass module documentation.
 * Untwisted Edwards curves exist, but they aren't used in real-world protocols.
 * @module
 */
/*! noble-curves - MIT License (c) 2022 Paul Miller (paulmillr.com) */
import { abool, abytes, aInRange, asafenumber, bytesToHex, bytesToNumberLE, concatBytes, copyBytes, hexToBytes, isBytes, notImplemented, validateObject, randomBytes as wcRandomBytes, } from "../utils.js";
import { createCurveFields, createKeygen, normalizeZ, ScalarMultiplier, validatePointCons, } from "./curve.js";
import { FpLegendre } from "./modular.js";
// Be friendly to bad ECMAScript parsers by not using bigint literals
// prettier-ignore
const _0n = /* @__PURE__ */ BigInt(0), _1n = /* @__PURE__ */ BigInt(1), _2n = /* @__PURE__ */ BigInt(2), _4n = /* @__PURE__ */ BigInt(4), _8n = /* @__PURE__ */ BigInt(8);
// Affine Edwards-equation check only; this does not prove subgroup membership, canonical
// encoding, prime-order base-point requirements, or identity exclusion.
function isEdValidXY(Fp, CURVE, x, y) {
    const x2 = Fp.sqr(x);
    const y2 = Fp.sqr(y);
    const left = Fp.add(Fp.mul(CURVE.a, x2), y2);
    const right = Fp.add(Fp.ONE, Fp.mul(CURVE.d, Fp.mul(x2, y2)));
    return Fp.eql(left, right);
}
/**
 * @param params - Curve parameters. See {@link EdwardsOpts}.
 * @param extraOpts - Optional helpers and overrides. See {@link EdwardsExtraOpts}.
 * @returns Edwards point constructor. Generator validation here only checks
 *   that `(Gx, Gy)` satisfies the affine Edwards equation.
 *   RFC 8032 base-point constraints like `B != (0,1)` and `[L]B = 0`
 *   are left to the caller's chosen parameters, since eager subgroup
 *   validation here adds about 10-15ms to heavyweight imports like ed448.
 *   The returned constructor also eagerly marks `Point.BASE` for W=6
 *   precompute caching. Some code paths still assume
 *   `Fp.BYTES === Fn.BYTES`, so mismatched byte lengths are not fully audited here.
 * @throws If the curve parameters or Edwards overrides are invalid. {@link Error}
 * @example
 * ```ts
 * import { edwards } from '@noble/curves/abstract/edwards.js';
 * import { jubjub } from '@noble/curves/misc.js';
 * // Build a point constructor from explicit curve parameters, then use its base point.
 * const Point = edwards(jubjub.Point.CURVE());
 * Point.BASE.toHex();
 * ```
 */
export function edwards(params, extraOpts = {}) {
    validateObject(extraOpts, {}, {}, 'extraOpts');
    const opts = extraOpts;
    const validated = createCurveFields('edwards', params, opts, opts.FpFnLE);
    const { Fp, Fn } = validated;
    let CURVE = validated.CURVE;
    const { h: cofactor } = CURVE;
    // The unified add-2008-hwcd formulas (see EdwardsPoint.add/double) are complete —
    // exception-free for every input pair — only when a is a square and d a non-square in Fp
    // (Bernstein–Birkner–Joye–Lange–Peters, "Twisted Edwards curves", thm 3.3). The constant-time
    // kernels in curve.ts assume completeness, so an incomplete curve could silently produce
    // wrong results on exceptional inputs. Fail construction instead.
    if (FpLegendre(Fp, CURVE.a) !== 1)
        throw new Error('edwards: CURVE.a must be a square in Fp for complete addition formulas');
    if (FpLegendre(Fp, CURVE.d) !== -1)
        throw new Error('edwards: CURVE.d must be a non-square in Fp for complete addition formulas');
    validateObject(opts, {}, { uvRatio: 'function', randomBytes: 'function' });
    const randomBytes = opts.randomBytes === undefined ? wcRandomBytes : opts.randomBytes;
    // Coordinate and ZIP-215 bounds follow the base-field byte container, not scalar bytes.
    const MASK = _2n << (BigInt(Fp.BYTES * 8) - _1n);
    function isOdd(n) {
        if (!Fp.isOdd)
            throw new Error('Field does not have .isOdd()');
        return Fp.isOdd(n);
    }
    // sqrt(u/v)
    const uvRatio = opts.uvRatio === undefined
        ? (u, v) => {
            try {
                return { isValid: true, value: Fp.sqrt(Fp.div(u, v)) };
            }
            catch (e) {
                return { isValid: false, value: _0n };
            }
        }
        : opts.uvRatio;
    // Validate whether the passed curve params are valid.
    // equation ax² + y² = 1 + dx²y² should work for generator point.
    if (!isEdValidXY(Fp, CURVE, CURVE.Gx, CURVE.Gy))
        throw new Error('bad curve params: generator point');
    // Multiplication by param `a` sits on the double() / add() hot paths. For the common twists
    // a=-1 (ed25519, jubjub) and a=1 (ed448) the full field multiplication is replaced with
    // negation / identity. Selection depends only on public curve constants.
    const mulA = Fp.eql(CURVE.a, Fp.neg(Fp.ONE)) ? (x) => Fp.neg(x)
        : Fp.eql(CURVE.a, Fp.ONE) ? (x) => x
            : (x) => Fp.mul(CURVE.a, x); // prettier-ignore
    /**
     * Asserts coordinate is valid: 0 <= n < MASK.
     * Coordinates >= Fp.ORDER are allowed for zip215.
     */
    function acoord(title, n, banZero = false) {
        const min = banZero ? _1n : _0n;
        aInRange('coordinate ' + title, n, min, MASK);
        return n;
    }
    function aedpoint(other) {
        if (!(other instanceof Point))
            throw new Error('EdwardsPoint expected');
    }
    // Extended Point works in extended coordinates: (X, Y, Z, T) ∋ (x=X/Z, y=Y/Z, T=xy).
    // https://en.wikipedia.org/wiki/Twisted_Edwards_curve#Extended_coordinates
    class Point {
        static BASE = new Point(CURVE.Gx, CURVE.Gy, Fp.ONE, Fp.mul(CURVE.Gx, CURVE.Gy));
        static ZERO = new Point(Fp.ZERO, Fp.ONE, Fp.ONE, Fp.ZERO);
        static Fp = Fp;
        static Fn = Fn;
        X;
        Y;
        Z;
        T;
        constructor(X, Y, Z, T) {
            this.X = acoord('x', X);
            this.Y = acoord('y', Y);
            this.Z = acoord('z', Z, true);
            this.T = acoord('t', T);
            Object.freeze(this);
        }
        static CURVE() {
            return CURVE;
        }
        /**
         * Create one extended Edwards point from affine coordinates.
         * Does NOT validate that the point is on-curve or torsion-free.
         * Use `.assertValidity()` on adversarial inputs.
         */
        static fromAffine(p) {
            if (p instanceof Point)
                throw new Error('extended point not allowed');
            const { x, y } = p || {};
            acoord('x', x);
            acoord('y', y);
            return new Point(x, y, Fp.ONE, Fp.mul(x, y));
        }
        // Uses algo from RFC8032 5.1.3.
        static fromBytes(bytes, zip215 = false) {
            const len = Fp.BYTES;
            const { a, d } = CURVE;
            bytes = copyBytes(abytes(bytes, len, 'point'));
            abool(zip215, 'zip215');
            const normed = copyBytes(bytes); // copy again, we'll manipulate it
            const lastByte = bytes[len - 1]; // select last byte
            normed[len - 1] = lastByte & ~0x80; // clear last bit
            const y = bytesToNumberLE(normed);
            // zip215=true is good for consensus-critical apps. =false follows RFC8032 / NIST186-5.
            // RFC8032 prohibits >= p, but ZIP215 doesn't
            // zip215=true:  0 <= y < MASK (2^256 for ed25519)
            // zip215=false: 0 <= y < P (2^255-19 for ed25519)
            const max = zip215 ? MASK : Fp.ORDER;
            aInRange('point.y', y, _0n, max);
            // Ed25519: x² = (y²-1)/(dy²+1) mod p. Ed448: x² = (y²-1)/(dy²-1) mod p. Generic case:
            // ax²+y²=1+dx²y² => y²-1=dx²y²-ax² => y²-1=x²(dy²-a) => x²=(y²-1)/(dy²-a)
            const y2 = Fp.sqr(y); // denominator is always non-0 mod p.
            const u = Fp.sub(y2, Fp.ONE); // u = y² - 1
            const v = Fp.sub(Fp.mulN(d, y2), a); // v = d y² - a.
            let { isValid, value: x } = uvRatio(u, v); // √(u/v)
            if (!isValid)
                throw new Error('bad point: invalid y coordinate');
            const isXOdd = isOdd(x); // There are 2 square roots. Use x_0 bit to select proper
            const isLastByteOdd = (lastByte & 0x80) !== 0; // x_0, last bit
            if (!zip215 && Fp.is0(x) && isLastByteOdd)
                // if x=0 and x_0 = 1, fail
                throw new Error('bad point: x=0 and x_0=1');
            if (isLastByteOdd !== isXOdd)
                x = Fp.neg(x); // if x_0 != x mod 2, set x = p-x
            return Point.fromAffine({ x, y });
        }
        static fromHex(hex, zip215 = false) {
            return Point.fromBytes(hexToBytes(hex), zip215);
        }
        get x() {
            return this.toAffine().x;
        }
        get y() {
            return this.toAffine().y;
        }
        precompute(windowSize = 6, isLazy = true) {
            wnaf.setWindowSize(this, windowSize);
            if (!isLazy)
                this.multiply(_2n); // random number
            return this;
        }
        // Useful in fromAffine() - not for fromBytes(), which always created valid points.
        assertValidity() {
            const p = this;
            const { a, d } = CURVE;
            // Keep generic Edwards validation fail-closed on the neutral point.
            // Even though ZERO is algebraically valid and can roundtrip through encodings, higher-level
            // callers often reach it only through broken hash/scalar plumbing; rejecting it here avoids
            // silently treating that degenerate state as an ordinary public point.
            if (p.is0())
                throw new Error('bad point: ZERO'); // TODO: optimize, with vars below?
            // Equation in affine coordinates: ax² + y² = 1 + dx²y²
            // Equation in projective coordinates (X/Z, Y/Z, Z):  (aX² + Y²)Z² = Z⁴ + dX²Y²
            const { X, Y, Z, T } = p;
            const X2 = Fp.sqr(X); // X²
            const Y2 = Fp.sqr(Y); // Y²
            const Z2 = Fp.sqr(Z); // Z²
            const Z4 = Fp.sqr(Z2); // Z⁴
            const aX2 = Fp.mul(X2, a); // aX²
            const left = Fp.mul(Fp.add(aX2, Y2), Z2); // (aX² + Y²)Z²
            const right = Fp.add(Z4, Fp.mul(d, Fp.mul(X2, Y2))); // Z⁴ + dX²Y²
            if (!Fp.eql(left, right))
                throw new Error('bad point: equation left != right (1)');
            // In Extended coordinates we also have T, which is x*y=T/Z: check X*Y == Z*T
            const XY = Fp.mul(X, Y);
            const ZT = Fp.mul(Z, T);
            if (!Fp.eql(XY, ZT))
                throw new Error('bad point: equation left != right (2)');
        }
        // Compare one point to another.
        equals(other) {
            aedpoint(other);
            const { X: X1, Y: Y1, Z: Z1 } = this;
            const { X: X2, Y: Y2, Z: Z2 } = other;
            const X1Z2 = Fp.mul(X1, Z2);
            const X2Z1 = Fp.mul(X2, Z1);
            const Y1Z2 = Fp.mul(Y1, Z2);
            const Y2Z1 = Fp.mul(Y2, Z1);
            return Fp.eql(X1Z2, X2Z1) && Fp.eql(Y1Z2, Y2Z1);
        }
        is0() {
            return this.equals(Point.ZERO);
        }
        negate() {
            // Flips point sign to a negative one (-x, y in affine coords)
            return new Point(Fp.neg(this.X), this.Y, this.Z, Fp.neg(this.T));
        }
        // Fast algo for doubling Extended Point.
        // https://hyperelliptic.org/EFD/g1p/auto-twisted-extended.html#doubling-dbl-2008-hwcd
        // Cost: 4M + 4S + 1*a + 6add + 1*2.
        double() {
            const { X: X1, Y: Y1, Z: Z1 } = this;
            const A = Fp.sqr(X1); // A = X12
            const B = Fp.sqr(Y1); // B = Y12
            const C = Fp.mul(Fp.sqr(Z1), _2n); // C = 2*Z12
            const D = mulA(A); // D = a*A
            const x1y1 = Fp.addN(X1, Y1);
            const E = Fp.sub(Fp.subN(Fp.sqr(x1y1), A), B); // E = (X1+Y1)2-A-B
            const G = Fp.addN(D, B); // G = D+B
            const F = Fp.subN(G, C); // F = G-C
            const H = Fp.subN(D, B); // H = D-B
            const X3 = Fp.mul(E, F); // X3 = E*F
            const Y3 = Fp.mul(G, H); // Y3 = G*H
            const T3 = Fp.mul(E, H); // T3 = E*H
            const Z3 = Fp.mul(F, G); // Z3 = F*G
            return new Point(X3, Y3, Z3, T3);
        }
        // Fast algo for adding 2 Extended Points.
        // https://hyperelliptic.org/EFD/g1p/auto-twisted-extended.html#addition-add-2008-hwcd
        // Cost: 9M + 1*a + 1*d + 7add.
        add(other) {
            aedpoint(other);
            const { d } = CURVE;
            const { X: X1, Y: Y1, Z: Z1, T: T1 } = this;
            const { X: X2, Y: Y2, Z: Z2, T: T2 } = other;
            const A = Fp.mul(X1, X2); // A = X1*X2
            const B = Fp.mul(Y1, Y2); // B = Y1*Y2
            const C = Fp.mul(Fp.mulN(T1, d), T2); // C = T1*d*T2
            const D = Fp.mul(Z1, Z2); // D = Z1*Z2
            // E = (X1+Y1)*(X2+Y2)-A-B
            const E = Fp.sub(Fp.subN(Fp.mulN(Fp.addN(X1, Y1), Fp.addN(X2, Y2)), A), B);
            const F = Fp.subN(D, C); // F = D-C
            const G = Fp.addN(D, C); // G = D+C
            const H = Fp.sub(B, mulA(A)); // H = B-a*A
            const X3 = Fp.mul(E, F); // X3 = E*F
            const Y3 = Fp.mul(G, H); // Y3 = G*H
            const T3 = Fp.mul(E, H); // T3 = E*H
            const Z3 = Fp.mul(F, G); // Z3 = F*G
            return new Point(X3, Y3, Z3, T3);
        }
        subtract(other) {
            // Validate before calling `negate()` so wrong inputs fail with the point guard
            // instead of leaking a foreign `negate()` error.
            aedpoint(other);
            return this.add(other.negate());
        }
        // Constant-time multiplication.
        multiply(scalar) {
            // 1 <= scalar < L
            // Keep the subgroup-scalar contract strict instead of reducing 0 / n to ZERO.
            // In keygen/signing-style callers, those values usually mean broken hash/scalar plumbing,
            // and failing closed is safer than silently producing the identity point.
            if (!Fn.isValidNot0(scalar))
                throw new RangeError('invalid scalar: expected 1 <= sc < curve.n');
            const { p, f } = wnaf.mulSecret(this, scalar, cofactor, normalize);
            return normalize([p, f])[0];
        }
        // Non-constant-time multiplication. Uses double-and-add algorithm.
        // It's faster, but should only be used when you don't care about
        // an exposed private key e.g. sig verification.
        // Keeps the same subgroup-scalar contract: 0 is allowed for public-scalar callers, but
        // n and larger values are rejected instead of being reduced mod n to the identity point.
        multiplyUnsafe(scalar) {
            // 0 <= scalar < L
            if (!Fn.isValid(scalar))
                throw new RangeError('invalid scalar: expected 0 <= sc < curve.n');
            if (scalar === _0n)
                return Point.ZERO;
            if (this.is0() || scalar === _1n)
                return this;
            return wnaf.mulUnsafe(this, scalar, normalize);
        }
        // Checks if point is of small order.
        // If you add something to small order point, you will have "dirty"
        // point with torsion component.
        // Clears cofactor and checks if the result is 0.
        isSmallOrder() {
            return this.clearCofactor().is0();
        }
        // Multiplies point by curve order and checks if the result is 0.
        // Returns `false` is the point is dirty.
        isTorsionFree() {
            return wnaf.mulUnsafe(this, CURVE.n).is0();
        }
        // Converts Extended point to default (x, y) coordinates.
        // Can accept precomputed Z^-1 - for example, from invertBatch.
        toAffine(invertedZ) {
            const p = this;
            let iz = invertedZ;
            if (iz != null && typeof iz !== 'bigint')
                throw new TypeError('"invertedZ" expected bigint, got type=' + typeof iz);
            const { X, Y, Z } = p;
            const is0 = p.is0();
            if (iz == null)
                iz = is0 ? Fp.create(_8n) : Fp.inv(Z);
            const x = Fp.mul(X, iz);
            const y = Fp.mul(Y, iz);
            const zz = Fp.mul(Z, iz);
            if (is0)
                return { x: Fp.ZERO, y: Fp.ONE };
            if (!Fp.eql(zz, Fp.ONE))
                throw new Error('invZ was invalid');
            return { x, y };
        }
        clearCofactor() {
            if (cofactor === _1n)
                return this;
            // 2.8-3.8x speed-up vs naive
            if (cofactor === _2n)
                return this.double();
            if (cofactor === _4n)
                return this.double().double();
            if (cofactor === _8n)
                return this.double().double().double();
            return this.multiplyUnsafe(cofactor);
        }
        toBytes() {
            const { x, y } = this.toAffine();
            // Fp.toBytes() allows non-canonical encoding of y (>= p).
            const bytes = Fp.toBytes(y);
            // Each y has 2 valid points: (x, y), (x,-y).
            // When compressing, it's enough to store y and use the last byte to encode sign of x
            bytes[bytes.length - 1] |= isOdd(x) ? 0x80 : 0;
            return bytes;
        }
        toHex() {
            return bytesToHex(this.toBytes());
        }
        toString() {
            return `<Point ${this.is0() ? 'ZERO' : this.toHex()}>`;
        }
    }
    // Keep constructor work cheap: subgroup/generator validation belongs to the caller's curve
    // parameters, and doing the extra checks here adds about 10-15ms to heavy module imports.
    // Callers that construct custom curves are responsible for supplying the correct base point.
    // try {
    //   Point.BASE.assertValidity();
    //   if (!Point.BASE.isTorsionFree()) throw new Error('bad point: not in prime-order subgroup');
    // } catch {
    //   throw new Error('bad curve params: generator point');
    // }
    const normalize = (points) => normalizeZ(Point, points);
    const wnaf = new ScalarMultiplier(Point, randomBytes);
    // Enable W=6 wNAF precomputes. Slows down first publicKey computation.
    // Disable for tiny toy curves, with scalar fields < 6 bits.
    if (wnaf.bits >= 6)
        Point.BASE.precompute(6);
    Object.freeze(Point.prototype);
    Object.freeze(Point);
    return Point;
}
/**
 * Base class for prime-order points like Ristretto255 and Decaf448.
 * These points eliminate cofactor issues by representing equivalence classes
 * of Edwards curve points. Multiple Edwards representatives can describe the
 * same abstract wrapper element, so wrapper validity is not the same thing as
 * the hidden representative being torsion-free.
 * @param ep - Backing Edwards point.
 * @example
 * Base class for prime-order points like Ristretto255 and Decaf448.
 *
 * ```ts
 * import { ristretto255 } from '@noble/curves/ed25519.js';
 * const point = ristretto255.Point.BASE.multiply(2n);
 * ```
 */
export class PrimeEdwardsPoint {
    static BASE;
    static ZERO;
    static Fp;
    static Fn;
    ep;
    /**
     * Wrap one internal Edwards representative directly.
     * This is not a canonical encoding boundary: alternate Edwards
     * representatives may still describe the same abstract wrapper element.
     */
    constructor(ep) {
        this.ep = ep;
    }
    // Static methods that must be implemented by subclasses
    static fromBytes(_bytes) {
        notImplemented();
    }
    static fromHex(_hex) {
        notImplemented();
    }
    get x() {
        return this.toAffine().x;
    }
    get y() {
        return this.toAffine().y;
    }
    // Common implementations
    clearCofactor() {
        // no-op for the abstract prime-order wrapper group; this is about the
        // wrapper element, not the hidden Edwards representative.
        return this;
    }
    assertValidity() {
        // Keep wrapper validity at the abstract-group boundary. Canonical decode
        // may choose Edwards representatives that differ by small torsion, so
        // checking `this.ep.isTorsionFree()` here would reject valid wrapper points.
        this.ep.assertValidity();
    }
    /**
     * Return affine coordinates of the current internal Edwards representative.
     * This is a convenience helper, not a canonical Ristretto/Decaf encoding.
     * Equal abstract elements may expose different `x` / `y`; use
     * `toBytes()` / `fromBytes()` for canonical roundtrips.
     */
    toAffine(invertedZ) {
        return this.ep.toAffine(invertedZ);
    }
    toHex() {
        return bytesToHex(this.toBytes());
    }
    toString() {
        return this.toHex();
    }
    isTorsionFree() {
        // Abstract Ristretto/Decaf elements are already prime-order even when the
        // hidden Edwards representative is not torsion-free.
        return true;
    }
    isSmallOrder() {
        return false;
    }
    add(other) {
        this.assertSame(other);
        return this.init(this.ep.add(other.ep));
    }
    subtract(other) {
        this.assertSame(other);
        return this.init(this.ep.subtract(other.ep));
    }
    multiply(scalar) {
        return this.init(this.ep.multiply(scalar));
    }
    multiplyUnsafe(scalar) {
        return this.init(this.ep.multiplyUnsafe(scalar));
    }
    double() {
        return this.init(this.ep.double());
    }
    negate() {
        return this.init(this.ep.negate());
    }
    precompute(windowSize, isLazy) {
        this.ep.precompute(windowSize, isLazy);
        // Keep the wrapper identity stable like the backing Edwards API instead of
        // allocating a fresh wrapper around the same cached point.
        return this;
    }
}
/**
 * Initializes EdDSA signatures over given Edwards curve.
 * @param Point - Edwards point constructor.
 * @param cHash - Hash function.
 * @param eddsaOpts - Optional signature helpers. See {@link EdDSAOpts}.
 * @returns EdDSA helper namespace.
 * @throws If the hash function, options, or derived point operations are invalid. {@link Error}
 * @example
 * Initializes EdDSA signatures over given Edwards curve.
 *
 * ```ts
 * import { eddsa } from '@noble/curves/abstract/edwards.js';
 * import { jubjub } from '@noble/curves/misc.js';
 * import { sha512 } from '@noble/hashes/sha2.js';
 * const sigs = eddsa(jubjub.Point, sha512);
 * const { secretKey, publicKey } = sigs.keygen();
 * const msg = new TextEncoder().encode('hello noble');
 * const sig = sigs.sign(msg, secretKey);
 * const isValid = sigs.verify(sig, msg, publicKey);
 * ```
 */
export function eddsa(Point, cHash, eddsaOpts = {}) {
    validatePointCons(Point);
    if (typeof cHash !== 'function')
        throw new Error('"hash" function param is required');
    const hash = cHash;
    const opts = eddsaOpts;
    validateObject(opts, {}, {
        adjustScalarBytes: 'function',
        randomBytes: 'function',
        domain: 'function',
        prehash: 'function',
        zip215: 'boolean',
        mapToCurve: 'function',
        toMontgomery: 'function',
        toMontgomerySecret: 'function',
    });
    const { prehash } = opts;
    const { BASE, Fp, Fn } = Point;
    const outputLen = hash.outputLen;
    const expectedLen = 2 * Fp.BYTES;
    // When hash metadata is available, reject incompatible EdDSA wrappers at construction time
    // instead of deferring the mismatch until the first keygen/sign call.
    if (outputLen !== undefined) {
        asafenumber(outputLen, 'hash.outputLen');
        if (outputLen !== expectedLen)
            throw new Error(`hash.outputLen must be ${expectedLen}, got ${outputLen}`);
    }
    const randomBytes = opts.randomBytes === undefined ? wcRandomBytes : opts.randomBytes;
    const toMontgomery = opts.toMontgomery;
    const toMontgomerySecret = opts.toMontgomerySecret;
    const adjustScalarBytes = opts.adjustScalarBytes === undefined
        ? (bytes) => bytes
        : opts.adjustScalarBytes;
    const domain = opts.domain === undefined
        ? (data, ctx, phflag) => {
            abool(phflag, 'phflag');
            if (ctx.length || phflag)
                throw new Error('Contexts/pre-hash are not supported');
            return data;
        }
        : opts.domain; // NOOP
    // Parse an EdDSA digest as a little-endian integer and reduce it modulo the scalar field order.
    function modN_LE(hash) {
        return Fn.create(bytesToNumberLE(hash)); // Not Fn.fromBytes: it has length limit
    }
    // Get the hashed private scalar per RFC8032 5.1.5
    function getPrivateScalar(key) {
        const len = lengths.secretKey;
        abytes(key, lengths.secretKey, 'secretKey');
        // Hash private key with curve's hash function to produce uniformingly random input
        // Check byte lengths: ensure(64, h(ensure(32, key)))
        const hashed = abytes(hash(key), 2 * len, 'hashedSecretKey');
        // Slice before clamping so in-place adjustors don't corrupt the prefix half.
        const head = adjustScalarBytes(hashed.slice(0, len)); // clear first half bits, produce FE
        const prefix = hashed.slice(len, 2 * len); // second half is called key prefix (5.1.6)
        const scalar = modN_LE(head); // The actual private scalar
        return { head, prefix, scalar };
    }
    /** Convenience method that creates public key from scalar. RFC8032 5.1.5
     * Also exposes the derived scalar/prefix tuple and point form reused by sign().
     */
    function getExtendedPublicKey(secretKey) {
        const { head, prefix, scalar } = getPrivateScalar(secretKey);
        const point = BASE.multiply(scalar); // Point on Edwards curve aka public key
        const pointBytes = point.toBytes();
        return { head, prefix, scalar, point, pointBytes };
    }
    /** Calculates EdDSA pub key. RFC8032 5.1.5. */
    function getPublicKey(secretKey) {
        return getExtendedPublicKey(secretKey).pointBytes;
    }
    // Hash domain-separated chunks into a little-endian scalar modulo the group order.
    function hashDomainToScalar(context = Uint8Array.of(), ...msgs) {
        const msg = concatBytes(...msgs);
        return modN_LE(hash(domain(msg, abytes(context, undefined, 'context'), !!prehash)));
    }
    /** Signs message with secret key. RFC8032 5.1.6 */
    function sign(msg, secretKey, options = {}) {
        validateObject(options, {}, {}, 'options');
        // Snapshot once: nonce and challenge must use the same invocation-time message bytes.
        msg = copyBytes(abytes(msg, undefined, 'message'));
        if (prehash)
            msg = prehash(msg); // for ed25519ph etc.
        const { prefix, scalar, pointBytes } = getExtendedPublicKey(secretKey);
        const r = hashDomainToScalar(options.context, prefix, msg); // r = dom2(F, C) || prefix || PH(M)
        // RFC 8032 5.1.6 allows r mod L = 0, and SUPERCOP ref10 accepts the resulting identity-point
        // signature.
        // We intentionally keep the safe multiply() rejection here so a miswired all-zero hash provider
        // fails loudly instead of silently producing a degenerate signature.
        const R = BASE.multiply(r).toBytes(); // R = rG
        const k = hashDomainToScalar(options.context, R, pointBytes, msg); // R || A || PH(M)
        const s = Fn.create(r + k * scalar); // S = (r + k * s) mod L
        if (!Fn.isValid(s))
            throw new Error('sign failed: invalid s'); // 0 <= s < L
        const rs = concatBytes(R, Fn.toBytes(s));
        return abytes(rs, lengths.signature, 'result');
    }
    // Keep the shared helper strict by default: RFC 8032 / NIST-style wrappers should reject
    // non-canonical encodings unless they explicitly opt into ZIP-215's more permissive decode rules.
    const verifyOpts = {
        zip215: opts.zip215,
    };
    /**
     * Verifies EdDSA signature against message and public key. RFC 8032 §§5.1.7 and 5.2.7.
     * A cofactored verification equation is checked.
     */
    function verify(sig, msg, publicKey, options = verifyOpts) {
        // Validate before destructuring so explicit null follows the standard options error.
        validateObject(options);
        // Preserve the wrapper-selected default for `{}` / `{ zip215: undefined }`, not just omitted opts.
        const { context } = options;
        const zip215 = options.zip215 === undefined ? !!verifyOpts.zip215 : options.zip215;
        const len = lengths.signature;
        sig = abytes(sig, len, 'signature');
        msg = abytes(msg, undefined, 'message');
        publicKey = abytes(publicKey, lengths.publicKey, 'publicKey');
        if (zip215 !== undefined)
            abool(zip215, 'zip215');
        if (prehash)
            msg = prehash(msg); // for ed25519ph, etc
        const mid = len / 2;
        const r = sig.subarray(0, mid);
        const s = bytesToNumberLE(sig.subarray(mid, len));
        let A, R, SB;
        try {
            // ZIP-215 is more permissive than RFC 8032 / NIST186-5. Use it only for wrappers that
            // explicitly want consensus-style unreduced encoding acceptance.
            // zip215=true:  0 <= y < MASK (2^256 for ed25519)
            // zip215=false: 0 <= y < P (2^255-19 for ed25519)
            A = Point.fromBytes(publicKey, zip215);
            R = Point.fromBytes(r, zip215);
            SB = BASE.multiplyUnsafe(s); // 0 <= s < l is done inside
        }
        catch (error) {
            return false;
        }
        // RFC 8032 §§5.1.7/5.2.7 and FIPS 186-5 §§7.7.2/7.8.2 only decode A' and check the cofactored
        // verification equation; they do not add a separate low-order-public-key rejection here.
        // Strict mode still rejects small-order A' intentionally for SBS-style non-repudiation and to
        // avoid ambiguous verification outcomes where unusual low-order keys can make distinct
        // key/signature/message combinations verify.
        if (!zip215 && A.isSmallOrder())
            return false;
        // ZIP-215 accepts noncanonical / unreduced point encodings, so the challenge hash must use the
        // exact signature/public-key bytes rather than canonicalized re-encodings of the decoded points.
        const k = hashDomainToScalar(context, r, publicKey, msg);
        const RkA = R.add(A.multiplyUnsafe(k));
        // Check the cofactored verification equation via the curve cofactor h.
        // [h][S]B = [h]R + [h][k]A'
        return RkA.subtract(SB).clearCofactor().is0();
    }
    const _size = Fp.BYTES; // 32 for ed25519, 57 for ed448
    const lengths = {
        secretKey: _size,
        publicKey: _size,
        signature: 2 * _size,
        seed: _size,
    };
    function randomSecretKey(seed) {
        seed = seed === undefined ? randomBytes(lengths.seed) : seed;
        return abytes(seed, lengths.seed, 'seed');
    }
    function isValidSecretKey(key) {
        return isBytes(key) && key.length === lengths.secretKey;
    }
    function isValidPublicKey(key, zip215) {
        try {
            // Preserve the wrapper-selected default for omitted / `undefined` ZIP-215 flags here too.
            return !!Point.fromBytes(key, zip215 === undefined ? verifyOpts.zip215 : zip215);
        }
        catch (error) {
            return false;
        }
    }
    const utils = {
        getExtendedPublicKey,
        randomSecretKey,
        isValidSecretKey,
        isValidPublicKey,
        /** Converts an Edwards public key to a companion Montgomery public key. */
        toMontgomery(publicKey) {
            if (toMontgomery === undefined)
                throw new Error('Montgomery conversion is not supported for this curve');
            return toMontgomery(Point.fromBytes(publicKey));
        },
        toMontgomerySecret(secretKey) {
            if (toMontgomerySecret === undefined)
                throw new Error('Montgomery conversion is not supported for this curve');
            return toMontgomerySecret(secretKey);
        },
    };
    Object.freeze(lengths);
    Object.freeze(utils);
    return Object.freeze({
        keygen: createKeygen(randomSecretKey, getPublicKey),
        getPublicKey,
        sign,
        verify,
        utils,
        Point,
        lengths,
    });
}
