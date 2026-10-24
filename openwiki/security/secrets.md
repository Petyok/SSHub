---
type: Security Model
title: Secrets & Security, credential store, askpass delivery, auth modal, host-key policy, and input safety
description: How SSHub handles secrets and security-sensitive behaviour. Profile-namespaced credential store (OS keyring via the keyring crate, locked credentials.json fallback, StoreLocator so the askpass helper reopens the same store, lazy opening in the CLI), Secret Service lookup for KeePassXC, the two SSH_ASKPASS delivery paths (staged file with references-only hop entries, and a private socket channel bound to the asking ssh pid), anchored prompt matching, read-once and LocalCommand deletion for detached tunnels, the opt-in remember box, host-key policy per path, the destination allowlist, export refusals, OSC 52 relay limits, session-log exposure, file permissions, and the leading-dash target guard.
resource: src/credentials.rs
tags: [security, secrets, keyring, askpass, host-keys, permissions]
---

# Secrets & Security

## Credential store (`src/credentials.rs`)

`PasswordStore` (`get`, `set`, `delete`, `lookup`, `locator`) is the seam. Keys
are `host:{id}` and `identity:{id}` (`host_key`, `identity_key`), always wrapped
in `NamespacedPasswordStore`, which prefixes `profile:<id>:` in profile mode and
nothing in compat mode ([profiles](../architecture/profiles.md)). SQLite keeps
only `has_password` flags and the `secret_lookup` spec, never secret material.

| Store | When |
|---|---|
| `OsKeyring` | `keyring` crate, service `"sshub"`, when `check_keyring_available()` succeeds |
| `FilePasswordStore` | `profiles/<name>/credentials.json` when the keyring probe fails; written through a fresh 0600 `credentials.json.tmp` (symlinks refused) and an atomic rename; every read-modify-write holds an exclusive lock on the 0600 sidecar `credentials.json.lock` |
| `NoopPasswordStore` | tests (`src/app/tests/` also has an in-memory `MemoryPasswordStore`) |

- `check_keyring_available` is a real **write, read back, delete** round trip on
  a unique probe account. A read-only check reported "available" with no session
  bus and every remember write then failed.
- When the keyring is available, `migrate_fallback_to_keyring` moves entries
  from `credentials.json` into it and deletes the file only if every entry
  succeeded. The TUI shows "OS keyring unavailable. Using credentials.json
  fallback." when it fell back.
- **`StoreLocator`** (`Keyring { prefix }` or `File { path, prefix }`, never a
  secret) is what `PasswordStore::locator()` returns; `NamespacedPasswordStore`
  folds its profile prefix into it. It lets the askpass helper, a separate
  process, open exactly the store the TUI or CLI used, with no profile env
  needed. A store that lives only in-process returns `None`.
- **Credentials lock** (`FilePasswordStore::lock`): the lock sits on a sidecar
  file because `write_map` replaces the data file by rename (a lock on it would
  guard an inode that is gone). Two sshub processes, or an import next to the
  TUI, no longer drop each other's secret; reads take the same lock best-effort
  (a read-only directory still reads).
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
  its namespaced entries. A profile **move** transfer first carries the moved hosts'
  and identities' `host:{id}` / `identity:{id}` secrets into the destination
  profile's store under the new ids (`apply_transfer_plan_with_secrets`), then
  deletes them from the source (`TransferResult::purge_source_secrets`). A row
  whose secret could not be carried is not deleted from the source at all, so
  the move can be repeated, and the failure is shown in a popup (TUI) or on
  stderr (CLI). A copy carries nothing and leaves the source's secrets alone.

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

Two rules hold on both delivery paths:

- **Anchored prompts.** A stored secret answers only a prompt of its own class.
  `PromptClass::classify` (`src/session/auth_prompt.rs`) goes by the prompt's
  *shape* (starts with `Enter passphrase…`, ends with `password:`), so a
  server's `(user@host) ` keyboard-interactive text or a banner that merely
  contains those words is `Generic`; host-key questions (`HostKey`) win over
  credential needles. Hop prompts are stricter still: `hop_auth::asker` accepts
  only a whole `user@host's password:` (no whitespace in the login) or
  `Enter passphrase for key '<path>':`, and the hop it names must be unique.
- **Pid binding.** The asking ssh must be the one the secret is for. The
  destination's secret goes to the destination ssh only; a hop's secret goes
  only when the asking ssh's command line ends with that hop's `-J` name
  (`hop_auth::runs_for`: `/proc/<pid>/cmdline`, `ps` elsewhere). A hostile hop
  can make ssh print any `user@host's password:`, but cannot make itself run for
  another hop. Neither the destination's secret nor another hop's ever answers
  a hop.

### Staged file (`src/session/askpass.rs`)

Used where the private channel is not: `sshub host connect`, `sshub exec`,
tunnels (TUI and detached), broadcast, OS detection, and embedded sessions that
are not plain interactive `ssh` (mosh sessions and key-push sessions).

1. `AskpassSecret::staged` writes a fresh 0600 `sshub-askpass-<pid>-<n>` under
   `$XDG_RUNTIME_DIR` (or the temp dir). Format: line 1 is the **destination's**
   secret (empty when it has none); `C<TAB>p|k` says whether it is a password
   or a key passphrase; one `S` line names the credential store
   (`S<TAB>keyring<TAB>prefix` or `S<TAB>file<TAB>prefix<TAB>path`, from the
   `StoreLocator`); each remaining line is a hop,
   `H<TAB>class<TAB>user<TAB>host<TAB>key<TAB>via<TAB>ref…`.
2. **References only for hops.** A hop line holds `k:<store key>` or
   `l:<secret lookup spec>` references, never the secret
   (`app::util::hop_secrets` reads nothing from the store). The helper opens
   the store from the `S` line and reads the first reference that resolves
   when that hop actually asks; an empty value or one with a line break is
   refused. A hop is left out of the file (its prompt then goes to the user)
   when its store cannot be opened from another process, differs from the one
   the `S` line names, or a field holds a tab or line break. A destination
   secret with a line break cannot be staged.
3. The child gets `SSH_ASKPASS=<sshub>`, `SSH_ASKPASS_REQUIRE=force`,
   `SSHUB_ASKPASS_FILE=<path>`, `SSHUB_ASKPASS_OWNER=<sshub pid>`.
4. **Jump-host guard** (#141): the helper walks its process ancestry up to the
   owner pid (16 levels, `/proc` on Linux, `proc_pidinfo` on macOS); if another
   process named `ssh` sits in between, the asker is a ProxyJump/ProxyCommand
   hop and may get only its own hop line (anchored prompt, same class, unique,
   pid bound as above), never line 1; otherwise it exits 1. Where the process
   tree cannot be read the helper assumes a hop (fails closed). A non-hop asker
   gets line 1, and only for a `Password`/`Passphrase` prompt matching the `C`
   class; a 2FA code or host-key question gets nothing.
5. The file is removed on drop. **Detached tunnels** (#144) outlive sshub, so
   `stage_tunnel_askpass` also adds
   `-o PermitLocalCommand=yes -o 'LocalCommand=rm -f -- <path>'` (`%` doubled,
   quoted for `sh`): ssh runs it after authentication, hops included and even
   with `-N`, so nothing stays on disk for the tunnel's lifetime. The helper
   also gets `SSHUB_ASKPASS_READ_ONCE` and wipes each secret as it is answered
   (`consume`: the destination line is blanked, a hop line removed, the file
   deleted with the last one, and deleted if it cannot be rewritten). The path
   is recorded in `<id>.askpass` next to the pid file; `remove_tunnel_pid`
   (stop, stale cleanup) removes the staged file through `remove_outlived`,
   which accepts only `sshub-askpass-*` names. A SIGKILL before the first
   prompt can still leave a staged file behind.

Detached tunnel pid files (`tunnels/<id>.pid`, 0600, created exclusively) carry
the process **start time** on line 2 (field 22 of `/proc/<pid>/stat`,
`ps -o lstart=` elsewhere). `tunnel_pid_alive` trusts, and `stop` signals, a
recorded pid only while its start time still matches, so a recycled pid is
never SIGTERMed; a legacy bare-pid file is trusted only while the pid is an
`ssh` process.

### Private channel (`src/session/askpass_channel.rs`)

Used by every embedded interactive `ssh` session (`session::interactive_auth`;
not mosh, not key push).

- `Channel::new` creates a 0700 `sshub-auth-*` directory under
  `$XDG_RUNTIME_DIR` (or the temp dir) holding a 0600 Unix socket, plus a
  64-hex-char token from 32 bytes of `/dev/urandom`. The ssh child gets
  `SSHUB_ASKPASS_SOCKET`, `SSHUB_ASKPASS_TOKEN` and `SSHUB_ASKPASS_CHANNEL=2`
  (the mode that makes the helper report its parent pid as `ssh_pid`).
- Each helper invocation sends `{token, prompt, ssh_pid}` (unknown fields are
  rejected) and waits for an answer or cancel. Limits: 120 s timeout, answers
  at most 4096 bytes without CR/LF/NUL, 32 KiB frames, 8 queued clients.
  Nothing about prompts or answers enters diagnostics. The socket and token are
  torn down as soon as the session is connected.
- **Destination** = the request whose `ssh_pid` equals the pty child's pid
  (`destination_pid`; the `sh -c 'exec ssh …'` wrapper keeps the pid). A request
  without a pid is not the destination. The stored secret is taken once and
  answers only a prompt of its class.
- **Hops** (`src/session/hop_auth.rs`): a request from another pid is matched
  with `find_for` (anchored prompt, and the asking ssh runs for that hop) to
  the one saved hop it names; that hop's own secret is fetched when it asks
  and offered once per (hop, class). A prompt that names no saved hop, or two,
  opens the modal, titled `Session: <host>, hop <name>`, and its Remember box
  saves to that hop host.
- The auth modal (`AppMode::AuthPrompt`, `src/app/auth.rs`) is masked, one at a
  time, and capped at three attempts per class for each ssh in the chain (the
  destination, each saved hop and each other pid count separately; host-key
  questions have their own cap of three).

Ssh children outside the TUI (tunnels, `exec`, `connect`, broadcast, test
connect, key push) use the staged file above.

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
| `sshub host connect`, `sshub exec`, tunnels, key push (each only with a stored secret) | `StrictHostKeyChecking=accept-new` | TOFU so a staged secret is not stuck behind a prompt; `sshub exec` and tunnels without a secret run `BatchMode=yes` |
| Broadcast | `StrictHostKeyChecking=accept-new` (runner default), `ConnectTimeout`, `BatchMode=no` with a secret or hop refs, else `yes` | Never prompts |
| OS detection | `accept-new`, `ConnectTimeout=8`, `BatchMode=no` with a secret, else `yes` | Never prompts |
| Test connect (`src/ssh/test_connect.rs`) | `StrictHostKeyChecking=yes`, `UpdateHostKeys=no` in an isolated `-F` config with `ProxyCommand none` | A diagnostic never touches known_hosts |
| mosh with a stored secret | accept-new injected into mosh's inner ssh | |
| SFTP (libssh2) | `verify_host_key` | Appends unknown keys to `~/.ssh/known_hosts` itself and treats a **changed** key as a hard error |

`LogLevel=DEBUG1` replaces `-v` because ssh does not hand it on to jump hosts,
whose own `Authenticated to` line used to mark the session connected too early
(#140). The embedded ssh writes that log to its own `-E` FIFO, separate from the
stderr FIFO and inside a private 0700 temp dir; only that stream is scanned for
the connected marker, so a server banner on stderr cannot pose as ssh output.
The known hosts manager is documented in
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
`launcher.db`, `config.toml`, `credentials.json` (and its `.lock`), session
logs, tunnel PID files and staged askpass files 0600. `ssh-keygen -y`
passphrase probing (`src/ssh/keyfile.rs`) uses a separate 0600 secret file plus
0700 helper script in the temp dir.

## Input safety

- **Destination allowlist** (`ssh::destination_problem`, `src/ssh/host.rs`): a
  stored address and effective login must be made of what a real host or login
  name is (`map::parse_raw_hop`: login `[A-Za-z0-9.+_%@-]` with no leading `-`,
  host `[A-Za-z0-9._%-]`, a bracketed IPv6 literal, no `ssh://` prefix or
  surrounding whitespace). OpenSSH before 9.6 pipes `ssh -W '[%h]:%p'` through
  `sh -c` behind a ProxyJump (CVE-2023-51385), and `Match exec` /
  `ProxyCommand` in the user's own config expand `%h`/`%u` the same way. It is
  enforced at every layer: the host form, `sshub host add/edit`, all three
  importers (rows land in `skipped_invalid`), `ResolvedConnection::jump_error`
  (connect, tunnels, broadcast and test connect refuse; connecting without the
  jump would bypass the bastion), and finally `build_ssh_argv`, which swaps a
  failing target for `refused-invalid-destination.invalid` so no caller can leak
  one. ssh_config rows are exempt: they connect by alias. ProxyJump hops use
  the same allowlist (`map::hops::jump_problem`).
- **Leading-dash guard** (#101): `store::hosts::reject_option_like` refuses a
  host `name`, `address` or `username` starting with `-` on add, edit and
  import (`is_option_like` lets the host form say so before saving), and
  `ssh::safe_ssh_target` rewrites any such target that still reaches ssh or mosh
  to `ssh://-...`, which OpenSSH refuses instead of parsing as an option (an
  address of `-oProxyCommand=id` used to run `id` locally).
- **Export refusals** (`src/ssh/export.rs`, `sshub export` / `Shift+E`): every
  launcher host is resolved first (group defaults baked in) and the **whole
  export fails** naming the host when one row does not validate, because every
  later `ssh` run reads the file. Refused: an unresolvable host; a `Host`
  name, `HostName` or `User` that is empty, starts with `-`, or holds
  whitespace, control characters, quotes, `\` or `#` (the `Host` line also
  `* ? ! ,`); a ProxyJump that is not valid hops; control characters in a key,
  certificate path or remote command. `%` is doubled so ssh does not expand
  tokens, and paths with spaces are quoted. Written atomically with a `.bak`.
- **Broadcast and OS detection** drop the host's stored `remote_command`
  (`ssh::without_remote_command`) so it cannot run in place of the probe;
  broadcast runs mosh hosts over ssh.
- Ad-hoc connect (`src/app/adhoc.rs`) validates the destination (no leading
  `-`, no whitespace or control characters) and places it after `--`.
- `sshub exec` holds back everything after `--` from global flag parsing, and
  audits runs without recording the command.
- Imports print nothing to stderr while the TUI is in raw mode
  (`src/ssh/import.rs`).
- `sshub host resolve` (`cmd_resolve`, JSON `HostResolveJson`) exposes only
  `has_stored_secret`, never the secret.
