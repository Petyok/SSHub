# Changelog

All notable changes to SSHub are documented in this file.

## [Unreleased]

### Added

- **Stored passwords and passphrases are no longer write-only** (issue #60) - a
  saved secret showed as `(set)` and nothing else: you could not tell which one
  was stored, could not get it out, and editing meant retyping from scratch. The
  field now opens prefilled from the credential store, masked, with `Ctrl+R` to
  show and copy it and `Ctrl+Y` to copy without showing. Both say what they
  copied and never the value, the reveal drops as soon as the field is left, and
  the value stays out of every log, audit row and diagnostic. Both binds are
  rebindable in the keybinding editor, and the form's hint row names them while
  the field is focused, since a masked value gives no clue on its own.
- **Ad-hoc connect** - typing an unknown `[user@]host[:port]` into the fuzzy
  palette (`/`), IPv6 in brackets included, now offers a "connect without saving"
  row instead of reporting no matches, and Enter opens an embedded session
  straight to it. The destination is validated and passed after `--`, so a host
  that starts with a dash cannot turn into an ssh flag.
- **Local shell tab** - `Ctrl+Shift+T` opens a session tab running your login
  shell (`$SHELL`, falling back to `/bin/sh`), with the same detach and close
  behaviour as an ssh tab.
- **Push a public key to a host** (PR #16) - `Shift+P` from the hosts list picks
  an identity, or from the Keys tab picks a host, and installs that identity's
  public key into the remote `~/.ssh/authorized_keys`. The remote side runs under
  `umask 077`, creates the file if missing, and appends only when an exact
  matching line is not already there, so repeating it is harmless. The public key
  comes from the `.pub` file when one exists, otherwise `ssh-keygen -y` extracts
  it with the passphrase staged through askpass rather than argv. Authentication
  reuses the same stored-credential path as a normal connect, and the result is
  written to the audit log. The help overlay has been advertising this key since
  the feature was ceded to this PR; now it exists.
- **Generate SSH keys from the Keys tab** (PR #15) - `g` opens a form for the key
  type (Ed25519 or RSA-4096), passphrase, comment and target path. The passphrase
  reaches `ssh-keygen` through askpass rather than argv, the new key is registered
  as an identity immediately, and its passphrase goes to the credential store when
  one was set. Generation refuses to overwrite an existing key or its `.pub`.
- **Credentials survive a missing keyring** (PR #1) - where no Secret Service is
  reachable (WSL, Docker, a headless box), passwords and passphrases used to have
  nowhere to go. They now fall back to an owner-only `credentials.json` in the
  data directory, written through a `0600` temp file and an atomic rename, and the
  status bar says so rather than letting you assume the keyring took it. **The
  fallback file is plaintext**, since there is no keyring to encrypt against. Once
  a keyring shows up again, its contents migrate into it on the next launch and
  the file is removed, but only if every entry made it across.
- **Install from npm** (issue #51) - `npx sshub-tui` and `npm install -g sshub-tui`
  now work, installing a command still called `sshub`. The npm package ships the
  same prebuilt binaries the GitHub release does, delivered as platform-specific
  optional dependencies, so there is no build step and nothing is fetched from
  outside the registry. Prebuilt for Linux x64, macOS arm64 and macOS x64; every
  other platform keeps using `cargo install sshub`.
- **UI motion pass** (issue #35) - the interface moves instead of cutting
  between states. Popups drop in from off the top and are thrown back up on
  close; tab bodies slide; a zoomed panel morphs out of its grid slot; the
  full-screen host view slides in on connect and off on the way out; the SFTP
  tab slides between its picker, "connecting…" and browser states, with the two
  panes meeting in the middle and parting again. The host list scrolls under
  the cursor rather than jumping half a panel, its highlight wipes in, and a
  group's rows are revealed one at a time as it folds. Status is legible in
  motion too: a host's dot flashes when its ping class changes, a reconnecting
  tunnel's dot breathes, the header tally counts to its new values, a fresh
  ping reading grows into the latency graph, and the SFTP queue has a progress
  bar that sweeps between the worker's chunked updates. Everything is gated on
  the existing `appearance.disable_animation` reduced-motion toggle, and the
  frame rate only rises while something is actually animating.
- **SFTP between two servers** - the left pane can be pointed at a second host
  with `o` (and back to local files with `O`), so two servers sit side by side
  with their own connections, listings and file operations. Transfers between
  them are relayed through a local temp file, one leg at a time, since libssh2
  has no server-to-server copy; an item only leaves the queue once it lands on
  the far end, so a failure part-way keeps it for a retry.
- **SFTP queue stays open during a transfer** - files can be staged while the
  queue runs and roll into the next pass when the current one finishes. The
  local pane stays browsable meanwhile.
- **SFTP `..` row** - both panes list their parent directory as a selectable
  row, so walking up no longer depends on knowing about `Backspace`.

### Changed

- **SFTP hides dotfiles** (issue #44), with `.` to show them - in a home
  directory they were most of the listing, pushing what you came for below the
  fold. This changes what an existing install shows on first run, so the pane
  says how many entries it is holding back, and a search beginning with a dot
  (`.ssh`) lifts the hiding rather than reporting no matches. The toggle covers
  both panes and is remembered across restarts. The `..` row is exempt from the
  hiding, being a way out rather than an entry, but steps aside while searching
  so the cursor lands on a result.
- **Smaller release binaries** (issue #42) - release builds had no profile at
  all, so they shipped with the symbol table and without cross-crate
  optimisation. Adding `lto = "thin"`, `codegen-units = 1` and
  `strip = "symbols"` takes the Linux x86-64 binary from 14.8 MB to 11.6 MB
  (-21.8%), which every distribution channel carries: the release tarballs,
  `cargo install`, and anything that repackages them. `cargo test` and a plain
  `cargo build` are untouched, but every release build now takes noticeably
  longer -- including `just build` and `just install`, which build in release
  mode, and including every dependency, since `codegen-units = 1` applies across
  the whole graph. One trade to be aware of when reporting a crash: stripping
  the symbol table leaves `RUST_BACKTRACE` output empty in a released binary,
  so a backtrace worth sending has to come from a debug build.

### Fixed

- **Upgrading while running broke password auth until restart** (issue #63) - the
  askpass helper is this binary, found through `current_exe()`, and an upgrade
  replaces the file rather than writing into it, so the running process was left
  pointing at `<path> (deleted)`. ssh could not exec that, no stored secret was
  delivered, and the failure read as `Permission denied (publickey)`, which points
  at the server rather than at the upgrade. The path is now re-resolved when it
  disappears, so an in-place upgrade keeps working, and when the helper genuinely
  cannot be found the session log says why instead of leaving a rejected-key
  message to explain itself.
- **The agent panel printed over the identity cards** - its position was derived
  from whole card rows while the grid scrolls by lines, so mid-scroll it landed on
  a card and its own short fields left the card showing through, giving lines like
  `loaded keys 0n (no key)` and `agent socket (not set)────┘`. It is a fixed strip
  at the bottom of the tab now, with the grid sized to whole card rows, so the two
  cannot share a row whatever the scroll is doing. Sizing to whole rows also
  removes the sliver of a cut-off card that used to sit above the rule and slide
  around while everything else stayed still. The transient notice moved to the
  last row for the same reason.
- **A stored secret could not be removed** - clearing the password field meant
  "leave whatever is stored alone", because an empty field was the only signal
  the save path had. It now compares against what the store held when the form
  opened, so an emptied field deletes the entry, and the field masks while typing
  too, since it arrives carrying a real secret.
- **Cycling session tabs from the dashboard threw you into the session** (issue
  #49) - `Ctrl+[` / `Ctrl+]` there moved the selection *and* immediately opened
  the session full screen, so a key named "previous / next session tab" left the
  dashboard entirely, with no transition. It now moves the selection along the
  session strip and stays put; opening the selected session is what
  `Ctrl+Shift+S` is for, and the footer says so. The strip carries its highlight
  across instead of teleporting, reading the same travel state the full-screen
  tab bar uses. A chip collapsed into the `+N` overflow marker has nowhere to
  travel and is left alone, and reduced motion jumps straight to the target.
- **Entering a running session did not animate** - leaving one slid the session
  off to the right, but arriving cut straight to it, because the slide was armed
  on connect only. Re-entering from the dashboard now slides in the same way a
  fresh connect does; re-deriving the mode while already inside a session, when an
  overlay closes over it or a phase changes, does not replay it.
- **The session slide arrived over a black screen** - it blanked the columns it
  had not reached yet, so connecting flashed black before the host appeared. The
  dashboard is now snapshotted the way the session view already was, and the
  session slides over it.
- **Narrow footers dropped the wrong hints** - the pairs that say how to get out
  (`? help`, `q quit`) or back into a running session (`resume`) were pushed into
  the middle of the row by whatever was appended after them, panel-zoom hints,
  broadcast hints, session hints, and the middle is exactly what truncation eats.
  They are now moved to the end and pinned explicitly.
- **Arrow keys did nothing in Midnight Commander** (PR #56) - full-screen
  applications ask for application cursor mode (DECCKM) and then expect the
  cursor keys as SS3 sequences, while SSHub always sent the normal CSI ones. The
  embedded terminal tracked the mode all along; only the key encoder ignored it.
  Holding Shift looked like a workaround because modified cursor keys take a
  different encoding, but Midnight Commander read those as "select file". Arrows,
  `Home` and `End` now follow whatever mode the remote application asked for.
- **SFTP could not leave the login directory** - `Backspace` did nothing on a
  fresh remote pane, because the parent of the server-resolved `"."` is the
  empty path, which the server rejects as a listing target.
- **SFTP staged from the wrong pane** - `←` queued whatever the *remote* cursor
  sat on whichever pane had focus, so browsing locally and reaching for `←`
  queued an entry you weren't looking at, and queued it again after every local
  `cd`. The arrows still point at the destination; the source is now the
  focused pane.
- **A failed SFTP transfer retried itself forever** - a worker follows an error
  with its usual completion event, which the queue's auto-continue read as
  "start the next pass". Failed runs now stop and wait for the user, and both
  workers are told to cancel.
- **An unreachable SFTP host opened a blank browser** - it now fails into a
  modal popup, with a connect timeout instead of an open-ended wait.

## [0.10.0] - 2026-07-19

### Added

- **Broadcast mode** - run one command across a whole group or tag at once
  (issue #3). Pick a target from a menu, type the command, and review a dry-run
  preview (deselect hosts with `e`, edit the command with `c`) before it runs
  non-interactively over SSH on every host concurrently with a bounded worker
  pool (default 8). A live docked panel slides into the bottom-right corner and
  shows per-host status, exit code, and output with failures first; it joins the
  dashboard panel focus/zoom (`Alt`+arrows to focus, `z` to zoom for full
  per-host output), auto-dismisses on a countdown, and can be cancelled with `x`.
  Each host result is written to the audit log (with the error text on failure),
  and failed hosts pop animated error toasts. Authenticates with key/agent
  (`BatchMode`) and stored passwords (via `SSH_ASKPASS`, the same path a live
  session uses).
- **Dashboard panel focus + zoom** - focus any dashboard panel with `Alt`+arrows
  and zoom it to the full body with `z` or `Alt+Enter` (tmux-style), issue #18.
  Zoomed panels scroll, support drag-to-copy text selection (OSC52), and carry
  per-panel actions: connect to the selected host from the ping / recent panels,
  cycle the auth panel's filters, and remove the selected key from ssh-agent.
- **PuTTY and mRemoteNG import** - import saved sessions from a PuTTY registry
  export or an mRemoteNG `confCons.xml` into the host store, alongside the
  existing ssh_config and Termius import (issue #12).
- **Headless CLI** - drive the whole launcher without opening the TUI. New
  subcommands cover hosts (`list` / `show` / `connect` / `resolve` / `search` /
  `add` / `edit` / `rename` / `delete` / `duplicate`, with top-level `connect`,
  `list`, and `groups` aliases), group CRUD, identity CRUD plus `agent-remove`,
  tunnels (`list` / `show` / `create` / `delete` / `start` / `stop`, and
  `start --foreground`), one-shot SFTP (`ls` / `get` / `put` / `rm` / `mkdir` /
  `rename` / `chmod`), the audit log (`list` / `stats`), `tags`, and
  `sync` / `import` / `export`. Machine-readable output with `--format json`
  where applicable (plain text stays the default). Exit codes follow a fixed
  convention: `0` success, `1` operational failure, `2` usage or bad flags.
  Destructive commands refuse to run without `--yes` (`db purge` keeps its
  `--yes-i-am-stupid` guard). `sshub completions bash|zsh|fish` emits a shell
  completion script, optionally caching the host-name list to a file with
  `--cache PATH` so completion stays fast on large inventories.

### Removed

- **External-terminal launcher** - removed the retired `TerminalLauncher`
  subsystem (Kitty / Ghostty / custom command launchers) and the `terminal` /
  `launch_command` config keys (issue #30). Embedded PTY sessions have been the
  only transport for a while, so this was dead code. Old `config.toml` files that
  still carry those keys keep loading fine; the keys are simply ignored.

## [0.9.0] - 2026-07-16

### Added

- **Mosh transport** — per-host `Transport` field (`ssh` or `mosh`) in the host form
  and detail panel. Embedded sessions use `mosh` when selected; tunnels and SFTP stay
  ssh-only. Graceful error when `mosh` is not installed.
  (Schema v13.)
- **Session logging** — opt-in capture of embedded SSH session PTY output to plain-text
  files under `~/.local/share/sshub/logs/<host-dir>/`, with rotated segment files inside
  (managed hosts use `{sanitized-name}-{id}`). Toggle globally in Settings (`Ctrl+H`) or
  per host in the host form (`inherit` / `on` / `off`). Size-based rotation and per-host
  retention cap are configurable in `config.toml`. The audit tab shows the log directory
  path for each connect event. Pure `~/.ssh/config` aliases without a launcher
  row may share a log directory when sanitized host names collide. **Warning:** logs
  capture everything echoed to the terminal, including passwords if they appear on
  screen. (Schema v12.)
- **Tunnel keep alive** — per-tunnel **Keep alive** toggle in the tunnel form
  (uses existing `auto_connect` column). Enabled tunnels start on app launch and
  automatically reconnect after unexpected exit with exponential backoff, jitter,
  and a capped retry count (`[tunnel_reconnect]` in `config.toml`, editable on the
  Tunnels tab with `R`: `max_attempts`, delays in seconds, stable time, jitter).
  Manual stop or kill disables the retry loop until the tunnel is started again
  (until the next app launch for keep-alive auto-start). The Tunnels tab shows
  `starting` / `reconnecting` / `gave up` with attempt counter; audit logs
  reconnect attempts. A tunnel must stay up for `stable_secs` (default 5s) before
  it counts as reconnected. Background ssh uses `ServerAliveInterval`,
  `ServerAliveCountMax`, and `TCPKeepAlive` so dead paths (e.g. VPN dropped) tear
  down instead of leaving a stale local listener.

### Fixed

- **Session keybind hints** — the connected-session header and dashboard footer
  now show your configured keybindings (from `config.toml` / the keybind editor)
  instead of hardcoded `Ctrl+T` / `Ctrl+D` defaults. Connecting-screen hints
  (`expand log`, `cancel`) follow the same config.
- **Tunnel auth on TUI** — background `ssh -N` tunnels no longer open `/dev/tty`
  for password prompts (which painted over the dashboard and stole mouse/keyboard
  input). Tunnels use `BatchMode=yes` when no stored credential is available
  (fail fast with an error in the Tunnels tab) and `SSH_ASKPASS` when a host or
  identity password is in the keyring.

## [0.8.0] - 2026-07-12

### Added

- **SFTP file operations** — manage files directly in the browser: delete a
  file or folder (`d`, recursive for non-empty directories), create a new folder
  (`n`), rename/move (`R`), and change permissions (`M`, octal chmod). Remote ops
  run on the background worker; local ops use the filesystem directly.
- **Recursive directory transfers** — staging a directory now uploads/downloads
  the whole tree, with a progress bar over the total bytes.
- **Cursor navigation in text fields** — `←`/`→` move the edit cursor, `Home`/`End`
  jump to the edges, and `Delete` removes the character ahead of the cursor, across
  every form and prompt (host, identity, group, tunnel, SFTP mkdir/rename, import).
  Previously the cursor was pinned to the end and only backspace worked.
- **`SSHUB_VERSION_LABEL`** — override the version shown in the tab bar: set it
  empty to hide the version, or to a custom string. (Unset keeps the build version.)

### Fixed

- **SFTP transfers follow symlinks properly** — a symlink to a file transfers
  with the target's size (progress no longer overshoots), and symlinks to
  directories or broken links are skipped instead of failing the whole queue.
- **Help overlay** — scrolling is clamped to the rendered content, so it no
  longer overshoots into blank space.
- **Settings overlay** — the footer hint no longer spills onto the popup border.

## [0.7.0] - 2026-07-10

### Added

- **SFTP file transfer** — a new tab with a dual-pane browser (local left / remote right): browse both sides, filter with `/`, stage uploads and downloads into a queue, and run them sequentially with a progress bar. Native libssh2 transport on a background thread, trust-on-first-use host-key verification, atomic temp-file + rename writes. Open it for the current SSH host with `Ctrl+Shift+F`; jump back to that host's session with `s`.
- **OS auto-detection** — a background probe on first connect detects the remote distro; the host card renders its logo as crisp Braille art in brand colors (font-logos + chafa), like Termius.
- **Multiple groups per host** — hosts can belong to several groups at once, picked via a multi-select checkbox in the host form. A reserved **Favorites** group with a ★ marker in the list, toggled with `f`. (Schema v11: `host_group_memberships` join table.)
- **Settings overlay** (`Ctrl+H`) — toggle an opaque background (for transparent terminals), OS logos, quit confirmation, and the startup animation.
- **Richer host card** — fact sheet with user@host:port, group, key/identity, tags, and last-connected, next to the OS logo.
- **Accept a changed host key** — when a server's fingerprint changes, a prompt offers to purge the stale `known_hosts` entry and reconnect.
- **Version shown in the tab bar.**

### Changed

- **Tab order** — SFTP is inserted as the 2nd tab: `1` hosts, `2` sftp, `3` tunnels, `4` identities, `5` audit. Existing custom tab keybinds are migrated on config load.
- **Latency panel** now reflects the selected host, not an all-hosts aggregate.
- **ssh log lines wrap** instead of truncating with an ellipsis.

### Fixed

- SFTP picker: connect to the host you filtered to (a stale index could connect to the wrong host once the search cleared); queue re-entry guard; navigation frozen during a running queue; stale remote listings dropped; search input captured before tab-switch so typed letters don't fire tab binds.
- Keybinds migration persists so it runs exactly once.

## [0.5.7] - 2026-07-09

### Fixed

- Text selection keeps its full range when an autoscrolling drag carries it off-screen

## [0.5.6] - 2026-07-08

### Added

- **Autoscroll while selecting** — extending a selection past the viewport edge scrolls the session

### Fixed

- Long ssh log lines are clamped with an ellipsis so they stay inside the box
- Clipboard pastes are forwarded into sessions as bracketed paste
- The connect command stays visible in the ssh log after connecting
- `j`/`k` can be typed in search and the palette; dropped bare-key type-ahead

## [0.5.0] - 2026-07-08

### Added

- **Mouse text selection** — select text in an embedded session with the mouse and copy on release

## [0.4.0] - 2026-07-08

### Changed

- **Relicensed from MIT to AGPL-3.0-or-later** starting this release — a strong copyleft so forks and derivatives must stay open under the same terms. Versions ≤ 0.3.1 remain available under MIT.

## [0.3.1] - 2026-07-08

### Changed

- README images use absolute URLs so they render on the crates.io page; added a crates.io version badge and documented `cargo install sshub` plus the prebuilt release binaries. (Docs only — no code changes.)

## [0.3.0] - 2026-07-08

### Changed

- **ratatui 0.30** — upgraded the TUI stack; dropped the vendored vt100 fork in favour of upstream vt100 0.16 (which now carries the scrollback fix)
- **Scrollable help** — the `?` overlay scrolls (↑↓/j k/PgUp/PgDn/Home/End) with a pinned footer instead of truncating
- **Visible popup frames** — modal overlays (help, keybindings, palette, tag filter, group/field pickers, import prompt) now draw a distinct border so they read as dialogs
- **ssh_config Include** — importing hosts now follows `Include` directives (with tilde/relative/glob resolution)
- **Live hot reload** — the config watcher watches the containing directory, so editor rename-saves (vim `:w`, VSCode) keep triggering reloads
- **Config saves preserve** hand-written comments and any keys SSHub doesn't model
- **Installable crate** — publishable to crates.io (`cargo install sshub`); the demo fixture seeder moved from a binary to an example so it isn't installed

### Fixed

Security:

- Custom launcher now rejects `<`/`>` redirection in host fields (shell injection)
- ssh_config export flattens newlines in fields (config-directive injection)
- Private-key passphrase is delivered to `ssh-keygen` via `SSH_ASKPASS`, not as an argv argument visible in `ps`

Stability & correctness:

- Popups no longer panic on small terminals (a `clamp(min > max)` crash)
- Dashboard, host-form, and confirm text no longer overflow their borders on narrow terminals or when zoomed
- Editing a running tunnel now stops the old `ssh -N` child (was orphaned, holding its port); tunnel stderr is drained to avoid a pipe-buffer stall
- Mouse-wheel scrolling reaches hosts past group headers
- Confirm/keybind matching: `Shift+Y`/`Shift+N` bindings work, and editor-captured single-letter bindings fire correctly
- New hosts get a distinct `sort_order`, so Manual-mode reordering works
- A corrupt or locked legacy `metadata.db` no longer bricks startup
- The Audit tab's filter/range survives the periodic 10s refresh; the ping worker is no longer orphaned when the host list reloads to empty
- Confirm-delete wraps long host names; keybind labels no longer overflow the value column

## [0.2.0] - 2026-07-07

### Added

- **Embedded SSH sessions** — connect runs in an in-TUI PTY; detach with Ctrl+D and return to the dashboard while SSH keeps running
- **Session tab strip** — open sessions shown in the header; Ctrl+T opens host picker, Ctrl+[/] switches tabs, Ctrl+Shift+S focuses a session from the dashboard
- **Configurable keybindings** — 61 actions in `config.toml`; command palette and keybind editor
- **Nested host groups** — groups can contain sub-groups; redesigned group management overlays
- **Connect spinner** — visual feedback while SSH connects; `ssh -v` debug routed off the PTY
- **Failure screen** — plain-language disconnect reason with dismiss
- **Multi-tag filter** — AND filter via `#` tag picker
- **Group jump** — quick navigation between groups
- **Reachable ping stats** — latency sparkline in host detail
- **Tag-filter picker** — visible UI for tag selection; empty groups hidden while filtering

### Changed

- Import/export tests use explicit export paths to avoid parallel env races
- Process cleanup on quit — detached SSH children killed via `App::shutdown_all()` and `Drop`

### Fixed

- Connect spinner stays visible for unreachable hosts
- Enter confirms multi-tag selection
- Duplicate `KeyCode::Tab` arm in key event mapping
- Group tree order in manager; edit group on `E`; themed dropdown pickers

## [0.1.0] - 2026-06-01

Initial release — TUI launcher for SSH hosts with hybrid `~/.ssh/config` + launcher DB, tunnels, keys, audit log, fuzzy search, and file watcher hot reload.
