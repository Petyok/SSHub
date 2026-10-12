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
import tempfile
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
# Seeded once per run; every scenario then works on its own copy (env_for).
SEEDED = ROOT / "demo" / "home" / ".local" / "share" / "sshub"

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
# Seconds per character. 80ms reads as typing; faster reads as a paste.
TYPING = 0.08

RESYNC = "resync"   # scheduled action: force a full repaint (see Tape.begin)

KEYS = {
    "enter": b"\r", "esc": b"\x1b", "tab": b"\t", "space": b" ", "backspace": b"\x7f",
    "up": b"\x1b[A", "down": b"\x1b[B", "right": b"\x1b[C", "left": b"\x1b[D",
    "pgup": b"\x1b[5~", "pgdn": b"\x1b[6~",
    "ctrl+pgup": b"\x1b[5;5~", "ctrl+pgdn": b"\x1b[6;5~",
    "ctrl+a": b"\x01", "ctrl+d": b"\x04", "ctrl+f": b"\x06", "ctrl+h": b"\x08",
    "ctrl+k": b"\x0b", "ctrl+n": b"\x0e", "ctrl+r": b"\x12", "ctrl+s": b"\x13",
    "ctrl+t": b"\x14", "ctrl+w": b"\x17", "ctrl+y": b"\x19",
    # ESC prefix is how a terminal spells Alt.
    "alt+s": b"\x1bs",
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
        """Keep this moment as a screenshot.

        Scheduled like a keystroke rather than remembered as a script offset:
        the recording is trimmed on a *measured* moment, so anything derived from
        script time drifts, and a still that lands 200ms early catches a popup
        still sliding in. Extracted from the rendered GIF, so it carries the
        GIF's 256-colour palette — for a terminal that is a few dozen colours
        wide, that is lossless in practice.
        """
        self.script.append((self.t, ("still", name)))
        # Reserve a moment afterwards. Without it the marker and whatever comes
        # next (usually the Esc that closes the overlay) share a timestamp, and
        # the screenshot catches the popup already sliding back out — clipped at
        # the top, which is exactly how the close animation starts.
        self.t += 0.3
        return self


def record(tape: Tape, exe: Path, env: dict[str, str], cast: Path) -> float:
    """Run sshub under a PTY, feed it `tape`, write an asciicast. Returns the
    duration of the visible part."""
    master, slave = pty.openpty()

    def winsize(cols: int) -> None:
        fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, cols, 0, 0))

    winsize(COLS)
    proc = subprocess.Popen(
        [str(exe)],
        stdin=slave, stdout=slave, stderr=slave, env=env,
        close_fds=True, preexec_fn=os.setsid, cwd=env.get("SSHUB_CWD", str(ROOT)),
    )
    os.close(slave)

    script = sorted(tape.script, key=lambda item: item[0])
    t0 = time.monotonic()
    trim_at: float | None = None
    stills: list[tuple[str, float]] = []
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
            elif isinstance(action, tuple) and action[0] == "still":
                stills.append((action[1], time.monotonic() - t0))
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
        stills = [(name, round(at - trim_at, 6)) for name, at in stills]
    tape.stills = stills
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
    ], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def grab_stills(gif: Path, stills: list[tuple[str, float]]) -> None:
    """Pull each marked moment out of the rendered GIF.

    Takes the last frame at or before the mark, which is the one that was on
    screen then. Not `-ss` seeking (a GIF whose frames carry real, uneven delays
    does not seek predictably) and not the first frame *after* the mark either: an
    idle screen emits no frames at all, since ratatui only writes what changed, so
    "the next frame" is whatever moved next — usually a popup opening.
    """
    SHOTS.mkdir(parents=True, exist_ok=True)
    for name, at in stills:
        # -update rewrites the same file for every selected frame, so the one
        # left behind is the last that qualified.
        subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-i", str(gif),
                        "-vf", f"select=lte(t\\,{at:.3f})", "-update", "1",
                        "-fps_mode", "passthrough", str(SHOTS / f"{name}.png")],
                       check=True)
        print(f"    {SHOTS.name}/{name}.png  (t={at:.2f}s)")


def env_for(slug: str, release: Path, cwd: Path | None = None) -> dict[str, str]:
    """A pristine config and database per scenario, so one run cannot leak
    settings or hosts into another (add-host's new host, a session in "recent").

    Motion is deliberately left ON. `disable_animation` is the reduced-motion
    toggle: it switches off every slide these recordings exist to show. The old
    tapes used it to skip the intro animation, which is why four of the five GIFs
    had no motion in them at all — the intro exits on a keypress, so skip it that
    way instead.
    """
    cfg = Path(f"/tmp/sshub-demo-{slug}")
    data = Path(f"/tmp/sshub-demo-data-{slug}")
    for d in (cfg, data):
        if d.exists():
            shutil.rmtree(d)
    shutil.copytree(ROOT / "demo" / "home" / ".config" / "sshub", cfg)
    shutil.copytree(SEEDED, data)
    # Fresh throwaway home: keeps your username out of frame, and gives the SFTP
    # scenario a pristine local folder (it downloads into it).
    if DEMO_HOME.exists():
        shutil.rmtree(DEMO_HOME)
    shutil.copytree(ROOT / "demo" / "home" / "sftp-local", DEMO_HOME / "sftp-local")
    (DEMO_HOME / ".ssh").mkdir(mode=0o700, parents=True, exist_ok=True)
    # The SFTP identity is seeded with this path (see main), so the keys tab
    # shows a path under the throwaway home instead of your checkout's.
    for suffix in ("", ".pub"):
        shutil.copy2(ROOT / "demo" / "home" / ".ssh" / f"demo_client_ed25519{suffix}",
                     DEMO_HOME / ".ssh")
    env = dict(os.environ)
    # Whatever terminal runs the recorder, the recording is of a colour one.
    for var in ("NO_COLOR", "TERMINFO"):
        env.pop(var, None)
    env.update({
        "HOME": str(DEMO_HOME),
        "TERM": "xterm-256color",
        "COLORTERM": "truecolor",
        "SSHUB_VERSION_LABEL": "",
        "SSHUB_CONFIG_DIR": str(cfg),
        "SSHUB_DATA_DIR": str(data),
        "SSHUB_SSH_CONFIG": str(ROOT / "demo" / "home" / "ssh_config"),
        "PATH": f"{ROOT}/demo/bin:{release}:" + os.environ.get("PATH", ""),
    })
    if cwd:
        env["SSHUB_CWD"] = str(cwd)
    return env


def opening(t: Tape) -> Tape:
    """Dismiss the intro animation, let the dashboard settle, start recording."""
    return t.sleep(0.9).key("enter").sleep(1.2).begin()


# ── Scenarios ──────────────────────────────────────────────────────────────────
#
# Pacing: at least 0.7s between cursor or selection steps (faster reads as
# jitter), 1-1.6s after an overlay opens or closes, typing at TYPING. Keys are
# the defaults (demo/home/.config/sshub/config.toml has no [keybinds]), so a
# GIF never teaches a chord a fresh install does not have.

def hero() -> Tape:
    t = Tape()
    # Open on the intro's settled final frame (logo + tagline + blinking prompt).
    t.sleep(11.5).begin().sleep(2.5).key("enter").sleep(1.2)
    t.key("down", times=3, gap=0.7).sleep(1.0)
    t.type("/").sleep(1.2).type("google").sleep(1.0).key("enter").sleep(3.0)
    t.type("uptime").sleep(0.7).key("enter").sleep(1.8)
    t.key("ctrl+d").sleep(3.0)
    return t


def navigate() -> Tape:
    t = opening(Tape()).sleep(1.5)
    # Walk into the nested groups.
    t.key("down", times=4, gap=0.7).sleep(1.0)
    # Collapse the group, then expand it again. Space folds; Enter would connect.
    t.key("space").sleep(1.6).key("space").sleep(1.6)
    # Fuzzy host palette.
    t.type("/").sleep(1.2).type("quad").sleep(1.6).key("esc").sleep(1.2)
    # Group manager overlay (nested groups).
    t.type("G").sleep(1.6).key("down").sleep(0.8).key("down").sleep(0.8).key("esc").sleep(1.2)
    # Multi-tag filter: # opens the picker, Space toggles, Enter applies.
    t.type("#").sleep(1.2)
    t.key("down").sleep(0.7).key("space").sleep(0.8)
    t.key("down").sleep(0.7).key("space").sleep(1.0).key("enter")
    t.sleep(3.5)
    return t


def connect() -> Tape:
    t = opening(Tape()).sleep(1.0)
    # Connect via the fuzzy palette: the session runs in an embedded PTY.
    t.type("/").sleep(1.2).type("google").sleep(1.0).key("enter").sleep(3.0)
    t.type('cowsay "SSHub: secure shell x undefined behavior"').sleep(0.7)
    t.key("enter").sleep(2.0)
    # Detach; SSH keeps running (session strip in the header).
    t.key("ctrl+d").sleep(2.0)
    # Second session tab, through the host picker.
    t.key("ctrl+t").sleep(1.4).type("quad").sleep(1.0).key("enter").sleep(2.5)
    # Previous tab, then back to the dashboard.
    t.key("ctrl+pgup").sleep(1.8).key("ctrl+d").sleep(1.6)
    # The session switcher finds a running session by name.
    t.key("alt+s").sleep(1.4).key("down").sleep(0.8).key("enter").sleep(2.0)
    t.key("ctrl+d").sleep(1.6)
    # Close both session tabs from the dashboard.
    t.key("ctrl+w").sleep(1.2).key("ctrl+w").sleep(2.5)
    return t


def add_host() -> Tape:
    t = opening(Tape()).sleep(1.0)
    t.type("a").sleep(1.4)                                    # open the form
    t.type("172.217.21.238").sleep(0.7)                       # address
    t.key("down").sleep(0.7).type("BlahblahblahPASSWORD!!!").sleep(0.7)
    t.key("down").sleep(0.7)                                  # secret lookup (empty)
    t.key("down").sleep(0.7).type("pizza-lover-9000").sleep(0.7)
    t.key("down").sleep(0.7).type("Demo VM").sleep(0.7)       # label
    t.key("down").sleep(0.7).type("demo-vm-1").sleep(0.7)     # name
    t.key("down").sleep(0.7)                                  # port (default)
    # Group: a multi-select picker. Space ticks, Enter closes.
    t.key("down").sleep(0.7).key("enter").sleep(1.2)
    t.key("down").sleep(0.7).key("space").sleep(0.8)
    t.key("down").sleep(0.7).key("down").sleep(0.7).key("space").sleep(0.8)
    t.key("enter").sleep(1.2)
    t.key("down").sleep(0.7)                                  # identity (default)
    t.key("down").sleep(0.7).type("demo,pizza").sleep(0.8)    # tags
    t.key("ctrl+s").sleep(1.6)                                # save
    t.type("/").sleep(1.2).type("Demo").sleep(3.0)
    return t


def sftp() -> Tape:
    t = opening(Tape())
    # Open the SFTP tab and connect to the demo server.
    # Lowercase on purpose: in this search a capital S opens the snippet manager.
    t.type("2").sleep(1.5).type("/").sleep(1.0).type("sftp demo").sleep(1.0)
    t.key("enter").sleep(3.0)
    # Browse the remote tree: descend into a directory and back out.
    t.key("down").sleep(0.8).key("enter").sleep(1.5).key("backspace").sleep(1.5)
    # Stage a download (remote -> local).
    t.key("down").sleep(0.7).key("down").sleep(0.7).key("left").sleep(2.0).still("sftp")
    # Focus the local pane and stage an upload (local -> remote).
    t.key("tab").sleep(0.9).key("down").sleep(0.7).key("right").sleep(1.2)
    # Run the queue, with its progress bar.
    t.type("c").sleep(4.5)
    # File operations on the remote pane, each targeted through / search.
    t.key("tab").sleep(0.8)
    t.type("/").type("README").sleep(0.8).key("enter").sleep(0.8)
    t.type("M").sleep(1.2).key("backspace", times=3, gap=0.1).type("600").sleep(0.8)
    t.key("enter").sleep(1.5).type("/").key("esc").sleep(0.8)
    t.type("n").sleep(1.0).type("backups").sleep(0.8).key("enter").sleep(1.5)
    t.type("/").type("backups").sleep(0.8).key("enter").sleep(0.8)
    t.type("d").sleep(1.2).type("y").sleep(1.5).type("/").key("esc").sleep(0.8)
    return t


def themes() -> Tape:
    t = opening(Tape()).sleep(1.0)
    t.key("down", times=2, gap=0.7).sleep(1.0)
    # Settings opens on the Theme row; Enter opens the picker.
    t.key("ctrl+h").sleep(1.4).key("enter").sleep(1.6)
    # Every step previews that theme on the whole interface, not just the box.
    t.key("down", times=4, gap=1.8).sleep(1.8)
    # Back up to one and keep it: only Enter writes the choice.
    t.key("up", times=2, gap=1.2).sleep(1.2).key("enter").sleep(1.4)
    t.key("esc").sleep(1.4)
    t.key("down", times=2, gap=0.8).sleep(2.0)
    return t


def broadcast() -> Tape:
    t = opening(Tape()).sleep(1.0)
    # b: pick a group or tag, type one command, check the hosts, run.
    t.type("b").sleep(1.4)
    t.key("down", times=BROADCAST_TARGET, gap=0.7).sleep(1.0).key("enter").sleep(1.2)
    t.type("uptime").sleep(0.8).key("enter").sleep(1.8)
    t.type("y").sleep(5.0)
    return t


def snippets() -> Tape:
    t = opening(Tape()).sleep(1.0)
    t.type("/").sleep(1.2).type("google").sleep(1.0).key("enter").sleep(3.0)
    t.type("uname -a").sleep(0.7).key("enter").sleep(1.5)
    # A command this session ran comes back as a dim suggestion; Tab takes it.
    t.type("una").sleep(1.6).key("tab").sleep(1.0).key("enter").sleep(1.5)
    # Ctrl+N: the snippet library. Enter runs one...
    t.key("ctrl+n").sleep(1.4).type("mem").sleep(1.2).key("enter").sleep(2.0)
    # ...Tab only types it, to edit before running.
    t.key("ctrl+n").sleep(1.4).type("disk").sleep(1.2).key("tab").sleep(1.2)
    t.type(" /").sleep(0.8).key("enter").sleep(2.0)
    t.key("ctrl+d").sleep(2.0)
    return t


def screenshots() -> Tape:
    """Stills only; its GIF is scratch."""
    t = opening(Tape())
    t.key("down").sleep(2.0).still("hosts")
    t.type("/").sleep(0.8).type("quad").sleep(2.0).still("palette").key("esc").sleep(0.5)
    t.type("#").sleep(2.0).still("tags").key("esc").sleep(0.5)
    t.type("a").sleep(2.0).still("add-host").key("esc").sleep(0.5)
    t.key("ctrl+k").sleep(2.0).still("keybindings").key("esc").sleep(0.5)
    t.type("?").sleep(2.0).still("help").key("esc").sleep(0.5)
    t.key("ctrl+h").sleep(2.0).still("settings")
    t.key("enter").sleep(1.0).key("down", times=THEME_STILL, gap=0.5).sleep(2.0)
    t.still("theme-picker").key("esc").sleep(0.8).key("esc").sleep(0.8)
    t.type("4").sleep(1.0).type("H").sleep(2.0).still("known-hosts").key("esc").sleep(0.5)
    return t


def known_hosts(home: Path) -> None:
    """Give the known hosts manager something to list: throwaway keys for the
    demo hosts, written to the throwaway home (never yours)."""
    lines = []
    with tempfile.TemporaryDirectory() as tmp:
        for i, (host, kind) in enumerate([
            ("8.8.8.8", "ed25519"), ("8.8.4.4", "ed25519"), ("[1.1.1.1]:5432", "ecdsa"),
            ("[1.0.0.1]:5432", "ecdsa"), ("9.9.9.9", "ed25519"), ("4.2.2.2", "rsa"),
        ]):
            key = Path(tmp) / f"k{i}"
            subprocess.run(["ssh-keygen", "-q", "-t", kind, "-N", "", "-C", "", "-f", str(key)],
                           check=True)
            algo, blob = Path(f"{key}.pub").read_text().split()[:2]
            lines.append(f"{host} {algo} {blob}")
    (home / ".ssh" / "known_hosts").write_text("\n".join(lines) + "\n")


# Menu rows to step over: the broadcast target (#prod) and the theme shown in
# the picker still. Both lists are ordered by the app, so the counts are pinned
# to the seeded data and the built-in themes.
BROADCAST_TARGET = 8
THEME_STILL = 2

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
    "themes": {"build": themes},
    "broadcast": {"build": broadcast},
    "snippets": {"build": snippets},
    "screenshots": {"build": screenshots, "scratch": True, "home": known_hosts},
}


def build() -> Path:
    """Build this checkout's binary and return the directory it lands in.

    Not `ROOT/target`: worktrees share one target dir through an untracked
    `.cargo/config.toml`, and CARGO_TARGET_DIR can move it too, so ask cargo.
    That shared dir also fingerprints the crate identically for every worktree,
    so a build here can report itself fresh and leave another worktree's binary
    in place; touching the crate roots makes cargo rebuild from this checkout.
    `--no-default-features` links the system OpenSSL: this binary never ships,
    and the vendored build is most of a cold compile.
    """
    for root in ("src/lib.rs", "src/main.rs"):
        os.utime(ROOT / root)
    subprocess.run(["cargo", "build", "--release", "--no-default-features"],
                   cwd=ROOT, check=True)
    meta = subprocess.run(["cargo", "metadata", "--format-version", "1", "--no-deps"],
                          cwd=ROOT, check=True, capture_output=True, text=True)
    return Path(json.loads(meta.stdout)["target_directory"]) / "release"


def main(argv: list[str]) -> int:
    if not shutil.which("agg"):
        print("agg not found: cargo install --git https://github.com/asciinema/agg",
              file=sys.stderr)
        return 1
    names = argv or list(SCENARIOS)
    for name in names:
        if name not in SCENARIOS:
            print(f"no such scenario: {name}", file=sys.stderr)
            return 1
    release = build()
    # The SFTP host is only seeded when its client key exists (seed-demo.sh), so
    # make the key first: every scenario's host list shows that host.
    subprocess.run(["bash", "demo/sftp-server.sh", "keys"], cwd=ROOT, check=True)
    # The demo hosts are seeded once, outside the recording (the old tapes typed
    # this into the shell and then had to wait for cargo mid-take). The SFTP key
    # is recorded at the path env_for copies it to, under the throwaway home.
    shutil.rmtree(SEEDED, ignore_errors=True)
    seed_env = dict(os.environ, SSHUB_DATA_DIR=str(SEEDED),
                    SSHUB_DEMO_SFTP_KEY=str(DEMO_HOME / ".ssh" / "demo_client_ed25519"))
    subprocess.run(["bash", "demo/seed-demo.sh"], cwd=ROOT, check=True, env=seed_env,
                   stdout=subprocess.DEVNULL)

    BUILD.mkdir(parents=True, exist_ok=True)
    GIFS.mkdir(parents=True, exist_ok=True)

    failed = False
    for name in names:
        spec = SCENARIOS[name]
        tape = spec["build"]()
        want = tape.script[-1][0] - (tape.start or 0.0) + TAIL
        if spec.get("setup"):
            subprocess.run(spec["setup"], cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
        try:
            print(f"==> recording {name}")
            cast = BUILD / f"{name}.cast"
            env = env_for(name, release, spec.get("cwd"))
            if spec.get("home"):
                spec["home"](DEMO_HOME)
            got = record(tape, release / "sshub", env, cast)
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
        grab_stills(gif, tape.stills)
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
