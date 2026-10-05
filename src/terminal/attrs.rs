use crate::terminal::term::BufWrite as _;

/// Represents a foreground or background color for cells.
#[derive(Eq, PartialEq, Debug, Copy, Clone, Default)]
pub enum Color {
    /// The default terminal color.
    #[default]
    Default,

    /// An indexed terminal color.
    Idx(u8),

    /// An RGB terminal color. The parameters are (red, green, blue).
    Rgb(u8, u8, u8),
}

// shellglass: widened to u16 when conceal landed — the low byte was full.
const TEXT_MODE_INTENSITY: u16 = 0b0000_0011;
const TEXT_MODE_BOLD: u16 = 0b0000_0001;
const TEXT_MODE_DIM: u16 = 0b0000_0010;
const TEXT_MODE_ITALIC: u16 = 0b0000_0100;
// shellglass: upstream's underline bit became strikethrough; underline moved
// to the 3-bit style field below (0 = no underline).
const TEXT_MODE_STRIKETHROUGH: u16 = 0b0000_1000;
const TEXT_MODE_INVERSE: u16 = 0b0001_0000;
// shellglass: underline style (SGR 4:n / 21 / 24), kitty's numbering — 0 none,
// 1 single, 2 double, 3 curly, 4 dotted, 5 dashed.
const TEXT_MODE_UNDERLINE_SHIFT: u16 = 5;
const TEXT_MODE_UNDERLINE: u16 = 0b1110_0000;
// shellglass: conceal (SGR 8/28) — ECMA-48 "hidden"; most terminals blank the
// glyph (xterm, foot, alacritty, wezterm). kitty ignores SGR 8 entirely (its
// SGR table has no case 8), a documented deviation we do NOT copy: the mirror
// showing text a concealing terminal hides is a content leak.
const TEXT_MODE_CONCEALED: u16 = 0b1_0000_0000;
// shellglass: blink (SGR 5/6 set — one bit, rapid isn't distinguished, same
// as kitty — SGR 25 clears). kitty renders blinking text (cursor.c: S(blink)).
const TEXT_MODE_BLINK: u16 = 0b10_0000_0000;

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Attrs {
    pub fgcolor: Color,
    pub bgcolor: Color,
    // shellglass: underline color (SGR 58/59); Default = follow the text color.
    pub ulcolor: Color,
    // shellglass: OSC 8 hyperlink id, resolved through the Screen's link
    // table. Rides the attrs because OSC 8 is SGR-like state (everything
    // printed while a link is open carries it) — but it is NOT reset by
    // SGR 0 (hyperlinks are independent of SGR; see sgr()/decstr()), and
    // Cell::clear strips it so erased cells are never clickable.
    pub link: Option<std::num::NonZeroU32>,
    pub mode: u16,
}

impl Attrs {
    pub fn bold(&self) -> bool {
        self.mode & TEXT_MODE_BOLD != 0
    }

    pub fn dim(&self) -> bool {
        self.mode & TEXT_MODE_DIM != 0
    }

    fn intensity(&self) -> u16 {
        self.mode & TEXT_MODE_INTENSITY
    }

    pub fn set_bold(&mut self) {
        self.mode &= !TEXT_MODE_INTENSITY;
        self.mode |= TEXT_MODE_BOLD;
    }

    pub fn set_dim(&mut self) {
        self.mode &= !TEXT_MODE_INTENSITY;
        self.mode |= TEXT_MODE_DIM;
    }

    pub fn set_normal_intensity(&mut self) {
        self.mode &= !TEXT_MODE_INTENSITY;
    }

    pub fn italic(&self) -> bool {
        self.mode & TEXT_MODE_ITALIC != 0
    }

    pub fn set_italic(&mut self, italic: bool) {
        if italic {
            self.mode |= TEXT_MODE_ITALIC;
        } else {
            self.mode &= !TEXT_MODE_ITALIC;
        }
    }

    pub fn underline(&self) -> bool {
        self.underline_style() != 0
    }

    pub fn set_underline(&mut self, underline: bool) {
        self.set_underline_style(u8::from(underline));
    }

    // shellglass: 0 none, 1 single, 2 double, 3 curly, 4 dotted, 5 dashed.
    pub fn underline_style(&self) -> u8 {
        u8::try_from((self.mode & TEXT_MODE_UNDERLINE) >> TEXT_MODE_UNDERLINE_SHIFT)
            .expect("3-bit field")
    }

    pub fn set_underline_style(&mut self, style: u8) {
        debug_assert!(style <= 5);
        self.mode &= !TEXT_MODE_UNDERLINE;
        self.mode |= (u16::from(style) << TEXT_MODE_UNDERLINE_SHIFT) & TEXT_MODE_UNDERLINE;
    }

    // shellglass: strikethrough (SGR 9/29).
    pub fn strikethrough(&self) -> bool {
        self.mode & TEXT_MODE_STRIKETHROUGH != 0
    }

    pub fn set_strikethrough(&mut self, strikethrough: bool) {
        if strikethrough {
            self.mode |= TEXT_MODE_STRIKETHROUGH;
        } else {
            self.mode &= !TEXT_MODE_STRIKETHROUGH;
        }
    }

    pub fn inverse(&self) -> bool {
        self.mode & TEXT_MODE_INVERSE != 0
    }

    pub fn set_inverse(&mut self, inverse: bool) {
        if inverse {
            self.mode |= TEXT_MODE_INVERSE;
        } else {
            self.mode &= !TEXT_MODE_INVERSE;
        }
    }

    // shellglass: conceal (SGR 8/28).
    pub fn concealed(&self) -> bool {
        self.mode & TEXT_MODE_CONCEALED != 0
    }

    pub fn set_concealed(&mut self, concealed: bool) {
        if concealed {
            self.mode |= TEXT_MODE_CONCEALED;
        } else {
            self.mode &= !TEXT_MODE_CONCEALED;
        }
    }

    // shellglass: blink (SGR 5/6, 25 off).
    pub fn blink(&self) -> bool {
        self.mode & TEXT_MODE_BLINK != 0
    }

    pub fn set_blink(&mut self, blink: bool) {
        if blink {
            self.mode |= TEXT_MODE_BLINK;
        } else {
            self.mode &= !TEXT_MODE_BLINK;
        }
    }

    pub fn write_escape_code_diff(&self, contents: &mut Vec<u8>, other: &Self) {
        if self != other && self == &Self::default() {
            crate::terminal::term::ClearAttrs.write_buf(contents);
            return;
        }

        let attrs = crate::terminal::term::Attrs::default();

        let attrs = if self.fgcolor == other.fgcolor {
            attrs
        } else {
            attrs.fgcolor(self.fgcolor)
        };
        let attrs = if self.bgcolor == other.bgcolor {
            attrs
        } else {
            attrs.bgcolor(self.bgcolor)
        };
        let attrs = if self.intensity() == other.intensity() {
            attrs
        } else {
            attrs.intensity(match self.intensity() {
                0 => crate::terminal::term::Intensity::Normal,
                TEXT_MODE_BOLD => crate::terminal::term::Intensity::Bold,
                TEXT_MODE_DIM => crate::terminal::term::Intensity::Dim,
                _ => unreachable!(),
            })
        };
        let attrs = if self.italic() == other.italic() {
            attrs
        } else {
            attrs.italic(self.italic())
        };
        // shellglass: underline rides its style (upstream's bool is gone).
        let attrs = if self.underline_style() == other.underline_style() {
            attrs
        } else {
            attrs.underline_style(self.underline_style())
        };
        let attrs = if self.strikethrough() == other.strikethrough() {
            attrs
        } else {
            attrs.strikethrough(self.strikethrough())
        };
        let attrs = if self.ulcolor == other.ulcolor {
            attrs
        } else {
            attrs.ulcolor(self.ulcolor)
        };
        let attrs = if self.inverse() == other.inverse() {
            attrs
        } else {
            attrs.inverse(self.inverse())
        };
        // shellglass: conceal
        let attrs = if self.concealed() == other.concealed() {
            attrs
        } else {
            attrs.concealed(self.concealed())
        };
        // shellglass: blink
        let attrs = if self.blink() == other.blink() {
            attrs
        } else {
            attrs.blink(self.blink())
        };

        attrs.write_buf(contents);
    }
}
