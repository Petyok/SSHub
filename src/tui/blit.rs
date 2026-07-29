//! Cell-level buffer effects: the shared primitives behind the slides and
//! fades (#35).
//!
//! A slide draws its layer at rest into a standalone [`Buffer`], then copies
//! those cells into the frame at an eased offset. Going through cells (instead
//! of shifting the `Rect` a renderer draws into) lets a layer travel past the
//! edge of the screen, which a `Rect` cannot express, and keeps every renderer
//! free of animation concerns.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use crate::tui::theme;
use crate::tui::tween::color_lerp;

/// Copy the `region` cells of `src` into `dst` offset by (`dx`, `dy`), dropping
/// whatever lands outside `clip`.
///
/// `src` must be a standalone buffer (never `dst` itself), so the copy order
/// doesn't matter and a cell can't be read after it was overwritten.
pub fn blit(dst: &mut Buffer, region: Rect, clip: Rect, src: &Buffer, dx: i32, dy: i32) {
    for y in region.top()..region.bottom() {
        let ty = y as i32 + dy;
        if ty < clip.top() as i32 || ty >= clip.bottom() as i32 {
            continue;
        }
        for x in region.left()..region.right() {
            let tx = x as i32 + dx;
            if tx < clip.left() as i32 || tx >= clip.right() as i32 {
                continue;
            }
            if let (Some(s), Some(d)) = (src.cell((x, y)), dst.cell_mut((tx as u16, ty as u16))) {
                *d = s.clone();
            }
        }
    }
}

/// Clone the `area` cells of `src` into a standalone buffer keeping the same
/// absolute coordinates, so a later frame can blit them while sliding.
pub fn snapshot(src: &Buffer, area: Rect) -> Buffer {
    let mut out = Buffer::empty(area);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let (Some(s), Some(d)) = (src.cell((x, y)), out.cell_mut((x, y))) {
                *d = s.clone();
            }
        }
    }
    out
}

/// Fade the cells of `area` up out of the background, `k` of the way in
/// (`k >= 1.0` is fully drawn, `0.0` invisible).
///
/// Used where a panel's *content* changes under a fixed frame -- a different
/// host's detail, a re-filtered table -- and sliding it would be a lie about
/// where it came from, but swapping it outright reads as a flicker.
pub fn fade(buf: &mut Buffer, area: Rect, k: f32) {
    if k >= 1.0 {
        return;
    }
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let Some(c) = buf.cell_mut((x, y)) else {
                continue;
            };
            c.fg = color_lerp(theme::BG, c.fg, k);
            if c.bg != Color::Reset {
                c.bg = color_lerp(theme::BG, c.bg, k);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> Rect {
        Rect::new(2, 1, 8, 3)
    }

    /// A source buffer whose every cell carries its own column as a symbol, so
    /// an offset is readable straight off the destination row.
    fn ruler(area: Rect) -> Buffer {
        let mut b = Buffer::empty(area);
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                b.cell_mut((x, y))
                    .unwrap()
                    .set_symbol(&format!("{}", x % 10));
            }
        }
        b
    }

    fn row(buf: &Buffer, area: Rect, y: u16) -> String {
        (area.left()..area.right())
            .map(|x| buf.cell((x, y)).unwrap().symbol().to_string())
            .collect()
    }

    #[test]
    fn blit_moves_cells_horizontally_and_clips_both_edges() {
        let a = area();
        let src = ruler(a);
        let mut dst = Buffer::empty(a);
        blit(&mut dst, a, a, &src, 3, 0);
        // The three leftmost columns were vacated (nothing slid in from off
        // screen) and the three rightmost source columns fell off the clip.
        assert_eq!(row(&dst, a, 1), "   23456");
    }

    #[test]
    fn blit_moves_cells_vertically() {
        let a = area();
        let src = ruler(a);
        let mut dst = Buffer::empty(a);
        blit(&mut dst, a, a, &src, 0, 1);
        // Row 1 is vacated, rows 2..4 carry what rows 1..3 held.
        assert_eq!(row(&dst, a, 1), " ".repeat(a.width as usize));
        assert_eq!(row(&dst, a, 2), row(&src, a, 1));
    }

    #[test]
    fn blit_with_no_offset_is_a_plain_copy() {
        let a = area();
        let src = ruler(a);
        let mut dst = Buffer::empty(a);
        blit(&mut dst, a, a, &src, 0, 0);
        assert_eq!(row(&dst, a, 1), row(&src, a, 1));
    }

    #[test]
    fn blit_past_the_clip_writes_nothing() {
        let a = area();
        let src = ruler(a);
        let mut dst = Buffer::empty(a);
        blit(&mut dst, a, a, &src, a.width as i32, 0);
        assert_eq!(row(&dst, a, 1), " ".repeat(a.width as usize));
    }

    #[test]
    fn fade_at_rest_leaves_the_cells_alone() {
        let a = area();
        let mut buf = Buffer::empty(a);
        buf.cell_mut((2, 1)).unwrap().fg = theme::GREEN;
        fade(&mut buf, a, 1.0);
        assert_eq!(buf.cell((2, 1)).unwrap().fg, theme::GREEN);
    }

    #[test]
    fn fade_pulls_colours_toward_the_background() {
        let a = area();
        let mut buf = Buffer::empty(a);
        buf.cell_mut((2, 1)).unwrap().fg = theme::GREEN;
        buf.cell_mut((2, 1)).unwrap().bg = theme::SEL_BG;
        fade(&mut buf, a, 0.0);
        assert_eq!(buf.cell((2, 1)).unwrap().fg, theme::BG);
        assert_eq!(buf.cell((2, 1)).unwrap().bg, theme::BG);
        // A transparent background stays transparent rather than being painted.
        assert_eq!(buf.cell((3, 1)).unwrap().bg, Color::Reset);
    }

    #[test]
    fn snapshot_copies_the_area_verbatim() {
        let a = area();
        let src = ruler(a);
        let snap = snapshot(&src, a);
        for y in a.top()..a.bottom() {
            assert_eq!(row(&snap, a, y), row(&src, a, y));
        }
    }
}
