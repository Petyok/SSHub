# TUI rendering overview

SSHub uses `ratatui` 0.30 with a synchronous event loop. The UI is split into dashboard chrome, tab bodies, and modal overlays.

## Event loop

`src/lib.rs`:

- `run_app()` loads config and creates `App`.
- If stdout is a terminal, it enters raw mode and alternate screen.
- `POLL_INTERVAL` is 50 ms. Each tick drains crossterm events, background channels, and redraws.

`src/app/mod.rs` holds `App` state and dispatch. `app.tick()` is called each loop iteration.

## Frame layout

`src/tui/dashboard_layout.rs` divides the frame into:

- header (stats + clock),
- session strip (active embedded-session tabs),
- tab bar,
- body,
- footer.

Zoom levels (`app.ui_zoom`, `[`/`]`) widen the hosts/name columns.

## Tab dispatch

`src/tui/mod.rs::render_inner()`:

```text
0 → hosts
1 → sftp
2 → tunnels
3 → identities
4 → audit
```

The current tab is also used in `src/app/keys.rs` to route normal-mode keys.

## Overlay modes

`AppMode` in `src/app/types.rs` is the single source of truth for what is on screen. Examples:

- `Palette` — fuzzy quick-connect search.
- `HostForm`, `IdentityForm`, `GroupForm`, `TunnelForm` — input forms.
- `FieldPicker`, `GroupFieldPicker`, `TunnelHostPicker`, `SessionHostPicker` — dropdown pickers.
- `KeybindEditor`, `Settings`, `TunnelReconnectSettings` — configuration overlays.
- `Connecting`, `Session` — fullscreen embedded PTY (rendered by `src/session/render.rs`).

Overlays render on top of the dashboard body but underneath or over the chrome depending on mode.

## Widgets

`src/tui/widgets/`:

- `footer.rs` — tab-specific key hints and horizontal rules.
- `header.rs` — stats, clock, tab strip.
- `tab_bar.rs` — tab labels with current indicator.
- `status_bar.rs` — used by tab bodies for inline notices.
- `middle_stack.rs` — tunnel status / reconnect views.
- `panel_box.rs` — reusable bordered panel.
- `text.rs` — helpers.

`src/tui/screens/` contains the main tab renderers plus help and form screens:

- `hosts.rs`, `tunnels.rs`, `keys.rs`, `audit.rs`, `sftp.rs`.
- `host_form.rs`, `group_form.rs`, `help.rs`, `palette.rs`, `tag_filter.rs`.

## Theme

`src/tui/theme.rs` defines colors used across the app. It is not user-configurable today.

## Startup animation

`src/tui/animation.rs` draws the splash screen. It can be disabled via settings.

## Rendering tips

- Use `src/tui/mod.rs::fit_popup()` to clamp popup dimensions safely on small terminals.
- When adding a new overlay, add an `AppMode` variant and a `match` arm in both `render_inner` and the key handler.
- The `opaque_background` setting paints a solid background cell-by-cell after the normal render pass.

## What to watch when changing the TUI

- `src/app/types.rs::AppMode` — used by render, key, and mouse dispatch.
- `src/tui/mod.rs` — the main render router.
- `src/app/keys.rs` — must handle the same mode set.
- `src/tui/widgets/footer.rs` — new tab actions need a footer hint.
- `src/tui/screens/help.rs` — new actions need help text.
- Tests: E2E tests assert frame content, so any label/text change may need matching test updates.
