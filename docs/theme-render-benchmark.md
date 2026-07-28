# Theme render benchmark

The runtime theme system paints gradients by post-processing the ratatui buffer
after a frame has been drawn. The design constraint was that this must cost
**less than 2 ms of additional median render time** per frame compared with a
theme that has no gradients at all.

This document is the evidence for that constraint. Two different things live in
it, and it matters which is which.

**The bound comes from the gradient theme's own frame time.** A whole `fire`
frame at `200x60` takes about `0.25 ms` at the median, and the gradient pass
runs serially inside that frame — so it cannot possibly cost more than the whole
frame does. That is a conservative upper bound on the gradient work, it holds
without any comparison at all, and it sits roughly an order of magnitude below
`2 ms` even before you subtract everything else the frame is doing.

**The printed delta bounds nothing.** It is a smoke observation, and it is worth
being blunt about why it cannot be read as a cost:

- `high-contrast` and `fire` differ in every value they set, not only in
  gradients, so the difference mixes in everything else the two themes do
  differently.
- They are always measured in the same order, solid first, so any drift over the
  run lands in the difference too.
- Faster work elsewhere in `fire` can offset gradient work *inside* the
  difference, hiding it rather than showing it.
- The benchmark computes the delta with `saturating_sub`, so a negative
  observation is reported as `0.000 ms` rather than as what it was. That has
  happened: see the reviewer run below, where the gradient side measured
  *faster* than the solid one.

It is also **not** a CI gate: the benchmark asserts nothing about timing and is
`#[ignore]`d, so it only runs when someone asks for it. A shared runner's
scheduling noise is larger than the effect being measured, so a timing assertion
there would fail for reasons that have nothing to do with SSHub.

Isolating what the pass actually costs would mean measuring one app state twice,
once with gradient paints and once with equivalent solid paints, alternating the
order between runs. That has not been done, and no claim here depends on it.

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
| gradient | `fire` | Opaque app background, 3 gradient definitions (`blaze`, `updraft`, `cinder`) referenced by 5 roles: `components.dashboard.host_list.border_focused`, `components.dashboard.details.border_focused`, `components.dashboard.latency.border_focused`, `components.separator.primary` and `components.header.separator`. No background gradient. |

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

Two independent reviewer runs on different hardware produced solid `0.224 ms` /
gradient `0.218 ms` and solid `0.262 ms` / gradient `0.254 ms` — in both, the
gradient side measured *faster* than the solid one, and the reported delta was
`0.000 ms` only because `saturating_sub` clamps it. That is the clearest
possible demonstration that the delta is not measuring the gradient pass.

**Result.** The claim this measurement supports is the one that needs no
comparison: a whole gradient frame at `200x60` has a median of `0.204 – 0.254 ms`
across every run recorded here, local and reviewer alike. The gradient pass runs
inside that frame, so its cost is bounded by it — roughly an order of magnitude
under the `2 ms` criterion, and that is before subtracting everything else the
frame does. The deltas are recorded for completeness and support no claim: they
range from a clamped zero to `0.023 ms`, which is the size of the run-to-run
noise, not the size of an effect.

## Why a frame this cheap is plausible

Gradient sampling allocates nothing per cell — no `Vec`, `String`, `Box` or heap
closure — and runs only over the rects that actually carry a gradient role, not
over the whole buffer. A theme without gradients never enters the painter at
all: the solid path is a plain blanking pass. That is the design reason the
effect is too small to separate from the noise; the numbers above are consistent
with it, not a proof of it.
