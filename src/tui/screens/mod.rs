pub mod audit;
pub mod auth;
pub mod broadcast;
pub mod field_picker;
pub mod group_form;
pub mod group_manage;
pub mod help;
pub mod hop_lines;
pub mod host_form;
pub mod host_transfer;
pub mod keybind_editor;
pub mod keychain;
pub mod keygen;
pub mod keys;
pub mod known_hosts;
pub mod log_browser;
pub mod map;
pub mod palette;
pub mod push_key_pickers;
pub mod session_picker;
pub mod settings;
pub mod sftp;
pub mod snippet_form;
pub mod snippet_manage;
pub mod snippet_picker;
pub mod tag_filter;
pub mod theme_picker;
pub mod tunnel_reconnect;
pub mod tunnels;

use ratatui::buffer::Buffer;
use ratatui::layout::{Margin, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, StyledGrapheme};
use ratatui::widgets::{Block, Paragraph, Widget, Wrap};
use unicode_width::UnicodeWidthChar;

use crate::app::{App, ClickHit, ClickTarget};

/// A form popup's paragraph that also knows which of its lines show which
/// field, so the click map (#116) can find every field's rows (wrapped ones
/// included) and focus the field a click lands on.
pub struct FormBody {
    para: Paragraph<'static>,
    /// Each line with the field it shows, by position in the form's `ALL`.
    rows: Vec<(Line<'static>, Option<usize>)>,
    wrap: Option<Wrap>,
    /// A row (index into `rows`) that must stay on screen when the form is
    /// taller than its popup: the bottom of an open picker or raw-hop box,
    /// which take input and must not fall off the edge.
    keep_visible: Option<usize>,
}

impl FormBody {
    /// `block` must draw all four borders: the field rows start inside them.
    pub fn new(
        rows: Vec<(Line<'static>, Option<usize>)>,
        block: Block<'static>,
        wrap: Option<Wrap>,
    ) -> Self {
        let mut para =
            Paragraph::new(rows.iter().map(|(l, _)| l.clone()).collect::<Vec<_>>()).block(block);
        if let Some(wrap) = wrap {
            para = para.wrap(wrap);
        }
        Self {
            para,
            rows,
            wrap,
            keep_visible: None,
        }
    }

    /// Scroll the form, if it overflows, so row `row` is the last one drawn.
    pub fn keep_visible(mut self, row: Option<usize>) -> Self {
        self.keep_visible = row;
        self
    }

    /// Drawn height of `line` at `width`.
    fn height_of(&self, line: &Line<'static>, width: u16) -> u16 {
        match self.wrap {
            Some(wrap) => Paragraph::new(line.clone())
                .wrap(wrap)
                .line_count(width)
                .max(1) as u16,
            None => 1,
        }
    }

    /// Rows scrolled off the top of `inner` so `keep_visible` ends in view.
    fn scroll(&self, inner: Rect) -> u16 {
        let Some(row) = self.keep_visible else {
            return 0;
        };
        let end: u16 = self
            .rows
            .iter()
            .take(row + 1)
            .map(|(l, _)| self.height_of(l, inner.width))
            .sum();
        end.saturating_sub(inner.height)
    }

    /// Record each field's drawn rows of `area` (where the form is drawn,
    /// borders included) in `app.click_map`. A field line's value is its last
    /// span; everything before it (marker, label) is chrome.
    pub fn record_clicks(&self, app: &App, area: Rect) {
        let inner = area.inner(Margin::new(1, 1));
        let mut map = app.click_map.borrow_mut();
        let mut y = inner.y;
        let mut skip = self.scroll(inner);
        for (line, field) in &self.rows {
            if y >= inner.bottom() {
                break;
            }
            let height = self.height_of(line, inner.width);
            // Scrolled off the top: not on screen, not clickable.
            if skip > 0 {
                skip = skip.saturating_sub(height);
                continue;
            }
            if let Some(index) = *field {
                let graphemes: Vec<_> = line.styled_graphemes(Style::default()).collect();
                let chrome: usize = line
                    .spans
                    .iter()
                    .rev()
                    .skip(1)
                    .map(|s| s.styled_graphemes(Style::default()).count())
                    .sum();
                // The value's display cells before each of its graphemes, and
                // in all: what `char_at_cell` counts in.
                let mut cells = vec![0];
                for g in &graphemes[chrome..] {
                    let width: usize = g.symbol.chars().map(|c| c.width().unwrap_or(0)).sum();
                    cells.push(cells[cells.len() - 1] + width);
                }
                let rows = self.lay_out(line, &graphemes, inner.width, height);
                for (ry, drawn) in (y..inner.bottom()).zip(rows) {
                    let mut value = drawn.into_iter().filter(|&(_, i)| i >= chrome);
                    let first = value.next();
                    let last = value.next_back().or(first);
                    let (value_x, start, end) = match (first, last) {
                        (Some((x, first)), Some((_, last))) => {
                            (inner.x + x, cells[first - chrome], cells[last - chrome + 1])
                        }
                        // A row of label only: its value starts further down.
                        _ => (inner.x, 0, 0),
                    };
                    map.push(ClickTarget {
                        area: Rect::new(inner.x, ry, inner.width, 1),
                        hit: ClickHit::Field {
                            index,
                            value_x,
                            start,
                            end,
                        },
                    });
                }
            }
            y += height;
        }
    }

    /// Where the paragraph draws `line` (made of `graphemes`) in `height`
    /// rows of `width`: per row, the column and index of each grapheme drawn
    /// on it, left to right. Ratatui lays the line out itself, each grapheme
    /// coloured with its index, so this follows its word wrap exactly: words
    /// pushed to the next row, and blanks dropped at the breaks.
    fn lay_out(
        &self,
        line: &Line,
        graphemes: &[StyledGrapheme],
        width: u16,
        height: u16,
    ) -> Vec<Vec<(u16, usize)>> {
        let mut tagged = Line::from(
            graphemes
                .iter()
                .enumerate()
                .map(|(i, g)| {
                    let [_, r, gr, b] = (i as u32).to_be_bytes();
                    Span::styled(g.symbol, Style::new().fg(Color::Rgb(r, gr, b)))
                })
                .collect::<Vec<_>>(),
        );
        tagged.alignment = line.alignment;
        let mut para = Paragraph::new(tagged);
        if let Some(wrap) = self.wrap {
            para = para.wrap(wrap);
        }
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        para.render(buf.area, &mut buf);
        (0..height)
            .map(|y| {
                (0..width)
                    .filter_map(|x| match buf[(x, y)].fg {
                        Color::Rgb(r, g, b) => Some((x, u32::from_be_bytes([0, r, g, b]) as usize)),
                        _ => None,
                    })
                    .collect()
            })
            .collect()
    }
}

impl Widget for FormBody {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let scroll = self.scroll(area.inner(Margin::new(1, 1)));
        self.para.scroll((scroll, 0)).render(area, buf);
    }
}
