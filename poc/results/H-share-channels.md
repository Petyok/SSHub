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

## Headless pipeline proof

Driver: `assets/companion/poc/pipeline-proof.mjs`. Headless Chromium 156 with fake audio and video devices, page served from loopback. Audio input is the sender's FSK burst, camera input is a y4m QR. This is not over the air and not a phone.

| pipeline | result | time to completion | notes |
|----------|--------|--------------------|-------|
| H audio receive, slow (60 ms symbols) | PASS | 8.5 s | trial 1, 56 B decoded, page errors 0 |
| H audio receive, fast (30 ms symbols) | PASS | 5.0 s | trial 1, 56 B decoded, page errors 0 |
| H camera QR receive (v10, 56 B) | PASS | 343 ms | 1 frame, page errors 0 |

Final run: `gpu-verify/logs/pipeline-proof5-20261024T140512.log`. Across the earlier passing runs, slow took 5.5 s, fast took 2.0 s, and the camera took 319 to 332 ms. Slow exceeds the H3 target of under 5 s in the final run.

Bug found by this proof: `share-transmit.html` sent the raw payload without `L.frame`, so `share-receive.html` could never decode its burst. Fixed in the same change. The offline bench framed its payloads, so it did not catch this.

## NOT run

- H3 over-the-air success (phone to laptop speaker, 10 attempts, under 5 s).
- H4 channel matrix (10 attempts per cell).
- H5 channel order decision.
- Camera-based QR reading (H2 over the air).
- Button-driven trials are covered headless by the Headless pipeline proof above. Over-the-air phone-to-laptop trials are NOT run; see `MANUAL-RUNS.md`. Task 5 matrix not run.
- Real speaker and microphone response (H1 noise is simulated only).

## Over the air

Scheduled for the next session, not run: H3 and H4 (phone speaker to laptop microphone, FSK, 10 attempts per cell). Driver: `assets/companion/scripts/phone-ota.mjs audio --noqr`.

### Reverse QR: phone screen to laptop webcam (measured)

Conditions: Samsung SM-A536E (6.5 in, 1080x2400, Chrome 155), screen brightness at maximum, held by the maintainer at roughly 13 to 25 cm from the laptop webcam (the distance is an estimate, see below), quiet room, normal indoor light. Receiver: throwaway Chromium on the laptop, webcam "Apple Facetime HD" at 1280x720 (fixed focus), jsQR on every frame at about 7 frames per second, live preview with an aiming overlay. Payload 160 B (seed 1), QR error correction M, 4 fps, 4 module quiet zone, black on white. Driver: `phone-ota.mjs reverseqr --len 160 --fps 4`.

**The first attempt failed for a page bug, not for the channel.** `share-transmit.html` had no viewport meta, so Chrome laid it out at 980 CSS px; the v10 code (65 modules with the quiet zone) was 260 CSS px wide, 26 percent of the screen (about 1.8 cm, 0.28 mm per module). A single 10 s try then found nothing. After the fix (viewport meta, code as wide as the screen, integer device pixels per module) it works.

| code | modules | phone px per module | share of screen width | frames | runs ok | median | worst |
|---|---|---|---|---|---|---|---|
| v6, ECC M, 4 fps, 160 B | 41 (49 with quiet zone) | 22 | 99.8 percent | 2 | 10 of 10 | 0.67 s | 4.90 s |

Run times (ms): 1145, 458, 459, 672, 454, 4895, 466, 3363, 555, 1462. The two slow runs lost a frame and waited for the next lap (a lap is 0.5 s at 4 fps; indexed looped chunks, no parity). Preflight (first valid chunk): 644 ms. A first valid chunk was decoded without help at the first aim; the holder did not need to hunt. Battery 32 C, 66 percent.

Survey while the holder moved slowly nearer and farther (20 s, 102 webcam frames): 80 of 102 frames carried a valid chunk. The QR measured 6.6 to 11.8 webcam px per module on decoded frames (median 9.4). Estimated distance for that range: about 13 cm (11.8 px) to 24 cm (6.6 px). That estimate assumes the 67.7 mm screen width of this model and a webcam field of view of 1.33 times the distance [INFERENCE, not measured with a ruler].

Limits found: the smallest module that was still decoded was 6.6 webcam px (1.43 mm on the phone). The far limit was NOT reached: no frame failed because the code was too small, so the true minimum is lower than 6.6 px. The phone-screen-region detector in the page (bright region bounding box) was unreliable at this exposure (it reported a region on 9 of 102 frames) and is not used for any conclusion.

What this means for the product: the code must be as wide as the phone screen with an integer number of device pixels per module, and the page must declare the viewport. The earlier 26 percent code could not have worked at any reachable distance for a fixed-focus webcam: 0.28 mm per module is about 1.3 webcam px per module at 25 cm (from the measured 4.6 px per mm at 24 cm, the same estimate as above). Frame count matters more than size once the code is large: a lost frame costs a whole lap.

Not measured: v10 at full width (the product's QR_FRAME_BYTES is 250, a v10 code), closer than about 13 cm, farther than about 25 cm, other lighting, front camera.

Real companion page (`assets/companion/src`, read, not run on the phone in this test): it declares the viewport, and its reply QR is drawn by `drawQr` with `cssPx = min(560, innerWidth - 32)`, an integer scale in CSS pixels (not device pixels), at 8 fps, QR version chosen by the payload (frames of up to 250 B are v10, 57 modules, 65 with the quiet zone). On a 384 CSS px screen that is 352 px, scale 5, canvas 325 CSS px: 85 percent of the width, 0.88 mm per module (against 1.43 mm in the v6 test above), and 14.06 device px per module at this phone's 2.8125 device pixel ratio, which is not an integer (module widths alternate 14 and 15 px). By the measured 4.6 px per mm at 24 cm that is about 4 webcam px per module at 24 cm and about 2.4 at 40 cm; jsQR needs about 3 [INFERENCE, not run]. Next session: run the bench with the real page (v10, full width, 8 fps) before settling the size.
