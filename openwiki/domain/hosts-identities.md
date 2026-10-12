---
type: Domain Concept
title: Hosts, Groups & Identities, host sources, nested groups with connection defaults, Favorites, identities, certificates, and imports
description: Core SSHub domain concepts. Hosts from two sources (managed launcher rows and read-only ~/.ssh/config), nested groups with a reserved Favorites group, M:N membership and inherited connection defaults, identities wrapping SSH keys and certificates with ssh-agent integration, key generation and public-key push, OS auto-detection, and Termius, PuTTY and mRemoteNG import.
resource: src/store/types.rs
tags: [domain, hosts, groups, identities, ssh-agent, certificates, import]
---

# Hosts, Groups & Identities

## Hosts

A host is the central entity. Storage and merge rules live in
[data model](../architecture/data-model.md#hybrid-host-model-srchostsloaderrs-srcssh);
the user-facing model:

- **Managed hosts** (`HostSource::Launcher`): full CRUD from the
  [host form](../workflows/tui.md) or [CLI](../workflows/cli.md). Fields include
  address, port, username, identity, ProxyJump, agent forwarding, transport
  (ssh/mosh), tags, notes, environment, per-host session-log and ping overrides
  (inherit / on / off), and an optional `secret_lookup` (Linux Secret Service,
  see [secrets](../security/secrets.md#secret-service-lookup)). Port, agent
  forwarding and transport may be unset, meaning "inherit from the group".
- **ssh_config hosts** (`HostSource::SshConfig`): imported and synced from the
  ssh config; metadata is editable, connection fields track the file
  (hot-reloaded by the [file watcher](../architecture/overview.md#file-watcher)).
- **Legacy aliases**: ssh_config entries with no DB row; surfaced read-only
  with metadata from `metadata.db`.
- A `name`, `address` or `username` starting with `-` is refused on every write
  path (host form, CLI, importers), and `ssh::safe_ssh_target` rewrites any such
  target that still reaches ssh to the `ssh://` form OpenSSH refuses.

Resolution goes through `HostResolver` / `SshConfigResolver`
(`src/ssh/resolver.rs`), which lists `Host` aliases (following `Include` up to
depth 16) and resolves effective options with `ssh -G`. `build_ssh_argv` /
`build_mosh_argv` (`src/ssh/host.rs`) build argv, and connect paths use
`app::util::resolved_session_argv` so group defaults apply.

**OS auto-detection** (`src/osinfo/`): on first connect a background worker runs
`cat /etc/os-release 2>/dev/null || uname -s` over ssh (BatchMode without a
secret, askpass with one); `parse_os` maps the output to a canonical id stored in
`hosts.os_icon`, and the host card renders a vendored logo (Braille set in
`assets/os_logos.json`, full-colour set for the zoomed panel in
`assets/os_logos_large.json`). Failures are silent by design.

## Groups, defaults & Favorites

`host_groups` are **nested** (`parent_id`) and a host can belong to **several
groups** via `host_group_memberships`; `hosts.group_id` stays in sync with the
primary membership. A `reserved` flag marks the built-in **Favorites** group,
found by flag and never by name, so a user's own "Favorites" group is never
hijacked; `f` toggles favorite. Groups are managed from the group manager
(`Shift+G`), the group form, or `sshub group ...`.

Groups can carry defaults for six connection fields (identity, username, port,
ProxyJump, transport, agent forwarding). A host's own value wins, then the most
specific group default across all its memberships, then the global default
(port 22, ssh, forwarding off). The host form shows inherited values as muted
placeholders; clearing a field restores inheritance. Tags and favorites never
inherit. Algorithm: `src/store/group_defaults.rs`, summarised in
[data model](../architecture/data-model.md#group-inherited-connection-defaults-srcstoregroup_defaultsrs).

## Identities

An identity (`src/store/identities.rs`) bundles a display name, username,
private key path and optional **certificate** path; a "Default" identity is
seeded. Hosts and groups reference identities for connection defaults. Key
passphrases and host passwords live in the credential store under
`identity:{id}` / `host:{id}` (prefixed per profile), see
[secrets](../security/secrets.md).

- **ssh-agent** (`src/ssh/agent.rs`): `detect_agent` lists loaded keys; the Keys
  tab adds (`p`) and removes (`r`) keys, and `ssh-add` receives the certificate
  alongside the key (`agent_add_argv`). CLI: `sshub identity agent-remove`.
- **Certificates** (`src/ssh/cert.rs`): every cert is read through `ssh-keygen -L`
  in one parser (`status_for`, oracle-tested). The Keys tab badges
  `Valid` / `Expired` / `NotYetValid` / `Missing` / `Unreadable` / `Mismatch`
  (cert not issued for that key), and the identity form shows key id,
  principals and validity.
- **Key badges** (`src/tui/screens/keys.rs`): a key path containing `-sk` /
  `_sk` shows `ed25519-sk` / `ecdsa-sk` (hardware security keys).
- **Key files** (`src/ssh/keyfile.rs`): `ssh-keygen -y` probing tells whether a
  key needs a passphrase and whether a stored one matches; the passphrase is fed
  through a 0600 secret file plus a 0700 helper script, never argv.
- **Key generation** (`src/app/keygen.rs`, `KeyAction::GenerateKey`, default
  `g` on the Keys tab): ed25519 (default path `~/.ssh/id_ed25519_sshub`) or
  RSA 4096 via `keyfile::generate_key_pair`.
- **Public-key push** (`src/app/push_key.rs`, `Shift+P` from the hosts list or
  the Keys tab): reads the public key (`ssh-keygen -y`, using a stored
  passphrase), then runs an idempotent remote append to
  `~/.ssh/authorized_keys` (`umask 077`, `grep -qxF` before `echo >>`) in an
  embedded session; the outcome is written to the audit log.

## Imports (`src/import/`)

- **Termius** (`termius_csv.rs`): `L00t.csv` plus `ssh_keys/` (format in
  `docs/termius-export-format.md`) become managed hosts and identities.
  Passwords and passphrases are re-stored with write verification; failures are
  reported as `keyring_failures` instead of being dropped. TUI `Shift+T`, CLI
  `sshub import --from termius PATH`; covered by `tests/e2e/termius_import.rs`.
- **PuTTY** (`putty.rs`): a Windows `regedit /e` `.reg` export or the Unix
  `~/.putty/sessions/` directory (the default path). CLI only:
  `sshub import --from putty [PATH]`.
- **mRemoteNG** (`mremoteng.rs`): `confCons.xml`, parsed by a hand-rolled XML
  reader (no XML crate). CLI only: `sshub import --from mremoteng PATH`.
- **ssh config**: `Shift+I` / `sshub import` (default `--from ssh`) and
  `sshub sync`; export with `Shift+E` / `sshub export` to `exported.conf`, never
  to the user's own config. `--dry-run` previews the file-based importers.

Every importer drops entries whose name, address or username starts with `-`
and counts them in its summary. The PuTTY, mRemoteNG and Termius fixtures are
agent-authored, not real exports (`docs/oracle-tests.md`).

## Change guidance

- Host CRUD invariants (dedupe with ssh_config sources, favorite semantics) are
  pinned by `tests/e2e/host_crud.rs`, `host_sort.rs`, `group_crud.rs`,
  `hybrid_compat.rs`, `ssh_config_sync.rs`; group defaults by tests in
  `src/store/group_defaults.rs`.
- Import and sync must never overwrite `source = launcher` rows and never write
  the user's own ssh config.
