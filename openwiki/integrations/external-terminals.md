---
type: Integration
title: Integrations — terminal stream demos and external touchpoints
description: SSHub's current integration surface includes embedded PTY demo recording, OS keyring, SSH tooling, and GitHub/crates.io automation; the former external-terminal launcher was removed in 0.10.0.
resource: demo/record.py
tags: [integrations, terminal, demo, asciicast]
---

# Integrations

## External-terminal launcher status

The retired Kitty/Ghostty/custom `TerminalLauncher` subsystem and its `terminal` / `launch_command` configuration keys were removed in 0.10.0. Embedded [PTY sessions](../workflows/sessions-sftp.md) are the connection transport. Old configuration files remain loadable because those legacy keys are ignored; do not add new runtime behavior under the removed `src/launcher/` path.

## Demo pipeline (`demo/`)

README GIFs and screenshots are now recorded from the terminal byte stream rather than timer-sampled screenshots. `demo/record.py` drives the scenarios, records timestamped asciicast output, and renders it with `agg`; timing checks compare each take with the scenario's expected duration. This preserves the animation timeline at 1.00x and avoids VHS capture-rate distortion. The tapes keep reduced-motion disabled so the motion pass is visible.

The fixture home (`demo/home/` plus mock commands in `demo/bin/`) is seeded by `demo/seed-demo.sh` and the `seed-demo` Cargo example. `demo/sftp-server.sh` supports transfer scenarios. Generated GIFs and screenshots live under `demo/gifs/` and `demo/screenshots/`; the demo tooling is contributor/release infrastructure, not product runtime.

## Other external touchpoints

- **OS keyring / Secret Service** — see [secrets](../security/secrets.md).
- **ssh-agent, ssh -G, ssh-keygen, known_hosts, and Termius backups** — see [hosts & identities](../domain/hosts-identities.md) and the [known-hosts manager](../workflows/known-hosts.md).
- **GitHub Actions / crates.io** — see [CI & automation](../operations/ci-cd.md).
