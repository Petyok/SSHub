#!/usr/bin/env python3
"""Regenerate the LARGE, full-colour OS logos used by the zoomed host-detail
panel (issue #18) from fastfetch's built-in ASCII logos.

    scripts/gen_os_logos.py            # writes assets/os_logos_large.json

Requires `fastfetch` on PATH (the source of the art). For each canonical distro
id we shell out to

    fastfetch --logo <name> --pipe false -s " "

which prints ONLY the logo with ANSI SGR colour codes, parse those codes into
per-span RGB, and store the result as structured coloured lines. The small
Braille set (assets/os_logos.json, used by the non-zoomed card) is NOT touched
by this script and is generated separately (chafa from font-logos SVGs).

Re-run this whenever fastfetch ships nicer/updated logos. It is deterministic
given a fastfetch version; commit the regenerated JSON.
"""

import json
import os
import re
import subprocess
import sys

# Canonical id (matches CANONICAL_IDS in src/osinfo/logos.rs) -> fastfetch logo.
IDS = {
    "arch": "arch",
    "ubuntu": "ubuntu",
    "debian": "debian",
    "alpine": "alpine",
    "fedora": "fedora",
    "rocky": "rocky",
    "rhel": "rhel",
    "centos": "centos",
    "almalinux": "almalinux",
    "opensuse": "opensuse",
    "linuxmint": "linuxmint",
    "manjaro": "manjaro",
    "popos": "pop",
    "kali": "kali",
    "gentoo": "gentoo",
    "void": "void",
    "nixos": "nixos",
    "endeavouros": "endeavouros",
    "freebsd": "freebsd",
    "macos": "macos",
    "linux": "linux",
}

# Colours are stored as either an ANSI 16-colour INDEX (0-15, an int) so the
# terminal palette/theme still applies at render time (this is how fastfetch
# intends its logos, e.g. Ubuntu = bold-red = the theme's bright red), or a
# truecolor [r, g, b] list for 24-bit / 256-colour escapes. Rust maps 0-15 to
# ratatui's named Colors (Black..White, LightBlack..White).
DEFAULT_IDX = 7  # uncoloured cells (interior padding, invisible anyway)


def xterm256(n):
    """256-colour index -> [r, g, b] (16 base colours pass through as indices)."""
    if n < 16:
        return n
    if n < 232:
        n -= 16
        levels = [0, 95, 135, 175, 215, 255]
        return ("rgb", (levels[n // 36 % 6], levels[n // 6 % 6], levels[n % 6]))
    v = 8 + (n - 232) * 10
    return ("rgb", (v, v, v))


SGR = re.compile(r"\x1b\[([0-9;]*)m")


def parse_line(line):
    """Split one ANSI line into [text, [r, g, b]] spans, coalescing runs."""
    spans = []
    fg = "default"
    bold = False
    pos = 0
    buf = []

    def flush(colour):
        text = "".join(buf)
        buf.clear()
        if text != "":
            c = list(colour[1]) if isinstance(colour, tuple) else colour
            spans.append([text, c])

    for m in SGR.finditer(line):
        # Text before this escape belongs to the current colour.
        buf.append(line[pos:m.start()])
        pos = m.end()
        prev = _resolve(fg, bold)
        codes = [int(c) if c else 0 for c in m.group(1).split(";")]
        fg, bold = _apply(codes, fg, bold)
        # Only break a span when the resolved colour actually changes.
        if _resolve(fg, bold) != prev:
            flush(prev)
    buf.append(line[pos:])
    flush(_resolve(fg, bold))

    # Drop a trailing all-space span (fastfetch pads every line to a fixed
    # width); keep interior spacing which positions the glyphs.
    while spans and spans[-1][0].strip() == "":
        spans.pop()
    return spans


def _apply(codes, fg, bold):
    i = 0
    while i < len(codes):
        c = codes[i]
        if c == 0:
            fg, bold = "default", False
        elif c == 1:
            bold = True
        elif c == 22:
            bold = False
        elif 30 <= c <= 37:
            fg = c - 30
        elif c == 39:
            fg = "default"
        elif 90 <= c <= 97:
            fg = c - 90 + 8
        elif c == 38 and i + 1 < len(codes):
            if codes[i + 1] == 5 and i + 2 < len(codes):
                fg = xterm256(codes[i + 2])
                i += 2
            elif codes[i + 1] == 2 and i + 4 < len(codes):
                fg = ("rgb", (codes[i + 2], codes[i + 3], codes[i + 4]))
                i += 4
        i += 1
    return fg, bold


def _resolve(fg, bold):
    if fg == "default":
        return DEFAULT_IDX
    if isinstance(fg, tuple) and fg and fg[0] == "rgb":
        return fg
    if isinstance(fg, int):
        return fg + 8 if (bold and fg < 8) else fg
    return DEFAULT_IDX


def dump_logo(name):
    res = subprocess.run(
        ["fastfetch", "--logo", name, "--pipe", "false", "-s", " "],
        capture_output=True, text=True,
    )
    if res.returncode != 0:
        raise SystemExit(f"fastfetch failed for {name}: {res.stderr}")
    raw = res.stdout.rstrip("\n").split("\n")
    lines = [parse_line(l) for l in raw]
    while lines and not lines[-1]:  # trim trailing blank rows
        lines.pop()
    return lines


def main():
    if subprocess.run(["which", "fastfetch"], capture_output=True).returncode != 0:
        raise SystemExit("fastfetch not found on PATH")
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    out = {cid: dump_logo(name) for cid, name in IDS.items()}
    dest = os.path.join(root, "assets", "os_logos_large.json")
    with open(dest, "w", encoding="utf-8") as f:
        json.dump(out, f, ensure_ascii=False, separators=(",", ":"))
        f.write("\n")
    print(f"wrote {dest}: {len(out)} logos", file=sys.stderr)


if __name__ == "__main__":
    main()
