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
[ -x target/release/sshub ] || cargo build --release
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

    echo "==> encoding $name.gif"
    mp4="demo/build/$name.mp4"
    palette="demo/build/$name-palette.png"
    ffmpeg -y -loglevel error -i "$mp4" -vf palettegen "$palette"
    ffmpeg -y -loglevel error -i "$mp4" -i "$palette" \
        -filter_complex '[0:v][1:v]paletteuse' "demo/gifs/$name.gif"
    ls -lh "demo/gifs/$name.gif" | awk '{print "    " $NF ": " $5}'
done
