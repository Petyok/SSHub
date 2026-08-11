//! Static gradient sampling and ratatui buffer post-processing.
//!
//! Widgets always render with their solid fallback colour first; only a role
//! that actually resolved to a gradient runs one of the painters below over the
//! finished cells. That keeps every frame glyph and the whole solid-colour path
//! in ratatui's hands, and it means a theme without gradients costs nothing.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use crate::theme::model::{GradientDirection, ResolvedGradient, ResolvedGradientStop};

/// Which channel of a cell a painter writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaintChannel {
    Foreground,
    Background,
}

/// Which cells of the target rect a painter is allowed to touch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellSelection {
    /// Every cell in the rect.
    All,
    /// Only cells whose current value on the painted channel is this colour.
    ///
    /// `Matching(Color::Reset)` is how a background pass repaints just the
    /// still-unpainted canvas without overwriting what a widget already drew.
    Matching(Color),
}

/// Resolves a relative position into a colour without touching the heap.
///
/// Borrows the resolved stop slice, so a caller can build one sampler per role
/// and reuse it across a whole rect.
pub struct GradientSampler<'a> {
    stops: &'a [ResolvedGradientStop],
}

impl<'a> GradientSampler<'a> {
    pub fn new(gradient: &'a ResolvedGradient) -> Self {
        Self {
            stops: &gradient.stops,
        }
    }

    /// Colour at relative position `t`, clamped to `0.0..=1.0`.
    pub fn sample(&self, t: f64) -> Color {
        sample_stops(self.stops, t.clamp(0.0, 1.0))
    }
}

/// Colour of `stops` at an already clamped `t`.
///
/// One linear walk over a slice the resolver capped at 32 entries: no cache, no
/// binary search, no allocation. The resolver also guarantees ascending,
/// `0.0`/`1.0`-anchored positions and opaque RGB stops, so none of that is
/// re-checked here.
pub(crate) fn sample_stops(stops: &[ResolvedGradientStop], t: f64) -> Color {
    let Some(first) = stops.first() else {
        return Color::Reset;
    };
    let mut lower = first;
    for stop in stops {
        if stop.position <= t {
            lower = stop;
        } else {
            let span = stop.position - lower.position;
            let local = if span <= f64::EPSILON {
                0.0
            } else {
                (t - lower.position) / span
            };
            return mix_srgb(lower.color, stop.color, local);
        }
    }
    lower.color
}

/// Linear per-channel sRGB mix; non-RGB endpoints cannot be interpolated and
/// snap to the nearer end instead of silently producing black.
///
/// The `clamp` + `round` per channel is deliberately the resolver's convention
/// (`resolve.rs`), so a gradient endpoint renders as the exact same byte triple
/// as the solid colour of the same value.
fn mix_srgb(from: Color, to: Color, t: f64) -> Color {
    let (Color::Rgb(r0, g0, b0), Color::Rgb(r1, g1, b1)) = (from, to) else {
        return if t < 0.5 { from } else { to };
    };
    let channel = |a: u8, b: u8| -> u8 {
        let value = a as f64 + (b as f64 - a as f64) * t;
        value.clamp(0.0, 255.0).round() as u8
    };
    Color::Rgb(channel(r0, r1), channel(g0, g1), channel(b0, b1))
}

/// Relative sample position of cell `(x, y)` within `area`.
///
/// `None` means the cell has no position on this gradient at all: `area` is
/// empty, the cell lies outside it, or the direction is `perimeter` and the
/// cell is an interior cell rather than part of the ring. Callers skip those
/// cells instead of painting them with a substitute colour.
pub(crate) fn gradient_position(
    direction: GradientDirection,
    area: Rect,
    x: u16,
    y: u16,
) -> Option<f64> {
    if !contains(area, x, y) {
        return None;
    }
    let norm = |value: u16, origin: u16, len: u16| -> f64 {
        if len <= 1 {
            0.0
        } else {
            (value - origin) as f64 / (len - 1) as f64
        }
    };
    let hx = norm(x, area.x, area.width);
    let vy = norm(y, area.y, area.height);
    // A rect one cell high has no vertical axis to spread a diagonal over, and
    // one cell wide has no horizontal one; the spec degrades both to the full
    // colour range along the axis that does exist.
    let flat_x = area.height <= 1;
    let flat_y = area.width <= 1;

    Some(match direction {
        GradientDirection::Horizontal => hx,
        GradientDirection::Vertical => vy,
        GradientDirection::DiagonalDown => match (flat_x, flat_y) {
            (true, true) => 0.0,
            (true, false) => hx,
            (false, true) => vy,
            (false, false) => (hx + vy) / 2.0,
        },
        GradientDirection::DiagonalUp => match (flat_x, flat_y) {
            (true, true) => 0.0,
            (true, false) => hx,
            (false, true) => 1.0 - vy,
            (false, false) => (hx + (1.0 - vy)) / 2.0,
        },
        GradientDirection::Perimeter => return perimeter_position(area, x, y),
    })
}

/// Position along the clockwise outer ring of `area`, starting at its top-left
/// corner; `None` for interior cells. Degenerate rects fall back to their
/// natural single-line direction.
fn perimeter_position(area: Rect, x: u16, y: u16) -> Option<f64> {
    if area.width <= 1 && area.height <= 1 {
        return Some(0.0);
    }
    if area.height <= 1 {
        return Some((x - area.x) as f64 / (area.width - 1) as f64);
    }
    if area.width <= 1 {
        return Some((y - area.y) as f64 / (area.height - 1) as f64);
    }

    let w = area.width as u32;
    let h = area.height as u32;
    let dx = (x - area.x) as u32;
    let dy = (y - area.y) as u32;
    let last_x = w - 1;
    let last_y = h - 1;

    // Branch order is the spec's walk: the top row owns both upper corners, the
    // right column owns the bottom-right one, the bottom row the bottom-left —
    // so every corner is visited exactly once.
    let index = if dy == 0 {
        dx
    } else if dx == last_x {
        last_x + dy
    } else if dy == last_y {
        last_x + last_y + (last_x - dx)
    } else if dx == 0 {
        2 * last_x + last_y + (last_y - dy)
    } else {
        return None;
    };
    let length = 2 * w + 2 * h - 4;
    Some(index as f64 / (length - 1) as f64)
}

/// Paints the outer cell ring of an already rendered block.
///
/// Foreground only, and the glyphs stay exactly where ratatui put them — this
/// runs *after* the block drew its borders with the solid fallback colour.
///
/// The ring is a **geometric** restriction: cells are still sampled with the
/// gradient's own direction. `perimeter` is the direction a closed-frame role
/// normally carries and the only one that walks the ring, so with any other
/// direction the frame simply shows that direction's colours where the ring
/// crosses them — a `horizontal` gradient, for instance, paints each vertical
/// edge as a flat block of the left/right colour.
pub fn paint_gradient_ring(buf: &mut Buffer, area: Rect, gradient: &ResolvedGradient) {
    paint_gradient_ring_selective(buf, area, gradient, CellSelection::All);
}

/// The same ring, restricted to the cells `selection` admits.
///
/// `Matching(border_colour)` is what lets a caller gradient a block's frame
/// without touching the title, label or badge sitting on the same top row:
/// those were written in a different colour, so they simply do not match.
pub fn paint_gradient_ring_selective(
    buf: &mut Buffer,
    area: Rect,
    gradient: &ResolvedGradient,
    selection: CellSelection,
) {
    paint(
        buf,
        area,
        gradient,
        PaintChannel::Foreground,
        selection,
        &[],
        true,
    );
}

/// Paints a separator, title or footer segment.
pub fn paint_gradient_line(
    buf: &mut Buffer,
    area: Rect,
    gradient: &ResolvedGradient,
    channel: PaintChannel,
    selection: CellSelection,
) {
    paint(buf, area, gradient, channel, selection, &[], false);
}

/// Paints a filled region, leaving every cell inside `exclusions` untouched.
///
/// `exclusions` is what keeps a background pass off the remote PTY viewport:
/// those cells carry the host's own ANSI colours and must never be recoloured
/// by a theme.
pub fn paint_gradient_area(
    buf: &mut Buffer,
    area: Rect,
    gradient: &ResolvedGradient,
    channel: PaintChannel,
    selection: CellSelection,
    exclusions: &[Rect],
) {
    paint(buf, area, gradient, channel, selection, exclusions, false);
}

fn paint(
    buf: &mut Buffer,
    area: Rect,
    gradient: &ResolvedGradient,
    channel: PaintChannel,
    selection: CellSelection,
    exclusions: &[Rect],
    ring_only: bool,
) {
    // Iterate the part of the rect that exists in the buffer, but keep sampling
    // against the full rect so clipping shifts no colour.
    let target = area.intersection(buf.area);
    if target.is_empty() {
        return;
    }
    let sampler = GradientSampler::new(gradient);

    for y in target.y..target.bottom() {
        for x in target.x..target.right() {
            if ring_only && !on_ring(area, x, y) {
                continue;
            }
            if exclusions.iter().any(|rect| contains(*rect, x, y)) {
                continue;
            }
            let Some(position) = gradient_position(gradient.direction, area, x, y) else {
                continue;
            };
            let Some(cell) = buf.cell_mut((x, y)) else {
                continue;
            };
            let current = match channel {
                PaintChannel::Foreground => cell.fg,
                PaintChannel::Background => cell.bg,
            };
            if let CellSelection::Matching(wanted) = selection {
                if current != wanted {
                    continue;
                }
            }
            let color = sampler.sample(position);
            match channel {
                PaintChannel::Foreground => cell.fg = color,
                PaintChannel::Background => cell.bg = color,
            }
        }
    }
}

fn contains(area: Rect, x: u16, y: u16) -> bool {
    !area.is_empty() && x >= area.x && x < area.right() && y >= area.y && y < area.bottom()
}

/// Whether `(x, y)` is on the outer cell ring of `area`. A rect one cell wide
/// or high is its own ring.
fn on_ring(area: Rect, x: u16, y: u16) -> bool {
    contains(area, x, y)
        && (x == area.x || x + 1 == area.right() || y == area.y || y + 1 == area.bottom())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::model::GradientDirection::{
        DiagonalDown, DiagonalUp, Horizontal, Perimeter, Vertical,
    };
    use crate::theme::model::{GradientDirection, ResolvedGradient, ResolvedGradientStop};
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Color;

    fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color::Rgb(r, g, b)
    }

    fn directed<const N: usize>(
        direction: GradientDirection,
        stops: [(f64, Color); N],
    ) -> ResolvedGradient {
        ResolvedGradient {
            direction,
            stops: stops
                .into_iter()
                .map(|(position, color)| ResolvedGradientStop { position, color })
                .collect(),
        }
    }

    fn gradient<const N: usize>(stops: [(f64, Color); N]) -> ResolvedGradient {
        directed(Horizontal, stops)
    }

    /// Black to white, so a sampled channel doubles as the position in eighths.
    fn ramp(direction: GradientDirection) -> ResolvedGradient {
        directed(direction, [(0.0, rgb(0, 0, 0)), (1.0, rgb(255, 255, 255))])
    }

    #[test]
    fn sampler_interpolates_multiple_stops_with_v1_rounding() {
        let gradient = gradient([
            (0.0, rgb(0, 0, 0)),
            (0.25, rgb(100, 50, 0)),
            (1.0, rgb(255, 255, 255)),
        ]);
        let sampler = GradientSampler::new(&gradient);
        assert_eq!(sampler.sample(0.0), rgb(0, 0, 0));
        assert_eq!(sampler.sample(0.25), rgb(100, 50, 0));
        assert_eq!(sampler.sample(1.0), rgb(255, 255, 255));
    }

    #[test]
    fn sampler_rounds_half_way_channels_like_the_resolver() {
        // 0 -> 255 at the midpoint is 127.5, which the resolver's `round()`
        // convention lifts to 128 rather than truncating to 127.
        let ramp = ramp(Horizontal);
        let sampler = GradientSampler::new(&ramp);
        assert_eq!(sampler.sample(0.5), rgb(128, 128, 128));
    }

    #[test]
    fn sampler_clamps_positions_outside_the_unit_range() {
        let ramp = ramp(Horizontal);
        let sampler = GradientSampler::new(&ramp);
        assert_eq!(sampler.sample(-4.0), rgb(0, 0, 0));
        assert_eq!(sampler.sample(9.0), rgb(255, 255, 255));
        assert_eq!(sampler.sample(f64::NEG_INFINITY), rgb(0, 0, 0));
        assert_eq!(sampler.sample(f64::INFINITY), rgb(255, 255, 255));
    }

    #[test]
    fn horizontal_and_vertical_positions_are_relative_to_the_component_rect() {
        let area = Rect::new(4, 7, 5, 3);
        assert_eq!(gradient_position(Horizontal, area, 4, 7), Some(0.0));
        assert_eq!(gradient_position(Horizontal, area, 6, 9), Some(0.5));
        assert_eq!(gradient_position(Horizontal, area, 8, 7), Some(1.0));
        assert_eq!(gradient_position(Vertical, area, 4, 7), Some(0.0));
        assert_eq!(gradient_position(Vertical, area, 8, 8), Some(0.5));
        assert_eq!(gradient_position(Vertical, area, 4, 9), Some(1.0));
    }

    #[test]
    fn diagonals_average_both_axes_on_a_two_dimensional_rect() {
        let area = Rect::new(0, 0, 3, 3);
        assert_eq!(gradient_position(DiagonalDown, area, 0, 0), Some(0.0));
        assert_eq!(gradient_position(DiagonalDown, area, 1, 1), Some(0.5));
        assert_eq!(gradient_position(DiagonalDown, area, 2, 2), Some(1.0));
        assert_eq!(gradient_position(DiagonalUp, area, 0, 2), Some(0.0));
        assert_eq!(gradient_position(DiagonalUp, area, 1, 1), Some(0.5));
        assert_eq!(gradient_position(DiagonalUp, area, 2, 0), Some(1.0));
    }

    #[test]
    fn degenerate_diagonals_use_the_complete_color_range() {
        let horizontal = Rect::new(0, 0, 5, 1);
        assert_eq!(gradient_position(DiagonalDown, horizontal, 0, 0), Some(0.0));
        assert_eq!(gradient_position(DiagonalDown, horizontal, 4, 0), Some(1.0));
        let vertical = Rect::new(0, 0, 1, 5);
        assert_eq!(gradient_position(DiagonalUp, vertical, 0, 0), Some(1.0));
        assert_eq!(gradient_position(DiagonalUp, vertical, 0, 4), Some(0.0));
    }

    #[test]
    fn degenerate_rects_never_divide_by_zero() {
        for direction in [Horizontal, Vertical, DiagonalDown, DiagonalUp, Perimeter] {
            // 1x1 pins every direction at the start of the scale.
            assert_eq!(
                gradient_position(direction, Rect::new(3, 3, 1, 1), 3, 3),
                Some(0.0),
                "{direction:?} on 1x1"
            );
            // 0xN and Nx0 have no cells at all.
            for empty in [Rect::new(0, 0, 0, 4), Rect::new(0, 0, 4, 0)] {
                assert_eq!(
                    gradient_position(direction, empty, 0, 0),
                    None,
                    "{direction:?} on {empty:?}"
                );
            }
        }
    }

    #[test]
    fn single_row_and_single_column_rects_span_their_natural_axis() {
        let row = Rect::new(0, 0, 4, 1);
        assert_eq!(gradient_position(Horizontal, row, 3, 0), Some(1.0));
        assert_eq!(gradient_position(Vertical, row, 3, 0), Some(0.0));
        assert_eq!(gradient_position(Perimeter, row, 0, 0), Some(0.0));
        assert_eq!(gradient_position(Perimeter, row, 3, 0), Some(1.0));

        let column = Rect::new(0, 0, 1, 4);
        assert_eq!(gradient_position(Vertical, column, 0, 3), Some(1.0));
        assert_eq!(gradient_position(Horizontal, column, 0, 3), Some(0.0));
        assert_eq!(gradient_position(Perimeter, column, 0, 0), Some(0.0));
        assert_eq!(gradient_position(Perimeter, column, 0, 3), Some(1.0));
    }

    #[test]
    fn positions_outside_the_rect_have_no_sample_position() {
        let area = Rect::new(2, 2, 3, 3);
        for (x, y) in [(1, 2), (5, 2), (2, 1), (2, 5)] {
            assert_eq!(gradient_position(Horizontal, area, x, y), None, "{x},{y}");
        }
    }

    /// The spec's clockwise ring order for a 4x3 rect, top-left first.
    fn ring_order_4x3() -> Vec<(u16, u16)> {
        vec![
            (0, 0),
            (1, 0),
            (2, 0),
            (3, 0), // top row, both corners
            (3, 1),
            (3, 2), // right column down to the bottom-right corner
            (2, 2),
            (1, 2),
            (0, 2), // bottom row leftwards to the bottom-left corner
            (0, 1), // left column upwards, both corners already counted
        ]
    }

    #[test]
    fn perimeter_walks_clockwise_and_counts_each_corner_once() {
        let area = Rect::new(0, 0, 4, 3);
        let path = ring_order_4x3();
        let length = 2 * 4 + 2 * 3 - 4;
        assert_eq!(path.len(), length);

        let mut seen = Vec::new();
        for (index, (x, y)) in path.iter().enumerate() {
            let position = gradient_position(Perimeter, area, *x, *y).expect("ring cell");
            let expected = index as f64 / (length - 1) as f64;
            assert!(
                (position - expected).abs() < 1e-6,
                "cell {x},{y} at index {index}: {position} != {expected}"
            );
            assert!(!seen.contains(&position), "duplicate position at {x},{y}");
            seen.push(position);
        }
    }

    #[test]
    fn perimeter_interior_cells_are_not_on_the_ring() {
        let area = Rect::new(0, 0, 4, 3);
        assert_eq!(gradient_position(Perimeter, area, 1, 1), None);
        assert_eq!(gradient_position(Perimeter, area, 2, 1), None);
    }

    #[test]
    fn perimeter_closes_without_a_visible_seam() {
        // First and last stop resolve to the same colour (a resolver invariant),
        // so the cell before the top-left corner matches the corner itself.
        let ring = directed(
            Perimeter,
            [
                (0.0, rgb(10, 20, 30)),
                (0.5, rgb(200, 100, 0)),
                (1.0, rgb(10, 20, 30)),
            ],
        );
        let sampler = GradientSampler::new(&ring);
        let area = Rect::new(0, 0, 4, 3);
        let start = gradient_position(Perimeter, area, 0, 0).unwrap();
        let end = gradient_position(Perimeter, area, 0, 1).unwrap();
        assert_eq!(start, 0.0);
        assert_eq!(end, 1.0);
        assert_eq!(sampler.sample(start), sampler.sample(end));
    }

    #[test]
    fn ring_painter_recolors_only_the_outer_ring_and_keeps_glyphs() {
        let area = Rect::new(0, 0, 4, 3);
        let mut buffer = Buffer::empty(area);
        for (x, y) in [(0, 0), (1, 1)] {
            buffer[(x, y)].set_symbol("┌");
        }
        paint_gradient_ring(&mut buffer, area, &ramp(Perimeter));

        assert_eq!(buffer[(0, 0)].fg, rgb(0, 0, 0));
        assert_eq!(buffer[(0, 0)].symbol(), "┌");
        assert_eq!(buffer[(0, 1)].fg, rgb(255, 255, 255));
        // Interior cells keep the default foreground and their glyph.
        assert_eq!(buffer[(1, 1)].fg, Color::Reset);
        assert_eq!(buffer[(1, 1)].symbol(), "┌");
        assert_eq!(buffer[(2, 1)].fg, Color::Reset);
        // Backgrounds are never touched by the ring pass.
        assert_eq!(buffer[(0, 0)].bg, Color::Reset);
    }

    #[test]
    fn ring_painter_restricts_geometrically_and_keeps_the_gradients_direction() {
        // A non-perimeter direction is legal and documented: the ring shows that
        // direction's colours where it crosses them, so a horizontal gradient
        // leaves each vertical edge a flat block.
        let area = Rect::new(0, 0, 4, 3);
        let mut buffer = Buffer::empty(area);
        paint_gradient_ring(&mut buffer, area, &ramp(Horizontal));
        for y in 0..3 {
            assert_eq!(buffer[(0, y)].fg, rgb(0, 0, 0), "left edge at {y}");
            assert_eq!(buffer[(3, y)].fg, rgb(255, 255, 255), "right edge at {y}");
        }
        assert_eq!(buffer[(1, 1)].fg, Color::Reset, "interior stays untouched");
    }

    #[test]
    fn ring_painter_treats_flat_rects_as_a_single_line() {
        let area = Rect::new(0, 0, 4, 1);
        let mut buffer = Buffer::empty(area);
        paint_gradient_ring(&mut buffer, area, &ramp(Perimeter));
        assert_eq!(buffer[(0, 0)].fg, rgb(0, 0, 0));
        assert_eq!(buffer[(3, 0)].fg, rgb(255, 255, 255));
    }

    #[test]
    fn line_painter_writes_the_requested_channel_only() {
        let area = Rect::new(0, 0, 5, 1);
        let mut buffer = Buffer::empty(area);
        paint_gradient_line(
            &mut buffer,
            area,
            &ramp(Horizontal),
            PaintChannel::Foreground,
            CellSelection::All,
        );
        assert_eq!(buffer[(0, 0)].fg, rgb(0, 0, 0));
        assert_eq!(buffer[(4, 0)].fg, rgb(255, 255, 255));
        assert_eq!(buffer[(0, 0)].bg, Color::Reset);

        let mut buffer = Buffer::empty(area);
        paint_gradient_line(
            &mut buffer,
            area,
            &ramp(Horizontal),
            PaintChannel::Background,
            CellSelection::All,
        );
        assert_eq!(buffer[(0, 0)].bg, rgb(0, 0, 0));
        assert_eq!(buffer[(4, 0)].bg, rgb(255, 255, 255));
        assert_eq!(buffer[(0, 0)].fg, Color::Reset);
    }

    #[test]
    fn line_painter_can_restrict_itself_to_one_solid_color() {
        let area = Rect::new(0, 0, 4, 1);
        let mut buffer = Buffer::empty(area);
        buffer[(0, 0)].set_fg(rgb(9, 9, 9));
        buffer[(1, 0)].set_fg(rgb(7, 7, 7));
        paint_gradient_line(
            &mut buffer,
            area,
            &ramp(Horizontal),
            PaintChannel::Foreground,
            CellSelection::Matching(rgb(7, 7, 7)),
        );
        assert_eq!(buffer[(0, 0)].fg, rgb(9, 9, 9));
        assert_ne!(buffer[(1, 0)].fg, rgb(7, 7, 7));
        assert_eq!(buffer[(2, 0)].fg, Color::Reset);
    }

    #[test]
    fn area_painter_can_limit_writes_to_reset_backgrounds_and_exclude_pty() {
        let gradient = ramp(Horizontal);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 8, 3));
        buffer[(1, 1)].set_bg(rgb(1, 2, 3));
        paint_gradient_area(
            &mut buffer,
            Rect::new(0, 0, 8, 3),
            &gradient,
            PaintChannel::Background,
            CellSelection::Matching(Color::Reset),
            &[Rect::new(4, 0, 4, 3)],
        );
        assert_eq!(buffer[(1, 1)].bg, rgb(1, 2, 3));
        assert_eq!(buffer[(5, 1)].bg, Color::Reset);
        assert_ne!(buffer[(2, 1)].bg, Color::Reset);
    }

    #[test]
    fn area_painter_honours_multiple_and_partly_overlapping_exclusions() {
        let area = Rect::new(0, 0, 6, 2);
        let mut buffer = Buffer::empty(area);
        paint_gradient_area(
            &mut buffer,
            area,
            &ramp(Horizontal),
            PaintChannel::Background,
            CellSelection::All,
            &[
                Rect::new(1, 0, 2, 1),
                Rect::new(2, 0, 2, 2),
                // Empty and out-of-buffer exclusions must not hide anything.
                Rect::new(0, 0, 0, 2),
                Rect::new(40, 40, 3, 3),
            ],
        );
        for (x, y) in [(1, 0), (2, 0), (3, 0), (2, 1), (3, 1)] {
            assert_eq!(buffer[(x, y)].bg, Color::Reset, "excluded {x},{y}");
        }
        for (x, y) in [(0, 0), (4, 0), (5, 0), (0, 1), (1, 1)] {
            assert_ne!(buffer[(x, y)].bg, Color::Reset, "painted {x},{y}");
        }
    }

    #[test]
    fn area_painter_samples_relative_to_the_area_not_the_clipped_region() {
        // The right half of the area hangs outside the buffer; the cells that do
        // exist must still carry the colours of their position in the full area.
        let mut buffer = Buffer::empty(Rect::new(0, 0, 3, 1));
        paint_gradient_area(
            &mut buffer,
            Rect::new(0, 0, 5, 1),
            &ramp(Horizontal),
            PaintChannel::Background,
            CellSelection::All,
            &[],
        );
        assert_eq!(buffer[(0, 0)].bg, rgb(0, 0, 0));
        assert_eq!(buffer[(2, 0)].bg, rgb(128, 128, 128));
    }

    #[test]
    fn painters_ignore_empty_and_out_of_bounds_areas() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 3, 2));
        let gradient = ramp(Horizontal);
        for area in [
            Rect::new(0, 0, 0, 2),
            Rect::new(0, 0, 3, 0),
            Rect::new(20, 20, 4, 4),
        ] {
            paint_gradient_ring(&mut buffer, area, &gradient);
            paint_gradient_line(
                &mut buffer,
                area,
                &gradient,
                PaintChannel::Background,
                CellSelection::All,
            );
            paint_gradient_area(
                &mut buffer,
                area,
                &gradient,
                PaintChannel::Foreground,
                CellSelection::All,
                &[],
            );
        }
        assert_eq!(buffer, Buffer::empty(Rect::new(0, 0, 3, 2)));
    }

    // -----------------------------------------------------------------------
    // Structural proof: sampling and painting allocate nothing.
    // -----------------------------------------------------------------------

    use crate::test_alloc::allocations_during;

    #[test]
    fn sampling_and_painting_allocate_nothing_per_cell() {
        let gradient = directed(
            Perimeter,
            [
                (0.0, rgb(10, 20, 30)),
                (0.4, rgb(80, 0, 200)),
                (1.0, rgb(10, 20, 30)),
            ],
        );
        let area = Rect::new(0, 0, 60, 20);
        let mut buffer = Buffer::empty(area);
        let exclusions = [Rect::new(10, 5, 20, 8)];
        let sampler = GradientSampler::new(&gradient);

        // The counter itself must be honest: a deliberate allocation is seen.
        assert!(
            allocations_during(|| {
                std::hint::black_box(vec![0u8; 64]);
            }) > 0
        );

        let allocations = allocations_during(|| {
            for step in 0..=1000 {
                std::hint::black_box(sampler.sample(step as f64 / 1000.0));
            }
            paint_gradient_ring(&mut buffer, area, &gradient);
            paint_gradient_line(
                &mut buffer,
                Rect::new(0, 0, 60, 1),
                &gradient,
                PaintChannel::Foreground,
                CellSelection::Matching(Color::Reset),
            );
            paint_gradient_area(
                &mut buffer,
                area,
                &gradient,
                PaintChannel::Background,
                CellSelection::Matching(Color::Reset),
                &exclusions,
            );
        });
        assert_eq!(allocations, 0);
    }
}
