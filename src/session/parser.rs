//! VT100 parser wrapper. Maintains an in-memory `vt100::Screen` that the
//! renderer reads via `tui-term`.

pub struct ParserState {
    inner: vt100::Parser,
}

impl ParserState {
    pub fn new(rows: u16, cols: u16) -> Self {
        Self {
            inner: vt100::Parser::new(rows, cols, 10_000),
        }
    }

    pub fn process(&mut self, bytes: &[u8]) {
        self.inner.process(bytes);
    }

    pub fn set_size(&mut self, rows: u16, cols: u16) {
        self.inner.set_size(rows, cols);
    }

    pub fn screen(&self) -> &vt100::Screen {
        self.inner.screen()
    }

    /// Current scrollback offset (0 = pinned to bottom).
    pub fn scrollback(&self) -> usize {
        self.inner.screen().scrollback()
    }

    pub fn set_scrollback(&mut self, rows: usize) {
        // vt100 caps the value at `scrollback.len()` internally. Our vendored
        // 0.15.2 has the saturating-fix backported so any value up to the
        // full buffer (10k rows) is safe.
        self.inner.set_scrollback(rows);
    }

    /// Bump the scrollback offset up by `rows` (showing older content).
    pub fn scroll_up(&mut self, rows: usize) {
        let next = self.scrollback().saturating_add(rows);
        self.set_scrollback(next);
    }

    /// Reduce the scrollback offset by `rows` (toward the live view).
    pub fn scroll_down(&mut self, rows: usize) {
        let next = self.scrollback().saturating_sub(rows);
        self.set_scrollback(next);
    }

    pub fn snap_to_bottom(&mut self) {
        self.inner.set_scrollback(0);
    }

    /// Wipe the terminal buffer and scrollback. Used once ssh has
    /// authenticated so the `-v` handshake noise doesn't clutter the session.
    pub fn clear_buffer(&mut self) {
        let (rows, cols) = self.inner.screen().size();
        self.inner = vt100::Parser::new(rows, cols, 10_000);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parser_with(rows: u16, cols: u16, stream: &[u8]) -> ParserState {
        let mut p = ParserState::new(rows, cols);
        p.process(stream);
        p
    }

    /// Reproduces the bug: scrolling past the screen height used to panic
    /// (vt100 0.15.2 underflow). Vendored patch must keep it from crashing
    /// and must let us actually read older rows.
    #[test]
    fn scrollback_beyond_screen_height_does_not_panic() {
        // Print 100 numbered lines on a 10-row terminal.
        let mut bytes = Vec::new();
        for i in 1..=100 {
            bytes.extend_from_slice(format!("line-{i:03}\r\n").as_bytes());
        }
        let mut p = parser_with(10, 80, &bytes);

        // Way past one screen — would have panicked pre-patch.
        p.set_scrollback(60);
        assert_eq!(p.scrollback(), 60);

        // Top visible row should be ~50 rows back from "line-100".
        let first_visible_text: String = (0..10)
            .filter_map(|col| p.screen().cell(0, col).map(|c| c.contents()))
            .collect();
        assert!(
            first_visible_text.starts_with("line-"),
            "top row should be a numbered line, got {first_visible_text:?}"
        );
    }

    #[test]
    fn snap_returns_to_zero_offset() {
        let mut p = ParserState::new(10, 80);
        p.process(b"hello\r\n");
        p.set_scrollback(5);
        p.snap_to_bottom();
        assert_eq!(p.scrollback(), 0);
    }
}
