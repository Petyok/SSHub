# Embedded terminal compatibility

SSHub compiles its terminal model from `src/terminal/`. It is adapted from
Jesse Luehrs' vt100 and iksteen/shellglass revision
`dc7f399f4ffc3daf0593ff9a272338ac9b1affc6`. Both the vt100 MIT license and the
MIT license of tui-term's adapted renderer bridge are included beside the
source. There are no terminal git/path dependencies or Cargo patches: the
published SSHub crate contains the same terminal model as the checkout.
`tui-term` runs without its default vt100 feature and renders the local model
through its `Screen` and `Cell` traits.

## Query policy

The model intentionally ignores identity/status queries. `ParserState` uses
an independent, persistent vte observer to answer DSR 5/6, DA1, character-grid
size (`CSI 18 t`) and DECRQM for synchronized output (2026). It flushes screen
bytes at each completed query and synchronized-update boundary, so replies
describe state at the query, not
at the end of a PTY read. The observer advances whole spans using
`advance_until_terminated`, avoiding per-byte method overhead on plain output.

Following [PR #114](https://github.com/Petyok/SSHub/pull/114), kitty keyboard,
DA2 and DECXCPR remain unanswered. A kitty query followed by DA1 receives only
DA1. A zero-flags kitty reply would incorrectly advertise protocol support.
Clipboard reads remain refused. Query queues, the reply rate limiter and the
background PTY writer retain their existing bounds.

## Presentation and input

Mode 2026 caches only the remote pane. The stream observer locates transaction
boundaries and captures at the last BSU when the read ends in active mode,
before parsing the following partial frame. This preserves the latest complete
frame with at most one capture per input block, even in a flood of mode toggles.
When the mode ends, the next draw uses the live grid. Headers, footer
notices, selection and ghost overlays still render. The one-second deadline
is measured from the last pane draw, not from the latest BSU, so a continuous
BSU stream cannot extend a hold indefinitely. At the deadline the pane
fails open and continues drawing at the normal rate until ESU; repeated BSU
does not rearm it. A new transaction after ESU can hold again. Resize
invalidates the snapshot; scrollback and returning from scrollback bypass
the hold. Active selection and inline completion fail open for the current
transaction because copying and completion placement use the live model.
The pane buffer is reused at unchanged dimensions, with cells reset before
each capture so old text and attributes cannot leak into a new frame.

The PTY keeps its original minimum size of 1x1. The model handles wrapping in
a one-row grid by marking soft wraps before moving the source row into history.

For mouse 1016, PTY creation and resize report physical body dimensions derived
from the host window, excluding SSHub's header/footer. Reports encode the
centre of the cell using the actual size read back from the PTY. Unknown
pixel geometry causes a report to be dropped rather than mislabeled cell
coordinates. Mouse 1005 validates every encoded value against its maximum
2047 and produces a complete report or no report. Mouse 1015 uses decimal
urxvt coordinates.

## Resource and rendering limits

- REP is limited to rows × columns for each command.
- The raw OSC buffer is fixed at 96 KiB; the query observer's OSC buffer is
  fixed at 1 KiB. This also bounds unterminated OSC strings.
- At most 8192 URIs are retained, each at most 2048 UTF-8 bytes. Oversized or invalid UTF-8 links
  are rejected and clear the current link attribute.
- Titles use at most 4096 UTF-8 bytes; the saved-title stack has 16 entries.
- The richer model retains 44-byte cells; this is a deliberate memory cost of
  the parsed attributes rather than a claim of zero-cost compatibility.

Bold, italic, single underline, inverse, dim, colours, strikethrough, conceal,
blink and cursor shape are mapped to the renderer. Default OSC colours affect
cells that use default colours, before the cursor overlay is drawn. Underline variants are parsed but draw as a
single underline; hyperlink metadata is bounded but not emitted to the outer
terminal. These limits should not be described as full visual support for all
parsed attributes or all terminal applications.

## Verification

Run `just test`, `cargo fmt --all -- --check` and
`cargo clippy --all-targets -- -D warnings`.

`cargo package --allow-dirty --locked` verifies a default-feature build from
normalized, packaged sources. A local installation of the extracted package
with `cargo install --path target/package/sshub-0.17.3 --debug --locked --root
<temporary-root>` additionally checks installation; neither command publishes.

Regression coverage includes HVP, cursor save/restore, query order and split
reads, private-query silence, character-size replies after resize, the repeated
BSU presentation deadline, permanent fail-open, coalesced and split frame
boundaries, bounded mode-toggle rendering, ESU release, resize/scrollback
bypass, 1x1 wrapping and height-only resize,
REP bounds, OSC storage bounds, exact UTF-8 mouse bytes and actual kernel PTY
pixel geometry on creation and resize.

`cargo run --release --no-default-features --example terminal_compat_probe`
measures bytewise versus bulk observer parsing and a 4 KiB maximum-count REP
stream. These measurements are local-development probes, not assertions with
machine-dependent timing thresholds.

With btop on PATH, `cargo run --no-default-features --example
terminal_compat_probe -- btop <frame.json>` runs real btop in a local PTY with
isolated configuration for six seconds and captures SSHub's rendered cells.
The capture uses Ratatui's TestBackend, not a desktop-window screenshot. It
checks the local process/PTY/parser/renderer path, not remote SSH transport or
all btop menus. No claims are made for untested nvim, helix, lazygit or tmux.
