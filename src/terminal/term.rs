// TODO: read all of this from terminfo

pub trait BufWrite {
    fn write_buf(&self, buf: &mut Vec<u8>);
}

#[derive(Default, Debug)]
#[must_use = "this struct does nothing unless you call write_buf"]
pub struct ClearScreen;

impl BufWrite for ClearScreen {
    fn write_buf(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(b"\x1b[H\x1b[J");
    }
}

#[derive(Default, Debug)]
#[must_use = "this struct does nothing unless you call write_buf"]
pub struct ClearRowForward;

impl BufWrite for ClearRowForward {
    fn write_buf(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(b"\x1b[K");
    }
}

#[derive(Default, Debug)]
#[must_use = "this struct does nothing unless you call write_buf"]
pub struct Crlf;

impl BufWrite for Crlf {
    fn write_buf(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(b"\r\n");
    }
}

#[derive(Default, Debug)]
#[must_use = "this struct does nothing unless you call write_buf"]
pub struct Backspace;

impl BufWrite for Backspace {
    fn write_buf(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(b"\x08");
    }
}

#[derive(Default, Debug)]
#[must_use = "this struct does nothing unless you call write_buf"]
pub struct SaveCursor;

impl BufWrite for SaveCursor {
    fn write_buf(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(b"\x1b7");
    }
}

#[derive(Default, Debug)]
#[must_use = "this struct does nothing unless you call write_buf"]
pub struct RestoreCursor;

impl BufWrite for RestoreCursor {
    fn write_buf(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(b"\x1b8");
    }
}

#[derive(Default, Debug)]
#[must_use = "this struct does nothing unless you call write_buf"]
pub struct MoveTo {
    row: u16,
    col: u16,
}

impl MoveTo {
    pub fn new(pos: crate::terminal::grid::Pos) -> Self {
        Self {
            row: pos.row,
            col: pos.col,
        }
    }
}

impl BufWrite for MoveTo {
    fn write_buf(&self, buf: &mut Vec<u8>) {
        if self.row == 0 && self.col == 0 {
            buf.extend_from_slice(b"\x1b[H");
        } else {
            buf.extend_from_slice(b"\x1b[");
            extend_itoa(buf, self.row + 1);
            buf.push(b';');
            extend_itoa(buf, self.col + 1);
            buf.push(b'H');
        }
    }
}

#[derive(Default, Debug)]
#[must_use = "this struct does nothing unless you call write_buf"]
pub struct ClearAttrs;

impl BufWrite for ClearAttrs {
    fn write_buf(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(b"\x1b[m");
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Intensity {
    Normal,
    Bold,
    Dim,
}

#[derive(Default, Debug)]
#[must_use = "this struct does nothing unless you call write_buf"]
pub struct Attrs {
    fgcolor: Option<crate::terminal::Color>,
    bgcolor: Option<crate::terminal::Color>,
    // shellglass: underline color (SGR 58/59); Default = reset (59).
    ulcolor: Option<crate::terminal::Color>,
    intensity: Option<Intensity>,
    italic: Option<bool>,
    // shellglass: upstream's underline bool became the SGR 4:n style value.
    underline_style: Option<u8>,
    strikethrough: Option<bool>,
    inverse: Option<bool>,
    // shellglass: conceal (SGR 8/28)
    concealed: Option<bool>,
    // shellglass: blink (SGR 5, 25 off)
    blink: Option<bool>,
}

impl Attrs {
    pub fn fgcolor(mut self, fgcolor: crate::terminal::Color) -> Self {
        self.fgcolor = Some(fgcolor);
        self
    }

    pub fn bgcolor(mut self, bgcolor: crate::terminal::Color) -> Self {
        self.bgcolor = Some(bgcolor);
        self
    }

    pub fn intensity(mut self, intensity: Intensity) -> Self {
        self.intensity = Some(intensity);
        self
    }

    pub fn italic(mut self, italic: bool) -> Self {
        self.italic = Some(italic);
        self
    }

    // shellglass: 0 none, 1 single, 2 double, 3 curly, 4 dotted, 5 dashed.
    pub fn underline_style(mut self, style: u8) -> Self {
        self.underline_style = Some(style);
        self
    }

    // shellglass
    pub fn strikethrough(mut self, strikethrough: bool) -> Self {
        self.strikethrough = Some(strikethrough);
        self
    }

    // shellglass
    pub fn ulcolor(mut self, ulcolor: crate::terminal::Color) -> Self {
        self.ulcolor = Some(ulcolor);
        self
    }

    pub fn inverse(mut self, inverse: bool) -> Self {
        self.inverse = Some(inverse);
        self
    }

    // shellglass
    pub fn concealed(mut self, concealed: bool) -> Self {
        self.concealed = Some(concealed);
        self
    }

    // shellglass
    pub fn blink(mut self, blink: bool) -> Self {
        self.blink = Some(blink);
        self
    }
}

impl BufWrite for Attrs {
    #[allow(unused_assignments)]
    #[allow(clippy::branches_sharing_code)]
    fn write_buf(&self, buf: &mut Vec<u8>) {
        if self.fgcolor.is_none()
            && self.bgcolor.is_none()
            && self.ulcolor.is_none()
            && self.intensity.is_none()
            && self.italic.is_none()
            && self.underline_style.is_none()
            && self.strikethrough.is_none()
            && self.inverse.is_none()
            && self.concealed.is_none()
            && self.blink.is_none()
        {
            return;
        }

        buf.extend_from_slice(b"\x1b[");
        let mut first = true;

        macro_rules! write_param {
            ($i:expr) => {{
                if first {
                    first = false;
                } else {
                    buf.push(b';');
                }
                extend_itoa(buf, $i);
            }};
        }

        if let Some(fgcolor) = self.fgcolor {
            match fgcolor {
                crate::terminal::Color::Default => {
                    write_param!(39);
                }
                crate::terminal::Color::Idx(i) => {
                    if i < 8 {
                        write_param!(i + 30);
                    } else if i < 16 {
                        write_param!(i + 82);
                    } else {
                        write_param!(38);
                        write_param!(5);
                        write_param!(i);
                    }
                }
                crate::terminal::Color::Rgb(r, g, b) => {
                    write_param!(38);
                    write_param!(2);
                    write_param!(r);
                    write_param!(g);
                    write_param!(b);
                }
            }
        }

        if let Some(bgcolor) = self.bgcolor {
            match bgcolor {
                crate::terminal::Color::Default => {
                    write_param!(49);
                }
                crate::terminal::Color::Idx(i) => {
                    if i < 8 {
                        write_param!(i + 40);
                    } else if i < 16 {
                        write_param!(i + 92);
                    } else {
                        write_param!(48);
                        write_param!(5);
                        write_param!(i);
                    }
                }
                crate::terminal::Color::Rgb(r, g, b) => {
                    write_param!(48);
                    write_param!(2);
                    write_param!(r);
                    write_param!(g);
                    write_param!(b);
                }
            }
        }

        if let Some(intensity) = self.intensity {
            match intensity {
                Intensity::Normal => write_param!(22),
                Intensity::Bold => write_param!(1),
                Intensity::Dim => write_param!(2),
            }
        }

        if let Some(italic) = self.italic {
            if italic {
                write_param!(3);
            } else {
                write_param!(23);
            }
        }

        // shellglass: single underline stays the maximally-compatible `4`;
        // fancier styles ride the kitty/xterm `4:n` subparam form, off is `24`.
        if let Some(style) = self.underline_style {
            match style {
                0 => write_param!(24),
                1 => write_param!(4),
                n => {
                    write_param!(4);
                    buf.push(b':');
                    extend_itoa(buf, n);
                }
            }
        }

        // shellglass
        if let Some(strikethrough) = self.strikethrough {
            if strikethrough {
                write_param!(9);
            } else {
                write_param!(29);
            }
        }

        // shellglass: conceal — re-emitted so the SSH view's real terminal
        // applies its own hiding.
        if let Some(concealed) = self.concealed {
            if concealed {
                write_param!(8);
            } else {
                write_param!(28);
            }
        }

        // shellglass: blink — re-emitted so the SSH view's real terminal
        // animates it itself.
        if let Some(blink) = self.blink {
            if blink {
                write_param!(5);
            } else {
                write_param!(25);
            }
        }

        // shellglass: underline color has no compact idx-form shorthands.
        if let Some(ulcolor) = self.ulcolor {
            match ulcolor {
                crate::terminal::Color::Default => write_param!(59),
                crate::terminal::Color::Idx(i) => {
                    write_param!(58);
                    write_param!(5);
                    write_param!(i);
                }
                crate::terminal::Color::Rgb(r, g, b) => {
                    write_param!(58);
                    write_param!(2);
                    write_param!(r);
                    write_param!(g);
                    write_param!(b);
                }
            }
        }

        if let Some(inverse) = self.inverse {
            if inverse {
                write_param!(7);
            } else {
                write_param!(27);
            }
        }

        buf.push(b'm');
    }
}

#[derive(Debug)]
#[must_use = "this struct does nothing unless you call write_buf"]
pub struct MoveRight {
    count: u16,
}

impl MoveRight {
    pub fn new(count: u16) -> Self {
        Self { count }
    }
}

impl Default for MoveRight {
    fn default() -> Self {
        Self { count: 1 }
    }
}

impl BufWrite for MoveRight {
    fn write_buf(&self, buf: &mut Vec<u8>) {
        match self.count {
            0 => {}
            1 => buf.extend_from_slice(b"\x1b[C"),
            n => {
                buf.extend_from_slice(b"\x1b[");
                extend_itoa(buf, n);
                buf.push(b'C');
            }
        }
    }
}

#[derive(Debug)]
#[must_use = "this struct does nothing unless you call write_buf"]
pub struct EraseChar {
    count: u16,
}

impl EraseChar {
    pub fn new(count: u16) -> Self {
        Self { count }
    }
}

impl Default for EraseChar {
    fn default() -> Self {
        Self { count: 1 }
    }
}

impl BufWrite for EraseChar {
    fn write_buf(&self, buf: &mut Vec<u8>) {
        match self.count {
            0 => {}
            1 => buf.extend_from_slice(b"\x1b[X"),
            n => {
                buf.extend_from_slice(b"\x1b[");
                extend_itoa(buf, n);
                buf.push(b'X');
            }
        }
    }
}

#[derive(Default, Debug)]
#[must_use = "this struct does nothing unless you call write_buf"]
pub struct HideCursor {
    state: bool,
}

impl HideCursor {
    pub fn new(state: bool) -> Self {
        Self { state }
    }
}

impl BufWrite for HideCursor {
    fn write_buf(&self, buf: &mut Vec<u8>) {
        if self.state {
            buf.extend_from_slice(b"\x1b[?25l");
        } else {
            buf.extend_from_slice(b"\x1b[?25h");
        }
    }
}

#[derive(Debug)]
#[must_use = "this struct does nothing unless you call write_buf"]
pub struct MoveFromTo {
    from: crate::terminal::grid::Pos,
    to: crate::terminal::grid::Pos,
}

impl MoveFromTo {
    pub fn new(from: crate::terminal::grid::Pos, to: crate::terminal::grid::Pos) -> Self {
        Self { from, to }
    }
}

impl BufWrite for MoveFromTo {
    fn write_buf(&self, buf: &mut Vec<u8>) {
        if self.to.row == self.from.row + 1 && self.to.col == 0 {
            crate::terminal::term::Crlf.write_buf(buf);
        } else if self.from.row == self.to.row && self.from.col < self.to.col {
            crate::terminal::term::MoveRight::new(self.to.col - self.from.col).write_buf(buf);
        } else if self.to != self.from {
            crate::terminal::term::MoveTo::new(self.to).write_buf(buf);
        }
    }
}

#[derive(Default, Debug)]
#[must_use = "this struct does nothing unless you call write_buf"]
pub struct ApplicationKeypad {
    state: bool,
}

impl ApplicationKeypad {
    pub fn new(state: bool) -> Self {
        Self { state }
    }
}

impl BufWrite for ApplicationKeypad {
    fn write_buf(&self, buf: &mut Vec<u8>) {
        if self.state {
            buf.extend_from_slice(b"\x1b=");
        } else {
            buf.extend_from_slice(b"\x1b>");
        }
    }
}

#[derive(Default, Debug)]
#[must_use = "this struct does nothing unless you call write_buf"]
pub struct ApplicationCursor {
    state: bool,
}

impl ApplicationCursor {
    pub fn new(state: bool) -> Self {
        Self { state }
    }
}

impl BufWrite for ApplicationCursor {
    fn write_buf(&self, buf: &mut Vec<u8>) {
        if self.state {
            buf.extend_from_slice(b"\x1b[?1h");
        } else {
            buf.extend_from_slice(b"\x1b[?1l");
        }
    }
}

#[derive(Default, Debug)]
#[must_use = "this struct does nothing unless you call write_buf"]
pub struct BracketedPaste {
    state: bool,
}

impl BracketedPaste {
    pub fn new(state: bool) -> Self {
        Self { state }
    }
}

impl BufWrite for BracketedPaste {
    fn write_buf(&self, buf: &mut Vec<u8>) {
        if self.state {
            buf.extend_from_slice(b"\x1b[?2004h");
        } else {
            buf.extend_from_slice(b"\x1b[?2004l");
        }
    }
}

#[derive(Default, Debug)]
#[must_use = "this struct does nothing unless you call write_buf"]
pub struct MouseProtocolMode {
    mode: crate::terminal::MouseProtocolMode,
    prev: crate::terminal::MouseProtocolMode,
}

impl MouseProtocolMode {
    pub fn new(
        mode: crate::terminal::MouseProtocolMode,
        prev: crate::terminal::MouseProtocolMode,
    ) -> Self {
        Self { mode, prev }
    }
}

impl BufWrite for MouseProtocolMode {
    fn write_buf(&self, buf: &mut Vec<u8>) {
        if self.mode == self.prev {
            return;
        }

        match self.mode {
            crate::terminal::MouseProtocolMode::None => match self.prev {
                crate::terminal::MouseProtocolMode::None => {}
                crate::terminal::MouseProtocolMode::Press => {
                    buf.extend_from_slice(b"\x1b[?9l");
                }
                crate::terminal::MouseProtocolMode::PressRelease => {
                    buf.extend_from_slice(b"\x1b[?1000l");
                }
                crate::terminal::MouseProtocolMode::ButtonMotion => {
                    buf.extend_from_slice(b"\x1b[?1002l");
                }
                crate::terminal::MouseProtocolMode::AnyMotion => {
                    buf.extend_from_slice(b"\x1b[?1003l");
                }
            },
            crate::terminal::MouseProtocolMode::Press => {
                buf.extend_from_slice(b"\x1b[?9h");
            }
            crate::terminal::MouseProtocolMode::PressRelease => {
                buf.extend_from_slice(b"\x1b[?1000h");
            }
            crate::terminal::MouseProtocolMode::ButtonMotion => {
                buf.extend_from_slice(b"\x1b[?1002h");
            }
            crate::terminal::MouseProtocolMode::AnyMotion => {
                buf.extend_from_slice(b"\x1b[?1003h");
            }
        }
    }
}

#[derive(Default, Debug)]
#[must_use = "this struct does nothing unless you call write_buf"]
pub struct MouseProtocolEncoding {
    encoding: crate::terminal::MouseProtocolEncoding,
    prev: crate::terminal::MouseProtocolEncoding,
}

impl MouseProtocolEncoding {
    pub fn new(
        encoding: crate::terminal::MouseProtocolEncoding,
        prev: crate::terminal::MouseProtocolEncoding,
    ) -> Self {
        Self { encoding, prev }
    }
}

impl BufWrite for MouseProtocolEncoding {
    fn write_buf(&self, buf: &mut Vec<u8>) {
        if self.encoding == self.prev {
            return;
        }

        match self.encoding {
            crate::terminal::MouseProtocolEncoding::Default => match self.prev {
                crate::terminal::MouseProtocolEncoding::Default => {}
                crate::terminal::MouseProtocolEncoding::Utf8 => {
                    buf.extend_from_slice(b"\x1b[?1005l");
                }
                crate::terminal::MouseProtocolEncoding::Sgr => {
                    buf.extend_from_slice(b"\x1b[?1006l");
                }
                crate::terminal::MouseProtocolEncoding::Urxvt => {
                    buf.extend_from_slice(b"\x1b[?1015l");
                }
                crate::terminal::MouseProtocolEncoding::SgrPixels => {
                    buf.extend_from_slice(b"\x1b[?1016l");
                }
            },
            crate::terminal::MouseProtocolEncoding::Utf8 => {
                buf.extend_from_slice(b"\x1b[?1005h");
            }
            crate::terminal::MouseProtocolEncoding::Sgr => {
                buf.extend_from_slice(b"\x1b[?1006h");
            }
            crate::terminal::MouseProtocolEncoding::Urxvt => {
                buf.extend_from_slice(b"\x1b[?1015h");
            }
            crate::terminal::MouseProtocolEncoding::SgrPixels => {
                buf.extend_from_slice(b"\x1b[?1016h");
            }
        }
    }
}

fn extend_itoa<I: itoa::Integer>(buf: &mut Vec<u8>, i: I) {
    let mut itoa_buf = itoa::Buffer::new();
    buf.extend_from_slice(itoa_buf.format(i).as_bytes());
}
