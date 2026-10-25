# Bluetooth Low Energy channel (terminal as GATT peripheral, phone page as Web Bluetooth central): results

VERDICT PASS for the contract, the link layer, the driver, the integration and the build matrix; BLOCKED for everything that needs a radio (the BlueZ and CoreBluetooth transports and the phone page have never touched a real adapter, see "Not run").

Branch `poc/ble` (local, no upstream), from `feature/host-sync` eed72ec. Contract: `docs/host-sync-contract.md` section C20 (commits d2dbcf0, a17bf18, reported to the director before step 2). Code: `src/sync/media/ble_link.rs` (+ `ble_link/tests.rs`), `ble.rs` (+ `ble/tests.rs`), `ble_bluez.rs`, `ble_mac.rs`, `src/cli/sync_phone_ble.rs`. Logs are under `/home/petruha/sshub-dev/gpu-verify/logs/`. Device test steps: `host-sync-plan/handoff/ble-device-test.md`.

## Design in one paragraph

The terminal is the GATT peripheral: one service, RX (phone writes), TX (terminal notifies), INFO (read: proto, max frame, tag). It advertises the service UUID and the name `SSHub-<4 random Crockford characters>` shown on the terminal screen; nothing about the host or the user is in it. The link carries whole frames of kinds 1 to 10 unchanged; no pairing, no bonding, the link adds no security (frames are signed or sealed end to end already). Messages: HELLO / HELLO_ACK (message size and window), DATA (frame id, chunk index and count, first chunk carries length and CRC-32), cumulative ACK every 4 chunks, ABORT, BYE, PING. One frame in flight per direction, window 8, ack timeout 5 s with 3 retries, HELLO within 10 s, ping 30 s, idle 90 s, frame deadline 120 s, advertising window 300 s, a length is judged before a byte is buffered.

## Crate decision: `bluer` 0.17.4 (pinned `=`), tokio confined to one thread

| Option | Verdict |
|---|---|
| `bluer` (official BlueZ binding, BSD-2): advertisement, GATT server with `AcquireWrite`/`AcquireNotify` datagram sockets (MTU known), field-proven examples | **chosen** |
| thin layer on the `dbus` crate sshub already links (`dbus-secret-service`), `dbus-crossroads` for the GATT application | rejected: ~600 lines of hand-written `org.bluez.GattApplication1` / `LEAdvertisement1` / fd passing for `Acquire*`, no hardware here to test any of it; the saved dependency weight is not worth an unverifiable server |

Contract section 8 says "no async runtime". `bluer` is async on tokio, so the runtime is **confined**: `ble_bluez.rs` owns one dedicated thread with a `current_thread` runtime (`tokio` with only `rt`, `sync`, `macros`, `time`); the driver (`ble.rs`) is a plain thread that talks to it over `std::sync::mpsc` and an unbounded tokio channel; nothing async crosses `Transport` (`poll`, `send`, `advertise`, `disconnect`). The whole thing is behind the `bluetooth` feature and a `cfg(target_os = "linux")` target dependency block.

**Default dependency tree unchanged:** `cargo tree --prefix none -e normal,build,dev` before (HEAD without the feature) and after: 447 lines each, diff empty (ble-b1-20261025T210943.log, line `TREE DIFF rc=0 lines=0 before=447`). New with the feature on Linux: bluer, tokio, futures, dbus-tokio, dbus-crossroads, tokio-stream, pin-project, custom_debug, macaddr, strum, uuid and their small deps (the probe: `bluer` with `bluetoothd` = 76 locked packages, `dbus` 0.9 and `libdbus-sys` shared with the existing tree; needs `libdbus-1-dev` to build, already in the CI image). macOS: `objc2` 0.6, `objc2-foundation` 0.3, `objc2-core-bluetooth` 0.3, `dispatch2` 0.3 (the first two are in the lock already through cpal).

## Claims

| Claim | Criterion | Measured | Result |
|---|---|---|---|
| Byte layout of every message and the INFO characteristic is the contract's | exact bytes | `the_contract_byte_layout_is_exact`, `messages_round_trip_and_reject_malformed_input` (12 malformed shapes rejected) | PASS |
| CRC-32 is IEEE | check value 0xCBF43926 | equal | PASS |
| Chunk arithmetic: `count = 1 + ceil(max(0, len - (chunk - 15)) / (chunk - 7))` | boundaries at chunk 20 and 244, 1 MiB | boundary values; 1 MiB needs 4400 chunks at 244 (fits u16), over u16 at 20 (sender refuses, receiver rejects) | PASS |
| Full exchange both ways at every message size and frame length | exact bytes | proposals 20, 185, 244, 512, notify caps 20, 100, 244; lengths 7 to 100 000 B: 72 combinations, both directions at once | PASS |
| Lossy link: 2% loss of DATA/ACK/PING, 10% duplicates, both ways at once, 20 000 B up and 9 000 B down | exactly once, exact bytes | 12 seeds, all delivered (handshake reliable, as GATT writes are) | PASS |
| Limits enforced before buffering: length > 1 MiB, count not matching length, length < 7, first payload of the wrong size, message above the agreed size | ABORT 2, `rx` stays empty | all five refused, nothing kept | PASS |
| A frame without `"SWR1"` or with a kind outside 1..=10 never reaches the application | ABORT 3 | 3 variants, `SendFailed(3)` at the sender | PASS |
| Corrupted chunk: CRC fails, sender retries under a new frame id; persistent corruption is given up | retry then `SendFailed(1)` | 1 retry (id 3), 3 attempts then give up, session stays up | PASS |
| Version mismatch | ABORT 5 then BYE | exact | PASS |
| One frame in flight; second frame while one is in progress | ABORT 3, first frame untouched | exact | PASS |
| Gaps and duplicates answered with where the receiver is; repeat of a finished frame re-acknowledged, delivered once | ACK(next) | exact (incl. re-ACK of the last chunk and of chunk 0) | PASS |
| Timers: HELLO 10 s, ack timeout 5 s x 3 then close, frame deadline 120 s, ping every 30 s, idle 90 s | virtual clock | all five | PASS |
| Renegotiation (second HELLO / HELLO_ACK) requeues the frame in flight under a new id | exact bytes after | passes; found by the lossy test: a stale partial frame is dropped on both sides | PASS |
| Full channel code through a fake transport: unlock exchange (challenge out, reply in, channel ends, BYE done, central dropped), 100 KB reply, enrollment steps across a reconnect (same tag, current step pushed again), frame the step does not expect, drop-central, dead radio, advertising window, no HELLO, newest queued frame wins | as described | 9 driver tests with the real `serve` thread, phone side a `Link` | PASS |
| Tags and names leak nothing | 4 Crockford chars, no `I L O U`, name <= 10 bytes, random per arming | 200 draws, > 40 distinct of 50 | PASS |
| `bluetooth` build, clippy `-D warnings`, full test suite | green | build rc=0, clippy rc=0, 2332 + 127 + 100 + 28 + 16 + 5 + 1 tests passed, 0 failed, 7 ignored (ble-b2-20261025T212816.log) | PASS |
| `audio,camera,bluetooth` together (ALSA headers from the .debs) incl. `media-probe`, clippy | green | rc=0 both | PASS |
| macOS transport type-checks | `cargo check` for aarch64 and x86_64 apple-darwin | rc=0 both (the module compiled in a scratch crate beside `ble_link.rs`, because the full crate needs a macOS C toolchain for sqlite/openssl; ble-b2 log) | PASS (type check only) |
| Full CI mirror, default features | fmt, clippy, build, test | all PASS; 2612 tests passed, 0 failed, 8 ignored (ble-full-20261025T211756.log) | PASS |
| Default dependency tree | diff empty | empty | PASS |

## Integration

- `media::MediaKind::Ble`, `MediaSupport.ble`, `MediaEvent::Sent(kind)`, `ListenHandle::{send_frame, drop_central, ble_tag}`; `listen` and `listen_with` behave as before for audio and camera. In a build without the feature a Bluetooth request yields one status ("built without the `bluetooth` feature") and `ble_tag()` is `None`.
- Unlock modal (`ui_shares.rs`): with the feature the modal listens on Bluetooth too, sends the same challenge frame the QR shows, shows `Bluetooth: pick SSHub-XXXX in the phone's list`, then `the challenge is on the phone: approve there`; the reply goes through `open_phone_reply` like every other channel. New path kind `UnlockPathKind::Bluetooth`.
- Phone enrollment (`app/sync_phone.rs`): with the feature the flow listens on Bluetooth, pushes every frame it shows (also the admitted frame) and the modal adds the line to pick `SSHub-XXXX`; the phone's frames enter through the same inbox as the camera's.
- CLI: `--ble` on `sync-phone-enroll`, `sync-share-request`, `sync-notice`, `sync-cert-request`, `sync-phone-ops` (flag documented in each verb's `--help`, the usage text, README, man page). It prints `bluetooth: pick SSHub-XXXX in the phone page's device list`, statuses as `bluetooth:` lines and `sent:` when the phone has the frame; stdin keeps working next to it. In a build without the feature it exits 1 with "this build has no Bluetooth channel: build with `--features bluetooth`".
- `examples/media-probe.rs --ble`: advertises, sends a 3000 byte synthetic frame, prints every frame the page sends back with its CRC-32.

## vhci check on gpu-serv (read only, nothing loaded, nothing run beyond these)

Commands: `ls -l /dev/vhci`, `cat /proc/modules | grep -i -E "bluetooth|vhci|btusb"`, `modinfo hci_vhci`, `uname -r`, `docker run --rm sshub-hostsync-tools:local sh -c "command -v bluetoothd btvirt btmgmt"`.

- `/dev/vhci`: **exists** (`crw------- root root 10, 137`, a static devnode; opening it makes the kernel autoload the module), root only.
- `hci_vhci` is **not loaded** (no bluetooth module in `/proc/modules`); the module file is installed (`/lib/modules/5.15.0-191-generic/kernel/drivers/bluetooth/hci_vhci.ko`), kernel 5.15.0-191-generic, `CONFIG_BT=m`, `CONFIG_BT_HCIVHCI=m`.
- The tools image has no `bluetoothd`, `btvirt` or `btmgmt`.

**Proposal (not done; needs the maintainer's approval because it loads a kernel module on the shared host):** (1) `modprobe hci_vhci` on gpu-serv (reversible with `rmmod`; virtual controllers are host-wide, there is no real Bluetooth on that machine to disturb, still the maintainer's call); (2) a throwaway container (`--rm`, root inside, `--device /dev/vhci --cap-add NET_ADMIN --cap-add NET_RAW`) with `dbus`, `bluez` and BlueZ's `btvirt` (Debian may not package it; building BlueZ with `--enable-testing` provides it) creating two virtual controllers hci0 and hci1; (3) `bluetoothd` on the system bus of the container; (4) an `#[ignore]` integration test: sshub's `BluezTransport` serving on hci0, a `bluer` GATT client (as the stand-in for the page) on hci1 discovering `SSHub-XXXX`, connecting, `startNotify` on TX, writing HELLO and exchanging frames through `Link::phone`, asserting the same results as the fake-transport tests. Needs one small code change first: an adapter selector (`SSHUB_BLE_ADAPTER`, default the first) so the test can pin hci0. What it would prove that nothing here does: that the `bluer` GATT application registers with a real BlueZ, that `AcquireWrite`/`AcquireNotify` deliver datagrams with the MTU we compute, advertising registration and removal, and reconnect behaviour. What it would not prove: Chrome on Android, real radio, the Android MTU exchange.

## Commands

- Full CI mirror (default features): `verify.sh /home/petruha/sshub-dev/.worktrees/hs-ble ble-full` (ble-full-20261025T211756.log)
- Bluetooth matrix: `tools.sh <worktree> ble-b2 -- 'sh /work/.scratch/b2.sh'`: `cargo build --features bluetooth`, `cargo clippy --all-targets --features bluetooth -- -D warnings`, `cargo test --features bluetooth` (whole suite), `cargo build --features audio,camera,bluetooth --example media-probe` and clippy with the ALSA headers unpacked from the .debs (no root), `cargo check --target aarch64-apple-darwin` and `x86_64-apple-darwin` of `ble_mac.rs` in a scratch crate (ble-b2-20261025T212816.log)
- Link and driver tests alone: `cargo test --lib sync::media` and `cargo test --lib --features bluetooth sync::media::ble` (ble-t2-20261025T211345.log)
- Dependency probes: ble-probe-20261025T202742.log (bluer tree, 76 packages), mac type-check iterations ble-mac-*.log
- Device session: `host-sync-plan/handoff/ble-device-test.md`

## Not run

- **No real radio, anywhere:** `ble_bluez.rs` (advertisement, GATT application, the select loop, MTU, device disconnect) compiled and linted only; `ble_mac.rs` type-checked only. The first run on hardware is the device session.
- **The phone page side** (Web Bluetooth) is the page worker's; nothing here exercised Chrome, the chooser, `requestDevice`, the real Android MTU, `writeValueWithoutResponse` pacing.
- Range, throughput and battery numbers: none measured (the device test collects them).
- macOS behaviour beyond the type check: permission prompt, `maximumUpdateValueLength`, and the fact that a CoreBluetooth peripheral cannot disconnect a central (`drop_central` forgets the central and advertises again; the central leaves when its page does).
- **No key in the TUI to drop the connected phone** (contract C20 says the TUI offers one): `ListenHandle::drop_central()` exists and is tested at the driver level; wiring a key into the unlock and enrollment modals is left to the TUI owner (modal key handling is theirs).
- Windows: the feature builds; the channel reports that it is not implemented.
- The vhci container test (proposal above).
