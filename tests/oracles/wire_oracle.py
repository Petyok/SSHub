#!/usr/bin/env python3
"""Independent encoder for tests/vectors/wire/*.json (frames of kinds 11 and 12, contract 4j/4l).
Standard library only: struct builds the frames, the committed JSON is compared byte for byte.

Run: python3 tests/oracles/wire_oracle.py           (check)
     python3 tests/oracles/wire_oracle.py --emit    (rewrite the vector file)
"""
import json
import struct
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
RENEW = ROOT / "tests/vectors/wire/cert_renew_request.json"


def header(kind):
    return b"SWR1" + struct.pack(">HB", 1, kind)


def renew(cert_id, expiry, evidence, opened=None, total=None):
    out = header(11) + bytes.fromhex(cert_id) + struct.pack(">qB", expiry, len(evidence))
    for term, seq, ts in evidence:
        out += bytes.fromhex(term) + struct.pack(">Qq", seq, ts)
    if opened is not None:  # None: a frame without the block
        out += bytes([len(opened)])
        for name in opened:
            raw = name.encode()
            assert 1 <= len(raw) <= 48
            out += bytes([len(raw)]) + raw
        out += struct.pack(">H", total)
    return out


CID = "05" * 32
EV = [("01" * 8, 1 << 40, -1), ("02" * 8, 2 << 40, -2)]
CASES = [
    ("renew-no-block", dict(cert_id=CID, expiry=1800000000, evidence=EV)),
    ("renew-one-host", dict(cert_id=CID, expiry=1800000000, evidence=EV, opened=["web1"], total=1)),
    ("renew-names-and-more", dict(cert_id=CID, expiry=1800000000, evidence=[], opened=["web1", "d\u00e9p\u00f4t"], total=40)),
    ("renew-total-only", dict(cert_id=CID, expiry=1800000000, evidence=EV[:1], opened=[], total=9)),
    ("renew-eight-longest", dict(cert_id=CID, expiry=1800000000, evidence=[], opened=["h" * 48] * 8, total=65535)),
]


def document():
    vectors = []
    for vid, c in CASES:
        vectors.append({"id": vid, "inputs": {
            "cert_id": c["cert_id"], "expiry": c["expiry"],
            "evidence": [{"terminal": t, "head_seq": s, "head_ts": ts} for t, s, ts in c["evidence"]],
            "opened": c.get("opened"), "opened_total": c.get("total"),
        }, "outputs": {"frame": renew(**c).hex()}})
    return json.dumps({"meta": {"contract": "docs/host-sync-contract.md sections 4j, 4l",
                                "opened": "null = the frame carries no opened-hosts block"},
                       "vectors": vectors}, indent=2, ensure_ascii=False) + "\n"


if __name__ == "__main__":
    if "--emit" in sys.argv:
        RENEW.write_text(document())
    elif RENEW.read_text() != document():
        sys.exit("FAIL wire vectors differ from the oracle")
    print("ok", len(CASES), "wire vectors")
