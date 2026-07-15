A code wiki for this local repository. Prioritize a concise quickstart, architecture overview, source map, key workflows, domain concepts, operations/runbook notes, testing guidance, and integration points. Inspect git history to understand reasoning behind code changes and the progression of the repository. Keep pages grounded in the repository structure and recent code changes. Prefer practical navigation for engineers over generic summaries.

## Invariant checklist (verify against code before merge)

When editing or reviewing OpenWiki pages, confirm these facts still match `src/`:

| Topic | Ground truth |
|-------|--------------|
| **Schema** | `SCHEMA_VERSION` **12** in `src/store/migrate.rs`. v12 adds `hosts.session_logging` and `auth_events.log_path`. |
| **Tabs** | Five dashboard tabs, `active_tab` **0–4**: 0 hosts, 1 SFTP, 2 tunnels, 3 identities (keychain), 4 audit. |
| **`auth_events` columns** | `host_name`, `username`, `via`, `status`, `note`, `log_path`, `created_at` (plus `id`). Detail line above audit table shows `note` + `log_path`; not per-row columns. |
| **Event loop order** | `run_terminal_loop` (`src/lib.rs`): `terminal.size()` → session `drain` + `resize` → render → `poll_keys_and_watcher` (50 ms poll; `handle_key` / `handle_mouse` / `handle_paste`; watcher, ping, SFTP, `probe_rx`, `os_detect_rx`, `tick_tunnels`, `refresh_auth_cache`). Resize from `terminal.size()`, not crossterm resize events. Headless = one `TestBackend` frame. |
| **Audit cache refresh** | `refresh_auth_cache()` — 10 s throttle in poll, respects filter/range. `refresh_audit_events()` — immediate on audit filter/range/tab switch. |
| **Tab key handlers** | `handle_key_normal` (0), `handle_key_sftp` (1), `handle_key_tunnels` (2), `handle_key_keychain` (3), `handle_key_audit` (4). |
| **Tunnel APIs** | `App::tick_tunnels()` → `TunnelManager::check_health` + `tick_reconnect` (`src/tunnel.rs`, `src/app/tunnels.rs`). |
| **Channels** | `probe_rx` = optional drain for `SshLogEntry` (SSH log panel; mostly fed by session/connect today); `os_detect_rx` = OS logo auto-detect. |

After substantive edits, update `openwiki/.last-update.json` (`gitHead`, `validatedAt`).
