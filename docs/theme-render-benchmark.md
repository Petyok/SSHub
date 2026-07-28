# Theme render benchmark

The runtime theme system paints gradients by post-processing the ratatui buffer
after a frame has been drawn. The design constraint was that this must cost
**less than 2 ms of additional median render time** per frame compared with a
theme that has no gradients at all.

This document is the evidence for that constraint. Read it for what it is: a
**smoke measurement that establishes an upper bound**, not a controlled
experiment that attributes a cost to the gradient pass.

- It is **not** a CI gate. The benchmark asserts nothing about timing and is
  `#[ignore]`d, so it only runs when someone asks for it. A shared runner's
  scheduling noise is larger than the effect being measured, so a timing
  assertion there would fail for reasons that have nothing to do with SSHub.
- It is **not** an isolation experiment. It compares two differently configured
  built-in themes, always measured in the same order. The printed delta is the
  combined effect of everything those two themes do differently, plus whatever
  drift the fixed ordering introduces.

That is enough to answer the question the constraint asks — "can a gradient
theme cost 2 ms more per frame?" — and it answers it with a very wide margin.
It is not enough to say what a gradient pass costs. Isolating that would mean
measuring one app state twice, once with gradient paints and once with
equivalent solid paints, alternating the order between runs. That has not been
done, and no claim here depends on it.

## How to reproduce

```bash
cargo test --release --lib tui::tests::theme_gradient_release_benchmark \
  -- --ignored --exact --nocapture
```

The benchmark lives in `src/tui/mod.rs` (`tui::tests::theme_gradient_release_benchmark`).
It renders the **real** frame — the same `tui::render` the application calls —
at `200x60`, which is a large terminal on purpose: gradient work scales with the
number of cells, so a big frame is the unfavourable case.

Both sides are built-ins. `high-contrast` is the closest comparison the
built-ins offer, because at least the presence of an opaque background pass is
not what separates the two:

| Side | Theme | What it defines |
| --- | --- | --- |
| solid | `high-contrast` | Opaque app background, **no** gradients at all |
| gradient | `fire` | Opaque app background, 3 gradient definitions (`blaze`, `updraft`, `cinder`) referenced by 5 roles: three focused panel frames, the primary separator and the tunnel-table separator. No background gradient. |

They still differ in every other value they set. Per theme: 100 warm-up frames
that are thrown away, then 1,000 measured frames; the durations are sorted and
the median is reported, so a single scheduling hiccup cannot move the result.

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

Ten consecutive runs on an otherwise idle machine:

| Run | Solid median (`high-contrast`) | Gradient median (`fire`) | Delta |
| --- | --- | --- | --- |
| 1 | 0.208 ms | 0.213 ms | 0.005 ms |
| 2 | 0.200 ms | 0.204 ms | 0.004 ms |
| 3 | 0.191 ms | 0.210 ms | 0.019 ms |
| 4 | 0.193 ms | 0.217 ms | 0.023 ms |
| 5 | 0.210 ms | 0.220 ms | 0.009 ms |
| 6 | 0.209 ms | 0.211 ms | 0.002 ms |
| 7 | 0.209 ms | 0.221 ms | 0.012 ms |
| 8 | 0.212 ms | 0.221 ms | 0.009 ms |
| 9 | 0.210 ms | 0.214 ms | 0.004 ms |
| 10 | 0.205 ms | 0.220 ms | 0.015 ms |

An independent reviewer's repeat run on different hardware produced solid
`0.224 ms`, gradient `0.218 ms`, delta `0.000 ms` — the gradient side measuring
*faster* than the solid one, which is the clearest possible statement of how far
this sits inside the noise floor.

**Result:** across every run recorded here the observed difference stays under
`0.03 ms`, and one independent run put it at zero or below. Whatever a gradient
theme costs per frame at `200x60`, it is bounded far below the `2 ms`
acceptance criterion — roughly two orders of magnitude below. The measurement
does not support any narrower claim than that, and none is made.

## Why an upper bound this low is plausible

Gradient sampling allocates nothing per cell — no `Vec`, `String`, `Box` or heap
closure — and runs only over the rects that actually carry a gradient role, not
over the whole buffer. A theme without gradients never enters the painter at
all: the solid path is a plain blanking pass. That is the design reason the
effect is hard to measure; the numbers above are consistent with it, not a proof
of it.
