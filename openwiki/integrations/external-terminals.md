---
type: Integration
title: Integrations — external terminal launchers and the demo pipeline
description: SSHub's external integrations — legacy terminal configuration retained for compatibility and the VHS-based demo recording pipeline under demo/ that produces README GIFs and screenshots.
resource: src/config.rs
tags: [integrations, terminal, kitty, ghostty, demo]
---

# Integrations

## External terminal configuration

The `terminal` and `launch_command` settings remain in `src/config.rs` for compatibility with the legacy external-launcher design, but the current source tree has no `src/launcher/` implementation. Production TUI sessions use the embedded PTY in `src/session/`; the headless `host connect` path runs `ssh` or `mosh` directly. Treat external-terminal launcher behavior as historical configuration, not a supported runtime extension point. Verify `src/app/connect.rs` and `src/main.rs` before changing this boundary.

## Demo pipeline (`demo/`)

README GIFs and screenshots are reproducible artifacts:

- **VHS tapes** (`demo/tapes/*.tape` — hero, navigate, connect, add-host, sftp, screenshots) → MP4 in gitignored `demo/build/`; `demo/record.sh` (driven by `just record-gifs`) does a two-pass ffmpeg GIF conversion to avoid VHS's RAM-hungry single-graph palette encoding.
- **Fixture home** (`demo/home/` + `demo/bin/` mock `ssh`/`cowsay`) — `demo/seed-demo.sh` runs the `seed-demo` cargo **example** (`demo/seed_demo.rs`, deliberately an example so `cargo install` never ships it) against the fake home via `SSHUB_DATA_DIR`/`SSHUB_SSH_CONFIG`.
- `demo/sftp-server.sh` stands up a local SFTP server for transfer demos.
- Outputs: `demo/gifs/` (5 GIFs) and `demo/screenshots/` (8 PNGs), excluded from the published crate via `Cargo.toml`'s `exclude`.

Design history: `docs/superpowers/specs/2026-07-12-demo-tapes-redesign-design.md`.

## Other external touchpoints

- **OS keyring / Secret Service** — see [secrets](../security/secrets.md).
- **ssh-agent, ssh -G, ssh-keygen, Termius backups** — see [hosts & identities](../domain/hosts-identities.md).
- **GitHub Actions / crates.io** — see [CI & automation](../operations/ci-cd.md).
