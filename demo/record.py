#!/usr/bin/env python3
"""Record the README GIFs and screenshots by capturing the PTY byte stream.

Why not VHS (which the tapes under demo/tapes/ used to drive): VHS renders the
session in a headless browser and *screenshots* it on a timer, so a slide only
survives if a screenshot happens to land in the middle of it. SSHub redraws at
~60fps while animating, which that pipeline cannot keep up with; its capture rate
collapses and, because the video is written at a constant frame rate, the frames
it missed come out as a sped-up recording rather than a choppy one. Measured on
the navigate tape with motion on: 0.28x real time, every transition down to three
frames. Lowering the rate, raising it, and stretching the animations all moved the
number around without fixing the mechanism.

This records what the program actually wrote, tagged with the moment it wrote it
(asciicast v2), so nothing is sampled and the timeline is real by construction.
`agg` renders that to a GIF, honouring those timestamps: idle stretches become one
long frame, a slide becomes a run of 20-40ms ones, and the same recording is a
third of the size.

Requires `agg` (cargo install --git https://github.com/asciinema/agg) and ffmpeg.

Usage:
    demo/record.py                  # everything
    demo/record.py hero navigate    # only these
"""
from __future__ import annotations

import fcntl
import json
import os
import pty
import select
import shutil
import signal
import struct
import subprocess
import sys
import termios
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BUILD = ROOT / "demo" / "build"
GIFS = ROOT / "demo" / "gifs"
SHOTS = ROOT / "demo" / "screenshots"
# Recordings run under a throwaway home, not yours. The SFTP browser puts the
# local pane's path on screen, so a real `$HOME` would publish your username and
# directory layout in the README — it already did, in the GIFs this replaces.
DEMO_HOME = Path("/tmp/sshub-demo-home")

COLS, ROWS = 150, 40
# The "Mono" variant, or agg warns that the cell metrics are off.
FONT = "JetBrainsMono Nerd Font Mono"
FONT_SIZE = 16
# Catppuccin Mocha: background, foreground, then the 16 ANSI slots — the palette
# the tapes asked xterm.js for, so the recordings keep the look they had.
THEME = ",".join([
    "1e1e2e", "cdd6f4",
    "45475a", "f38ba8", "a6e3a1", "f9e2af", "89b4fa", "f5c2e7", "94e2d5", "bac2de",
    "585b70", "f38ba8", "a6e3a1", "f9e2af", "89b4fa", "f5c2e7", "94e2d5", "a6adc8",
])
FPS_CAP = 50
TAIL = 1.5      # keep recording this long after the last keystroke
TYPING = 0.04   # seconds per character, matching the tapes' `Set TypingSpeed 40ms`

RESYNC = "resync"   # scheduled action: force a full repaint (see Tape.begin)

KEYS = {
    "enter": b"\r", "esc": b"\x1b", "tab": b"\t", "space": b" ", "backspace": b"\x7f",
    "up": b"\x1b[A", "down": b"\x1b[B", "right": b"\x1b[C", "left": b"\x1b[D",
    "pgup": b"\x1b[5~", "pgdn": b"\x1b[6~",
    "ctrl+a": b"\x01", "ctrl+d": b"\x04", "ctrl+f": b"\x06", "ctrl+h": b"\x08",
    "ctrl+k": b"\x0b", "ctrl+r": b"\x12", "ctrl+s": b"\x13", "ctrl+t": b"\x14",
    "ctrl+w": b"\x17", "ctrl+y": b"\x19",
    # ESC prefix is how a terminal spells Alt.
    "ctrl+alt+n": b"\x1b\x0e",
}


class Tape:
    """A keystroke timeline. Reads like the .tape files it replaces, minus the
    shell: the binary is started directly with its environment already set, so
    there is nothing to type into a prompt and nothing to `Wait` for."""

    def __init__(self) -> None:
        self.t = 0.0
        self.script: list[tuple[float, object]] = []
        self.stills: list[tuple[str, float]] = []
        self.start: float | None = None

    def sleep(self, seconds: float) -> "Tape":
        self.t += seconds
        return self

    def key(self, name: str, times: int = 1, gap: float = 0.0) -> "Tape":
        for i in range(times):
            if i:
                self.t += gap
            self.script.append((self.t, KEYS[name]))
        return self

    def type(self, text: str, cps: float = TYPING) -> "Tape":
        for ch in text:
            self.script.append((self.t, ch.encode()))
            self.t += cps
        return self

    def begin(self) -> "Tape":
        """Start the visible recording here — the old tapes' `Hide` / `Show`.

        Trimming a byte-stream recording is not simply dropping the earlier
        events: ratatui writes cell diffs, so the survivors would paint onto a
        screen that was never drawn — the logo and the header counters would be
        missing or half-there for the whole GIF. Wiggling the terminal size makes
        the app repaint in full (`src/lib.rs` resizes on any size change), and the
        trim point is then measured at the moment the width is restored, so that
        full repaint is the first thing the recording contains.
        """
        self.script.append((self.t, RESYNC))
        self.t += 0.35
        self.start = self.t          # placeholder; record() measures the real one
        return self

    def still(self, name: str) -> "Tape":
        """Keep this moment as a screenshot. Extracted from the rendered GIF, so
        it carries the GIF's 256-colour palette — for a terminal that is a few
        dozen colours wide, that is lossless in practice."""
        self.stills.append((name, self.t))
        return self


def record(tape: Tape, env: dict[str, str], cast: Path) -> float:
    """Run sshub under a PTY, feed it `tape`, write an asciicast. Returns the
    duration of the visible part."""
    master, slave = pty.openpty()

    def winsize(cols: int) -> None:
        fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, cols, 0, 0))

    winsize(COLS)
    proc = subprocess.Popen(
        [str(ROOT / "target" / "release" / "sshub")],
        stdin=slave, stdout=slave, stderr=slave, env=env,
        close_fds=True, preexec_fn=os.setsid, cwd=env.get("SSHUB_CWD", str(ROOT)),
    )
    os.close(slave)

    script = sorted(tape.script, key=lambda item: item[0])
    t0 = time.monotonic()
    trim_at: float | None = None
    events: list[list] = []
    step = 0
    while True:
        now = time.monotonic() - t0
        while step < len(script) and now >= script[step][0]:
            action = script[step][1]
            if action is RESYNC:
                # Narrow, let that repaint land and be discarded, then restore:
                # everything from here on is a complete screen.
                winsize(COLS - 1)
                time.sleep(0.12)
                winsize(COLS)
                trim_at = time.monotonic() - t0
            else:
                os.write(master, action)
            step += 1
        # Poll tightly while keys remain, so their timing holds.
        ready, _, _ = select.select([master], [], [], 0.005 if step < len(script) else 0.05)
        if ready:
            try:
                data = os.read(master, 65536)
            except OSError:
                break
            if not data:
                break
            events.append([round(time.monotonic() - t0, 6), "o",
                           data.decode("utf-8", "replace")])
        if step >= len(script) and (
                proc.poll() is not None or time.monotonic() - t0 > script[-1][0] + TAIL):
            break

    if proc.poll() is None:
        os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
    os.close(master)
    proc.wait()

    if trim_at is not None:
        events = [[round(t - trim_at, 6), kind, data]
                  for t, kind, data in events if t >= trim_at]
    header = {"version": 2, "width": COLS, "height": ROWS,
              "timestamp": int(time.time()),
              "env": {"TERM": "xterm-256color", "SHELL": "/bin/sh"}}
    cast.write_text("\n".join([json.dumps(header)] + [json.dumps(e) for e in events]) + "\n")
    return events[-1][0] if events else 0.0


def render(cast: Path, gif: Path) -> None:
    subprocess.run([
        "agg", "--font-family", FONT, "--font-size", str(FONT_SIZE),
        "--theme", THEME, "--fps-cap", str(FPS_CAP),
        "--last-frame-duration", "2", str(cast), str(gif),
    ], check=True, stderr=subprocess.DEVNULL)


def grab_stills(gif: Path, stills: list[tuple[str, float]], offset: float) -> None:
    SHOTS.mkdir(parents=True, exist_ok=True)
    for name, at in stills:
        subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-ss", f"{max(at - offset, 0):.2f}",
                        "-i", str(gif), "-frames:v", "1", str(SHOTS / f"{name}.png")],
                       check=True)
        print(f"    {SHOTS.name}/{name}.png")


def env_for(slug: str, cwd: Path | None = None) -> dict[str, str]:
    """A pristine config per scenario, so one run cannot leak settings into
    another.

    Motion is deliberately left ON. `disable_animation` is the reduced-motion
    toggle: it switches off every slide these recordings exist to show. The old
    tapes used it to skip the intro animation, which is why four of the five GIFs
    had no motion in them at all — the intro exits on a keypress, so skip it that
    way instead.
    """
    cfg = Path(f"/tmp/sshub-demo-{slug}")
    if cfg.exists():
        shutil.rmtree(cfg)
    shutil.copytree(ROOT / "demo" / "home" / ".config" / "sshub", cfg)
    # Fresh throwaway home: keeps your username out of frame, and gives the SFTP
    # scenario a pristine local folder (it downloads into it).
    if DEMO_HOME.exists():
        shutil.rmtree(DEMO_HOME)
    shutil.copytree(ROOT / "demo" / "home" / "sftp-local", DEMO_HOME / "sftp-local")
    (DEMO_HOME / ".ssh").mkdir(mode=0o700, parents=True, exist_ok=True)
    env = dict(os.environ)
    env.update({
        "HOME": str(DEMO_HOME),
        "TERM": "xterm-256color",
        "SSHUB_VERSION_LABEL": "",
        "SSHUB_CONFIG_DIR": str(cfg),
        "SSHUB_DATA_DIR": str(ROOT / "demo" / "home" / ".local" / "share" / "sshub"),
        "SSHUB_SSH_CONFIG": str(ROOT / "demo" / "home" / "ssh_config"),
        "PATH": f"{ROOT}/demo/bin:{ROOT}/target/release:" + os.environ.get("PATH", ""),
    })
    if cwd:
        env["SSHUB_CWD"] = str(cwd)
    return env


def opening(t: Tape) -> Tape:
    """Dismiss the intro animation, let the dashboard settle, start recording."""
    return t.sleep(0.9).key("enter").sleep(1.2).begin()


# ── Scenarios: faithful ports of the tapes they replace ────────────────────────

def hero() -> Tape:
    t = Tape()
    # Open on the intro's settled final frame (logo + tagline + blinking prompt).
    t.sleep(11.5).begin().sleep(2.5).key("enter").sleep(1.2)
    t.key("down", times=3, gap=0.7).sleep(1.0)
    t.type("/").sleep(0.6).type("google").sleep(0.9).key("enter").sleep(3.0)
    t.type("uptime").sleep(0.4).key("enter").sleep(1.8)
    t.key("ctrl+d").sleep(3.0)
    return t


def navigate() -> Tape:
    t = opening(Tape()).sleep(1.5)
    # Walk into the nested groups.
    t.key("down", times=4, gap=0.7).sleep(1.0)
    # Collapse the group, then expand it again. Space folds; Enter would connect.
    t.key("space").sleep(1.6).key("space").sleep(1.6)
    # Fuzzy host palette.
    t.type("/").sleep(0.8).type("quad", cps=0.08).sleep(1.6).key("esc").sleep(0.8)
    # Group manager overlay (nested groups).
    t.type("G").sleep(1.6).key("down").sleep(0.8).key("down").sleep(0.8).key("esc").sleep(1.0)
    # Multi-tag filter: # opens the picker, Space toggles, Enter applies.
    t.type("#").sleep(1.2)
    t.key("down").sleep(0.7).key("space").sleep(0.8)
    t.key("down").sleep(0.7).key("space").sleep(1.0).key("enter")
    t.sleep(3.5)
    return t


def connect() -> Tape:
    t = opening(Tape()).sleep(1.0)
    # Connect via the fuzzy palette — the session runs in an embedded PTY.
    t.type("/").sleep(0.5).type("google").sleep(0.8).key("enter").sleep(3.0)
    t.type('cowsay "SSHub: secure shell x undefined behavior"').sleep(0.4)
    t.key("enter").sleep(2.0)
    # Detach; SSH keeps running (session strip in the header).
    t.key("ctrl+d").sleep(2.0)
    # Re-focus the detached session from the dashboard.
    t.key("ctrl+alt+n").sleep(1.5)
    # Second session tab, through the host picker.
    t.key("ctrl+t").sleep(1.0).type("quad").sleep(0.8).key("enter").sleep(2.5)
    # Switch tabs, then detach.
    t.key("ctrl+a").sleep(1.5).key("ctrl+d").sleep(1.5)
    # Close both session tabs from the dashboard.
    t.key("ctrl+w").sleep(1.0).key("ctrl+w").sleep(2.5)
    # go up for full cycle
    t.key("up").sleep(2.0)
    return t


def add_host() -> Tape:
    t = opening(Tape()).sleep(1.0)
    t.type("a").sleep(1.0)                                    # open the form
    t.type("172.217.21.238").sleep(0.4)                       # address
    t.key("down").sleep(0.4).type("BlahblahblahPASSWORD!!!").sleep(0.4)
    t.key("down").sleep(0.4).type("pizza-lover-9000").sleep(0.4)
    t.key("down").sleep(0.4).type("Demo VM").sleep(0.4)       # label
    t.key("down").sleep(0.4).type("demo-vm-1").sleep(0.4)     # name
    t.key("down").sleep(0.4)                                  # port (default)
    # Group: a multi-select picker — Space ticks, Enter closes.
    t.key("down").key("enter").sleep(0.7)
    t.key("down").sleep(0.4).key("space").sleep(0.5)
    t.key("down").sleep(0.4).key("down").sleep(0.4).key("space").sleep(0.6)
    t.key("enter").sleep(0.5)
    t.key("down").sleep(0.4)                                  # identity (default)
    t.key("down").sleep(0.4).type("demo,pizza").sleep(0.5)    # tags
    t.key("ctrl+s").sleep(1.5)                                # save
    t.type("/").sleep(0.8).type("Demo").sleep(3.0)
    return t


def sftp() -> Tape:
    t = opening(Tape())
    # Open the SFTP tab and connect to the demo server.
    t.type("2").sleep(1.5).type("/").sleep(0.6).type("SFTP demo").sleep(1.0)
    t.key("enter").sleep(3.0)
    # Browse the remote tree: descend into a directory and back out.
    t.key("down").sleep(0.8).key("enter").sleep(1.5).key("backspace").sleep(1.5)
    # Stage a download (remote -> local).
    t.key("down").sleep(0.6).key("down").sleep(0.6).key("left").sleep(2.0).still("sftp")
    # Focus the local pane and stage an upload (local -> remote).
    t.key("tab").sleep(0.9).key("down").sleep(0.6).key("right").sleep(1.2)
    # Run the queue, with its progress bar.
    t.type("c").sleep(4.5)
    # File operations on the remote pane, each targeted through / search.
    t.key("tab").sleep(0.7)
    t.type("/").type("README").sleep(0.8).key("enter").sleep(0.7)
    t.type("M").sleep(1.0).key("backspace", times=3).type("600").sleep(0.8)
    t.key("enter").sleep(1.5).type("/").key("esc").sleep(0.6)
    t.type("n").sleep(0.8).type("backups").sleep(0.8).key("enter").sleep(1.5)
    t.type("/").type("backups").sleep(0.8).key("enter").sleep(0.5)
    t.type("d").sleep(1.2).type("y").sleep(1.5).type("/").key("esc").sleep(0.8)
    return t


def screenshots() -> Tape:
    """Stills only — its GIF is scratch."""
    t = opening(Tape())
    t.key("down").sleep(2.0).still("hosts")
    t.type("/").sleep(0.8).type("quad").sleep(2.0).still("palette").key("esc").sleep(0.5)
    t.type("#").sleep(2.0).still("tags").key("esc").sleep(0.5)
    t.type("a").sleep(2.0).still("add-host").key("esc").sleep(0.5)
    t.key("ctrl+k").sleep(2.0).still("keybindings").key("esc").sleep(0.5)
    t.type("?").sleep(2.0).still("help").key("esc").sleep(0.5)
    t.key("ctrl+h").sleep(2.0).still("settings").key("esc").sleep(0.5)
    return t


SCENARIOS: dict[str, dict] = {
    "hero": {"build": hero},
    "navigate": {"build": navigate},
    "connect": {"build": connect},
    "add-host": {"build": add_host},
    "sftp": {
        "build": sftp,
        "cwd": DEMO_HOME / "sftp-local",
        "setup": ["bash", "demo/sftp-server.sh", "start"],
        "teardown": ["bash", "demo/sftp-server.sh", "stop"],
    },
    "screenshots": {"build": screenshots, "scratch": True},
}


def main(argv: list[str]) -> int:
    if not shutil.which("agg"):
        print("agg not found: cargo install --git https://github.com/asciinema/agg",
              file=sys.stderr)
        return 1
    if not (ROOT / "target" / "release" / "sshub").exists():
        subprocess.run(["cargo", "build", "--release"], cwd=ROOT, check=True)
    # The demo hosts are seeded once, outside the recording — the old tapes typed
    # this into the shell and then had to wait for cargo mid-take.
    subprocess.run(["bash", "demo/seed-demo.sh"], cwd=ROOT, check=True,
                   stdout=subprocess.DEVNULL)

    names = argv or list(SCENARIOS)
    for name in names:
        if name not in SCENARIOS:
            print(f"no such scenario: {name}", file=sys.stderr)
            return 1
    BUILD.mkdir(parents=True, exist_ok=True)
    GIFS.mkdir(parents=True, exist_ok=True)

    failed = False
    for name in names:
        spec = SCENARIOS[name]
        tape = spec["build"]()
        offset = tape.start or 0.0
        want = tape.script[-1][0] - offset + TAIL
        if spec.get("setup"):
            subprocess.run(spec["setup"], cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
        try:
            print(f"==> recording {name}")
            cast = BUILD / f"{name}.cast"
            got = record(tape, env_for(name, spec.get("cwd")), cast)
        finally:
            if spec.get("teardown"):
                subprocess.run(spec["teardown"], cwd=ROOT, check=False,
                               stdout=subprocess.DEVNULL)
        ratio = got / want if want else 0.0
        verdict = "ok" if abs(ratio - 1) <= 0.1 else "OFF"
        print(f"    pacing: {got:.1f}s recorded vs {want:.1f}s scripted ({ratio:.2f}x) {verdict}")
        failed |= verdict == "OFF"

        print(f"==> encoding {name}.gif")
        gif = (BUILD if spec.get("scratch") else GIFS) / f"{name}.gif"
        render(cast, gif)
        print(f"    {gif.relative_to(ROOT)}: {gif.stat().st_size / 1e6:.1f} MB")
        grab_stills(gif, tape.stills, offset)
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
