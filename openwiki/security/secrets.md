---
type: Security Model
title: Secrets & Security — OS keyring, SSH_ASKPASS staging, host-key policy, and file permissions
description: How SSHub handles secrets and security-sensitive behavior — OS keyring storage via the keyring crate, SSH_ASKPASS re-exec secret staging, TOFU host-key policies, session-log secret capture warning, and 0600/0700 permission enforcement.
resource: src/credentials.rs
tags: [security, secrets, keyring, askpass, permissions]
---

# Secrets & Security

## OS keyring (`src/credentials.rs`)

Host passwords and identity key passphrases live in the OS keyring through the `keyring` crate, service name `"sshub"`, keys `host:{id}` / `identity:{id}`. `PasswordStore` is the trait seam; `OsKeyring` in production, `NoopPasswordStore` in tests. SQLite stores only `has_password` flags — never secret material ([data model](../architecture/data-model.md)).

- `Cargo.toml` enables real backends (`apple-native`, `windows-native`, `sync-secret-service`, `crypto-rust`): without a backend feature, keyring 3.x **silently falls back to an in-memory mock** that works within one process but persists nothing — which "looks exactly like 'passwords aren't being saved'".
- On Linux this needs the D-Bus Secret Service (build: `libdbus-1-dev`; runtime: an unlocked gnome-keyring/KWallet or similar). Without a provider, SSHub warns and ssh falls back to interactive prompting.
- [Termius import](../domain/hosts-identities.md#termius-import) re-stores imported secrets with write-verification and reports `keyring_failures` instead of dropping them silently.
- `sshub db purge` orphans keyring entries by design (only SQLite is wiped).

## SSH_ASKPASS staging (`src/session/askpass.rs`)

Secrets reach ssh without appearing in argv or PTY history:

1. The secret is written to a 0600 file under `$XDG_RUNTIME_DIR`.
2. The child process gets `SSH_ASKPASS=<path to the sshub binary itself>`, `SSH_ASKPASS_REQUIRE=force`, `SSHUB_ASKPASS_FILE=<path>`.
3. ssh re-executes sshub; `main.rs` calls `maybe_run_askpass()` **first** (before touching argv or the TUI), prints the staged secret, and exits.
4. The file is removed on Drop. Caveat: a SIGKILL can leave a stale staged file behind (no atexit cleanup) — see quickstart [backlog](../quickstart.md#backlog).

The same mechanism feeds [tunnel](../workflows/tunnels.md) spawns (`stage_tunnel_askpass`) and `ssh-keygen -y` passphrase probing (`src/ssh/keyfile.rs`) — explicitly because "`ps` would expose it" in argv.

## Host-key policy

TOFU mirroring `accept-new` everywhere: ssh spawns inject `StrictHostKeyChecking=accept-new` when a secret is staged; [SFTP](../workflows/sessions-sftp.md#sftp) appends unknown keys to `~/.ssh/known_hosts` manually (libssh2's writer would drop unparsable lines) and treats a **changed** key as a hard MITM error.

## Session logs capture secrets

[Session logging](../workflows/sessions-sftp.md#session-logging) is opt-in and captures **everything echoed to the terminal — including typed passwords**. The in-app help screen carries this warning; keep it in sync if logging behavior changes.

## Filesystem permissions (`src/secure_fs.rs`)

Best-effort (Unix-only) hardening: data/log/PID directories 0700, secret-bearing files 0600. Applied to session logs, askpass staging, tunnel PID files, and askpass helper scripts.

## Input-safety details worth preserving

- `src/ssh/export.rs::conf_val` flattens CR/LF in host fields so an exported `exported.conf` can't be used to inject a `Host *` stanza.
- Imports print nothing to stderr while the TUI is in raw mode (`src/ssh/import.rs`) — a diagnostic would corrupt the UI.
- `src/cli/resolve` output exposes only `has_stored_secret`, never the secret.
