#!/usr/bin/env python3
"""List functions in src/ that no test executes even once.

Reads the JSON export of `cargo llvm-cov` (see `just coverage`) and prints the
never-executed functions, largest first. Size is the span of the function's
coverage regions, which is close enough to its line count for ranking and does
not need the source parsed twice.

Function names in the report are v0-mangled, and there is no rustfilt on a bare
distro toolchain, so the name is read back out of the source at the region's
first line instead. That also keeps the output clickable.
"""

import json
import pathlib
import re
import signal
import sys

# Piping into `head` should end the script, not raise BrokenPipeError.
signal.signal(signal.SIGPIPE, signal.SIG_DFL)


def name_at(path: pathlib.Path, line: int, cache: dict) -> str:
    """The nearest `fn <name>` at or above `line` — the enclosing function."""
    if path not in cache:
        try:
            cache[path] = path.read_text().splitlines()
        except OSError:
            cache[path] = []
    lines = cache[path]
    for i in range(min(line, len(lines)) - 1, -1, -1):
        m = re.match(r"\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([a-z_][A-Za-z0-9_]*)", lines[i])
        if m:
            return m.group(1)
    return "?"


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print("usage: uncovered-functions.py <cov.json>", file=sys.stderr)
        return 2

    report = json.loads(pathlib.Path(argv[1]).read_text())
    data = report["data"][0]
    root = str(pathlib.Path.cwd()) + "/"
    cache: dict = {}

    # The report lists each function once per test binary, so a function that
    # ran under `cargo test --test e2e` still appears with count 0 under the
    # binaries that never touched it. Only the sum over its mangled name says
    # whether anything executed it.
    total_count: dict = {}
    for fn in data["functions"]:
        total_count[fn["name"]] = total_count.get(fn["name"], 0) + fn.get("count", 0)

    # Keyed by (file, name): one function can appear once per monomorphization.
    worst: dict = {}
    for fn in data["functions"]:
        if total_count[fn["name"]] != 0:
            continue
        files = [f.replace(root, "") for f in fn.get("filenames", [])]
        files = [f for f in files if f.startswith("src/") and "/tests/" not in f]
        if not files:
            continue
        bounds = [r[0] for r in fn["regions"]] + [r[2] for r in fn["regions"]]
        if not bounds:
            continue
        span, start, path = max(bounds) - min(bounds), min(bounds), files[0]
        # A single-line span is a degenerate entry, not a function body: an
        # unused instantiation or a closure collapsed onto its signature line.
        # They pass the segment check below by accident (one line to look at)
        # and put covered functions like `expand_tilde` in the output.
        if span == 0:
            continue
        key = (path, name_at(pathlib.Path(path), start, cache))
        if key not in worst or span > worst[key][0]:
            worst[key] = (span, start)

    # A zero `count` is not proof on its own: the report also carries degenerate
    # entries (a monomorphization in a codegen unit nothing called) that sit on
    # lines the suite does execute. The file's own line segments are the
    # arbiter — keep a function only if nothing in its line span ever ran.
    executed: dict = {}
    for f in data["files"]:
        path = f["filename"].replace(root, "")
        hits = executed.setdefault(path, set())
        for seg in f["segments"]:
            # segment = [line, col, count, has_count, is_region_entry, ...]
            if seg[3] and seg[2] > 0:
                hits.add(seg[0])

    worst = {
        k: v
        for k, v in worst.items()
        if not any(line in executed.get(k[0], ()) for line in range(v[1], v[1] + v[0] + 1))
    }

    rows = sorted(((v[0], v[1], k[1], k[0]) for k, v in worst.items()), reverse=True)
    print(f"{'lines':>5}  function — location")
    print("-" * 72)
    for span, start, name, path in rows:
        print(f"{span:>5}  {name} — {path}:{start}")
    print(f"\n{len(rows)} functions in src/ are never executed by any test.")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
