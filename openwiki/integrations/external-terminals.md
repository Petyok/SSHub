---
type: Integration
title: Integrations — demo pipeline and external touchpoints
description: SSHub's external integrations — the VHS-based demo recording pipeline under demo/ that produces README GIFs and screenshots, plus other external touchpoints (OS keyring, ssh-agent, CI). The 0.9.x external-terminal launcher subsystem (kitty/ghostty/custom) was removed in 0.10.0.
resource: demo/record.sh
tags: [integrations, demo, vhs]
---

# Integrations

## External terminal launchers — removed in 0.10.0

The `TerminalLauncher` abstraction (`src/launcher/` — kitty, ghostty, and custom-command launchers configured via the `terminal` / `launch_command` keys in `config.toml`) was **removed in 0.10.0** (issue #30). Embedded PTY sessions had already become the only connect transport, so the subsystem was dead code. Old `config.toml` files that still carry those keys keep loading fine; the keys are simply ignored. Sessions now always run in the [embedded PTY](../workflows/sessions-sftp.md), and the headless [CLI](../workflows/cli.md) `connect` spawns ssh/mosh directly.

## Demo pipeline (`demo/`)

README GIFs and screenshots are reproducible artifacts:

- **VHS tapes** (`demo/tapes/*.tape` — hero, navigate, connect, add-host, sftp, screenshots) → MP4 in gitignored `demo/build/`; `demo/record.sh` (driven by `just record-gifs`) does a two-pass ffmpeg GIF conversion to avoid VHS's RAM-hungry single-graph palette encoding.
- **Fixture home** (`demo/home/` + `demo/bin/` mock `ssh`/`cowsay`) — `demo/seed-demo.sh` runs the `seed-demo` cargo **example** (`demo/seed_demo.rs`, deliberately an example so `cargo install` never ships it) against the fake home via `SSHUB_DATA_DIR`/`SSHUB_SSH_CONFIG`.
- `demo/sftp-server.sh` stands up a local SFTP server for transfer demos.
- Outputs: `demo/gifs/` (5 GIFs) and `demo/screenshots/` (8 PNGs), excluded from the published crate via `Cargo.toml`'s `exclude`.

Design history: `docs/superpowers/specs/2026-07-12-demo-tapes-redesign-design.md`.

## Other external touchpoints

- **OS keyring / Secret Service** — see [secrets](../security/secrets.md).
- **ssh-agent, ssh -G, ssh-keygen, Termius/PuTTY/mRemoteNG imports** — see [hosts & identities](../domain/hosts-identities.md).
- **GitHub Actions / crates.io** — see [CI & automation](../operations/ci-cd.md).
