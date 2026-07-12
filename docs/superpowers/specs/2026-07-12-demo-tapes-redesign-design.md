# Demo tapes redesign

Date: 2026-07-12. Status: approved.

## Goal

Replace the sprawling ~85 s `overview.tape` with a set of short, uniformly
styled clips: one hero GIF at the top of the README plus a focused clip per
feature section. Every clip stays under ~20 s of visible time so the GIFs are
light and hold attention. (The OOM that used to kill long recordings is
already fixed separately — tapes render to mp4 and `demo/record.sh` does a
streaming two-pass GIF encode — so length here is an editorial budget, not a
technical one.)

## Shared style

All tapes use the same header: 1200x700, FontSize 16, Catppuccin Mocha,
Framerate 24, `TypingSpeed 40ms` (down from 90 ms). Pauses 400–900 ms between
steps instead of 800–1800 ms; each clip holds its final frame ~2.5–3 s so the
GIF loop doesn't snap. Non-hero tapes boot with `disable_animation = true`
via the copied-config trick from `sftp.tape` / `screenshots.tape`.

## The set

| Tape | ~Visible | Content |
|------|----------|---------|
| `hero.tape` (new) | ~16 s | Intro animation plays off-camera (`Hide` + ~11.5 s wait; animation lasts 9.95 s); open on its settled final frame ~2.5 s → Enter → walk 3 hosts in the tree (card follows) → `/` fuzzy → connect → `uptime` in the embedded PTY → Ctrl+D detach (session strip) → hold. |
| `navigate.tape` (new) | ~17 s | Walk nested groups → Space collapse/expand → `/` fuzzy palette (peek, Esc) → Shift+G group manager (peek, Esc) → `#` multi-tag filter, toggle two tags, Enter apply → hold on the filtered list. |
| `connect.tape` (rewrite) | ~20 s | Same scenes as before, tightened: palette → PTY → cowsay → Ctrl+D detach → Ctrl+Alt+N attach → Ctrl+T second tab (quad) → Ctrl+A switch → Ctrl+D → Ctrl+W ×2 close → hold on dashboard. |
| `add-host.tape` (rewrite) | ~16 s | Same flow (form fields, multi-group picker, tags, Ctrl+S, `/` find the new host), field pauses cut to ~400 ms. |
| `sftp.tape` | untouched | Already re-recorded and current on this branch. |
| `screenshots.tape` | untouched | PNG-only. |

`overview.tape` and `demo/gifs/overview.gif` are deleted.

## Wiring

- `demo/record.sh` default list becomes: hero, navigate, connect, add-host,
  sftp, screenshots.
- README: `hero.gif` replaces `overview.gif` at the top; a new "Navigating"
  blurb with `navigate.gif` lands before the Connect section; other sections
  keep their filenames.

## Verification

`vhs validate` on every tape; recording itself is run by the user
(`just record-gifs`), after which the refreshed GIFs get committed.
