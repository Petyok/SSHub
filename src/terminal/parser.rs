/// A parser for terminal output which produces an in-memory representation of
/// the terminal contents.
///
/// shellglass: generic over the per-cell data slot `T` too (default `()`);
/// see [`Cell`](crate::terminal::Cell) and [`Screen::place_data`](crate::terminal::Screen).
pub struct Parser<CB: crate::terminal::callbacks::Callbacks<T> = (), T = ()> {
    // Fixed OSC buffer: enough for the permitted 64 KiB base64 clipboard
    // payload, while unterminated OSC strings cannot retain unbounded memory.
    parser: Box<vte::Parser<{ 96 * 1024 }>>,
    screen: crate::terminal::perform::WrappedScreen<CB, T>,
}

impl Parser {
    /// Creates a new terminal parser of the given size and with the given
    /// amount of scrollback.
    #[must_use]
    pub fn new(rows: u16, cols: u16, scrollback_len: usize) -> Self {
        Self {
            parser: Box::new(vte::Parser::new_with_size()),
            screen: crate::terminal::perform::WrappedScreen::new(rows, cols, scrollback_len),
        }
    }
}

impl<CB: crate::terminal::callbacks::Callbacks<T>, T> Parser<CB, T> {
    /// Creates a new terminal parser of the given size and with the given
    /// amount of scrollback. Terminal events will be reported via method
    /// calls on the provided [`Callbacks`](crate::terminal::callbacks::Callbacks)
    /// implementation.
    ///
    /// shellglass: also generic over the cell-data type `T` — annotate the
    /// parser type (or the screen use) when stamping data, e.g.
    /// `let p: sshub::terminal::Parser<CB, MyTag> = sshub::terminal::Parser::new_with_callbacks(…)`.
    pub fn new_with_callbacks(rows: u16, cols: u16, scrollback_len: usize, callbacks: CB) -> Self {
        Self {
            parser: Box::new(vte::Parser::new_with_size()),
            screen: crate::terminal::perform::WrappedScreen::new_with_callbacks(
                rows,
                cols,
                scrollback_len,
                callbacks,
            ),
        }
    }

    /// Processes the contents of the given byte string, and updates the
    /// in-memory terminal state.
    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.screen, bytes);
    }

    /// Returns a reference to a [`Screen`](crate::terminal::Screen) object containing
    /// the terminal state.
    #[must_use]
    pub fn screen(&self) -> &crate::terminal::Screen<T> {
        &self.screen.screen
    }

    /// Returns a mutable reference to a [`Screen`](crate::terminal::Screen) object
    /// containing the terminal state.
    #[must_use]
    pub fn screen_mut(&mut self) -> &mut crate::terminal::Screen<T> {
        &mut self.screen.screen
    }

    /// Returns a reference to the [`Callbacks`](crate::terminal::callbacks::Callbacks)
    /// state object passed into the constructor.
    pub fn callbacks(&self) -> &CB {
        &self.screen.callbacks
    }

    /// Returns a mutable reference to the
    /// [`Callbacks`](crate::terminal::callbacks::Callbacks) state object passed into
    /// the constructor.
    pub fn callbacks_mut(&mut self) -> &mut CB {
        &mut self.screen.callbacks
    }
}

impl Default for Parser {
    /// Returns a parser with dimensions 80x24 and no scrollback.
    fn default() -> Self {
        Self::new(24, 80, 0)
    }
}

impl std::io::Write for Parser {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.process(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
