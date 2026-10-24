#!/usr/bin/env python3
"""Independent oracle for tests/vectors/op/ops.json.

Rebuilds the canonical bytes of every op from the vector inputs following ONLY
the rules of docs/host-sync-contract.md (C2 and the Op/OpBody listing), then
checks the committed canonical bytes, the BLAKE3 hash and the Ed25519 signature
with libraries that are not the Rust implementation (python-blake3,
cryptography). It does not read any Rust source.

Rules used (contract C2): compact JSON, fixed hand-ordered fields, `prev_hash`
and every fixed 32-byte value as lowercase hex strings, every `Vec<u8>` as a
base64 string (standard alphabet, padded), uids as 32 lowercase hex characters
(already strings in the inputs), integers as plain literals, strings as raw
UTF-8 with only the mandatory JSON escapes, alg as the two-byte wire form
`[sig, wrap]`, bodies externally tagged, patch fields omitted when absent and
`null` when cleared (the vector input simply omits or nulls them). In the
inputs every `Vec<u8>` is given as hex; the oracle converts it to base64.

Run: python3 tests/oracles/op_canonical.py [ops.json]
Exit status 0 only if every check passes. A built-in self-test mutates a
vector and requires the checks to fail, so a vacuous oracle cannot pass.
"""
import base64
import contextlib
import io
import json
import sys
from pathlib import Path

import blake3
from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric import ed25519

ROOT = Path(__file__).resolve().parents[2]

HEADER = ["device_id", "seq", "lamport", "prev_hash", "timestamp", "epoch", "alg", "body"]

# Contract section 2: field order of every body variant.
BODY_ORDER = {
    "HostUpsert": ["host_uid", "fields"],
    "HostDelete": ["host_uid"],
    "GroupUpsert": ["group_uid", "fields"],
    "GroupDelete": ["group_uid"],
    "IdentityUpsert": ["identity_uid", "fields"],
    "IdentityDelete": ["identity_uid"],
    "CredUpdate": ["host_uid", "ciphertext", "wrapped_shares"],
    "DeviceAdd": ["device", "name", "role", "attendance", "kind"],
    "DeviceRevoke": ["device_id"],
    "ShareRelease": ["host_uid", "request_id", "requester"],
    "ShareReshareSession": ["epoch", "to", "sealed"],
    "ShareStorageKey": ["epoch", "sealed"],
    "ShareReshareBatch": ["epoch", "new_holders", "items"],
    "ShareReshareConfirmBatch": ["epoch", "items"],
    "WrapKeyRotate": ["device_id", "wrap_epoch", "x25519_pub", "mlkem_pub"],
    "CertIssue": ["device_id", "scope", "expiry", "max_new_hosts_per_day", "epoch_lo", "epoch_hi"],
    "CertRevoke": ["cert_id"],
    "CertHostOpened": ["cert_id", "host_uid"],
    "PhoneReenroll": ["initiated_at", "signers", "via_paper", "third_factor", "liveness", "paper_sig"],
    "PhoneReenrollCancel": ["reenroll_hash", "by_phone", "third_factor"],
    "RecoveryApprove": ["reenroll_hash"],
    "RecoveryAck": ["reenroll_hash", "seen_at"],
    "CheckpointPropose": ["cut", "state_hash", "state_len", "prev_checkpoint", "required"],
    "CheckpointSign": ["proposal_hash"],
    "ModeChange": ["to", "migrated_on", "cosigner"],
    "HeadAttest": ["device_id", "seq", "chain_hash"],
}

# Items of the reshare batches (contract section 2: ReshareItem, ConfirmItem).
ITEM_ORDER = {
    "ShareReshareBatch": ["host_uid", "sub_shares", "commitments"],
    "ShareReshareConfirmBatch": ["host_uid", "sealed_share"],
}

# Contract section 2 and "Patch fields": order of the fields objects.
FIELDS_ORDER = {
    "HostUpsert": ["name", "label", "address", "port", "username", "proxy_jump", "forward_agent",
                   "remote_command", "transport", "environment", "os_icon", "tags", "notes",
                   "favorite", "secret_lookup", "has_password", "group_uids", "identity_uid"],
    "GroupUpsert": ["name", "default_identity_uid", "default_username", "default_port",
                    "default_proxy_jump", "default_transport", "default_forward_agent", "parent_uid"],
    "IdentityUpsert": ["name", "username", "has_password"],
}

# Fixed 32-byte values: hex strings in the inputs and in the canonical form.
HEX32_KEYS = {"ed25519_pub", "x25519_pub", "reenroll_hash", "chain_hash", "cert_id",
              "state_hash", "prev_checkpoint", "proposal_hash", "op_hash"}
# Fixed 64-byte value (a signature): a 128 character hex string, or null.
HEX64_KEYS = {"paper_sig"}
# Vec<u8>: hex in the inputs, base64 in the canonical form.
VEC_KEYS = {"mlkem_pub", "ciphertext", "sealed", "sealed_share"}

FAILS = []
COUNT = 0


def check(name, ok, detail=""):
    global COUNT
    COUNT += 1
    if not ok:
        FAILS.append(name)
        print(f"FAIL {name} {detail}")


def s(v):
    # ensure_ascii=False: raw UTF-8; json escapes only quote, backslash and C0 controls.
    return json.dumps(v, ensure_ascii=False)


def num(v):
    assert isinstance(v, int) and not isinstance(v, bool), v
    return str(v)


def b64(hexstr):
    return s(base64.b64encode(bytes.fromhex(hexstr)).decode("ascii"))


def obj(pairs):
    return "{" + ",".join(f"{s(k)}:{v}" for k, v in pairs) + "}"


def seq(items):
    return "[" + ",".join(items) + "]"


def value(key, v):
    """Serialise one input value by the key it sits under."""
    if v is None:
        return "null"
    if key in VEC_KEYS:
        return b64(v)
    if key in HEX64_KEYS:
        assert len(v) == 128 and v == v.lower()
        bytes.fromhex(v)
        return s(v)
    if key in HEX32_KEYS:
        assert len(v) == 64 and v == v.lower()
        bytes.fromhex(v)
        return s(v)
    if key == "commitments":
        return seq([b64(c) for c in v])
    if key == "sub_shares":
        return seq([seq([s(d), b64(b)]) for d, b in v])
    if key == "alg":
        return seq([str(x) for x in v])
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, int):
        return num(v)
    if isinstance(v, str):
        return s(v)
    if isinstance(v, list):
        return seq([value(key, x) for x in v])
    if isinstance(v, dict):
        # Nested records (device, wrapped share, head attest, scope, third factor):
        # order is the declaration order of the contract.
        return obj([(k, value(k, v[k])) for k in nested_order(key, v)])
    raise AssertionError((key, v))


def nested_order(key, v):
    if key == "device":
        return ["ed25519_pub", "x25519_pub", "mlkem_pub"]
    if key == "wrapped_shares":
        return ["recipient", "alg", "sealed"]
    if key == "liveness":
        return ["device_id", "seq", "chain_hash"]
    if key == "cut":
        return ["device_id", "seq", "op_hash", "lamport"]
    # scope is a single-key externally tagged object (third factor is a string).
    assert len(v) == 1, (key, v)
    return list(v)


def body(b):
    assert len(b) == 1
    (name, inner), = b.items()
    out = []
    for k in BODY_ORDER[name]:
        if k == "fields":
            order = FIELDS_ORDER[name]
            extra = set(inner["fields"]) - set(order)
            assert not extra, extra
            out.append((k, obj([(f, value(f, inner["fields"][f])) for f in order if f in inner["fields"]])))
        elif k in ("liveness", "wrapped_shares", "cut"):
            out.append((k, seq([value(k, x) for x in inner[k]])))
        elif k == "items":
            order = ITEM_ORDER[name]
            out.append((k, seq([obj([(f, value(f, it[f])) for f in order]) for it in inner[k]])))
        else:
            out.append((k, value(k, inner[k])))
    return obj([(name, obj(out))])


def canonical(op):
    parts = []
    for k in HEADER:
        if k == "body":
            parts.append((k, body(op["body"])))
        elif k == "prev_hash":
            h = op[k]
            assert len(h) == 64 and h == h.lower()
            parts.append((k, s(h)))
        else:
            parts.append((k, value(k, op[k])))
    return obj(parts).encode("utf-8")


def verify(pub, msg, sig):
    try:
        ed25519.Ed25519PublicKey.from_public_bytes(pub).verify(sig, msg)
        return True
    except InvalidSignature:
        return False


def check_vector(v, pub):
    name = v["id"]
    rebuilt = canonical(v["op"])
    check(f"{name}: canonical bytes", rebuilt.hex() == v["canonical_hex"],
          f"\n  oracle {rebuilt.decode()[:300]}\n  rust   {bytes.fromhex(v['canonical_hex']).decode(errors='replace')[:300]}")
    check(f"{name}: blake3", blake3.blake3(rebuilt).hexdigest() == v["hash_hex"])
    check(f"{name}: ed25519", verify(pub, rebuilt, bytes.fromhex(v["signature_hex"])))


def main():
    path = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "tests/vectors/op/ops.json"
    data = json.loads(path.read_text())
    pub = bytes.fromhex(data["meta"]["signer_ed25519_pub_hex"])
    for v in data["vectors"]:
        check_vector(v, pub)

    # Self-test: the oracle must notice a one-byte change in every layer.
    first = data["vectors"][0]
    mut = json.loads(json.dumps(first))
    mut["op"]["seq"] += 1
    n = len(FAILS)
    with contextlib.redirect_stdout(io.StringIO()):
        check_vector(mut, pub)
    check("self-test: mutated input is caught", len(FAILS) - n == 3)
    del FAILS[n:]
    bad_sig = bytearray(bytes.fromhex(first["signature_hex"]))
    bad_sig[0] ^= 1
    check("self-test: flipped signature fails", not verify(pub, bytes.fromhex(first["canonical_hex"]), bytes(bad_sig)))

    names = {list(v["op"]["body"])[0] for v in data["vectors"]}
    check("every body variant has a vector", names == set(BODY_ORDER), str(set(BODY_ORDER) - names))
    print(f"{COUNT} checks, {len(FAILS)} failures")
    sys.exit(1 if FAILS else 0)


if __name__ == "__main__":
    main()
