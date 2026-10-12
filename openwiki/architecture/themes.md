---
type: Architecture
title: Theme System Internals, registry, resolver, ThemeManager, and picker
description: How src/theme turns five embedded built-ins and user TOML files into one immutable ResolvedTheme owned by App (25 semantic slots, 234 component roles, static gradients), how the runtime falls back to default, how the theme picker previews and persists, and where the sshub theme CLI fits. The authoring guide is docs/theme-system.md.
resource: src/theme/mod.rs
tags: [theme, tui, rendering, config, architecture]
---

# Theme System Internals

Colours are data. Every renderer reads its style from the active
`ResolvedTheme`; there are no colour constants left in the render path
(`src/tui/theme.rs` only maps values to roles; its `legacy` palette is
`#[cfg(test)]` parity data). The theme **file format**, the role catalogue,
the built-ins and worked examples are documented in
[docs/theme-system.md](../../docs/theme-system.md); the gradient cost is in
[docs/theme-render-benchmark.md](../../docs/theme-render-benchmark.md). This page
covers the code. User-facing picker keys: [profiles & themes
workflow](../workflows/profiles-themes.md).

## Pipeline (`src/theme/`)

| Module | Job |
|---|---|
| `builtins.rs` | The five built-ins (`default`, `summer`, `aqua`, `fire`, `high-contrast`), embedded from `assets/themes/*.toml` with `include_str!` so they go through the same parser as user files |
| `parse.rs` | Span-preserving TOML parser: reports *shape* and keeps byte ranges for `file:line:column` diagnostics |
| `validate.rs` | Schema rules and limits (`MAX_PALETTE_ENTRIES = 256`, `MAX_GRADIENTS = 128`, `MAX_GRADIENT_STOPS = 32`; sentinels `auto`, `native`, `terminal`) |
| `catalog.rs` | The frozen V1 role catalogue, generated from one `role_catalog!` declaration: 25 semantic slots (`SEMANTIC_SPECS`) and 234 component roles (`ROLE_SPECS`), 233 of which fall back to a semantic slot (asserted in tests) |
| `resolve.rs` | `extends` chain (implicit `default` parent, `MAX_INHERITANCE_DEPTH = 16`), deep-merge root first with `"auto"` resets, then reference resolution (`MAX_COLOR_REFERENCE_DEPTH = 16`) into one `ResolvedTheme` |
| `registry.rs` | `ThemeRegistry`: registers built-ins, reads `themes/*.toml` lexicographically (at most `MAX_USER_THEME_FILES = 256`), validates each file in isolation, resolves all together. A broken file is listed with its reason and never takes the others down; a user file named like a built-in is always invalid |
| `manager.rs` | `ThemeManager`: the only place that turns `appearance.active_theme` into the active theme. A missing or invalid id falls back to the embedded `default`, keeps the configured id, shows a hint, and **never rewrites `config.toml`** |
| `gradient.rs` | Static gradients (`horizontal`, `vertical`, `diagonal_down`, `diagonal_up`, `perimeter`) painted as buffer post-processing over cells that already have their solid fallback colour |
| `model.rs` | Definition side (spans) vs resolved side; `ValidationMode::Strict` (CLI `theme check`) and `Compatible` (runtime: unknown roles from a newer SSHub load with a warning) |

## Ownership in the app

- `App` owns the `ThemeManager` in a **private** field; renderers take
  `app.theme()` (`&ResolvedTheme`; with `transparent_sshub_background` on it
  returns the manager's ground-released variant). There is no global mutable
  theme state, which keeps tests deterministic.
- `App::new_with_profile` loads themes before hosts
  (`App::load_themes_from`), and the load is infallible by construction:
  `default` is embedded, so a broken themes directory cannot stop startup.
- **Themes directory, as the code resolves it today**: the TUI reads
  `<directory of config.toml>/themes`, i.e. `profiles/<name>/themes/` in
  profile mode and `<config dir>/themes/` in compat mode
  (`src/app/mod.rs::new_with_profile`). The headless `sshub theme list|show`
  resolve `config::config_dir_path()/themes`, i.e. `~/.config/sshub/themes/`
  (or `$SSHUB_CONFIG_DIR/themes/`). The two agree only in compat mode;
  `docs/theme-system.md` and the 0.14.0 changelog name `~/.config/sshub/themes`.
- The startup profile picker always renders on the embedded `default`
  (`ThemeRegistry::builtins`), because no profile is loaded yet.

## Theme picker (`src/app/theme_picker.rs`, `src/tui/screens/theme_picker.rs`)

Opened from the Settings Theme row (`AppMode::ThemePicker`). It tracks three
ids: `saved_id` (what `config.toml` holds), `original_id` (active when the
picker opened) and `preview_id` (painting the whole UI right now). Moving the
cursor previews on the entire interface; `Esc` restores `original_id`; only
`Enter` persists, through `App::config_target()`, the same profile-aware writer
as every other setting (0.14.2 fixed a write to the global config that startup
never read). Every change of the painted theme must go through
`ThemeManager::activate_resolved` so buffer snapshots captured under the old
theme are invalidated.

## Rendering rules worth preserving

- The embedded PTY grid is painted from the two semantic slots
  `pty_background` / `pty_foreground`, always as a pair. Colours the remote
  chose itself are never touched, and gradients never paint the session grid.
- `appearance.transparent_sshub_background` and
  `appearance.transparent_session_background` release SSHub's own ground and
  the session grid ground respectively; selection bars, borders and status
  colours keep drawing.
- Golden output: `tests/fixtures/theme/default-dashboard.buffer` pins the
  `default` dashboard cell for cell (`src/tui/mod.rs` tests), and
  `src/theme/role_matrix.snapshot` pins the role catalogue.

## CLI (`src/cli/theme.rs`)

`sshub theme check FILE` (strict, all problems in one run, `did you mean`
suggestions), `sshub theme list`, `sshub theme show ID [--resolved]
[--format toml|json]`. Dispatched in `main.rs` before any database is opened,
and never writes user state.
