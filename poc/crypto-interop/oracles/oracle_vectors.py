#!/usr/bin/env python3
"""Independent oracle for vectors.json and blobs.json (PoC A).

Nothing here uses the Rust or the JS implementation. Every expected value is
recomputed from the input fields of each vector with different libraries:

  blake3 (python-blake3), cryptography (Ed25519, X25519, ML-KEM-768, Poly1305,
  ChaCha20 stream), PyNaCl/libsodium (XChaCha20-Poly1305 IETF), argon2-cffi
  (reference Argon2 C library), mnemonic (BIP39), kyber-py (second ML-KEM),
  and small pure-Python code for what no library offers (GF(256) Shamir,
  HChaCha20, EFF word encoding, canonical JSON, wire frames).

Run: python3 oracles/oracle_vectors.py [vectors.json] [blobs.json]
Exit status 0 only if every check passes.
"""
import hashlib
import json
import struct
import sys
from pathlib import Path

import argon2.low_level as a2
import blake3
import nacl.bindings as nb
from cryptography.hazmat.primitives.asymmetric import ed25519, mlkem, x25519
from cryptography.hazmat.primitives.ciphers import Cipher, algorithms
from cryptography.hazmat.primitives.poly1305 import Poly1305
from kyber_py.ml_kem import ML_KEM_768
from mnemonic import Mnemonic

ROOT = Path(__file__).resolve().parent.parent
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


# ---------------------------------------------------------------- primitives

def chacha20_stream_djb(key, nonce8, n, counter=0):
    """Original (djb) ChaCha20: 64-bit counter, 64-bit nonce."""
    iv = struct.pack("<Q", counter) + nonce8
    enc = Cipher(algorithms.ChaCha20(key, iv), mode=None).encryptor()
    return enc.update(bytes(n))


def hchacha20(key, nonce16):
    def rotl(v, c):
        return ((v << c) & 0xFFFFFFFF) | (v >> (32 - c))

    s = [0x61707865, 0x3320646E, 0x79622D32, 0x6B206574]
    s += list(struct.unpack("<8I", key)) + list(struct.unpack("<4I", nonce16))

    def qr(a, b, c, d):
        s[a] = (s[a] + s[b]) & 0xFFFFFFFF; s[d] = rotl(s[d] ^ s[a], 16)
        s[c] = (s[c] + s[d]) & 0xFFFFFFFF; s[b] = rotl(s[b] ^ s[c], 12)
        s[a] = (s[a] + s[b]) & 0xFFFFFFFF; s[d] = rotl(s[d] ^ s[a], 8)
        s[c] = (s[c] + s[d]) & 0xFFFFFFFF; s[b] = rotl(s[b] ^ s[c], 7)

    for _ in range(10):
        qr(0, 4, 8, 12); qr(1, 5, 9, 13); qr(2, 6, 10, 14); qr(3, 7, 11, 15)
        qr(0, 5, 10, 15); qr(1, 6, 11, 12); qr(2, 7, 8, 13); qr(3, 4, 9, 14)
    return struct.pack("<8I", *(s[0:4] + s[12:16]))


def x25519_shared(sec, pub):
    return x25519.X25519PrivateKey.from_private_bytes(sec).exchange(
        x25519.X25519PublicKey.from_public_bytes(pub)
    )


def x25519_pub(sec):
    return x25519.X25519PrivateKey.from_private_bytes(sec).public_key().public_bytes_raw()


def ed_pub(seed):
    return ed25519.Ed25519PrivateKey.from_private_bytes(seed).public_key().public_bytes_raw()


def ed_sign(seed, msg):
    return ed25519.Ed25519PrivateKey.from_private_bytes(seed).sign(msg)


def ed_verify(pub, msg, sig):
    try:
        ed25519.Ed25519PublicKey.from_public_bytes(pub).verify(sig, msg)
        return True
    except Exception:
        return False


def xchacha_seal(key, nonce24, pt):
    return nb.crypto_aead_xchacha20poly1305_ietf_encrypt(pt, None, nonce24, key)


def xchacha_open(key, nonce24, ct):
    return nb.crypto_aead_xchacha20poly1305_ietf_decrypt(ct, None, nonce24, key)


def chachabox_seal(recipient_pub, pt, eph_sec, nonce24):
    """crypto_box ChaChaBox as implemented by RustCrypto crypto_box 0.9 (secretbox over
    ChaCha20Legacy), rebuilt here from its definition: box key = HChaCha20(DH, 0^16),
    subkey = HChaCha20(box key, nonce[0:16]), stream = djb ChaCha20(subkey, nonce[16:24]),
    first 32 stream bytes are the Poly1305 key, tag is over the ciphertext only."""
    box_key = hchacha20(x25519_shared(eph_sec, recipient_pub), bytes(16))
    sub = hchacha20(box_key, nonce24[:16])
    ks = chacha20_stream_djb(sub, nonce24[16:24], 32 + len(pt))
    ct = bytes(a ^ b for a, b in zip(pt, ks[32:]))
    tag = Poly1305.generate_tag(ks[:32], ct)
    return x25519_pub(eph_sec) + nonce24 + tag + ct


def chachabox_open(recipient_sec, sealed):
    eph_pub, nonce24, body = sealed[:32], sealed[32:56], sealed[56:]
    tag, ct = body[:16], body[16:]
    box_key = hchacha20(x25519_shared(recipient_sec, eph_pub), bytes(16))
    sub = hchacha20(box_key, nonce24[:16])
    ks = chacha20_stream_djb(sub, nonce24[16:24], 32 + len(ct))
    Poly1305.verify_tag(ks[:32], ct, tag)
    return bytes(a ^ b for a, b in zip(ct, ks[32:]))


# GF(256), AES polynomial 0x11b, plain shift-and-add (no tables, no generator).
def gmul(a, b):
    r = 0
    while b:
        if b & 1:
            r ^= a
        a = ((a << 1) ^ 0x11B) if a & 0x80 else (a << 1)
        a &= 0xFF
        b >>= 1
    return r


def pow_gf(a, e):
    r = 1
    for _ in range(e):
        r = gmul(r, a)
    return r


def lagrange0(xs):
    out = []
    for i, xi in enumerate(xs):
        num, den = 1, 1
        for j, xj in enumerate(xs):
            if i != j:
                num = gmul(num, xj)
                den = gmul(den, xi ^ xj)
        out.append(gmul(num, pow_gf(den, 254)))
    return out


def shamir_split(secret, k, indices, rng_bytes):
    coeffs = [secret] + [rng_bytes[(i - 1) * 32 : i * 32] for i in range(1, k)]
    shares = []
    for x in indices:
        row = bytearray(32)
        for b in range(32):
            acc = 0
            for d in range(k - 1, -1, -1):
                acc = gmul(acc, x) ^ coeffs[d][b]
            row[b] = acc
        shares.append((x, bytes(row)))
    return shares


def shamir_combine(shares):
    xs = [x for x, _ in shares]
    l = lagrange0(xs)
    out = bytearray(32)
    for (x, sb), li in zip(shares, l):
        for b in range(32):
            out[b] ^= gmul(li, sb[b])
    return bytes(out)


def rng_stream(seed32, n):
    return chacha20_stream_djb(seed32, bytes(8), n)


def canon(v):
    return json.dumps(v, separators=(",", ":"), ensure_ascii=False)


def op_canonical(op):
    return canon(
        {
            "device_id": op["device_id"],
            "seq": op["seq"],
            "lamport": op["lamport"],
            "prev_hash": op["prev_hash"].lower(),
            "timestamp": op["timestamp"],
            "epoch": op["epoch"],
            "body": op["body"],
        }
    ).encode("utf-8")


def binding_preimage(op, nonce, expiry, eph, request_id):
    return (
        b"sshub/sync/challenge/v1"
        + struct.pack("<H", 1)
        + op_canonical(op)
        + nonce
        + struct.pack("<q", expiry)
        + eph
        + struct.pack("<Q", request_id)
    )


EFF = [w for w in (ROOT / "wordlists/eff_large.txt").read_text().split("\n") if w]
assert len(EFF) == 7776


def eff_encode(ent):
    v = int.from_bytes(ent, "big")
    assert v < 7776**6
    out = []
    for _ in range(6):
        out.append(EFF[v % 7776])
        v //= 7776
    return " ".join(out)


def mlkem_from_seeds(d, z):
    ek, dk = ML_KEM_768._keygen_internal(d, z)
    # second implementation: cryptography/OpenSSL derives the same ek from d||z
    ek2 = mlkem.MLKEM768PrivateKey.from_seed_bytes(d + z).public_key().public_bytes_raw()
    return ek, dk, ek2


def hybrid_open(blob, x_sec, mlkem_seed_d, mlkem_seed_z):
    assert blob[:4] == b"SSHW"
    eph_pub = blob[6:38]
    klen = struct.unpack(">H", blob[38:40])[0]
    kem_ct = blob[40 : 40 + klen]
    nonce = blob[40 + klen : 64 + klen]
    ct = blob[64 + klen :]
    x_ss = x25519_shared(x_sec, eph_pub)
    kem_ss = mlkem.MLKEM768PrivateKey.from_seed_bytes(mlkem_seed_d + mlkem_seed_z).decapsulate(kem_ct)
    kem_ss2 = ML_KEM_768.decaps(ML_KEM_768._keygen_internal(mlkem_seed_d, mlkem_seed_z)[1], kem_ct)
    check("hybrid: two ML-KEM implementations agree on the shared secret", kem_ss == kem_ss2)
    ikm = x_ss + kem_ss + eph_pub + x25519_pub(x_sec)
    key = blake3.blake3(ikm, derive_key_context="sshub/sync/wrap/v1").digest()
    return xchacha_open(key, nonce, ct), (eph_pub, kem_ct, nonce)


# ---------------------------------------------------------------- vectors.json

FIX_D, FIX_Z = bytes([0x66] * 32), bytes([0x77] * 32)
FIX_RECIP_X = bytes([0x55] * 32)

ARGON_PROFILES = {  # reference parameters from docs (mem KiB, iterations); lanes 1, 32 byte output
    "TEST": (32768, 1),
    "SNAPSHOT": (262144, 4),
    "EMAIL": (65536, 3),
    "MASTER": (1048576, 1),
}


def run_vectors(path):
    data = json.loads(Path(path).read_text())
    for v in data["vectors"]:
        vid, p, i, o, par = v["id"], v["primitive"], v["inputs"], v["outputs"], v["params"]
        if p == "chain_hash":
            check(vid, blake3.blake3(h(i["bytes"])).hexdigest() == o["hash"])
        elif p == "derive_key":
            check(vid, blake3.blake3(h(i["ikm"]), derive_key_context=par["context"]).hexdigest() == o["key"])
        elif p == "ed25519_sign":
            seed, msg = h(i["seed"]), h(i["msg"])
            check(vid + " pub", ed_pub(seed).hex() == o["pub"])
            check(vid + " sig", ed_sign(seed, msg).hex() == o["sig"])
            check(vid + " verify", ed_verify(h(o["pub"]), msg, h(o["sig"])) == o["verify_ok"])
        elif p == "x25519_dh":
            check(vid + " pub", x25519_pub(h(i["a_sec"])).hex() == o["a_pub"])
            check(vid + " shared", x25519_shared(h(i["a_sec"]), h(i["b_pub"])).hex() == o["shared"])
        elif p == "mlkem_encaps":
            d, z, m = h(par["d"]), h(par["z"]), h(par["m"])
            ek, dk, ek2 = mlkem_from_seeds(d, z)
            check(vid + " ek (kyber-py)", ek.hex() == i["ek"])
            check(vid + " ek (cryptography)", ek2.hex() == i["ek"])
            ss, ct = ML_KEM_768._encaps_internal(h(i["ek"]), m)
            check(vid + " ct", ct.hex() == o["ct"])
            check(vid + " ss", ss.hex() == o["ss"])
            ss3 = mlkem.MLKEM768PrivateKey.from_seed_bytes(d + z).decapsulate(h(o["ct"]))
            check(vid + " decaps by cryptography", ss3.hex() == o["ss"])
        elif p == "mlkem_decaps":
            d, z = h(par["d"]), h(par["z"])
            ek, dk, _ = mlkem_from_seeds(d, z)
            check(vid + " dk bytes", dk.hex() == i["dk"])
            check(vid, ML_KEM_768.decaps(h(i["dk"]), h(i["ct"])).hex() == o["ss"])
        elif p == "dek_seal":
            nonce = h(par["nonce"])
            ct = nonce + xchacha_seal(h(i["dek"]), nonce, h(i["plaintext"]))
            check(vid, ct.hex() == o["ct"])
        elif p == "argon2id":
            mem, it = ARGON_PROFILES[par["profile"]]
            check(vid + " params", (par["mem_kib"], par["iters"]) == (mem, it))
            key = a2.hash_secret_raw(i["input"].encode(), h(i["salt"]), it, mem, 1, 32, a2.Type.ID, 19)
            check(vid, key.hex() == o["key"])
        elif p == "bip39":
            check(vid, Mnemonic("english").to_mnemonic(h(i["entropy"])) == o["phrase"])
            check(vid + " decode", Mnemonic("english").to_entropy(o["phrase"]).hex() == i["entropy"])
        elif p == "eff7776":
            check(vid, eff_encode(h(i["entropy"])) == o["phrase"])
        elif p == "shamir_split":
            k, n = par["k"], par["n"]
            sh = shamir_split(h(i["secret"]), k, list(range(1, n + 1)), rng_stream(h(par["rng_seed"]), 32 * (k - 1)))
            got = [(x, b.hex()) for x, b in sh]
            want = [(s["index"], s["bytes_hex"]) for s in o["shares"]]
            check(vid, got == want)
        elif p == "shamir_combine":
            shares = [(s["index"], h(s["bytes_hex"])) for s in i["shares"]]
            check(vid, shamir_combine(shares).hex() == o["secret"])
        elif p == "reshare":
            my = (i["share"]["index"], h(i["share"]["bytes_hex"]))
            k = par["k"]
            sub = shamir_split(my[1], k, par["new_indices"], rng_stream(h(par["rng_seed"]), 32 * (k - 1)))
            want = [(s["index"], s["bytes_hex"]) for s in o["sub_shares"]]
            check(vid, [(x, b.hex()) for x, b in sub] == want and o["from"] == my[0])
        elif p == "op_canonical":
            raw = op_canonical(i["op"])
            check(vid, raw.hex() == o["canonical_hex"])
        elif p == "op_hash":
            check(vid, blake3.blake3(op_canonical(i["op"])).hexdigest() == o["hash"])
        elif p == "op_verify":
            sig = h(i["op"]["signature"])
            check(vid, ed_verify(h(i["pub"]), op_canonical(i["op"]), sig) == o["valid"])
        elif p == "binding_hash":
            pre = binding_preimage(i["op"], h(i["nonce"]), i["expiry"], h(i["eph"]), i["request_id"])
            check(vid + " preimage", pre.hex() == o["preimage"])
            check(vid, blake3.blake3(pre).hexdigest() == o["binding_hash"])
        elif p == "signed_approval":
            pre = binding_preimage(i["op"], h(i["nonce"]), i["expiry"], h(i["eph"]), i["request_id"])
            check(vid + " preimage", pre.hex() == o["preimage"])
            check(vid + " sig", ed_sign(h(par["holder_seed"]), pre).hex() == o["sig"])
            check(vid + " verify", ed_verify(h(i["holder_pub"]), pre, h(o["sig"])))
        elif p == "hybrid_wrap":
            r = i["recipient"]
            blob = h(o["blob"])
            pt, (eph_pub, kem_ct, nonce) = hybrid_open(blob, FIX_RECIP_X, FIX_D, FIX_Z)
            check(vid + " opens", pt.hex() == i["plaintext"])
            check(vid + " recipient keys", x25519_pub(FIX_RECIP_X).hex() == r["x25519_pub"])
            ss, ct = ML_KEM_768._encaps_internal(h(r["mlkem_pub"]), h(par["m"]))
            check(vid + " kem ct is the deterministic encapsulation of m", ct == kem_ct)
            check(vid + " eph pub", x25519_pub(h(par["eph_secret"])) == eph_pub)
            check(vid + " nonce", h(par["nonce"]) == nonce)
            # rebuild the whole blob independently
            x_ss = x25519_shared(h(par["eph_secret"]), h(r["x25519_pub"]))
            key = blake3.blake3(x_ss + ss + eph_pub + h(r["x25519_pub"]), derive_key_context="sshub/sync/wrap/v1").digest()
            mine = b"SSHW\x01\x01" + eph_pub + struct.pack(">H", len(ct)) + ct + nonce + xchacha_seal(key, nonce, h(i["plaintext"]))
            check(vid + " byte-identical rebuild", mine == blob)
        elif p == "sealed_box":
            mine = chachabox_seal(h(i["recipient_x25519_pub"]), h(i["plaintext"]), h(par["sealer_eph_secret"]), h(par["nonce"]))
            check(vid, mine.hex() == o["sealed"])
            check(vid + " open", chachabox_open(h(i["recipient_x25519_secret"]), mine) == h(i["plaintext"]))
        elif p == "share_response":
            ct = chachabox_seal(h(i["eph_x25519_pub"]), h(i["plaintext"]), bytes([0xCC] * 32), bytes([0xDD] * 24))
            frame = b"SWR1" + struct.pack(">H", 1) + b"\x02" + struct.pack(">Q", 42) + struct.pack(">Q", len(ct)) + ct
            check(vid, frame.hex() == o["frame"])
        else:
            check(vid, False, f"oracle has no case for primitive {p}")
    print(f"vectors.json: {len(data['vectors'])} vectors replayed")


# ---------------------------------------------------------------- blobs.json (JS output)

def run_blobs(path):
    d = json.loads(Path(path).read_text())["blobs"]
    hw = d["hybrid_wrap"]
    p = hw["params"]
    pt, parts = hybrid_open(h(hw["output"]["blob_hex"]), h(p["recipient_secrets_for_rust"]["x25519_secret"]), FIX_D, FIX_Z)
    check("blobs hybrid_wrap (JS) opens in Python", pt.hex() == p["plaintext"])
    sr = d["share_response"]
    frame = h(sr["output"]["frame_hex"])
    check("blobs share_response frame header", frame[:7] == b"SWR1\x00\x01\x02")
    rid, ln = struct.unpack(">QQ", frame[7:23])
    check("blobs share_response request_id and length", rid == sr["params"]["request_id"] and ln == len(frame) - 23)
    plain = chachabox_open(h(sr["params"]["challenge_eph_secret_for_rust"]), frame[23:])
    check("blobs share_response (JS) opens in Python", plain.hex() == sr["params"]["plaintext"])
    sa = d["signed_approval"]
    sp = sa["params"]
    op = sp["op"]
    pre = binding_preimage(op, h(sp["nonce"]), sp["expiry"], h(sp["eph_x25519_pub"]), sp["request_id"])
    check("blobs approval preimage (JS) equals Python preimage", pre.hex() == sa["output"]["preimage_hex"])
    check("blobs approval binding hash", blake3.blake3(pre).hexdigest() == sa["output"]["binding_hash_hex"])
    check("blobs approval signature verifies in Python", ed_verify(h(sp["holder_pub"]), pre, h(sa["output"]["sig_hex"])))
    check("blobs op signature verifies in Python", ed_verify(h(sp["requester_pub"]), op_canonical(op), h(op["signature"])))
    print("blobs.json: 3 blob kinds checked")


if __name__ == "__main__":
    vec = sys.argv[1] if len(sys.argv) > 1 else str(ROOT / "vectors.json")
    blobs = sys.argv[2] if len(sys.argv) > 2 else str(ROOT / "blobs.json")
    run_vectors(vec)
    run_blobs(blobs)
    wl = hashlib.sha256((ROOT / "wordlists/eff_large.txt").read_bytes()).hexdigest()
    print(f"eff_large.txt sha256 {wl}")
    print(f"{COUNT} checks, {len(FAILS)} failed")
    print("RESULT FAIL" if FAILS else "RESULT PASS")
    sys.exit(1 if FAILS else 0)
