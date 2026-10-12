---
type: Integration
title: Integrations, external tools, release channels, and the demo pipeline
description: SSHub's integration surface. The external binaries it shells out to (ssh, ssh-keygen, ssh-add, mosh, ping, curl, tar, npm, cargo), the OS keyring and Secret Service, the GitHub release check and in-place update per install channel, and the asciicast-based demo recording pipeline. The external-terminal launcher was removed in 0.10.0.
resource: src/update.rs
tags: [integrations, external-tools, update, npm, demo, asciicast]
---

# Integrations

## External-terminal launcher status

The Kitty/Ghostty/custom `TerminalLauncher` subsystem and its `terminal` /
`launch_command` config keys were removed in 0.10.0, and no trace of the trait
remains in `src/` or `tests/`. Embedded [PTY sessions](../workflows/sessions-sftp.md)
are the connection transport; `sshub host connect` hands the current terminal to
ssh/mosh. Old config files still load because the legacy keys are ignored.
`just install` still detects a terminal (kitty, ghostty, alacritty, foot, else
xterm) only to fill the `.desktop` launcher entry.

## External binaries

SSHub links no SSH client library for sessions; it shells out:

| Tool | Used by |
|---|---|
| `ssh` | sessions, `connect`, `exec`, tunnels, broadcast, OS detection, key push, `ssh -G` resolution |
| `mosh` | hosts with transport `mosh` |
| `ssh-keygen` | key probing and generation, certificate status (`-L`), known hosts fingerprints and removal (`-l`, `-R`) |
| `ssh-add` | ssh-agent list/add/remove |
| `ping` | reachability (`ping -c 1 -W 1`, every 30 s, switchable) |
| `curl`, `tar` | release check and tarball update (`src/update.rs`) |
| `npm`, `cargo` | in-place update for those install channels |

SFTP is the exception: it uses libssh2 through the `ssh2` crate, with OpenSSL
vendored by default (see [build & release](../operations/build-release.md)).

## OS keyring and Secret Service

The `keyring` crate (macOS Keychain, Windows Credential Manager, Linux Secret
Service) stores credentials; `dbus-secret-service` (Linux only) serves the
per-host `secret_lookup`, which reads KeePassXC entries through its Secret
Service integration. Details: [secrets](../security/secrets.md).

## Release check and update (`src/update.rs`, `src/app/update.rs`)

- With `[updates] check = true` (default) an interactive TUI start runs
  `spawn_check` on a worker thread: `curl` against
  `https://api.github.com/repos/Petyok/SSHub/releases/latest` (15 s timeout),
  compared as plain `MAJOR.MINOR.PATCH` (`is_newer`; pre-releases are never
  announced). A newer release shows next to the version in the tab bar, as a
  footer hint and as a toast. Smoke tests and `SSHUB_AUTO_QUIT` runs never reach
  the network.
- `install_channel()` decides how the binary was installed:
  `SSHUB_INSTALL_CHANNEL` (the npm shim sets `npm`), else `cargo` when the
  binary is in `$CARGO_HOME/bin`, else `source` inside a `target`/`*-target` dir
  or `~/.local/bin`, else none (release tarball or package). The tab-bar version
  shows the channel.
- `KeyAction::Update` (default `Shift+U`) installs a known update, or checks on
  demand: npm runs `npm install -g sshub-tui@<version>`, cargo runs
  `cargo install sshub --version <version>`, a tarball install downloads the
  release asset for linux-x64 or macOS arm64/x64 into a temp dir next to the
  binary and swaps it in atomically. `source` builds and `npx` runs get a hint
  instead. Quitting during an install asks first, even with quit confirmation
  off.

## npm packaging (`npm/`)

`npm/build.sh` assembles platform packages (`npm/platform`) and the `sshub-tui`
wrapper (`npm/wrapper`) from the tarballs attached to a GitHub release; nothing
is compiled. `just npm-build` / `just npm-publish` run it locally, and the
release workflow's `npm` job publishes with npm OIDC
([CI](../operations/ci-cd.md#releaseyml-binaries-npm-cratesio)).

## Demo pipeline (`demo/`)

README GIFs and screenshots are recorded from the terminal byte stream:
`demo/record.py` (run via `just record-gifs [scenario ...]`) drives scenarios
from its `SCENARIOS` table under a PTY, writes timestamped asciicast and renders
it with `agg` (needs `agg` and `ffmpeg`), checking each take against its
expected timing; motion stays on so animations are recorded at 1.00x. The
fixture home (`demo/home/`, mock commands in `demo/bin/`) is seeded by
`demo/seed-demo.sh` and the `seed-demo` Cargo example (`demo/seed_demo.rs`);
`demo/sftp-server.sh` backs transfer scenarios. Output lives in `demo/gifs/`
and `demo/screenshots/`. Contributor tooling: `Cargo.toml` `exclude` keeps the
GIFs, screenshots, fixture home, mock bin and `play.sh` out of the crate package
(it still lists a `demo/tapes` path that no longer exists).

## Other touchpoints

- **ssh-agent, ssh -G, ssh-keygen, known_hosts, and imports**: see
  [hosts & identities](../domain/hosts-identities.md) and the
  [known-hosts manager](../workflows/known-hosts.md).
- **GitHub Actions, crates.io, npm**: see [CI & automation](../operations/ci-cd.md).
