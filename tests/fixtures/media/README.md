# Media fixtures (sync::media tests)

Everything here was produced by the phone page's own encoders, not by the Rust code under test.

| file | what |
|---|---|
| `response-short.bin` | the 128 byte `ShareResponseWire` the page sends over audio (share only), request id 4242 |
| `response-full.bin` | the 711 byte reply the page shows as reverse QR (share, approval signature, courier op, snapshot blob) |
| `fast-128.wav` | the page's own audio samples (fast profile, `AudioBuffer.copyToChannel` hook) for `response-short.bin`; 48 kHz, 16 bit mono, 8.2 s |
| `slow-48.wav` | slow profile burst (`channels.mjs` `fskEncode`, identical samples to `poc/share-lib.js` `fskEncode`, checked by `gen/media-gen.mjs`) of the first 48 bytes of `response-short.bin`; 6.4 s (the full 128 bytes would be 16 s and 1.5 MB) |
| `qr-frame-0N.pbm` | the 3 data frames of the reverse QR as drawn on the page's canvas (black modules, white field, 4 module quiet zone, 5 px per module, QR version 11), P4 bitmaps |

Regenerate (tools container, checkout of branch `poc/phone`): run the phone page e2e
(`assets/companion/scripts/e2e.mjs`) cut after the "good" release block and with `gen/qr-dump.patch.mjs`
inserted in `captureDoc`; it writes `response-*.bin`, `fake-mic-fast.wav` (1 s lead, 2 s tail) and the PBMs.
Then `node gen/media-gen.mjs <response-short.bin> <outdir>` trims the page audio to `fast-128.wav` and
writes `slow-48.wav`.

## Decoder comparison tooling (`gen/qrexp*`, `gen/qr-oracle.py`)

`SSHUB_DUMP_DIR=/tmp/qr cargo test dump_qr_variants -- --ignored` writes camera-like PGM variants of the page
frames (and `rot/`: rotation only). `gen/qrexp.rs` (+ `qrexp.Cargo.toml`, copy both into a scratch cargo package as
`src/main.rs` / `Cargo.toml`) runs rqrr, quircs and rxing over them, `gen/qr-oracle.py` runs zxing-cpp and OpenCV
(tools container). Numbers: `docs/host-sync-poc-results.md`.
