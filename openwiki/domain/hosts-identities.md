---
type: Domain Concept
title: Hosts, Groups & Identities — host sources, nested groups, Favorites, and ssh-agent identities
description: Core SSHub domain concepts — hosts from two sources (managed launcher rows and read-only ~/.ssh/config), nested groups with a reserved Favorites group and M:N membership, identities wrapping SSH keys with ssh-agent integration, OS auto-detection, and Termius import.
resource: src/store/types.rs
tags: [domain, hosts, groups, identities, ssh-agent, termius]
---

# Hosts, Groups & Identities

## Hosts

A host is the central entity. Storage and merging rules live in [data model](../architecture/data-model.md#hybrid-host-model); the user-facing model:

- **Managed hosts** (`HostSource::Launcher`) — full CRUD from the [host form](../workflows/tui.md) or [CLI](../workflows/cli.md); fields include address, port, username, tags, description, environment, per-host session-logging override, and `transport` (ssh/mosh).
- **ssh_config hosts** (`HostSource::SshConfig`) — imported/synced from `~/.ssh/config`; editable metadata but the connection fields track the config file (hot-reloaded by the [file watcher](../architecture/overview.md#file-watcher)).
- **Legacy aliases** — ssh_config entries with no DB row; surfaced read-only with metadata from `metadata.db`.

Resolution always goes through `HostResolver` / `SshConfigResolver` (`src/ssh/resolver.rs`), which lists `Host` aliases (following `Include`, depth-capped at 16) and resolves effective options with `ssh -G`. `build_ssh_argv` / `build_mosh_argv` (`src/ssh/host.rs`) turn a resolved host into the spawn argv used by [embedded sessions](../workflows/sessions-sftp.md).

**OS auto-detection** (`src/osinfo/`): on first connect a background worker runs `cat /etc/os-release || uname -s` over ssh (BatchMode without a secret, askpass with one), `parse_os` maps it to a canonical id stored in `hosts.os_icon`, and the host card renders a vendored ANSI/Braille logo (`OsLogoWidget`). Failures are silent by design.

## Groups & Favorites

`host_groups` are **nested** (`parent_id`) and a host can belong to **multiple groups** via `host_group_memberships`. A `reserved` flag marks the built-in **Favorites** group — it's found by flag, never by name, so a user's pre-existing "Favorites" group is never hijacked; `f` toggles favorite and a ★ marker shows in the list. Managed from the group manager (`Shift+G`) or `sshub group …`. Shipped in 0.7.0 per `docs/superpowers/specs/2026-07-10-multi-group-favorites.md`.

## Identities

An identity (`src/store/identities.rs`) bundles a display name, username, and private key path; a "Default" identity is seeded. Hosts/groups reference identities for connection defaults. Secrets (key passphrases, host passwords) live in the OS keyring keyed `identity:{id}` / `host:{id}` — see [secrets](../security/secrets.md).

- **ssh-agent** (`src/ssh/agent.rs`) — wrappers over `ssh-add -l` / `-d`; the Keys tab shows loaded status and can add/remove keys (`Shift+A` / `r`; CLI: `sshub identity agent-remove`).
- **Key files** (`src/ssh/keyfile.rs`) — `ssh-keygen -y` probing detects whether a key needs a passphrase; the passphrase is fed through a staged 0600/0700 askpass script so it never appears in `ps` argv.
- **Probing** (`src/ssh/probe.rs`) — a background `ssh -v BatchMode` probe classifies stderr lines (auth methods, host-key state) for the detail panel. Per-host `ssh -v` probing was deliberately removed from connect because it "buried the events the user actually cares about" (`src/app/mod.rs`).

## Termius import (`src/import/termius_csv.rs`)

Imports Termius backups (`L00t.csv` + `ssh_keys/` directory — format documented in `docs/termius-export-format.md`) as managed hosts and identities. Passwords/passphrases are re-stored into the keyring with **write-verification**; failures surface as `keyring_failures` in the import report rather than silently dropping secrets. TUI: `Shift+T`; covered by `tests/e2e/termius_import.rs`.

## Change guidance

- Host CRUD invariants (dedupe with ssh_config sources, favorite semantics) are pinned by `tests/e2e/host_crud.rs`, `host_sort.rs`, `group_crud.rs`, `hybrid_compat.rs`, `ssh_config_sync.rs`.
- Import/sync must never overwrite `source=launcher` rows and never write the user's own `~/.ssh/config` (export goes to `exported.conf`).
