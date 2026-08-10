# PTY ground: themed background and foreground under the remote grid (PR #86 review)

## Problem

Manual verification of the theme system on PR #86 surfaced two defects in how
the embedded remote PTY relates to the active theme.

**1. A half fill makes light themes unreadable.** With `appearance.opaque_background`
enabled, `apply_app_background` fills every remaining `Color::Reset` cell —
the PTY viewport included — via `fill_reset_background` (`src/tui/mod.rs:236`).
That helper writes **only the background channel** (`src/tui/mod.rs:435-437`).
The remote grid's default foreground stays `Color::Reset`, so the host emulator
paints its own default foreground into our themed ground. Under `summer`, whose
canvas is cream, that is the emulator's near-white on cream: technically
rendered, practically invisible.

The defect is not specific to `summer`. Every light theme hits it, and a user
running a light *emulator* profile hits the mirror image under a dark theme
(dark default foreground on a dark canvas). Relying on "emulators default to a
light foreground" is what makes it look fixed on dark themes.

**2. The PTY does not follow the theme.** The PTY viewport is deliberately
listed in `FrameComposition::protected` (`src/tui/mod.rs:275-281`) so theme
paint can never recolour remote output. Consequently its untouched cells stay
`Color::Reset` and the host terminal's own ground — wallpaper included — shows
through, while every surface around it carries the theme. Today the only way to
back those cells is the global `opaque_background` switch, which couples "app
chrome is opaque" to "PTY is opaque" and offers no middle ground.

`components.session.background` already exists as a role, but it reaches only
the connecting and failure screens (`src/session/render.rs:420`); it never
touches the live grid.

## Why not an alpha channel

The reporter suggested that "Enable opaque" could modify an alpha channel, or
become an alpha slider. That is not implementable in a terminal grid, and the
reasoning belongs in the record:

- ANSI has no per-cell alpha. A cell carries either a colour or "default"; there
  is no third state and no blend factor.
- We cannot blend against what is behind the window. The wallpaper, or another
  window, is owned by the compositor, and the application never sees it.
- Blending is exclusively the host emulator's job (kitty `background_opacity`,
  WezTerm `window_background_opacity`), and it applies **only to cells with a
  default background** — precisely `Color::Reset`. The moment we write a colour,
  that cell is fully opaque and the emulator's opacity setting no longer applies
  to it.

The usable core of the idea survives as the design below: give the PTY its own
themeable ground, and give the user a switch that hands the surface back to the
emulator, where opacity actually works.

## Decisions

- A theme that paints its own ground paints it everywhere, the PTY included.
- Two new slots join the semantic core (23 → 25): `pty_background` and
  `pty_foreground`.
- In `assets/themes/default.toml` they are defined as *references*,
  `"semantic.background"` and `"semantic.text"`, not literals. No other theme
  file changes, and every derived user theme inherits the rule.
- Background and foreground are written as a **pair** or not at all.
- Only channels that hold `Color::Reset` are touched, tested per channel. Real
  ANSI colours from the remote are never rewritten. This is unchanged from today.
- The PTY ground is always a solid colour, never a gradient.
- **SSHub is opaque out of the box; transparency is the user's explicit
  choice.** Where a theme resolves a ground to `"terminal"`, `semantic.canvas`
  backs it, and the grid falls back to the `canvas`/`text` pair.
- `appearance.opaque_background` is **removed**, replaced by two independent
  toggles, both default `false`: `transparent_sshub_background` releases
  SSHub's own ground, `transparent_session_background` releases the grid.
- Rationale, found by manual testing: a switch asking to *fill* can only act
  where a theme left something unfilled, so it is inert under a theme that
  paints everything — measurably so, zero cells changed under four of the five
  built-ins. Which direction is open depends on the theme; asking to *release*
  is the question every theme can answer, so both toggles ask it.
- Releasing works by **role**, not by colour, and happens before anything is
  drawn: `ResolvedTheme::with_ground_released` resolves the four ground slots
  and every component role whose catalogue fallback is one of them to
  `Color::Reset`, and `App::theme` hands that view to the renderers. This
  reaches the panel bodies a theme paints through `semantic.surface` — which
  the widgets draw themselves, and which skipping the ground passes alone left
  untouched (only 408 of 1920 cells came free under `fire`).
- A colour comparison over the finished frame cannot do this correctly: a theme
  may give `selection_bg` and `surface` the same value, and after the release
  the two are no longer alike — one is wallpaper, the other still has to mark
  the selected row. Only the catalogue fallback can tell them apart.
- `Color` and `Tint` roles are never released: those paint marks and logos, not
  surfaces.
- A theme's `pty_background = "terminal"` now falls back to the canvas rather
  than to the emulator. Releasing a surface is the user's call, not the
  theme's.
- The ANSI palette (remapping the remote's 16 colours to the theme) is out of
  scope and becomes its own issue.

## The two semantic slots

```toml
# assets/themes/default.toml
pty_background = "semantic.background"
pty_foreground = "semantic.text"
```

Resolution order is irrelevant: `resolve_semantic` resolves each slot through
`resolve_reference`, which looks the target up in the merged core, recurses,
caches, and reports cycles (`src/theme/resolve.rs:473-597`). A slot referencing
another slot is therefore already supported, and merging happens before
resolution — so in `summer` the reference resolves to *summer's* background, not
to `default`'s.

The resulting behaviour, with no per-theme edits:

| Theme | `semantic.background` | PTY ground |
| --- | --- | --- |
| `default` | `"terminal"` | the `canvas`/`text` fallback |
| `summer` | `palette.cream` | cream with `palette.ink` text |
| `aqua` | `palette.abyss` | abyss with its text ramp |
| `fire` | `palette.coal` | coal with its text ramp |
| `high-contrast` | `#000000` | pure black ground with its text ramp |

A theme writing `pty_background = "terminal"` drops its own grid colour and
lands on the `canvas`/`text` fallback. It cannot make the grid see-through —
that is the user's call, through `transparent_session_background`.

`summer` and `aqua` set `components.app.background` to a **gradient** while
their `semantic.background` is solid. The PTY takes the solid semantic value.
This is deliberate: a gradient sweeping under arbitrary remote output has no
stable contrast against the remote's own foreground colours.

## The pair invariant

Wherever a PTY ground is painted, both channels are written for cells that carry
`Color::Reset` in that channel:

- `bg == Color::Reset` → the resolved PTY background
- `fg == Color::Reset` → the resolved PTY foreground

Writing only one channel is the defect in problem 1, and it has a second failure
mode: a cell with `Modifier::REVERSED` and both channels at `Reset` would, after
a background-only fill, swap our ground against a foreground that was never
defined. Writing the pair keeps reverse video correct because the emulator swaps
two known colours.

Cells the remote coloured explicitly keep their colours in both channels. A cell
with an explicit foreground and a default background receives only the
background, and vice versa — the per-channel `Reset` test handles this without a
special case.

## Render passes

`apply_app_background` (`src/tui/mod.rs:210`) grows from two passes to three.
The first is unchanged.

1. **Theme app background.** Paints SSHub's own surfaces, excluding
   `composition.protected`. Unchanged.
2. **PTY ground** (new, `apply_pty_ground`). Runs over `composition.protected` —
   the resting viewport plus the travelling bands of the exit slide and the
   session-tab slide (`src/tui/mod.rs:262-305`), so a session in transit is
   backed exactly like one at rest. `transparent_session_background` short-
   circuits the whole pass: the grid goes back to the emulator untouched.
   Otherwise, source of the pair:
   - `semantic.pty_background != Color::Reset` → that pair,
     `(pty_background, pty_foreground)`.
   - else → `(canvas, text)`, so the grid is opaque under every theme.
3. **Canvas fill.** Backs whatever pass 1 left — the cells a theme resolving to
   `"terminal"` never claimed. Takes `exclusions = &composition.protected`,
   because pass 2 has already decided what happens inside those regions.
   `transparent_sshub_background` skips passes 1 and 3 entirely.

Pass ordering matters and is asserted by tests: pass 1 cannot reach protected
regions, pass 2 owns them, pass 3 owns everything left over.

## Interaction with the splash fade

The dashboard fade-in (`src/tui/mod.rs:173-193`) blends towards
`PaintRole::AppBackground` and already excludes `composition.protected`, so it
does not interact with the new pass. The existing invariant — that the fade and
the exit slide never recolour remote cells — is unchanged, and the tests around
`allowed_pty_background` (`src/tui/mod.rs:2924`) are extended rather than
rewritten: the allowed set for a themed PTY gains the theme's `pty_background`.

## Files touched

| File | Change |
| --- | --- |
| `src/theme/catalog.rs` | two `SEMANTIC_SPECS` entries + `SemanticSlot` variants |
| `src/theme/model.rs` | `ResolvedSemantic` fields, `from_slots`, `slot()`, `with_ground_released`, the slot-count doc comments |
| `assets/themes/default.toml` | the two reference definitions, with the rationale as a comment |
| `src/tui/mod.rs` | `apply_pty_ground`, the exclusion in pass 3, `fill_reset_pair` helper |
| `src/config.rs` | `appearance.transparent_session_background` |
| `src/app/types.rs`, `src/app/keys.rs` | the Settings row and its toggle |
| `README.md` | the settings-overlay feature lines |
| `docs/theme-system.md` | 23 → 25 semantic meanings, a section on the PTY ground and on why alpha is not available |
| `src/theme/role_matrix.snapshot` | regenerated if the matrix output changes |
| `CHANGELOG.md` | entry under `[Unreleased]` |

`SEMANTIC_SLOT_COUNT` is derived from `SEMANTIC_SPECS.len()`
(`src/theme/model.rs:660`), so array sizes follow automatically.

## Testing

### Theme resolution

- `default` resolves `pty_background` to `Color::Reset` — it references
  `background`, which is `"terminal"` — while `pty_foreground` follows
  `semantic.text` and is a colour like any other. The grid is opaque anyway,
  through the `canvas`/`text` fallback.
- `summer` resolves `pty_background` to its cream and `pty_foreground` to its
  ink, proving the reference resolves post-merge against the child theme.
- A user theme setting `pty_background = "terminal"` on top of `summer` resolves
  back to `Color::Reset`, and its grid then wears the canvas pair rather than
  going see-through: releasing a surface is the user's call, not the theme's.
- A cycle (`pty_background = "semantic.pty_foreground"` and back) is reported as
  a reference cycle, not a panic or a silent black.
- The existing "semantic core must be complete after inheritance" check still
  passes for all five built-ins.

### Render

- **Regression for problem 1:** under `summer`, no cell in the PTY viewport has
  `bg != Reset` while `fg == Reset`. This is the assertion that fails on today's
  code.
- **Regression for problem 2:** under `summer`, every untouched PTY cell carries
  the theme's cream, not `Reset`.
- Under every built-in, SSHub's own surfaces carry a ground out of the box, and
  each toggle releases its own surface without touching the other.
- A cell the remote coloured explicitly keeps both of its colours under every
  theme and both switch positions.
- A `REVERSED` cell with both channels at `Reset` ends with both channels set.
- The travelling band of an exit slide and of a session-tab slide is backed with
  the same pair as the resting viewport.
- With `transparent_session_background` on, `fire` leaves both PTY channels at
  `Reset` while its own chrome stays painted.
- With only `transparent_session_background` on, `default` leaves the grid at
  `Reset` and still fills every cell outside it — the division of labour between
  the two. With *both* on, nothing is filled anywhere, which is the point of the
  other switch.
- With only `transparent_sshub_background` on, `default`'s grid keeps the
  *authored* canvas pair. The released view has every ground slot at `Reset`, so
  the grid has to read its fallback from the theme as written, or the two
  switches stop being independent.
- A theme leaving `pty_foreground` **and** `text` to the emulator gets no ground
  at all: with no honest foreground left, writing the background alone would be
  the reported bug again.
- `sshub_is_opaque_by_default_and_transparent_on_request`: no transparent cell
  survives in the shipped state, and asking for transparency frees some.
- The `allowed_pty_background` matrix (`src/tui/mod.rs:2924`) is extended to
  cover the themed ground across the splash fade and the exit slide.

### Headless

- `sshub theme check` stays clean for all five built-ins.
- `sshub theme show --resolved` round-trips: its output re-reads to the same
  theme, now including the two new slots.

## Out of scope

**ANSI palette remapping**, tracked as its own issue. Without it, a remote that
emits colour — `ls`, `git status`, a coloured prompt — still chooses from the
emulator's 16-colour palette, which on `summer`'s cream ground can be
low-contrast (bright yellow on cream). The design there is a `[terminal]`
section carrying the 16 ANSI colours plus cursor, remapped in the vt100 render
path; the two slots introduced here are its natural default background and
foreground. This spec deliberately does not pre-build that structure.
