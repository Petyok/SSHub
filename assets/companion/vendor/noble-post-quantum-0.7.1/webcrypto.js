/**
 * Friendly async wrappers over ML-KEM and ML-KEM-768 + X25519 from built-in WebCrypto.
 * Private keys use the same raw seed accepted by the synchronous implementations' `keygen(seed)`;
 * they are not expanded decapsulation keys.
 *
 * # WebCrypto quirks
 *
 * - The algorithms are experimental: a runtime can expose `encapsulateBits` and friends while
 *   implementing none of them, so support is probed with a full round-trip in `isSupported()`.
 * - `MLKEM768-X25519` accepts `raw-seed` on import, but has no `raw-seed` / `raw-public` export.
 *   Its key bytes are read out of the JWK `priv` / `pub` members instead.
 * - base64url is hand-rolled: scure-base's `base64urlnopad` would do, but this module must not add
 *   dependencies, and importing the synchronous implementations for four byte lengths would pull
 *   the whole lattice math into a WebCrypto-only entrypoint.
 * @module
 */
/*! noble-post-quantum - MIT License (c) 2024 Paul Miller (paulmillr.com) */
import { abytes, cleanBytes, copyBytes, equalBytes } from "./utils.js";
function _subtle() {
    const cr = typeof globalThis === 'object' ? globalThis.crypto : null;
    const sb = cr?.subtle;
    if (typeof sb === 'object' && sb != null)
        return sb;
    throw new Error('crypto.subtle must be defined');
}
const PRIVATE_USAGES = ['decapsulateBits'];
const PUBLIC_USAGES = ['encapsulateBits'];
const ALL_USAGES = ['encapsulateBits', 'decapsulateBits'];
const PROBED_METHODS = ['encapsulateBits', 'decapsulateBits', 'getPublicKey'];
const SHARED_SECRET_LENGTH = 32;
const arrayBufferByteLength = Object.getOwnPropertyDescriptor(ArrayBuffer.prototype, 'byteLength')?.get;
// Typed-array constructors also accept numbers and array-like objects, which would turn malformed
// provider results into newly allocated all-zero buffers. Use the intrinsic getter as a cross-realm
// ArrayBuffer brand check before constructing a view.
function providerBytes(value, title) {
    try {
        if (arrayBufferByteLength === undefined)
            throw new TypeError('missing ArrayBuffer getter');
        arrayBufferByteLength.call(value);
    }
    catch {
        throw new TypeError(`WebCrypto "${title}" expected ArrayBuffer`);
    }
    return new Uint8Array(value);
}
function base64url(bytes) {
    let binary = '';
    for (let i = 0; i < bytes.length; i++)
        binary += String.fromCharCode(bytes[i]);
    return globalThis.btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}
function debase64url(value, title) {
    if (typeof value !== 'string')
        throw new Error(`WebCrypto JWK is missing ${title}`);
    const base64 = value.replace(/-/g, '+').replace(/_/g, '/');
    const padding = base64.length % 4;
    const binary = globalThis.atob(base64 + (padding ? '='.repeat(4 - padding) : ''));
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++)
        bytes[i] = binary.charCodeAt(i);
    return bytes;
}
/** Keys serialized through the `raw-seed` / `raw-public` WebCrypto formats. */
const rawCodec = {
    importPublic: (subtle, algorithm, publicKey) => subtle.importKey('raw-public', publicKey, algorithm, false, PUBLIC_USAGES),
    exportPublic: async (subtle, key, length) => abytes(new Uint8Array(await subtle.exportKey('raw-public', key)), length, 'publicKey'),
    exportPrivate: async (subtle, key, length) => abytes(new Uint8Array(await subtle.exportKey('raw-seed', key)), length, 'secretKey'),
};
/** Keys serialized through the JWK `pub` / `priv` members, for algorithms without raw export. */
function jwkCodec(webCryptoName) {
    const exportMember = async (subtle, key, member, length) => {
        const jwk = await subtle.exportKey('jwk', key);
        return abytes(debase64url(jwk?.[member], member), length, member);
    };
    return {
        importPublic: (subtle, algorithm, publicKey) => subtle.importKey('jwk', {
            kty: 'AKP',
            alg: webCryptoName,
            key_ops: PUBLIC_USAGES,
            ext: true,
            pub: base64url(publicKey),
        }, algorithm, true, PUBLIC_USAGES),
        exportPublic: (subtle, key, length) => exportMember(subtle, key, 'pub', length),
        exportPrivate: (subtle, key, length) => exportMember(subtle, key, 'priv', length),
    };
}
function createWebCryptoKEM(webCryptoName, lengths, codec) {
    const algorithm = Object.freeze({ name: webCryptoName });
    const frozen = Object.freeze(lengths);
    let supported;
    // Imports a raw seed, wiping the detached copy as soon as WebCrypto has consumed it.
    const importSecret = async (subtle, secretKey) => {
        const secret = copyBytes(abytes(secretKey, frozen.secretKey, 'secretKey'));
        try {
            return await subtle.importKey('raw-seed', secret, algorithm, false, PRIVATE_USAGES);
        }
        finally {
            cleanBytes(secret);
        }
    };
    const getPublicKey = async (secretKey) => {
        const subtle = _subtle();
        const privateKey = await importSecret(subtle, secretKey);
        const publicKey = await subtle.getPublicKey(privateKey, PUBLIC_USAGES);
        return codec.exportPublic(subtle, publicKey, frozen.publicKey);
    };
    const keygen = async (seed) => {
        if (seed !== undefined) {
            const secretKey = copyBytes(abytes(seed, frozen.seed, 'seed'));
            try {
                const publicKey = await getPublicKey(secretKey);
                return { secretKey: secretKey, publicKey };
            }
            catch (error) {
                cleanBytes(secretKey);
                throw error;
            }
        }
        const subtle = _subtle();
        const keys = await subtle.generateKey(algorithm, true, ALL_USAGES);
        const [secretKey, publicKey] = await Promise.all([
            codec.exportPrivate(subtle, keys.privateKey, frozen.secretKey),
            codec.exportPublic(subtle, keys.publicKey, frozen.publicKey),
        ]);
        return { secretKey, publicKey };
    };
    const encapsulate = async (publicKey) => {
        const subtle = _subtle();
        // No copy: the bytes are public and WebCrypto consumes them before the next await.
        const key = await codec.importPublic(subtle, algorithm, abytes(publicKey, frozen.publicKey, 'publicKey'));
        const { ciphertext, sharedKey } = await subtle.encapsulateBits(algorithm, key);
        // Provider outputs cross a trust boundary: some runtimes expose experimental methods with
        // incomplete implementations. Materialize the secret first so every later rejection can wipe
        // it, including a malformed ciphertext result.
        const sharedSecret = providerBytes(sharedKey, 'sharedKey');
        try {
            const cipherText = abytes(providerBytes(ciphertext, 'ciphertext'), frozen.cipherText, 'cipherText');
            abytes(sharedSecret, SHARED_SECRET_LENGTH, 'sharedSecret');
            return { cipherText, sharedSecret: sharedSecret };
        }
        catch (error) {
            cleanBytes(sharedSecret);
            throw error;
        }
    };
    const decapsulate = async (cipherText, secretKey) => {
        // Snapshot the ciphertext: the key import below awaits before WebCrypto reads these bytes.
        const cipher = copyBytes(abytes(cipherText, frozen.cipherText, 'cipherText'));
        const subtle = _subtle();
        const key = await importSecret(subtle, secretKey);
        const sharedSecret = providerBytes(await subtle.decapsulateBits(algorithm, key, cipher), 'sharedKey');
        try {
            return abytes(sharedSecret, SHARED_SECRET_LENGTH, 'sharedSecret');
        }
        catch (error) {
            cleanBytes(sharedSecret);
            throw error;
        }
    };
    return Object.freeze({
        webCryptoName,
        lengths: frozen,
        async isSupported() {
            if (supported !== undefined)
                return supported;
            let secretKey;
            let encapsulatedSecret;
            let decapsulatedSecret;
            try {
                const subtle = _subtle();
                for (const method of PROBED_METHODS)
                    if (typeof subtle[method] !== 'function')
                        return (supported = false);
                const generated = await keygen();
                secretKey = generated.secretKey;
                const { publicKey } = generated;
                const encapsulated = await encapsulate(publicKey);
                encapsulatedSecret = encapsulated.sharedSecret;
                decapsulatedSecret = await decapsulate(encapsulated.cipherText, secretKey);
                const ok = equalBytes(await getPublicKey(secretKey), publicKey) &&
                    equalBytes(encapsulatedSecret, decapsulatedSecret);
                return (supported = ok);
            }
            catch {
                return (supported = false);
            }
            finally {
                // A failed provider can throw at any point after producing secret material. Never let the
                // support probe retain a generated seed or shared-secret output that it received.
                if (secretKey !== undefined)
                    cleanBytes(secretKey);
                if (encapsulatedSecret !== undefined)
                    cleanBytes(encapsulatedSecret);
                if (decapsulatedSecret !== undefined)
                    cleanBytes(decapsulatedSecret);
            }
        },
        keygen,
        getPublicKey,
        encapsulate,
        decapsulate,
    });
}
const mlKem = (name, publicKey, cipherText) => createWebCryptoKEM(name, { seed: 64, secretKey: 64, publicKey, cipherText }, rawCodec);
/** WebCrypto ML-KEM-512 wrapper. */
export const ml_kem512 = /* @__PURE__ */ mlKem('ML-KEM-512', 800, 768);
/** WebCrypto ML-KEM-768 wrapper. */
export const ml_kem768 = /* @__PURE__ */ mlKem('ML-KEM-768', 1184, 1088);
/** WebCrypto ML-KEM-1024 wrapper. */
export const ml_kem1024 = /* @__PURE__ */ mlKem('ML-KEM-1024', 1568, 1568);
/** WebCrypto ML-KEM-768 + X25519 (X-Wing) wrapper. */
export const ml_kem768_x25519 = /* @__PURE__ */ createWebCryptoKEM('MLKEM768-X25519', { seed: 32, secretKey: 32, publicKey: 1216, cipherText: 1120 }, 
/* @__PURE__ */ jwkCodec('MLKEM768-X25519'));
