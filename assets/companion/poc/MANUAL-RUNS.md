# Manual runs for PoC C and PoC H

Automated offline checks live in `run-offline.mjs`. The runs below need a phone, a laptop with a speaker and microphone, and a person to hold the phone. Record results in `poc/results/C-qr-budget.md` and `poc/results/H-share-channels.md`.

## Setup (Android over USB)

1. Enable developer options and USB debugging on the phone. Connect it to the laptop.
2. Forward the serving port:
   `adb reverse tcp:8443 tcp:8443`
3. Serve the PoC pages from the worktree root, bound to loopback:
   `python3 -m http.server --bind 127.0.0.1 8443 -d assets/companion/poc`
4. On the phone's browser, open `http://localhost:8443/qr-scanner.html?seed=1&len=4096`.
   Chrome treats `localhost` as a secure context, so the camera API is available.
5. On the laptop, open `http://localhost:8443/share-transmit.html?seed=1&len=56&version=10` for H (sender) and `share-receive.html` on the phone for H (receiver), or swap roles as the run needs.

Hold the phone steady, screen at about 15 cm from the QR, room light on. Note the lighting in the table.

## C4: handheld QR reading (5 runs, under 5 s each at 80x24)

- Terminal player on the laptop: `node assets/companion/poc/terminal-player.mjs --version 5 --bytes 4096 --seed 1 --fps 4` (use `--once` only for the self-check).
- Scanner on the phone: `qr-scanner.html?seed=1&len=4096`.
- Record: time from first frame to a matching sha256, coverage grid at the end, lighting, distance.

## C5: recovery after interruption

- Start the scanner, cover the screen for 3 s mid-run, uncover, and continue.
- Record: whether the transfer completes, how many extra loops it took, and whether the sha256 matches.

## H3: over-the-air audio success (10 attempts, under 5 s, at least 8 of 10)

- Laptop speaker: `share-transmit.html`, press Send audio burst, profile slow or fast.
- Phone: `share-receive.html`, press Arm mic, then Run trial, one trial per burst.
- Export the CSV from the receiver. Record distance, volume, room, and profile per run.

## H4: channel matrix (10 attempts per cell)

Cells: audio slow, audio fast, camera QR v10, each at two distances (near 20 cm, far 60 cm). Fill one row per cell with successes out of 10 and median time.

## Result table

| run | channel | profile or version | distance | lighting or room | attempts | successes | median ms | notes |
|-----|---------|--------------------|----------|------------------|----------|-----------|-----------|-------|

Note the Android version, browser version (from the browser's About page), and laptop audio output device in the results file.
