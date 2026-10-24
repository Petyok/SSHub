# PoC C: QR budget (offline portion)

VERDICT BLOCKED

C4 (handheld worst case, 5 runs) and C5 (recovery after interruption) need a physical phone and were NOT run. The offline measurements below (C1 to C3) are complete. The phone model is not named because no phone was used.

## Claims

| claim | criterion | measured value | unit | PASS/FAIL |
|-------|-----------|----------------|------|-----------|
| C1 80x24 light | max legible version, M, decoded by zxing-cpp from xterm.js screenshot | 5 | version | PASS (measured) |
| C1 80x24 dark | same, dark palette (inverted polarity) | 5 | version | PASS (measured; plain decode 0 of 5, needs inversion) |
| C1 120x40 light | same | 13 | version | PASS (measured) |
| C1 120x40 dark | same, dark palette | 13 | version | PASS (measured; plain decode 0 of 13, needs inversion) |
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
| C3 4-holder hybrid credential (estimate) | 4 × (1088 ML-KEM ct + 32 X25519 eph + 16 tag + 12 nonce), no header | 4592 | bytes | MEASURED (computed; assumed field sizes) |
| C3 4-holder credential frames at v5 | 4592 / 78 | 59 | frames | MEASURED (computed) |
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
- Light palette: background `#ffffff`, foreground `#000000`. Dark palette: background `#000000`, foreground `#ffffff`. In the dark palette the code is light-on-dark (inverted); the decoder is run both ways and both are reported.
- Screenshot: Playwright element screenshot of `.xterm`, deviceScaleFactor 2, headless Chromium.
- Decode: zxing-cpp 3.1.1 via `decode-zxing.py` (OpenCV grayscale input). Legible = decoded bytes equal the payload. "legible_invert_allowed" is the reported figure; "plain" is shown separately.
- Caveat: a desktop decoder on a 2x screenshot is not a phone camera. It shows the code is decodable, not that a handheld camera reads it.

## Commands and logs

- `gpu-verify/tools.sh <hs-phone> phone-offline -- 'env PY=/opt/oracle/bin/python PLAYWRIGHT_MJS=/opt/node/lib/node_modules/playwright/index.mjs OUT_DIR=/tmp/poc-c-png PORT=8771 node /work/assets/companion/poc/run-offline.mjs'`
- Log: `gpu-verify/logs/phone-offline-20261024T134320.log` (container, `RESULT PASS` for the script; PoC verdict above is BLOCKED because C4 and C5 are not run).
- Earlier failed runs (logs `phone-offline-20261024T134159.log`, decode step needed OpenCV instead of PIL) are superseded.
- Screenshots: container `/tmp/poc-c-png` (not persisted).

## Tool versions

- Chromium 156.0.8078.4 (container, `/opt/ms-playwright`)
- Playwright 1.64.1, Node 22.23.4
- zxing-cpp 3.1.1, OpenCV 5.0.0.93 (headless), numpy 2.4.6 (oracle venv)
- xterm.js 5.5.0 (vendored), qrcode-generator 1.4.4 (vendored)

## NOT run

- C4 handheld worst case (phone camera, 5 runs, 80x24).
- C5 recovery after interruption (phone).
- Phone model: none named.
- Real-camera decode at either geometry.
- Laptop Chromium run of C1 (container only).
- Fountain coding is indexed looped chunks, not rateless. The spec's C5 implication (rateless) is not tested here.
