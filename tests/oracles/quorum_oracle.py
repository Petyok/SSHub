#!/usr/bin/env python3
"""Independent oracle for tests/vectors/quorum/vectors.json (codes, snapshots, email
fragment, wire frames, fountain frames). Libraries: python-blake3, argon2-cffi, PyNaCl
(XChaCha20-Poly1305), the standard library's RFC 4648 base32 mapped onto the Crockford
alphabet, and struct for the frames.

Run: python3 tests/oracles/quorum_oracle.py [vectors.json]
"""
import base64
import json
import struct
import sys
from pathlib import Path

import argon2.low_level as a2
import blake3
import nacl.bindings as nb

ROOT = Path(__file__).resolve().parents[2]
FAILS, COUNT = [], 0
RFC = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567"
CROCK = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"


def check(name, ok, detail=""):
    global COUNT
    COUNT += 1
    if not ok:
        FAILS.append(name)
        print(f"FAIL {name} {detail}")


def h(x):
    return bytes.fromhex(x)


def crockford(b):
    return base64.b32encode(b).decode().rstrip("=").translate(str.maketrans(RFC, CROCK))


def normalize(s):
    out = ""
    for c in s.upper():
        if c in " -\t":
            continue
        c = {"O": "0", "I": "1", "L": "1"}.get(c, c)
        if c not in CROCK:
            return None
        out += c
    return out


def argon(code, salt, p):
    return a2.hash_secret_raw(code.encode(), salt, p["iters"], p["mem_kib"], 1, 32, a2.Type.ID, 19)


def seal(key, nonce, pt, aad):
    return nb.crypto_aead_xchacha20poly1305_ietf_encrypt(pt, aad, nonce, key)


def snapshot_header(profile, issued_at, salt):
    return b"SSHN\x01" + struct.pack(">II", profile["mem_kib"], profile["iters"]) + struct.pack(">q", issued_at) + salt


def run(path):
    data = json.loads(Path(path).read_text())
    snap_blob = None
    for v in data["vectors"]:
        vid, p, i, o, par = v["id"], v["primitive"], v["inputs"], v["outputs"], v["params"]
        if p == "crockford_encode":
            check(vid, crockford(h(i["bytes"])) == o["code"])
        elif p == "normalize_code":
            n = normalize(i["input"])
            check(vid, (n is not None) == o["ok"] and (n or "") == o["code"])
        elif p == "snapshot_seal":
            salt, nonce, prof = h(par["salt"]), h(par["nonce"]), par["profile"]
            code = normalize(i["code"])
            key = argon(code, salt, prof)
            header = snapshot_header(prof, i["issued_at"], salt)
            plain = bytes([i["share_index"]]) + h(i["share_bytes"])
            blob = header + nonce + seal(key, nonce, plain, header)
            check(vid, blob.hex() == o["blob"])
        elif p == "snapshot_open":
            b = h(i["blob"])
            prof = {"mem_kib": struct.unpack(">I", b[5:9])[0], "iters": struct.unpack(">I", b[9:13])[0]}
            issued = struct.unpack(">q", b[13:21])[0]
            salt, nonce, ct = b[21:37], b[37:61], b[61:]
            now = i["now"]
            ok = False
            share = (0, "")
            code = normalize(i["code"])
            if code is not None and len(code) == 16 and issued - now <= 600 and now - issued <= 7 * 86400:
                try:
                    plain = nb.crypto_aead_xchacha20poly1305_ietf_decrypt(ct, b[:37], nonce, argon(code, salt, prof))
                    ok, share = True, (plain[0], plain[1:].hex())
                except Exception:
                    pass
            check(vid, ok == o["ok"] and (share == (o["share_index"], o["share_bytes"]) if ok else True))
        elif p == "email_fragment_seal":
            salt, nonce, prof = h(par["salt"]), h(par["nonce"]), par["profile"]
            key = argon(normalize(i["code"]), salt, prof)
            header = b"SSHE\x01" + salt + nonce
            check(vid, (header + seal(key, nonce, h(i["fragment"]), header)).hex() == o["blob"])
        elif p == "wire_challenge":
            op = h(i["op_canonical"])
            body = h(i["nonce"]) + struct.pack(">q", i["expiry"]) + h(i["eph_x25519_pub"]) + struct.pack(">Q", i["request_id"]) + struct.pack(">I", len(op)) + op + h(i["op_signature"])
            check(vid + " frame", (b"SWR1\x00\x01\x01" + body).hex() == o["frame"])
            pre = b"sshub/sync/challenge/v1" + struct.pack("<H", 1) + op + h(i["nonce"]) + struct.pack("<q", i["expiry"]) + h(i["eph_x25519_pub"]) + struct.pack("<Q", i["request_id"])
            check(vid + " preimage", pre.hex() == o["binding_preimage"])
            check(vid + " hash", blake3.blake3(pre).hexdigest() == o["binding_hash"])
        elif p == "wire_share_response":
            ct = h(i["ciphertext"])
            check(vid, (b"SWR1\x00\x01\x02" + struct.pack(">QQ", i["request_id"], len(ct)) + ct).hex() == o["frame"])
        elif p == "fountain_encode":
            payload = h(i["payload"])
            fb, pct = par["frame_bytes"], par["parity_percent"]
            header_len = 16
            block = fb - header_len
            k = -(-len(payload) // block)
            groups = 0 if (k < 2 or pct == 0) else max(1, -(-k * pct // 100))
            doc = blake3.blake3(payload).digest()[:4]
            blocks = [payload[j * block:(j + 1) * block].ljust(block, b"\0") for j in range(k)]
            frames = []
            for seq in range(k + groups):
                if seq < k:
                    chunk = blocks[seq]
                else:
                    g = seq - k
                    chunk = bytes(block)
                    for j in range(g, k, groups):
                        chunk = bytes(a ^ b for a, b in zip(chunk, blocks[j]))
                frames.append((b"QF" + doc + struct.pack(">IHHH", len(payload), block, groups, seq) + chunk).hex())
            check(vid, frames == o["frames"])
        else:
            check(vid, False, f"no oracle case for {p}")
    print(f"{len(data['vectors'])} vectors replayed")


if __name__ == "__main__":
    run(sys.argv[1] if len(sys.argv) > 1 else str(ROOT / "tests/vectors/quorum/vectors.json"))
    print(f"{COUNT} checks, {len(FAILS)} failed")
    print("RESULT FAIL" if FAILS else "RESULT PASS")
    sys.exit(1 if FAILS else 0)
