# The e2e phone

The ring's phone is the committed companion page (`phone-page/sshub-companion.html`, pinned byte for byte from poc/phone,
fingerprint checked on every start; refresh with `phone-refresh.sh <phone-worktree> <commit>`) in headless Chromium,
driven by `phone-driver.mjs` in the compose service `phone` (`phone-Dockerfile`). The driver adds no hook to the page:
it presses buttons, pastes text into the scan screen's paste box, reads the text the page shows or copies (ops text,
clipboard) and reads the QR the page draws back from the canvas like a camera. Scenarios call it through `phone CMD [JSON]`
(lib-creds.sh). The browser sits behind one small target table (`targets` in the driver); `PHONE_TARGET=desktop` is the
only one so far.

Proofs: `phone-selftest.sh` (tools image, no peers: restore, describe a Red notice, RedPhoneSign, text equals QR) and
scenarios 60, 65, 70 and 95 (compose, real CLI peers; run with `gpu-verify/e2e.sh` on a copy that holds only them).

## The terminal side is the real CLI

Enrollment: `sync-phone-enroll` (frames in and out as base64 text, SAS words, `y`/`n`; it admits the phone). A Red op:
`sync-notice <op-id>` prints the kind 3 notice, the phone approves, `sync-phone-ops` takes the phone's kind 5 text in.
Share release: `sync-share-request <host>` (kind 1 out, kind 2 in, one process). The page shows every enrollment frame as
text too (`#enroll-b64`, Copy button); the driver reads that text and checks it against the QR the page draws.
The helpers are in lib-creds.sh: `enroll_phone`, `phone_signs`, `red_complete`, `ring_with_phone`.

## Order that works (read from the ceremony rules)

A third factor signs only an op written after its registration, and registering it needs the phone. So the paper
registration is proposed first, the phone enrolls, and the phone and the second terminal complete the registration and
then the phone's own admission (phone, second device, paper). Only the proposing terminal sees the phone proofs count
(see FINDINGS.md, phone section).

## Driver commands

See the header of `phone-driver.mjs`. Enrollment: `enroll-start`, `enroll-commit`, `enroll-reveal`, `enroll-match`,
`enroll-finish`; Red notice: `red-open`, `red-approve`; share release: `share-open`, `share-approve`; checks:
`verify-ops`. `frame-notice` builds a kind 3 frame from op bytes and is used by phone-selftest.sh only.
