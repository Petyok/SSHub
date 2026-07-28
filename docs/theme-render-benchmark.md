# Theme render benchmark

The runtime theme system paints gradients by post-processing the ratatui buffer
after a frame has been drawn. The design constraint was that this must cost
**less than 2 ms of additional median render time** per frame compared with a
theme that has no gradients at all.

This document is the evidence for that number. It is **not** a CI gate: the
benchmark asserts nothing about timing, and it is `#[ignore]`d so it only runs
when someone asks for it. A shared runner's scheduling noise is larger than the
effect being measured, so a timing assertion there would fail for reasons that
have nothing to do with SSHub.

## How to reproduce

```bash
cargo test --release --lib tui::tests::theme_gradient_release_benchmark \
  -- --ignored --exact --nocapture
```

The benchmark lives in `src/tui/mod.rs` (`tui::tests::theme_gradient_release_benchmark`).
It renders the **real** frame — the same `tui::render` the application calls —
at `200x60`, which is a large terminal on purpose: gradient work scales with the
number of cells, so a big frame is the unfavourable case.

Both themes are built-ins, chosen so that the difference between them is the
gradient work and nothing else:

| Side | Theme | Why |
| --- | --- | --- |
| solid | `high-contrast` | Paints an opaque app background, defines **no** gradients |
| gradient | `fire` | Paints an opaque app background, defines 10 gradients across frames, separators and backgrounds |

Per theme: 100 warm-up frames that are thrown away, then 1,000 measured frames.
The durations are sorted and the median is reported, so a single scheduling
hiccup cannot move the result.

## Measurement

| | |
| --- | --- |
| Date | 2026-07-28 |
| CPU | 13th Gen Intel(R) Core(TM) i9-13950HX (32 logical CPUs) |
| OS | Ubuntu 24.04.4 LTS on Linux 6.18.33.2-microsoft-standard-WSL2 |
| Toolchain | `rustc 1.93.0 (254b59607 2026-01-19)`, `--release` |
| Frame size | 200 × 60 cells |
| Warm-up | 100 frames per theme |
| Samples | 1,000 frames per theme |

Four consecutive runs on an otherwise idle machine:

| Run | Solid median (`high-contrast`) | Gradient median (`fire`) | Delta |
| --- | --- | --- | --- |
| 1 | 0.208 ms | 0.213 ms | 0.005 ms |
| 2 | 0.200 ms | 0.204 ms | 0.004 ms |
| 3 | 0.191 ms | 0.210 ms | 0.019 ms |
| 4 | 0.193 ms | 0.217 ms | 0.023 ms |

**Representative figures — run 1:** solid `0.208 ms`, gradient `0.213 ms`,
delta `0.005 ms`.

**Result:** the additional median render time of a full gradient theme is
between `0.004 ms` and `0.023 ms` on this machine — roughly two orders of
magnitude below the `2 ms` acceptance criterion. The spread between runs is
larger than the effect itself, which is the honest way to read these numbers:
the gradient pass is not measurably expensive at this frame size.

## Why it is this cheap

Gradient sampling allocates nothing per cell — no `Vec`, `String`, `Box` or heap
closure — and runs only over the rects that actually carry a gradient role, not
over the whole buffer. A theme without gradients never enters the painter at
all: the solid path is a plain blanking pass, which is why `default` and
`high-contrast` cost exactly what they did before the theme system existed.
