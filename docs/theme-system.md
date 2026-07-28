# The SSHub theme system

SSHub's colours are data, not code. Every surface it paints — panels, popups,
forms, tabs, tunnels, SFTP, the audit log, the startup animation — reads its
style from a **theme**: a TOML file you can copy, edit and select at runtime.

Three things make that practical:

- **Runtime selection.** The theme picker previews a theme on the *whole* TUI
  while you move through the list, and `Esc` rolls back. Nothing is written
  until you press `Enter`.
- **Strict validation.** `sshub theme check` reads your file with the same
  parser and resolver the application uses, and reports every problem it finds
  in one run, with file, line and column.
- **Sensible inheritance.** The 234 component roles are driven by 23 semantic
  meanings — 233 of them fall back to a semantic slot, and the one exception,
  `components.os_logo.tint`, keeps the asset's own colours by default. Every
  user theme inherits from `default`, so changing `semantic.accent` alone
  recolours everything that uses the accent.

This guide is the long form. The README has the three-line version.

---

## Quick start

The shortest path from "I want different colours" to a theme of your own:

```bash
# 1. Make the themes directory. The ID of a theme is its file name.
mkdir -p ~/.config/sshub/themes

# 2. Copy a built-in as a starting point.
sshub theme show aqua > ~/.config/sshub/themes/mine.toml

# 3. Change the visible `name` inside the file — the ID came from the
#    file name, but two entries called "Aqua" in the picker help nobody.
$EDITOR ~/.config/sshub/themes/mine.toml

# 4. Check it before you look at it.
sshub theme check ~/.config/sshub/themes/mine.toml

# 5. Select it: Ctrl+H (Settings) → "Theme…" → Enter → pick → Enter.
sshub
```

`sshub theme show` prints the built-in's own source, comments included, so what
you get is a documented file rather than a colour dump. The first line reminds
you to change `name`.

Selecting a theme in the picker writes exactly one setting:

```toml
# ~/.config/sshub/config.toml
[appearance]
active_theme = "mine"
```

With `SSHUB_CONFIG_DIR` set, themes live in `$SSHUB_CONFIG_DIR/themes/`
instead.

---

## The five built-ins

The built-ins are embedded in the binary, so they work without any file at all,
and they go through the same parser, resolver and validator as your files.

| ID | What it is for |
| --- | --- |
| `default` | SSHub's original dark palette — deep greens, muted cyan, transparent surfaces. It is the root theme every other theme inherits from, and it is what you get with no configuration at all. |
| `summer` | A bright, warm daylight theme: cream surfaces, sun yellow and white, with soft yellow-white frame and separator gradients. The reference for light themes. |
| `aqua` | Deep-water blues with cyan and turquoise accents, and a ring-lit focus frame. The reference for `perimeter` gradients. |
| `fire` | Charcoal surfaces lit by red, orange and gold, with the status colours kept deliberately distinct from the decoration. |
| `high-contrast` | White on pure black with saturated status colours and **no** decorative gradients — the proof that every gradient is optional. |

`summer`, `aqua` and `fire` are commented in full and are meant to be copied.
All four non-root built-ins `extends = "default"` and set only their actual
deviations, so reading one shows you how little a theme has to say.

Built-in IDs are reserved. A file called `aqua.toml` in your themes directory is
listed in the picker — so you can see why it is not working — but it is always
invalid, and the built-in `aqua` keeps its place.

---

## The file format, one layer at a time

### 1. The smallest theme that works

```toml
schema_version = 1
name = "My Theme"
```

That is a complete, valid theme. It inherits everything from `default`, so it
looks exactly like `default` — which is the point: you only write down what you
want to be different.

`schema_version` must be exactly `1`. A file that claims a version SSHub does
not know is rejected with a readable message rather than guessed at.

### 2. Metadata

| Field | Required | Meaning |
| --- | --- | --- |
| `schema_version` | yes | Must be exactly `1` in V1 |
| `name` | yes | The display name in the picker |
| `extends` | no | Parent theme ID; defaults to `default` for user themes |
| `description` | no | One line, shown under the picker list |
| `author` | no | Free metadata; survives `theme show --resolved` |
| `palette` | no | Your own named colours |
| `semantic` | no | Overrides of the fixed semantic core |
| `gradients` | no | Named static gradients |
| `components` | no | Per-role overrides |

The technical **ID** of a user theme is its file name without `.toml`, and it
may only use ASCII lowercase letters, digits, `-` and `_`. The visible `name` is
free.

### 3. The palette — your own colours

```toml
[palette]
deep_sea = "#08202a"
lantern = "#ffb454"
```

Palette names are yours alone. Nothing in SSHub knows them; they exist so the
rest of the file can say `"palette.lantern"` instead of repeating a hex value.
An entry nobody references is reported as a warning, not an error.

### 4. The semantic core — the 23 meanings

This is the layer worth learning. 233 of SSHub's 234 component roles fall back
to one of exactly these 23 slots, so overriding a slot re-tints every component
that inherits from it. (The one that does not is `components.os_logo.tint`,
whose default is `"native"` — the detected distro logos keep their own brand
colours until you say otherwise.)

```text
background, canvas, surface, surface_raised,
border, border_focus, border_popup,
text, text_bright, text_highlight, text_muted, text_dim, text_inverse,
accent, selection_bg, selection_fg,
success, warning, error, info, connecting, exited, unknown
```

```toml
[semantic]
accent = "palette.lantern"
border_focus = "palette.lantern"
```

Two of them deserve a note:

- **`background`** decides whether SSHub paints its own app background at all.
  Resolve it to `"terminal"` and SSHub leaves your terminal's own background
  showing through (including any transparency it has). Resolve it to a colour
  and SSHub paints its own surfaces — never the remote PTY. Like every entry in
  `[semantic]`, it takes a colour only; a gradient there is a validation error
  (`` `semantic.background` does not support gradients ``). To sweep the app
  background with one, put the gradient on the paint role
  `components.app.background` instead — see [Static gradients](#static-gradients).
- **`canvas`** is the opaque companion value: the colour an otherwise
  transparent theme uses when something genuinely needs a solid ground, and the
  default mixing ground for simulated opacity.

Everything else is optional. What you do not set, you inherit.

### 5. Component overrides — the last 5%

```toml
[components.footer.key]
foreground = "#f8fff9"
background = "semantic.accent"
modifiers = ["bold"]
```

The table header is TOML shorthand: the block above sets the single role
`components.footer.key`. Each role has exactly one value type:

| Type | Accepts |
| --- | --- |
| `color` | a colour, a reference, `"terminal"`, or `"auto"` |
| `style` | `foreground`, `background`, `modifiers` — each optionally `"auto"`; the whole role resets with `{ auto = true }` |
| `paint` | a colour/reference, `"terminal"`, `"auto"`, or `{ gradient = "gradients.<name>" }` |
| `tint` | a colour/reference, `"native"` for untouched asset colours, or `"auto"` |

Supported modifiers: `bold`, `dim`, `italic`, `underlined`, `reversed`,
`crossed_out`.

A role that does not exist is an **error** for `sshub theme check` and a
non-fatal **warning** at runtime: SSHub ignores that one role, uses its semantic
fallback, and says so in the picker and in a start-up notice. That way a theme
written for a newer SSHub still works, and a typo still gets noticed.

---

## Colour values

Exactly one of these forms, anywhere a colour is accepted:

```toml
[palette]
# 1. a hex literal — exactly #RRGGBB
plain = "#08202a"

# 2. explicit channels, three integers in 0..255
channels = { rgb = [245, 180, 60] }

# 3. a qualified reference to another colour
alias = "palette.plain"
from_core = "semantic.accent"

# 4. a reference with brightness: -1.0..1.0, towards white or black
lifted = { color = "palette.plain", brightness = 0.12 }

# 5. a reference with simulated opacity over an explicit ground
ghost = { color = "semantic.accent", opacity = 0.35, over = "palette.plain" }
```

Rules worth knowing before your first `theme check` failure:

- A bare string is a hex literal **only** if it matches `^#[0-9a-fA-F]{6}$`.
  Every reference is qualified: `"palette.<name>"` or `"semantic.<name>"`.
  An unqualified string is never guessed at as an ANSI colour name.
- `rgb` and `color` are mutually exclusive; any other field is an error.
- `brightness` runs in `-1.0..1.0`; positive mixes towards white, negative
  towards black. It is applied **before** opacity.
- `opacity` runs in `0.0..1.0`. `over` is only allowed together with it, and
  defaults to `semantic.background`.

### Why opacity is simulated

Terminals have no alpha channel for text cells. SSHub therefore computes the
blend itself, per channel, in sRGB:

```text
result = color * opacity + over * (1 - opacity)
```

This is deterministic and cell-exact, but it is *not* terminal transparency: it
mixes against a colour SSHub knows, not against whatever is behind your window.

That is also why opacity **errors** when the mixing ground resolves to
`"terminal"`: `"terminal"` is `Color::Reset`, and SSHub has no RGB value for it
to mix with — only your terminal emulator knows what it looks like. If your
theme's `semantic.background` is `"terminal"`, give every opacity an explicit
opaque `over`:

```toml
# fails: semantic.background is "terminal" in `default`
soft = { color = "semantic.accent", opacity = 0.4 }

# works: an explicit opaque ground
soft = { color = "semantic.accent", opacity = 0.4, over = "semantic.canvas" }
```

The diagnostic even names the theme in the chain the `"terminal"` came from,
e.g. `semantic.surface resolves via default to terminal; opacity requires
opaque RGB`.

### The three sentinels

| Value | Where it is valid | What it means |
| --- | --- | --- |
| `"terminal"` | any colour position | Your terminal's own default (`Color::Reset`). Not a palette reference. |
| `"auto"` | only under `[components]` | Drop an inherited override and go back to the role's built-in semantic fallback. |
| `"native"` | only `tint` roles | Keep the embedded asset's own colours — the distro logos stay in their brand colours. |

`brightness`, `opacity` and `over` are all invalid on `"terminal"`. Typos in the
sentinels are recognised: `"atuo"` and `"termnial"` get a `did you mean` hint
rather than a generic complaint about an unqualified string.

---

## Inheritance

Every user theme has a parent. Unless you say otherwise it is `default`:

```toml
extends = "aqua"
```

`extends` is always a theme **ID**, never a path, so inheritance can never leave
the themes directory. Only the built-in root `default` has no parent. The
maximum chain depth is 16.

### Merge first, resolve second

This is the rule that makes inheritance useful rather than surprising: the whole
chain is deep-merged into one definition, and *only then* are colour references
resolved.

**Parent** (`base.toml`):

```toml
schema_version = 1
name = "Base"

[semantic]
accent = "#9ec99b"

[components.footer.key]
foreground = "semantic.accent"
modifiers = ["bold"]
```

**Child** (`child.toml`):

```toml
schema_version = 1
name = "Child"
extends = "base"

[semantic]
accent = "#ffb454"
```

The child never mentions `components.footer.key`, but the footer key comes out
**orange**: the inherited reference to `semantic.accent` is resolved after the
merge, against the child's value. That is why changing one semantic slot is
usually all a theme needs to do.

What wins, field by field:

- Metadata: the child's value replaces the parent's.
- `palette`, `semantic`, `gradients`: merged by entry name; a child entry
  replaces the parent's entry of that name completely.
- `components`: merged by full role path, and for `style` roles by individual
  field, so a child can change only the `foreground` and keep the inherited
  `modifiers`.
- A child may reference names that only the parent defines.

### Resetting with `"auto"`

`"auto"` is the undo button for inheritance:

```toml
[components.dashboard.host_list]
# drop an inherited border override; go back to the semantic fallback
border = "auto"

[components.footer.key]
background = "auto"      # one style field
modifiers = []           # no modifiers at all
```

- `"auto"` on a `color`, `paint` or `tint` role removes the inherited override.
- `"auto"` on a single `style` field removes just that field.
- `{ auto = true }` resets an entire `style` role.
- `modifiers = []` means *no modifiers*; `modifiers = "auto"` means *back to the
  role's semantic fallback*. They are different, and the difference is visible.

---

## Static gradients

V1 gradients are **static**. They are sampled from the geometry of the surface
being painted; there is no animation, no timeline, and no parameter that could
later be mistaken for one. (Animated gradients are a possible future feature,
deliberately kept out of the V1 schema.)

Gradients are always named under `[gradients]` and then referenced. Inline
multi-stop definitions inside a component are not allowed — it keeps large files
readable and lets one gradient be reused.

```toml
[gradients.panel_border]
direction = "horizontal"
stops = [
  { at = 0.0,  color = "semantic.accent" },
  { at = 0.55, color = "#a0ffe0" },
  { at = 1.0,  color = { rgb = [8, 40, 50], brightness = 0.05 } },
]

[components.dashboard.host_list]
border = { gradient = "gradients.panel_border" }

# The whole app background is a paint role too, which is where a gradient goes
# when you want the surface itself to sweep. `[semantic]` cannot hold one.
[components.app]
background = { gradient = "gradients.panel_border" }
```

### The five directions

| Direction | Sampling |
| --- | --- |
| `horizontal` | left to right across the surface |
| `vertical` | top to bottom |
| `diagonal_down` | top-left towards bottom-right |
| `diagonal_up` | bottom-left towards top-right |
| `perimeter` | once around a closed frame and back to the start |

### Stop rules

- At least two stops, at most 32.
- `at` runs in `0.0..1.0`, strictly ascending.
- The first stop is at `0.0`, the last at `1.0`.
- Stops use the full colour syntax, references included.
- A component may only reference a gradient if that role accepts one — the
  `paint` roles in the catalogue below.

### The perimeter rule

`perimeter` runs a single ramp all the way around a frame and back to where it
started, so it is only valid on a role that actually draws a **closed frame**
(the `Closed frame: yes` column in the catalogue). And because the ring closes,
the first and last stop must resolve to the **same** RGB colour — otherwise
there would be a visible seam where the ring meets itself. Both are validation
errors, not surprises at render time.

A visible multi-stop ring, four stops, closing on itself:

```toml
[gradients.lantern_ring]
direction = "perimeter"
stops = [
  { at = 0.0,  color = "palette.dusk" },
  { at = 0.35, color = "palette.lantern" },
  { at = 0.7,  color = "semantic.info" },
  { at = 1.0,  color = "palette.dusk" },
]

[components.dashboard.host_list]
border_focused = { gradient = "gradients.lantern_ring" }
```

Gradient sampling allocates nothing per cell, and the measured cost of a full
gradient theme against a solid one is recorded in
[docs/theme-render-benchmark.md](theme-render-benchmark.md).

---

## The role catalogue

234 roles, grouped by the surface they paint. Find the surface first, then the
role:

| You want to change… | Look under |
| --- | --- |
| the host list, the host card, the ping/agent panels | `components.dashboard` |
| the top bar, clock, counters | `components.header` |
| the tab strip and the session tabs | `components.tabs`, `components.session` |
| the bottom bar and its keys | `components.footer`, `components.status_bar` |
| any overlay frame, title, hint or legend | `components.popup` |
| any list overlay (theme, host, session, field pickers) | `components.picker` |
| the host / identity / group / tunnel forms | `components.form`, `components.group_form`, `components.tunnel_form` |
| the tunnels tab and its rows | `components.tunnels`, `components.tunnel` |
| the SFTP dual-pane browser and its queue | `components.sftp` |
| the audit log | `components.audit` |
| the broadcast overlays | `components.broadcast` |
| the identity cards on the keys tab | `components.identities` |
| the keybinding editor and the help overlay | `components.keybind`, `components.help` |
| the startup animation | `components.animation` |
| the detected-OS logos | `components.os_logo` |
| generic text, status words, selection, separators | `components.text`, `components.status`, `components.selection`, `components.separator` |
| the whole app background | `components.app` |

Read the table like this: **Falls back to** is what the role uses when no theme
in the chain sets it, and **Closed frame** says whether `perimeter` is allowed.
The table is generated from the same Rust definition the validator and the
renderers use, so it cannot drift.

<!-- THEME_ROLES:START -->
#### `components.animation`

| Role | Type | Falls back to | Closed frame |
| --- | --- | --- | --- |
| `components.animation.background` | paint | `semantic.background` | no |
| `components.animation.cursor` | style | `semantic.success` | no |
| `components.animation.halo` | paint | `semantic.selection_bg` | no |
| `components.animation.hub_early` | style | `semantic.success` | no |
| `components.animation.hub_flash` | style | `semantic.warning` | no |
| `components.animation.hub_label` | style | `semantic.text_muted` | no |
| `components.animation.hub_ready` | style | `semantic.text_bright` + bold | no |
| `components.animation.node` | style | `semantic.success` | no |
| `components.animation.node_label` | style | `semantic.text` | no |
| `components.animation.prompt_key` | style | `semantic.text_bright` | no |
| `components.animation.prompt_text` | style | `semantic.text_muted` | no |
| `components.animation.quip` | style | `semantic.text_dim` | no |
| `components.animation.spoke` | style | `semantic.text_dim` | no |
| `components.animation.tagline` | style | `semantic.text_muted` | no |
| `components.animation.tagline_accent` | style | `semantic.warning` | no |
| `components.animation.wordmark` | style | `semantic.text_bright` + bold | no |
| `components.animation.wordmark_accent` | style | `semantic.warning` | no |

#### `components.app`

| Role | Type | Falls back to | Closed frame |
| --- | --- | --- | --- |
| `components.app.background` | paint | `semantic.background` | no |

#### `components.audit`

| Role | Type | Falls back to | Closed frame |
| --- | --- | --- | --- |
| `components.audit.error` | color | `semantic.error` | no |
| `components.audit.filter_active` | style | `semantic.text_inverse` on `semantic.text_bright` | no |
| `components.audit.filter_inactive` | style | `semantic.text_dim` | no |
| `components.audit.note` | style | `semantic.text_muted` | no |
| `components.audit.row` | style | `semantic.text` | no |
| `components.audit.row_selected` | style | `semantic.selection_fg` on `semantic.selection_bg` | no |
| `components.audit.success` | color | `semantic.success` | no |
| `components.audit.table_header` | style | `semantic.text_bright` + bold | no |
| `components.audit.unknown` | color | `semantic.unknown` | no |
| `components.audit.warning` | color | `semantic.warning` | no |

#### `components.broadcast`

| Role | Type | Falls back to | Closed frame |
| --- | --- | --- | --- |
| `components.broadcast.countdown` | style | `semantic.info` | no |
| `components.broadcast.detail` | style | `semantic.text_dim` | no |
| `components.broadcast.error` | color | `semantic.error` | no |
| `components.broadcast.panel.background` | paint | `semantic.surface` | no |
| `components.broadcast.panel.border` | paint | `semantic.border` | yes |
| `components.broadcast.panel.border_focused` | paint | `semantic.border_focus` | yes |
| `components.broadcast.panel.count` | style | `semantic.text_dim` | no |
| `components.broadcast.panel.title` | style | `semantic.text_bright` | no |
| `components.broadcast.pending` | color | `semantic.text_muted` | no |
| `components.broadcast.running` | color | `semantic.warning` | no |
| `components.broadcast.stderr` | style | `semantic.error` | no |
| `components.broadcast.stdout` | style | `semantic.text_muted` | no |
| `components.broadcast.success` | color | `semantic.success` | no |

#### `components.command_palette`

| Role | Type | Falls back to | Closed frame |
| --- | --- | --- | --- |
| `components.command_palette.query` | style | `semantic.text_highlight` | no |
| `components.command_palette.row_selected` | style | `semantic.text_highlight` on `semantic.selection_bg` | no |

#### `components.dashboard`

| Role | Type | Falls back to | Closed frame |
| --- | --- | --- | --- |
| `components.dashboard.agent.background` | paint | `semantic.surface` | no |
| `components.dashboard.agent.border` | paint | `semantic.border` | yes |
| `components.dashboard.agent.border_focused` | paint | `semantic.border_focus` | yes |
| `components.dashboard.agent.title` | style | `semantic.text_bright` | no |
| `components.dashboard.auth.background` | paint | `semantic.surface` | no |
| `components.dashboard.auth.border` | paint | `semantic.border` | yes |
| `components.dashboard.auth.border_focused` | paint | `semantic.border_focus` | yes |
| `components.dashboard.auth.title` | style | `semantic.text_bright` | no |
| `components.dashboard.details.background` | paint | `semantic.surface` | no |
| `components.dashboard.details.border` | paint | `semantic.border` | yes |
| `components.dashboard.details.border_focused` | paint | `semantic.border_focus` | yes |
| `components.dashboard.details.field_marker` | style | `semantic.accent` | no |
| `components.dashboard.details.label` | style | `semantic.info` | no |
| `components.dashboard.details.metadata` | style | `semantic.text_muted` | no |
| `components.dashboard.details.title` | style | `semantic.text_bright` | no |
| `components.dashboard.details.value` | style | `semantic.text` | no |
| `components.dashboard.host_list.background` | paint | `semantic.surface` | no |
| `components.dashboard.host_list.border` | paint | `semantic.border` | yes |
| `components.dashboard.host_list.border_focused` | paint | `semantic.border_focus` | yes |
| `components.dashboard.host_list.count` | style | `semantic.text_dim` | no |
| `components.dashboard.host_list.group` | style | `semantic.info` | no |
| `components.dashboard.host_list.host` | style | `semantic.text` | no |
| `components.dashboard.host_list.host_selected` | style | `semantic.text_highlight` on `semantic.selection_bg` | no |
| `components.dashboard.host_list.match` | style | `semantic.warning` | no |
| `components.dashboard.host_list.title` | style | `semantic.text_bright` | no |
| `components.dashboard.latency.background` | paint | `semantic.surface` | no |
| `components.dashboard.latency.border` | paint | `semantic.border` | yes |
| `components.dashboard.latency.border_focused` | paint | `semantic.border_focus` | yes |
| `components.dashboard.latency.title` | style | `semantic.text_bright` | no |
| `components.dashboard.metrics.sparkline_high` | color | `semantic.error` | no |
| `components.dashboard.metrics.sparkline_low` | color | `semantic.success` | no |
| `components.dashboard.metrics.sparkline_medium` | color | `semantic.warning` | no |
| `components.dashboard.ping.background` | paint | `semantic.surface` | no |
| `components.dashboard.ping.border` | paint | `semantic.border` | yes |
| `components.dashboard.ping.border_focused` | paint | `semantic.border_focus` | yes |
| `components.dashboard.ping.title` | style | `semantic.text_bright` | no |
| `components.dashboard.recent.background` | paint | `semantic.surface` | no |
| `components.dashboard.recent.border` | paint | `semantic.border` | yes |
| `components.dashboard.recent.border_focused` | paint | `semantic.border_focus` | yes |
| `components.dashboard.recent.title` | style | `semantic.text_bright` | no |
| `components.dashboard.ssh_log.background` | paint | `semantic.surface` | no |
| `components.dashboard.ssh_log.border` | paint | `semantic.border` | yes |
| `components.dashboard.ssh_log.border_focused` | paint | `semantic.border_focus` | yes |
| `components.dashboard.ssh_log.title` | style | `semantic.text_bright` | no |

#### `components.focus`

| Role | Type | Falls back to | Closed frame |
| --- | --- | --- | --- |
| `components.focus.indicator` | style | `semantic.accent` | no |

#### `components.footer`

| Role | Type | Falls back to | Closed frame |
| --- | --- | --- | --- |
| `components.footer.background` | paint | `semantic.surface_raised` | no |
| `components.footer.key` | style | `semantic.text_bright` | no |
| `components.footer.label` | style | `semantic.text_muted` | no |
| `components.footer.separator` | paint | `semantic.text_dim` | no |

#### `components.form`

| Role | Type | Falls back to | Closed frame |
| --- | --- | --- | --- |
| `components.form.error` | style | `semantic.error` | no |
| `components.form.help` | style | `semantic.text_dim` | no |
| `components.form.input` | style | `semantic.text_bright` | no |
| `components.form.input_editing` | style | `semantic.text_bright` + bold + underlined | no |
| `components.form.input_focused` | style | `semantic.text_bright` | no |
| `components.form.label` | style | `semantic.text_dim` | no |
| `components.form.label_editing` | style | `semantic.warning` | no |
| `components.form.label_focused` | style | `semantic.info` | no |
| `components.form.value` | style | `semantic.text` | no |

#### `components.group_form`

| Role | Type | Falls back to | Closed frame |
| --- | --- | --- | --- |
| `components.group_form.label` | style | `semantic.text_muted` | no |
| `components.group_form.label_focused` | style | `semantic.text_bright` + bold | no |
| `components.group_form.marker` | style | `semantic.text_bright` + bold | no |
| `components.group_form.value` | style | `semantic.text` | no |
| `components.group_form.value_focused` | style | `semantic.text_bright` + bold | no |

#### `components.header`

| Role | Type | Falls back to | Closed frame |
| --- | --- | --- | --- |
| `components.header.background` | paint | `semantic.surface_raised` | no |
| `components.header.brand` | style | `semantic.text_inverse` on `semantic.text_bright` | no |
| `components.header.separator` | paint | `semantic.text_dim` | no |
| `components.header.session_active` | style | `semantic.text_inverse` on `semantic.text_bright` | no |
| `components.header.session_error` | color | `semantic.error` | no |
| `components.header.session_inactive` | style | `semantic.text_muted` | no |
| `components.header.session_more` | style | `semantic.text_muted` | no |
| `components.header.session_success` | color | `semantic.success` | no |
| `components.header.session_warning` | color | `semantic.warning` | no |
| `components.header.stats_label` | style | `semantic.text_muted` | no |
| `components.header.stats_value` | style | `semantic.text` | no |

#### `components.help`

| Role | Type | Falls back to | Closed frame |
| --- | --- | --- | --- |
| `components.help.description` | style | `semantic.text` | no |
| `components.help.key` | style | `semantic.text_bright` | no |
| `components.help.section` | style | `semantic.text_bright` | no |

#### `components.identities`

| Role | Type | Falls back to | Closed frame |
| --- | --- | --- | --- |
| `components.identities.agent.count` | style | `semantic.text_bright` | no |
| `components.identities.agent.label` | style | `semantic.text_muted` | no |
| `components.identities.agent.separator` | paint | `semantic.text_dim` | no |
| `components.identities.agent.value` | style | `semantic.text` | no |
| `components.identities.card.border` | paint | `semantic.border` | yes |
| `components.identities.card.border_selected` | paint | `semantic.accent` | yes |
| `components.identities.card.credential` | color | `semantic.warning` | no |
| `components.identities.card.key_type` | style | `semantic.text_muted` | no |
| `components.identities.card.loaded` | color | `semantic.success` | no |
| `components.identities.card.metadata` | style | `semantic.text_dim` | no |
| `components.identities.card.missing` | color | `semantic.unknown` | no |
| `components.identities.card.name` | style | `semantic.text_bright` + bold | no |
| `components.identities.card.selection` | style | `semantic.selection_fg` on `semantic.selection_bg` | no |
| `components.identities.card.text` | style | `semantic.text` | no |
| `components.identities.empty` | style | `semantic.text_dim` | no |
| `components.identities.notice` | style | `semantic.warning` | no |

#### `components.keybind`

| Role | Type | Falls back to | Closed frame |
| --- | --- | --- | --- |
| `components.keybind.marker` | style | `semantic.text_highlight` on `semantic.selection_bg` | no |
| `components.keybind.row` | style | `semantic.text` | no |
| `components.keybind.row_selected` | style | `semantic.text_highlight` on `semantic.selection_bg` | no |
| `components.keybind.value` | style | `semantic.text_muted` | no |
| `components.keybind.value_bound` | style | `semantic.success` | no |
| `components.keybind.value_capturing` | style | `semantic.warning` | no |

#### `components.os_logo`

| Role | Type | Falls back to | Closed frame |
| --- | --- | --- | --- |
| `components.os_logo.fallback` | color | `semantic.info` | no |
| `components.os_logo.tint` | tint | `native` (the asset's own colours) | no |

#### `components.picker`

| Role | Type | Falls back to | Closed frame |
| --- | --- | --- | --- |
| `components.picker.badge_error` | color | `semantic.error` | no |
| `components.picker.badge_success` | color | `semantic.success` | no |
| `components.picker.badge_warning` | color | `semantic.warning` | no |
| `components.picker.border` | paint | `semantic.accent` | yes |
| `components.picker.marker` | style | `semantic.selection_fg` on `semantic.selection_bg` | no |
| `components.picker.match` | style | `semantic.accent` | no |
| `components.picker.query` | style | `semantic.text_bright` | no |
| `components.picker.row` | style | `semantic.text` | no |
| `components.picker.row_selected` | style | `semantic.selection_fg` on `semantic.selection_bg` | no |

#### `components.popup`

| Role | Type | Falls back to | Closed frame |
| --- | --- | --- | --- |
| `components.popup.background` | paint | `semantic.surface` | no |
| `components.popup.border` | paint | `semantic.border_popup` | yes |
| `components.popup.error` | style | `semantic.error` | no |
| `components.popup.hint` | style | `semantic.text_dim` | no |
| `components.popup.legend` | style | `semantic.text_muted` | no |
| `components.popup.title` | style | `semantic.text_bright` | no |
| `components.popup.warning` | style | `semantic.warning` | no |

#### `components.selection`

| Role | Type | Falls back to | Closed frame |
| --- | --- | --- | --- |
| `components.selection.active` | style | `semantic.selection_fg` on `semantic.selection_bg` | no |
| `components.selection.inactive` | style | `semantic.text` on `semantic.surface_raised` | no |

#### `components.separator`

| Role | Type | Falls back to | Closed frame |
| --- | --- | --- | --- |
| `components.separator.primary` | paint | `semantic.border` | no |
| `components.separator.secondary` | paint | `semantic.text_dim` | no |

#### `components.session`

| Role | Type | Falls back to | Closed frame |
| --- | --- | --- | --- |
| `components.session.background` | paint | `semantic.background` | no |
| `components.session.border` | paint | `semantic.border_popup` | yes |
| `components.session.connecting` | color | `semantic.connecting` | no |
| `components.session.debug_tail` | style | `semantic.text_dim` | no |
| `components.session.exited` | color | `semantic.exited` | no |
| `components.session.scrollback` | style | `semantic.warning` | no |
| `components.session.title` | style | `semantic.text_inverse` on `semantic.text_bright` | no |

#### `components.settings`

| Role | Type | Falls back to | Closed frame |
| --- | --- | --- | --- |
| `components.settings.marker` | style | `semantic.text_highlight` on `semantic.selection_bg` | no |
| `components.settings.row_selected` | style | `semantic.text_highlight` on `semantic.selection_bg` | no |

#### `components.sftp`

| Role | Type | Falls back to | Closed frame |
| --- | --- | --- | --- |
| `components.sftp.local` | style | `semantic.info` | no |
| `components.sftp.notice` | style | `semantic.warning` | no |
| `components.sftp.panel.background` | paint | `semantic.surface` | no |
| `components.sftp.panel.border` | paint | `semantic.border` | yes |
| `components.sftp.panel.border_focused` | paint | `semantic.border_focus` | yes |
| `components.sftp.panel.count` | style | `semantic.text_dim` | no |
| `components.sftp.panel.title` | style | `semantic.text_bright` | no |
| `components.sftp.progress` | style | `semantic.warning` | no |
| `components.sftp.progress_complete` | style | `semantic.success` | no |
| `components.sftp.progress_remaining` | style | `semantic.text_dim` | no |
| `components.sftp.queue_download` | style | `semantic.success` | no |
| `components.sftp.queue_header` | style | `semantic.text_bright` + bold | no |
| `components.sftp.queue_upload` | style | `semantic.warning` | no |
| `components.sftp.remote` | style | `semantic.info` | no |
| `components.sftp.search` | style | `semantic.text_inverse` on `semantic.warning` | no |
| `components.sftp.selection` | style | `semantic.selection_fg` on `semantic.selection_bg` | no |

#### `components.status`

| Role | Type | Falls back to | Closed frame |
| --- | --- | --- | --- |
| `components.status.error` | color | `semantic.error` | no |
| `components.status.info` | color | `semantic.info` | no |
| `components.status.success` | color | `semantic.success` | no |
| `components.status.unknown` | color | `semantic.unknown` | no |
| `components.status.warning` | color | `semantic.warning` | no |

#### `components.status_bar`

| Role | Type | Falls back to | Closed frame |
| --- | --- | --- | --- |
| `components.status_bar.background` | paint | `semantic.surface_raised` | no |
| `components.status_bar.error` | style | `semantic.error` | no |
| `components.status_bar.message` | style | `semantic.text` | no |
| `components.status_bar.mode` | style | `semantic.text_inverse` on `semantic.text_bright` | no |
| `components.status_bar.toast` | style | `semantic.info` | no |

#### `components.table`

| Role | Type | Falls back to | Closed frame |
| --- | --- | --- | --- |
| `components.table.row` | style | `semantic.text` | no |
| `components.table.row_selected` | style | `semantic.selection_fg` on `semantic.selection_bg` | no |

#### `components.tabs`

| Role | Type | Falls back to | Closed frame |
| --- | --- | --- | --- |
| `components.tabs.active` | style | `semantic.text_inverse` on `semantic.text_bright` | no |
| `components.tabs.inactive` | style | `semantic.text_muted` | no |
| `components.tabs.separator` | paint | `semantic.text_dim` | no |

#### `components.text`

| Role | Type | Falls back to | Closed frame |
| --- | --- | --- | --- |
| `components.text.bright` | style | `semantic.text_bright` | no |
| `components.text.dim` | style | `semantic.text_dim` | no |
| `components.text.inverse` | style | `semantic.text_inverse` on `semantic.text_bright` | no |
| `components.text.muted` | style | `semantic.text_muted` | no |
| `components.text.primary` | style | `semantic.text` | no |

#### `components.tunnel`

| Role | Type | Falls back to | Closed frame |
| --- | --- | --- | --- |
| `components.tunnel.connecting` | color | `semantic.connecting` | no |
| `components.tunnel.retrying` | color | `semantic.warning` | no |
| `components.tunnel.running` | color | `semantic.success` | no |
| `components.tunnel.stopped` | color | `semantic.error` | no |
| `components.tunnel.unknown` | color | `semantic.unknown` | no |

#### `components.tunnel_form`

| Role | Type | Falls back to | Closed frame |
| --- | --- | --- | --- |
| `components.tunnel_form.border` | paint | `semantic.accent` | yes |
| `components.tunnel_form.help` | style | `semantic.text_dim` | no |
| `components.tunnel_form.label` | style | `semantic.text_muted` | no |
| `components.tunnel_form.label_focused` | style | `semantic.text_bright` | no |
| `components.tunnel_form.marker` | style | `semantic.success` | no |
| `components.tunnel_form.title` | style | `semantic.accent` | no |
| `components.tunnel_form.value` | style | `semantic.text` | no |
| `components.tunnel_form.value_editing` | style | `semantic.text_highlight` | no |
| `components.tunnel_form.value_focused` | style | `semantic.text_bright` | no |

#### `components.tunnels`

| Role | Type | Falls back to | Closed frame |
| --- | --- | --- | --- |
| `components.tunnels.direction` | style | `semantic.info` | no |
| `components.tunnels.empty` | style | `semantic.text_dim` | no |
| `components.tunnels.error` | style | `semantic.error` | no |
| `components.tunnels.metadata` | style | `semantic.text_dim` | no |
| `components.tunnels.notice` | style | `semantic.warning` | no |
| `components.tunnels.remote` | style | `semantic.text_muted` | no |
| `components.tunnels.row` | style | `semantic.text` | no |
| `components.tunnels.row_selected` | style | `semantic.selection_fg` on `semantic.selection_bg` | no |
| `components.tunnels.separator` | paint | `semantic.text_dim` | no |
| `components.tunnels.summary` | style | `semantic.text_muted` | no |
| `components.tunnels.table_header` | style | `semantic.text_bright` + bold | no |

<!-- THEME_ROLES:END -->

---

## The theme picker

Reach it with **Ctrl+H** (Settings) → the **Theme…** row → **Enter**.

| Key | What it does |
| --- | --- |
| `↑` / `↓` | Move the selection, wrapping at both ends |
| `PageUp` / `PageDown` | Move a page, stopping at the ends |
| `Home` / `End` | First / last entry |
| `Shift+↑` / `Shift+↓` | Scroll the diagnostics footer |
| `Enter` | Save the selected theme and close |
| `r` | Re-read the themes directory |
| `Esc` | Restore the theme that was active on open, and close |

**Live preview.** Moving the selection applies the theme to the entire SSHub
interface immediately, not just to the preview box — the preview shows the
detail (text roles, frames, tabs, status words, a selected row, the footer key
pair, backgrounds and gradients) while the frame around it shows the real thing.
Nothing is written to disk.

**Rollback.** `Esc` puts back whatever was active when you opened the picker,
even if that theme's file has been deleted in the meantime.

**Reload.** `r` re-reads the directory. A file you have just repaired becomes the
live preview immediately; a file you have just broken or deleted leaves the last
valid theme painting, and the selection stays in its slot with an explanation.

**The three states**, shown in the list's right-hand column:

| State | Meaning |
| --- | --- |
| `valid` | Resolves cleanly. Previewable, savable. |
| `warning` | Resolves, but mentions component roles this SSHub does not know — most likely a file from a newer version, or a typo. Previewable and savable; every ignored role is listed in the footer. |
| `invalid` | Cannot be resolved. Listed anyway, with the reason, so the file can be fixed. Never previewed, and `Enter` refuses it. |

The footer under the list shows, in this order: a failed save or reload, then
the selected theme's own diagnostics, then directory-level ones (an unusable
file name, the 256-file cut), and — when there is nothing wrong — the theme's
`description`. When there is more than fits, the legend says how many lines are
hidden and `Shift+↑`/`Shift+↓` scroll to them.

The row above the legend shows the selected user theme's path, or the themes
directory itself, so you always know where a new file goes.

---

## The CLI

All three commands run headless: no TUI, no database, no keyring.

```text
sshub theme check <file> [--format plain|json]
sshub theme list         [--format plain|json]
sshub theme show <id>    [--resolved] [--format toml|json]
```

### `theme check`

```console
$ sshub theme check ~/.config/sshub/themes/ocean.toml
OK: ocean (extends aqua), 23 colors, 2 gradients, 19 overrides
```

Failures carry file, line and column wherever the TOML parser can supply a
position, and a suggestion when a key is close enough to a real one:

```console
$ sshub theme check ~/.config/sshub/themes/ocean.toml
/home/you/.config/sshub/themes/ocean.toml:9:9 error: unknown component role `components.dashboard.host_list.bordr`
  help: did you mean `components.dashboard.host_list.border`?
FAILED: ocean — 1 error(s), 0 warning(s)
```

Independent problems are collected in one run rather than reported one per
invocation. `check` runs in **strict** mode, so an unknown component role is an
error here even though the running application would only warn about it.

The directory of the checked file forms the registry for that run, so a portable
theme package with a sibling parent can be checked as a unit. A parent resolved
from a sibling file gets a warning: it must be installed alongside the child.

### `theme list`

```console
$ sshub theme list
ID             NAME           STATE      SOURCE
default        Default        ok         built-in
summer         Summer         ok         built-in
aqua           Aqua           ok         built-in
fire           Fire           ok         built-in
high-contrast  High Contrast  ok         built-in
```

### `theme show`

`theme show <id>` prints the theme's source verbatim — comments and all — which
is what makes the copy workflow work. `--resolved` instead writes a complete,
standalone document with every semantic slot, gradient and component role
written out and no `extends`; it re-reads to exactly the same runtime theme.

```bash
sshub theme show aqua > ~/.config/sshub/themes/aqua-custom.toml
sshub theme check ~/.config/sshub/themes/aqua-custom.toml
```

The exported source starts with a comment naming the theme it came from and
reminding you to change `name` — the ID comes from the file name, but two
entries reading "Aqua" in the picker are nobody's friend.

`--format json` gives the same content structured; for `--resolved` that
includes `id`, `name`, `description`, `author`, every semantic slot, every
gradient with its stops, and every component role with its resolved value.

### Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Valid, possibly with warnings |
| `1` | Validation error, file error, or unknown theme ID for `show` |
| `2` | Wrong CLI usage: unknown option, missing argument, incompatible formats |

`theme list` returns `0` whenever the registry could be read, even if individual
themes are `warning` or `invalid` — their state is in the output. An I/O error
reading the registry is `1`.

---

## Limits and troubleshooting

**True Color.** SSHub emits RGB values. If your terminal does not support or
advertise true colour, its own colour reduction decides what you see. SSHub
makes no promise of colour fidelity there, but it never panics and the status
words (`up`, `warning`, `error`) stay readable as text, not just as colour.

**The remote PTY is never recoloured.** Theme backgrounds and gradients
explicitly exclude the embedded session viewport, even where its cells are
transparent. The colours your remote shell prints are the remote shell's.

**`opaque_background`.** The existing Settings toggle keeps its old meaning: when
it is on *and* your theme's `components.app.background` resolves to `"terminal"`,
otherwise transparent cells are filled with `semantic.canvas`, exactly as before
the theme system existed. It cannot override an app background a theme has set
deliberately, and it is the only thing allowed to put a solid backdrop behind
the remote PTY.

**An invalid theme never stops SSHub from starting.** If `active_theme` names a
theme that no longer resolves, SSHub falls back to `default`, shows the reason
through the non-fatal notice channel, and **does not rewrite your config** — so
fixing the file and restarting brings your theme back without you having to
select it again. One broken file also does not invalidate the others.

**Nothing is written to your themes directory.** SSHub never deletes or rewrites
a theme file. The only thing a successful `Enter` in the picker writes is
`appearance.active_theme` in `config.toml`, and the existing `toml_edit` merge
keeps your comments and unknown settings intact.

**Limits.** Per theme file: 1 MiB, 256 palette entries, 128 gradients, 32 stops
per gradient, inheritance depth 16, colour reference depth 16. Per directory:
256 theme files. Exceeding a limit is reported, not silently truncated.

**There is no file watcher.** Edit your file, then press `r` in the picker.

---

## Worked examples

Both examples below are extracted from this file by the test suite and run
through the real strict pipeline, so they are known to validate and resolve.
Copy either one into `~/.config/sshub/themes/<id>.toml` as it stands.

### 1. Minimal: just change the accent colour

Everything else stays exactly as `default` has it. Because the component roles
reference `semantic.accent` rather than a fixed colour, this one line moves the
focus indicators, the field markers, the accent text and the animation accents
together.

<!-- THEME_EXAMPLE:START id=mint -->
```toml
schema_version = 1
name = "Mint"
description = "default, with a brighter accent."

[semantic]
accent = "#5eead4"
```
<!-- THEME_EXAMPLE:END -->

```bash
$EDITOR ~/.config/sshub/themes/mint.toml
sshub theme check ~/.config/sshub/themes/mint.toml
```

### 2. Substantial: inheritance, palette maths, gradients and overrides

This one starts from `aqua` rather than `default`, defines its own palette
(including a colour built with simulated opacity over an explicit opaque
ground), moves several semantic slots, and then reaches for individual roles: a
four-stop `perimeter` ring on the focused panel frame, a horizontal ramp on the
separator, a restyled panel title, and one gradient it inherits from `aqua`
dropped again with `"auto"`.

<!-- THEME_EXAMPLE:START id=harbour -->
```toml
schema_version = 1
name = "Harbour"
description = "Aqua at dusk — a lantern-lit focus ring over deep water."
author = "the SSHub theme guide"
extends = "aqua"

[palette]
dusk = "#0a1b26"
lantern = "#ffb454"
# Simulated opacity needs an opaque mixing ground; `dusk` is one.
haze = { color = "palette.lantern", opacity = 0.35, over = "palette.dusk" }

[semantic]
# Painting the app background opts this theme out of terminal transparency.
background = "palette.dusk"
canvas = "palette.dusk"
accent = "palette.lantern"
# A slightly lifted surface, computed rather than eyeballed.
surface = { color = "palette.dusk", brightness = 0.08 }

# A closed ring: the first and last stop resolve to the same colour, so there
# is no seam where the ring meets itself.
[gradients.lantern_ring]
direction = "perimeter"
stops = [
  { at = 0.0, color = "palette.dusk" },
  { at = 0.35, color = "palette.lantern" },
  { at = 0.7, color = "semantic.info" },
  { at = 1.0, color = "palette.dusk" },
]

[gradients.dusk_line]
direction = "horizontal"
stops = [
  { at = 0.0, color = "palette.haze" },
  { at = 0.5, color = "palette.lantern" },
  { at = 1.0, color = "palette.haze" },
]

[components.dashboard.host_list]
border_focused = { gradient = "gradients.lantern_ring" }
title = { foreground = "palette.lantern", modifiers = ["bold"] }

[components.separator]
primary = { gradient = "gradients.dusk_line" }

# `aqua` rings every popup frame with its own gradient; this drops that
# inherited override, so popups go back to the plain `semantic.border_popup`.
[components.popup]
border = "auto"
```
<!-- THEME_EXAMPLE:END -->

Install and check it:

```bash
$EDITOR ~/.config/sshub/themes/harbour.toml
sshub theme check ~/.config/sshub/themes/harbour.toml
sshub                                    # Ctrl+H → Theme… → Harbour → Enter
```

---

## See also

- [README](../README.md) — the short version.
- `man sshub` — the `theme` subcommands.
- [docs/theme-render-benchmark.md](theme-render-benchmark.md) — the measured
  cost of gradient rendering.
