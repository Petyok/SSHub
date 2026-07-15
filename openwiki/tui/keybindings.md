# Keybindings

Almost every action in SSHub can be rebound by the user. Defaults live in code; overrides are saved to `~/.config/sshub/config.toml`.

## Defining actions

`src/keybinds.rs` defines the `KeyAction` enum and default key specs. The spec format is parsed in `src/app/util.rs::parse_keyspec`.

Spec examples:

- `q` — simple letter.
- `Enter`, `Esc`, `Tab`, `Backspace` — named special keys.
- `Ctrl+D`, `Shift+G`, `Alt+Enter` — modifier + key.
- `F1`..`F12` — function keys.

## User overrides

The `[keybindings]` table in `config.toml` maps action names to arrays of specs. For example:

```toml
[keybindings]
quit = ["q", "Ctrl+C"]
connect = ["Enter"]
search = ["/"]
```

`src/config.rs` loads these into `AppConfig::keybindings`. Missing actions fall back to the compiled-in default list, so users can override selectively without losing other bindings.

## Loading and saving

- On startup, `config::load_config()` reads `config.toml` and resolves each action to its `KeyAction` enum variant plus the list of `KeySpec`.
- The keybind editor (`Ctrl+K`) mutates `app.config.keybindings` and rewrites `config.toml` through `src/config.rs` save helpers, which use `toml_edit` to preserve comments.
- Some bindings may be migrated when defaults change (e.g. the v0.7.0 tab-order migration).

## Dispatch

`src/app/keys.rs` handles keys in `AppMode::Normal` (and some global overlays). It translates a matched key spec into an action method call on `App`.

`src/app/mouse.rs` maps mouse events to the same actions where appropriate (click row → select, double-click → connect, scroll → change selection).

Session-specific keys are parsed separately in `src/session/keys.rs` because they need to interpret combinations like `Ctrl+T` even while raw input is being forwarded to the PTY.

## Rendering hints

- The dashboard footer (`src/tui/widgets/footer.rs`) shows context-sensitive key hints per tab.
- The session header/footer (`src/session/render.rs`) show configured session keybinds from `app.config.keybindings`, not hardcoded defaults.
- **Hardcoded exceptions** (not `KeyAction`s — cannot be remapped in the keybind editor):
  - `Ctrl+H` — opens Settings from the dashboard (`src/app/keys.rs`).
  - `R` on the Tunnels tab — tunnel reconnect settings (`src/app/tunnels.rs`).
  - `R` on the SFTP tab — rename selected file/dir (`src/app/sftp.rs`; lowercase `r` refreshes panes).
- help overlay (`src/tui/screens/help.rs`) lists all actions and their current bindings.

## What to watch when changing bindings

- `src/keybinds.rs` — add the enum variant + default list, and update any string-based matching.
- `src/app/keys.rs` — wire the action in the relevant mode/tab handler.
- `src/app/mouse.rs` — if the action can be triggered by mouse, add it.
- `src/tui/widgets/footer.rs` / `src/tui/screens/help.rs` — add the hint/help entry.
- `src/config.rs` — if the action participates in saved overrides, ensure serialization round-trips.
- Tests: `src/app/tests/keybind.rs`, any e2e scenario that simulates key presses.
