#!/usr/bin/env python3
"""apk-diff.py A.apk B.apk: compares two unsigned APKs entry by entry (sha256 of the uncompressed bytes, plus the file hash).
Exit 0 when every entry is identical; otherwise lists the differing, missing and extra entries. Used by the reproducibility check."""
import hashlib, sys, zipfile

def entries(path):
    with zipfile.ZipFile(path) as z:
        return {i.filename: hashlib.sha256(z.read(i.filename)).hexdigest() for i in z.infolist() if not i.is_dir()}

def main(a, b):
    ea, eb = entries(a), entries(b)
    fa = hashlib.sha256(open(a, "rb").read()).hexdigest()
    fb = hashlib.sha256(open(b, "rb").read()).hexdigest()
    diff = sorted(n for n in ea.keys() & eb.keys() if ea[n] != eb[n])
    only_a, only_b = sorted(ea.keys() - eb.keys()), sorted(eb.keys() - ea.keys())
    print(f"A {a}: {len(ea)} entries, sha256 {fa}")
    print(f"B {b}: {len(eb)} entries, sha256 {fb}")
    print("whole files identical" if fa == fb else "whole files differ")
    for n in diff: print("DIFFERS  ", n)
    for n in only_a: print("ONLY IN A", n)
    for n in only_b: print("ONLY IN B", n)
    ok = not (diff or only_a or only_b)
    print("ENTRIES IDENTICAL" if ok else "ENTRIES DIFFER")
    return 0 if ok else 1

sys.exit(main(*sys.argv[1:3]))
