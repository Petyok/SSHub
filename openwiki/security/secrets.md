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
<!-- openwiki: broken internal link [../domain/hosts-identities.md#termius-import] heading anchor "termius-import" does not exist in "../domain/hosts-identities.md". Fix the href or restore the target, then delete this comment. -->
- [Termius import](../domain/hosts-identities.md#termius-import) re-stores imported secrets with write-verification and reports `keyring_failures` instead of dropping them silently.
- `sshub db purge` orphans keyring entries by design (only SQLite is wiped).
- If Secret Service is unavailable, SSHub uses an owner-only plaintext `credentials.json` in the data directory, written through a 0600 temporary file and atomic rename. The status reports the fallback; when a keyring becomes available, entries migrate back and the file is removed only after all entries succeed.
- Secret fields can be prefilled masked from the store. `Ctrl+R` reveals and copies, while `Ctrl+Y` copies without revealing; both actions report only that a value was copied. Leaving the field hides it again, and clearing a previously stored field explicitly deletes the credential.

## SSH_ASKPASS staging (`src/session/askpass.rs`)

Secrets reach ssh without appearing in argv or PTY history:

1. The secret is written to a 0600 file under `$XDG_RUNTIME_DIR`.
2. The child process gets `SSH_ASKPASS=<path to the sshub binary itself>`, `SSH_ASKPASS_REQUIRE=force`, `SSHUB_ASKPASS_FILE=<path>`.
3. ssh re-executes sshub; `main.rs` calls `maybe_run_askpass()` **first** (before touching argv or the TUI), prints the staged secret, and exits.
4. The file is removed on Drop. Caveat: a SIGKILL can leave a stale staged file behind (no atexit cleanup) — see quickstart [backlog](../quickstart.md#backlog).

The same mechanism feeds [tunnel](../workflows/tunnels.md) spawns (`stage_tunnel_askpass`) and `ssh-keygen -y` passphrase probing (`src/ssh/keyfile.rs`) — explicitly because "`ps` would expose it" in argv. The helper path is re-resolved when an in-place upgrade replaces the running executable, so password authentication does not require a restart; genuine helper lookup failures are written to the session log.

## Clipboard relay boundary

[Embedded sessions](../workflows/sessions-sftp.md) relay OSC 52 clipboard writes only from the session currently visible. Empty payloads, clipboard reads, background tabs, and non-session views are refused or dropped; payloads and queue capacity are bounded. This is enabled by default but can be disabled with `[clipboard] relay_from_pty = false`. Because the sequence is in the raw PTY stream, enabled session logs may contain it.

## Host-key policy

<!-- openwiki: broken internal link [../workflows/sessions-sftp.md#sftp] heading anchor "sftp" does not exist in "../workflows/sessions-sftp.md". Fix the href or restore the target, then delete this comment. -->
TOFU mirroring `accept-new` everywhere: ssh spawns inject `StrictHostKeyChecking=accept-new` when a secret is staged; [SFTP](../workflows/sessions-sftp.md#sftp) appends unknown keys to `~/.ssh/known_hosts` manually (libssh2's writer would drop unparsable lines) and treats a **changed** key as a hard MITM error.

## Session logs capture secrets

<!-- openwiki: broken internal link [../workflows/sessions-sftp.md#session-logging] heading anchor "session-logging" does not exist in "../workflows/sessions-sftp.md". Fix the href or restore the target, then delete this comment. -->
[Session logging](../workflows/sessions-sftp.md#session-logging) is opt-in and captures **everything echoed to the terminal — including typed passwords**. The in-app help screen carries this warning; keep it in sync if logging behavior changes.

## Filesystem permissions (`src/secure_fs.rs`)

Best-effort (Unix-only) hardening: data/log/PID directories 0700, secret-bearing files 0600. Applied to session logs, askpass staging, tunnel PID files, and askpass helper scripts.

## Input-safety details worth preserving

- `src/ssh/export.rs::conf_val` flattens CR/LF in host fields so an exported `exported.conf` can't be used to inject a `Host *` stanza.
- Host targets supplied to ssh/mosh are guarded against leading-dash option injection; host `name`, `address`, and `username` writes are rejected at the TUI, CLI, and importer boundaries. The guard also protects legacy rows at connect time (`src/ssh/host.rs`).
- Imports print nothing to stderr while the TUI is in raw mode (`src/ssh/import.rs`) — a diagnostic would corrupt the UI.
- the `resolve` CLI subcommand (`cmd_resolve` in `src/cli/host.rs`, JSON shape `HostResolveJson` in `src/cli/output.rs`) exposes only `has_stored_secret`, never the secret.
- `exec` deliberately omits the remote command from audit details because arguments can contain secrets; it uses `BatchMode` without a staged credential and refuses mosh one-shot execution. See [Headless CLI](../workflows/cli.md#command-tree).
