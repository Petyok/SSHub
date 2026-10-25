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
- verified on iOS or Safari. It is **unverified there**: it has never been run on an iPhone. It has run on desktop
  Chromium and, for the page flows, on one Android phone with Chrome (see Known limits). Do not rely on it on iOS
  before you have run the checks below yourself.

### The fingerprint, and its limit

The page shows a 64 character fingerprint (menu: About and fingerprint), and the terminal shows the one it expects.
It is the sha256 of the file with its own 64 hex characters replaced by zeros, because a file cannot contain its own
hash. To recompute it: replace the fingerprint inside the file with 64 zeros and run `sha256sum` on the result.

It detects a wrong or outdated build. It does not detect a malicious page: a substituted page can print any string.
Get the file from a source you trust (this repository, or the file the terminal ships) and compare against the
fingerprint printed by your terminal, not only the one on the phone.

The build is reproducible: building twice in the tools container gives the same bytes, and the committed file equals
that build (`assets/companion/scripts/verify-in-container.sh` runs the check; the fingerprint is written to
`assets/companion/FINGERPRINT`). So the fingerprint also tells you that the file is the one this repository builds
from its source, which anyone can rebuild and compare.

## Getting the page onto a phone

The camera needs a secure context, so the page must be opened from `https://` or from `http://localhost`.

### Testing over USB (Android)

On the computer that has the repository, with USB debugging enabled on the phone:

```
adb devices -l
adb reverse tcp:8443 tcp:8443
python3 -m http.server --bind 127.0.0.1 8443 -d assets/companion
```

Then open `http://localhost:8443/sshub-companion.html` in Chrome on the phone. `localhost` on the phone is the
computer through the reverse tunnel, and counts as a secure context. When you are done: stop the server, run
`adb reverse --remove tcp:8443`, clear the site data of `http://localhost:8443` in Chrome (it holds the test key)
and take back any camera permission you granted. The page itself makes no requests after it has loaded, so you can
pull the cable afterwards, but the browser may not keep the page across a restart: use the next section for daily
use.

To drive the page from the computer (as the test scripts in `assets/companion/scripts/` do), forward Chrome's DevTools
socket with `adb forward tcp:9222 localabstract:chrome_devtools_remote` (remove it with `adb forward --remove
tcp:9222`) and work in one tab of your own; do not touch other tabs.

Camera lessons from the real phone:

- Chrome on Android asks for the site's camera permission through a small collapsed chip in the address bar, not a
  dialog. Until you tap it and choose Allow, the page's camera request just stays pending with nothing on the screen.
- Chrome also needs Android's own camera permission (system settings, the app's permissions). After you grant it,
  **restart Chrome** (swipe it out of the recent apps and open it again): until then the request keeps waiting.
- Use one tab and one camera request at a time. A second page asking for the same camera froze the image in the
  tests.
- A browser tab hidden or a screen turned off locks the page; keep the screen on while you test.

### Daily use: self-host over HTTPS

Serve the one file from any static host you control over HTTPS (a personal web server, object storage with a
certificate, a Git hosting pages feature). No headers are required: the policy is inside the file. Pin the version:
publish a specific build, check its fingerprint against your terminal, and update only when you decide to. Add the
page to the home screen if your browser offers it. The keys live in the browser's own storage for that origin:
clearing site data deletes the key (see Backup below), and serving the page from two different origins creates two
different phones.

## First run

1. Open the page. Choose a PIN or passphrase of at least 10 characters (twice) and tap **Create a new key**.
2. The page shows 24 backup words. They are the key itself: anyone with them can act as this phone, and with them
   you can restore it elsewhere. Write them on paper, in order. Do not photograph them. Tick the box and continue.
3. The home screen shows this phone's device id and whether it is enrolled.

The PIN protects the key in the browser's storage with Argon2id (64 MiB, 3 passes). A short PIN is the weak point
of a stolen phone whatever the profile is, so the page asks for 10 characters; use a longer passphrase if the phone may
be lost. The page locks when it is hidden (a tab switch or screen off), wipes every key from memory and asks for the
PIN again. After two wrong PINs in a row each further wrong try makes the page wait (5 s, doubling, at most an hour),
and the wait survives a reload. Someone who can edit the browser's storage can reset that counter: it protects
against a person holding the unlocked-looking screen, not against a copy of the storage, where only the length of the
PIN and the Argon2id cost count.

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

Each frame the phone shows during enrollment is also offered as text under **Show as text for a terminal without a camera**
(the same bytes as the QR code, base64, with a Copy button): a terminal that has no camera takes the phone's frames by paste.

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
- the **age of the signed request**: the terminal signs the request right before it shows the challenge, so the page
  refuses a request signed more than 12 minutes ago (120 s plus 600 s of clock skew), one dated in the future, and a
  challenge whose expiry is more than 12 minutes after the signature (an old signed request wrapped in a fresh
  challenge); a request older than 2 minutes that is still accepted has its time row highlighted;
- a countdown: a request lives about 120 seconds.

A requester this phone has never met cannot be checked: the page refuses to enable **Approve** until you have typed
the requester's device id (shown in the warning) into the box. Approve such a request only if you started it yourself
on a terminal and its id is the one printed there.

Options before you approve: show a QR code, copy to the clipboard, play a sound (fast or slow), and optionally
issue a short code. Tap **Approve and release**. The phone then fires every channel you left on:

- **QR code**: hold the screen in front of the laptop camera with the brightness at maximum. The code is as wide as
  the screen, drawn with whole device pixels per module (version 6 frames of 106 bytes at 4 frames per second, with
  25% parity), and loops until you leave the screen; a reply of 711 bytes is 10 frames, one lap 2.5 seconds. In a test
  with a fixed-focus laptop webcam a code like this was read at about 13 to 25 cm. This is the primary channel.
- **Clipboard**: the same reply as base64, useful when the clipboard is shared with the computer.
- **Sound**: a burst for the laptop microphone. The fast profile takes about 8 seconds for the reply and is the
  default; the slow profile (about 16 seconds) is for noisy rooms. Audio is opt-in on the terminal and unmeasured
  over the air.
- **By hand**: expand "Type it by hand" for the share index and 24 words to type into the terminal. Look around
  first: anyone who sees them has this share.

Every approval also appends one signed entry to the phone's own chain and carries it in the reply (the terminal
delivers it to the ring). This is what shows the ring that the phone is alive. The phone copies the logical clock and
epoch of a request into that entry only when it verified the requester's signature, never from a requester it has not
met, and it refuses to sign when a counter would leave the 64 bit range.

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
while one is live. The Argon2id profile, the expiry and the credential's id (`host_uid`) are recorded inside the blob
(header version 2, the id is part of the authenticated header), so a terminal can tell which credential a blob opens
and refuse a second live blob for it; a phone too slow for the normal profile falls back to a weaker one, which the
terminal flags.

Menu **Short codes I issued** lists them. If the sealed blob never reached the terminal, or the credential was
reissued, tap **Declare it dead** so a new code can be issued; the old blob is still valid until it expires if it
did arrive, so only do this when you know it did not or the credential changed.

## Recovery: cancelling a recovery you did not start

If someone starts replacing your phone (it shows on every device for seven days), a terminal can show a recovery
notice QR. On the phone open **Recovery and a new phone**, scan it, and read the description (it shows when the
timelock ends). Tap **Cancel this recovery: I still have this phone**. A phone that can still sign proves the
recovery is an attack, so no other factor is needed. The phone signs a cancellation and shows it as a QR code and on
the clipboard: hold it to any terminal. Only cancel a recovery you did not start.

## Red operations: the phone's proof

Some changes are Red: adding or promoting a holder, revoking a holder, a trust certificate, a mode change, a third
factor, a role change, a bulk export, an exemption. A Red change takes effect only with three proofs: this phone, a
second attended terminal and a third factor (paper or email code). The terminal shows a Red notice as a QR code.
On the phone tap **Scan a share request** and scan it: the page describes the operation from its signed bytes
(every row of a Red operation is highlighted), names the proposer, and shows the hash of the operation. **Approve:
sign this operation with the phone** adds a signed proof that names that exact hash and shows it as a QR code and as
text for the terminal. The page refuses a notice older than 3 days, a notice that is not a Red operation, and a
trust certificate written by another device (the phone writes its own, below). Your signature is only the phone's
part: the other two proofs come from terminals.

## Trust sessions

A trust session lets one terminal open listed hosts without asking this phone each time, for a limited term. The
phone is the author and signer of the certificate, so a stolen terminal cannot issue one. Menu **Trust sessions**
(or **Scan a share request**) takes two kinds of request from a terminal:

- **Request for a trust session**: the terminal shows the device, the hosts (or one group), the term, the budget of
  new hosts per day and the epoch range. The phone shows all of it from the bytes (the device by name, or "not met"
  with a warning, every listed host, the term in days with its end date, the budget) and, on **Approve**, signs the
  certificate with exactly those fields. It refuses, with the reason and no Approve button: a term that is over or
  longer than 90 days, a budget outside 1 to 50, more than 64 hosts or none, an inverted epoch range, an epoch range that
  does not start at this phone's own epoch or is more than one epoch wide (a terminal asks for the phone's epoch plus
  one, so a certificate ends with the next reshare; the epoch row is highlighted), and a request that names this
  phone. The certificate takes effect only after a second terminal and a third factor approve it;
  carry the signed answer (QR or text) to the terminal.
- **Request to extend the budget**: when a certificate has used its new hosts of the day, the terminal shows a
  request for 1 to 50 more. The phone refuses a certificate it did not issue, a revoked or expired one, and a second
  extension of the same certificate within 24 hours (it says when the next one is possible). An extension covers its
  own 24 hours only and never changes scope or term.

The same menu lists the certificates this phone issued (active, expired or revoked, with extensions) and a **Revoke
this trust session** button for each active one. Carry the signed revocation to any terminal; peers stop honouring the
certificate on their next sync.

## Reshare and wrap keys

After a holder is revoked the ring reshares. The phone is only a receiver: it never contributes a sub-share and
keeps no old shares. Menu **Reshare and wrap keys**: **Rotate wrap keys** first (its old generations stay readable,
they derive from the same 24 words), then **Adopt a reshare** and scan or paste the reshare ops a terminal shows for
the phone. The page shows the epoch, the holder number, how many credentials are ready and the contributors (with
"signature not checked" for contributors it has never met), and **Publish the new shares** adds the phone's new
shares, sealed, to its chain for a terminal to take. The screen states the consequence of this design: a revocation
needs at least two other terminal holders to contribute, so a ring of two terminals plus the phone cannot revoke a
terminal by resharing (the paper phrase is offline and cannot contribute either); add a third terminal first.

## Handing ops to a terminal without a camera

Every screen that produces signed operations shows a QR code and the same bytes as text with a **Copy the text**
button (it is also copied automatically where the browser allows it): paste it into the terminal. **Export my ops as
text** in the menu does the same for the newest 8 entries of the phone's chain.

## Backup and restore

- **Show backup words** in the menu needs the PIN again and shows the 24 words. They are the same words as at first
  run.
- **Restore** on a fresh page: enter the 24 words and choose a new PIN. The restored phone is the same device as
  before (same id, same keys), so no recovery and no revocation are needed, but wipe the old phone. **A restored phone
  does not sign anything yet**: it has lost the record of its own chain, and a new entry with a sequence number the ring
  already holds would look like a fork, which the ring treats as proof that the phone is compromised. The home screen
  shows a red card until the phone has taken its chain: ask a terminal for this phone's own newest ops (a list the
  terminal can show as a QR code or as text), tap **Take my chain from a terminal** and scan or paste it. The page checks that every op is by this key,
  verifies each signature and that they form one chain, and continues after the newest. Only when this key has never
  signed anything (a brand new key restored from its words) tap **This key never signed anything**. On the lock
  screen, "Restore from 24 words instead" replaces the stored key and erases the enrollments and short-code list of
  the old key from this browser, with the same rule.
- Clearing site data, uninstalling the browser or switching origin deletes the stored key: you then need the words.

## If the phone is lost

- If you have the 24 words: restore them on a new phone and take the phone's own chain from a terminal before it signs
  anything (see Backup and restore). No recovery is needed, but anyone who holds the lost phone
  and knows or guesses the PIN can still approve requests, so consider the lost phone compromised until you have
  revoked it from a terminal.
- If you have lost both the phone and the words: on a terminal start phone recovery (two other attended devices or
  the paper phrase, plus a third factor and a seven day timelock; see the design chapter "Phone loss"). When it
  completes, enroll the new phone as above; the ring reissues shares to it and clears short codes.
- While there is no phone, the second share comes from your terminals (if you have at least two) or from paper.

## Known limits

- **iOS and Safari: unverified.** The page has never been run on an iPhone or in Safari (camera on a local page, Web
  Audio autoplay and storage persistence are unknown there). Do not rely on it before you have run the checks
  yourself.
- On Android Chrome (one Samsung phone, Android 16, Chrome 155) the page flows ran for real (first run, PIN,
  restore, release, short code, Red notice, export), driven from a computer with the camera, sound and clipboard
  switched off, and the code reading and the phone-screen-to-webcam reply were measured with test pages. Enrollment
  by camera, sound and clipboard of the real page, and the real page's reply QR read by a webcam, have not run on a
  phone yet.
- The fast audio reply is 8 seconds for 128 bytes and has not been measured over the air; the QR code is the primary
  channel.
- A requester the phone has not met is warned about and can be approved only after its device id is typed in.
- A document of the wrong shape (a broken op, a list that is not JSON, a number outside the 64 bit fields) ends on a
  refusal screen; the parser has a nesting and size limit (256 KiB per op) and never loops on unterminated input.
- **The page is inside a meta Content-Security-Policy, which cannot set `frame-ancestors`**: a page that frames this
  one is not stopped by the policy, and the policy needs `wasm-unsafe-eval` for the Argon2 code. Both are residual
  risks of a single offline file served without headers. The page has no network access in either case (`default-src
  'none'`, `connect-src 'none'`). Serve it from an origin you control, and over HTTPS, if you can add the headers
  `frame-ancestors 'none'` there.
- The fingerprint check needs the terminal to print the expected fingerprint; until the terminal does, compare with
  the repository file.
- The phone can rotate its wrap keys (Reshare and wrap keys), but its older generations derive from the same 24 words
  and can never be retired, so the forward secrecy of the reshare does not extend to the phone's own envelopes.
- The phone's chain carries the last 8 entries with each reply; a long gap needs the phone to touch another terminal.
  A list carries at most 8 ops, 256 KiB per op and 1 MiB in all.
- A trust session stops working when no terminal has seen this phone for 14 days.
- A ring of two terminals plus the phone cannot revoke a terminal by resharing: the phone never contributes.
- The fingerprint does not defend against a malicious page.
