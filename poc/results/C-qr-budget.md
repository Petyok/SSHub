# PoC C: QR budget (offline portion)

VERDICT PARTIAL (C4 measured on a real phone for four sizes, C5 not run; see "Over the air")

C4 (handheld worst case, 5 runs) and C5 (recovery after interruption) need a physical phone and were NOT run. The offline measurements below (C1 to C3) are complete. The phone model is not named because no phone was used.

## Claims

| claim | criterion | measured value | unit | PASS/FAIL |
|-------|-----------|----------------|------|-----------|
| C1 80x24 light | max legible version, M, decoded by zxing-cpp from xterm.js screenshot | 5 | version | PASS (measured) |
| C1 80x24 dark | same, dark theme, QR forced to explicit black-on-white with own quiet zone (round 3) | 5 | version | PASS (measured; plain decode 5 of 5, inversion not needed) |
| C1 120x40 light | same | 13 | version | PASS (measured) |
| C1 120x40 dark | same, dark theme as above (round 3) | 13 | version | PASS (measured; plain decode 13 of 13) |
| C1 80x24 height limit | v6 at 80x24: lines needed vs rows | 25 lines vs 24 rows | lines | FAIL (height binds; width would allow v13) |
| C2 bytes per frame, v5 M | frame payload = chunk 78 + header 6 | 84 | bytes | PASS (measured) |
| C2 bytes per frame, v13 M | same formula at 120x40 version | 331 capacity, 325 chunk-plus-header | bytes | PASS (computed; not decoded at 325) |
| C2 capacity v5 L / v13 L | reference, not used | 106 / 425 | bytes | MEASURED |
| C3 4 KiB, v5 | frames | 53 | frames | MEASURED |
| C3 4 KiB, v5 at 4 fps | cycle duration | 13.25 | s | MEASURED |
| C3 4 KiB, v5 at 8 fps | cycle duration | 6.625 | s | MEASURED |
| C3 4 KiB offline reassembly | all frames decoded and reassembled | true, 0 decode failures | bool | PASS |
| C3 6 KiB, v5 | frames | 79 | frames | MEASURED |
| C3 6 KiB, v5 at 4 fps | cycle duration | 19.75 | s | MEASURED |
| C3 6 KiB, v5 at 8 fps | cycle duration | 9.875 | s | MEASURED |
| C3 6 KiB offline reassembly | all frames decoded and reassembled | true, 0 decode failures | bool | PASS |
| C3 share-release challenge, 1519 B, v5 | frames | 20 | frames | MEASURED (size computed from `hs-crypto` wrap.rs layout) |
| C3 challenge 1519 B, v5 at 4 fps / 8 fps | cycle duration | 5.0 / 2.5 | s | MEASURED |
| C3 challenge 1519 B, v13 at 8 fps | frames / cycle | 5 / 0.625 | frames / s | MEASURED |
| C3 4-holder credential update, 5109 B ESTIMATE, v5 at 8 fps | frames / cycle | 66 / 8.25 | frames / s | MEASURED (size is an ESTIMATE; body fields unverified) |
| C3 4-holder credential update, 5109 B ESTIMATE, v13 at 8 fps | frames / cycle | 16 / 2.0 | frames / s | MEASURED (size is an ESTIMATE) |
| C4 handheld worst case | under 5 s at 80x24, 5 runs | not run | s | NOT RUN |
| C5 recovery after interruption | resume after interruption | not run | - | NOT RUN |

## Geometry

Rule: each text cell carries two vertical modules; quiet zone 4 modules on every side. Lines = ceil((modules + 8) / 2), width = modules + 8.

| terminal | v1 | v5 | v6 | v10 | v13 | v14 |
|----------|----|----|----|-----|-----|-----|
| 80x24 lines | 15 | 23 | 25 (fail) | - | - | - |
| 120x40 lines | 15 | 23 | 25 | 33 | 39 | 41 (fail) |

Finding: the spec estimate was width-based. At 80x24 the height binds, so v5 is the largest version that fits. At 120x40, v13 is the largest.

## Method

- Half-block renderer in `assets/companion/poc/qr-budget-rig.html`. Output is written to a real `Terminal` from the vendored xterm.js 5.5.0 (DOM renderer, font `monospace` 14 px, `lineHeight` 1.0, `scrollback` 0).
- Payload per version = full M capacity, seeded bytes (worst case for legibility).
- Light palette: background `#ffffff`, foreground `#000000`. Dark theme (round 3): the rig forces the QR to explicit black-on-white with its own quiet zone, whatever the TUI theme. Round 1 inverted light-on-dark results are superseded; plain decode now matches.
- Screenshot: Playwright element screenshot of `.xterm`, deviceScaleFactor 2, headless Chromium.
- Decode: zxing-cpp 3.1.1 via `decode-zxing.py` (OpenCV grayscale input). Legible = decoded bytes equal the payload. "legible_invert_allowed" is the reported figure; "plain" is shown separately.
- Caveat: a desktop decoder on a 2x screenshot is not a phone camera. It shows the code is decodable, not that a handheld camera reads it.

## Commands and logs

- `gpu-verify/tools.sh <hs-phone> phone-offline -- 'env PY=/opt/oracle/bin/python PLAYWRIGHT_MJS=/opt/node/lib/node_modules/playwright/index.mjs OUT_DIR=/tmp/poc-c-png PORT=8771 node /work/assets/companion/poc/run-offline.mjs'`
- Log (round 3, current): `gpu-verify/logs/phone-offline-r3-20261024T134956.log` (`page_errors: []`, Chromium 156.0.8078.4). Round 1 log `phone-offline-20261024T134320.log` is superseded.
- Earlier failed runs (logs `phone-offline-20261024T134159.log`, decode step needed OpenCV instead of PIL) are superseded.
- Screenshots: container `/tmp/poc-c-png` (not persisted).

## Tool versions

- Chromium 156.0.8078.4 (container, `/opt/ms-playwright`)
- Playwright 1.64.1, Node 22.23.4
- zxing-cpp 3.1.1, OpenCV 5.0.0.93 (headless), numpy 2.4.6 (oracle venv)
- xterm.js 5.5.0 (vendored), qrcode-generator 1.4.4 (vendored)

## Headless pipeline proof

| pipeline | result | time to completion | notes |
|----------|--------|--------------------|-------|
| C4 QR scanner (v5, 4096 B) | PASS | 12.93 s | 53 frames, sha256 af2ba40f7dde30b5c1e477a74dcd61d850b9121fd92a68659a1865b2580dcb80 MATCH, page errors 0 |

Headless Chromium with a fake camera from y4m. Not a phone and not handheld. Log: `gpu-verify/logs/pipeline-proof5-20261024T140512.log`.

## NOT run

- C4 handheld worst case (phone camera, 5 runs, 80x24).
- C5 recovery after interruption (phone).
- Phone model: none named.
- Real-camera decode at either geometry.
- Laptop Chromium run of C1 (container only).
- Fountain coding is indexed looped chunks, not rateless. The spec's C5 implication (rateless) is not tested here.

## Over the air (C4, real phone)

Setup: Samsung SM-A536E (Android 16, Chrome 155.0.8059.84), rear camera (480 px wide preview in the page), phone held still by the maintainer, laptop screen showing the animated code in one kitty window (truecolor half blocks, black on white, quiet zone 4, 8 fps, indexed looped chunks of the PoC H scheme: 6 byte chunk header plus the code capacity). Room as reported: quiet, normal indoor light. Scanner: `poc/qr-scanner.html` (jsQR 1.4.0 on every camera frame, about 20 Hz), 5 runs per block, "elapsed" = from the first decoded frame of the run to a matching sha256, "wall" = from the Reset tap to completion. Driver: `assets/companion/scripts/phone-ota.mjs camera`. Battery 33 to 34 C throughout.

Windows: a tiled window is only 701x825 px (66x35 cells at the owner's 12.5 pt font for v5; 82x20 at 10 pt when it shares the workspace). The v5 blocks ran in the tiled 80x24-sized window. The v13 blocks ran in a FLOATING window of 800x840 px (95x44 cells at 10 pt, measured by the player); the codes have their terminal's size (v5 is 45x23 cells, v13 is 77x39 cells), v13 at about 80% of the physical size it would have at the owner's font.

| block | chunks | distance | runs ok | worst elapsed | median elapsed | runs (s) |
|---|---|---|---|---|---|---|
| v5, 1714 B (the real challenge size) | 22 | ~30 cm | 5 of 5 | 7.47 s | 4.07 s | 7.47, 5.32, 4.07, 2.62, 2.62 |
| v5, 4096 B | 53 | ~30 cm | 5 of 5 | 21.78 s | 18.72 s | 19.40, 15.39, 21.78, 8.52, 18.72 |
| v13, 1714 B | 6 | ~50 to 60 cm | 5 of 5 | 1.19 s | 0.66 s | 0.62, 0.58, 1.19, 0.66, 1.18 |
| v13, 4096 B | 13 | ~50 to 60 cm | 5 of 5 | 4.59 s | 2.80 s | 4.59, 2.80, 2.80, 1.45, 2.88 (wall 8.01 s worst, includes the Reset tap; a lap is 13 / 8 = 1.6 s) |

Reading against the criterion (C4: under 5 s at 80x24, five runs): the 80x24 case with the real challenge (v5, 1714 B, 22 frames) is NOT under 5 s in every run: the worst run took 7.5 s and the median 4.1 s; the first runs were slower (7.5, 5.3 s) while the aim was settling, the last two took 2.6 s (a lap at 8 fps is 22 / 8 = 2.75 s, so 2.6 s is one lap). With 4096 B (53 frames, a lap of 6.6 s) the phone needed 2 to 3 laps (median 18.7 s). The 120x40 case (v13) is much faster because the challenge is 6 frames (a lap of 0.75 s): worst 1.2 s. Duplicate counts were high at v5 (up to 120): the phone decodes about 20 frames per second against 8 on screen, and a missed frame costs a whole lap, since there is no parity in this PoC scheme (the product uses parity frames, 25 percent, and the same loop).

Findings from running it:

- A half-block QR wider than the terminal wraps and turns into horizontal stripes. A first v13 attempt used a tiled window that was 66 to 73 cells wide for a 77 cell code and produced exactly that; the player now measures the tty and refuses to draw (exit status 3), and the bench checks the sender before every block: a capture of only the bench window, decoded with jsQR, deleted at once.
- Android Chrome only asks for the camera through a collapsed permission chip in the address bar; the first run of getUserMedia stayed pending until Chrome was restarted after the Android permission was granted.

Scheduled for the next session (not run): **C5** (recovery after covering the screen mid-scan), front camera, handheld walking, and v10 reverse QR; the reverse QR (phone screen to laptop webcam) was measured with a v6 code, see PoC H "Over the air"; handheld movement, other lighting. Block 3 (v13, 1714 B) and an earlier invalid v13 4096 B attempt came from an earlier driver session (the second attempt was disturbed by a second driver using the same camera; it is replaced by the clean 5 of 5 run above). The real sshub unlock modal has not been decoded yet.
