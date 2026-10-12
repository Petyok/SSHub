---
type: Security Model
title: Secrets & Security, credential store, askpass delivery, auth modal, host-key policy, and input safety
description: How SSHub handles secrets and security-sensitive behaviour. Profile-namespaced credential store (OS keyring via the keyring crate, credentials.json fallback, lazy opening in the CLI), Secret Service lookup for KeePassXC, the two SSH_ASKPASS delivery paths (staged file and private socket channel) with the jump-host guard, the opt-in remember box, host-key policy per path, OSC 52 relay limits, session-log exposure, file permissions, and the leading-dash target guard.
resource: src/credentials.rs
tags: [security, secrets, keyring, askpass, host-keys, permissions]
---

# Secrets & Security

## Credential store (`src/credentials.rs`)

`PasswordStore` (`get`, `set`, `delete`, `lookup`) is the seam. Keys are
`host:{id}` and `identity:{id}` (`host_key`, `identity_key`), always wrapped in
`NamespacedPasswordStore`, which prefixes `profile:<id>:` in profile mode and
nothing in compat mode ([profiles](../architecture/profiles.md)). SQLite keeps
only `has_password` flags and the `secret_lookup` spec, never secret material.

| Store | When |
|---|---|
| `OsKeyring` | `keyring` crate, service `"sshub"`, when `check_keyring_available()` succeeds |
| `FilePasswordStore` | `profiles/<name>/credentials.json` when the keyring probe fails; written through a fresh 0600 `credentials.json.tmp` (symlinks refused) and an atomic rename |
| `NoopPasswordStore` | tests (`src/app/tests/` also has an in-memory `MemoryPasswordStore`) |

- `check_keyring_available` is a real **write, read back, delete** round trip on
  a unique probe account. A read-only check reported "available" with no session
  bus and every remember write then failed.
- When the keyring is available, `migrate_fallback_to_keyring` moves entries
  from `credentials.json` into it and deletes the file only if every entry
  succeeded. The TUI shows "OS keyring unavailable. Using credentials.json
  fallback." when it fell back.
- **When the keyring is opened**: the TUI probes once in
  `App::new_with_profile`. The headless CLI holds the store in a `OnceCell`
  (`CliContext::password_store`) and opens it only for commands that read,
  store or remove a secret (connect, exec, host resolve, tunnel start, sftp,
  import, host and identity delete, add/edit with a password flag). Opening it
  eagerly hung every command, including `source <(sshub completions zsh)`, on a
  locked keyring whose unlock prompt could not open (#135).
- `Cargo.toml` enables real backends (`apple-native`, `windows-native`,
  `sync-secret-service`, `crypto-rust`); without one, keyring 3.x silently
  falls back to an in-memory mock that persists nothing. Linux builds need
  `libdbus-1-dev` and `pkg-config`.
- Secret fields in the host and identity forms are prefilled masked. `Ctrl+R`
  reveals and copies, `Ctrl+Y` copies without revealing (copies go out as OSC 52
  through `src/osc52.rs`). Clearing a stored field deletes the credential. A
  store that cannot be read is shown as such, not as an empty field.
- [Termius import](../domain/hosts-identities.md#imports-srcimport) re-stores
  secrets with write verification and reports `keyring_failures`.
- `sshub db purge` orphans keyring entries by design; deleting a profile purges
  its namespaced entries.

### Which secret a connection uses

`app::util::resolve_pending_secret`: (1) a stored passphrase of the effective
identity when it has a key (ssh offers the key before a password); (2) the
host's `secret_lookup`, which is **authoritative**: a failed lookup prompts
instead of falling back; (3) the host password; (4) the identity secret
(passphrase for a keyed identity, shared login password for a keyless one);
(5) nothing, so ssh uses the agent or prompts. Tunnels use
`resolve_pending_secret_for_managed` (lookup, then host password).

### Secret Service lookup

`hosts.secret_lookup` holds one `Attribute=value` pair (`parse_secret_lookup`),
for example `URL=ssh://root@10.0.0.5:22` or `Uuid=...`. On Linux
`secret_service_lookup` connects with `dbus-secret-service`, searches unlocked
and locked items, requires **exactly one** match (zero or several is an error
naming the count), lets the provider show its own unlock prompt, and returns the
secret. The spec names a provider entry, so it is not namespaced. Other
platforms return an error. With KeePassXC's Secret Service integration this
reads KeePassXC entries without copying passwords into SSHub.

## SSH_ASKPASS

Secrets never travel in argv or through the PTY. The helper is the sshub binary
itself: `main.rs` calls `maybe_run_askpass()` before anything else.
`askpass::helper_exe` re-resolves `/proc/self/exe` when an in-place upgrade
replaced the binary (`<path> (deleted)`), so password auth keeps working
without a restart; a genuine failure is written to the session diagnostics.

### Staged file (`src/session/askpass.rs`)

Used where the private channel is not: `sshub host connect`, `sshub exec`,
tunnels, broadcast, OS detection, and embedded sessions that are not plain
interactive `ssh` (mosh sessions and key-push sessions).

1. `AskpassSecret::new` writes the secret to a fresh 0600
   `sshub-askpass-<pid>-<n>` under `$XDG_RUNTIME_DIR` (or the temp dir).
2. The child gets `SSH_ASKPASS=<sshub>`, `SSH_ASKPASS_REQUIRE=force`,
   `SSHUB_ASKPASS_FILE=<path>`, `SSHUB_ASKPASS_OWNER=<sshub pid>`.
3. **Jump-host guard** (#141): the helper walks its process ancestry up to the
   owner pid (16 levels, `/proc` on Linux, `proc_pidinfo` on macOS); if another
   process named `ssh` sits in between, the asker is a ProxyJump/ProxyCommand hop
   and gets nothing (exit 1). A jump host that needs a password therefore needs
   its own key or agent on these paths.
4. The file is removed on drop. Detached tunnels (#144) call `outlive()` and set
   `SSHUB_ASKPASS_READ_ONCE` so the helper deletes the file after the first
   read; `sshub tunnel stop` removes it if ssh never asked (`remove_outlived`
   only accepts `sshub-askpass-*` names). A SIGKILL can still leave a staged
   file behind.

### Private channel (`src/session/askpass_channel.rs`)

Used by every embedded interactive `ssh` session (`session::interactive_auth`).

- `Channel::new` creates a 0700 `sshub-auth-*` directory under
  `$XDG_RUNTIME_DIR` (or the temp dir) holding a 0600 Unix socket, plus a
  64-hex-char token from 32 bytes of `/dev/urandom`. The ssh child gets
  `SSHUB_ASKPASS_SOCKET`, `SSHUB_ASKPASS_TOKEN`, `SSHUB_ASKPASS_CHANNEL`.
- Each helper invocation sends `{token, prompt, ssh_pid}` and waits for an
  answer or cancel. Limits: 120 s timeout, answers at most 4096 bytes without
  CR/LF/NUL, 32 KiB frames, 8 queued clients. Nothing about prompts or answers
  enters diagnostics. The socket and token are torn down as soon as the session
  is connected.
- `PromptClass::classify` (`src/session/auth_prompt.rs`) sorts prompts into
  `Password`, `Passphrase`, `HostKey`, `Generic`. Stored secrets are answered
  only for the matching class and only to the **destination** ssh
  (`destination_pid`); a jump host's prompt opens the normal modal.
- The auth modal (`AppMode::AuthPrompt`, `src/app/auth.rs`) is masked, one at a
  time, and capped at three attempts per class (host-key questions have their
  own cap of three).

### Remember is opt-in

The modal offers **Remember** only when a save target exists
(`SaveTarget::for_prompt`: a managed host is required; `Password` saves to the
host, `Passphrase` to the identity; host-key and generic prompts never save).
The box starts **clear**, and a remembered value is written only after the
session reports authenticated; a re-prompt discards the earlier value of that
class. A save failure leaves one audit row with status `fail` naming the
credential key and the error, never the value.

## Host-key policy

| Path | Option | Behaviour |
|---|---|---|
| Embedded ssh sessions | `StrictHostKeyChecking=ask`, `NumberOfPasswordPrompts=3`, `LogLevel=DEBUG1` (`prepare_session_connect_argv`) | Unknown and changed keys become explicit trust questions in the auth modal; the connect screen shows the server key fingerprint from the debug output |
| `sshub host connect`, `sshub exec`, tunnels, key push (each only with a stored secret) | `StrictHostKeyChecking=accept-new` | TOFU so a staged secret is not stuck behind a prompt; `sshub exec` without a secret runs `BatchMode=yes` |
| Broadcast and OS detection | `StrictHostKeyChecking=accept-new`, `ConnectTimeout`, `BatchMode=yes` without a secret (`BatchMode=no` plus staged askpass with one) | Never prompts |
| mosh with a stored secret | accept-new injected into mosh's inner ssh | |
| SFTP (libssh2) | `verify_host_key` | Appends unknown keys to `~/.ssh/known_hosts` itself and treats a **changed** key as a hard error |

`LogLevel=DEBUG1` replaces `-v` because ssh does not hand it on to jump hosts,
whose own `Authenticated to` line used to mark the session connected too early
(#140). The known hosts manager is documented in
[workflows/known-hosts](../workflows/known-hosts.md).

## Clipboard relay boundary

[Embedded sessions](../workflows/sessions-sftp.md) relay OSC 52 clipboard
writes only from the session currently visible
(`App::relay_visible_session_clipboard`); background tabs, the dashboard, SFTP
and tunnels drop theirs immediately and never replay them. Payloads are capped
at 64 KiB and the queue at 8 (`src/session/parser.rs`), drops are counted and
named in the notice, empty writes are ignored (a remote cannot clear your
clipboard) and clipboard reads stay unanswered. `src/osc52.rs` pins the
selector to `c`. On by default; `[clipboard] relay_from_pty = false` turns it
off. The sequence is part of the raw PTY stream, so an enabled session log may
contain it.

## Session logs capture secrets

[Session logging](../workflows/sessions-sftp.md#session-logging-srcsession_logrs) is opt-in and
captures **everything echoed to the terminal, including typed passwords**. The
in-app help (`src/tui/screens/help.rs`) carries this warning; keep it in sync.
Opt-in command history (`[command_history]`) is separate and filters through
`command_safety::classify_command`, which is best-effort.

## Filesystem permissions (`src/secure_fs.rs`)

Best-effort, Unix-only: data, profile, log and PID directories 0700;
`launcher.db`, `config.toml`, `credentials.json`, session logs, tunnel PID
files and staged askpass files 0600. `ssh-keygen -y` passphrase probing
(`src/ssh/keyfile.rs`) uses a separate 0600 secret file plus 0700 helper script
in the temp dir.

## Input safety

- **Leading-dash guard** (#101): `store::hosts::reject_option_like` refuses a
  host `name`, `address` or `username` starting with `-` on add, edit and
  import (`is_option_like` lets the host form say so before saving), and
  `ssh::safe_ssh_target` rewrites any such target that still reaches ssh or mosh
  to `ssh://-...`, which OpenSSH refuses instead of parsing as an option (an
  address of `-oProxyCommand=id` used to run `id` locally).
- Ad-hoc connect (`src/app/adhoc.rs`) validates the destination (no leading
  `-`, no whitespace or control characters) and places it after `--`.
- `sshub exec` holds back everything after `--` from global flag parsing, and
  audits runs without recording the command.
- `src/ssh/export.rs::conf_val` flattens CR/LF so `exported.conf` cannot gain an
  injected `Host *` stanza.
- Imports print nothing to stderr while the TUI is in raw mode
  (`src/ssh/import.rs`).
- `sshub host resolve` (`cmd_resolve`, JSON `HostResolveJson`) exposes only
  `has_stored_secret`, never the secret.
