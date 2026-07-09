# OS auto-detection + distro logo rendering

## Problem

`os_icon` is stored, editable and import-populated, but **rendered nowhere** in
the TUI, and nobody sets it by hand. Result: dead data. We want Termius-style
behaviour: on first connect, detect the remote OS automatically and show a
distro logo for the host.

## Decisions (locked with user)

- **Detection:** a dedicated **background ssh probe** (option A), not injecting
  commands into the live interactive PTY. Reuses `ssh_argv_for_entry` + the same
  `SSH_ASKPASS` secret path the session uses. Never pollutes the user's shell.
- **Logos:** **vendored fastfetch small logos** — plain ASCII glyphs + ANSI
  colors. No Nerd Font, no emoji, works in every terminal, and shows the actual
  distro in color. fastfetch is **only** used offline on the dev machine to
  extract the data; it is **not** a runtime dependency and is **never** installed
  on target hosts.
- **Rendering:** a **self-contained `OsLogo` widget**, placed in the hosts
  **detail panel** in v1. Designed decoupled so it can later be dropped into a
  header/top panel when there are many hosts (explicit headroom requirement).
  The host **list stays clean** — no per-row glyph — which sidesteps the
  emoji/Nerd-Font dilemma entirely.
- **Config:** `appearance.os_logo: bool`, default **`true`**. Detection runs
  regardless of this flag (data accumulates even when rendering is off).
- **Storage:** `os_icon: Option<String>` keeps its column, but now holds a
  **canonical OS id** (`ubuntu`, `debian`, `arch`, `macos`, …). Detection only
  writes when the field is **empty** — a hand-set value is never overwritten.

## Data artifact (already produced)

`assets/os_logos.json` — extracted in the main loop from fastfetch 2.65.2 on the
dev machine. 21 canonical ids: arch, ubuntu, debian, alpine, fedora, rocky, rhel,
centos, opensuse, linuxmint, manjaro, popos, kali, gentoo, void, nixos,
endeavouros, freebsd, macos, windows, linux (generic fallback).

Shape:
```json
{ "<canonical_id>": {
    "logo": "<fastfetch name>",
    "lines": [ [ {"fg": <0-7|null>, "bright": bool, "bold": bool, "text": "…"}, … ], … ]
} }
```
`fg` is the ANSI basic color index (0–7); `null` = default fg. Small logos are
≤17 lines × ≤25 cols. Trailing all-empty lines must be trimmed by the loader.

## Architecture — new module `src/osinfo/`

- **`parse.rs`** — pure `parse_os(output: &str) -> Option<CanonicalOs>`. Pulls
  `ID=` from `/etc/os-release` (strip quotes; map `ID_LIKE` / known aliases:
  `rhel|centos|rocky|almalinux`→their ids, `linuxmint`, `pop`→`popos`,
  `opensuse-*`→`opensuse`). Fallback on `uname -s`: `Darwin`→`macos`,
  `FreeBSD`→`freebsd`, `Linux`→`linux`. Unknown → `None`. Fully unit-testable.
- **`logos.rs`** — loads `assets/os_logos.json` via `include_str!` +
  `serde_json` into a `HashMap<&str, OsLogo>` (build once, `OnceLock`). Resolves
  `fg`+`bright` to a `ratatui::style::Color` and `bold` to `Modifier::BOLD`.
  `logo_for(id) -> Option<&OsLogo>`. Unknown/unmapped id → `None` (no render).
  Canonical-ids-with-no-small-logo (e.g. `almalinux`) resolve to `None` → the
  panel simply renders without a logo.
- **`detect.rs`** — background worker
  `spawn_os_detect_worker() -> (Sender<OsDetectCmd>, Receiver<OsDetectEvent>)`,
  mirroring `src/ping.rs` / the probe worker: one thread, blocking `recv()`,
  self-terminates when the command `Sender` drops (`tx.send(..).is_err()`).
  Runs the probe **through a `ProbeRunner` trait** so tests inject canned output
  instead of shelling to ssh (mirrors `MockLauncher`/`FixtureResolver`).
    - `OsDetectCmd { host_id: i64, argv: Vec<String>, secret: Option<PendingSecret> }`
    - `OsDetectEvent::Detected { host_id: i64, os: String }`
    - Probe command: `ssh <argv-with-BatchMode/timeouts> 'cat /etc/os-release 2>/dev/null || uname -s'`,
      output piped to `parse_os`. On parse `None` or ssh failure → **no event**
      (silent; we retry on a later connect since the field stays empty).
- **`widget.rs`** — `OsLogo` widget: takes a resolved logo + `Rect`, renders
  colored `Line`s, clamped to the area (never overflows width/height). Pure
  render; no app state. Reusable in detail panel now, header later.
- **`mod.rs`** — re-exports; `CanonicalOs`/`OsLogo` types.

## App wiring

- `App` gains (next to `ping_rx`/`probe_rx`): `os_detect_tx: Option<Sender<..>>`,
  `os_detect_rx: Option<Receiver<..>>`. Spawned in `App::new_with_deps`.
- `connect_host_entry`: after the connect succeeds and only if the entry's
  `os_icon` is empty **and** the entry is a managed host with a real `host_id`,
  send one `OsDetectCmd` (build argv via `ssh_argv_for_entry`, secret via
  `resolve_pending_secret`). Guard so a host is probed **once** per empty state
  (a `HashSet<i64> os_detect_inflight` on `App` prevents duplicate sends while a
  probe is running).
- `poll_keys_and_watcher`: drain `os_detect_rx` alongside ping/probe. On
  `Detected { host_id, os }`: `store.update_host(host_id, HostUpdate { os_icon:
  Some(Some(os)), ..default })`, remove from `os_detect_inflight`, then
  `reload_hosts()`.

## Rendering

- `render_detail_panel` (hosts tab): if `appearance.os_logo` and the selected
  host's `os_icon` resolves to a logo, carve a left sub-column (logo width + 1)
  and render `OsLogo`; existing detail fields shift right. If no logo / flag off
  → detail panel exactly as today.

## Config

- `AppConfig.appearance.os_logo: bool` default `true`. TOML round-trips through
  the existing `toml_edit` path; add to `[appearance]` docs + sample.

## Keybindings / form

- `OS_ICON_OPTIONS` extended to the full canonical list so the host form picker
  can still set/override manually. `os_icon_from_index` / `os_icon_index_from_option`
  updated. `(none)` stays index 0.

## Testing (offline, no network)

- `parse.rs`: table of os-release / uname inputs → expected canonical id,
  including quoted `ID="ubuntu"`, `ID=arch`, `ID_LIKE` fallbacks, `Darwin`,
  unknown → `None`.
- `logos.rs`: every extracted id resolves to a non-empty logo; each logo's lines
  are within the declared max width; unknown id → `None`; `almalinux` → `None`.
- `widget.rs`: `TestBackend` snapshot — logo renders within a small `Rect`
  without panicking or overflowing.
- `detect.rs`: worker driven with a `MockProbeRunner` returning canned
  os-release text → emits `Detected` with the right id; failure text → no event.
- `just test` green; `cargo clippy --all-targets` no new warnings; `cargo fmt`.

## Files

**New:** `assets/os_logos.json` (done), `src/osinfo/mod.rs`,
`src/osinfo/parse.rs`, `src/osinfo/logos.rs`, `src/osinfo/detect.rs`,
`src/osinfo/widget.rs`.

**Modified:** `Cargo.toml` (`serde_json` if not already a dep),
`src/lib.rs` (module decl + drain), `src/app/mod.rs` (fields + `OS_ICON_OPTIONS`),
`src/app/connect.rs` (trigger), `src/app/util.rs` (icon index helpers),
`src/config.rs` (`os_logo` flag), `src/tui/widgets/detail_panel.rs`
(logo sub-column), `README.md`/`CHANGELOG` + help screen note.

## Scope boundaries

- **v1:** background detect for managed hosts, canonical-id storage, 21 vendored
  logos, detail-panel render, config toggle, manual override preserved.
- **Deferred:** per-row list glyph, header/multi-host logo panel (widget is
  ready for it), logos for legacy/ssh_config-only hosts without a `host_id`,
  re-detect command, more distros.

## Risks

- **Second auth on password hosts:** the probe re-auths; handled by reusing the
  same `SSH_ASKPASS` secret. If auth is interactive-only and no secret is
  available, the probe fails silently — acceptable (no logo, retried later).
- **ssh latency:** probe runs off-thread with `ConnectTimeout`; never blocks the
  50 ms UI loop.
