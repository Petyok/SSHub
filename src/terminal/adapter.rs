// Adapted from tui-term 0.3.4 src/vt100_imp.rs.
// Copyright (c) 2023 a-kenji. MIT; see LICENSE-tui-term.
use ratatui::style::{Modifier, Style};

use tui_term::widget::{Cell, Screen};

/// Apply screen defaults as cell styles, before tui-term overlays the cursor.
pub(crate) fn render(
    screen: &crate::terminal::Screen,
    area: ratatui::layout::Rect,
    buffer: &mut ratatui::buffer::Buffer,
) {
    use ratatui::widgets::Widget;
    use tui_term::widget::PseudoTerminal;
    if screen.default_fg().is_none() && screen.default_bg().is_none() {
        PseudoTerminal::new(screen).render(area, buffer);
        return;
    }
    let cells = (0..area.height)
        .flat_map(|row| {
            (0..area.width).map(move |col| {
                screen.cell(row, col).map(|cell| DefaultColorsCell {
                    cell,
                    fg: screen.default_fg(),
                    bg: screen.default_bg(),
                })
            })
        })
        .collect();
    let styled = DefaultColorsScreen {
        screen,
        cells,
        width: area.width,
    };
    PseudoTerminal::new(&styled).render(area, buffer);
}

struct DefaultColorsScreen<'a> {
    screen: &'a crate::terminal::Screen,
    cells: Vec<Option<DefaultColorsCell<'a>>>,
    width: u16,
}

struct DefaultColorsCell<'a> {
    cell: &'a crate::terminal::Cell,
    fg: Option<(u8, u8, u8)>,
    bg: Option<(u8, u8, u8)>,
}

impl<'a> Screen for DefaultColorsScreen<'a> {
    type C = DefaultColorsCell<'a>;

    fn cell(&self, row: u16, col: u16) -> Option<&Self::C> {
        if col >= self.width {
            return None;
        }
        self.cells
            .get(usize::from(row) * usize::from(self.width) + usize::from(col))?
            .as_ref()
    }

    fn hide_cursor(&self) -> bool {
        Screen::hide_cursor(self.screen)
    }
    fn cursor_position(&self) -> (u16, u16) {
        Screen::cursor_position(self.screen)
    }
    fn cursor_shape(&self) -> tui_term::widget::CursorShape {
        Screen::cursor_shape(self.screen)
    }
}

impl Cell for DefaultColorsCell<'_> {
    fn has_contents(&self) -> bool {
        self.cell.has_contents()
    }

    fn apply(&self, target: &mut ratatui::buffer::Cell) {
        fill_buf_cell(self.cell, target);
        if self.cell.fgcolor() == crate::terminal::Color::Default {
            if let Some((r, g, b)) = self.fg {
                target.fg = ratatui::style::Color::Rgb(r, g, b);
            }
        }
        if self.cell.bgcolor() == crate::terminal::Color::Default {
            if let Some((r, g, b)) = self.bg {
                target.bg = ratatui::style::Color::Rgb(r, g, b);
            }
        }
    }
}

impl Screen for crate::terminal::Screen {
    type C = crate::terminal::Cell;

    #[inline]
    fn cell(&self, row: u16, col: u16) -> Option<&Self::C> {
        self.cell(row, col)
    }

    #[inline]
    fn hide_cursor(&self) -> bool {
        self.hide_cursor()
    }

    #[inline]
    fn cursor_shape(&self) -> tui_term::widget::CursorShape {
        use tui_term::widget::CursorShape;
        match self.cursor_style() {
            1 => CursorShape::BlinkingBlock,
            2 => CursorShape::SteadyBlock,
            3 => CursorShape::BlinkingUnderline,
            4 => CursorShape::SteadyUnderline,
            5 => CursorShape::BlinkingBar,
            6 => CursorShape::SteadyBar,
            _ => CursorShape::Default,
        }
    }

    fn cursor_position(&self) -> (u16, u16) {
        let (row, col) = self.cursor_position();
        let scrollback = u16::try_from(self.scrollback()).unwrap_or(u16::MAX);
        (row.saturating_add(scrollback), col)
    }
}

impl Cell for crate::terminal::Cell {
    #[inline]
    fn has_contents(&self) -> bool {
        self.has_contents()
    }

    #[inline]
    fn apply(&self, cell: &mut ratatui::buffer::Cell) {
        fill_buf_cell(self, cell)
    }
}

#[inline]
fn fill_buf_cell(screen_cell: &crate::terminal::Cell, buf_cell: &mut ratatui::buffer::Cell) {
    if screen_cell.has_contents() {
        buf_cell.set_symbol(screen_cell.contents());
    }

    let mut modifier = Modifier::empty();
    if screen_cell.bold() {
        modifier |= Modifier::BOLD;
    }
    if screen_cell.italic() {
        modifier |= Modifier::ITALIC;
    }
    if screen_cell.underline() {
        modifier |= Modifier::UNDERLINED;
    }
    if screen_cell.inverse() {
        modifier |= Modifier::REVERSED;
    }
    if screen_cell.dim() {
        modifier |= Modifier::DIM;
    }

    if screen_cell.strikethrough() {
        modifier |= Modifier::CROSSED_OUT;
    }
    if screen_cell.concealed() {
        modifier |= Modifier::HIDDEN;
    }
    if screen_cell.blink() {
        modifier |= Modifier::SLOW_BLINK;
    }

    let fg = map_color(screen_cell.fgcolor());
    let bg = map_color(screen_cell.bgcolor());

    buf_cell.set_style(Style::reset().fg(fg).bg(bg).add_modifier(modifier));
}

#[inline]
fn map_color(color: crate::terminal::Color) -> ratatui::style::Color {
    match color {
        crate::terminal::Color::Default => ratatui::style::Color::Reset,
        crate::terminal::Color::Idx(i) => ratatui::style::Color::Indexed(i),
        crate::terminal::Color::Rgb(r, g, b) => ratatui::style::Color::Rgb(r, g, b),
    }
}
