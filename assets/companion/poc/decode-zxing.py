#!/usr/bin/env python3
"""Decode PoC C screenshots with zxing-cpp.

usage: decode-zxing.py cases.json > decoded.json

Each case needs a "file" key. Output adds "plain_hex" and "invert_hex" (None when not decoded).
"plain" disables inversion, "invert" enables it. Both are reported, not merged.
"""
import json
import sys

import cv2
import zxingcpp


def decode(img, invert):
    res = zxingcpp.read_barcodes(
        img,
        formats=zxingcpp.BarcodeFormat.QRCode,
        try_invert=invert,
        try_rotate=False,
    )
    for r in res:
        if r.bytes:
            return r.bytes.hex()
    return None


def main():
    cases = json.load(open(sys.argv[1]))
    out = []
    for c in cases:
        img = cv2.imread(c["file"], cv2.IMREAD_GRAYSCALE)
        if img is None:
            raise SystemExit("cannot read " + c["file"])
        c["plain_hex"] = decode(img, False)
        c["invert_hex"] = decode(img, True)
        out.append(c)
    json.dump(out, sys.stdout)


if __name__ == "__main__":
    main()
