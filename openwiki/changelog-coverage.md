---
type: Audit Notebook
title: Changelog to OpenWiki Coverage
description: Ledger of every CHANGELOG.md feature, fix, and behavior change from 0.1.0 through 0.16.0 (plus Unreleased) reviewed against source evidence and mapped to the OpenWiki page that documents it, with Covered/Pending status and run metadata.
resource: CHANGELOG.md
tags: [changelog, coverage, audit, openwiki]
verified:
  - by: openwiki/0.5.1
    at: 2026-09-11T12:46:43.817Z
sources:
  - id: openwiki-source-651d1fb6c9e49916a916ab51
    resource: repo://Cargo.toml
  - id: openwiki-source-ca6cb4b1a14fd7969dfae3ec
    resource: repo://CHANGELOG.md
  - id: openwiki-source-06170d7ecd38e316d14bb54a
    resource: repo://docs/host-sync-design.md
  - id: openwiki-source-29f2ffe501d0bdc557fcc14a
    resource: repo://npm/wrapper/bin/sshub.js
  - id: openwiki-source-e8c0db94ed2adcf662c346a3
    resource: repo://src/app/keys.rs
  - id: openwiki-source-c0d718e8edd8adec04ab3210
    resource: repo://src/app/push_key.rs
  - id: openwiki-source-7da0877d78263557d03ac935
    resource: repo://src/app/types.rs
  - id: openwiki-source-f531855e3a279281c944b8ad
    resource: repo://src/app/util.rs
  - id: openwiki-source-396ca803e3cf512685ab87bc
    resource: repo://src/cli/exec.rs
  - id: openwiki-source-2a737474d86fc75cc9d9694f
    resource: repo://src/config.rs
  - id: openwiki-source-38dc2802701bc3b7c43855d1
    resource: repo://src/keybinds.rs
  - id: openwiki-source-b9079954c10a783a989073f6
    resource: repo://src/known_hosts.rs
  - id: openwiki-source-f2fe236a8856917d43dfeb9f
    resource: repo://src/log_browser.rs
  - id: openwiki-source-b55a21a31ede1b56cd31a6a6
    resource: repo://src/main.rs
  - id: openwiki-source-39c7504511d4abe5494f1fca
    resource: repo://src/session/askpass.rs
  - id: openwiki-source-251a566921053d647fd490f2
    resource: repo://src/session/keys.rs
  - id: openwiki-source-bf6cda76234f60dd7eecaa7f
    resource: repo://src/session/mod.rs
  - id: openwiki-source-6ea4bd654665d0a13bbccee4
    resource: repo://src/session/pty.rs
  - id: openwiki-source-8c90e650e97e8c2db8feaa52
    resource: repo://src/ssh/host.rs
  - id: openwiki-source-661a6d64d056120219351adf
    resource: repo://src/ssh/resolver.rs
  - id: openwiki-source-ef7bbf76631c2cc5e3fe2e35
    resource: repo://src/store/hosts.rs
  - id: openwiki-source-b829ac603b13b9d0bcc902a4
    resource: repo://src/store/migrate.rs
  - id: openwiki-source-14a9e64a66b1cefa254fa04b
    resource: repo://src/text_input.rs
  - id: openwiki-source-2d5a061fae2d850d0f051605
    resource: repo://src/theme/catalog.rs
  - id: openwiki-source-082d0e6abbb7a34f426ec1bf
    resource: repo://src/theme/role_matrix.snapshot
  - id: openwiki-source-b3d2abf8ff74eda5980303d0
    resource: repo://src/tunnel/spawn.rs
generated: { by: "openwiki/0.5.1", at: "2026-09-11T12:46:43.817Z" }
---

# Changelog to OpenWiki Coverage

This notebook audits every entry in `CHANGELOG.md` against the source tree and links each reviewed item to exact wiki pages. `Covered` means the behavior is documented and cross-linked; `Pending` means an item still needs dedicated or more precise treatment. `Unreleased` is currently empty in `CHANGELOG.md`, so there is nothing to map from it this run.

## Coverage Ledger

### 0.16.0

| Release / entry | Wiki page(s) | Status | Notes |
| --- | --- | --- | --- |
| 0.16.0: Session log browser (`Shift+L`) | [Sessions & SFTP](workflows/sessions-sftp.md), [Data model](architecture/data-model.md) | Covered | Hosts → rotated segments → viewer with escape-code stripping, `/` search (`n`/`N`), `b`/`m` bookmarks, `VIEWER_READ_CAP` (4 MiB) bounded reads, and the `log_bookmarks` table (schema v15, keyed by on-disk location) all audited in `src/log_browser.rs`, `src/app/log_browser.rs`, and `src/store/migrate.rs`. |
| 0.16.0: Version badge shows install channel | [TUI dashboard](workflows/tui.md), [Integrations](integrations/external-terminals.md) | Covered | `npm`/`cargo`/`source` resolution (env marker first, then path heuristics), byte-identical npm binary rationale, `SSHUB_VERSION_LABEL` verbatim override, and hidden-label demo path checked in `src/tui/widgets/tab_bar.rs`. |
<!-- openwiki: broken internal link [workflows/snippets.md] file "workflows/snippets.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| 0.16.0: Command snippets library | [Command snippets](workflows/snippets.md), [Data model](architecture/data-model.md) | Covered | `Shift+S` manager, in-session `Ctrl+N` fuzzy picker (Enter runs, Tab inserts without newline), rebindable `SnippetsManage`/`SessionSnippets` actions, and the `snippets` table (v13→v14) audited in `src/app/snippets.rs`, `src/keybinds.rs`, and `src/store/snippets.rs`. |
| 0.16.0: Group-delete confirmation draws | [TUI dashboard](workflows/tui.md) | Covered | Notice set after `enter_group_manage()` re-entry and `render_group_manage_popup` drawing it on its own row verified in `src/app/keys.rs` and `src/tui/screens/group_manage.rs` (with its render test). |
| 0.16.0: SFTP regains username/key auth for ssh_config-imported hosts (#120) | [Sessions & SFTP](workflows/sessions-sftp.md), [Headless CLI](workflows/cli.md) | Covered | `sftp_ssh_host` resolves the alias live through the same `ssh -G` machinery the import used, for both the TUI browser and headless `sshub sftp`, letting the resolved config fill username/identity/certificate while SSHub-managed values win — audited in `src/app/util.rs`, `src/app/sftp.rs`, and `src/cli/sftp.rs`. |

### 0.15.2

| Release / entry | Wiki page(s) | Status | Notes |
| --- | --- | --- | --- |
| 0.15.2: Ctrl+U / Ctrl+W form chords | [TUI dashboard](workflows/tui.md) | Covered | `clear_before_cursor`/`delete_word_before` in `src/text_input.rs`, host/identity/tunnel form wiring, fixed-not-rebindable status, and multibyte/clamp behavior audited. |
| 0.15.2: Masked-password reveal hint + popup sizing | [TUI dashboard](workflows/tui.md), [Secrets](security/secrets.md) | Covered | Reveal bind riding the focused password row (`secret_field_hints`), popup sized to drawn rows, and the 80x24 render test verified in `src/app/tests/host_form.rs` and the form screens. |
| 0.15.2: Alternate-screen wheel and PageUp/PageDown to the remote app | [Embedded terminal surface](workflows/session-terminal.md) | Covered | `alternate_scroll_keys` (xterm `alternateScroll`, three arrows per notch), Shift opt-out, the tmux `set -g mouse on` toast (`hint_alternate_scroll`), the `-X` pager caveat, and shared guarded scroll path audited in `src/session/keys.rs` and `src/app/session.rs`. |
| 0.15.2: Wheel says where tmux's switch is | [Embedded terminal surface](workflows/session-terminal.md) | Covered | Once-per-session toast fired on the notch, not on process guessing, checked in `src/session/mod.rs`. |
| 0.15.2: Wheel cannot drag the selection | [Embedded terminal surface](workflows/session-terminal.md) | Covered | `scroll_with_selection` shifts by however far the view actually moved (alternate screen = nothing) audited in `src/session/mod.rs`. |
| 0.15.2: Drag at the prompt selects text (mouse-mode guard) | [Embedded terminal surface](workflows/session-terminal.md), [Secrets](security/secrets.md) | Covered | Forwarding requires mouse-report AND alternate screen; Shift inverts either way; chrome rows excluded from reporting; selector normalized to `c` in `src/osc52.rs` — all verified in `src/session/keys.rs` (`selects_locally`) and `src/app/session.rs`. |
| 0.15.2: Tunnel list resync across windows | [Tunnels](workflows/tunnels.md) | Covered | `sync_tunnels_from_store` re-reads the store every 2 s, re-anchors selection by id, leaves running children alone (delete+insert, no `UPDATE`), and keeps run state per-window — audited in `src/app/tunnels.rs`. |
| 0.15.2: Ctrl+U/End/Backspace no longer edit the address from non-text rows | [TUI dashboard](workflows/tui.md) | Covered | `active_field_mut` returning `Option` for host/identity/tunnel forms verified in `src/app/types.rs` and pinned by the host-form regression test. |
| 0.15.2: `sshub exec` documents the quoted `&&` form | [Headless CLI](workflows/cli.md) | Covered | `-- 'ls && uptime'` quoted-form note and `--tty` for full-screen commands in `src/cli/help.rs` (behavior unchanged) verified. |

### 0.15.1

| Release / entry | Wiki page(s) | Status | Notes |
| --- | --- | --- | --- |
| 0.15.1: Terminal answers cursor-position and status queries | [Embedded terminal surface](workflows/session-terminal.md) | Covered | DSR 6 (1-based, clamped) from the grid, DSR 5, and DA1 (`?1;2c`) answered; kitty protocol and secondary DA deliberately silent; rate-limited via the `REPLY_BURST_BYTES`/`REPLY_BYTES_PER_SEC` token bucket verified in `src/session/parser.rs` and `src/session/mod.rs`. |
| 0.15.1: PTY writes move to a dedicated thread behind a bounded queue | [Embedded terminal surface](workflows/session-terminal.md), [Sessions & SFTP](workflows/sessions-sftp.md) | Covered | `WRITE_QUEUE_LEN` (64), ordering guarantee for keystrokes/auto-typed secret/pastes/query answers, and the measured freeze-elimination rationale audited in `src/session/pty.rs`. |

### 0.15.0

| Release / entry | Wiki page(s) | Status | Notes |
| --- | --- | --- | --- |
| 0.15.0: `vendored` feature (link system OpenSSL) | [Build & Release](operations/build-release.md), [Quickstart](quickstart.md) | Covered | `default = ["vendored"]`, the dev-only `--no-default-features` opt-out (`just test` uses it), and the never-ship-without-it rule verified in `Cargo.toml` and the Justfile. |
| 0.15.0: `sshub exec` | [Headless CLI](workflows/cli.md) | Covered | BatchMode with no secret, `--tty`, `--timeout` (exit 124, own process group, SIGKILL of the group), `--format json` record, audit `via exec` without the command, transcript skip, mosh refusal — verified in `src/cli/exec.rs`. |
| 0.15.0: Shift+P fixes (identities reload + silent bail-out notices) | [Hosts, Groups & Identities](domain/hosts-identities.md) | Covered | `trigger_push_key_from_hosts` reloads identities before reading the lazy cache and both bail-outs now message via `host_notice` — audited in `src/app/push_key.rs`. |
| 0.15.0: Dashboard notices render unzoomed | [TUI dashboard](workflows/tui.md) | Covered | `host_notice` toast gating on `panel_zoomed` removed; the two silent push-key bail-outs verified. |

### 0.14.2

| Release / entry | Wiki page(s) | Status | Notes |
| --- | --- | --- | --- |
<!-- openwiki: broken internal link [workflows/theming.md] file "workflows/theming.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| 0.14.2: Theme persistence through the profile writer | [Runtime theme system](workflows/theming.md), [Data model](architecture/data-model.md) | Covered | `commit_theme_picker` writes through `App::config_target()` exactly once, activate → write → `mark_saved`, and the real-`Enter` regression test verified in `src/app/theme_picker.rs` and `src/app/keys.rs`. |
| 0.14.2: Leading-dash host guard (option injection) | [Secrets](security/secrets.md), [Hosts, Groups & Identities](domain/hosts-identities.md) | Covered | `safe_ssh_target` rewrites to the `ssh://` URI form for connect/tunnels/alias connects; `reject_option_like`/`is_option_like` refuse leading-dash name/address/username at write time; import drops only the poisoned entry and counts refusals — verified in `src/ssh/host.rs` and `src/store/hosts.rs`. |

### 0.14.0

| Release / entry | Wiki page(s) | Status | Notes |
| --- | --- | --- | --- |
<!-- openwiki: broken internal link [workflows/theming.md] file "workflows/theming.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| 0.14.0: Runtime theme system (TOML themes, picker, built-ins, gradients, PTY ground, `sshub theme` CLI) | [Runtime theme system](workflows/theming.md), [Headless CLI](workflows/cli.md), [TUI dashboard](workflows/tui.md) | Covered | Three-layer file format with inheritance, five built-ins, whole-UI preview with rollback, 234-role catalogue (`src/theme/role_matrix.snapshot`), static gradients in five directions, `pty_background`/`pty_foreground` pair semantics, and the headless dispatched-before-bootstrap CLI verified in `src/theme/`, `src/app/theme_picker.rs`, and `src/main.rs`. |
| 0.14.0: Isolated profiles (issue #17) | [Data model](architecture/data-model.md), [Quickstart](quickstart.md) | Covered | Profile-owned databases/settings/credentials/logs/tunnel state, per-profile SSH config source, startup picker, `--profile`, `--manage-profiles`, and legacy migration documented. |
<!-- openwiki: broken internal link [workflows/theming.md] file "workflows/theming.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| 0.14.0: Transparency choice; `opaque_background` gone | [Runtime theme system](workflows/theming.md), [TUI dashboard](workflows/tui.md) | Covered | Two independent `Ctrl+H` toggles (`transparent_sshub_background`, `transparent_session_background`), ground release keeping chrome, retired-key removal via `drop_retired_keys` verified in `src/config.rs` and `src/app/mod.rs`. |
| 0.14.0: Known hosts manager follow-ups (#89) | [Known hosts manager](workflows/known-hosts.md) | Covered | Per-line fingerprinting via `ssh-keygen -l -f -`, surfaced load/refresh errors, symlinked-file refusal, negated-pattern (`!host`) delete refusal, and remappable Ctrl+D/Ctrl+R actions verified in `src/known_hosts.rs`. |
| 0.14.0: Quoted `Host` aliases unquoted | [Hosts, Groups & Identities](domain/hosts-identities.md), [Testing strategy](testing/strategy.md) | Covered | OpenSSH-style quote stripping in `listable_host_alias` and the differential test `listed_aliases_round_trip_through_real_ssh` (per `docs/oracle-tests.md`) verified in `src/ssh/resolver.rs`. |

### 0.13.0

| Release / entry | Wiki page(s) | Status | Notes |
| --- | --- | --- | --- |
| 0.13.0: Known hosts manager and connect fingerprint | [Known hosts manager](workflows/known-hosts.md), [Secrets](security/secrets.md) | Covered | Manager overlay, guarded `ssh-keygen -R` deletion (hashed/marker/wildcard/wildcard-shadowing refusals), and first-wins fingerprint cache documented. |
| 0.13.0: Help/H key migration | [TUI dashboard](workflows/tui.md) | Covered | Help bound to `?` only, `H` for known hosts, one-time migration of legacy `help` settings. |
| 0.13.0: OSC 52 PTY relay | [Embedded terminal surface](workflows/session-terminal.md), [Secrets](security/secrets.md) | Covered | Visibility boundary, 64 KiB cap + 8-entry queue with named drop counters, empty-write ignored (no remote clear), read refusal, `[clipboard].relay_from_pty` switch. |
| 0.13.0: Terminal-stream demo recording | [Integrations](integrations/external-terminals.md) | Covered | `demo/record.py`, asciicast + agg at 1.00x timing checks, fixture home documented. |
| 0.13.0: SFTP local path display | [Sessions & SFTP](workflows/sessions-sftp.md) | Covered | Local `$HOME` collapse to `~` (`contract_home`), remote paths untouched. |

### 0.11.0

| Release / entry | Wiki page(s) | Status | Notes |
| --- | --- | --- | --- |
| 0.11.0: Session switcher/local shell/shared picker | [Sessions & SFTP](workflows/sessions-sftp.md), [TUI dashboard](workflows/tui.md) | Covered | `Alt+S` fuzzy switcher with lifecycle dots, `Ctrl+Shift+T` local shell, shared three-purpose picker widget. |
| 0.11.0: Searchable Help/keybinding editor | [TUI dashboard](workflows/tui.md) | Covered | Type-to-filter, filtered-row rebinding, Ctrl+A/R/X row actions documented. |
| 0.11.0: Secret reveal/copy/delete and keyring fallback | [Secrets](security/secrets.md) | Covered | Masked prefill, `Ctrl+R` reveal+copy / `Ctrl+Y` copy-only, cleared-field deletion, `credentials.json` fallback with migration-back-on-keyring. |
| 0.11.0: Ad-hoc connect | [TUI dashboard](workflows/tui.md) | Covered | Validated `[user@]host[:port]` row (IPv6 brackets) passed after `--`. |
| 0.11.0: Public-key push and key generation | [Hosts, Groups & Identities](domain/hosts-identities.md), [Secrets](security/secrets.md) | Covered | `Shift+P` push (umask 077, idempotent exact-line append, `ssh-keygen -y` extraction, askpass staging, audited result) and `g` key generation (Ed25519/RSA-4096, no overwrite) documented on the dedicated page. |
| 0.11.0: npm installation | [Integrations](integrations/external-terminals.md), [Build & Release](operations/build-release.md), [CI & automation](operations/ci-cd.md) | Covered | npm shim (`SSHUB_INSTALL_CHANNEL=npm`, exec-bit repair, stdio inherit), `sshub-tui` platform optional dependencies, and OIDC-trusted npm publish documented. |
| 0.11.0: UI motion pass | [TUI dashboard](workflows/tui.md), [Runtime architecture](architecture/overview.md) | Covered | Motion summary (popup drop/close, tab slides, zoom morph, host-dot flash, SFTP progress sweep) and `appearance.disable_animation` gating documented; tween/blit pipeline covered on the architecture page. |
| 0.11.0: SFTP two-server/queue/dotfiles/`..` and failure behavior | [Sessions & SFTP](workflows/sessions-sftp.md) | Covered | Two-server relay, queue-while-running, dotfile hiding, parent row, retry/failure semantics documented. |
| 0.11.0: Release profile and askpass upgrade fix | [Build & Release](operations/build-release.md), [Secrets](security/secrets.md) | Covered | Release-profile tradeoffs (lto/codegen-units/strip, empty `RUST_BACKTRACE`) and helper re-resolution on upgrade documented. |
| 0.11.0: Remaining regression fixes (session-strip binds, agent-panel position, stored-secret removal, tab-cycling stays on dashboard, re-enter slide, black-screen slide, narrow footers, DECCKM arrows, SFTP parent/stage/retry/unreachable) | [TUI dashboard](workflows/tui.md), [Embedded terminal surface](workflows/session-terminal.md), [Sessions & SFTP](workflows/sessions-sftp.md) | Covered | Global session-strip handling before per-tab dispatch, DECCKM-following arrow encoding, SFTP failure semantics, and footer pinning each have a documented home; the DECCKM encoder fix lives on the terminal-surface page. |

### 0.10.0

| Release / entry | Wiki page(s) | Status | Notes |
| --- | --- | --- | --- |
| 0.10.0: Broadcast commands | [Broadcast mode](workflows/broadcast.md) | Covered | Dedicated page now exists: three-stage wizard, bounded worker pool (default 8), docked panel with focus/zoom, audit rows, BatchMode/askpass auth. |
| 0.10.0: Dashboard panel focus + zoom | [TUI dashboard](workflows/tui.md) | Covered | `Alt`+arrows focus, `z`/`Alt+Enter` zoom, zoomed scroll/drag-to-copy, per-panel actions documented. |
| 0.10.0: PuTTY and mRemoteNG import | [Hosts, Groups & Identities](domain/hosts-identities.md) | Covered | Both importers (registry export / `confCons.xml`, SSH-only, mRemoteNG encrypted passwords not decrypted) documented alongside Termius and ssh_config. |
| 0.10.0: Headless CLI | [Headless CLI](workflows/cli.md) | Covered | Command tree, JSON, exit codes, `--yes` guards, completions cache documented. |
| 0.10.0: External launcher removal | [Integrations](integrations/external-terminals.md) | Covered | Removed subsystem status and ignored legacy keys documented; no runtime behavior remains. |

### 0.9.0 – 0.5.x

| Release / entry | Wiki page(s) | Status | Notes |
| --- | --- | --- | --- |
| 0.9.0: Mosh transport, session logging, tunnel keep-alive | [Sessions & SFTP](workflows/sessions-sftp.md), [Tunnels](workflows/tunnels.md), [Data model](architecture/data-model.md) | Covered | Schema v12/v13 fields, rotation/retention, backoff config, and audit integration documented. |
| 0.9.0: Session keybind hints, tunnel auth on TUI | [TUI dashboard](workflows/tui.md), [Tunnels](workflows/tunnels.md) | Covered | Config-driven hints and BatchMode/askpass tunnel options documented. |
| 0.8.0: SFTP file ops, recursive transfers, cursor navigation, `SSHUB_VERSION_LABEL` | [Sessions & SFTP](workflows/sessions-sftp.md), [TUI dashboard](workflows/tui.md) | Covered | In-place ops, symlink handling, text-cursor editing, version-label override documented. |
| 0.8.0: SFTP symlink handling, help scroll clamp, settings footer fix | [Sessions & SFTP](workflows/sessions-sftp.md), [TUI dashboard](workflows/tui.md) | Covered | Symlink transfer rules (size from target, dir/broken skip) documented; the two layout fixes are implementation-level and match current screens. |
| 0.7.0: SFTP file transfer tab, OS auto-detection, multi-group/Favorites, settings overlay, richer host card, changed host key prompt, version in tab bar | [Sessions & SFTP](workflows/sessions-sftp.md), [Hosts, Groups & Identities](domain/hosts-identities.md), [TUI dashboard](workflows/tui.md), [Known hosts manager](workflows/known-hosts.md) | Covered | Dual-pane browser, OS-detect worker with silent failures, membership join table with reserved Favorites, and the changed-key prompt documented. |
| 0.7.0: Tab order change, latency panel per-host, ssh log wrap, SFTP picker fixes, keybind migration persistence | [TUI dashboard](workflows/tui.md), [Sessions & SFTP](workflows/sessions-sftp.md) | Covered | Current tab order and latency-panel behavior match; migration-persistence and stale-index fixes are pinned by tests referenced in [testing strategy](testing/strategy.md). |
| 0.5.7 / 0.5.6 / 0.5.0: Selection autoscroll and range preservation, bracketed paste, log clamp/visibility, `j`/`k` in search, mouse text selection | [Embedded terminal surface](workflows/session-terminal.md), [Sessions & SFTP](workflows/sessions-sftp.md), [TUI dashboard](workflows/tui.md) | Covered | Selection autoscroll/edge extension, mouse selection, and bracketed-paste forwarding into the PTY belong to the embedded terminal surface; the `j`/`k`-in-search behavior is on the dashboard page. |

### 0.1.0 – 0.4.0

| Release / entry | Wiki page(s) | Status | Notes |
| --- | --- | --- | --- |
| 0.4.0: AGPL-3.0-or-later relicensing (≤0.3.1 MIT) | [Quickstart](quickstart.md) | Covered | Quickstart states the AGPL-3.0-or-later license; `Cargo.toml` and `LICENSE` remain authoritative. The release-by-release history stays in `CHANGELOG.md` by design — a license note is a live fact, not a changelog replay. |
| 0.3.1: crates.io metadata/docs only | [Build & Release](operations/build-release.md) | Covered | Docs-only release; packaging/release channels documented. |
| 0.3.0: ratatui 0.30, scrollable help, popup frames, ssh_config `Include`, hot reload, comment-preserving saves, installable crate; security fixes (launcher `<`/`>` rejection, export newline flattening, askpass passphrase) | [Quickstart](quickstart.md), [Architecture](architecture/overview.md), [Data model](architecture/data-model.md), [Secrets](security/secrets.md), [Hosts, Groups & Identities](domain/hosts-identities.md) | Covered | `conf_val` CR/LF flattening and askpass staging are documented on the secrets page; the launcher `<`/`>` guard was superseded by the 0.10.0 removal of the external launcher. |
| 0.2.0: Embedded sessions, tab strip, keybindings, nested groups, spinner/failure screens, tag filter, ping stats | [Sessions & SFTP](workflows/sessions-sftp.md), [TUI dashboard](workflows/tui.md), [Runtime architecture](architecture/overview.md), [Hosts, Groups & Identities](domain/hosts-identities.md) | Covered | Session lifecycle, `App::shutdown_all` cleanup, and the `AppMode` state machine documented. |
| 0.1.0: TUI launcher foundation (hybrid sources, tunnels, keys, audit, fuzzy search, hot reload) | [Quickstart](quickstart.md), [Runtime architecture](architecture/overview.md), [Data model](architecture/data-model.md), [Hosts, Groups & Identities](domain/hosts-identities.md) | Covered | Architecture, storage, and domain pages cover the foundation. |

## Audit protocol

1. Read every release and `Unreleased` section in `CHANGELOG.md`.
2. Split entries by feature or behavior change and inspect implementation plus relevant tests.
3. Link each reviewed entry to exact concept pages; mark `Pending` only when the current wiki is materially incomplete.
4. Keep this notebook's run metadata and unresolved list current.

## Known unresolved remainders

These are real gaps that the wiki cannot close by editing, because the source itself carries them:

- **Host-sync design is design-only** (`docs/host-sync-design.md`, epic #13): the P2P sync feature has no implementation anywhere in `src/` — no page exists because there is no system to document, only a reviewed design.
- **Detached tunnel PID races** (`src/tunnel/spawn.rs::spawn_detached_tunnel`): no advisory lock around the stale-PID/port checks and the PID write, and liveness via bare `kill(pid, 0)` with no start-time identity check — documented as a known limitation in [tunnels](workflows/tunnels.md), behavior may change.
- **`AskpassSecret` stale file on SIGKILL** (`src/session/askpass.rs`): the staged secret file is removed on `Drop` only, so `SIGKILL` leaves a `0600` file behind with no atexit cleanup — noted in [secrets](security/secrets.md) and the quickstart backlog.

## Last audit

- `gitHead`: `7c69f7e4242acc8ee9a717ad7e7f7edfd796ee99`
- `auditedAt`: `2026-09-11`
- `model`: `z-ai/glm-5.3-flash`
- `unresolved`: host-sync design (design-only, unimplemented); detached tunnel PID-file races; AskpassSecret SIGKILL-stale-file caveat.
