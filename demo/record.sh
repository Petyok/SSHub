#!/usr/bin/env bash
# Record the README GIFs without getting OOM-killed on low-RAM machines.
#
# VHS's built-in GIF encoder runs palettegen+paletteuse in a single ffmpeg
# filter graph, which buffers every frame in RAM (~3.4 MB per 1200x700 frame:
# a 90s tape at 24fps needs ~7 GB). So the tapes output MP4 (streaming x264,
# flat memory regardless of length) and this script converts each MP4 to a
# GIF with a classic two-pass palette conversion, which also streams.
#
# MP4 masters and palettes stay in the gitignored demo/build/, so a GIF can
# be re-encoded (e.g. with different dithering) without re-recording.
#
# Usage:
#   demo/record.sh                 # all tapes
#   demo/record.sh overview sftp   # only these
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

command -v vhs >/dev/null || { echo "vhs not found on PATH" >&2; exit 1; }
command -v ffmpeg >/dev/null || { echo "ffmpeg not found on PATH" >&2; exit 1; }
# The tapes run the binary from target/demo/, built with `demo-motion`: VHS
# screenshots the terminal and cannot sample a 260ms slide (see the `anim_ms`
# docs in src/tui/mod.rs), so the demo build stretches every motion duration.
# It gets its own target dir on purpose — target/release/sshub must stay the
# binary you actually install.
cargo build --release --features demo-motion --target-dir target/demo
# The tapes run seed-demo.sh, which does `cargo run --example seed-demo` in the
# debug profile. Pre-build it so a cold cargo cache can't stall a recording
# mid-tape while the compiler churns.
cargo build --quiet --example seed-demo

TAPES=("$@")
if [ ${#TAPES[@]} -eq 0 ]; then
    TAPES=(hero navigate connect add-host sftp screenshots)
fi

mkdir -p demo/build demo/gifs

for name in "${TAPES[@]}"; do
    tape="demo/tapes/$name.tape"
    [ -f "$tape" ] || { echo "no such tape: $tape" >&2; exit 1; }
    echo "==> recording $name"
    vhs "$tape"

    # The screenshots tape only exists for its PNGs; its mp4 is scratch.
    [ "$name" = screenshots ] && continue

    # A take that plays faster than the tape asked for is the failure mode this
    # pipeline had for weeks without anyone noticing: VHS writes the container at
    # a fixed 25 fps, so when its screenshot loop cannot keep up with `Set
    # Framerate`, the missing frames come out as a sped-up video rather than a
    # dropped-frame one. Motion designed at 120-280ms then flashes past. Compare
    # the recorded duration against the tape's own sleeps and say so.
    python3 - "$tape" "demo/build/$name.mp4" <<'PY'
import re, subprocess, sys
tape, mp4 = sys.argv[1], sys.argv[2]
typing, visible, want, unknown = 0.05, True, 0.0, False
for line in open(tape):
    line = line.strip()
    if line.startswith("#"):
        continue
    if m := re.match(r"Set TypingSpeed (\d+(?:\.\d+)?)(ms|s)$", line):
        typing = float(m[1]) / (1000 if m[2] == "ms" else 1)
    elif line == "Hide":
        visible = False
    elif line == "Show":
        visible = True
    elif not visible:
        continue
    elif m := re.match(r"Sleep (\d+(?:\.\d+)?)(ms|s)$", line):
        want += float(m[1]) / (1000 if m[2] == "ms" else 1)
    elif m := re.match(r'Type(?:@\S+)? "(.*)"$', line):
        want += len(m[1]) * typing
    elif line.startswith("Wait"):
        unknown = True
    elif re.match(r"(Enter|Tab|Space|Backspace|Escape|Up|Down|Left|Right|Ctrl|Alt|Shift|PageUp|PageDown)", line):
        want += typing
got = float(subprocess.run(
    ["ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries",
     "format=duration", "-of", "default=nk=1:nw=1", mp4],
    capture_output=True, text=True, check=True).stdout)
if unknown or want <= 0:
    print(f"    pacing: {got:.1f}s recorded (tape has an open-ended Wait, not checked)")
    sys.exit(0)
ratio = got / want
verdict = "ok" if abs(ratio - 1) <= 0.15 else "OFF"
print(f"    pacing: {got:.1f}s recorded vs {want:.1f}s asked for ({ratio:.2f}x) {verdict}")
if verdict == "OFF":
    print("    ^ the take is time-compressed or stretched; animations will not read"
          " correctly. Check Set Framerate against what VHS can actually sustain.",
          file=sys.stderr)
PY

    echo "==> encoding $name.gif"
    mp4="demo/build/$name.mp4"
    palette="demo/build/$name-palette.png"
    # Encode at the rate the frames actually carry information. VHS duplicates
    # its captures up to the container's 25 fps, and the demo build draws motion
    # at 12.5 fps to match `Set Framerate 12`, so half of a 25 fps GIF is
    # duplicate frames — and 12.5 fps is exactly 8 centiseconds per frame, which
    # a GIF can express without rounding the delay.
    ffmpeg -y -loglevel error -i "$mp4" -vf 'fps=12.5,palettegen' "$palette"
    ffmpeg -y -loglevel error -i "$mp4" -i "$palette" \
        -filter_complex '[0:v]fps=12.5[v];[v][1:v]paletteuse' "demo/gifs/$name.gif"
    ls -lh "demo/gifs/$name.gif" | awk '{print "    " $NF ": " $5}'
done
