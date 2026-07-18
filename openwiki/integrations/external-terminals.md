---
type: Integration
title: Integrations — external terminal launchers and the demo pipeline
description: SSHub's external integrations — the TerminalLauncher abstraction for spawning sessions in kitty/ghostty/custom terminals (src/launcher) and the VHS-based demo recording pipeline under demo/ that produces README GIFs and screenshots.
resource: src/launcher/mod.rs
tags: [integrations, terminal, kitty, ghostty, demo]
---

# Integrations

## External terminal launchers (`src/launcher/`)

`TerminalLauncher` (`src/launcher/trait.rs`) is the abstraction for spawning SSH sessions in an external terminal window. `launcher_from_config` picks an implementation from the top-level `terminal` key in `config.toml` (e.g. `terminal = "kitty"`), with `launch_command` as a sibling top-level key used only when `terminal = "custom"`:

- **Kitty** (`kitty.rs`) — `kitty --class sshub-session --title "SSH: X" --hold -e <argv>`.
- **Ghostty** (`ghostty.rs`) — `ghostty -e <argv>`.
- **Custom** (`custom.rs`) — user command template with whitelisted placeholders (`{host} {user} {hostname} {port} {ssh_command} {ssh_args}`), POSIX-safe quoting; direct-argv fast path, `sh -c` wrapping only when shell operators are present.

The trait's only required method is `launch_ssh_argv`; default methods cover alias launches (`launch`), explicit-argv managed launches (`launch_managed`), and mosh-aware variants (`launch_with_transport`) built on `build_ssh_argv`/`build_mosh_argv` ([hosts](../domain/hosts-identities.md)).

> **Status:** since sessions moved to the [embedded PTY](../workflows/sessions-sftp.md), the launcher is effectively dead at runtime in the TUI (kept as an `AppDeps` seam and exercised by tests/`MockLauncher`). Treat it as a legacy/escape-hatch integration, not the primary connect path — check current usage in `src/app/connect.rs` before extending it.

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
