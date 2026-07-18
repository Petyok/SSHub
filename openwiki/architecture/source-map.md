# Source map

Quick reference: where the code for each domain lives. Use this when you need to change behavior and want the canonical file.

## Entry points & boot

| Topic | File | Notes |
|-------|------|-------|
| CLI parsing, `db purge`, askpass short-circuit | `src/main.rs` | `--dry-run`, `--version`, `--help` |
| App bootstrap, terminal/headless loop, config watcher attach | `src/lib.rs` | `run()`, `run_app()` |
| Config load/save, XDG paths, env overrides | `src/config.rs` | `AppConfig`, `load_config()`, `data_dir()` |
| Data directory permissions | `src/secure_fs.rs` | `restrict_dir`, `restrict_file` |

## Headless CLI

| Topic | File | Notes |
|-------|------|-------|
| Subcommand detection + dispatch | `src/cli/mod.rs` | `is_subcommand`, `run_subcommand` |
| Shared `CliContext` (store, resolver, config, hosts) | `src/cli/context.rs` | `reload_hosts` |
| Flag/format parsing, exit-code helpers | `src/cli/parse.rs` | `parse_format`, `usage`, `fail`, `fail_code` |
| Per-command usage text | `src/cli/help.rs` | mirrors real flags |
| Plain/JSON output formatting | `src/cli/output.rs` | |
| Host commands (list/show/connect/resolve/add/edit/...) | `src/cli/host.rs` | script(1) log wrap, audit event |
| Group commands | `src/cli/group.rs` | |
| Identity commands | `src/cli/identity.rs` | `agent-remove` |
| Tunnel commands (list/create/start/stop/...) | `src/cli/tunnel.rs` | detached + `--foreground` |
| SFTP one-shot commands | `src/cli/sftp.rs` | ls/get/put/rm/mkdir/rename/chmod |
| Audit list/stats | `src/cli/audit.rs` | |
| tags / sync / import / export | `src/cli/inventory.rs` | |
| Shell completions | `src/cli/completions.rs` | static tree + host cache |
| Shared host filters | `src/cli/filter.rs` | |

## Headless host helpers (shared by TUI + CLI)

| Topic | File | Notes |
|-------|------|-------|
| Merged host list loader | `src/hosts/loader.rs` | `load_merged_hosts` (mirrors `App::reload_hosts`) |
| Legacy-alias -> launcher duplicate, identity match | `src/hosts/crud.rs` | `duplicate_legacy_to_launcher`, `match_identity_for_ssh_host` |

## Application state & orchestration

| Topic | File | Notes |
|-------|------|-------|
| Main `App` struct, event loop dispatch, reload hosts | `src/app/mod.rs` | Central state bag |
| Domain enums: `AppMode`, `SortMode`, `HostEntry`, settings rows | `src/app/types.rs` | |
| Key/mouse input translation | `src/app/keys.rs`, `src/app/mouse.rs` | Mode/tab dispatch |
| Shared utility functions: key parsing, ssh argv helpers | `src/app/util.rs` | `parse_keyspec`, `ssh_argv_for_entry` |
| Host CRUD (add/edit/duplicate/delete) | `src/app/host_crud.rs` | |
| Host form state & validation | `src/app/host_form.rs` | |
| Host detail panel + per-field edit | `src/app/host_detail.rs` | |
| Group management + nested group form | `src/app/groups.rs` | |
| Field picker widgets for host/group forms | `src/app/field_picker.rs` | |
| Identities tab state/forms | `src/app/identities.rs` | |
| Host list sorting, filtering, tree building | `src/app/hostlist.rs` | |
| Tags + tag-filter popup | `src/app/tags.rs` | |
| Audit tab state/filter | `src/app/audit.rs` | |
| Import/export prompts | `src/app/import.rs` | |
| Tunnels tab logic | `src/app/tunnels.rs` | |
| SFTP tab logic | `src/app/sftp.rs` | |
| Embedded session tab logic | `src/app/session.rs` | |
| App unit + e2e tests | `src/app/tests/` | `mod.rs`, `session.rs`, `sftp.rs`, `keybind.rs`, ... |

## Rendering

| Topic | File | Notes |
|-------|------|-------|
| Top-level render dispatcher | `src/tui/mod.rs` | `render()` |
| Dashboard layout (header/tabs/body/footer) | `src/tui/dashboard_layout.rs` | Zoom levels |
| Theme colors | `src/tui/theme.rs` | |
| Startup animation | `src/tui/animation.rs` | |
| Common text helpers | `src/tui/text.rs` | |
| Hosts tab screen | `src/tui/screens/hosts.rs` | |
| Tunnels tab screen | `src/tui/screens/tunnels.rs` | |
| Keys/identities screen | `src/tui/screens/keys.rs` | |
| Audit screen | `src/tui/screens/audit.rs` | |
| Help overlay | `src/tui/screens/help.rs` | |
| Host/group/identity/tunnel form renderers | `src/tui/screens/*_form.rs` | |
| Palette, tag filter, field pickers, session host picker | `src/tui/screens/*.rs` | |
| Header, footer, tab bar, status bar, detail panel | `src/tui/widgets/*.rs` | |

## SSH, resolver, import/export

| Topic | File | Notes |
|-------|------|-------|
| Public module surface | `src/ssh/mod.rs` | Re-exports |
| `ssh -G` resolver, host aliases, `~/.ssh/config` path | `src/ssh/resolver.rs` | `SshConfigResolver`, `HostResolver` |
| Import/sync from ssh config | `src/ssh/import.rs` | `import_ssh_config`, `sync_ssh_config_hosts` |
| Export launcher hosts to ssh config | `src/ssh/export.rs` | `export_launcher_hosts` |
| `ssh` argv builders | `src/ssh/host.rs` | `build_ssh_argv`, `build_ssh_alias_argv`, `SshHost` |
| Key file inspection, passphrase checks | `src/ssh/keyfile.rs` | `key_is_encrypted`, `passphrase_matches` |
| ssh-agent query | `src/ssh/agent.rs` | `AgentInfo` |
| Host-key / handshake probe log | `src/ssh/probe.rs` | `SshLogEntry` |

## Persistence

| Topic | File | Notes |
|-------|------|-------|
| `LauncherStore` + `open_default()` | `src/store/mod.rs` | |
| Domain types | `src/store/types.rs` | `ManagedHost`, `Identity`, `Tunnel`, `HostGroup`, ... |
| Host CRUD + search | `src/store/hosts.rs` | |
| Identity CRUD | `src/store/identities.rs` | |
| Tunnel CRUD | `src/store/tunnels.rs` | |
| Schema migrations | `src/store/migrate.rs` | `SCHEMA_VERSION` |
| Legacy metadata DB | `src/metadata/db.rs` | `MetadataDb`, `MetadataStore` |

## Embedded sessions

| Topic | File | Notes |
|-------|------|-------|
| Session lifecycle + `PendingSecret` | `src/session/mod.rs` | `SessionPhase`, `SessionConfig` |
| PTY runtime + byte pump | `src/session/pty.rs` | `PtyRuntime`, `PtyEvent` |
| VT100 parse + screen snapshot | `src/session/parser.rs` | `ParserState` |
| Rendering inside the dashboard | `src/session/render.rs` | |
| Mouse text selection | `src/session/render.rs` | `Selection` |
| `SSH_ASKPASS` helper | `src/session/askpass.rs` | `maybe_run_askpass` |
| Session keybind parsing | `src/session/keys.rs` | |
| PTY transcript logging | `src/session_log.rs` | `SessionLogWriter` |
| Connect workflow | `src/app/connect.rs` | Builds config, resolves secret, spawns session |

## Tunnels

| Topic | File | Notes |
|-------|------|-------|
| `TunnelManager`, reconnect, health, `ReconnectEvent` | `src/tunnel/mod.rs` | `stop_user`, `check_health`, `tick_reconnect` |
| Tunnel argv build, detached spawn, PID files, runtime state | `src/tunnel/spawn.rs` | `build_tunnel_argv`, `spawn_detached_tunnel`, `tunnel_runtime_state`, `TunnelRuntimeState` |
| Tunnel reconnect audit logging | `src/tunnel/audit.rs` | `log_tunnel_reconnect_events` |
| Tunnels tab app logic + event loop hook | `src/app/tunnels.rs`, `src/lib.rs` | `toggle_tunnel`, `tick_tunnels` |
| Tunnels tab render | `src/tui/screens/tunnels.rs` | |
| Reconnect settings overlay | `src/tui/screens/tunnel_reconnect.rs` | NEW |

## SFTP

| Topic | File | Notes |
|-------|------|-------|
| Pure UI state model | `src/sftp/model.rs` | |
| libssh2 transport | `src/sftp/transport.rs` | |
| Background worker + commands/events | `src/sftp/worker.rs` | `spawn_sftp_worker` |
| SFTP tab app logic | `src/app/sftp.rs` | |

## Keybindings, search, input, OS detection

| Topic | File | Notes |
|-------|------|-------|
| Keybinding definitions + defaults | `src/keybinds.rs` | `KeyAction` enum |
| Fuzzy search wrapper | `src/search.rs` | `HostSearch` |
| Cursor-aware text input widget | `src/text_input.rs` | |
| OS auto-detection worker + parse | `src/osinfo/detect.rs`, `parse.rs` | |
| OS logos + widget | `src/osinfo/logos.rs`, `widget.rs` | |

## Watcher & operations

| Topic | File | Notes |
|-------|------|-------|
| `~/.ssh/config` hot reload watcher | `src/watcher.rs` | `spawn_config_watcher` |
| Latency worker | `src/ping.rs` | |
| Credential store abstraction | `src/credentials.rs` | `PasswordStore` over keyring |
| Terminal launchers (legacy external mode) | `src/launcher/*.rs` | kitty / ghostty / custom |
| Common dev commands | `Justfile` | test, build, release, install |

## Tests

| Topic | File | Notes |
|-------|------|-------|
| E2E TestBackend scenarios | `tests/e2e/mod.rs` | |
| Binary smoke tests | `tests/smoke/*.rs` | help, dry-run, config load |
| Test fixtures | `tests/fixtures/` | ssh config, ssh -G output |
| Shared test helpers | `tests/support/` | `FixtureResolver`, `MockLauncher` |
| Unit tests inside app module | `src/app/tests/` | |
