use crate::terminal::term::BufWrite as _;
use unicode_width::UnicodeWidthChar as _;

const MODE_APPLICATION_KEYPAD: u8 = 0b0000_0001;
const MODE_APPLICATION_CURSOR: u8 = 0b0000_0010;
const MODE_HIDE_CURSOR: u8 = 0b0000_0100;
const MODE_ALTERNATE_SCREEN: u8 = 0b0000_1000;
const MODE_BRACKETED_PASTE: u8 = 0b0001_0000;
// shellglass: DEC private mode 2026 — synchronized update in progress.
const MODE_SYNCHRONIZED_UPDATE: u8 = 0b0010_0000;
// shellglass: DECAWM off (`CSI ? 7 l`) — inverted so the zero default keeps
// autowrap on, like a real terminal's power-on state.
const MODE_NO_AUTOWRAP: u8 = 0b0100_0000;
// shellglass: IRM (`CSI 4 h`) — printed text shifts the rest of the row right.
const MODE_INSERT: u8 = 0b1000_0000;

/// The xterm mouse handling mode currently in use.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Default)]
pub enum MouseProtocolMode {
    /// Mouse handling is disabled.
    #[default]
    None,

    /// Mouse button events should be reported on button press. Also known as
    /// X10 mouse mode.
    Press,

    /// Mouse button events should be reported on button press and release.
    /// Also known as VT200 mouse mode.
    PressRelease,

    // Highlight,
    /// Mouse button events should be reported on button press and release, as
    /// well as when the mouse moves between cells while a button is held
    /// down.
    ButtonMotion,

    /// Mouse button events should be reported on button press and release,
    /// and mouse motion events should be reported when the mouse moves
    /// between cells regardless of whether a button is held down or not.
    AnyMotion,
    // DecLocator,
}

/// The encoding to use for the enabled [`MouseProtocolMode`].
#[derive(Copy, Clone, Debug, Eq, PartialEq, Default)]
pub enum MouseProtocolEncoding {
    /// Default single-printable-byte encoding.
    #[default]
    Default,

    /// UTF-8-based encoding.
    Utf8,

    /// SGR-like encoding.
    Sgr,

    /// urxvt-like encoding.
    Urxvt,

    /// SGR-like encoding with pixel (rather than cell) coordinates.
    SgrPixels,
}

/// shellglass: how [`Screen::place_data_with`] stamps a region. The default
/// (both `false`) is [`Screen::place_data`]'s sixel behaviour.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Default)]
pub struct PlaceOpts {
    /// Don't move the cursor and never scroll: the region is stamped downwards
    /// from the cursor row and clipped at the last one. kitty's `C=1` on `a=p`,
    /// which zellij emits after positioning the cursor itself.
    pub hold: bool,
    /// The stamped slots survive text written into their cells, dying only on
    /// erase (or when the row scrolls out of the grid) — kitty's placement
    /// lifetime. Without it a repaint of the covered cells silently drops the
    /// placement, while the terminal still shows the image.
    pub sticky: bool,
}

/// Represents the overall terminal state.
///
/// shellglass: generic over the per-cell data slot `T` (default `()` — see
/// [`Cell`](crate::terminal::Cell) and [`Screen::place_data`]).
#[derive(Clone, Debug)]
pub struct Screen<T = ()> {
    grid: crate::terminal::grid::Grid<T>,
    alternate_grid: crate::terminal::grid::Grid<T>,

    attrs: crate::terminal::attrs::Attrs,
    saved_attrs: crate::terminal::attrs::Attrs,

    // shellglass: the last graphic character drawn, so REP (CSI b) can repeat
    // it. Never cleared by cursor movement — ECMA-48 speaks of the preceding
    // character in the *data stream*, and kitty/xterm behave the same way.
    last_graphic_char: Option<char>,

    // shellglass: tab stops (upstream hardcoded every-8 in grid::col_tab).
    // Screen-level (not per-grid) because the primary and alternate screens
    // share one tab-stop table in real terminals. HTS sets, TBC clears, HT/
    // CHT/CBT navigate; RIS resets via Screen::new.
    tab_stops: std::collections::BTreeSet<u16>,

    // shellglass: wrapping counters of BSU (`CSI ? 2026 h`) and ESU
    // (`CSI ? 2026 l`) events, so a consumer sampling the screen between
    // reads can detect edges that opened AND closed inside one read — the
    // mode bit alone can't distinguish "still the same update" from "a new
    // one started after the last presented".
    sync_starts: u32,
    sync_ends: u32,

    // shellglass: DECSCUSR (`CSI n SP q`) cursor style, raw value 0-6 —
    // 0 default, 1/2 blinking/steady block, 3/4 underline, 5/6 bar.
    cursor_style: u8,

    // shellglass: OSC 10/11 default foreground/background overrides
    // (`None` = the terminal's own default); OSC 110/111 reset, RIS wipes.
    default_fg: Option<(u8, u8, u8)>,
    default_bg: Option<(u8, u8, u8)>,

    // shellglass: window title (OSC 0/2; icon names aren't rendered) plus the
    // XTWINOPS 22/23 save/restore stack. RIS wipes both.
    title: String,
    title_stack: Vec<String>,

    // shellglass: OSC 8 hyperlink table, id → URI, deduped by URI so a
    // redrawn link keeps its id (no spurious diffs downstream) — plus the
    // reverse map that does the dedup. Bounded: past the cap the lowest
    // (oldest) ids are pruned; a pruned id still stamped in a cell resolves
    // to no link, which can only happen if the cell has long scrolled into
    // survival edge cases (the cap far exceeds any screen's cell count).
    links: std::collections::BTreeMap<u32, String>,
    link_ids: std::collections::HashMap<String, std::num::NonZeroU32>,
    next_link_id: u32,

    modes: u8,
    mouse_protocol_mode: MouseProtocolMode,
    mouse_protocol_encoding: MouseProtocolEncoding,
}

impl<T> Screen<T> {
    pub(crate) fn new(size: crate::terminal::grid::Size, scrollback_len: usize) -> Self {
        let mut grid = crate::terminal::grid::Grid::new(size, scrollback_len);
        grid.allocate_rows();
        Self {
            grid,
            alternate_grid: crate::terminal::grid::Grid::new(size, 0),

            attrs: crate::terminal::attrs::Attrs::default(),
            saved_attrs: crate::terminal::attrs::Attrs::default(),

            last_graphic_char: None,

            tab_stops: default_tab_stops(size.cols),

            sync_starts: 0,
            sync_ends: 0,

            cursor_style: 0,

            default_fg: None,
            default_bg: None,

            title: String::new(),
            title_stack: Vec::new(),

            links: std::collections::BTreeMap::new(),
            link_ids: std::collections::HashMap::new(),
            next_link_id: 1,

            modes: 0,
            mouse_protocol_mode: MouseProtocolMode::default(),
            mouse_protocol_encoding: MouseProtocolEncoding::default(),
        }
    }

    /// Resizes the terminal.
    pub fn set_size(&mut self, rows: u16, cols: u16) {
        // shellglass: newly-revealed columns get default tab stops (existing
        // stops are kept — navigation bounds itself by the current width).
        let old_cols = self.grid.size().cols;
        if cols > old_cols {
            self.tab_stops
                .extend(default_tab_stops(cols).range(old_cols..));
        }
        self.grid
            .set_size(crate::terminal::grid::Size { rows, cols });
        self.alternate_grid
            .set_size(crate::terminal::grid::Size { rows, cols });
    }

    /// Returns the current size of the terminal.
    ///
    /// The return value will be (rows, cols).
    #[must_use]
    pub fn size(&self) -> (u16, u16) {
        let size = self.grid().size();
        (size.rows, size.cols)
    }

    /// Scrolls to the given position in the scrollback.
    ///
    /// This position indicates the offset from the top of the screen, and
    /// should be `0` to put the normal screen in view.
    ///
    /// This affects the return values of methods called on the screen: for
    /// instance, `screen.cell(0, 0)` will return the top left corner of the
    /// screen after taking the scrollback offset into account.
    ///
    /// The value given will be clamped to the actual size of the scrollback.
    pub fn set_scrollback(&mut self, rows: usize) {
        self.grid_mut().set_scrollback(rows);
    }

    /// Returns the current position in the scrollback.
    ///
    /// This position indicates the offset from the top of the screen, and is
    /// `0` when the normal screen is in view.
    #[must_use]
    pub fn scrollback(&self) -> usize {
        self.grid().scrollback()
    }

    /// Returns the text contents of the terminal.
    ///
    /// This will not include any formatting information, and will be in plain
    /// text format.
    #[must_use]
    pub fn contents(&self) -> String {
        let mut contents = String::new();
        self.write_contents(&mut contents);
        contents
    }

    fn write_contents(&self, contents: &mut String) {
        self.grid().write_contents(contents);
    }

    /// Returns the text contents of the terminal by row, restricted to the
    /// given subset of columns.
    ///
    /// This will not include any formatting information, and will be in plain
    /// text format.
    ///
    /// Newlines will not be included.
    pub fn rows(&self, start: u16, width: u16) -> impl Iterator<Item = String> + '_ {
        self.grid().visible_rows().map(move |row| {
            let mut contents = String::new();
            row.write_contents(&mut contents, start, width, false);
            contents
        })
    }

    /// Returns the text contents of the terminal logically between two cells.
    /// This will include the remainder of the starting row after `start_col`,
    /// followed by the entire contents of the rows between `start_row` and
    /// `end_row`, followed by the beginning of the `end_row` up until
    /// `end_col`. This is useful for things like determining the contents of
    /// a clipboard selection.
    #[must_use]
    pub fn contents_between(
        &self,
        start_row: u16,
        start_col: u16,
        end_row: u16,
        end_col: u16,
    ) -> String {
        match start_row.cmp(&end_row) {
            std::cmp::Ordering::Less => {
                let (_, cols) = self.size();
                let mut contents = String::new();
                for (i, row) in self
                    .grid()
                    .visible_rows()
                    .enumerate()
                    .skip(usize::from(start_row))
                    .take(usize::from(end_row) - usize::from(start_row) + 1)
                {
                    if i == usize::from(start_row) {
                        row.write_contents(&mut contents, start_col, cols - start_col, false);
                        if !row.wrapped() {
                            contents.push('\n');
                        }
                    } else if i == usize::from(end_row) {
                        row.write_contents(&mut contents, 0, end_col, false);
                    } else {
                        row.write_contents(&mut contents, 0, cols, false);
                        if !row.wrapped() {
                            contents.push('\n');
                        }
                    }
                }
                contents
            }
            std::cmp::Ordering::Equal => {
                if start_col < end_col {
                    self.rows(start_col, end_col - start_col)
                        .nth(usize::from(start_row))
                        .unwrap_or_default()
                } else {
                    String::new()
                }
            }
            std::cmp::Ordering::Greater => String::new(),
        }
    }

    /// Return escape codes sufficient to reproduce the entire contents of the
    /// current terminal state. This is a convenience wrapper around
    /// [`contents_formatted`](Self::contents_formatted) and
    /// [`input_mode_formatted`](Self::input_mode_formatted).
    #[must_use]
    pub fn state_formatted(&self) -> Vec<u8> {
        let mut contents = vec![];
        self.write_contents_formatted(&mut contents);
        self.write_input_mode_formatted(&mut contents);
        contents
    }

    /// Return escape codes sufficient to turn the terminal state of the
    /// screen `prev` into the current terminal state. This is a convenience
    /// wrapper around [`contents_diff`](Self::contents_diff) and
    /// [`input_mode_diff`](Self::input_mode_diff).
    #[must_use]
    pub fn state_diff(&self, prev: &Self) -> Vec<u8> {
        let mut contents = vec![];
        self.write_contents_diff(&mut contents, prev);
        self.write_input_mode_diff(&mut contents, prev);
        contents
    }

    /// Returns the formatted visible contents of the terminal.
    ///
    /// Formatting information will be included inline as terminal escape
    /// codes. The result will be suitable for feeding directly to a raw
    /// terminal parser, and will result in the same visual output.
    #[must_use]
    pub fn contents_formatted(&self) -> Vec<u8> {
        let mut contents = vec![];
        self.write_contents_formatted(&mut contents);
        contents
    }

    fn write_contents_formatted(&self, contents: &mut Vec<u8>) {
        crate::terminal::term::HideCursor::new(self.hide_cursor()).write_buf(contents);
        let prev_attrs = self.grid().write_contents_formatted(contents);
        self.attrs.write_escape_code_diff(contents, &prev_attrs);
    }

    /// Returns the formatted visible contents of the terminal by row,
    /// restricted to the given subset of columns.
    ///
    /// Formatting information will be included inline as terminal escape
    /// codes. The result will be suitable for feeding directly to a raw
    /// terminal parser, and will result in the same visual output.
    ///
    /// You are responsible for positioning the cursor before printing each
    /// row, and the final cursor position after displaying each row is
    /// unspecified.
    // the unwraps in this method shouldn't be reachable
    #[allow(clippy::missing_panics_doc)]
    pub fn rows_formatted(&self, start: u16, width: u16) -> impl Iterator<Item = Vec<u8>> + '_ {
        let mut wrapping = false;
        self.grid().visible_rows().enumerate().map(move |(i, row)| {
            // number of rows in a grid is stored in a u16 (see Size), so
            // visible_rows can never return enough rows to overflow here
            let i = i.try_into().unwrap();
            let mut contents = vec![];
            row.write_contents_formatted(&mut contents, start, width, i, wrapping, None, None);
            if start == 0 && width == self.grid.size().cols {
                wrapping = row.wrapped();
            }
            contents
        })
    }

    /// Returns a terminal byte stream sufficient to turn the visible contents
    /// of the screen described by `prev` into the visible contents of the
    /// screen described by `self`.
    ///
    /// The result of rendering `prev.contents_formatted()` followed by
    /// `self.contents_diff(prev)` should be equivalent to the result of
    /// rendering `self.contents_formatted()`. This is primarily useful when
    /// you already have a terminal parser whose state is described by `prev`,
    /// since the diff will likely require less memory and cause less
    /// flickering than redrawing the entire screen contents.
    #[must_use]
    pub fn contents_diff(&self, prev: &Self) -> Vec<u8> {
        let mut contents = vec![];
        self.write_contents_diff(&mut contents, prev);
        contents
    }

    fn write_contents_diff(&self, contents: &mut Vec<u8>, prev: &Self) {
        if self.hide_cursor() != prev.hide_cursor() {
            crate::terminal::term::HideCursor::new(self.hide_cursor()).write_buf(contents);
        }
        let prev_attrs = self
            .grid()
            .write_contents_diff(contents, prev.grid(), prev.attrs);
        self.attrs.write_escape_code_diff(contents, &prev_attrs);
    }

    /// Returns a sequence of terminal byte streams sufficient to turn the
    /// visible contents of the subset of each row from `prev` (as described
    /// by `start` and `width`) into the visible contents of the corresponding
    /// row subset in `self`.
    ///
    /// You are responsible for positioning the cursor before printing each
    /// row, and the final cursor position after displaying each row is
    /// unspecified.
    // the unwraps in this method shouldn't be reachable
    #[allow(clippy::missing_panics_doc)]
    pub fn rows_diff<'a>(
        &'a self,
        prev: &'a Self,
        start: u16,
        width: u16,
    ) -> impl Iterator<Item = Vec<u8>> + 'a {
        self.grid()
            .visible_rows()
            .zip(prev.grid().visible_rows())
            .enumerate()
            .map(move |(i, (row, prev_row))| {
                // number of rows in a grid is stored in a u16 (see Size), so
                // visible_rows can never return enough rows to overflow here
                let i = i.try_into().unwrap();
                let mut contents = vec![];
                row.write_contents_diff(
                    &mut contents,
                    prev_row,
                    start,
                    width,
                    i,
                    false,
                    false,
                    crate::terminal::grid::Pos { row: i, col: start },
                    crate::terminal::attrs::Attrs::default(),
                );
                contents
            })
    }

    /// Returns terminal escape sequences sufficient to set the current
    /// terminal's input modes.
    ///
    /// Supported modes are:
    /// * application keypad
    /// * application cursor
    /// * bracketed paste
    /// * xterm mouse support
    #[must_use]
    pub fn input_mode_formatted(&self) -> Vec<u8> {
        let mut contents = vec![];
        self.write_input_mode_formatted(&mut contents);
        contents
    }

    fn write_input_mode_formatted(&self, contents: &mut Vec<u8>) {
        crate::terminal::term::ApplicationKeypad::new(self.mode(MODE_APPLICATION_KEYPAD))
            .write_buf(contents);
        crate::terminal::term::ApplicationCursor::new(self.mode(MODE_APPLICATION_CURSOR))
            .write_buf(contents);
        crate::terminal::term::BracketedPaste::new(self.mode(MODE_BRACKETED_PASTE))
            .write_buf(contents);
        crate::terminal::term::MouseProtocolMode::new(
            self.mouse_protocol_mode,
            MouseProtocolMode::None,
        )
        .write_buf(contents);
        crate::terminal::term::MouseProtocolEncoding::new(
            self.mouse_protocol_encoding,
            MouseProtocolEncoding::Default,
        )
        .write_buf(contents);
    }

    /// Returns terminal escape sequences sufficient to change the previous
    /// terminal's input modes to the input modes enabled in the current
    /// terminal.
    #[must_use]
    pub fn input_mode_diff(&self, prev: &Self) -> Vec<u8> {
        let mut contents = vec![];
        self.write_input_mode_diff(&mut contents, prev);
        contents
    }

    fn write_input_mode_diff(&self, contents: &mut Vec<u8>, prev: &Self) {
        if self.mode(MODE_APPLICATION_KEYPAD) != prev.mode(MODE_APPLICATION_KEYPAD) {
            crate::terminal::term::ApplicationKeypad::new(self.mode(MODE_APPLICATION_KEYPAD))
                .write_buf(contents);
        }
        if self.mode(MODE_APPLICATION_CURSOR) != prev.mode(MODE_APPLICATION_CURSOR) {
            crate::terminal::term::ApplicationCursor::new(self.mode(MODE_APPLICATION_CURSOR))
                .write_buf(contents);
        }
        if self.mode(MODE_BRACKETED_PASTE) != prev.mode(MODE_BRACKETED_PASTE) {
            crate::terminal::term::BracketedPaste::new(self.mode(MODE_BRACKETED_PASTE))
                .write_buf(contents);
        }
        crate::terminal::term::MouseProtocolMode::new(
            self.mouse_protocol_mode,
            prev.mouse_protocol_mode,
        )
        .write_buf(contents);
        crate::terminal::term::MouseProtocolEncoding::new(
            self.mouse_protocol_encoding,
            prev.mouse_protocol_encoding,
        )
        .write_buf(contents);
    }

    /// Returns terminal escape sequences sufficient to set the current
    /// terminal's drawing attributes.
    ///
    /// Supported drawing attributes are:
    /// * fgcolor
    /// * bgcolor
    /// * bold
    /// * dim
    /// * italic
    /// * underline
    /// * inverse
    ///
    /// This is not typically necessary, since
    /// [`contents_formatted`](Self::contents_formatted) will leave
    /// the current active drawing attributes in the correct state, but this
    /// can be useful in the case of drawing additional things on top of a
    /// terminal output, since you will need to restore the terminal state
    /// without the terminal contents necessarily being the same.
    #[must_use]
    pub fn attributes_formatted(&self) -> Vec<u8> {
        let mut contents = vec![];
        self.write_attributes_formatted(&mut contents);
        contents
    }

    fn write_attributes_formatted(&self, contents: &mut Vec<u8>) {
        crate::terminal::term::ClearAttrs.write_buf(contents);
        self.attrs
            .write_escape_code_diff(contents, &crate::terminal::attrs::Attrs::default());
    }

    /// Returns the current cursor position of the terminal.
    ///
    /// The return value will be (row, col).
    #[must_use]
    pub fn cursor_position(&self) -> (u16, u16) {
        let pos = self.grid().pos();
        (pos.row, pos.col)
    }

    /// Returns terminal escape sequences sufficient to set the current
    /// cursor state of the terminal.
    ///
    /// This is not typically necessary, since
    /// [`contents_formatted`](Self::contents_formatted) will leave
    /// the cursor in the correct state, but this can be useful in the case of
    /// drawing additional things on top of a terminal output, since you will
    /// need to restore the terminal state without the terminal contents
    /// necessarily being the same.
    ///
    /// Note that the bytes returned by this function may alter the active
    /// drawing attributes, because it may require redrawing existing cells in
    /// order to position the cursor correctly (for instance, in the case
    /// where the cursor is past the end of a row). Therefore, you should
    /// ensure to reset the active drawing attributes if necessary after
    /// processing this data, for instance by using
    /// [`attributes_formatted`](Self::attributes_formatted).
    #[must_use]
    pub fn cursor_state_formatted(&self) -> Vec<u8> {
        let mut contents = vec![];
        self.write_cursor_state_formatted(&mut contents);
        contents
    }

    fn write_cursor_state_formatted(&self, contents: &mut Vec<u8>) {
        crate::terminal::term::HideCursor::new(self.hide_cursor()).write_buf(contents);
        self.grid()
            .write_cursor_position_formatted(contents, None, None);

        // we don't just call write_attributes_formatted here, because that
        // would still be confusing - consider the case where the user sets
        // their own unrelated drawing attributes (on a different parser
        // instance) and then calls cursor_state_formatted. just documenting
        // it and letting the user handle it on their own is more
        // straightforward.
    }

    /// Returns the [`Cell`](crate::terminal::Cell) object at the given location in the
    /// terminal, if it exists.
    #[must_use]
    pub fn cell(&self, row: u16, col: u16) -> Option<&crate::terminal::Cell<T>> {
        self.grid()
            .visible_cell(crate::terminal::grid::Pos { row, col })
    }

    /// shellglass: stamp per-cell data over a `width`×`height` region at the
    /// current cursor position, advancing (and scrolling) one row per region
    /// row exactly as printing it would, and leaving the cursor at column 0
    /// of the region's last row — where a sixel-scrolling terminal leaves it
    /// after an inline image, the motivating consumer.
    ///
    /// Each covered cell gets `data(row_off, col_off)` in its data slot; the
    /// slot rides the cell through scrolling, reflow, and line
    /// insertion/deletion, and dies when the cell's contents are overwritten
    /// or erased — mirroring a cell-based sixel terminal's own erase
    /// semantics. Cell text and attributes are untouched (overlays draw
    /// *over* cells). A too-wide region is clipped at the right edge.
    ///
    /// See [`place_data_with`](Self::place_data_with) for the placements that
    /// need other cursor or lifetime semantics.
    pub fn place_data(&mut self, width: u16, height: u16, data: impl FnMut(u16, u16) -> T) {
        self.place_data_with(width, height, PlaceOpts::default(), data);
    }

    /// shellglass: [`place_data`](Self::place_data) with the two knobs a
    /// graphics protocol richer than sixel needs — see [`PlaceOpts`].
    pub fn place_data_with(
        &mut self,
        width: u16,
        height: u16,
        opts: PlaceOpts,
        mut data: impl FnMut(u16, u16) -> T,
    ) {
        let PlaceOpts { hold, sticky } = opts;
        let size = self.grid().size();
        let left = self.grid().pos().col.min(size.cols - 1);
        let width = width.clamp(1, size.cols - left);
        let top = self.grid().pos().row;
        for row_off in 0..height.max(1) {
            let row = if hold {
                match top.checked_add(row_off) {
                    Some(row) if row < size.rows => row,
                    _ => break, // clipped at the bottom, never scrolled
                }
            } else {
                if row_off > 0 {
                    self.grid_mut().row_inc_scroll(1);
                }
                self.grid().pos().row
            };
            for col_off in 0..width {
                let pos = crate::terminal::grid::Pos {
                    row,
                    col: left + col_off,
                };
                if let Some(cell) = self.grid_mut().drawing_cell_mut(pos) {
                    cell.set_data(Some(data(row_off, col_off)), sticky);
                }
            }
        }
        if !hold {
            self.grid_mut().col_set(0);
        }
    }

    /// Returns whether the text in row `row` should wrap to the next line.
    #[must_use]
    pub fn row_wrapped(&self, row: u16) -> bool {
        self.grid()
            .visible_row(row)
            .is_some_and(crate::terminal::row::Row::wrapped)
    }

    /// Returns whether the alternate screen is currently in use.
    #[must_use]
    pub fn alternate_screen(&self) -> bool {
        self.mode(MODE_ALTERNATE_SCREEN)
    }

    /// Returns whether the terminal should be in application keypad mode.
    #[must_use]
    pub fn application_keypad(&self) -> bool {
        self.mode(MODE_APPLICATION_KEYPAD)
    }

    /// Returns whether the terminal should be in application cursor mode.
    #[must_use]
    pub fn application_cursor(&self) -> bool {
        self.mode(MODE_APPLICATION_CURSOR)
    }

    /// Returns whether the terminal should be in hide cursor mode.
    #[must_use]
    pub fn hide_cursor(&self) -> bool {
        self.mode(MODE_HIDE_CURSOR)
    }

    /// shellglass: the DECSCUSR (`CSI n SP q`) cursor style, raw value 0-6 —
    /// 0 default, 1 blinking block, 2 steady block, 3 blinking underline,
    /// 4 steady underline, 5 blinking bar, 6 steady bar.
    #[must_use]
    pub fn cursor_style(&self) -> u8 {
        self.cursor_style
    }

    /// shellglass: the OSC 10 default-foreground override, if an application
    /// set one (`None` = the terminal's own default).
    #[must_use]
    pub fn default_fg(&self) -> Option<(u8, u8, u8)> {
        self.default_fg
    }

    /// shellglass: the OSC 11 default-background override.
    #[must_use]
    pub fn default_bg(&self) -> Option<(u8, u8, u8)> {
        self.default_bg
    }

    /// shellglass: the window title (OSC 0/2), empty if never set.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    // shellglass: OSC 0/2
    pub(crate) fn set_title(&mut self, s: &[u8]) {
        // Bound retained titles, preserving a valid UTF-8 boundary.
        let title = String::from_utf8_lossy(&s[..s.len().min(4096)]);
        self.title = title.chars().take(1024).collect();
    }

    // shellglass: XTWINOPS 22 — save the title on the stack. Bounded like
    // xterm: beyond the cap the oldest entry is discarded.
    pub(crate) fn title_push(&mut self) {
        const MAX_TITLE_STACK: usize = 16;
        if self.title_stack.len() >= MAX_TITLE_STACK {
            self.title_stack.remove(0);
        }
        self.title_stack.push(self.title.clone());
    }

    // shellglass: XTWINOPS 23 — restore the last saved title (a pop from an
    // empty stack is a no-op, like xterm).
    pub(crate) fn title_pop(&mut self) {
        if let Some(t) = self.title_stack.pop() {
            self.title = t;
        }
    }

    /// shellglass: resolve an OSC 8 hyperlink id (from [`Cell::link`]) to its
    /// URI. `None` for ids pruned from the bounded table.
    #[must_use]
    pub fn link_uri(&self, id: std::num::NonZeroU32) -> Option<&str> {
        self.links.get(&id.get()).map(String::as_str)
    }

    // shellglass: OSC 8 open — everything printed until the close carries the
    // link. Same URI ⇒ same id (dedup), so redrawn links are stable.
    pub(crate) fn link_open(&mut self, uri: &[u8]) {
        const MAX_LINKS: usize = 8192;
        if uri.len() > 2048 {
            self.attrs.link = None;
            return;
        }
        let Ok(uri) = std::str::from_utf8(uri) else {
            self.attrs.link = None;
            return;
        };
        let uri = uri.to_owned();
        if let Some(&id) = self.link_ids.get(&uri) {
            self.attrs.link = Some(id);
            return;
        }
        let Some(id) = std::num::NonZeroU32::new(self.next_link_id) else {
            return; // 4 billion distinct URIs: stop allocating, keep parsing
        };
        self.next_link_id += 1;
        while self.links.len() >= MAX_LINKS {
            let (&oldest, _) = self.links.iter().next().unwrap();
            let uri = self.links.remove(&oldest).unwrap();
            self.link_ids.remove(&uri);
        }
        self.links.insert(id.get(), uri.clone());
        self.link_ids.insert(uri, id);
        self.attrs.link = Some(id);
    }

    // shellglass: OSC 8 close (`OSC 8 ; ; ST`).
    pub(crate) fn link_close(&mut self) {
        self.attrs.link = None;
    }

    // shellglass: OSC 10 / 110
    pub(crate) fn set_default_fg(&mut self, c: Option<(u8, u8, u8)>) {
        self.default_fg = c;
    }

    // shellglass: OSC 11 / 111
    pub(crate) fn set_default_bg(&mut self, c: Option<(u8, u8, u8)>) {
        self.default_bg = c;
    }

    /// shellglass: whether a synchronized update (DEC private mode 2026) is
    /// in progress — the application asked for output between BSU and ESU to
    /// be presented atomically.
    #[must_use]
    pub fn synchronized_update(&self) -> bool {
        self.mode(MODE_SYNCHRONIZED_UPDATE)
    }

    /// shellglass: how many synchronized updates have begun (wrapping).
    #[must_use]
    pub fn synchronized_update_starts(&self) -> u32 {
        self.sync_starts
    }

    /// shellglass: how many synchronized updates have ended (wrapping).
    #[must_use]
    pub fn synchronized_update_ends(&self) -> u32 {
        self.sync_ends
    }

    /// Returns whether the terminal should be in bracketed paste mode.
    #[must_use]
    pub fn bracketed_paste(&self) -> bool {
        self.mode(MODE_BRACKETED_PASTE)
    }

    /// Returns the currently active [`MouseProtocolMode`].
    #[must_use]
    pub fn mouse_protocol_mode(&self) -> MouseProtocolMode {
        self.mouse_protocol_mode
    }

    /// Returns the currently active [`MouseProtocolEncoding`].
    #[must_use]
    pub fn mouse_protocol_encoding(&self) -> MouseProtocolEncoding {
        self.mouse_protocol_encoding
    }

    /// Returns the currently active foreground color.
    #[must_use]
    pub fn fgcolor(&self) -> crate::terminal::Color {
        self.attrs.fgcolor
    }

    /// Returns the currently active background color.
    #[must_use]
    pub fn bgcolor(&self) -> crate::terminal::Color {
        self.attrs.bgcolor
    }

    /// Returns whether newly drawn text should be rendered with the bold text
    /// attribute.
    #[must_use]
    pub fn bold(&self) -> bool {
        self.attrs.bold()
    }

    /// Returns whether newly drawn text should be rendered with the dim text
    /// attribute.
    #[must_use]
    pub fn dim(&self) -> bool {
        self.attrs.dim()
    }

    /// Returns whether newly drawn text should be rendered with the italic
    /// text attribute.
    #[must_use]
    pub fn italic(&self) -> bool {
        self.attrs.italic()
    }

    /// Returns whether newly drawn text should be rendered with the
    /// underlined text attribute.
    #[must_use]
    pub fn underline(&self) -> bool {
        self.attrs.underline()
    }

    /// Returns whether newly drawn text should be rendered with the inverse
    /// text attribute.
    #[must_use]
    pub fn inverse(&self) -> bool {
        self.attrs.inverse()
    }

    pub(crate) fn grid(&self) -> &crate::terminal::grid::Grid<T> {
        if self.mode(MODE_ALTERNATE_SCREEN) {
            &self.alternate_grid
        } else {
            &self.grid
        }
    }

    fn grid_mut(&mut self) -> &mut crate::terminal::grid::Grid<T> {
        if self.mode(MODE_ALTERNATE_SCREEN) {
            &mut self.alternate_grid
        } else {
            &mut self.grid
        }
    }

    fn enter_alternate_grid(&mut self) {
        self.grid_mut().set_scrollback(0);
        self.set_mode(MODE_ALTERNATE_SCREEN);
        self.alternate_grid.allocate_rows();
    }

    fn exit_alternate_grid(&mut self) {
        self.clear_mode(MODE_ALTERNATE_SCREEN);
    }

    fn save_cursor(&mut self) {
        self.grid_mut().save_cursor();
        self.saved_attrs = self.attrs;
    }

    fn restore_cursor(&mut self) {
        self.grid_mut().restore_cursor();
        self.attrs = self.saved_attrs;
    }

    fn set_mode(&mut self, mode: u8) {
        self.modes |= mode;
    }

    fn clear_mode(&mut self, mode: u8) {
        self.modes &= !mode;
    }

    fn mode(&self, mode: u8) -> bool {
        self.modes & mode != 0
    }

    fn set_mouse_mode(&mut self, mode: MouseProtocolMode) {
        self.mouse_protocol_mode = mode;
    }

    fn clear_mouse_mode(&mut self, mode: MouseProtocolMode) {
        if self.mouse_protocol_mode == mode {
            self.mouse_protocol_mode = MouseProtocolMode::default();
        }
    }

    fn set_mouse_encoding(&mut self, encoding: MouseProtocolEncoding) {
        self.mouse_protocol_encoding = encoding;
    }

    fn clear_mouse_encoding(&mut self, encoding: MouseProtocolEncoding) {
        if self.mouse_protocol_encoding == encoding {
            self.mouse_protocol_encoding = MouseProtocolEncoding::default();
        }
    }
}

impl<T> Screen<T> {
    pub(crate) fn text(&mut self, c: char) {
        let pos = self.grid().pos();
        let size = self.grid().size();
        let attrs = self.attrs;

        let width = c.width();
        if width.is_none() && (u32::from(c)) < 256 {
            // don't even try to draw control characters
            return;
        }
        let width = width
            .unwrap_or(1)
            .try_into()
            // width() can only return 0, 1, or 2
            .unwrap();

        self.last_graphic_char = Some(c);

        // shellglass: a glyph wider than the whole terminal can't be placed —
        // the wrap/placement path below assumes col_wrap reserved `width`
        // columns, which is impossible when cols < width (a width-2 char on a
        // 1-col terminal). Drop it rather than run the assumption off the end
        // (an out-of-bounds unwrap); a terminal that narrow showing a
        // double-width glyph is degenerate anyway.
        if width > size.cols {
            return;
        }

        // it doesn't make any sense to wrap if the last column in a row
        // didn't already have contents. don't try to handle the case where a
        // character wraps because there was only one column left in the
        // previous row - literally everything handles this case differently,
        // and this is tmux behavior (and also the simplest). i'm open to
        // reconsidering this behavior, but only with a really good reason
        // (xterm handles this by introducing the concept of triple width
        // cells, which i really don't want to do).
        // shellglass: with autowrap off (DECAWM, `CSI ? 7 l`) the cursor
        // clamps at the right margin and new text overwrites the edge cell,
        // like xterm — never spilling onto the next line.
        // shellglass: saturating_sub — a wide char (width 2) wider than the
        // whole terminal (cols 1) would underflow `size.cols - width` (a
        // release-mode u16 wrap to ~65535, a debug panic on the parser thread).
        let right_margin = size.cols.saturating_sub(width);
        if self.mode(MODE_NO_AUTOWRAP) {
            if width > 0 && pos.col > right_margin {
                self.grid_mut().col_set(right_margin);
            }
        } else {
            let mut wrap = false;
            if pos.col > right_margin {
                let last_cell = self
                    .grid()
                    .drawing_cell(crate::terminal::grid::Pos {
                        row: pos.row,
                        col: size.cols.saturating_sub(1),
                    })
                    // pos.row is valid, since it comes directly from
                    // self.grid().pos() which we assume to always have a valid
                    // row value. size.cols - 1 is also always a valid column.
                    .unwrap();
                if last_cell.has_contents() || last_cell.is_wide_continuation() {
                    wrap = true;
                }
            }
            self.grid_mut().col_wrap(width, wrap);
        }
        let pos = self.grid().pos();
        // shellglass: insert mode (IRM, `CSI 4 h`) shifts the rest of the row
        // right before the glyph lands; combining characters (width 0) still
        // modify the preceding cell in place.
        if width > 0 && self.mode(MODE_INSERT) {
            self.grid_mut().insert_cells(width);
        }

        if width == 0 {
            if pos.col > 0 {
                let mut prev_cell = self
                    .grid_mut()
                    .drawing_cell_mut(crate::terminal::grid::Pos {
                        row: pos.row,
                        col: pos.col - 1,
                    })
                    // pos.row is valid, since it comes directly from
                    // self.grid().pos() which we assume to always have a
                    // valid row value. pos.col - 1 is valid because we just
                    // checked for pos.col > 0.
                    .unwrap();
                if prev_cell.is_wide_continuation() {
                    prev_cell = self
                        .grid_mut()
                        .drawing_cell_mut(crate::terminal::grid::Pos {
                            row: pos.row,
                            col: pos.col - 2,
                        })
                        // pos.row is valid, since it comes directly from
                        // self.grid().pos() which we assume to always have a
                        // valid row value. we know pos.col - 2 is valid
                        // because the cell at pos.col - 1 is a wide
                        // continuation character, which means there must be
                        // the first half of the wide character before it.
                        .unwrap();
                }
                prev_cell.append(c);
            } else if pos.row > 0 {
                let prev_row = self
                    .grid()
                    .drawing_row(pos.row - 1)
                    // pos.row is valid, since it comes directly from
                    // self.grid().pos() which we assume to always have a
                    // valid row value. pos.row - 1 is valid because we just
                    // checked for pos.row > 0.
                    .unwrap();
                if prev_row.wrapped() {
                    let mut prev_cell = self
                        .grid_mut()
                        .drawing_cell_mut(crate::terminal::grid::Pos {
                            row: pos.row - 1,
                            col: size.cols - 1,
                        })
                        // pos.row is valid, since it comes directly from
                        // self.grid().pos() which we assume to always have a
                        // valid row value. pos.row - 1 is valid because we
                        // just checked for pos.row > 0. col of size.cols - 1
                        // is always valid.
                        .unwrap();
                    if prev_cell.is_wide_continuation() {
                        prev_cell = self
                            .grid_mut()
                            .drawing_cell_mut(crate::terminal::grid::Pos {
                                row: pos.row - 1,
                                col: size.cols - 2,
                            })
                            // pos.row is valid, since it comes directly from
                            // self.grid().pos() which we assume to always
                            // have a valid row value. pos.row - 1 is valid
                            // because we just checked for pos.row > 0. col of
                            // size.cols - 2 is valid because the cell at
                            // size.cols - 1 is a wide continuation character,
                            // so it must have the first half of the wide
                            // character before it.
                            .unwrap();
                    }
                    prev_cell.append(c);
                }
            }
        } else {
            if self
                .grid()
                .drawing_cell(pos)
                // pos.row is valid because we assume self.grid().pos() to
                // always have a valid row value. pos.col is valid because we
                // called col_wrap() immediately before this, which ensures
                // that self.grid().pos().col has a valid value.
                .unwrap()
                .is_wide_continuation()
            {
                let prev_cell = self
                    .grid_mut()
                    .drawing_cell_mut(crate::terminal::grid::Pos {
                        row: pos.row,
                        col: pos.col - 1,
                    })
                    // pos.row is valid because we assume self.grid().pos() to
                    // always have a valid row value. pos.col is valid because
                    // we called col_wrap() immediately before this, which
                    // ensures that self.grid().pos().col has a valid value.
                    // pos.col - 1 is valid because the cell at pos.col is a
                    // wide continuation character, so it must have the first
                    // half of the wide character before it.
                    .unwrap();
                prev_cell.clear(attrs);
            }

            if self
                .grid()
                .drawing_cell(pos)
                // pos.row is valid because we assume self.grid().pos() to
                // always have a valid row value. pos.col is valid because we
                // called col_wrap() immediately before this, which ensures
                // that self.grid().pos().col has a valid value.
                .unwrap()
                .is_wide()
            {
                let next_cell = self
                    .grid_mut()
                    .drawing_cell_mut(crate::terminal::grid::Pos {
                        row: pos.row,
                        col: pos.col + 1,
                    })
                    // pos.row is valid because we assume self.grid().pos() to
                    // always have a valid row value. pos.col is valid because
                    // we called col_wrap() immediately before this, which
                    // ensures that self.grid().pos().col has a valid value.
                    // pos.col + 1 is valid because the cell at pos.col is a
                    // wide character, so it must have the second half of the
                    // wide character after it.
                    .unwrap();
                next_cell.set(' ', attrs);
            }

            let cell = self
                .grid_mut()
                .drawing_cell_mut(pos)
                // pos.row is valid because we assume self.grid().pos() to
                // always have a valid row value. pos.col is valid because we
                // called col_wrap() immediately before this, which ensures
                // that self.grid().pos().col has a valid value.
                .unwrap();
            cell.set(c, attrs);
            self.grid_mut().col_inc(1);
            if width > 1 {
                let pos = self.grid().pos();
                if self
                    .grid()
                    .drawing_cell(pos)
                    // pos.row is valid because we assume self.grid().pos() to
                    // always have a valid row value. pos.col is valid because
                    // we called col_wrap() earlier, which ensures that
                    // self.grid().pos().col has a valid value. this is true
                    // even though we just called col_inc, because this branch
                    // only happens if width > 1, and col_wrap takes width
                    // into account.
                    .unwrap()
                    .is_wide()
                {
                    let next_next_pos = crate::terminal::grid::Pos {
                        row: pos.row,
                        col: pos.col + 1,
                    };
                    let next_next_cell = self
                        .grid_mut()
                        .drawing_cell_mut(next_next_pos)
                        // pos.row is valid because we assume
                        // self.grid().pos() to always have a valid row value.
                        // pos.col is valid because we called col_wrap()
                        // earlier, which ensures that self.grid().pos().col
                        // has a valid value. this is true even though we just
                        // called col_inc, because this branch only happens if
                        // width > 1, and col_wrap takes width into account.
                        // pos.col + 1 is valid because the cell at pos.col is
                        // wide, and so it must have the second half of the
                        // wide character after it.
                        .unwrap();
                    next_next_cell.clear(attrs);
                    if next_next_pos.col == size.cols - 1 {
                        self.grid_mut()
                            .drawing_row_mut(pos.row)
                            // we assume self.grid().pos().row is always valid
                            .unwrap()
                            .wrap(false);
                    }
                }
                let next_cell = self
                    .grid_mut()
                    .drawing_cell_mut(pos)
                    // pos.row is valid because we assume self.grid().pos() to
                    // always have a valid row value. pos.col is valid because
                    // we called col_wrap() earlier, which ensures that
                    // self.grid().pos().col has a valid value. this is true
                    // even though we just called col_inc, because this branch
                    // only happens if width > 1, and col_wrap takes width
                    // into account.
                    .unwrap();
                next_cell.clear(crate::terminal::attrs::Attrs::default());
                next_cell.set_wide_continuation(true);
                self.grid_mut().col_inc(1);
            }
        }
    }

    // control codes

    pub(crate) fn bs(&mut self) {
        self.grid_mut().col_dec(1);
    }

    pub(crate) fn tab(&mut self) {
        // shellglass: next tab stop after the cursor, else the last column
        // (the right margin), consulting the HTS/TBC-managed table. The
        // cursor column can sit at `cols` (wrap pending) — clamp before
        // building the range or it inverts and BTreeSet::range panics.
        let cols = self.grid().size().cols;
        let col = self.grid().pos().col.min(cols - 1);
        let next = self
            .tab_stops
            .range((col + 1)..cols)
            .next()
            .copied()
            .unwrap_or(cols - 1);
        self.grid_mut().col_set(next);
    }

    pub(crate) fn lf(&mut self) {
        self.grid_mut().row_inc_scroll(1);
    }

    pub(crate) fn vt(&mut self) {
        self.lf();
    }

    pub(crate) fn ff(&mut self) {
        self.lf();
    }

    pub(crate) fn cr(&mut self) {
        self.grid_mut().col_set(0);
    }

    // escape codes

    // ESC 7
    pub(crate) fn decsc(&mut self) {
        self.save_cursor();
    }

    // ESC 8
    pub(crate) fn decrc(&mut self) {
        self.restore_cursor();
    }

    // shellglass: ESC H (HTS) — set a tab stop at the cursor column.
    pub(crate) fn hts(&mut self) {
        let col = self.grid().pos().col;
        self.tab_stops.insert(col);
    }

    // ESC =
    pub(crate) fn deckpam(&mut self) {
        self.set_mode(MODE_APPLICATION_KEYPAD);
    }

    // ESC >
    pub(crate) fn deckpnm(&mut self) {
        self.clear_mode(MODE_APPLICATION_KEYPAD);
    }

    // ESC M
    pub(crate) fn ri(&mut self) {
        self.grid_mut().row_dec_scroll(1);
    }

    // ESC c
    pub(crate) fn ris(&mut self) {
        *self = Self::new(self.grid.size(), self.grid.scrollback_len());
    }

    // shellglass: CSI ! p (DECSTR, soft terminal reset) — restore the defined
    // subset of state without touching screen content or the cursor position:
    // SGR to normal, saved-cursor (DECSC) data cleared, scroll margins to the
    // full screen, origin mode off, autowrap on, insert→replace mode, cursor
    // visible, cursor keys and keypad to normal. Deliberately untouched,
    // matching xterm: content, cursor position, the alternate screen, tab
    // stops (only RIS resets those), and mouse tracking.
    pub(crate) fn decstr(&mut self) {
        self.attrs = crate::terminal::attrs::Attrs::default();
        self.saved_attrs = crate::terminal::attrs::Attrs::default();
        self.clear_mode(MODE_HIDE_CURSOR);
        self.clear_mode(MODE_APPLICATION_CURSOR);
        self.clear_mode(MODE_APPLICATION_KEYPAD);
        self.clear_mode(MODE_NO_AUTOWRAP);
        self.clear_mode(MODE_INSERT);
        self.cursor_style = 0; // xterm's DECSTR resets DECSCUSR too
        self.grid_mut().soft_reset();
    }

    // shellglass: CSI n SP q (DECSCUSR, set cursor style).
    pub(crate) fn decscusr(&mut self, style: u8) {
        self.cursor_style = style;
    }

    // csi codes

    // CSI @
    pub(crate) fn ich(&mut self, count: u16) {
        self.grid_mut().insert_cells(count);
    }

    // CSI A
    pub(crate) fn cuu(&mut self, offset: u16) {
        self.grid_mut().row_dec_clamp(offset);
    }

    // CSI B
    pub(crate) fn cud(&mut self, offset: u16) {
        self.grid_mut().row_inc_clamp(offset);
    }

    // CSI C
    pub(crate) fn cuf(&mut self, offset: u16) {
        self.grid_mut().col_inc_clamp(offset);
    }

    // CSI D
    pub(crate) fn cub(&mut self, offset: u16) {
        self.grid_mut().col_dec(offset);
    }

    // CSI E
    pub(crate) fn cnl(&mut self, offset: u16) {
        self.grid_mut().col_set(0);
        self.grid_mut().row_inc_clamp(offset);
    }

    // CSI F
    pub(crate) fn cpl(&mut self, offset: u16) {
        self.grid_mut().col_set(0);
        self.grid_mut().row_dec_clamp(offset);
    }

    // CSI G
    pub(crate) fn cha(&mut self, col: u16) {
        self.grid_mut().col_set(col - 1);
    }

    // CSI H
    pub(crate) fn cup(&mut self, (row, col): (u16, u16)) {
        self.grid_mut().set_pos(crate::terminal::grid::Pos {
            row: row - 1,
            col: col - 1,
        });
    }

    // CSI J
    pub(crate) fn ed(&mut self, mode: u16, mut unhandled: impl FnMut(&mut Self)) {
        let attrs = self.attrs;
        match mode {
            0 => self.grid_mut().erase_all_forward(attrs),
            1 => self.grid_mut().erase_all_backward(attrs),
            2 => self.grid_mut().erase_all(attrs),
            _ => unhandled(self),
        }
    }

    // CSI ? J
    pub(crate) fn decsed(&mut self, mode: u16, unhandled: impl FnMut(&mut Self)) {
        self.ed(mode, unhandled);
    }

    // CSI K
    pub(crate) fn el(&mut self, mode: u16, mut unhandled: impl FnMut(&mut Self)) {
        let attrs = self.attrs;
        match mode {
            0 => self.grid_mut().erase_row_forward(attrs),
            1 => self.grid_mut().erase_row_backward(attrs),
            2 => self.grid_mut().erase_row(attrs),
            _ => unhandled(self),
        }
    }

    // CSI ? K
    pub(crate) fn decsel(&mut self, mode: u16, unhandled: impl FnMut(&mut Self)) {
        self.el(mode, unhandled);
    }

    // CSI L
    pub(crate) fn il(&mut self, count: u16) {
        self.grid_mut().insert_lines(count);
    }

    // CSI M
    pub(crate) fn dl(&mut self, count: u16) {
        self.grid_mut().delete_lines(count);
    }

    // CSI P
    pub(crate) fn dch(&mut self, count: u16) {
        self.grid_mut().delete_cells(count);
    }

    // CSI S
    pub(crate) fn su(&mut self, count: u16) {
        self.grid_mut().scroll_up(count);
    }

    // CSI T
    pub(crate) fn sd(&mut self, count: u16) {
        self.grid_mut().scroll_down(count);
    }

    // CSI X
    pub(crate) fn ech(&mut self, count: u16) {
        let attrs = self.attrs;
        self.grid_mut().erase_cells(count, attrs);
    }

    // shellglass: CSI b (REP) — repeat the preceding graphic character. Going
    // back through the full print path buys wrapping and wide-character
    // handling for free. ncurses ≥ 6 emits this whenever terminfo advertises
    // `rep` (xterm-256color and xterm-kitty both do), so dropping it loses
    // real screen content.
    pub(crate) fn rep(&mut self, count: u16) {
        if let Some(c) = self.last_graphic_char {
            for _ in
                0..usize::from(count).min(usize::from(self.size().0) * usize::from(self.size().1))
            {
                self.text(c);
            }
        }
    }

    // shellglass: CSI I (CHT) — forward `count` tab stops.
    pub(crate) fn cht(&mut self, count: u16) {
        for _ in 0..count {
            self.tab();
        }
    }

    // shellglass: CSI Z (CBT) — back `count` tab stops (or column 0).
    pub(crate) fn cbt(&mut self, count: u16) {
        for _ in 0..count {
            let col = self.grid().pos().col;
            let prev = self
                .tab_stops
                .range(..col)
                .next_back()
                .copied()
                .unwrap_or(0);
            self.grid_mut().col_set(prev);
        }
    }

    // shellglass: CSI g (TBC) — clear the tab stop at the cursor (0) or all (3).
    pub(crate) fn tbc(&mut self, mode: u16) {
        match mode {
            0 => {
                let col = self.grid().pos().col;
                self.tab_stops.remove(&col);
            }
            3 => self.tab_stops.clear(),
            _ => {}
        }
    }

    // CSI d
    pub(crate) fn vpa(&mut self, row: u16) {
        self.grid_mut().row_set(row - 1);
    }

    // CSI ? h
    // shellglass: CSI h (SM) — only IRM is modeled; the rest keep reporting.
    pub(crate) fn sm(&mut self, params: &vte::Params, mut unhandled: impl FnMut(&mut Self)) {
        for param in params {
            match param {
                [4] => self.set_mode(MODE_INSERT),
                _ => unhandled(self),
            }
        }
    }

    // shellglass: CSI l (RM)
    pub(crate) fn rm(&mut self, params: &vte::Params, mut unhandled: impl FnMut(&mut Self)) {
        for param in params {
            match param {
                [4] => self.clear_mode(MODE_INSERT),
                // Claude Code emits bare `CSI 25 l` during teardown alongside
                // the real `CSI ? 25 h`. Mode 25 is only defined in DEC-private
                // form; terminals ignore the bare form, so do not report it as
                // a possible rendering gap.
                [25] => {}
                _ => unhandled(self),
            }
        }
    }

    pub(crate) fn decset(&mut self, params: &vte::Params, mut unhandled: impl FnMut(&mut Self)) {
        for param in params {
            match param {
                [1] => self.set_mode(MODE_APPLICATION_CURSOR),
                [6] => self.grid_mut().set_origin_mode(true),
                // shellglass: DECAWM — autowrap back on (the default)
                [7] => self.clear_mode(MODE_NO_AUTOWRAP),
                [9] => self.set_mouse_mode(MouseProtocolMode::Press),
                // Deliberately ignored: att610 cursor blink (12), focus
                // reporting (1004), color-scheme notifications (2031), urxvt
                // application-ESC (7727), and sixel display/scroll modes
                // (80/8452). SSHub does not implement these input protocols
                // or graphics; accepting the mode bytes is not advertising
                // support for them.
                [12 | 1004 | 2031 | 7727 | 80 | 8452] => {}
                [25] => self.clear_mode(MODE_HIDE_CURSOR),
                [47] => self.enter_alternate_grid(),
                [1000] => {
                    self.set_mouse_mode(MouseProtocolMode::PressRelease);
                }
                [1002] => {
                    self.set_mouse_mode(MouseProtocolMode::ButtonMotion);
                }
                [1003] => self.set_mouse_mode(MouseProtocolMode::AnyMotion),
                [1005] => {
                    self.set_mouse_encoding(MouseProtocolEncoding::Utf8);
                }
                [1006] => {
                    self.set_mouse_encoding(MouseProtocolEncoding::Sgr);
                }
                [1015] => {
                    self.set_mouse_encoding(MouseProtocolEncoding::Urxvt);
                }
                [1016] => {
                    self.set_mouse_encoding(MouseProtocolEncoding::SgrPixels);
                }
                [1049] => {
                    self.decsc();
                    self.alternate_grid.clear();
                    self.enter_alternate_grid();
                }
                [2004] => self.set_mode(MODE_BRACKETED_PASTE),
                // shellglass: BSU
                [2026] => {
                    self.set_mode(MODE_SYNCHRONIZED_UPDATE);
                    self.sync_starts = self.sync_starts.wrapping_add(1);
                }
                _ => unhandled(self),
            }
        }
    }

    // CSI ? l
    pub(crate) fn decrst(&mut self, params: &vte::Params, mut unhandled: impl FnMut(&mut Self)) {
        for param in params {
            match param {
                [1] => self.clear_mode(MODE_APPLICATION_CURSOR),
                [6] => self.grid_mut().set_origin_mode(false),
                // shellglass: DECAWM — autowrap off (clamp at the margin)
                [7] => self.set_mode(MODE_NO_AUTOWRAP),
                [9] => self.clear_mouse_mode(MouseProtocolMode::Press),
                // shellglass: blink / focus / scheme-notify / urxvt app-ESC /
                // sixel display+scroll (80, 8452) off — the deliberately-ignored
                // set, see decset
                [12 | 1004 | 2031 | 7727 | 80 | 8452] => {}
                [25] => self.set_mode(MODE_HIDE_CURSOR),
                [47] => {
                    self.exit_alternate_grid();
                }
                [1000] => {
                    self.clear_mouse_mode(MouseProtocolMode::PressRelease);
                }
                [1002] => {
                    self.clear_mouse_mode(MouseProtocolMode::ButtonMotion);
                }
                [1003] => {
                    self.clear_mouse_mode(MouseProtocolMode::AnyMotion);
                }
                [1005] => {
                    self.clear_mouse_encoding(MouseProtocolEncoding::Utf8);
                }
                [1006] => {
                    self.clear_mouse_encoding(MouseProtocolEncoding::Sgr);
                }
                [1015] => {
                    self.clear_mouse_encoding(MouseProtocolEncoding::Urxvt);
                }
                [1016] => {
                    self.clear_mouse_encoding(MouseProtocolEncoding::SgrPixels);
                }
                [1049] => {
                    self.exit_alternate_grid();
                    self.decrc();
                }
                [2004] => self.clear_mode(MODE_BRACKETED_PASTE),
                // shellglass: ESU
                [2026] => {
                    self.clear_mode(MODE_SYNCHRONIZED_UPDATE);
                    self.sync_ends = self.sync_ends.wrapping_add(1);
                }
                _ => unhandled(self),
            }
        }
    }

    // CSI m
    pub(crate) fn sgr(&mut self, params: &vte::Params, mut unhandled: impl FnMut(&mut Self)) {
        // XXX really i want to just be able to pass in a default Params
        // instance with a 0 in it, but vte doesn't allow creating new Params
        // instances
        if params.is_empty() {
            // shellglass: SGR resets don't close OSC 8 hyperlinks (they are
            // independent state; only OSC 8 ; ; ST closes one).
            self.attrs = crate::terminal::attrs::Attrs {
                link: self.attrs.link,
                ..crate::terminal::attrs::Attrs::default()
            };
            return;
        }

        let mut iter = params.iter();

        macro_rules! next_param {
            () => {
                match iter.next() {
                    Some(n) => n,
                    _ => return,
                }
            };
        }

        macro_rules! to_u8 {
            ($n:expr) => {
                if let Some(n) = u16_to_u8($n) {
                    n
                } else {
                    return;
                }
            };
        }

        macro_rules! next_param_u8 {
            () => {
                if let &[n] = next_param!() {
                    to_u8!(n)
                } else {
                    return;
                }
            };
        }

        loop {
            match next_param!() {
                // shellglass: preserving the OSC 8 link — see above
                [0] => {
                    self.attrs = crate::terminal::attrs::Attrs {
                        link: self.attrs.link,
                        ..crate::terminal::attrs::Attrs::default()
                    };
                }
                [1] => self.attrs.set_bold(),
                [2] => self.attrs.set_dim(),
                [3] => self.attrs.set_italic(true),
                [4] => self.attrs.set_underline(true),
                // shellglass: underline style subparam (4:0 none … 4:5
                // dashed) — helix/neovim emit 4:3 undercurl for diagnostics.
                [4, n] if *n <= 5 => {
                    self.attrs.set_underline_style(to_u8!(*n));
                }
                // shellglass: blink — 5 slow and 6 rapid share one bit (kitty
                // doesn't distinguish either)
                [5 | 6] => self.attrs.set_blink(true),
                [7] => self.attrs.set_inverse(true),
                // shellglass: conceal (ECMA-48 "hidden"; kitty ignores it,
                // most other terminals blank the glyph — see attrs.rs)
                [8] => self.attrs.set_concealed(true),
                // shellglass: strikethrough
                [9] => self.attrs.set_strikethrough(true),
                // shellglass: double underline (ECMA-48; kitty renders it)
                [21] => self.attrs.set_underline_style(2),
                [22] => self.attrs.set_normal_intensity(),
                [23] => self.attrs.set_italic(false),
                [24] => self.attrs.set_underline(false),
                // shellglass: blink off
                [25] => self.attrs.set_blink(false),
                [27] => self.attrs.set_inverse(false),
                // shellglass: conceal off (reveal)
                [28] => self.attrs.set_concealed(false),
                // shellglass: strikethrough off
                [29] => self.attrs.set_strikethrough(false),
                [n] if (30..=37).contains(n) => {
                    self.attrs.fgcolor = crate::terminal::Color::Idx(to_u8!(*n) - 30);
                }
                // shellglass: [_, r, g, b] is the colon form 38:2::r:g:b with an
                // (ignored) colorspace id, matching the 58 handling below.
                [38, 2, r, g, b] | [38, 2, _, r, g, b] => {
                    self.attrs.fgcolor =
                        crate::terminal::Color::Rgb(to_u8!(*r), to_u8!(*g), to_u8!(*b));
                }
                [38, 5, i] => {
                    self.attrs.fgcolor = crate::terminal::Color::Idx(to_u8!(*i));
                }
                [38] => match next_param!() {
                    [2] => {
                        let r = next_param_u8!();
                        let g = next_param_u8!();
                        let b = next_param_u8!();
                        self.attrs.fgcolor = crate::terminal::Color::Rgb(r, g, b);
                    }
                    [5] => {
                        self.attrs.fgcolor = crate::terminal::Color::Idx(next_param_u8!());
                    }
                    _ => {
                        unhandled(self);
                        return;
                    }
                },
                [39] => {
                    self.attrs.fgcolor = crate::terminal::Color::Default;
                }
                [n] if (40..=47).contains(n) => {
                    self.attrs.bgcolor = crate::terminal::Color::Idx(to_u8!(*n) - 40);
                }
                // shellglass: [_, r, g, b] is the colon form 48:2::r:g:b with an
                // (ignored) colorspace id, matching the 58 handling below.
                [48, 2, r, g, b] | [48, 2, _, r, g, b] => {
                    self.attrs.bgcolor =
                        crate::terminal::Color::Rgb(to_u8!(*r), to_u8!(*g), to_u8!(*b));
                }
                [48, 5, i] => {
                    self.attrs.bgcolor = crate::terminal::Color::Idx(to_u8!(*i));
                }
                [48] => match next_param!() {
                    [2] => {
                        let r = next_param_u8!();
                        let g = next_param_u8!();
                        let b = next_param_u8!();
                        self.attrs.bgcolor = crate::terminal::Color::Rgb(r, g, b);
                    }
                    [5] => {
                        self.attrs.bgcolor = crate::terminal::Color::Idx(next_param_u8!());
                    }
                    _ => {
                        unhandled(self);
                        return;
                    }
                },
                [49] => {
                    self.attrs.bgcolor = crate::terminal::Color::Default;
                }
                // shellglass: underline color (SGR 58/59), the same three
                // shapes as 38/48 plus the colon form with an (ignored)
                // colorspace id — kitty emits 58:2::r:g:b.
                [58, 2, r, g, b] | [58, 2, _, r, g, b] => {
                    self.attrs.ulcolor =
                        crate::terminal::Color::Rgb(to_u8!(*r), to_u8!(*g), to_u8!(*b));
                }
                [58, 5, i] => {
                    self.attrs.ulcolor = crate::terminal::Color::Idx(to_u8!(*i));
                }
                [58] => match next_param!() {
                    [2] => {
                        let r = next_param_u8!();
                        let g = next_param_u8!();
                        let b = next_param_u8!();
                        self.attrs.ulcolor = crate::terminal::Color::Rgb(r, g, b);
                    }
                    [5] => {
                        self.attrs.ulcolor = crate::terminal::Color::Idx(next_param_u8!());
                    }
                    _ => {
                        unhandled(self);
                        return;
                    }
                },
                [59] => {
                    self.attrs.ulcolor = crate::terminal::Color::Default;
                }
                [n] if (90..=97).contains(n) => {
                    self.attrs.fgcolor = crate::terminal::Color::Idx(to_u8!(*n) - 82);
                }
                [n] if (100..=107).contains(n) => {
                    self.attrs.bgcolor = crate::terminal::Color::Idx(to_u8!(*n) - 92);
                }
                _ => unhandled(self),
            }
        }
    }

    // CSI r
    pub(crate) fn decstbm(&mut self, (top, bottom): (u16, u16)) {
        self.grid_mut().set_scroll_region(top - 1, bottom - 1);
    }

    // shellglass: SCOSC/SCORC (ANSI.SYS/xterm save/restore cursor). Unlike
    // DECSC/DECRC (ESC 7/ESC 8) these save only the cursor state, not the
    // attributes; the save slot is shared with DECSC, as in xterm. Powerline
    // prompts use them to draw right-aligned segments.

    // CSI s
    pub(crate) fn scosc(&mut self) {
        self.grid_mut().save_cursor();
    }

    // CSI u
    pub(crate) fn scorc(&mut self) {
        self.grid_mut().restore_cursor();
    }
}

// shellglass: the power-on tab stops — every 8th column.
fn default_tab_stops(cols: u16) -> std::collections::BTreeSet<u16> {
    (8..cols).step_by(8).collect()
}

fn u16_to_u8(i: u16) -> Option<u8> {
    if i > u16::from(u8::MAX) {
        None
    } else {
        // safe because we just ensured that the value fits in a u8
        Some(i.try_into().unwrap())
    }
}

#[cfg(test)]
mod resource_bound_tests {
    use crate::terminal::Parser;

    #[test]
    fn rep_is_bounded_by_grid_area_even_for_maximum_count() {
        let mut parser = Parser::new(2, 3, 100);
        parser.process(b"X\x1b[65535b");
        assert_eq!(parser.screen().contents(), "XXXX");
        assert_eq!(parser.screen().cell(1, 0).unwrap().contents(), "X");
        assert!(!parser.screen().cell(1, 1).unwrap().has_contents());
        // Final cursor/content can alias after many wraps; retained history
        // distinguishes six repetitions from 65535, and fails on the fork.
        parser.screen_mut().set_scrollback(100);
        assert_eq!(parser.screen().scrollback(), 1);
        let mut small = Parser::new(1, 1, 100);
        small.process(b"X\x1b[65535b");
        assert_eq!(small.screen().contents(), "X");
    }

    #[test]
    fn long_links_are_rejected_and_saved_titles_have_bounded_lengths() {
        let mut parser = Parser::new(2, 3, 0);
        parser.screen_mut().link_open(&vec![b'x'; 2049]);
        assert!(parser.screen().links.is_empty());
        parser.screen_mut().link_open(&vec![b'x'; 2048]);
        assert_eq!(parser.screen().links.len(), 1);
        parser.screen_mut().link_open(&[0xff; 2048]);
        assert_eq!(parser.screen().links.len(), 1);
        assert!(parser.screen().attrs.link.is_none());
        for _ in 0..20 {
            parser.screen_mut().set_title(&vec![b'a'; 100_000]);
            parser.screen_mut().title_push();
        }
        assert_eq!(parser.screen().title_stack.len(), 16);
        assert!(parser
            .screen()
            .title_stack
            .iter()
            .all(|title| title.len() <= 4096));
        parser.screen_mut().set_title("😀".repeat(2000).as_bytes());
        assert!(parser.screen().title().len() <= 4096);
    }
}
