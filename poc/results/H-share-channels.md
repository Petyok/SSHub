# PoC H: share channels (offline portion)

VERDICT BLOCKED

H3 (over-the-air success, 10 attempts) and H4 (channel matrix, 10 attempts per cell) need a physical phone and laptop speaker and microphone. They were NOT run. H5 (channel order decision) depends on H3 and H4 and is not decided. The offline measurements below (H1 and H2) are complete.

## Claims

| claim | criterion | measured value | unit | PASS/FAIL |
|-------|-----------|----------------|------|-----------|
| H1 framing round-trip | frame then deframe returns payload | true | bool | PASS |
| H1 framing corruption | single corrupted byte rejected | true | bool | PASS |
| H1 framed length, 56-byte payload | LEN:1 + payload + CRC16 | 59 | bytes | MEASURED |
| H1 clean FSK round-trip, 56-byte payload | decoded equals payload, 50 trials | 50 of 50 | trials | PASS |
| H1 burst duration, 56-byte payload, slow profile (60 ms symbols) | 300 ms preamble + 118 nibbles × 60 ms (spec expects ≈7.38 s) | 7380 | ms | PASS (within 7 to 9 s) |
| H1 clean FSK round-trip, fast profile (30 ms symbols), 56-byte payload | decoded equals payload, 50 trials | 50 of 50 | trials | PASS |
| H1 burst duration, fast profile | 300 ms preamble + 118 nibbles × 30 ms | 3840 | ms | MEASURED |
| H1 simulated noise, slow profile, SNR 20 / 10 / 5 / 0 dB | round-trip, 50 trials each | 50 / 49 / 41 / 33 | trials | MEASURED (round 1 gave 50 / 49 / 41 / 39 at 0 dB; round 3 value is the current one) |
| H1 simulated noise, fast profile, SNR 20 / 10 / 5 / 0 dB | round-trip, 50 trials each | 50 / 50 / 47 / 42 | trials | MEASURED |
| H2 direct, 4096 B, in order | reassembled equals payload | true (41 chunks) | bool | PASS |
| H2 direct, 4096 B, reversed with duplicates | reassembled equals payload | true | bool | PASS |
| H2 direct, 4096 B, 20% gaps then loop | complete after loop | true, 78 frames consumed | bool | PASS |
| H2 direct, 6144 B, in order | reassembled equals payload | true (62 chunks) | bool | PASS |
| H2 direct, 6144 B, reversed with duplicates | reassembled equals payload | true | bool | PASS |
| H2 direct, 6144 B, 20% gaps then loop | complete after loop | true, 117 frames consumed | bool | PASS |
| H2 QR chunks, 6144 B, v10 M | chunk 207 B (capacity 213 minus 6 header), 30 frames, all decoded, reassembled | true, 0 decode failures | bool | PASS |
| H3 over-the-air success under 5 s | at least 8 of 10 | not run | count | NOT RUN |
| H4 channel matrix, 10 attempts per cell | per-cell success | not run | count | NOT RUN |
| H5 channel order decision | depends on H3 and H4 | not decided | - | NOT RUN |

## Method

- `assets/companion/poc/share-lib.js` (`window.ShareLib`): CRC-16/CCITT-FALSE, frame/deframe (LEN:1, payload, CRC16 BE), indexed looped chunks (header idx:2 total:2 chunkCRC:2, 6 bytes; NOT rateless), reassembler, MFSK encoder/decoder (48 kHz, 16 tones from 1600 Hz in 100 Hz steps, 1000 Hz sync, Goertzel detection with 1 ms alignment search). Two profiles: slow (60 ms symbols, 5 ms ramp) and fast `FSK_FAST` (30 ms symbols, 3 ms ramp).
- `share-channel-bench.html` `runSelfTests`, executed in headless Chromium 156 by `run-offline.mjs`.
- H1 noise: additive white Gaussian noise at the stated SNR, relative to buffer power. SIMULATED. It is not room acoustics, speaker response, or microphone response.
- H2 QR: frames rendered with qrcode-generator, decoded with jsQR 1.4.0 (inversion attempts both), in memory. No camera.
- Caveat: the chunk scheme is indexed looped, not rateless. H2 gap recovery relies on the sender looping the set.

## Commands and logs

- `gpu-verify/tools.sh <hs-phone> phone-offline -- 'env PY=/opt/oracle/bin/python PLAYWRIGHT_MJS=/opt/node/lib/node_modules/playwright/index.mjs OUT_DIR=/tmp/poc-c-png PORT=8771 node /work/assets/companion/poc/run-offline.mjs'`
- Log (round 3, current): `gpu-verify/logs/phone-offline-r3-20261024T134956.log`. `page_errors` empty. Round 1 log `phone-offline-20261024T134320.log` is superseded.

## Tool versions

- Chromium 156.0.8078.4 (container), Playwright 1.64.1, Node 22.23.4
- jsQR 1.4.0 (Apache-2.0, vendored), qrcode-generator 1.4.4 (vendored)

## NOT run

- H3 over-the-air success (phone to laptop speaker, 10 attempts, under 5 s).
- H4 channel matrix (10 attempts per cell).
- H5 channel order decision.
- Camera-based QR reading (H2 over the air).
- Task 2 and Task 4 UI: the offline self-test page is the only UI built. Task 5 matrix not run.
- Real speaker and microphone response (H1 noise is simulated only).
