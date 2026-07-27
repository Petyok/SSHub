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
use ratatui::style::{Color, Style};

use crate::theme::catalog::PaintRole;
use crate::theme::gradient::{paint_gradient_area, CellSelection, PaintChannel};
use crate::theme::model::{ResolvedPaint, ResolvedTheme};
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
/// `background` names the role the cells fade *out of*; a gradient role is
/// sampled per coordinate rather than flattened to one colour.
pub fn fade(buf: &mut Buffer, area: Rect, k: f32, theme: &ResolvedTheme, background: PaintRole) {
    if k >= 1.0 {
        return;
    }
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let base = fade_ground(theme, background, area, x, y);
            let Some(c) = buf.cell_mut((x, y)) else {
                continue;
            };
            c.fg = color_lerp(base, c.fg, k);
            if c.bg != Color::Reset {
                c.bg = color_lerp(base, c.bg, k);
            }
        }
    }
}

/// The opaque colour a fade blends towards at one cell.
///
/// A role that resolved to `"terminal"` has no colour to blend with —
/// `Color::Reset` is not a value `color_lerp` can interpolate, so it would turn
/// a fade into a switch at the halfway mark. `semantic.canvas` is the opaque
/// companion the spec names for exactly this case, and for `default` it is the
/// literal `theme::BG` this used to hard-code.
fn fade_ground(theme: &ResolvedTheme, role: PaintRole, area: Rect, x: u16, y: u16) -> Color {
    match theme.paint_color_at(role, area, x, y) {
        Color::Reset => theme.semantic().canvas,
        color => color,
    }
}

/// Fill `area`'s background with a paint role.
///
/// Solid roles blank the rect with their colour; a gradient blanks first (the
/// painter recolours cells, it does not clear them) and is then sampled per
/// cell. Callers run this *before* drawing the content it sits behind.
///
/// Deliberately takes no exclusions: it is for SSHub's own chrome rects, never
/// for a region that could overlap the remote PTY viewport.
pub fn fill_paint(buf: &mut Buffer, area: Rect, theme: &ResolvedTheme, role: PaintRole) {
    match theme.paint(role) {
        ResolvedPaint::Solid(color) => blank(buf, area, Style::default().bg(*color)),
        ResolvedPaint::Gradient(_) => {
            blank(buf, area, Style::default());
            if let Some(gradient) = theme.paint_gradient(role) {
                paint_gradient_area(
                    buf,
                    area,
                    gradient,
                    PaintChannel::Background,
                    CellSelection::All,
                    &[],
                );
            }
        }
    }
}

/// The foreground colour a one-row line should be drawn in, plus the gradient
/// that has to run over it afterwards.
///
/// Splitting it this way keeps the widget's own `set_string` in charge of the
/// glyphs: it draws with the solid fallback, and only a gradient role costs a
/// second pass ([`paint_line`]).
pub fn line_color(theme: &ResolvedTheme, role: PaintRole, area: Rect) -> Color {
    theme.paint_color_at(role, area, area.x, area.y)
}

/// Run a paint role's gradient over an already drawn line's foreground.
///
/// A no-op for a solid role, so the cheap path stays cheap for the two
/// gradient-free built-ins.
pub fn paint_line(buf: &mut Buffer, area: Rect, theme: &ResolvedTheme, role: PaintRole) {
    if let Some(gradient) = theme.paint_gradient(role) {
        crate::theme::gradient::paint_gradient_line(
            buf,
            area,
            gradient,
            PaintChannel::Foreground,
            CellSelection::All,
        );
    }
}

/// Overwrite every cell of `area` with a space in `style`.
fn blank(buf: &mut Buffer, area: Rect, style: Style) {
    let target = area.intersection(buf.area);
    for y in target.y..target.bottom() {
        for x in target.x..target.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.reset();
                cell.set_style(style);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::model::{ThemeId, ValidationMode};
    use crate::theme::registry::ThemeRegistry;
    use crate::tui::theme;
    use std::rc::Rc;

    fn area() -> Rect {
        Rect::new(2, 1, 8, 3)
    }

    /// A resolved built-in, by id. Built-ins are embedded, so this touches no
    /// filesystem at all.
    fn resolved(id: &str) -> Rc<crate::theme::model::ResolvedTheme> {
        ThemeRegistry::builtins(ValidationMode::Strict)
            .unwrap()
            .resolved(&ThemeId::parse(id).unwrap())
            .unwrap()
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
        let theme = resolved("default");
        let mut buf = Buffer::empty(a);
        buf.cell_mut((2, 1)).unwrap().fg = theme::GREEN;
        fade(&mut buf, a, 1.0, &theme, PaintRole::AppBackground);
        assert_eq!(buf.cell((2, 1)).unwrap().fg, theme::GREEN);
    }

    #[test]
    fn fade_pulls_colours_toward_the_background() {
        let a = area();
        let theme = resolved("default");
        let mut buf = Buffer::empty(a);
        buf.cell_mut((2, 1)).unwrap().fg = theme::GREEN;
        buf.cell_mut((2, 1)).unwrap().bg = theme::SEL_BG;
        fade(&mut buf, a, 0.0, &theme, PaintRole::AppBackground);
        // `default` leaves the app background at "terminal", so the fade falls
        // back to the canvas — which is the `theme::BG` it used to hard-code.
        assert_eq!(buf.cell((2, 1)).unwrap().fg, theme::BG);
        assert_eq!(buf.cell((2, 1)).unwrap().bg, theme::BG);
        assert_eq!(theme.semantic().canvas, theme::BG);
        // A transparent background stays transparent rather than being painted.
        assert_eq!(buf.cell((3, 1)).unwrap().bg, Color::Reset);
    }

    #[test]
    fn fade_towards_a_gradient_samples_per_cell() {
        // A flattened gradient would give every column the same target colour,
        // which is exactly the bug `paint_color_at`-as-a-fill produces.
        let a = Rect::new(0, 0, 8, 1);
        let theme = gradient_background_theme();
        let mut buf = Buffer::empty(a);
        for x in a.left()..a.right() {
            buf.cell_mut((x, 0)).unwrap().fg = theme::GREEN;
        }
        fade(&mut buf, a, 0.0, &theme, PaintRole::AppBackground);
        let row: Vec<_> = (a.left()..a.right())
            .map(|x| buf.cell((x, 0)).unwrap().fg)
            .collect();
        assert!(
            row.windows(2).any(|pair| pair[0] != pair[1]),
            "the fade ground is flat: {row:?}"
        );
    }

    #[test]
    fn fill_paint_lays_down_a_solid_role_and_samples_a_gradient_one() {
        let a = Rect::new(0, 0, 8, 2);

        // Solid: every cell carries that one colour.
        let fire = resolved("fire");
        let mut buf = Buffer::empty(a);
        fill_paint(&mut buf, a, &fire, PaintRole::AppBackground);
        let expected = match fire.paint(PaintRole::AppBackground) {
            ResolvedPaint::Solid(color) => *color,
            other => panic!("fire's app background is solid, got {other:?}"),
        };
        assert!((a.left()..a.right()).all(|x| buf.cell((x, 0)).unwrap().bg == expected));

        // Gradient: the row sweeps instead of sitting on one colour.
        let washed = gradient_background_theme();
        let mut buf = Buffer::empty(a);
        fill_paint(&mut buf, a, &washed, PaintRole::AppBackground);
        let row: Vec<_> = (a.left()..a.right())
            .map(|x| buf.cell((x, 0)).unwrap().bg)
            .collect();
        assert!(
            row.windows(2).any(|pair| pair[0] != pair[1]),
            "the fill is flat: {row:?}"
        );
    }

    /// A theme whose app background is a black-to-white horizontal sweep. No
    /// built-in paints one, and a flattened sample is only visible against a
    /// gradient that actually varies.
    fn gradient_background_theme() -> Rc<crate::theme::model::ResolvedTheme> {
        let source = "schema_version = 1\nname = \"Washed\"\nextends = \"default\"\n\n\
             [gradients.wash]\ndirection = \"horizontal\"\n\
             stops = [ { at = 0.0, color = \"#000000\" }, { at = 1.0, color = \"#ffffff\" } ]\n\n\
             [components.app]\nbackground = { gradient = \"gradients.wash\" }\n";
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("washed.toml"), source).unwrap();
        let registry = ThemeRegistry::load_installed(dir.path(), ValidationMode::Strict).unwrap();
        registry
            .resolved(&ThemeId::parse("washed").unwrap())
            .expect("the gradient theme resolves")
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
