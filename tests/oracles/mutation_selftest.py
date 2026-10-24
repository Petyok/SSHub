#!/usr/bin/env python3
"""Proves an oracle can fail: for every primitive in a vectors file, corrupt one
output of one vector and require the oracle to exit non-zero.

Usage: python3 tests/oracles/mutation_selftest.py <oracle.py> <vectors.json>
"""
import json
import subprocess
import sys
import tempfile
from pathlib import Path


def mutate(value):
    if isinstance(value, bool):
        return not value
    if isinstance(value, str) and value and all(c in "0123456789abcdef" for c in value):
        return ("0" if value[0] != "0" else "1") + value[1:]
    if isinstance(value, str):
        return "x" + value
    return None


def main(oracle, vectors):
    data = json.loads(Path(vectors).read_text())
    seen = set()
    missed = []
    for idx, v in enumerate(data["vectors"]):
        if v["primitive"] in seen or v["params"].get("profile") == "MASTER":
            continue
        field = next((k for k, x in v["outputs"].items() if mutate(x) is not None), None)
        if field is None:
            continue
        seen.add(v["primitive"])
        bad = json.loads(json.dumps(data))
        bad["vectors"][idx]["outputs"][field] = mutate(v["outputs"][field])
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
            json.dump(bad, f)
        r = subprocess.run([sys.executable, oracle, f.name], capture_output=True, text=True)
        caught = r.returncode != 0
        print(f"{'caught' if caught else 'MISSED'} {v['primitive']}.{field}")
        if not caught:
            missed.append(v["primitive"])
    print(f"{len(seen)} primitives mutated, {len(missed)} missed")
    sys.exit(1 if missed else 0)


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
