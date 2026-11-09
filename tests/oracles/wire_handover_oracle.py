#!/usr/bin/env python3
"""Independent encoder for tests/vectors/wire/phone_handover_request.json (kind 12, contract 4j and 4l b: trailer 0x01 || op_hash(32) || op_lamport u64 BE).
Standard library only: struct builds the frames, the committed JSON is compared byte for byte.

Run: python3 tests/oracles/wire_handover_oracle.py           (check)
     python3 tests/oracles/wire_handover_oracle.py --emit    (rewrite the vector file)
"""
import json
import struct
import sys
from pathlib import Path

OUT = Path(__file__).resolve().parents[2] / "tests/vectors/wire/phone_handover_request.json"


def handover(ed, x, mlkem, name, terminal, op_hash=None, op_lamport=None):
    raw = name.encode()
    out = b"SWR1" + struct.pack(">HB", 1, 12) + bytes.fromhex(ed) + bytes.fromhex(x)
    out += struct.pack(">H", len(bytes.fromhex(mlkem))) + bytes.fromhex(mlkem)
    out += bytes([len(raw)]) + raw + bytes.fromhex(terminal)
    if op_hash is not None:  # stage 2: the frame the NEW phone is shown
        out += b"\x01" + bytes.fromhex(op_hash) + struct.pack(">Q", op_lamport)
    return out


BASE = dict(ed="01" * 32, x="02" * 32, mlkem="3031323334", name="new phone", terminal="09" * 8)
CASES = [
    ("handover-request", {**BASE}),
    ("handover-admission", {**BASE, "op_hash": "ab" * 32, "op_lamport": 72623859790382856}),
    ("handover-admission-long-name", {**BASE, "name": "n" * 255, "op_hash": "5a" * 32, "op_lamport": 2**64 - 1}),
]


def document():
    vectors = [{"id": i, "inputs": {**c, "op_hash": c.get("op_hash"), "op_lamport": c.get("op_lamport")}, "outputs": {"frame": handover(**c).hex()}}
               for i, c in CASES]
    return json.dumps({"meta": {"contract": "docs/host-sync-contract.md sections 4j, 4l",
                                "op_hash": "null = the request; set = the admission frame"},
                       "vectors": vectors}, indent=2) + "\n"


if __name__ == "__main__":
    if "--emit" in sys.argv:
        OUT.write_text(document())
    elif OUT.read_text() != document():
        sys.exit("FAIL wire vectors differ from the oracle")
    print("ok", len(CASES), "wire vectors")
