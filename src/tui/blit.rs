//! Cell-level buffer blitting, the shared primitive behind every slide (#35).
//!
//! A slide draws its layer at rest into a standalone [`Buffer`], then copies
//! those cells into the frame at an eased offset. Going through cells (instead
//! of shifting the `Rect` a renderer draws into) lets a layer travel past the
//! edge of the screen, which a `Rect` cannot express, and keeps every renderer
//! free of animation concerns.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

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
    fn snapshot_copies_the_area_verbatim() {
        let a = area();
        let src = ruler(a);
        let snap = snapshot(&src, a);
        for y in a.top()..a.bottom() {
            assert_eq!(row(&snap, a, y), row(&src, a, y));
        }
    }
}
