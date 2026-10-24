# Phone companion

`assets/companion/sshub-companion.html` is a single offline web page that turns a phone into a share holder of your
SSHub ring. Read `docs/host-sync-design.md` first for what a holder is; this guide is about the page.

## What it is and is not

It is:

- one HTML file with no network access of any kind (a strict Content-Security-Policy pins its one script and one
  style by hash, and blocks every request), so it works with the phone in airplane mode;
- a holder of its own key: an Ed25519, X25519 and ML-KEM identity stored in the browser, wrapped under your PIN;
- a way to approve a credential release: the terminal shows an animated QR code, the phone shows what is being
  asked and who asks, you approve, and the answer travels back by QR code, clipboard or sound.

It is not:

- a sync peer: it has no SSH and no copy of your hosts (it stores no credential keys, the terminal sends it only
  its own wrapped share inside each request);
- a password manager or a place to read passwords;
- verified on iOS or Safari. Only desktop Chromium has been run against it; Android Chrome and any iPhone browser
  are unverified. Do not rely on it there before you have run the checks below yourself.

### The fingerprint, and its limit

The page shows a 64 character fingerprint (menu: About and fingerprint), and the terminal shows the one it expects.
It is the sha256 of the file with its own 64 hex characters replaced by zeros, because a file cannot contain its own
hash. To recompute it: replace the fingerprint inside the file with 64 zeros and run `sha256sum` on the result.

It detects a wrong or outdated build. It does not detect a malicious page: a substituted page can print any string.
Get the file from a source you trust (this repository, or the file the terminal ships) and compare against the
fingerprint printed by your terminal, not only the one on the phone.

## Getting the page onto a phone

The camera needs a secure context, so the page must be opened from `https://` or from `http://localhost`.

### Testing over USB (Android)

On the computer that has the repository, with USB debugging enabled on the phone:

```
adb reverse tcp:8443 tcp:8443
python3 -m http.server --bind 127.0.0.1 8443 -d assets/companion
```

Then open `http://localhost:8443/sshub-companion.html` in Chrome on the phone. `localhost` on the phone is the
computer through the reverse tunnel, and counts as a secure context. Stop the server when you are done. The page
itself makes no requests after it has loaded, so you can pull the cable afterwards, but the browser may not keep the
page across a restart: use the next section for daily use.

### Daily use: self-host over HTTPS

Serve the one file from any static host you control over HTTPS (a personal web server, object storage with a
certificate, a Git hosting pages feature). No headers are required: the policy is inside the file. Pin the version:
publish a specific build, check its fingerprint against your terminal, and update only when you decide to. Add the
page to the home screen if your browser offers it. The keys live in the browser's own storage for that origin:
clearing site data deletes the key (see Backup below), and serving the page from two different origins creates two
different phones.

## First run

1. Open the page. Choose a PIN or passphrase of at least 6 characters (twice) and tap **Create a new key**.
2. The page shows 24 backup words. They are the key itself: anyone with them can act as this phone, and with them
   you can restore it elsewhere. Write them on paper, in order. Do not photograph them. Tick the box and continue.
3. The home screen shows this phone's device id and whether it is enrolled.

The PIN protects the key in the browser's storage with Argon2id (64 MiB, 3 passes). A short PIN is the weak point
of a stolen phone whatever the profile is; use a longer passphrase if the phone may be lost. The page locks when it
is hidden (a tab switch or screen off) and asks for the PIN again.

## Enrollment

Enrolling introduces the phone to a terminal of your ring. It uses the same commit, reveal and four-word check as
machine enrollment, over QR codes instead of SSH. Here both sides always have a screen, so the four words are a real
check (unlike a headless machine reached over ssh, where the words prove nothing).

1. On the terminal start phone enrollment. It shows a short QR code.
2. On the phone tap **Enroll with a terminal**, name the phone, and scan the code (step 1 of 4).
3. The phone shows its own commitment as a QR code: hold it to the terminal camera, then tap Next.
4. Scan the terminal's second QR code (its keys). The phone checks they match the first code; if they do not, it
   stops and says so.
5. Both screens now show four words. If they are identical, tap **The words match**; if not, tap **They differ**
   (someone sits between you). Hold the phone's confirmation QR to the terminal camera, then scan the terminal's
   confirmation.

Only after the terminal's signed confirmation does the phone record the terminal. Declining or a bad signature
enrolls nothing. The terminal then adds the phone to the ring (promotion to holder is a separate step on the
terminal).

## Approving a release

On the terminal's unlock screen, tap **Scan a share request** on the phone and point the camera at the QR code. The
animated code is read over a few seconds; a missed frame comes round again.

The approval screen is built from the signed operation bytes, not from any text the terminal supplies:

- the headline and rows name the **requesting device** (by id, and by name if the phone knows it) and the
  **credential** being released; both are highlighted because they are what you are deciding;
- the **binding words** are four words computed from everything that is signed: the operation, the nonce, the
  expiry, the terminal's one-time key and the material sent to the phone. Compare them with the words the terminal
  shows when it shows them; a mismatch means the request was altered on the way;
- a list of checks (expiry, request and requester agree, signature, own share unwrapped for this credential);
- a countdown: a request lives about 120 seconds.

Options before you approve: show a QR code, copy to the clipboard, play a sound (fast or slow), and optionally
issue a short code. Tap **Approve and release**. The phone then fires every channel you left on:

- **QR code**: hold it in front of the laptop camera. It loops until you leave the screen. This is the primary
  channel.
- **Clipboard**: the same reply as base64, useful when the clipboard is shared with the computer.
- **Sound**: a burst for the laptop microphone. The fast profile takes about 8 seconds for the reply and is the
  default; the slow profile (about 16 seconds) is for noisy rooms. Audio is opt-in on the terminal and unmeasured
  over the air.
- **By hand**: expand "Type it by hand" for the share index and 24 words to type into the terminal. Look around
  first: anyone who sees them has this share.

Every approval also appends one signed entry to the phone's own chain and carries it in the reply (the terminal
delivers it to the ring). This is what shows the ring that the phone is alive.

### Refusals

The page refuses, with a reason, and offers no Approve button:

| Message (shortened) | Meaning |
|---|---|
| challenge is malformed | the QR content is not a valid request, or is damaged |
| challenge expired N s ago | the request is older than its expiry on this phone's clock; ask the terminal again, and check the phone's clock |
| challenge claims to live N s | the expiry is much longer than any terminal issues |
| this is a X operation, not a share release | the request is for something this phone does not release |
| asks for a different request id | the operation and the request disagree |
| signed by A but names B as the requester | the device that signed is not the one named as asking |
| the signature of A does not verify | the operation was altered after it was signed, or the signature is wrong |
| the share inside the material belongs to a different credential | the terminal paired the description of one credential with the share of another |
| this phone cannot release for this request | the material was not made for this phone's key, or the share does not belong to this credential |

If the requesting device is not one the phone met at enrollment, the page does not refuse: it warns that it cannot
check that signature. Check the requester's id and name yourself.

## Short codes

When you approve you can also issue a short code: 16 characters (for example `K7Q2-9FMX-C41T-8PWD`) that let
the terminal open this credential later without the phone for 7 days. The code exists only on the phone screen;
write it down or type it on the terminal. The phone issues **one live code per credential** and refuses a second
while one is live. The Argon2id profile and the expiry are recorded inside the blob; a phone too slow for the
normal profile falls back to a weaker one, which the terminal flags.

Menu **Short codes I issued** lists them. If the sealed blob never reached the terminal, or the credential was
reissued, tap **Declare it dead** so a new code can be issued; the old blob is still valid until it expires if it
did arrive, so only do this when you know it did not or the credential changed.

## Recovery: cancelling a recovery you did not start

If someone starts replacing your phone (it shows on every device for seven days), a terminal can show a recovery
notice QR. On the phone open **Recovery and a new phone**, scan it, and read the description (it shows when the
timelock ends). Tap **Cancel this recovery: I still have this phone**. A phone that can still sign proves the
recovery is an attack, so no other factor is needed. The phone signs a cancellation and shows it as a QR code and on
the clipboard: hold it to any terminal. Only cancel a recovery you did not start.

## Backup and restore

- **Show backup words** in the menu needs the PIN again and shows the 24 words. They are the same words as at first
  run.
- **Restore** on a fresh page: enter the 24 words and choose a new PIN. The restored phone is the same device as
  before (same id, same keys): no recovery and no revocation are needed, but wipe the old phone. On the lock screen,
  "Restore from 24 words instead" replaces the stored key and erases the enrollments and short-code list of the old
  key from this browser.
- Clearing site data, uninstalling the browser or switching origin deletes the stored key: you then need the words.

## If the phone is lost

- If you have the 24 words: restore them on a new phone. Nothing else is needed, but anyone who holds the lost phone
  and knows or guesses the PIN can still approve requests, so consider the lost phone compromised until you have
  revoked it from a terminal.
- If you have lost both the phone and the words: on a terminal start phone recovery (two other attended devices or
  the paper phrase, plus a third factor and a seven day timelock; see the design chapter "Phone loss"). When it
  completes, enroll the new phone as above; the ring reissues shares to it and clears short codes.
- While there is no phone, the second share comes from your terminals (if you have at least two) or from paper.

## Known limits

- Unverified on iOS and Safari, and not yet run on a physical Android phone: camera reading, sound and clipboard are
  tested only with simulated devices in headless Chromium.
- The fast audio reply is 8 seconds for 128 bytes; the QR code is the primary channel.
- A requester the phone has not met is warned about, not refused.
- The phone does not rotate its wrap keys; its shares stay readable with the same 24 words (see the residual risk
  list in the design for what that means after a revocation).
- The phone's chain carries the last 8 entries with each reply; a long gap needs the phone to touch another terminal.
- The fingerprint does not defend against a malicious page.
