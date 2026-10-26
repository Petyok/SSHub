# sshub-phone-core

The phone's rules in Rust, shared with the terminal: everything the companion page
(`assets/companion/src/*.mjs`) decides, on top of `sshub-sync-proto`, behind uniffi. Design:
`docs/host-sync-android.md` (branch `poc/android`), contract C21. Rust never touches the platform: Kotlin passes bytes and
`now` in; the core returns views (`src/views.rs`) and bytes.

| module | content | port of |
|---|---|---|
| `identity` | secret to keys, wrap generations (derived, up to 32), 24 words | `keys.mjs` |
| `state` | `PhoneState`, sealed blob, version counter | `state.mjs` |
| `chain` | authoring every op the phone signs, two list head adoption | `state.mjs`, `ops.mjs` |
| `describe`, `release`, `notices` | what an approval shows, the 4 field `ShareRelease`, the reply, Red and recovery notices | `ops.mjs`, `release.mjs`, `app.mjs` |
| `cert`, `reshare` | trust sessions, reshare adoption, wrap key rotation | `cert.mjs`, `reshare.mjs` |
| `enroll` | joining side of the enrollment | `app.mjs`, `frames.mjs` |
| `channels`, `ble` | QR animation, camera decode, FSK, the C20 link | `channels.mjs`, `scan.mjs`, `ble.mjs` |
| `ffi` | the uniffi object `PhoneCore` | |

## Tests

`cargo test -p sshub-phone-core` (host). `tests/parity.rs` runs the scenario of
`assets/companion/scripts/parity-page.mjs` through the core and compares with `test/fixtures/parity-page.json`, which the
page produced; the gate regenerates that file with node and fails on a difference, so neither side can drift.

## Android build

The workspace root strips symbols in `[profile.release]`; uniffi library mode reads its metadata from symbols, so the Android
library is built with the dedicated profile (`strip = "none"`, size optimised; AGP strips the packaged `.so`):

```sh
cargo ndk -t arm64-v8a -t x86_64 build -p sshub-phone-core --profile phone --locked
```

Default builds (`cargo build`, `cargo install sshub`) are unaffected: the crate is a workspace member but not a default member,
and `[profile.phone]` is only used when asked for.
