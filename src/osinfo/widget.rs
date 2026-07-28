//! Ratatui rendering for a resolved [`OsLogo`].
//!
//! Two render paths share the same colored-span data:
//!
//! * [`OsLogoWidget`] — a [`Widget`] that paints into a carved `Rect`,
//!   clamping every line to `area.width`/`area.height` so a logo wider or
//!   taller than its sub-column never spills over the border.
//! * [`logo_to_lines`] — composes the logo into owned [`Line`]s for embedding
//!   in the `Paragraph`-based detail panel (which has no `Rect` at build time).
//!
//! Pure rendering: no app state, no I/O.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::theme::model::ResolvedTint;

use super::logos::{span_style, OsLogo, OsLogoSpan};

/// Display width of a logo span, in terminal columns.
///
/// The vendored logos are ASCII/box-drawing art (one column per char), so —
/// matching the rest of the codebase's rendering helpers (see
/// `tui::text::ellipsize`) — char count is used as the column count.
fn span_width(span: &OsLogoSpan) -> usize {
    span.text.chars().count()
}

/// Rendered dimensions of a logo, as `(width, height)` in terminal cells.
///
/// `width` is the widest line (display columns); `height` is the line count.
/// Callers use this to carve a sub-column (e.g. `width + 1` for a gutter)
/// before handing the remaining `Rect` to [`OsLogoWidget`].
pub fn logo_dimensions(logo: &OsLogo) -> (u16, u16) {
    let width = logo
        .lines
        .iter()
        .map(|line| line.0.iter().map(span_width).sum::<usize>())
        .max()
        .unwrap_or(0)
        .min(u16::MAX as usize) as u16;
    let height = logo.lines.len().min(u16::MAX as usize) as u16;
    (width, height)
}

/// Widget that paints an [`OsLogo`] into a `Rect`, one logo line per row.
///
/// Both dimensions are clamped: rows beyond `area.height` are dropped, and
/// each line is truncated (by chars) at `area.width` so it never overflows
/// into neighbouring cells.
pub struct OsLogoWidget<'a> {
    pub logo: &'a OsLogo,
    /// How the active theme recolours the art. `Native` is the default and
    /// leaves every embedded RGB/ANSI value exactly as the asset stored it.
    pub tint: ResolvedTint,
}

impl<'a> OsLogoWidget<'a> {
    pub fn new(logo: &'a OsLogo, tint: ResolvedTint) -> Self {
        Self { logo, tint }
    }
}

impl<'a> Widget for OsLogoWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let max_rows = area.height as usize;
        let max_cols = area.width;

        for (row, line) in self.logo.lines.iter().take(max_rows).enumerate() {
            let y = area.y + row as u16;
            // Column cursor, relative to area.x, in display columns.
            let mut col: u16 = 0;
            for span in line.0.iter() {
                if col >= max_cols {
                    break;
                }
                let remaining = (max_cols - col) as usize;
                let w = span_width(span);
                let text = if w <= remaining {
                    span.text.clone()
                } else {
                    span.text.chars().take(remaining).collect()
                };
                if text.is_empty() {
                    continue;
                }
                buf.set_string(area.x + col, y, &text, span_style(span, self.tint));
                col += text.chars().count() as u16;
            }
        }
    }
}

/// Compose an [`OsLogo`] into owned [`Line`]s for a `Paragraph`.
///
/// Used by the detail panel, whose renderer returns a `Paragraph<'static>`
/// and therefore cannot carve a `Rect` to run [`OsLogoWidget`]. Each logo
/// line becomes a [`Line`] of colored [`Span`]s; the caller is responsible for
/// any width clamping the `Paragraph` layout applies.
pub fn logo_to_lines(logo: &OsLogo, tint: ResolvedTint) -> Vec<Line<'static>> {
    logo.lines
        .iter()
        .map(|line| {
            let spans: Vec<Span<'static>> = line
                .0
                .iter()
                .map(|s| Span::styled(s.text.clone(), span_style(s, tint)))
                .collect();
            Line::from(spans)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::osinfo::logos::{OsLogoLine, OsLogoSpan};
    use ratatui::backend::TestBackend;
    use ratatui::style::{Color, Style};
    use ratatui::Terminal;

    fn sample_logo() -> OsLogo {
        OsLogo {
            id: "test",
            lines: vec![
                OsLogoLine(vec![
                    OsLogoSpan {
                        text: "aa".to_string(),
                        style: Style::default().fg(Color::Red),
                    },
                    OsLogoSpan {
                        text: "bb".to_string(),
                        style: Style::default().fg(Color::Blue),
                    },
                ]),
                OsLogoLine(vec![OsLogoSpan {
                    text: "cccc".to_string(),
                    style: Style::default().fg(Color::Green),
                }]),
            ],
        }
    }

    #[test]
    fn renders_in_exact_rect_without_panic() {
        let logo = sample_logo();
        let mut term = Terminal::new(TestBackend::new(4, 2)).unwrap();
        term.draw(|f| {
            let area = f.area();
            f.render_widget(OsLogoWidget::new(&logo, ResolvedTint::Native), area);
        })
        .unwrap();
        let buf = term.backend().buffer();
        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "a");
        assert_eq!(buf.cell((2, 0)).unwrap().symbol(), "b");
        assert_eq!(buf.cell((0, 1)).unwrap().symbol(), "c");
        // Color carried through from the span style.
        assert_eq!(buf.cell((0, 0)).unwrap().fg, Color::Red);
        assert_eq!(buf.cell((2, 0)).unwrap().fg, Color::Blue);
    }

    #[test]
    fn clamps_to_undersized_rect_without_overflow() {
        let logo = sample_logo();
        // Rect narrower and shorter than the logo: 2x1.
        let mut term = Terminal::new(TestBackend::new(2, 1)).unwrap();
        term.draw(|f| {
            f.render_widget(OsLogoWidget::new(&logo, ResolvedTint::Native), f.area());
        })
        .unwrap();
        let buf = term.backend().buffer();
        // Only the first two columns of the first row survive.
        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "a");
        assert_eq!(buf.cell((1, 0)).unwrap().symbol(), "a");
    }

    #[test]
    fn dimensions_report_width_and_height() {
        let logo = sample_logo();
        // row 0: "aa"+"bb" = 4 cols; row 1: "cccc" = 4 cols; 2 rows.
        assert_eq!(logo_dimensions(&logo), (4, 2));
    }

    #[test]
    fn logo_to_lines_preserves_spans() {
        let logo = sample_logo();
        let lines = logo_to_lines(&logo, ResolvedTint::Native);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans.len(), 2);
        assert_eq!(lines[0].spans[0].content, "aa");
        assert_eq!(lines[0].spans[0].style.fg, Some(Color::Red));
        assert_eq!(lines[1].spans[0].content, "cccc");
    }

    /// Cells that actually show a glyph, and the foregrounds they carry.
    fn visible_colors(buf: &ratatui::buffer::Buffer) -> std::collections::BTreeSet<String> {
        let mut out = std::collections::BTreeSet::new();
        for y in buf.area.top()..buf.area.bottom() {
            for x in buf.area.left()..buf.area.right() {
                let cell = buf.cell((x, y)).unwrap();
                let sym = cell.symbol();
                if sym.chars().all(|c| c == '\u{2800}' || c.is_whitespace()) {
                    continue;
                }
                out.insert(format!("{:?}", cell.fg));
            }
        }
        out
    }

    fn render_logo(id: &str, tint: ResolvedTint) -> ratatui::buffer::Buffer {
        let logo = crate::osinfo::large_logo_for(id).expect("a large logo");
        let (w, h) = logo_dimensions(logo);
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| f.render_widget(OsLogoWidget::new(logo, tint), f.area()))
            .unwrap();
        term.backend().buffer().clone()
    }

    /// `native` is not "no tint applied by accident" — it is the contract that
    /// the vendored brand colours survive. And a colour tint must land on the
    /// visible glyphs, not merely somewhere in the buffer.
    #[test]
    fn native_logo_preserves_source_colors_and_tint_recolors_visible_cells() {
        let native = render_logo("macos", ResolvedTint::Native);
        assert!(
            visible_colors(&native).len() > 1,
            "the rainbow apple must keep more than one colour under `native`"
        );

        let tint = Color::Rgb(1, 2, 3);
        let tinted = render_logo("macos", ResolvedTint::Color(tint));
        assert_eq!(
            visible_colors(&tinted),
            std::collections::BTreeSet::from([format!("{tint:?}")]),
            "a colour tint flattens every visible cell onto itself"
        );

        // Ubuntu's art carries ANSI-16 indices rather than truecolour, and they
        // survive `native` for the same reason: nothing recolours them.
        let ubuntu = render_logo("ubuntu", ResolvedTint::Native);
        assert!(!visible_colors(&ubuntu).is_empty());
        assert!(
            !visible_colors(&ubuntu).contains(&format!("{tint:?}")),
            "`native` must not invent a colour the asset never had"
        );
    }

    /// A run of blank Braille keeps its native style: the tint is defined on
    /// the cells a reader can see, and recolouring invisible ones would make
    /// the rule untestable rather than merely harmless.
    #[test]
    fn a_tint_leaves_blank_runs_alone() {
        use crate::osinfo::logos::span_style;

        let blank = OsLogoSpan {
            text: "\u{2800}\u{2800}".to_string(),
            style: Style::default().fg(Color::Red),
        };
        let glyph = OsLogoSpan {
            text: "\u{2800}\u{28ff}".to_string(),
            style: Style::default().fg(Color::Red),
        };
        let tint = ResolvedTint::Color(Color::Rgb(1, 2, 3));
        assert_eq!(span_style(&blank, tint).fg, Some(Color::Red));
        assert_eq!(span_style(&glyph, tint).fg, Some(Color::Rgb(1, 2, 3)));
        assert_eq!(
            span_style(&glyph, ResolvedTint::Native).fg,
            Some(Color::Red)
        );
    }

    /// The `Paragraph` path the detail panel uses must tint exactly like the
    /// widget path; two renderers of the same data cannot disagree.
    #[test]
    fn both_render_paths_apply_the_same_tint() {
        let logo = crate::osinfo::large_logo_for("macos").expect("a large logo");
        let tint = ResolvedTint::Color(Color::Rgb(1, 2, 3));
        let lines = logo_to_lines(logo, tint);
        let visible: std::collections::BTreeSet<_> = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .filter(|s| {
                !s.content
                    .chars()
                    .all(|c| c == '\u{2800}' || c.is_whitespace())
            })
            .map(|s| format!("{:?}", s.style.fg))
            .collect();
        assert_eq!(
            visible,
            std::collections::BTreeSet::from([format!("{:?}", Some(Color::Rgb(1, 2, 3)))])
        );

        let native: std::collections::BTreeSet<_> = logo_to_lines(logo, ResolvedTint::Native)
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| format!("{:?}", s.style.fg))
            .collect();
        assert!(native.len() > 1, "`native` keeps the source colours");
    }

    /// `default` publishes `native`, so an unthemed SSHub shows brand colours.
    #[test]
    fn the_default_theme_leaves_logos_native() {
        use crate::osinfo::logos::os_logo_tint;
        assert_eq!(
            os_logo_tint(&crate::test_support::resolved_default()),
            ResolvedTint::Native
        );
    }
}
