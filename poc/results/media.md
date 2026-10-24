# Terminal-side media receivers (audio FSK, reverse QR): results

VERDICT PASS for the decoders and the build matrix; BLOCKED for everything that needs a real microphone, camera or phone (`mic.rs`, `cam.rs` never ran against hardware, see "Not run").

Branch `poc/media` (local, no upstream). Code: `src/sync/media/` (`fsk.rs`, `qrscan.rs` always compiled; `mic.rs` feature `audio`; `cam.rs` feature `camera`; `mod.rs` the API), `examples/media-probe.rs`, fixtures in `tests/fixtures/media/` (README there says how each file was made). Logs are under `/home/petruha/sshub-dev/gpu-verify/logs/`.

## API for the unlock modal

```rust
use sshub::sync::media::{self, MediaKind, MediaEvent, MediaSupport, ListenHandle};

media::available() -> MediaSupport { audio: bool, camera: bool }   // feature compiled AND device present (enumerates; call when the modal opens)
media::devices()   -> Vec<String>                                   // "audio: ..." / "camera: ..." lines for diagnostics
media::listen(&[MediaKind::Audio, MediaKind::Camera], tx: mpsc::Sender<MediaEvent>) -> ListenHandle
// Drop(ListenHandle): stops the capture threads and releases the devices (joins them, < ~0.2 s)

enum MediaEvent {
    Status(MediaKind, String),           // one line per channel row: "listening on the microphone (48000 Hz)", "hearing a transmission", "no microphone found", "built without the `audio` feature", ...
    Progress(MediaKind, held, needed),   // camera only: blocks of the reply
    Reply(MediaKind, Vec<u8>),           // complete ShareResponseWire bytes (ShareResponseWire::decode already succeeded; request id NOT checked) -> unlock_paths::open_phone_reply
}
```

A channel ends itself after its first valid reply (device released at once); the other keeps listening until the handle is dropped. A kind that this build or machine cannot serve emits one `Status` and starts nothing. Capture runs only while the handle lives; nothing is written to disk; sample and frame buffers die with the thread. Frames that are valid fountain documents but not a `ShareResponseWire` (e.g. the terminal's own challenge shown in view) give a `Status` and listening continues.

## Claims

| Claim | Criterion | Measured | Result |
|---|---|---|---|
| FSK decoder reads the phone page's own audio (fast profile, 128 B share-only `ShareResponseWire`, samples captured from the page's `AudioBuffer.copyToChannel`) | exact bytes, `ShareResponseWire::decode` ok, request id 4242 | equal | PASS |
| FSK slow profile (60 ms), PCM from the page's `fskEncode` | exact bytes | equal (48 B payload; `share-lib.js` and `channels.mjs` encoders checked sample-identical for both profiles by `gen/media-gen.mjs`) | PASS |
| Streaming: chunk sizes 441 / 1024 / 4096 / 48000, 1.5 s silence before, 2 s after | exact | 4 of 4 sizes, both profiles | PASS |
| Three bursts back to back in one stream (slow, fast, slow) | all three, in order | 3 of 3 | PASS |
| Silence, white noise only, cut-off burst, burst with two symbols wiped | nothing decoded, no panic; CRC refuses the wiped one | nothing decoded | PASS |
| White noise, 12 seeds per cell, SNR vs burst power | all seeds at 20/10/5/0 dB required | 20: 12/12 and 12/12; 10: 12/12, 12/12; 5: 12/12, 12/12; 0: 12/12, 12/12; -3: 12/12, 12/12; -6: 6/12, 12/12; -9: 5/12, 6/12 (fast / slow). PoC H (JS Goertzel, 50 trials) had 42/50 fast and 33/50 slow at 0 dB | PASS (margin to -3 dB) |
| Input rates 8, 16, 32, 44.1, 96, 192 kHz (windowed-sinc downsample of the page PCM, clean and +10 dB) | exact at 16 kHz and up | all four columns true at every rate, 8 kHz included | PASS |
| Sound card clock off by 100, 500, 1000, 2000 ppm either sign | exact up to 500 ppm required | ok to 1000 ppm, fails at 2000 ppm (no drift tracking; real cards are within 100 ppm) | PASS |
| QR: the page's own canvas frames (3 data frames of the 711 B full reply, QR v11, 5 px/module) decode and `FountainDecoder` returns the exact 711 B `response-full.bin` | exact | `Progress(1,3)`, `Progress(2,3)`, `Complete(711 B)` | PASS |
| QR out of order, duplicated, one frame missing until the loop repeats | complete only after the missing frame arrives | completes on the 8th push | PASS |
| QR synthetic loop, 3000 B, 17 frames, each lap a random 60% of the frames shuffled with 30% repeats, frames from `fountain_encode` drawn by the `qrcode` crate | exact | complete in lap 3 after 35 frames | PASS |
| Foreign fountain document first, then the real one | follows the new one after 24 foreign frames | exact | PASS |
| Robustness (3 page frames per row, 640x480 table, lighting gradient; blur sigma in px of the 4 px/module image) | required rows 3/3 | scale 2, 2.5, 3, 4, 5 px/module: 3/3; blur 0.5, 1: 3/3, 1.5, 2: 0/3; sensor noise 4, 8, 16, 24: 3/3; rotation 3 to 90 deg at 5 px/module and 4 px/module (rotate, then bin): 3/3; 4 px/module + rot 8 + blur 1 + noise 6: 3/3; 3 px/module + rot 5 + blur 1 + noise 4: 0/3 | PASS |
| Full `verify.sh` | CI mirror | fmt, clippy, build PASS; test 2039 pass, 1 fail = `watcher::tests::debounce_coalesces_rapid_writes` (known flake, also failing in the op-log run; not touched) | PASS (flake) |
| `cargo build --features audio`, `--features camera`, `--features audio,camera --example media-probe`, clippy `--all-targets -D warnings` default and with both, `cargo fmt --check` | all green | all rc=0 (media-final log; libasound2-dev see below) | PASS |
| macOS `nokhwa` backend type-checks (`cargo check --target x86_64-apple-darwin`, backend copied into a scratch crate because the full crate needs a macOS C toolchain for sqlite/openssl) | compiles | compiles (media-20261024T194859.log) | PASS (type check only) |

## QR decoder choice (measured)

144 camera-like variants of the page's frames at 1280x720 (px/module 3 to 6, blur sigma 0.7 to 2.2, rotation 0/10/30 deg, sensor noise 4; `tests dump_qr_variants`), same images for every decoder, number decoded:

| decoder | plain | with 2x upscale retry | ms per 1280x720 frame |
|---|---|---|---|
| rqrr 0.11 | 44 | 70 | 8 (2x: 34) |
| quircs 0.10 | 52 | 73 | 3 (2x: 5) |
| rxing 0.9 (TryHarder) | 86 | | 2 |
| zxing-cpp 3.1.1 (oracle, C++) | 117 | | |
| OpenCV 5.0 (oracle, C++) | 45 | | |

Rotation only at 5 px/module (54 variants, 0 to 45 deg, sensor-clean): quircs 54/54, rxing 54/54, zxing 54/54, rqrr 36/54.

Chosen: `quircs` (MIT, pure Rust, 8 small deps, 3 ms) with a 2x bilinear retry when a grid was found but did not decode: +40% frames (52 to 73) for 2 ms extra only on frames that already hold a code. rxing decodes more of the hard variants (86 vs 73) but drags the `image` crate and a large tree; since the frames loop (a missed frame returns next lap) the 18% gap costs seconds, not correctness. Revisit rxing if the real-camera run shows the reply too slow. Blur above sigma 1.5 at 4 px/module is lost by every pure-Rust decoder; keeping the phone at 25 to 40 cm and in focus matters more than the decoder.

## Dependencies per feature

| feature | crates (new) | build time, Linux | run time, Linux | macOS |
|---|---|---|---|---|
| (always) | `quircs` 0.10 (+ num-derive, num-traits, thiserror, proc macros) | none | none | none |
| `audio` | `cpal` 0.18 (alsa 0.11, alsa-sys 0.4, dasp_sample, libc; on macOS coreaudio-rs, objc2 family) | `libasound2-dev` + `pkg-config` (alsa-sys) | `libasound.so.2` (ALSA, also the PulseAudio/PipeWire shim path); no JACK/PipeWire/Pulse feature enabled | CoreAudio, system frameworks only; first use triggers the microphone permission prompt for the terminal app |
| `camera` | Linux: `linuxvideo` 0.3.5 (bitflags, libc, log, uoctl; 0BSD, raw V4L2 ioctls). macOS: `nokhwa` 0.10.11 with only `input-avfoundation` (nokhwa-core, nokhwa-bindings-macos, objc, cocoa, core-*, image, flume, ...) | nothing on Linux (no libv4l, no bindgen, no libclang); macOS needs a C compiler for `objc_exception` (Xcode CLT, always there for a Rust toolchain) | `/dev/video*` readable by the user (video group); nothing else | AVFoundation; first use triggers the camera permission prompt; `nokhwa_initialize`/`nokhwa_check` handled in `cam.rs` |

Rejected for camera: `nokhwa` on Linux (`input-v4l` pulls `v4l` 0.14 -> `v4l2-sys-mit` -> `bindgen` 0.65 -> `clang-sys`: needs libclang at BUILD time, verified in the container; `v4l` alone has the same bindgen chain), `v4l2r` (bindgen 0.72, also libclang), `v4l` with `libv4l` (needs libv4l at run time). `linuxvideo` talks V4L2 ioctls directly in safe-ish Rust and builds in the stock container with nothing installed.

Camera formats: Linux asks for 1280x720 then 640x480 in raw `YUYV` or `GREY` and takes the luma plane directly. Cameras that only offer MJPEG (rare for built-in laptop cameras, common for some USB ones) report "no camera offers raw YUYV or GREY video"; adding an MJPEG decode (nokhwa-core's `mjpeg`, i.e. the `image` crate) is the follow-up if the real run needs it. Windows: both features build but `cam.rs` reports "not implemented on this OS" and `available()` is false for the camera (cpal works there).

Installed only inside the command in the tools container, uid 10001, no root: `libasound2-dev`, `libasound2`, `libasound2-data` fetched with `apt-get download` and unpacked into a sysroot (`PKG_CONFIG_PATH`/`PKG_CONFIG_SYSROOT_DIR`), because the container has no sudo. On a developer machine that is `apt install libasound2-dev pkg-config`.

## Phone side checked bit for bit

- Audio: `channels.mjs` `fskEncode` is sample-identical to `poc/share-lib.js` `fskEncode` for both profiles (`gen/media-gen.mjs`); the fast fixture is what the shipped page played in headless Chromium (e2e run, 36 checks pass), request id 4242.
- Reverse QR: frames are the page's canvas pixels (`#reverse-qr`, black on white, 4 module quiet zone, version 11 for 250 byte frames, 8 fps loop), dumped as PBM; the full reply is the 711 B `response-full.bin` of the same run.
- Format limits: the audio reply is the share only (128 B, 8.2 s fast, about 16 s slow); the reverse QR carries the full reply with courier ops and snapshot.

## Commands

- Fixtures (phone page e2e cut after the release block, PBM dump patched in, `gen/media-gen.mjs`): `tools.sh <worktree> media-fixtures -- 'sh .scratch/emit.sh'` (log media-fixtures-20261024T182710.log; scratch script not committed, the README in `tests/fixtures/media/` lists the steps)
- Decoder tests: `tools.sh <worktree> media -- 'cargo test --lib sync::media -- --nocapture --test-threads=4'` (log media-final-20261024T195639.log, last section)
- Feature builds, clippy, fmt, libasound sysroot: same log, steps `b_audio`, `b_camera`, `b_both`, `c_default`, `c_both`, `fmt`
- Full CI mirror: `verify.sh <worktree> media-full` (log media-full-20261024T195036.log)
- Decoder comparison: `SSHUB_DUMP_DIR=/tmp/qr cargo test dump_qr_variants -- --ignored`, then `gen/qrexp.rs` and `gen/qr-oracle.py` (log media-20261024T193904.log)
- Dependency probes: `probe-deps-20261024T183052.log`, `probe2-20261024T183557.log`, `rxinfo-20261024T191849.log`
- Hardware run (maintainer's laptop, phone attached): `cargo run --example media-probe --features audio,camera -- 60`, then approve on the phone with sound and QR armed

## Not run

- `mic.rs` and `cam.rs` against real devices (no microphone or camera in the verification container): cpal stream setup, sample formats (F32/I16/U16/I32 handled; others reported), device failure paths, V4L2 format negotiation and the buffer loop, macOS permission flow. Everything behind them (decoders, API, event plumbing) is tested.
- Over the air: a real speaker and microphone (room acoustics, speaker band limits, echo, laptop noise suppression that may eat tones), and a real phone screen seen by a real webcam (glare, moire, auto exposure, focus hunting). The noise and blur tables are simulations; the 3 px/module + blur 1 + rotation row failing says only that a phone held far away will not work.
- A run with `--features audio,camera` tests: the features were built and linted (clippy `--all-targets`), the test suite ran with default features only.
- Windows camera capture (not implemented), MJPEG-only webcams.
- The unlock modal integration (TUI worker).
