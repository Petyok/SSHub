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

use super::logos::{OsLogo, OsLogoSpan};

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
}

impl<'a> OsLogoWidget<'a> {
    pub fn new(logo: &'a OsLogo) -> Self {
        Self { logo }
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
                buf.set_string(area.x + col, y, &text, span.style);
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
pub fn logo_to_lines(logo: &OsLogo) -> Vec<Line<'static>> {
    logo.lines
        .iter()
        .map(|line| {
            let spans: Vec<Span<'static>> = line
                .0
                .iter()
                .map(|s| Span::styled(s.text.clone(), s.style))
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
            f.render_widget(OsLogoWidget::new(&logo), area);
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
            f.render_widget(OsLogoWidget::new(&logo), f.area());
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
        let lines = logo_to_lines(&logo);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans.len(), 2);
        assert_eq!(lines[0].spans[0].content, "aa");
        assert_eq!(lines[0].spans[0].style.fg, Some(Color::Red));
        assert_eq!(lines[1].spans[0].content, "cccc");
    }
}
