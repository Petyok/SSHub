/**
 * FROST: Flexible Round-Optimized Schnorr Threshold Protocol for Two-Round Schnorr Signatures.
 *
 * See {@link https://datatracker.ietf.org/doc/rfc9591/ | RFC 9591} and
 * {@link https://frost.zfnd.org | frost.zfnd.org}.
 * @module
 */
import { utf8ToBytes } from '@noble/hashes/utils.js';
import { aarray, abytes, asafenumber, astring, bytesToHex, bytesToNumberBE, bytesToNumberLE, concatBytes, copyBytes, equalBytes, hexToBytes, randomBytes, validateObject, } from "../utils.js";
import { mulAddUnsafe, validatePointCons } from "./curve.js";
import { poly } from "./fft.js";
import {} from "./hash-to-curve.js";
import { getMinHashLength, mapHashToField } from "./modular.js";
// PubKey = commitments, verifyingShares
// PrivKey = id, signingShare, commitment
const validateSigners = (signers, title = 'signers') => {
    validateObject(signers, { min: 'number', max: 'number' }, {}, title);
    asafenumber(signers.min, title + '.min');
    asafenumber(signers.max, title + '.max');
    // Compatibility with frost-rs intentionally narrows RFC 9591's positive-nonzero threshold rule
    // to `min >= 2`, even though the RFC text itself allows `MIN_PARTICIPANTS = 1`.
    // This API is for actual threshold signing across participants; 1-of-n degenerates to ordinary
    // single-signer mode, which does not need FROST's network/coordination machinery at all.
    if (signers.min < 2 || signers.max < 2 || signers.min > signers.max)
        throw new Error('Wrong signers info: min=' + signers.min + ' max=' + signers.max);
};
const validateCommitmentsNum = (signers, len) => {
    // RFC 9591 Sections 5.2/5.3 require MIN_PARTICIPANTS <= NUM_PARTICIPANTS <= MAX_PARTICIPANTS.
    if (len < signers.min || len > signers.max)
        throw new Error('Wrong number of commitments=' + len);
};
class AggErr extends Error {
    // Empty means aggregation failed before per-share verification could attribute a signer.
    cheaters;
    constructor(msg, cheaters) {
        super(msg);
        this.cheaters = cheaters;
    }
}
/**
 * Builds a FROST ciphersuite API from concrete curve and hash hooks.
 * @param opts - Ciphersuite construction options. See {@link FrostOpts}.
 * @returns FROST API bound to the supplied ciphersuite.
 * @example
 * Create a suite from a curve-specific option object.
 * ```ts
 * import { createFROST } from '@noble/curves/abstract/frost.js';
 * import { ed25519 } from '@noble/curves/ed25519.js';
 * import { sha512 } from '@noble/hashes/sha2.js';
 * const frost = createFROST({
 *   name: 'FROST-ED25519-SHA512-v1',
 *   Point: ed25519.Point,
 *   hash: sha512,
 * });
 * ```
 */
export function createFROST(opts) {
    validateObject(opts, {
        name: 'string',
        hash: 'function',
    }, {
        hashToScalar: 'function',
        validatePoint: 'function',
        parsePublicKey: 'function',
        adjustScalar: 'function',
        adjustPoint: 'function',
        challenge: 'function',
        adjustNonces: 'function',
        adjustSecret: 'function',
        adjustPublic: 'function',
        adjustGroupCommitmentShare: 'function',
        adjustTx: 'object',
        adjustDKG: 'function',
    });
    // Cheap constructor-surface sanity check only: this verifies the generic static hooks/fields that
    // FROST consumes, but it does not certify point semantics like BASE/ZERO correctness.
    validatePointCons(opts.Point);
    const { Point, validatePoint, parsePublicKey, adjustScalar, adjustPoint: adjustPointHook, challenge, adjustNonces, adjustSecret, adjustPublic, adjustGroupCommitmentShare, adjustDKG, } = opts;
    const Fn = opts.Fn === undefined ? Point.Fn : opts.Fn;
    const adjustTx = opts.adjustTx === undefined
        ? undefined
        : { encode: opts.adjustTx.encode, decode: opts.adjustTx.decode };
    if (adjustTx)
        validateObject(adjustTx, { encode: 'function', decode: 'function' });
    // Hashes
    const hashBytes = opts.hash;
    const hashToScalar = opts.hashToScalar === undefined
        ? (msg, opts = { DST: new Uint8Array() }) => {
            const t = hashBytes(concatBytes(opts.DST, msg));
            return Fn.create(Fn.isLE ? bytesToNumberLE(t) : bytesToNumberBE(t));
        }
        : opts.hashToScalar;
    const H1Prefix = utf8ToBytes(opts.H1 !== undefined ? opts.H1 : opts.name + 'rho');
    const H2Prefix = utf8ToBytes(opts.H2 !== undefined ? opts.H2 : opts.name + 'chal');
    const H3Prefix = utf8ToBytes(opts.H3 !== undefined ? opts.H3 : opts.name + 'nonce');
    const H4Prefix = utf8ToBytes(opts.H4 !== undefined ? opts.H4 : opts.name + 'msg');
    const H5Prefix = utf8ToBytes(opts.H5 !== undefined ? opts.H5 : opts.name + 'com');
    const HDKGPrefix = utf8ToBytes(opts.HDKG !== undefined ? opts.HDKG : opts.name + 'dkg');
    const HIDPrefix = utf8ToBytes(opts.HID !== undefined ? opts.HID : opts.name + 'id');
    const H1 = (msg) => hashToScalar(msg, { DST: H1Prefix });
    // Empty H2 still passes `{ DST: new Uint8Array() }` into custom hashToScalar hooks.
    // The built-in fallback hashes that identically to omitted DST, which is how
    // the Ed25519 suite models RFC 9591's undecorated H2 challenge hash.
    const H2 = (msg) => hashToScalar(msg, { DST: H2Prefix });
    const H3 = (msg) => hashToScalar(msg, { DST: H3Prefix });
    const H4 = (msg) => hashBytes(concatBytes(H4Prefix, msg));
    const H5 = (msg) => hashBytes(concatBytes(H5Prefix, msg));
    const HDKG = (msg) => hashToScalar(msg, { DST: HDKGPrefix });
    const HID = (msg) => hashToScalar(msg, { DST: HIDPrefix });
    // /Hashes
    const randomScalar = (rng = randomBytes) => {
        if (typeof rng !== 'function')
            throw new TypeError('"rng" expected function, got type=' + typeof rng);
        // Intentional divergence from RFC 9591 §4.1 / §5.1: the RFC nonce_generate helper outputs a
        // Scalar in [0, p-1], but round-one commit publishes ScalarBaseMult(nonce) values and §3.1
        // requires SerializeElement / DeserializeElement to reject the identity element. Keep noble's
        // mapHashToField generation here so round-one public nonce commitments stay in 1..n-1.
        const t = mapHashToField(rng(getMinHashLength(Fn.ORDER)), Fn.ORDER, Fn.isLE);
        // We cannot use Fn.fromBytes here because the field can have a different
        // byte width, like ed448.
        return Fn.isLE ? bytesToNumberLE(t) : bytesToNumberBE(t);
    };
    const serializePoint = (p) => p.toBytes();
    const validatePublicPoint = (p) => {
        // RFC 9591 Section 3.1 DeserializeElement rejects off-curve, identity, and non-prime-order
        // elements. This remains mandatory when parsePublicKey replaces the normal byte decoder.
        p.assertValidity();
        if (p.is0())
            throw new Error('invalid point: identity');
        if (!p.isTorsionFree())
            throw new Error('bad point: not in prime-order subgroup');
        if (validatePoint)
            validatePoint(p);
        return p;
    };
    const parsePoint = (bytes) => validatePublicPoint(Point.fromBytes(bytes));
    // RFC 9591 Sections 4.1/5.1 model each participant's round-one output as two public commitments.
    const nonceCommitments = (identifier, nonces) => ({
        identifier,
        hiding: serializePoint(Point.BASE.multiply(Fn.fromBytes(nonces.hiding))),
        binding: serializePoint(Point.BASE.multiply(Fn.fromBytes(nonces.binding))),
    });
    const adjustPoint = adjustPointHook === undefined ? (n) => n : adjustPointHook;
    // We use hex to make it easier to use inside objects
    const validateIdentifier = (n) => {
        // Identifiers are canonical non-zero scalars. Custom / derived identifiers are allowed, so this
        // is intentionally not bounded by the current signers.max slot count.
        if (!Fn.isValid(n) || Fn.is0(n))
            throw new Error('Invalid identifier ' + n);
        return n;
    };
    const serializeIdentifier = (id) => bytesToHex(Fn.toBytes(validateIdentifier(id)));
    const parseIdentifier = (id, title = 'identifier') => {
        astring(id, title);
        const n = validateIdentifier(Fn.fromBytes(hexToBytes(id)));
        // Keep string-keyed maps stable by accepting only the canonical serialized form.
        if (serializeIdentifier(n) !== id)
            throw new Error('expected canonical identifier hex');
        return n;
    };
    const copyRound1Package = (p) => ({
        identifier: serializeIdentifier(parseIdentifier(p.identifier)),
        commitment: p.commitment.map((c) => copyBytes(c)),
        proofOfKnowledge: copyBytes(p.proofOfKnowledge),
    });
    const canonicalRound1Packages = (packages) => {
        const snapshot = packages.map(copyRound1Package);
        snapshot.sort((a, b) => {
            const ai = parseIdentifier(a.identifier);
            const bi = parseIdentifier(b.identifier);
            return ai < bi ? -1 : ai > bi ? 1 : 0;
        });
        return snapshot;
    };
    const equalRound1Transcripts = (a, b) => {
        if (a.length !== b.length)
            return false;
        for (let i = 0; i < a.length; i++) {
            const p = a[i];
            const q = b[i];
            if (p.identifier !== q.identifier || p.commitment.length !== q.commitment.length)
                return false;
            for (let j = 0; j < p.commitment.length; j++) {
                if (!equalBytes(p.commitment[j], q.commitment[j]))
                    return false;
            }
            if (!equalBytes(p.proofOfKnowledge, q.proofOfKnowledge))
                return false;
        }
        return true;
    };
    const Signature = {
        // RFC 9591 Appendix A encodes signatures canonically as
        // SerializeElement(R) || SerializeScalar(z).
        encode: (R, z) => {
            let res = concatBytes(serializePoint(R), Fn.toBytes(z));
            if (adjustTx)
                res = adjustTx.encode(res);
            return res;
        },
        decode: (sig) => {
            if (adjustTx)
                sig = adjustTx.decode(sig);
            // We don't know size of point, but we know size of scalar
            const Rbytes = sig.subarray(0, -Fn.BYTES);
            const R = parsePoint(Rbytes);
            // RFC 9591 Section 3.1 SerializeElement is canonical: a signature must not verify under an
            // alternative point encoding (e.g. re-encoding a weierstrass R uncompressed as 65 bytes).
            if (serializePoint(R).length !== Rbytes.length)
                throw new Error('invalid signature encoding');
            const z = Fn.fromBytes(sig.subarray(-Fn.BYTES));
            return { R, z };
        },
    };
    // Generates pair of (scalar, point)
    const genPointScalarPair = (rng = randomBytes) => {
        let n = randomScalar(rng);
        if (adjustScalar)
            n = adjustScalar(n);
        let p = Point.BASE.multiply(n);
        return { scalar: n, point: p };
    };
    // No roots here: root-based methods will throw.
    // `poly` expects a structured roots-of-unity domain, but FROST uses an
    // arbitrary domain and only needs the non-root operations below.
    const nrErr = 'roots are unavailable in FROST polynomial mode';
    const noRoots = {
        info: { G: Fn.ZERO, oddFactor: Fn.ZERO, powerOfTwo: 0 },
        roots() {
            throw new Error(nrErr);
        },
        brp() {
            throw new Error(nrErr);
        },
        inverse() {
            throw new Error(nrErr);
        },
        omega() {
            throw new Error(nrErr);
        },
        clear() { },
    };
    const Poly = poly(Fn, noRoots);
    // Variable-time MSM over public inputs only (VSS / nonce commitments, binding factors).
    // Interleaved wNAF beats pippenger ~3x at FROST-sized inputs (n <= dozens of signers).
    const msm = (points, scalars) => mulAddUnsafe(Point, points, scalars);
    // Internal stuff uses bigints & Points, external Uint8Arrays
    const polynomialEvaluate = (x, coeffs) => {
        if (!coeffs.length)
            throw new Error('empty coefficients');
        return Poly.monomial.eval(coeffs, x);
    };
    const deriveInterpolatingValue = (L, xi) => {
        const err = 'invalid parameters';
        // Generates lagrange coefficient
        if (!L.some((x) => Fn.eql(x, xi)))
            throw new Error(err);
        // Throws error if any x-coordinate is represented more than once in L.
        const Lset = new Set(L);
        if (Lset.size !== L.length)
            throw new Error(err);
        // Or if xi is missing
        if (!Lset.has(xi))
            throw new Error(err);
        let num = Fn.ONE;
        let den = Fn.ONE;
        for (const x of L) {
            if (Fn.eql(x, xi))
                continue;
            num = Fn.mul(num, x); // num *= x
            den = Fn.mul(den, Fn.sub(x, xi)); // RFC 9591 §4.2: denominator *= x_j - x_i
        }
        return Fn.div(num, den);
    };
    const evalutateVSS = (identifier, commitment) => {
        // RFC 9591 Appendix C.2: S_i' = Σ_j ScalarMult(vss_commitment[j], i^j).
        const monomial = Poly.monomial.basis(identifier, commitment.length);
        return msm(commitment, monomial);
    };
    // High-level internal stuff
    const generateSecretPolynomial = (signers, secret, coeffs, rng = randomBytes) => {
        validateSigners(signers);
        if (secret !== undefined)
            abytes(secret, Fn.BYTES, 'secret');
        if (coeffs !== undefined)
            aarray(coeffs, 'coeffs');
        if (typeof rng !== 'function')
            throw new TypeError('"rng" expected function, got type=' + typeof rng);
        // Dealer/DKG polynomial sampling reuses the same hardened scalar derivation as round-one
        // nonces: overriding `rng` only swaps the entropy source, not the non-zero `1..n-1` policy.
        const secretScalar = secret === undefined ? randomScalar(rng) : Fn.fromBytes(secret);
        if (!coeffs) {
            coeffs = [];
            for (let i = 0; i < signers.min - 1; i++)
                coeffs.push(randomScalar(rng));
        }
        if (coeffs.length !== signers.min - 1)
            throw new Error('wrong coefficients length');
        const coefficients = [secretScalar, ...coeffs];
        // RFC 9591 Appendix C.2 commits to every polynomial coefficient with ScalarBaseMult.
        const commitment = coefficients.map((i) => Point.BASE.multiply(i));
        return { coefficients, commitment, secret: secretScalar };
    };
    // Pretty much sign+verify, same as basic
    const ProofOfKnowledge = {
        challenge: (id, verKey, R) => HDKG(concatBytes(Fn.toBytes(id), serializePoint(verKey), serializePoint(R))),
        compute(id, coefficents, commitments, rng = randomBytes) {
            if (coefficents.length < 1)
                throw new Error('coefficients should have at least one element');
            const { point: R, scalar: k } = genPointScalarPair(rng);
            const verKey = commitments[0]; // verify key is first one
            const c = this.challenge(id, verKey, R);
            const mu = Fn.add(k, Fn.mul(coefficents[0], c)); // mu = k + coeff[0] * c
            return Signature.encode(R, mu);
        },
        validate(id, commitment, proof) {
            if (commitment.length < 1)
                throw new Error('commitment should have at least one element');
            const { R, z } = Signature.decode(proof);
            const phi = parsePoint(commitment[0]);
            const c = this.challenge(id, phi, R);
            // R === z*G - phi*c. All inputs are public: variable-time multiplication is safe here.
            if (!R.equals(Point.BASE.multiplyUnsafe(z).subtract(phi.multiplyUnsafe(c))))
                throw new Error('invalid proof of knowledge');
        },
    };
    const Basic = {
        challenge: (R, PK, msg) => {
            if (challenge)
                return challenge(R, PK, msg);
            return H2(concatBytes(serializePoint(R), serializePoint(PK), msg));
        },
        sign(msg, sk, rng = randomBytes) {
            const { point: R, scalar: r } = genPointScalarPair(rng);
            const PK = Point.BASE.multiply(sk); // sk*G
            const c = this.challenge(R, PK, msg);
            const z = Fn.add(r, Fn.mul(c, sk)); // r + c * sk
            return [R, z];
        },
        verify(msg, R, z, PK) {
            if (adjustPointHook)
                PK = adjustPointHook(PK);
            if (adjustPointHook)
                R = adjustPointHook(R);
            // Signature, message and public key are all public: variable-time is safe on this path.
            const c = this.challenge(R, PK, msg);
            const zB = Point.BASE.multiplyUnsafe(z); // z*G
            const cA = PK.multiplyUnsafe(c); // c*PK
            let check = zB.subtract(cA).subtract(R); // zB - cA - R
            // No clearCoffactor on ristretto
            if (check.clearCofactor)
                check = check.clearCofactor();
            return Point.ZERO.equals(check);
        },
    };
    // === vssVerify
    const validateSecretShare = (identifier, commitment, signingShare) => {
        // RFC 9591 Appendix C.2 `vss_verify(share_i, vss_commitment)` is purely algebraic.
        // Public FROST packages still go through Section 3.1 element encoding,
        // which rejects identity points, so a zero share or commitment does not
        // become valid wire data just because VSS matches.
        if (!Point.BASE.multiply(signingShare).equals(evalutateVSS(identifier, commitment)))
            throw new Error('invalid secret share');
    };
    const Identifier = {
        fromNumber(n) {
            if (!Number.isSafeInteger(n))
                throw new Error('expected safe interger');
            return serializeIdentifier(BigInt(n));
        },
        // Not in spec, but in FROST implementation,
        // seems useful and nice, no need to sync identifiers (would require more interactions)
        derive(s) {
            astring(s, 's');
            // Derived identifiers may land anywhere in the scalar field; they are not restricted to
            // sequential `1..max_signers` values.
            return serializeIdentifier(HID(utf8ToBytes(s)));
        },
    };
    // RFC 9591 §4.1: nonce_generate() hashes 32 fresh RNG bytes with SerializeScalar(secret).
    const generateNonce = (secret, rng = randomBytes) => H3(concatBytes(rng(32), Fn.toBytes(secret)));
    const getGroupCommitment = (GPK, commitmentList, msg) => {
        const CL = commitmentList.map((i) => [
            i.identifier,
            parseIdentifier(i.identifier),
            parsePoint(i.hiding),
            parsePoint(i.binding),
        ]);
        // RFC 9591 Sections 4.3/4.4/4.5 and 5.2/5.3 treat commitment_list as sorted by identifier.
        CL.sort((a, b) => (a[1] < b[1] ? -1 : a[1] > b[1] ? 1 : 0));
        // Encode commitment list
        const Cbytes = [];
        for (const [_, id, hC, bC] of CL)
            Cbytes.push(Fn.toBytes(id), serializePoint(hC), serializePoint(bC));
        const encodedCommitmentHash = H5(concatBytes(...Cbytes));
        const rhoPrefix = concatBytes(serializePoint(GPK), H4(msg), encodedCommitmentHash);
        // Compute binding factors
        const bindingFactors = {};
        for (const [i, id] of CL) {
            bindingFactors[i] = H1(concatBytes(rhoPrefix, Fn.toBytes(id)));
        }
        // Hiding commitments all carry scalar 1, so add them directly and keep only the
        // binding commitments in the MSM: same result, half the MSM size.
        let hidingSum = Point.ZERO;
        const points = [];
        const scalars = [];
        for (const [i, _, hC, bC] of CL) {
            if (Point.ZERO.equals(hC) || Point.ZERO.equals(bC))
                throw new Error('infinity commitment');
            hidingSum = hidingSum.add(hC);
            points.push(bC);
            scalars.push(bindingFactors[i]);
        }
        const groupCommitment = hidingSum.add(msm(points, scalars)); //  GC += hC + bC*bindingFactor
        const identifiers = CL.map((i) => i[1]);
        return { identifiers, groupCommitment, bindingFactors };
    };
    const prepareShare = (PK, commitmentList, msg, identifier) => {
        // RFC 9591 Sections 4.4/4.5/4.6 feed directly into the Section 5.2 signer computation.
        const GPK = adjustPoint(parsePoint(PK));
        const id = parseIdentifier(identifier);
        const { identifiers, groupCommitment, bindingFactors } = getGroupCommitment(GPK, commitmentList, msg);
        const bindingFactor = bindingFactors[identifier];
        const lambda = deriveInterpolatingValue(identifiers, id);
        const challenge = Basic.challenge(groupCommitment, GPK, msg);
        return { lambda, challenge, bindingFactor, groupCommitment };
    };
    Object.freeze(Identifier);
    const frost = {
        Identifier,
        // DKG is Distributed Key Generation, not Trusted Dealer Key Generation.
        DKG: Object.freeze({
            // NOTE: we allow to pass secret scalar from user side,
            // this way it can be derived, instead of random generation
            round1: (id, signers, secret, rng = randomBytes) => {
                const idNum = parseIdentifier(id, 'id');
                validateSigners(signers);
                const { coefficients, commitment } = generateSecretPolynomial(signers, secret, undefined, rng);
                const proofOfKnowledge = ProofOfKnowledge.compute(idNum, coefficients, commitment, rng);
                const commitmentBytes = commitment.map(serializePoint);
                const round1Public = {
                    identifier: serializeIdentifier(idNum),
                    commitment: commitmentBytes,
                    proofOfKnowledge,
                };
                // store secret information for signing
                const round1Secret = {
                    identifier: idNum,
                    coefficients,
                    commitment: commitment.map(serializePoint),
                    // Copy threshold metadata instead of retaining the caller-owned object by reference.
                    signers: { min: signers.min, max: signers.max },
                    step: 1,
                };
                return { public: round1Public, secret: round1Secret };
            },
            round2: (secret, others) => {
                validateObject(secret, { identifier: 'bigint', commitment: 'object', signers: 'object' }, { coefficients: 'object', round1Cache: 'object', round2Cache: 'object', step: 'number' }, 'secret');
                validateSigners(secret.signers, 'secret.signers');
                aarray(others, 'others');
                if (others.length !== secret.signers.max - 1)
                    throw new Error('wrong number of round1 packages');
                if (!secret.coefficients || secret.step === 3)
                    throw new Error('round3 package used in round2');
                // Snapshot before validation, then authenticate and cache this exact owned transcript.
                const authenticatedRound1 = canonicalRound1Packages(others);
                if (secret.round2Cache !== undefined) {
                    if (secret.round1Cache === undefined ||
                        !equalRound1Transcripts(secret.round1Cache, authenticatedRound1))
                        throw new Error('round1 packages do not match authenticated transcript');
                    return secret.round2Cache;
                }
                const res = {};
                for (const p of authenticatedRound1) {
                    if (p.commitment.length !== secret.signers.min)
                        throw new Error('wrong number of commitments');
                    const id = parseIdentifier(p.identifier);
                    if (id === secret.identifier)
                        throw new Error('duplicate id=' + serializeIdentifier(id));
                    ProofOfKnowledge.validate(id, p.commitment, p.proofOfKnowledge);
                    for (const c of p.commitment)
                        parsePoint(c);
                    if (res[p.identifier])
                        throw new Error('Duplicate id=' + id);
                    const signingShare = Fn.toBytes(polynomialEvaluate(id, secret.coefficients));
                    res[p.identifier] = {
                        identifier: serializeIdentifier(secret.identifier),
                        signingShare: signingShare,
                    };
                }
                secret.round1Cache = authenticatedRound1;
                secret.round2Cache = res;
                secret.step = 2;
                return res;
            },
            round3: (secret, round1, round2) => {
                validateObject(secret, { identifier: 'bigint', commitment: 'object', signers: 'object' }, { coefficients: 'object', round1Cache: 'object', round2Cache: 'object', step: 'number' }, 'secret');
                validateSigners(secret.signers, 'secret.signers');
                aarray(round1, 'round1');
                aarray(round2, 'round2');
                if (round1.length !== secret.signers.max - 1)
                    throw new Error('wrong length of round1 packages');
                if (!secret.coefficients || secret.step !== 2 || !secret.round1Cache)
                    throw new Error('round2 package used in round3');
                const suppliedRound1 = canonicalRound1Packages(round1);
                const authenticatedRound1 = secret.round1Cache;
                if (!equalRound1Transcripts(authenticatedRound1, suppliedRound1))
                    throw new Error('round1 packages do not match authenticated transcript');
                if (round2.length !== authenticatedRound1.length)
                    throw new Error('wrong length of round2 packages');
                const merged = {};
                // Use the authenticated owned transcript after comparison; never re-read caller input.
                for (const r1 of authenticatedRound1) {
                    if (!r1.identifier || !r1.commitment)
                        throw new Error('wrong round1 share');
                    merged[r1.identifier] = { ...r1 };
                }
                for (const r2 of round2) {
                    if (!r2.identifier || !r2.signingShare)
                        throw new Error('wrong round2 share');
                    if (!merged[r2.identifier])
                        throw new Error('round1 share for ' + r2.identifier + ' is missing');
                    merged[r2.identifier].signingShare = r2.signingShare;
                }
                if (Object.keys(merged).length !== authenticatedRound1.length)
                    throw new Error('mismatch identifiers between rounds');
                let signingShare = Fn.ZERO;
                if (secret.commitment.length !== secret.signers.min)
                    throw new Error('wrong commitments length');
                const localCommitment = secret.commitment.map(parsePoint);
                const localShare = polynomialEvaluate(secret.identifier, secret.coefficients);
                validateSecretShare(secret.identifier, localCommitment, localShare);
                const localCommitmentBytes = localCommitment.map(serializePoint);
                const commitments = {
                    [serializeIdentifier(secret.identifier)]: localCommitmentBytes,
                };
                for (const k in merged) {
                    const v = merged[k];
                    if (!v.signingShare || !v.commitment)
                        throw new Error('mismatch identifiers');
                    const id = parseIdentifier(k); // from
                    const signingSharePart = Fn.fromBytes(v.signingShare);
                    const commitment = v.commitment.map(parsePoint);
                    validateSecretShare(secret.identifier, commitment, signingSharePart);
                    signingShare = Fn.add(signingShare, signingSharePart);
                    const idSer = serializeIdentifier(id);
                    if (commitments[idSer])
                        throw new Error('duplicated id=' + idSer);
                    commitments[idSer] = v.commitment;
                }
                signingShare = Fn.add(signingShare, localShare);
                const mergedCommitment = new Array(secret.signers.min).fill(Point.ZERO);
                for (const k in commitments) {
                    const v = commitments[k];
                    if (v.length !== secret.signers.min)
                        throw new Error('wrong commitments length');
                    for (let i = 0; i < v.length; i++)
                        mergedCommitment[i] = mergedCommitment[i].add(parsePoint(v[i]));
                }
                const mergedCommitmentBytes = mergedCommitment.map(serializePoint);
                const verifyingShares = {};
                for (const k in commitments)
                    verifyingShares[k] = serializePoint(evalutateVSS(parseIdentifier(k), mergedCommitment));
                // This is enough to sign stuff
                let res = {
                    public: {
                        signers: { min: secret.signers.min, max: secret.signers.max },
                        commitments: mergedCommitmentBytes,
                        verifyingShares: Object.fromEntries(Object.entries(verifyingShares).map(([k, v]) => [k, v.slice()])),
                    },
                    secret: {
                        identifier: serializeIdentifier(secret.identifier),
                        signingShare: Fn.toBytes(signingShare),
                    },
                };
                if (adjustDKG)
                    res = adjustDKG(res);
                for (let i = 0; i < secret.coefficients.length; i++)
                    secret.coefficients[i] -= secret.coefficients[i];
                delete secret.coefficients;
                delete secret.round1Cache;
                delete secret.round2Cache;
                secret.step = 3;
                return res;
            },
            clean(secret) {
                validateObject(secret, { identifier: 'bigint', commitment: 'object', signers: 'object' }, { coefficients: 'object', round1Cache: 'object', round2Cache: 'object', step: 'number' }, 'secret');
                // Instead of replacing secret bigint with another (zero?), we subtract it from itself
                // in the hope that JIT will modify it inplace, instead of creating new value.
                // This is unverified and may not work, but it is best we can do in regard of bigints.
                secret.identifier -= secret.identifier;
                if (secret.coefficients) {
                    for (let i = 0; i < secret.coefficients.length; i++)
                        secret.coefficients[i] -= secret.coefficients[i];
                }
                // for (const c of secret.commitment) c.fill(0);
                delete secret.round1Cache;
                delete secret.round2Cache;
                secret.step = 3;
            },
        }),
        // Trusted dealer setup
        // Generates keys for all participants
        trustedDealer(signers, identifiers, secret, rng = randomBytes) {
            // if no identifiers provided, we generated default identifiers
            validateSigners(signers);
            if (identifiers === undefined) {
                identifiers = [];
                for (let i = 1; i <= signers.max; i++)
                    identifiers.push(Identifier.fromNumber(i));
            }
            else {
                aarray(identifiers, 'identifiers');
                if (identifiers.length !== signers.max)
                    throw new Error('identifiers should be array of ' + signers.max);
            }
            const identifierNums = {};
            for (const id of identifiers) {
                const idNum = parseIdentifier(id);
                if (id in identifierNums)
                    throw new Error('duplicated id=' + id);
                identifierNums[id] = idNum;
            }
            const sp = generateSecretPolynomial(signers, secret, undefined, rng);
            const commitmentBytes = sp.commitment.map(serializePoint);
            const secretShares = {};
            const verifyingShares = {};
            for (const id of identifiers) {
                const signingShare = polynomialEvaluate(identifierNums[id], sp.coefficients);
                verifyingShares[id] = serializePoint(Point.BASE.multiply(signingShare));
                secretShares[id] = {
                    identifier: id,
                    signingShare: Fn.toBytes(signingShare),
                };
            }
            return {
                public: {
                    signers: { min: signers.min, max: signers.max },
                    commitments: commitmentBytes,
                    verifyingShares,
                },
                secretShares,
            };
        },
        // Validate secret (from trusted dealer or DKG)
        validateSecret(secret, pub) {
            validateObject(secret, { identifier: 'string', signingShare: 'object' }, {}, 'secret');
            abytes(secret.signingShare, Fn.BYTES, 'secret.signingShare');
            validateObject(pub, {
                signers: 'object',
                commitments: 'object',
                verifyingShares: 'object',
            }, {}, 'pub');
            validateSigners(pub.signers, 'pub.signers');
            aarray(pub.commitments, 'pub.commitments');
            const id = parseIdentifier(secret.identifier);
            const commitment = pub.commitments.map(parsePoint);
            const signingShare = Fn.fromBytes(secret.signingShare);
            validateSecretShare(id, commitment, signingShare);
        },
        // Actual signing
        // Round 1: each participant commit to nonces
        // Nonces kept private, commitments sent to coordinator (or every other participant)
        // NOTE: we don't need the message at this point, which lets a coordinator
        // keep multiple nonce commitments per participant in advance and skip
        // round1 for signing.
        // But then each participant needs to remember generated shares
        commit(secret, rng = randomBytes) {
            validateObject(secret, { identifier: 'string', signingShare: 'object' }, {}, 'secret');
            abytes(secret.signingShare, Fn.BYTES, 'secret.signingShare');
            if (typeof rng !== 'function')
                throw new TypeError('"rng" expected function, got type=' + typeof rng);
            const secretScalar = Fn.fromBytes(secret.signingShare);
            const hiding = generateNonce(secretScalar, rng);
            const binding = generateNonce(secretScalar, rng);
            const nonces = { hiding: Fn.toBytes(hiding), binding: Fn.toBytes(binding) };
            return { nonces, commitments: nonceCommitments(secret.identifier, nonces) };
        },
        // Round2: sign. Each participant creates a signature share from the secret
        // and the selected nonce commitments.
        signShare(secret, pub, nonces, commitmentList, msg) {
            validateObject(secret, { identifier: 'string', signingShare: 'object' }, {}, 'secret');
            abytes(secret.signingShare, Fn.BYTES, 'secret.signingShare');
            validateObject(pub, {
                signers: 'object',
                commitments: 'object',
                verifyingShares: 'object',
            }, {}, 'pub');
            validateSigners(pub.signers, 'pub.signers');
            aarray(pub.commitments, 'pub.commitments');
            validateObject(nonces, { hiding: 'object', binding: 'object' }, {}, 'nonces');
            abytes(nonces.hiding, Fn.BYTES, 'nonces.hiding');
            abytes(nonces.binding, Fn.BYTES, 'nonces.binding');
            aarray(commitmentList, 'commitmentList');
            abytes(msg, undefined, 'msg');
            validateCommitmentsNum(pub.signers, commitmentList.length);
            const hidingNonce0 = Fn.fromBytes(nonces.hiding);
            const bindingNonce0 = Fn.fromBytes(nonces.binding);
            if (Fn.is0(hidingNonce0) || Fn.is0(bindingNonce0))
                throw new Error('signing nonces already used');
            // Reject a coordinator-assigned commitment pair that does not match the signer's own nonce
            // pair. This must happen before suite-specific nonce adjustment; secp256k1-tr may negate the
            // actual signing nonces later, but the coordinator still assigns the original commitments.
            const expectedCommitment = {
                identifier: secret.identifier,
                hiding: serializePoint(Point.BASE.multiply(hidingNonce0)),
                binding: serializePoint(Point.BASE.multiply(bindingNonce0)),
            };
            const commitment = commitmentList.find((i) => i.identifier === secret.identifier);
            if (!commitment)
                throw new Error('missing signer commitment');
            if (bytesToHex(commitment.hiding) !== bytesToHex(expectedCommitment.hiding) ||
                bytesToHex(commitment.binding) !== bytesToHex(expectedCommitment.binding))
                throw new Error('incorrect signer commitment');
            if (adjustSecret)
                secret = adjustSecret(secret, pub);
            if (adjustPublic)
                pub = adjustPublic(pub);
            const SK = Fn.fromBytes(secret.signingShare);
            const { lambda, challenge, bindingFactor, groupCommitment } = prepareShare(pub.commitments[0], commitmentList, msg, secret.identifier);
            const N = adjustNonces ? adjustNonces(groupCommitment, nonces) : nonces;
            const hidingNonce = adjustNonces ? Fn.fromBytes(N.hiding) : hidingNonce0;
            const bindingNonce = adjustNonces ? Fn.fromBytes(N.binding) : bindingNonce0;
            const t = Fn.mul(Fn.mul(lambda, SK), challenge); // challenge * lambda * SK
            const t2 = Fn.mul(bindingNonce, bindingFactor); // bindingNonce * bindingFactor
            const r = Fn.toBytes(Fn.add(Fn.add(hidingNonce, t2), t)); // t + t2 + hidingNonce
            // RFC 9591 round-one commitments are one-time-use, and round two must use the nonce
            // corresponding to the published commitment. This API returns mutable local nonce bytes,
            // so consume them after a successful signShare() call: later all-zero reuse fails closed.
            nonces.hiding.fill(0);
            nonces.binding.fill(0);
            return r;
        },
        // Each participant (or coordinator) can verify signatures from other participants
        verifyShare(pub, commitmentList, msg, identifier, sigShare) {
            validateObject(pub, {
                signers: 'object',
                commitments: 'object',
                verifyingShares: 'object',
            }, {}, 'pub');
            validateSigners(pub.signers, 'pub.signers');
            aarray(pub.commitments, 'pub.commitments');
            aarray(commitmentList, 'commitmentList');
            abytes(msg, undefined, 'msg');
            parseIdentifier(identifier);
            abytes(sigShare, Fn.BYTES, 'sigShare');
            if (adjustPublic)
                pub = adjustPublic(pub);
            const comm = commitmentList.find((i) => i.identifier === identifier);
            if (!comm)
                throw new Error('cannot find identifier commitment');
            const PK = parsePoint(pub.verifyingShares[identifier]);
            const hidingNonceCommitment = parsePoint(comm.hiding);
            const bindingNonceCommitment = parsePoint(comm.binding);
            const { lambda, challenge, bindingFactor, groupCommitment } = prepareShare(pub.commitments[0], commitmentList, msg, identifier);
            // Signature shares, commitments and verifying shares are public: vartime is safe here.
            // hC + bC * bF
            let commShare = hidingNonceCommitment.add(bindingNonceCommitment.multiplyUnsafe(bindingFactor));
            if (adjustGroupCommitmentShare)
                commShare = adjustGroupCommitmentShare(groupCommitment, commShare);
            const l = Point.BASE.multiplyUnsafe(Fn.fromBytes(sigShare)); // sigShare*G
            // commShare + PK * (challenge * lambda)
            const r = commShare.add(PK.multiplyUnsafe(Fn.mul(challenge, lambda)));
            return l.equals(r);
        },
        // Aggregate multiple signature shares into groupSignature
        aggregate(pub, commitmentList, msg, sigShares) {
            validateObject(pub, {
                signers: 'object',
                commitments: 'object',
                verifyingShares: 'object',
            }, {}, 'pub');
            validateSigners(pub.signers, 'pub.signers');
            aarray(pub.commitments, 'pub.commitments');
            aarray(commitmentList, 'commitmentList');
            abytes(msg, undefined, 'msg');
            validateObject(sigShares, {}, {}, 'sigShares');
            // verifyShare() applies adjustPublic too, so keep the original package for attribution.
            const rawPub = pub;
            if (adjustPublic)
                pub = adjustPublic(pub);
            try {
                validateCommitmentsNum(pub.signers, commitmentList.length);
            }
            catch {
                throw new AggErr('aggregation failed', []);
            }
            const ids = commitmentList.map((i) => i.identifier);
            const seen = new Set();
            for (const id of ids) {
                // `sigShares` is identifier-keyed, so duplicate commitments would reuse one share twice.
                if (seen.has(id))
                    throw new AggErr('aggregation failed', []);
                seen.add(id);
            }
            if (ids.length !== Object.keys(sigShares).length)
                throw new AggErr('aggregation failed', []);
            for (const id of ids) {
                if (!(id in sigShares) || !(id in pub.verifyingShares))
                    throw new AggErr('aggregation failed', []);
            }
            const GPK = parsePoint(pub.commitments[0]);
            const { groupCommitment } = getGroupCommitment(GPK, commitmentList, msg);
            let z = Fn.ZERO;
            // RFC 9591 Section 5.3 aggregates by summing the validated signature shares.
            for (const id of ids)
                z = Fn.add(z, Fn.fromBytes(sigShares[id])); // z += zi
            if (!Basic.verify(msg, groupCommitment, z, GPK)) {
                const cheaters = [];
                for (const id of ids) {
                    if (!this.verifyShare(rawPub, commitmentList, msg, id, sigShares[id]))
                        cheaters.push(id);
                }
                throw new AggErr('aggregation failed', cheaters);
            }
            return Signature.encode(groupCommitment, z);
        },
        // Basic sign/verify using single key
        sign(msg, secretKey) {
            let sk = Fn.fromBytes(secretKey);
            // Taproot single-key signing needs the same scalar normalization as threshold keys.
            if (adjustScalar)
                sk = adjustScalar(sk);
            const [R, z] = Basic.sign(msg, sk);
            return Signature.encode(R, z);
        },
        verify(sig, msg, publicKey) {
            const PK = parsePublicKey
                ? validatePublicPoint(parsePublicKey(publicKey))
                : parsePoint(publicKey);
            const { R, z } = Signature.decode(sig);
            return Basic.verify(msg, R, z, PK);
        },
        // Combine multiple secret shares to restore secret
        combineSecret(shares, signers) {
            aarray(shares, 'shares');
            validateSigners(signers);
            if (shares.length < signers.min || shares.length > signers.max)
                throw new Error('wrong secret shares array');
            const points = [];
            const seen = {};
            // Interpolate over the full provided share set and reject duplicate identifiers.
            for (const s of shares) {
                const idNum = parseIdentifier(s.identifier);
                const id = serializeIdentifier(idNum);
                if (seen[id])
                    throw new Error('duplicated id=' + id);
                seen[id] = true;
                points.push([idNum, Fn.fromBytes(s.signingShare)]);
            }
            const xCoords = points.map(([x]) => x);
            let res = Fn.ZERO;
            for (const [x, y] of points)
                res = Fn.add(res, Fn.mul(y, deriveInterpolatingValue(xCoords, x)));
            return Fn.toBytes(res);
        },
        // Utils
        utils: Object.freeze({
            Fn, // NOTE: we re-export it here because it may be different from Point.Fn (ed448 is fun!)
            // Test RNG overrides still go through noble's non-zero scalar derivation; this is not a raw
            // "bytes become scalar" escape hatch.
            randomScalar: (rng = randomBytes) => Fn.toBytes(genPointScalarPair(rng).scalar),
            generateSecretPolynomial: (signers, secret, coeffs, rng) => {
                const res = generateSecretPolynomial(signers, secret, coeffs, rng);
                return { ...res, commitment: res.commitment.map(serializePoint) };
            },
        }),
    };
    return Object.freeze(frost);
}
