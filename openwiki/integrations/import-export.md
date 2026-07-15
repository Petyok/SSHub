# Import / export & SSH config integration

SSHub works as a hybrid launcher: it reads `~/.ssh/config` as a host source, lets you import those aliases, and can export managed hosts back to ssh config format.

## Hot reload

`src/watcher.rs` watches the parent directory of `~/.ssh/config` (not the file itself). That way editors that save by writing a temp file and renaming it in place trigger reloads. Events are debounced for 300 ms before the app re-imports.

If you remove an ssh-config alias, the imported host disappears from SSHub on the next reload.

## Import from `~/.ssh/config`

- Hosts are imported with `ssh_config` as their `source`.
- Their connection fields are read-only in the UI.
- You can still toggle favorite, add tags, add notes, and audit them.
- `src/ssh/import.rs` runs `import_ssh_config` and hashes source blocks so stale aliases can be removed after reload.

Use `Shift+I` in the app.

## Export to `~/.ssh/config`

Managed launcher hosts can be exported back to `~/.ssh/config` with `Shift+E`.

- `src/ssh/export.rs` writes to a temp file and renames it atomically.
- Comments and unknown keys outside SSHub-managed blocks are preserved.
- Newlines in fields are flattened to avoid config-directive injection.

## Termius import

`src/import/` contains a Termius backup JSON importer (`Shift+T`). It parses the Termius export format and creates managed hosts/identities. `docs/termius-export-format.md` documents the expected fields.

## Resolver

`src/ssh/resolver.rs` implements `HostResolver`:

- `SshConfigResolver` shells out to `ssh -G` to resolve canonical host attributes, respect `Include` directives, and parse aliases.
- `FixtureResolver` is used in tests and reads fixtures from `tests/fixtures/`.

The resolver output is combined with DB rows in `app/mod.rs::reload_hosts()`. Launcher rows win on name collisions.

## Host-key handling

`src/ssh/probe.rs` records ssh handshake log lines. If a host key changed, SSHub can prompt the user to purge the stale `known_hosts` entry and reconnect.

## What to watch when changing integration code

- `src/ssh/import.rs` — changes here affect how aliases, `Include` directives, and multi-host blocks are parsed.
- `src/ssh/export.rs` — must preserve user comments and never inject directives through field newlines.
- `src/watcher.rs` — debounce and inode-rename semantics; watcher tests are sensitive on macOS.
- `tests/fixtures/ssh_config` and `tests/fixtures/ssh_g/` — fixtures used by `FixtureResolver`.
- `src/app/import.rs` — import/export prompt UI.

Relevant tests: `src/app/tests/`, `tests/e2e/mod.rs` import/export scenarios.
