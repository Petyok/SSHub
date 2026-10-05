use unicode_width::UnicodeWidthChar as _;

// chosen to make the size of the cell struct 32 bytes upstream (44 with the
// shellglass underline-color + hyperlink attrs; per-cell data is `T`-sized
// on top of that)
const CONTENT_BYTES: usize = 22;

const IS_WIDE: u8 = 0b1000_0000;
const IS_WIDE_CONTINUATION: u8 = 0b0100_0000;
// shellglass: the data slot survives text written into this cell (see
// Cell::set_data). Rides a spare bit of `len` — the length itself needs 5 bits
// for at most CONTENT_BYTES, so no carry can reach this one.
const STICKY_DATA: u8 = 0b0010_0000;
const LEN_BITS: u8 = 0b0001_1111;

/// Represents a single terminal cell.
///
/// shellglass: generic over an optional per-cell data slot `T` (default `()`:
/// no slot, no overhead). Consumers stamp data with
/// [`Screen::place_data`](crate::terminal::Screen::place_data); the slot dies with the
/// cell's contents (overwrite/erase), so its lifetime rides the terminal's own
/// cell semantics — shellglass stores its inline-image overlay tag here.
/// A STICKY slot ([`PlaceOpts`](crate::terminal::PlaceOpts)) survives text written over
/// the cell and dies only on erase, which is how kitty graphics placements
/// behave (printing over one leaves the image alone) as opposed to a sixel,
/// which the print erases.
/// Equality deliberately ignores the slot: two cells that render identically
/// are equal; the data is consumer metadata, not part of the picture.
#[derive(Clone, Debug)]
pub struct Cell<T = ()> {
    contents: [u8; CONTENT_BYTES],
    len: u8,
    attrs: crate::terminal::attrs::Attrs,
    data: Option<T>,
}
const _: () = assert!(std::mem::size_of::<Cell<()>>() == 44);

impl<T> PartialEq<Self> for Cell<T> {
    fn eq(&self, other: &Self) -> bool {
        // shellglass: the sticky-data bit lives in `len` but is slot metadata,
        // not picture — mask it out so equality still ignores the slot.
        if self.len | STICKY_DATA != other.len | STICKY_DATA {
            return false;
        }
        if self.attrs != other.attrs {
            return false;
        }
        let len = self.len();
        self.contents[..len] == other.contents[..len]
    }
}

impl<T> Eq for Cell<T> {}

impl<T> Cell<T> {
    pub(crate) fn new() -> Self {
        Self {
            contents: Default::default(),
            len: 0,
            attrs: crate::terminal::attrs::Attrs::default(),
            data: None,
        }
    }

    fn len(&self) -> usize {
        usize::from(self.len & LEN_BITS)
    }

    pub(crate) fn set(&mut self, c: char, a: crate::terminal::attrs::Attrs) {
        // shellglass: overwriting text drops the data slot — unless the slot is
        // sticky, which outlives everything but an erase.
        let sticky = self.len & STICKY_DATA;
        self.len = sticky;
        if sticky == 0 {
            self.data = None;
        }
        self.append_char(0, c);
        // strings in this context should always be an arbitrary character
        // followed by zero or more zero-width characters, so we should only
        // have to look at the first character
        self.set_wide(c.width().unwrap_or(1) > 1);
        self.attrs = a;
    }

    pub(crate) fn append(&mut self, c: char) {
        let len = self.len();
        if len >= CONTENT_BYTES - 4 {
            return;
        }
        if len == 0 {
            self.contents[0] = b' ';
            self.len += 1;
        }

        // we already checked that we have space for another codepoint
        self.append_char(self.len(), c);
    }

    // Writes bytes representing c at start
    // Requires caller to verify start <= CODEPOINTS_IN_CELL * 4
    fn append_char(&mut self, start: usize, c: char) {
        c.encode_utf8(&mut self.contents[start..]);
        self.len += u8::try_from(c.len_utf8()).unwrap();
    }

    pub(crate) fn clear(&mut self, attrs: crate::terminal::attrs::Attrs) {
        // shellglass: a sticky slot outlives an erase of the cell's text, the
        // way kitty keeps a graphics placement through EL and a partial ED
        // (only a whole-display erase drops its images — see `drop_data`).
        let sticky = self.len & STICKY_DATA;
        self.len = sticky;
        self.attrs = attrs;
        // shellglass: an erased cell keeps drawing attrs (bg) but must never
        // be a clickable link, and erasing drops a non-sticky data slot.
        self.attrs.link = None;
        if sticky == 0 {
            self.data = None;
        }
    }

    // shellglass: drop the data slot no matter how sticky it is — the wipe a
    // whole-display erase performs (kitty's `grman_clear`).
    pub(crate) fn drop_data(&mut self) {
        self.len &= !STICKY_DATA;
        self.data = None;
    }

    /// shellglass: this cell's data slot, if a consumer stamped one (see
    /// [`Screen::place_data`](crate::terminal::Screen::place_data)).
    #[must_use]
    pub fn data(&self) -> Option<&T> {
        self.data.as_ref()
    }

    // shellglass: stamp (or clear) the data slot without touching the cell's
    // text or attributes — overlays draw *over* cells. `sticky` keeps the slot
    // through text written into the cell (an erase still drops it).
    pub(crate) fn set_data(&mut self, data: Option<T>, sticky: bool) {
        if sticky && data.is_some() {
            self.len |= STICKY_DATA;
        } else {
            self.len &= !STICKY_DATA;
        }
        self.data = data;
    }

    /// Returns the text contents of the cell.
    ///
    /// Can include multiple unicode characters if combining characters are
    /// used, but will contain at most one character with a non-zero character
    /// width.
    // Since contents has been constructed by appending chars encoded as UTF-8 it will be valid UTF-8
    #[allow(clippy::missing_panics_doc)]
    #[must_use]
    pub fn contents(&self) -> &str {
        std::str::from_utf8(&self.contents[..self.len()]).unwrap()
    }

    /// Returns whether the cell contains any text data.
    #[must_use]
    pub fn has_contents(&self) -> bool {
        self.len() > 0
    }

    /// Returns whether the text data in the cell represents a wide character.
    #[must_use]
    pub fn is_wide(&self) -> bool {
        self.len & IS_WIDE != 0
    }

    /// Returns whether the cell contains the second half of a wide character
    /// (in other words, whether the previous cell in the row contains a wide
    /// character)
    #[must_use]
    pub fn is_wide_continuation(&self) -> bool {
        self.len & IS_WIDE_CONTINUATION != 0
    }

    fn set_wide(&mut self, wide: bool) {
        if wide {
            self.len |= IS_WIDE;
        } else {
            self.len &= !IS_WIDE;
        }
    }

    pub(crate) fn set_wide_continuation(&mut self, wide: bool) {
        if wide {
            self.len |= IS_WIDE_CONTINUATION;
        } else {
            self.len &= !IS_WIDE_CONTINUATION;
        }
    }

    pub(crate) fn attrs(&self) -> &crate::terminal::attrs::Attrs {
        &self.attrs
    }

    /// Returns the foreground color of the cell.
    #[must_use]
    pub fn fgcolor(&self) -> crate::terminal::Color {
        self.attrs.fgcolor
    }

    /// Returns the background color of the cell.
    #[must_use]
    pub fn bgcolor(&self) -> crate::terminal::Color {
        self.attrs.bgcolor
    }

    /// Returns whether the cell should be rendered with the bold text
    /// attribute.
    #[must_use]
    pub fn bold(&self) -> bool {
        self.attrs.bold()
    }

    /// Returns whether the cell should be rendered with the dim text
    /// attribute.
    #[must_use]
    pub fn dim(&self) -> bool {
        self.attrs.dim()
    }

    /// Returns whether the cell should be rendered with the italic text
    /// attribute.
    #[must_use]
    pub fn italic(&self) -> bool {
        self.attrs.italic()
    }

    /// Returns whether the cell should be rendered with the underlined text
    /// attribute.
    #[must_use]
    pub fn underline(&self) -> bool {
        self.attrs.underline()
    }

    /// shellglass: the underline style — 0 none, 1 single, 2 double, 3 curly,
    /// 4 dotted, 5 dashed (SGR `4:n` / `21`, kitty's numbering).
    #[must_use]
    pub fn underline_style(&self) -> u8 {
        self.attrs.underline_style()
    }

    /// shellglass: whether the cell should be rendered struck through
    /// (SGR 9/29).
    #[must_use]
    pub fn strikethrough(&self) -> bool {
        self.attrs.strikethrough()
    }

    /// shellglass: the underline color (SGR 58/59); `Color::Default` means
    /// the underline follows the text color.
    #[must_use]
    pub fn ulcolor(&self) -> crate::terminal::Color {
        self.attrs.ulcolor
    }

    /// shellglass: the OSC 8 hyperlink id covering this cell, resolved via
    /// [`Screen::link_uri`](crate::terminal::Screen::link_uri).
    #[must_use]
    pub fn link(&self) -> Option<std::num::NonZeroU32> {
        self.attrs.link
    }

    /// Returns whether the cell should be rendered with the inverse text
    /// attribute.
    #[must_use]
    pub fn inverse(&self) -> bool {
        self.attrs.inverse()
    }

    /// Returns whether the cell is concealed (SGR 8/28) — the contents stay
    /// in the buffer, but a renderer must not draw the glyph.
    // shellglass
    #[must_use]
    pub fn concealed(&self) -> bool {
        self.attrs.concealed()
    }

    /// Returns whether the cell should be rendered blinking (SGR 5/6, 25 off).
    // shellglass
    #[must_use]
    pub fn blink(&self) -> bool {
        self.attrs.blink()
    }
}
