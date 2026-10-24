/**
 * ML-KEM: Module Lattice-based Key Encapsulation Mechanism from
 * [FIPS-203](https://csrc.nist.gov/pubs/fips/203/ipd). A.k.a. CRYSTALS-Kyber.
 *
 * Key encapsulation is similar to DH / ECDH (think X25519), with important differences:
 * * Unlike in ECDH, we can't verify if it was "Bob" who've sent the shared secret
 * * Unlike ECDH, it is probabalistic and relies on quality of randomness (CSPRNG).
 * * Decapsulation never throws an error, even when shared secret was
 *   encrypted by a different public key. It will just return a different shared secret.
 *
 * There are some concerns with regards to security: see
 * [djb blog](https://blog.cr.yp.to/20231003-countcorrectly.html) and
 * [mailing list](https://groups.google.com/a/list.nist.gov/g/pqc-forum/c/W2VOzy0wz_E).
 *
 * Has similar internals to ML-DSA, but their keys and params are different.
 *
 * Check out [official site](https://www.pq-crystals.org/kyber/resources.shtml),
 * [repo](https://github.com/pq-crystals/kyber),
 * [spec](https://datatracker.ietf.org/doc/draft-cfrg-schwabe-kyber/).
 * @module
 */
/*! noble-post-quantum - MIT License (c) 2024 Paul Miller (paulmillr.com) */
import { sha3_256, sha3_512, shake256 } from '@noble/hashes/sha3.js';
import { swap32IfBE, u32 } from '@noble/hashes/utils.js';
import { genCrystals, XOF128 } from "./_crystals.js";
import { abytes, cleanBytes, copyBytes, equalBytes, getMask, randomBytes, splitCoder, vecCoder, } from "./utils.js";
/** Key encapsulation mechanism interface */
const N = 256; // Kyber (not FIPS-203) supports different lengths, but all std modes were using 256
const Q = 3329; // 13*(2**8)+1, modulo prime
const F = 3303; // 3303 ≡ 128**(−1) mod q (FIPS-203)
const ROOT_OF_UNITY = 17; // ζ = 17 ∈ Zq is a primitive 256-th root of unity modulo Q. ζ**128 ≡−1
// treeshake: keep genCrystals behind the object so PARAMS-only bundles can drop it entirely.
// Shared CRYSTALS helper in the ML-KEM branch: Kyber mode, 7-bit bit-reversal,
// and Uint16Array polys because current coefficients stay reduced modulo q.
const crystals = /* @__PURE__ */ genCrystals({
    N,
    Q,
    F,
    ROOT_OF_UNITY,
    newPoly: (n) => new Uint16Array(n),
    brvBits: 7,
    isKyber: true,
});
/** Internal params of ML-KEM versions */
// prettier-ignore
/** Built-in ML-KEM parameter presets keyed by the public export names
 * `ml_kem512` / `ml_kem768` / `ml_kem1024`.
 * `RBGstrength` is Table 2's required randomness-source strength in bits,
 * not a generic security label.
 */
export const PARAMS = /* @__PURE__ */ (() => Object.freeze({
    512: Object.freeze({ N, Q, K: 2, ETA1: 3, ETA2: 2, du: 10, dv: 4, RBGstrength: 128 }),
    768: Object.freeze({ N, Q, K: 3, ETA1: 2, ETA2: 2, du: 10, dv: 4, RBGstrength: 192 }),
    1024: Object.freeze({ N, Q, K: 4, ETA1: 2, ETA2: 2, du: 11, dv: 5, RBGstrength: 256 }),
}))();
// FIPS-203: compress/decompress
const compress = (d) => {
    // d=12 is the ByteEncode12/ByteDecode12 path, not lossy compression.
    // ByteDecode12 interprets each 12-bit word modulo q; without that reduction the public-key
    // modulus check in encapsulate() becomes a no-op for malformed coefficients like 4095.
    if (d >= 12)
        return { encode: (i) => i, decode: (i) => (i >= Q ? i - Q : i) };
    // Comments map to python implementation in RFC (draft-cfrg-schwabe-kyber)
    // const round = (i: number) => Math.floor(i + 0.5) | 0;
    const a = 2 ** (d - 1);
    return {
        // This only matches standalone Compress_d after bitsCoder masks the result into Z_(2^d).
        encode: (i) => ((i << d) + Q / 2) / Q,
        // const decompress = (i: number) => round((Q / 2 ** d) * i);
        decode: (i) => (i * Q + a) >>> d,
    };
};
// Raw ByteEncode_d / ByteDecode_d from FIPS 203 operate on d-bit words directly.
// That differs from `polyCoder(d)` for d<12, where noble folds packing together with the lossy
// ciphertext compression step used by u/v. Tests that exercise the spec's raw packing surface need
// this exact non-lossy variant instead.
const byteCoder = (d) => crystals.bitsCoder(d, d === 12
    ? { encode: (i) => i, decode: (i) => (i >= Q ? i - Q : i) }
    : { encode: (i) => i, decode: (i) => i });
// NOTE: we merge encoding and compress because it is faster, also both require same d param
// d=12 is the ByteEncode12/ByteDecode12 path rather than compression, and caller-side
// public-key modulus checks route through this helper's decode/encode roundtrip.
// Converts between bytes and d-bits compressed representation.
// Kinda like convertRadix2 from @scure/base.
// decode(encode(t)) == t, but there is loss of information on encode(decode(t))
const polyCoder = (d) => (d === 12 ? byteCoder(12) : crystals.bitsCoder(d, compress(d)));
// Coefficients always stay reduced in [0, Q) here (samplers, NTT and coders all reduce),
// so one conditional correction replaces the generic mod().
function polyAdd(a_, b_) {
    const a = a_;
    const b = b_;
    // Mutates `a` in place; callers must pass two N=256 polynomials.
    for (let i = 0; i < N; i++) {
        const r = a[i] + b[i]; // a += b
        a[i] = r >= Q ? r - Q : r;
    }
}
function polySub(a_, b_) {
    const a = a_;
    const b = b_;
    // Mutates `a` in place; callers must pass two N=256 polynomials.
    for (let i = 0; i < N; i++) {
        const r = a[i] - b[i]; // a -= b
        a[i] = r < 0 ? r + Q : r;
    }
}
// FIPS-203: Computes the product of two degree-one polynomials with respect to a quadratic modulus
function BaseCaseMultiply(a0, a1, b0, b1, zeta) {
    // `zeta` here is Algorithm 11's γ = ζ^(2BitRev_7(i)+1).
    // Reduce a1*b1 before multiplying by zeta: a1*b1*zeta would reach ~2^35, forcing JS engines
    // into slow float fmod; with the extra reduction every intermediate fits int32.
    const c0 = crystals.mod(crystals.mod(a1 * b1) * zeta + a0 * b0);
    const c1 = crystals.mod(a0 * b1 + a1 * b0);
    return { c0, c1 };
}
// FIPS-203: Computes the product (in the ring Tq) of two NTT representations.
// Works in place on `f`; `g` is read-only and both inputs must already be in NTT form.
function MultiplyNTTs(f_, g_) {
    const f = f_;
    const g = g_;
    for (let i = 0; i < N / 2; i++) {
        let z = crystals.nttZetas[64 + (i >> 1)];
        if (i & 1)
            z = -z;
        const { c0, c1 } = BaseCaseMultiply(f[2 * i + 0], f[2 * i + 1], g[2 * i + 0], g[2 * i + 1], z);
        f[2 * i + 0] = c0;
        f[2 * i + 1] = c1;
    }
    return f;
}
// Return poly in NTT representation
function SampleNTT(xof_) {
    const xof = xof_;
    // The reader must already bind the Algorithm 7 seed||j||i bytes
    // and return block lengths divisible by 3.
    const r = new Uint16Array(N);
    for (let j = 0; j < N;) {
        const b = xof();
        if (b.length % 3)
            throw new Error('SampleNTT: unaligned block');
        for (let i = 0; j < N && i + 3 <= b.length; i += 3) {
            const d1 = ((b[i + 0] >> 0) | (b[i + 1] << 8)) & 0xfff;
            const d2 = ((b[i + 1] >> 4) | (b[i + 2] << 4)) & 0xfff;
            if (d1 < Q)
                r[j++] = d1;
            if (j < N && d2 < Q)
                r[j++] = d2;
        }
    }
    return r;
}
// Sampling from the centered binomial distribution
// Returns poly with small coefficients (noise/errors) stored modulo q in ordinary coefficient form.
// Current callers only use Table 2 eta values {2,3} and PRF outputs of exactly 64*eta bytes.
const sampleCBDBytes = (buf, eta) => {
    const r = new Uint16Array(N);
    // CBD consumes the PRF bitstream in little-endian byte order; normalize the word view on BE,
    // then swap it back so callers still observe `buf` as read-only.
    const b32 = u32(buf);
    swap32IfBE(b32);
    let len = 0;
    for (let i = 0, p = 0, bb = 0, t0 = 0; i < b32.length; i++) {
        let b = b32[i];
        for (let j = 0; j < 32; j++) {
            bb += b & 1;
            b >>= 1;
            len += 1;
            if (len === eta) {
                t0 = bb;
                bb = 0;
            }
            else if (len === 2 * eta) {
                r[p++] = crystals.mod(t0 - bb);
                bb = 0;
                len = 0;
            }
        }
    }
    swap32IfBE(b32);
    if (len)
        throw new Error(`sampleCBD: leftover bits: ${len}`);
    return r;
};
function sampleCBD(PRF_, seed, nonce, eta) {
    const PRF = PRF_;
    return sampleCBDBytes(PRF((eta * N) / 4, seed, nonce), eta);
}
// K-PKE
// Internal ML-KEM subroutine only: exact 32-byte `seed` / `msg` inputs
// come from Algorithms 13-15, and the helper mutates decoded temporary
// polynomials in place while leaving caller byte arrays unchanged.
const genKPKE = (opts_) => {
    const opts = opts_;
    const { K, PRF, XOF, HASH512, ETA1, ETA2, du, dv } = opts;
    const poly1 = polyCoder(1);
    const polyV = polyCoder(dv);
    const polyU = polyCoder(du);
    const publicCoder = splitCoder('publicKey', vecCoder(polyCoder(12), K), 32);
    const secretCoder = vecCoder(polyCoder(12), K);
    const cipherCoder = splitCoder('ciphertext', vecCoder(polyU, K), polyV);
    const seedCoder = splitCoder('seed', 32, 32);
    // Algorithm 14 (K-PKE.Encrypt) core, after ek parsing. `tHat` and every poly returned by
    // `getA(i, j)` are treated as disposable scratch: they are mutated in place and wiped/dropped,
    // so callers holding cached copies must pass fresh copies.
    const encryptCore = (tHat, getA, msg, seed) => {
        const rHat = [];
        for (let i = 0; i < K; i++)
            rHat.push(crystals.NTT.encode(sampleCBD(PRF, seed, i, ETA1)));
        const tmp2 = new Uint16Array(N);
        const u = [];
        for (let i = 0; i < K; i++) {
            const e1 = sampleCBD(PRF, seed, K + i, ETA2);
            const tmp = new Uint16Array(N);
            for (let j = 0; j < K; j++) {
                const aij = getA(i, j); // A[j][i], inplace transpose access
                polyAdd(tmp, MultiplyNTTs(aij, rHat[j])); // t += aij * rHat[j]
            }
            polyAdd(e1, crystals.NTT.decode(tmp)); // e1 += tmp
            u.push(e1);
            polyAdd(tmp2, MultiplyNTTs(tHat[i], rHat[i])); // t2 += tHat[i] * rHat[i]
            cleanBytes(tmp);
        }
        const e2 = sampleCBD(PRF, seed, 2 * K, ETA2);
        polyAdd(e2, crystals.NTT.decode(tmp2)); // e2 += tmp2
        const v = poly1.decode(msg); // encode plaintext m into polynomial v
        polyAdd(v, e2); // v += e2
        cleanBytes(tHat, rHat, tmp2, e2);
        return cipherCoder.encode([u, v]);
    };
    return {
        secretCoder,
        lengths: {
            secretKey: secretCoder.bytesLen,
            publicKey: publicCoder.bytesLen,
            cipherText: cipherCoder.bytesLen,
        },
        keygen: (seed) => {
            abytes(seed, 32, 'seed');
            const seedDst = new Uint8Array(33);
            seedDst.set(seed);
            // FIPS 203 Algorithm 13 appends the parameter-set byte `k`
            // before `G(d || k)`, so expanding the same 32-byte seed
            // under a different ML-KEM parameter set yields unrelated keys.
            seedDst[32] = K;
            const seedHash = HASH512(seedDst);
            const [rho, sigma] = seedCoder.decode(seedHash);
            const sHat = [];
            const tHat = [];
            for (let i = 0; i < K; i++)
                sHat.push(crystals.NTT.encode(sampleCBD(PRF, sigma, i, ETA1)));
            const x = XOF(rho);
            for (let i = 0; i < K; i++) {
                const e = crystals.NTT.encode(sampleCBD(PRF, sigma, K + i, ETA1));
                for (let j = 0; j < K; j++) {
                    const aji = SampleNTT(x.get(j, i)); // A[i][j], inplace
                    polyAdd(e, MultiplyNTTs(aji, sHat[j]));
                }
                tHat.push(e); // t ← A ◦ s + e
            }
            x.clean();
            const res = {
                publicKey: publicCoder.encode([tHat, rho]),
                secretKey: secretCoder.encode(sHat),
            };
            cleanBytes(rho, sigma, sHat, tHat, seedDst, seedHash);
            return res;
        },
        encrypt: (publicKey, msg, seed) => {
            const [tHat, rho] = publicCoder.decode(publicKey);
            const x = XOF(rho);
            const res = encryptCore(tHat, (i, j) => SampleNTT(x.get(i, j)), msg, seed);
            x.clean();
            return res;
        },
        // Expands the full Â matrix (public data derived from rho) once, so repeated encryptions
        // against the same ek skip the K² SampleNTT XOF expansions. Cached polys are copied per
        // call because encryptCore mutates its inputs in place.
        prepare: (publicKey) => {
            const [tHat, rho] = publicCoder.decode(publicKey);
            const x = XOF(rho);
            const A = [];
            for (let i = 0; i < K; i++)
                for (let j = 0; j < K; j++)
                    A.push(SampleNTT(x.get(i, j)));
            x.clean();
            return {
                encrypt: (msg, seed) => encryptCore(tHat.map((p) => p.slice()), (i, j) => A[i * K + j].slice(), msg, seed),
                clean: () => cleanBytes(tHat, A),
            };
        },
        decrypt: (cipherText, privateKey) => {
            const [u, v] = cipherCoder.decode(cipherText);
            const sk = secretCoder.decode(privateKey); // s  ← ByteDecode_12(dkPKE)
            const tmp = new Uint16Array(N);
            // tmp += sk[i] * u[i]
            for (let i = 0; i < K; i++)
                polyAdd(tmp, MultiplyNTTs(sk[i], crystals.NTT.encode(u[i])));
            polySub(v, crystals.NTT.decode(tmp)); // w = v' - tmp
            // `v` now holds w, from which the plaintext is just a 1-bit threshold away, so wipe it too.
            // encode() allocates its own buffer, so the returned bytes do not alias `v`.
            const res = poly1.encode(v);
            cleanBytes(tmp, sk, u, v);
            return res;
        },
    };
};
/**
 * Public ML-KEM wrapper over the internal K-PKE subroutine.
 * `keygen(seed)` and `encapsulate(publicKey, msg)` are deterministic/test-oriented hooks that map
 * more directly to Algorithms 16-17 than to the pure no-input / random-internal Algorithms 19-20.
 * `encapsulate`'s optional `msg` is the 32-byte message randomness `m` of Algorithm 17, the
 * pre-image the shared secret is derived from, NOT a plaintext to encrypt: ML-KEM is a key
 * encapsulation mechanism, not a cipher. Omit it to draw fresh randomness; pass it only to
 * reproduce a known-answer vector, and only as 32 uniformly random bytes, since a low-entropy or
 * reused value makes the shared secret predictable. The same holds for `keygen`'s optional `seed`.
 * decapsulate() tries to follow the Algorithms 18/21 implicit-reject structure as closely as
 * practical here by re-encrypting, comparing ciphertexts, returning `Khat` on match or `Kbar` on
 * mismatch, and zeroizing the non-returned shared-secret candidate; JS/JIT still provides no
 * constant-time guarantees for that path.
 */
function createKyber(opts) {
    const rawOpts = opts;
    const KPKE = genKPKE(rawOpts);
    const { HASH256, HASH512, KDF } = rawOpts;
    const { secretCoder: KPKESecretCoder, lengths } = KPKE;
    const secretCoder = splitCoder('secretKey', lengths.secretKey, lengths.publicKey, 32, 32);
    const msgLen = 32;
    const seedLen = 64;
    // FIPS-203 includes additional verification check for modulus
    const validateModulus = (publicKey, fn) => {
        const eke = publicKey.subarray(0, 384 * rawOpts.K);
        // Copy because of inplace encoding
        const ek = KPKESecretCoder.encode(KPKESecretCoder.decode(copyBytes(eke)));
        // (Modulus check.) Perform the computation ek ← ByteEncode12(ByteDecode12(eke)).
        // If ek = ̸ eke, the input is invalid. (See Section 4.2.1.)
        const ok = equalBytes(ek, eke);
        cleanBytes(ek);
        if (!ok)
            throw new Error(`ML-KEM.${fn}: wrong publicKey modulus`);
    };
    const kemLengths = Object.freeze({
        ...lengths,
        seed: 64,
        msg: msgLen,
        msgRand: msgLen,
        secretKey: secretCoder.bytesLen,
    });
    return Object.freeze({
        info: Object.freeze({ type: 'ml-kem' }),
        lengths: kemLengths,
        keygen: (seed) => {
            // A generated seed carries z (the implicit-rejection secret) and must be wiped once the
            // secret key holds a copy, matching ml-dsa / slh-dsa / falcon keygen. A caller-supplied
            // seed is the caller's to manage (and the immutability test requires it stay untouched).
            const ownSeed = seed === undefined;
            const s = ownSeed ? randomBytes(seedLen) : seed;
            let sk;
            let publicKeyHash;
            try {
                abytes(s, seedLen, 'seed');
                const keys = KPKE.keygen(s.subarray(0, 32));
                const publicKey = keys.publicKey;
                sk = keys.secretKey;
                publicKeyHash = HASH256(publicKey);
                // (dkPKE||ek||H(ek)||z)
                const secretKey = secretCoder.encode([sk, publicKey, publicKeyHash, s.subarray(32)]);
                return {
                    publicKey: publicKey,
                    secretKey: secretKey,
                };
            }
            finally {
                if (sk !== undefined)
                    cleanBytes(sk);
                if (publicKeyHash !== undefined)
                    cleanBytes(publicKeyHash);
                if (ownSeed)
                    cleanBytes(s);
            }
        },
        getPublicKey: (secretKey) => {
            const [_sk, publicKey, _publicKeyHash, _z] = secretCoder.decode(secretKey);
            return Uint8Array.from(publicKey);
        },
        encapsulate: (publicKey, msg) => {
            // A generated message is the preimage of the shared secret (K = G(m || H(ek))[0:32]) and
            // must be wiped. A caller-supplied message is the deterministic-randomness hook and the
            // caller's to manage (the immutability test requires it stay untouched).
            const ownMsg = msg === undefined;
            const m = ownMsg ? randomBytes(msgLen) : msg;
            let kr;
            try {
                abytes(publicKey, lengths.publicKey, 'publicKey');
                abytes(m, msgLen, 'message');
                validateModulus(publicKey, 'encapsulate');
                // derive randomness
                kr = HASH512.create().update(m).update(HASH256(publicKey)).digest();
                const cipherText = KPKE.encrypt(publicKey, m, kr.subarray(32, 64));
                return {
                    cipherText: cipherText,
                    sharedSecret: kr.subarray(0, 32),
                };
            }
            finally {
                if (kr !== undefined)
                    cleanBytes(kr.subarray(32));
                if (ownMsg)
                    cleanBytes(m);
            }
        },
        decapsulate: (cipherText, secretKey) => {
            abytes(secretKey, secretCoder.bytesLen, 'secretKey'); // 768*k + 96
            abytes(cipherText, lengths.cipherText, 'cipherText'); // 32(du*k + dv)
            // test ← H(dk[384𝑘 ∶ 768𝑘 + 32])) .
            const k768 = secretCoder.bytesLen - 96;
            const start = k768 + 32;
            const test = HASH256(secretKey.subarray(k768 / 2, start));
            // If test ≠ dk[768𝑘 + 32 ∶ 768𝑘 + 64], then input checking has failed.
            if (!equalBytes(test, secretKey.subarray(start, start + 32)))
                throw new Error('invalid secretKey: hash check failed');
            const [sk, publicKey, publicKeyHash, z] = secretCoder.decode(secretKey);
            const msg = KPKE.decrypt(cipherText, sk);
            // derive randomness, Khat, rHat = G(mHat || h)
            const kr = HASH512.create().update(msg).update(publicKeyHash).digest();
            const Khat = kr.subarray(0, 32);
            // re-encrypt using the derived randomness
            const cipherText2 = KPKE.encrypt(publicKey, msg, kr.subarray(32, 64));
            // if ciphertexts do not match, “implicitly reject”
            const isValid = equalBytes(cipherText, cipherText2);
            const Kbar = KDF.create({ dkLen: 32 }).update(z).update(cipherText).digest();
            // kr[32:64] is the derived K-PKE encryption randomness: wipe it like encapsulate() does.
            cleanBytes(msg, cipherText2, kr.subarray(32), !isValid ? Khat : Kbar);
            return (isValid ? Khat : Kbar);
        },
        /**
         * Experimental prototype: pre-expand a public key so repeated encapsulate/decapsulate
         * against the same key skip re-validation, H(ek), t̂ decoding and the K² SampleNTT
         * XOF expansions of Â. Only public data is cached; see {@link KEMPrepared}.
         */
        prepare: (publicKey) => {
            abytes(publicKey, lengths.publicKey, 'publicKey');
            validateModulus(publicKey, 'prepare');
            const ek = copyBytes(publicKey); // detach from the caller before caching
            const publicKeyHash = HASH256(ek);
            const cached = KPKE.prepare(ek);
            return Object.freeze({
                publicKey: ek,
                encapsulate: (msg) => {
                    // As in the non-prepared encapsulate: a generated message is the shared-secret
                    // preimage and is wiped; a caller-supplied one is left untouched.
                    const ownMsg = msg === undefined;
                    const m = ownMsg ? randomBytes(msgLen) : msg;
                    let kr;
                    try {
                        abytes(m, msgLen, 'message');
                        kr = HASH512.create().update(m).update(publicKeyHash).digest();
                        const cipherText = cached.encrypt(m, kr.subarray(32, 64));
                        return {
                            cipherText: cipherText,
                            sharedSecret: kr.subarray(0, 32),
                        };
                    }
                    finally {
                        if (kr !== undefined)
                            cleanBytes(kr.subarray(32));
                        if (ownMsg)
                            cleanBytes(m);
                    }
                },
                decapsulate: (cipherText, secretKey) => {
                    abytes(secretKey, secretCoder.bytesLen, 'secretKey');
                    abytes(cipherText, lengths.cipherText, 'cipherText');
                    const [sk, ekEmbedded, storedHash, z] = secretCoder.decode(secretKey);
                    // Under KEMPrepared's read-only publicKey contract, bind dk to the prepared key.
                    // Together with publicKeyHash = H(ek) computed in prepare(), this is equivalent to (and
                    // stronger than) FIPS 203 §7.3's `H(dk[384k : 768k+32]) == dk[768k+32 : 768k+64]`.
                    if (!equalBytes(ekEmbedded, ek) || !equalBytes(storedHash, publicKeyHash))
                        throw new Error('ML-KEM.decapsulate: secretKey does not match prepared publicKey');
                    const msg = KPKE.decrypt(cipherText, sk);
                    // derive randomness, Khat, rHat = G(mHat || h)
                    const kr = HASH512.create().update(msg).update(publicKeyHash).digest();
                    const Khat = kr.subarray(0, 32);
                    // re-encrypt using the derived randomness and cached Â/t̂
                    const cipherText2 = cached.encrypt(msg, kr.subarray(32, 64));
                    // if ciphertexts do not match, “implicitly reject”
                    const isValid = equalBytes(cipherText, cipherText2);
                    const Kbar = KDF.create({ dkLen: 32 }).update(z).update(cipherText).digest();
                    cleanBytes(msg, cipherText2, kr.subarray(32), !isValid ? Khat : Kbar);
                    return (isValid ? Khat : Kbar);
                },
                clean: cached.clean,
            });
        },
    });
}
// FIPS 203's PRF_eta binding: current callers use only 32-byte keys, one-byte nonces,
// and dkLen values {128, 192}; out-of-range nonce numbers still wrap modulo 256 here.
function shakePRF(dkLen, key, nonce) {
    return shake256
        .create({ dkLen })
        .update(key)
        .update(new Uint8Array([nonce]))
        .digest();
}
// Fixed ML-KEM hash/XOF bindings. `KDF` here is the spec's fixed 32-byte `J` call,
// and swapping any field changes the scheme rather than tuning an internal dependency.
const opts = /* @__PURE__ */ (() => ({
    HASH256: sha3_256,
    HASH512: sha3_512,
    KDF: shake256,
    XOF: XOF128,
    PRF: shakePRF,
}))();
// Parameter-set instantiation step for the spec's "ML-KEM-x" names; current correctness relies
// on the internal PARAMS rows rather than local validation of arbitrary KEMParam objects.
const mk = (params) => createKyber({
    ...opts,
    ...params,
});
/**
 * ML-KEM-512: Table 2 row `k=2, η1=3, η2=2, du=10, dv=4`; Table 3 sizes `800/1632/768/32`.
 * The ASD lifecycle note here is external policy guidance, not a FIPS 203 requirement.
 * @example
 * Generate deterministic ML-KEM-512 keys, encapsulate a shared secret, and decapsulate it.
 * ```ts
 * import { ml_kem512 } from '@noble/post-quantum/ml-kem.js';
 * const seed = new Uint8Array(ml_kem512.lengths.seed!);
 * const { secretKey, publicKey } = ml_kem512.keygen(seed);
 * const msg = new Uint8Array(ml_kem512.lengths.msgRand!);
 * const { cipherText, sharedSecret } = ml_kem512.encapsulate(publicKey, msg);
 * const recovered = ml_kem512.decapsulate(cipherText, secretKey);
 * const publicKey2 = ml_kem512.getPublicKey(secretKey);
 * ```
 */
export const ml_kem512 = /* @__PURE__ */ (() => mk(PARAMS[512]))();
/**
 * ML-KEM-768: Table 2 row `k=3, η1=2, η2=2, du=10, dv=4`; Table 3 sizes `1184/2400/1088/32`.
 * The ASD lifecycle note here is external policy guidance, not a FIPS 203 requirement.
 */
export const ml_kem768 = /* @__PURE__ */ (() => mk(PARAMS[768]))();
/**
 * ML-KEM-1024: Table 2 row `k=4, η1=2, η2=2, du=11, dv=5`; Table 3 sizes `1568/3168/1568/32`.
 * The ASD lifecycle note here is external policy guidance, not a FIPS 203 requirement.
 */
export const ml_kem1024 = /* @__PURE__ */ (() => mk(PARAMS[1024]))();
// NOTE: for tests only, don't use. This keeps the exact internal ML-KEM math surfaces available
// without re-implementing them in separate test code.
export const __tests = /* @__PURE__ */ (() => Object.freeze({
    Compress_d: (x, d) => {
        if (d < 1 || d > 11)
            throw new Error(`Compress_d: expected d in [1..11], got ${d}`);
        return compress(d).encode(x) & getMask(d);
    },
    Decompress_d: (y, d) => {
        if (d < 1 || d > 11)
            throw new Error(`Decompress_d: expected d in [1..11], got ${d}`);
        return compress(d).decode(y);
    },
    ByteEncode_d: (F, d) => {
        if (d < 1 || d > 12)
            throw new Error(`ByteEncode_d: expected d in [1..12], got ${d}`);
        return byteCoder(d).encode(F);
    },
    ByteDecode_d: (B, d) => {
        if (d < 1 || d > 12)
            throw new Error(`ByteDecode_d: expected d in [1..12], got ${d}`);
        return byteCoder(d).decode(B);
    },
    NTT: (f) => crystals.NTT.encode(Uint16Array.from(f)),
    NTT_inv: (fHat) => crystals.NTT.decode(Uint16Array.from(fHat)),
    MultiplyNTTs: (fHat, gHat) => MultiplyNTTs(Uint16Array.from(fHat), Uint16Array.from(gHat)),
    SamplePolyCBD: (B, eta) => {
        abytes(B, 64 * eta, 'B');
        return sampleCBDBytes(B, eta);
    },
    SampleNTT: (B) => {
        abytes(B, 34, 'B');
        const xof = XOF128(B.subarray(0, 32));
        try {
            return SampleNTT(xof.get(B[32], B[33]));
        }
        finally {
            xof.clean();
        }
    },
}))();
