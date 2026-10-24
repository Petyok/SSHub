#!/usr/bin/env python3
"""Independent oracle for tests/vectors/crypto/vectors.json.

Recomputes every vector from its inputs with libraries that are not the Rust
implementation and not the browser implementation: python-blake3, cryptography
(Ed25519, X25519, ML-KEM-768), PyNaCl/libsodium (XChaCha20-Poly1305), argon2-cffi
(reference Argon2 C code), mnemonic (BIP39), kyber-py (a second ML-KEM), plus a few
lines of Python for the pieces no library has (EFF word encoding, header layouts).

Run: python3 tests/oracles/crypto_oracle.py [vectors.json]
Exit status 0 only if every check passes.
"""
import json
import struct
import sys
from pathlib import Path

import argon2.low_level as a2
import blake3
import nacl.bindings as nb
from cryptography.hazmat.primitives.asymmetric import ed25519, mlkem, x25519
from cryptography.hazmat.primitives.ciphers import Cipher, algorithms
from kyber_py.ml_kem import ML_KEM_768
from mnemonic import Mnemonic

ROOT = Path(__file__).resolve().parents[2]
L = 2**252 + 27742317777372353535851937790883648493
FAILS = []
COUNT = 0


def check(name, ok, detail=""):
    global COUNT
    COUNT += 1
    if not ok:
        FAILS.append(name)
        print(f"FAIL {name} {detail}")


def h(x):
    return bytes.fromhex(x)


def derive_key(ctx, ikm):
    return blake3.blake3(ikm, derive_key_context=ctx).digest()


def xof(ctx, ikm, n):
    return blake3.blake3(ikm, derive_key_context=ctx).digest(length=n)


def x_pub(sec):
    return x25519.X25519PrivateKey.from_private_bytes(sec).public_key().public_bytes_raw()


def x_dh(sec, pub):
    return x25519.X25519PrivateKey.from_private_bytes(sec).exchange(x25519.X25519PublicKey.from_public_bytes(pub))


def ed_pub(seed):
    return ed25519.Ed25519PrivateKey.from_private_bytes(seed).public_key().public_bytes_raw()


def ed_sign(seed, msg):
    return ed25519.Ed25519PrivateKey.from_private_bytes(seed).sign(msg)


def ed_verify_strict(pub, msg, sig):
    # RFC 8032 strict: S must be canonical (< L); then the library check
    if int.from_bytes(sig[32:], "little") >= L:
        return False
    try:
        ed25519.Ed25519PublicKey.from_public_bytes(pub).verify(sig, msg)
        return True
    except Exception:
        return False


def aead_seal(key, nonce, pt, aad=None):
    return nb.crypto_aead_xchacha20poly1305_ietf_encrypt(pt, aad, nonce, key)


def aead_open(key, nonce, ct, aad=None):
    return nb.crypto_aead_xchacha20poly1305_ietf_decrypt(ct, aad, nonce, key)


def identity_from_xof(ctx, ikm):
    o = xof(ctx, ikm, 128)
    ed_seed, x_sec, d, z = o[:32], o[32:64], o[64:96], o[96:128]
    ek, dk = ML_KEM_768._keygen_internal(d, z)
    ek2 = mlkem.MLKEM768PrivateKey.from_seed_bytes(d + z).public_key().public_bytes_raw()
    check("identity: kyber-py and OpenSSL derive the same ML-KEM key from d, z", ek == ek2)
    return dict(ed_seed=ed_seed, x_sec=x_sec, d=d, z=z, dk=dk, ed_pub=ed_pub(ed_seed), x_pub=x_pub(x_sec), ek=ek)


def pubs_equal(idn, o):
    return idn["ed_pub"].hex() == o["ed25519_pub"] and idn["x_pub"].hex() == o["x25519_pub"] and idn["ek"].hex() == o["mlkem_pub"]


def argon(inp, salt, mem, it):
    return a2.hash_secret_raw(inp.encode(), salt, it, mem, 1, 32, a2.Type.ID, 19)


# GF(256), AES polynomial 0x11b, plain shift-and-add; Shamir by Horner / Lagrange.
def gmul(a, b):
    r = 0
    while b:
        if b & 1:
            r ^= a
        a = ((a << 1) ^ 0x11B) & 0xFF if a & 0x80 else (a << 1) & 0xFF
        b >>= 1
    return r


def gpow(a, e):
    r = 1
    for _ in range(e):
        r = gmul(r, a)
    return r


def lagrange0(xs):
    out = []
    for i, xi in enumerate(xs):
        num = den = 1
        for j, xj in enumerate(xs):
            if i != j:
                num = gmul(num, xj)
                den = gmul(den, xi ^ xj)
        out.append(gmul(num, gpow(den, 254)))
    return out


def rng_stream(seed, n):
    # rand_chacha ChaCha20Rng::from_seed: djb ChaCha20, 64 bit counter 0, zero nonce
    return Cipher(algorithms.ChaCha20(seed, bytes(16)), mode=None).encryptor().update(bytes(n))


def shamir_split(secret, k, indices, seed):
    stream = rng_stream(seed, 32 * (k - 1))
    coeffs = [secret] + [stream[(i - 1) * 32 : i * 32] for i in range(1, k)]
    out = []
    for x in indices:
        row = bytearray(32)
        for b in range(32):
            acc = 0
            for d in range(k - 1, -1, -1):
                acc = gmul(acc, x) ^ coeffs[d][b]
            row[b] = acc
        out.append((x, bytes(row)))
    return out


def shamir_combine(shares):
    l = lagrange0([x for x, _ in shares])
    out = bytearray(32)
    for (_, sb), li in zip(shares, l):
        for b in range(32):
            out[b] ^= gmul(li, sb[b])
    return bytes(out)


EFF = [w for w in (ROOT / "assets/wordlists/eff_large.txt").read_text().split("\n") if w]
assert len(EFF) == 7776


def eff_encode(ent):
    v = int.from_bytes(ent, "big")
    assert v < 7776**6
    out = []
    for _ in range(6):
        out.append(EFF[v % 7776])
        v //= 7776
    return " ".join(out)


def normalize(p):
    return " ".join(p.lower().split())


def wrap_open(blob, idn):
    """Opens a hybrid wrap blob with the recipient's keys; returns plaintext or None."""
    try:
        assert blob[:4] == b"SSHW"
        eph_pub = blob[6:38]
        klen = struct.unpack(">H", blob[38:40])[0]
        kem_ct = blob[40 : 40 + klen]
        nonce = blob[40 + klen : 64 + klen]
        header, ct = blob[: 64 + klen], blob[64 + klen :]
        x_ss = x_dh(idn["x_sec"], eph_pub)
        kem_ss = mlkem.MLKEM768PrivateKey.from_seed_bytes(idn["d"] + idn["z"]).decapsulate(kem_ct)
        key = derive_key("sshub/sync/wrap/v1", x_ss + kem_ss + eph_pub + idn["x_pub"] + kem_ct)
        return aead_open(key, nonce, ct, header)
    except Exception:
        return None


def run(path):
    data = json.loads(Path(path).read_text())
    for v in data["vectors"]:
        vid, p, i, o, par = v["id"], v["primitive"], v["inputs"], v["outputs"], v["params"]
        if p == "chain_hash":
            check(vid, blake3.blake3(h(i["bytes"])).hexdigest() == o["hash"])
        elif p == "derive_key":
            check(vid, derive_key(par["context"], h(i["ikm"])).hex() == o["key"])
        elif p == "derive_xof":
            check(vid, xof(par["context"], h(i["ikm"]), par["out_len"]).hex() == o["out"])
            check(vid + " prefix is derive_key", o["out"][:64] == derive_key(par["context"], h(i["ikm"])).hex())
        elif p == "ed25519_sign":
            seed = xof(par["context"], h(i["ikm"]), 128)[:32]
            check(vid + " pub", ed_pub(seed).hex() == o["pub"])
            check(vid + " sig", ed_sign(seed, h(i["msg"])).hex() == o["sig"])
        elif p == "ed25519_verify":
            check(vid, ed_verify_strict(h(i["pub"]), h(i["msg"]), h(i["sig"])) == o["valid"])
        elif p == "x25519_dh":
            check(vid + " pub", x_pub(h(i["a_sec"])).hex() == o["a_pub"])
            check(vid + " shared", x_dh(h(i["a_sec"]), h(i["b_pub"])).hex() == o["shared"])
        elif p == "mlkem_encaps":
            d, z, m = h(par["d"]), h(par["z"]), h(par["m"])
            ek, dk = ML_KEM_768._keygen_internal(d, z)
            check(vid + " ek", ek.hex() == i["ek"])
            ss, ct = ML_KEM_768._encaps_internal(h(i["ek"]), m)
            check(vid + " ct, ss", (ct.hex(), ss.hex()) == (o["ct"], o["ss"]))
            check(vid + " OpenSSL decaps", mlkem.MLKEM768PrivateKey.from_seed_bytes(d + z).decapsulate(h(o["ct"])).hex() == o["ss"])
        elif p == "mlkem_decaps":
            ek, dk = ML_KEM_768._keygen_internal(h(par["d"]), h(par["z"]))
            check(vid + " dk bytes", dk.hex() == i["dk"])
            check(vid, ML_KEM_768.decaps(h(i["dk"]), h(i["ct"])).hex() == o["ss"])
        elif p == "dek_seal":
            n = h(par["nonce"])
            check(vid, (n + aead_seal(h(i["dek"]), n, h(i["plaintext"]))).hex() == o["ct"])
        elif p == "argon2id":
            check(vid, argon(i["input"], h(i["salt"]), par["mem_kib"], par["iters"]).hex() == o["key"])
        elif p == "bip39":
            check(vid, Mnemonic("english").to_mnemonic(h(i["entropy"])) == o["phrase"])
            check(vid + " decode", Mnemonic("english").to_entropy(o["phrase"]).hex() == i["entropy"])
        elif p == "eff7776":
            check(vid, eff_encode(h(i["entropy"])) == o["phrase"])
        elif p == "identity_from_seed":
            check(vid, pubs_equal(identity_from_xof(par["context"], h(i["ikm"])), o))
        elif p == "key_file":
            idn = identity_from_xof(par["context"], h(i["ikm"]))
            pr = par["profile"]
            salt, nonce = h(par["salt"]), h(par["nonce"])
            key = argon(i["passphrase"], salt, pr["mem_kib"], pr["iters"])
            header = b"SSHK\x01" + struct.pack(">II", pr["mem_kib"], pr["iters"]) + salt + nonce
            plain = idn["ed_seed"] + idn["x_sec"] + idn["dk"] + b"\x00"  # no older wrapping generations
            check(vid, (header + aead_seal(key, nonce, plain, header)).hex() == o["blob"])
        elif p == "sealed_box":
            idn = identity_from_xof(par["recipient_context"], h(i["recipient_ikm"]))
            eph_s, nonce = h(par["eph_secret"]), h(par["nonce"])
            eph_p = x_pub(eph_s)
            check(vid + " recipient pub", idn["x_pub"].hex() == i["recipient_x25519_pub"])
            key = derive_key("sshub/sync/seal/v1", x_dh(eph_s, idn["x_pub"]) + eph_p + idn["x_pub"])
            sealed = eph_p + nonce + aead_seal(key, nonce, h(i["plaintext"]), eph_p + idn["x_pub"])
            check(vid, sealed.hex() == o["sealed"])
        elif p == "hybrid_wrap":
            idn = identity_from_xof(par["recipient_context"], h(i["recipient_ikm"]))
            check(vid + " recipient keys", pubs_equal(idn, i["recipient"]))
            eph_s, nonce = h(par["eph_secret"]), h(par["nonce"])
            eph_p = x_pub(eph_s)
            ss, kem_ct = ML_KEM_768._encaps_internal(idn["ek"], h(par["m"]))
            key = derive_key("sshub/sync/wrap/v1", x_dh(eph_s, idn["x_pub"]) + ss + eph_p + idn["x_pub"] + kem_ct)
            header = b"SSHW\x01\x01" + eph_p + struct.pack(">H", len(kem_ct)) + kem_ct + nonce
            check(vid, (header + aead_seal(key, nonce, h(i["plaintext"]), header)).hex() == o["blob"])
            check(vid + " opens", wrap_open(h(o["blob"]), idn) == h(i["plaintext"]))
        elif p == "hybrid_unwrap_reject":
            idn = identity_from_xof(par["recipient_context"], h(i["recipient_ikm"]))
            check(vid, (wrap_open(h(i["blob"]), idn) is not None) == o["opens"])
        elif p == "vault_recipient":
            stretched = argon(normalize(i["phrase"]), h(i["salt"]), par["mem_kib"], par["iters"])
            seed = derive_key("sshub/sync/vault/seed/v1", stretched + h(i["ring_secret"]))
            check(vid, pubs_equal(identity_from_xof("sshub/sync/vault/v1", seed), o))
        elif p == "paper_derive":
            ent = Mnemonic("english").to_entropy(i["phrase"])
            ctx = {"unwrap": "sshub/sync/paper/unwrap/v1", "recover": "sshub/sync/paper/recover/v1"}[par["domain"]]
            check(vid, pubs_equal(identity_from_xof(ctx, bytes(ent)), o))
        elif p == "shamir_split":
            sh = shamir_split(h(i["secret"]), par["k"], par["indices"], h(par["rng_seed"]))
            check(vid, [(x, b.hex()) for x, b in sh] == [(q["index"], q["bytes_hex"]) for q in o["shares"]])
        elif p == "shamir_combine":
            check(vid, shamir_combine([(q["index"], h(q["bytes_hex"])) for q in i["shares"]]).hex() == o["secret"])
        elif p == "reshare":
            my = (i["share"]["index"], h(i["share"]["bytes_hex"]))
            sub = shamir_split(my[1], par["k"], par["new_indices"], h(par["rng_seed"]))
            check(vid + " sub-shares", [(x, b.hex()) for x, b in sub] == [(q["index"], q["bytes_hex"]) for q in o["sub_shares"]] and o["from"] == my[0])
            commits = [derive_key("sshub/sync/reshare/commit/v1", bytes([my[0], x]) + b).hex() for x, b in sub]
            check(vid + " commitments", commits == o["commitments"])
        else:
            check(vid, False, f"oracle has no case for primitive {p}")
    print(f"{len(data['vectors'])} vectors replayed")


if __name__ == "__main__":
    run(sys.argv[1] if len(sys.argv) > 1 else str(ROOT / "tests/vectors/crypto/vectors.json"))
    print(f"{COUNT} checks, {len(FAILS)} failed")
    print("RESULT FAIL" if FAILS else "RESULT PASS")
    sys.exit(1 if FAILS else 0)
