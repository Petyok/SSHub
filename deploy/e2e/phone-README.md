# The e2e phone

The ring's phone is the committed companion page (`phone-page/sshub-companion.html`, pinned byte for byte from poc/phone,
fingerprint checked on every start; refresh with `phone-refresh.sh <phone-worktree> <commit>`) in headless Chromium,
driven by `phone-driver.mjs` in the compose service `phone` (`phone-Dockerfile`). The driver adds no hook to the page:
it presses buttons, pastes text into the scan screen's paste box, reads the text the page shows or copies (ops text,
clipboard) and reads the QR the page draws back from the canvas like a camera. Scenarios call it through `phone CMD [JSON]`
(lib-creds.sh). The browser sits behind one small target table (`targets` in the driver); `PHONE_TARGET=desktop` is the
only one so far.

Proofs: `phone-selftest.sh` (tools image, no peers: restore, describe a Red notice, RedPhoneSign, text equals QR) and
scenario `65-phone-driver` (compose, real CLI peers; run with `gpu-verify/e2e.sh` on a copy that holds only scenario 65).

## CLI verbs missing for a real phone (the terminal side must be the real CLI; nothing was added in Rust)

Checked against the binary's own `--help` listing (sync verbs: sync-enroll sync-join sync-now sync-review sync-serve
sync-status plus the admin verbs sync-approve sync-export sync-factor sync-factor-register sync-import-kit sync-migrate
sync-mode sync-recovery sync-revoke sync-role sync-exemption). All phone code lives in the TUI (`src/app/sync_phone.rs`,
`sync_trust.rs`) and the library (`sync::phone_enroll`, `wire.rs`, `hub.rs`); the CLI has none of it. Missing, with the
library call each would wrap and the text channel (base64 of the frame, as the TUI shows/accepts it):

| proposed verb | does | library |
|---|---|---|
| `sync-phone-enroll` | phone enrollment as text: `begin` prints the kind 6 commitment; then each phone frame (kind 6, 7, 8) is read from stdin and the next frame or the four words printed; `--answer y/n`; on Verified it admits the phone | `PhoneEnroll::{begin, scan, answer}`, `admit_phone` |
| `sync-notice <op-id>` | prints the kind 3 Red notice (base64) of a pending Red op, for the phone to scan or paste | `RecoveryNoticeWire::encode` |
| `sync-phone-ops` | reads the phone's kind 5 ops text (base64) from stdin and ingests it (courier ingest); prints what was applied | `PhoneOpsWire::decode`, courier ingest of `hub.rs` |
| `sync-share-request <host>` | prints the kind 1 challenge (base64) of a share release, waits | challenge builder of `unlock_paths` |
| `sync-share-reply` | reads the kind 2 response text from the phone and finishes the unlock | `ShareResponseWire`, `unwrap_share_reply` |
| `sync-cert-request` | prints the kind 9 trust-session request or kind 10 extension request | `cert::cert_request`, `cert_extend_request` |

Until `sync-notice` exists the scenario builds the kind 3 frame from the op bytes in the peer's database (read only, marked
SHIM, `phone frame-notice`). Nothing can stand in for the ingest or for enrollment, so the phone proof does not reach the
ring yet: scenarios 60, 70 and 95 still miss `phone` in their ceremonies.

## Page gap (mine, not done)

The page shows an enrollment frame only as a QR code (`#enroll-qr`), never as text, while the TUI asks for "the phone's
frame as pasted text (base64, as the page copies it)". A terminal without a camera therefore cannot take the phone's kind
6 to 8 frames. The driver reads the QR back from the canvas, which a camera would do. A copy-text field next to the QR
(as `opsOut` has) would close it.

## Driver commands

See the header of `phone-driver.mjs`. Enrollment: `enroll-start`, `enroll-commit`, `enroll-reveal`, `enroll-match`,
`enroll-finish`; Red notice: `red-open`, `red-approve`; share release: `share-open`, `share-approve`; checks:
`verify-ops`. Enrollment and share release are written against the page's real screens but have not run against a
terminal, because no terminal verb exists.
