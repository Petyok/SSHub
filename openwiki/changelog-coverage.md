---
type: Audit Notebook
title: Changelog to OpenWiki Coverage
description: Ledger of CHANGELOG.md features, fixes, and behavior changes reviewed against source evidence and mapped to OpenWiki pages.
resource: CHANGELOG.md
tags: [changelog, coverage, audit, openwiki]
---

# Changelog to OpenWiki Coverage

This notebook audits `CHANGELOG.md` entries against the source tree and links
each reviewed item to wiki pages. `Covered` means the behavior is documented and
cross-linked; `Pending` means it still needs a dedicated or more precise
treatment. Source references name where the behavior lives today.

## Coverage Ledger

| Release / entry | Wiki page(s) | Status | Notes |
| --- | --- | --- | --- |
| 0.17.7: Update notice and `Shift+U` install per channel | [Integrations](integrations/external-terminals.md#release-check-and-update-srcupdaters-srcappupdaters), [TUI dashboard](workflows/tui.md), [Runtime architecture](architecture/overview.md) | Covered | `src/update.rs` (`spawn_check`, `install_channel`, `install`), `src/app/update.rs`, `[updates] check`. |
| 0.17.7: Error popup title | [TUI dashboard](workflows/tui.md) | Covered | `AppMode::Notice`; title wording is renderer detail. |
| 0.17.6: DEC Special Graphics line drawing | [Sessions & SFTP](workflows/sessions-sftp.md) | Covered | `src/session/parser.rs` `XtermCompat`. |
| 0.17.6: Typing after ticking Remember | [Session tools](workflows/session-tools.md#interactive-authentication-modal), [Secrets](security/secrets.md#remember-is-opt-in) | Covered | `src/app/auth.rs`. |
| 0.17.5: Host passwords from KeePassXC (`secret_lookup`) | [Secrets](security/secrets.md#secret-service-lookup), [Data model](architecture/data-model.md), [Hosts, Groups & Identities](domain/hosts-identities.md), [Headless CLI](workflows/cli.md) | Covered | `credentials::secret_service_lookup`, schema v19, `app::util::resolve_pending_secret`. |
| 0.17.5: Issue forms | none | Covered | `.github/ISSUE_TEMPLATE/{bug_report,feature_request,config}.yml`; repository metadata, no wiki page needed. |
| 0.17.5: ProxyJump hosts connect again (#140) | [Secrets](security/secrets.md#host-key-policy), [Sessions & SFTP](workflows/sessions-sftp.md) | Covered | `LogLevel=DEBUG1` (`SSH_DEBUG_ARGS`), `InteractiveAuth::destination_pid`. |
| 0.17.5: Stored passwords stay off jump hosts outside the TUI (#141) | [Secrets](security/secrets.md#staged-file-srcsessionaskpassrs) | Covered | `askpass::asked_by_jump_hop`, `SSHUB_ASKPASS_OWNER`. |
| 0.17.5: Detached tunnels log in with a stored password (#144) | [Secrets](security/secrets.md#staged-file-srcsessionaskpassrs), [Tunnels](workflows/tunnels.md) | Covered | `AskpassSecret::outlive`, `SSHUB_ASKPASS_READ_ONCE`, `remove_outlived`. |
| 0.17.4: Ping probes can be switched off, per-host override | [Runtime architecture](architecture/overview.md#background-workers-and-channels), [Data model](architecture/data-model.md), [TUI dashboard](workflows/tui.md), [Headless CLI](workflows/cli.md) | Covered | `[ping] enabled`, `hosts.ping` (v18), `App::ping_targets`. |
| 0.17.4: Keyring opened only by secret-using commands (#135) | [Secrets](security/secrets.md#credential-store-srccredentialsrs) | Covered | `CliContext::password_store` (`OnceCell`). |
| 0.17.4: btop and xterm sequences, synchronized output | [Sessions & SFTP](workflows/sessions-sftp.md) | Covered | `src/session/parser.rs`. |
| 0.17.3: Faster startup, less idle work, dead code removed | [Runtime architecture](architecture/overview.md) | Covered | `src/ssh/probe.rs` now only defines `SshLogEntry`; dead probe classifier gone. Performance detail not documented. |
| 0.17.2: Remembering a credential is opt-in | [Secrets](security/secrets.md#remember-is-opt-in), [Session tools](workflows/session-tools.md#interactive-authentication-modal) | Covered | `SaveTarget::for_prompt`, modal `remember: false`. |
| 0.17.2: Unreadable credential store not shown as empty | [Secrets](security/secrets.md#credential-store-srccredentialsrs) | Covered | Host and identity forms. |
| 0.17.1: v16 rebuild cascade data loss fixed, v17 repair | [Data model](architecture/data-model.md#launcherdb-schema-srcstore) | Covered | `migrate_v15_to_v16` temp-table carry-over, `migrate_v16_to_v17`. |
| 0.17.1: Keyed identity passphrase outranks host password | [Secrets](security/secrets.md#which-secret-a-connection-uses) | Covered | `resolve_pending_secret` order. |
| 0.17.1: Pre-migration database snapshot | [Data model](architecture/data-model.md#launcherdb-schema-srcstore), [INSTRUCTIONS](INSTRUCTIONS.md) | Covered | `backup_before_migration`, `KEPT_BACKUPS = 3`. |
| 0.17.0: Session command suggestions (ghost text) | [Session tools](workflows/session-tools.md#ghost-text-suggestions), [Data model](architecture/data-model.md) | Covered | `src/app/ghost.rs`, `src/suggestions.rs`, `src/session/history.rs`, `command_history` table. |
| 0.17.0: Interactive SSH authentication (private askpass channel) | [Secrets](security/secrets.md#private-channel-srcsessionaskpass_channelrs), [Session tools](workflows/session-tools.md#interactive-authentication-modal) | Covered | `src/session/askpass_channel.rs`, `interactive_auth.rs`, `src/app/auth.rs`. |
| 0.17.0: Group-inherited connection defaults | [Data model](architecture/data-model.md#group-inherited-connection-defaults-srcstoregroup_defaultsrs), [Hosts, Groups & Identities](domain/hosts-identities.md#groups-defaults--favorites), [Headless CLI](workflows/cli.md) | Covered | `src/store/group_defaults.rs`, schema v16. |
| 0.17.0: Hardware security-key badges | [Hosts, Groups & Identities](domain/hosts-identities.md#identities) | Covered | `src/tui/screens/keys.rs`. |
| 0.17.0: SSH certificates on identities | [Hosts, Groups & Identities](domain/hosts-identities.md#identities) | Covered | `src/ssh/cert.rs`, `agent_add_argv`. |
| 0.17.0: In-app profile management | [Profiles](architecture/profiles.md#in-app-manager-and-switching-srcappprofile_managerrs), [Profiles & themes](workflows/profiles-themes.md) | Covered | `src/app/profile_manager.rs`, `lib.rs::apply_pending_profile`. |
| 0.17.0: Cross-profile transfers | [Profiles](architecture/profiles.md#cross-profile-transfers), [Data model](architecture/data-model.md), [Profiles & themes](workflows/profiles-themes.md#moving-data-between-profiles), [Headless CLI](workflows/cli.md) | Covered | `src/store/transfer.rs`, `src/cli/transfer.rs`; the CLI form is `sshub <entity> transfer`, not `sshub transfer`. |
| 0.17.0: Auth prompt scrolling, askpass closes on authenticated | [Secrets](security/secrets.md#private-channel-srcsessionaskpass_channelrs) | Covered | ratatui `unstable-rendered-line-info` feature in `Cargo.toml`. |
| 0.16.0: Session log browser | [Session tools](workflows/session-tools.md#session-log-browser), [Data model](architecture/data-model.md) | Covered | `src/log_browser.rs`, `src/app/log_browser.rs`, `log_bookmarks` (v15). |
| 0.16.0: Version badge shows install channel | [Integrations](integrations/external-terminals.md#release-check-and-update-srcupdaters-srcappupdaters) | Covered | `update::install_channel`. |
| 0.16.0: Command snippets library | [Session tools](workflows/session-tools.md#command-snippets), [Data model](architecture/data-model.md) | Covered | `src/store/snippets.rs` (v14), `src/app/snippets.rs`. |
| 0.16.0: Group deleted notice | [TUI dashboard](workflows/tui.md) | Covered | Minor UI fix. |
| 0.16.0: SFTP credentials for ssh_config-imported hosts (#120) | [Sessions & SFTP](workflows/sessions-sftp.md#sftp-srcsftp) | Covered | `app::util::sftp_ssh_host`: group defaults first, then `ssh -G` fills empty user, identity and certificate for `source = ssh_config` rows. |
| 0.15.2: Ctrl+U / Ctrl+W in form fields | [TUI dashboard](workflows/tui.md) | Covered | `src/text_input.rs`. |
| 0.15.2: Masked password hint, form sizing | [TUI dashboard](workflows/tui.md), [Secrets](security/secrets.md) | Covered | Reveal bind on the password row. |
| 0.15.2: Wheel and PageUp/PageDown on the alternate screen, tmux hint, selection drag, mouse forwarding rules | [Sessions & SFTP](workflows/sessions-sftp.md) | Covered | `session::keys::selects_locally`. |
| 0.15.2: Tunnel list re-synced from the store | [Tunnels](workflows/tunnels.md), [Runtime architecture](architecture/overview.md#background-workers-and-channels) | Covered | `sync_tunnels_from_store` every 2 s. |
| 0.15.2: Host form non-text rows no longer edit the address | [TUI dashboard](workflows/tui.md) | Covered | Regression fix. |
| 0.15.2: `sshub exec` shell quoting note | [Headless CLI](workflows/cli.md) | Covered | Help text only. |
| 0.15.1: Terminal answers cursor/status queries | [Sessions & SFTP](workflows/sessions-sftp.md) | Covered | `src/session/parser.rs`. |
| 0.15.1: PTY writes on their own thread | [Runtime architecture](architecture/overview.md#background-workers-and-channels) | Covered | `src/session/pty.rs` bounded `sync_channel`. |
| 0.15.0: `vendored` feature, `--no-default-features` | [Build & Release](operations/build-release.md#cargo-features-and-profiles-cargotoml) | Covered | `Cargo.toml` `[features]`. |
| 0.15.0: `sshub exec` | [Headless CLI](workflows/cli.md), [Secrets](security/secrets.md#host-key-policy) | Covered | `src/cli/exec.rs`. |
| 0.15.0: Shift+P before Identities visited; dashboard notices | [Hosts, Groups & Identities](domain/hosts-identities.md#identities), [TUI dashboard](workflows/tui.md) | Covered | `src/app/push_key.rs`. |
| 0.14.2: Theme choice persisted to the profile config | [Theme internals](architecture/themes.md#theme-picker-srcapptheme_pickerrs-srctuiscreenstheme_pickerrs) | Covered | `App::config_target`. |
| 0.14.2: Leading-dash host target guard (#101) | [Secrets](security/secrets.md#input-safety), [Data model](architecture/data-model.md), [Hosts, Groups & Identities](domain/hosts-identities.md#hosts) | Covered | `store::hosts::reject_option_like`, `ssh::safe_ssh_target`. |
| 0.14.0: Runtime theme system and `sshub theme` CLI | [Theme internals](architecture/themes.md), [Profiles & themes](workflows/profiles-themes.md#themes) | Covered | `src/theme/`, `src/cli/theme.rs`, `docs/theme-system.md`. The changelog's `~/.config/sshub/themes` is what the CLI reads; the TUI reads the profile's `themes/` in profile mode. |
| 0.14.0: Isolated profiles | [Profiles](architecture/profiles.md), [Data model](architecture/data-model.md), [Quickstart](quickstart.md#data-paths) | Covered | `src/profile/`. |
| 0.14.0: Transparency toggles replace `opaque_background` | [Data model](architecture/data-model.md#configuration-srcconfigrs), [Theme internals](architecture/themes.md) | Covered | `RETIRED_KEYS`. |
| 0.14.0: Known hosts manager follow-ups | [Known hosts manager](workflows/known-hosts.md) | Covered | `src/known_hosts.rs`. |
| 0.14.0: Quoted `Host` aliases | [Data model](architecture/data-model.md#hybrid-host-model-srchostsloaderrs-srcssh), [Testing](testing/strategy.md#oracle-tests) | Covered | Oracle test `listed_aliases_round_trip_through_real_ssh`. |
| 0.13.0: Known hosts manager and connect fingerprint | [Known hosts manager](workflows/known-hosts.md), [Secrets](security/secrets.md) | Covered | `src/known_hosts.rs`, `src/tui/screens/known_hosts.rs`. |
| 0.13.0: Help/H key migration | [TUI dashboard](workflows/tui.md) | Covered | `src/keybinds.rs`, `src/config.rs`. |
| 0.13.0: OSC 52 PTY relay | [Sessions & SFTP](workflows/sessions-sftp.md), [Secrets](security/secrets.md#clipboard-relay-boundary) | Covered | Visibility boundary, 64 KiB cap, queue of 8, read refusal. |
| 0.13.0: Terminal-stream demo recording | [Integrations](integrations/external-terminals.md#demo-pipeline-demo) | Covered | `demo/record.py`. |
| 0.13.0: SFTP local path display | [Sessions & SFTP](workflows/sessions-sftp.md) | Covered | Local `$HOME` shown as `~`. |
| 0.11.0: Session switcher/local shell/shared picker | [Sessions & SFTP](workflows/sessions-sftp.md), [TUI dashboard](workflows/tui.md) | Covered | `src/app/session_picker.rs`, `src/app/local_shell.rs`. |
| 0.11.0: Searchable Help/keybinding editor | [TUI dashboard](workflows/tui.md) | Covered | |
| 0.11.0: Secret reveal/copy/delete and keyring fallback | [Secrets](security/secrets.md) | Covered | |
| 0.11.0: Ad-hoc connect | [TUI dashboard](workflows/tui.md), [Secrets](security/secrets.md#input-safety) | Covered | `src/app/adhoc.rs`. |
| 0.11.0: Public-key push and key generation | [Hosts, Groups & Identities](domain/hosts-identities.md#identities) | Covered | `src/app/push_key.rs`, `src/app/keygen.rs`. |
| 0.11.0: npm installation | [Integrations](integrations/external-terminals.md#npm-packaging-npm), [CI & automation](operations/ci-cd.md#releaseyml-binaries-npm-cratesio) | Covered | `npm/build.sh`, release `npm` job. |
| 0.11.0: UI motion and panel/layout fixes | [Runtime architecture](architecture/overview.md#render-pipeline-srctui), [TUI dashboard](workflows/tui.md) | Covered | `src/tui/tween.rs`, `src/tui/blit.rs`, `TAB_ANIM`; per-animation behavior is not catalogued. |
| 0.11.0: SFTP two-server/queue/dotfiles/parent and failure behavior | [Sessions & SFTP](workflows/sessions-sftp.md) | Covered | |
| 0.11.0: release profile and askpass upgrade fix | [Build & Release](operations/build-release.md#cargo-features-and-profiles-cargotoml), [Secrets](security/secrets.md#ssh_askpass) | Covered | `askpass::helper_exe`. |
| 0.11.0: remaining session-strip, agent-panel, cursor-key, and SFTP fixes | [TUI dashboard](workflows/tui.md), [Sessions & SFTP](workflows/sessions-sftp.md) | Pending | Regression behavior remains distributed rather than catalogued. |
| 0.10.0: Broadcast commands | [Broadcast](workflows/broadcast.md) | Covered | `src/broadcast/mod.rs`, `src/app/broadcast.rs`. |
| 0.10.0: Panel focus/zoom | [TUI dashboard](workflows/tui.md) | Covered | `PanelId`, `Alt+Arrows`, `z`. |
| 0.10.0: PuTTY/mRemoteNG import | [Hosts, Groups & Identities](domain/hosts-identities.md#imports-srcimport) | Covered | `src/import/putty.rs`, `src/import/mremoteng.rs`, `sshub import --from`. |
| 0.10.0: Headless CLI | [Headless CLI](workflows/cli.md) | Covered | |
| 0.10.0: external launcher removal | [Integrations](integrations/external-terminals.md#external-terminal-launcher-status) | Covered | No `TerminalLauncher` remains in `src/` or `tests/`. |
| 0.9.0 to 0.8.0: transport, logging, tunnels, SFTP operations, inputs, version label | [Sessions & SFTP](workflows/sessions-sftp.md), [Tunnels](workflows/tunnels.md), [TUI dashboard](workflows/tui.md), [Secrets](security/secrets.md) | Covered | |
| 0.7.0 to 0.5.x: SFTP, groups, OS detection, selection, clipboard paste, log rendering | [Sessions & SFTP](workflows/sessions-sftp.md), [Hosts, Groups & Identities](domain/hosts-identities.md), [TUI dashboard](workflows/tui.md) | Covered | |
| 0.4.0: AGPL relicensing | [Quickstart](quickstart.md) | Pending | License is authoritative in `LICENSE`; release-history detail not recorded. |
| 0.3.0 to 0.1.0: TUI foundation, config/import/hot reload, embedded sessions, initial launcher | [Quickstart](quickstart.md), [Runtime architecture](architecture/overview.md), [Data model](architecture/data-model.md), [Hosts, Groups & Identities](domain/hosts-identities.md), [Testing](testing/strategy.md) | Covered | |

## Audit protocol

1. Read every release and `Unreleased` section in `CHANGELOG.md`.
2. Split entries by feature or behavior change and inspect implementation plus relevant tests.
3. Link each reviewed entry to exact concept pages; mark `Pending` only when the current wiki is materially incomplete.
4. Keep this notebook's run metadata and unresolved list current.

## Last audit

- `gitHead`: `0ffbc22` (v0.17.7)
- `auditedAt`: `2026-10-12`
- `method`: manual refresh, every fact checked against `src/`
- `unresolved`: 0.11.0 remaining regression fixes; 0.4.0 relicensing history
