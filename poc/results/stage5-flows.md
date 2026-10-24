# Stage 5 flows: phone companion page

VERDICT: PASS in the tools container (headless Chromium, fake devices, Rust-generated fixtures). NOT run on a real phone: camera, microphone, speaker, clipboard sync and Android Chrome behaviour are unmeasured.

All numbers below come from one run of `assets/companion/scripts/verify-in-container.sh` (steps: `npm ci`, build twice and `cmp`, selftest build, node suites, headless vector replay, headless end to end, Rust verifier of the page's replies).

## Claims

End to end rows (`assets/companion/scripts/e2e.mjs`, 61 checks, 0 failed; criterion is the claim text itself):

| claim | criterion | measured | unit | PASS/FAIL |
|---|---|---|---|---|
| first run offers key creation and restore | as stated | sshub companion | - | PASS |
| a short PIN is refused | as stated | The PIN needs at least 6 characters. | - | PASS |
| 24 backup words shown once after key generation | as stated | 24 | words | PASS |
| Continue stays disabled until the words are confirmed written | as stated | True | - | PASS |
| the device id is derived from the backup words | as stated | 3db6aab47c937949 | - | PASS |
| IndexedDB holds only the PIN-wrapped secret (SSHP, no plaintext key bytes) | as stated | 101 | bytes | PASS |
| reload asks for the PIN; the right one unlocks the same device | as stated | 3db6aab47c937949 | - | PASS |
| hiding the page locks it | as stated | unlock screen shown | - | PASS |
| restore from 24 words gives the fixture phone identity | as stated | ee8a1d5cf97016a7 | - | PASS |
| enrollment step 2 shows the phone commitment (role 0, 32 bytes) | as stated | 1dbb95246f9f4eb5 | - | PASS |
| a terminal reveal that does not open its commitment is refused | as stated | The terminal reveal does not open the commitment it showed first. Enrollment stopped: someone is interfering. | - | PASS |
| the phone reveal opens the commitment it showed first | as stated | match | - | PASS |
| the four SAS words on the phone equal the words computed from both reveals | as stated | output-story-portion-obtain | - | PASS |
| the SAS words equal the Rust mirror when both reveals are the fixture ones | as stated | clump-quantum-clever-vague | - | PASS |
| the phone signs the transcript with its device key (confirm, role 0, ok) | as stated | signature verifies | - | PASS |
| enrollment completes only with the terminal signature; the ring then names the terminal | as stated | 1355b0f2cb05902f | - | PASS |
| "They differ" sends a signed decline and enrolls nothing | as stated | ok=false | - | PASS |
| declined enrollment leaves the ring empty | as stated | not enrolled | - | PASS |
| scan of the Rust challenge (80x24 frames, 8 fps loop) reaches the approval screen | as stated | 3775 | ms incl. unlock | PASS |
| the description names the requesting device | as stated | work laptop [1355b0f2cb05902f] | - | PASS |
| requester and credential are highlighted as dangerous | as stated | 2 | highlighted rows | PASS |
| the other rows are not highlighted | as stated | 4 | plain rows | PASS |
| the binding words equal the Rust binding hash words | as stated | fashion-fantasy-arm-final | - | PASS |
| every check on the screen passes | as stated | 6 | checks | PASS |
| a 16 character short code is shown after the release | as stated | G18J-T420-FZJ4-7Z89 | - | PASS |
| the reverse QR carries one exact ShareResponseWire (kind 2) | as stated | 711 | bytes | PASS |
| opened with the ephemeral secret the share is the phone share | as stated | share equal | - | PASS |
| the approval signature verifies over the binding preimage | as stated | verifies | - | PASS |
| the reply carries the phone chain head (courier) and the short-code blob | as stated | 1 op, snapshot 110 B | - | PASS |
| reverse QR looped at the QR frame rate | as stated | 4 frames read over a looping canvas | - | PASS |
| the clipboard holds the same response as base64 | as stated | 948 | base64 chars | PASS |
| the audio burst (fast) decodes from the fake microphone WAV with the PoC H decoder | as stated | 128 bytes | - | PASS |
| the audio reply is the share without courier and opens to the same share | as stated | equal | - | PASS |
| fast burst duration for the 128 byte reply | as stated | 8.16 | s | PASS |
| the manual entry shows index and 24 words | as stated | 24 | words | PASS |
| the short-code list shows the credential as live | as stated | 0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a | - | PASS |
| a second short code for a live credential is refused by the phone | as stated | checkbox disabled | - | PASS |
| an envelope (post-reshare) share unwraps and is described | as stated | envelope epoch 3 | - | PASS |
| the envelope reply opens to the phone share and carries no second snapshot | as stated | no snapshot | - | PASS |
| the phone chain continues across releases (courier ops seq 1, 2) | as stated | 1,2 | - | PASS |
| refused: wrong host_uid in the material | as stated | this phone cannot release for this request: the share inside the material belongs to a different credential | - | PASS |
| refused: wrong host_uid in an envelope | as stated | this phone cannot release for this request: the envelope share does not belong to this credential or epoch | - | PASS |
| refused: op bytes altered after signing | as stated | the signature of 1355b0f2cb05902f does not verify over these operation bytes | - | PASS |
| refused: flipped signature bit | as stated | the signature of 1355b0f2cb05902f does not verify over these operation bytes | - | PASS |
| refused: request id differs from the op | as stated | the operation asks for a different request id than the challenge carries | - | PASS |
| refused: requester differs from the signer | as stated | the operation is signed by 1355b0f2cb05902f but names d145c6f525b2e221 as the requester | - | PASS |
| refused: another op kind | as stated | this is a HeadAttest operation, not a share release; this phone releases shares only | - | PASS |
| refused: material wrapped to another device | as stated | this phone cannot release for this request: invalid tag | - | PASS |
| refused: expiry far in the future | as stated | challenge claims to live 3600 s, more than the 120 s a terminal issues | - | PASS |
| refused: expired challenge | as stated | challenge expired 5 s ago | - | PASS |
| refused: a good challenge seen after its expiry on the phone clock | as stated | challenge expired 380 s ago | - | PASS |
| a requester the phone never met is shown with a warning, not silently trusted | as stated | this phone has never been introduced to a098a5efa2e3bf52; its signature cannot be checked here | - | PASS |
| the recovery notice is described from its op bytes with the timelock end | as stated | The ring accepts the new phone after 1970-01-08T00:00Z unless it is cancelled. | - | PASS |
| the phone cancels the recovery with its own signed op, no factor needed | as stated | 1181 bytes, kind 5 | - | PASS |
| backup words need the PIN again and equal the stored identity words | as stated | 24 words | - | PASS |
| the page shows the fingerprint written by the build | as stated | 183a2f19ccc106507ee462a2d4216601050e24951b4091936498bdaa031d8d2f | - | PASS |
| the fingerprint is the sha256 of the file with the fingerprint zeroed | as stated | 183a2f19ccc106507ee462a2d4216601050e24951b4091936498bdaa031d8d2f | - | PASS |
| no request left the origin | as stated | 0 | external requests | PASS |
| every request went to the page origin | as stated | 20 | requests | PASS |
| no Content-Security-Policy violation was reported | as stated | 0 | violations | PASS |
| no uncaught page error | as stated | 0 | errors | PASS |
Other rows:

| claim | criterion | measured | unit | PASS/FAIL |
|---|---|---|---|---|
| Crypto vectors replayed by the node suite (`tests/vectors/crypto`) | all | 58 of 58 | vectors | PASS |
| Quorum vectors replayed by the node suite (`tests/vectors/quorum`) | all | 21 of 21 | vectors | PASS |
| Crypto vectors replayed inside headless Chromium (selftest build) | all | 58 of 58, 0 external requests | vectors | PASS |
| Node suite total (`test/phone.test.mjs`, `quorum.test.mjs`, `replay.test.js`) | 0 failed | 52 pass, 0 fail | tests | PASS |
| Every op vector of the op-log package parses strictly, verifies and is described | 35 of 35 | 35 of 35 | vectors | PASS |
| Phone encoders reproduce the canonical bytes of the HeadAttest and PhoneReenrollCancel vectors | byte equal | equal | bool | PASS |
| Build reproducible | two builds identical (`cmp`) | identical | bool | PASS |
| Committed html equals the container build | `cmp` | identical (sha256 of file 6a408764...0133) | bool | PASS |
| Page size | informational | 319006 | bytes | MEASURED |
| Rust opens what the page sent (`unwrap_share_reply` with the ephemeral secret, combine with the requester share and decrypt the credential, approval signature over the binding preimage, courier op signatures, snapshot blob opens with the shown code, recovery cancel op) | all assertions | 3 tests passed (logs phone-run8, phone-final) with the verifier reading both shares from `challenges.json`; on poc/quorum2 as merged the verifier still draws them from a fresh `shamir_split` and fails on that one comparison (log phone-merged), spec sent to the Rust owner | tests | PASS with the one-line fixture fix, FAIL as committed on quorum2 |
| Slow audio profile burst for the 128 byte reply | computed from 60 ms symbols | about 16 | s | COMPUTED (node test decodes both profiles with the PoC H decoder) |

## What each flow does

- First run: PIN (at least 6 characters) wraps a 32 byte secret with Argon2id (64 MiB x 3) as `SSHP`; 24 BIP39 words are that secret; restore and show-words (PIN again) are covered. The page locks when hidden.
- Enrollment over QR: terminal commit, phone commit, terminal reveal, phone reveal, four SAS words, signed confirm both ways (byte layouts of `enroll.rs`, role 0 is the phone). A reveal that does not open its commitment, and a declined SAS, enroll nothing.
- Release: the challenge is decoded with exact lengths, the op is parsed from its bytes (strict canonical check, no duplicate keys or escapes), the description is rendered from those bytes, the requester is named and highlighted with the credential, the binding hash words are shown, expiry and signature are checked, the material is unwrapped and its host_uid compared. Refusals are listed in the table. Approve fires reverse QR, clipboard and audio together and appends one HeadAttest op to the phone chain (courier, at most 8 ops per reply).
- Short codes: one live per credential enforced on the phone, the Argon2 profile and 7 day expiry recorded in the blob, a list with "declare dead".
- Recovery: a notice is described from its PhoneReenroll op with the timelock end; the phone cancels with its own signed op, no factor.

## Commands

- Whole run: `gpu-verify/tools.sh <worktree> <name> -- 'sh /work/assets/companion/scripts/verify-in-container.sh'`
- Fixtures: `SSHUB_EMIT_PHONE_FIXTURES=1 cargo test --lib sync::phone_fixtures` (Rust, owned by the stage 3 package since the hand-over).
- Logs: `gpu-verify/logs/phone-run8-*.log` (first all green), `phone-final-*.log`, `phone-merged-*.log` (with poc/quorum2 merged, Rust unpatched).
- Versions: node 22.23.4, Playwright 1.64.1, Chromium 156, cargo 1.99.1, esbuild 0.25.12, noble 2.4.0 family, hash-wasm 4.12.0, qrcode-generator 1.4.4, jsQR 1.4.0.

## Fingerprint

`assets/companion/FINGERPRINT` = sha256 of the file with its own 64 hex characters replaced by zeros. It detects a wrong or outdated build, not a malicious page (a substituted page can print any string).

## Residuals and NOT run

- NOT run: real phone camera and microphone, Android Chrome, real speaker to laptop microphone, handheld QR scanning (PoC C4/C5), clipboard sync services, `file://` camera behaviour (the harness serves the page from `http://localhost`).
- The enrollment e2e plays the terminal in JS with the Rust fixture keys; the terminal side of enrollment in Rust (camera input) is not part of this package.
- A short PIN is the weak link of the stored key whatever the Argon2 profile is; no WebAuthn PRF.
- The phone does not rotate wrap keys (C14): shares wrapped to an old key generation would need the same secret, which the 24 words give.
- The courier carries the last 8 ops; a longer gap needs the phone to touch another terminal.
- Slow Argon2 phone: the 256 MiB x 4 snapshot profile falls back to 64 MiB x 8, recorded in the blob and flagged; not exercised on a slow device.
- An unknown requester is warned about, not refused: the phone only learns a terminal's key at enrollment.
