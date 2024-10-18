use unicode_width::UnicodeWidthChar as _;

const CODEPOINTS_IN_CELL: usize = 6;
const BYTES_IN_CHAR: usize = 4;

/// Represents a single terminal cell.
#[derive(Clone, Debug, Eq)]
pub struct Cell {
    contents: [u8; CODEPOINTS_IN_CELL * BYTES_IN_CHAR],
    // Number of bytes needed to represent all characters
    num_bytes: u8,
    // Length of characters
    len: u8,
    attrs: crate::attrs::Attrs,
    selected: bool,
}

impl PartialEq<Self> for Cell {
    fn eq(&self, other: &Self) -> bool {
        if self.len != other.len || self.num_bytes != other.num_bytes {
            return false;
        }
        if self.attrs != other.attrs {
            return false;
        }
        let num_bytes = self.num_bytes();
        // self.len() always returns a valid value
        self.contents[..num_bytes] == other.contents[..num_bytes]
    }
}

impl Cell {
    pub(crate) fn new() -> Self {
        Self {
            contents: Default::default(),
            num_bytes: 0,
            len: 0,
            attrs: crate::attrs::Attrs::default(),
            selected: false,
        }
    }

    #[inline]
    fn len(&self) -> usize {
        usize::from(self.len & 0x0f)
    }

    #[inline]
    fn num_bytes(&self) -> usize {
        usize::from(self.num_bytes & 0x0f)
    }

    pub(crate) fn set(&mut self, c: char, a: crate::attrs::Attrs) {
        self.num_bytes = 0;
        self.len = 0;
        self.append_char(0, c);
        // strings in this context should always be an arbitrary character
        // followed by zero or more zero-width characters, so we should only
        // have to look at the first character
        self.set_wide(c.width().unwrap_or(1) > 1);
        self.attrs = a;
    }

    pub(crate) fn append(&mut self, c: char) {
        let len = self.len();
        // This implies that len is at most 5 meaning num_bytes is at most 20
        // with still enough room for a 4 byte char.
        if len >= CODEPOINTS_IN_CELL {
            return;
        }
        if len == 0 {
            // 0 is always less than 6
            self.contents[0] = b' ';
            self.num_bytes += 1;
            self.len += 1;
        }

        let num_bytes = self.num_bytes();
        // we already checked that len < CODEPOINTS_IN_CELL
        self.append_char(num_bytes, c);
    }

    #[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
    #[inline]
    // Writes bytes representing c at start
    // Requires caller to verify start <= CODEPOINTS_IN_CELL * 4
    fn append_char(&mut self, start: usize, c: char) {
        match c.len_utf8() {
            1 => {
                self.contents[start] = c as u8;
                self.num_bytes += 1;
            }
            n => {
                c.encode_utf8(&mut self.contents[start..]);
                self.num_bytes += n as u8;
            }
        }
        self.len += 1;
    }

    pub(crate) fn clear(&mut self, attrs: crate::attrs::Attrs) {
        self.len = 0;
        self.num_bytes = 0;
        self.attrs = attrs;
        self.selected = false;
    }

    pub(crate) fn selected(&self) -> bool {
        self.selected
    }

    pub(crate) fn select(&mut self, selected: bool) {
        self.selected = selected;
    }

    /// Returns the text contents of the cell.
    ///
    /// Can include multiple unicode characters if combining characters are
    /// used, but will contain at most one character with a non-zero character
    /// width.
    #[must_use]
    pub fn contents(&self) -> &str {
        let num_bytes = self.num_bytes();
        // Since contents has been constructed by appending chars encoded as UTF-8 it will be valid UTF-8
        unsafe { std::str::from_utf8_unchecked(&self.contents[..num_bytes]) }
    }

    /// Returns whether the cell contains any text data.
    #[must_use]
    pub fn has_contents(&self) -> bool {
        self.len > 0
    }

    /// Returns whether the text data in the cell represents a wide character.
    #[must_use]
    pub fn is_wide(&self) -> bool {
        self.len & 0x80 == 0x80
    }

    /// Returns whether the cell contains the second half of a wide character
    /// (in other words, whether the previous cell in the row contains a wide
    /// character)
    #[must_use]
    pub fn is_wide_continuation(&self) -> bool {
        self.len & 0x40 == 0x40
    }

    fn set_wide(&mut self, wide: bool) {
        if wide {
            self.len |= 0x80;
        } else {
            self.len &= 0x7f;
        }
    }

    pub(crate) fn set_wide_continuation(&mut self, wide: bool) {
        if wide {
            self.len |= 0x40;
        } else {
            self.len &= 0xbf;
        }
    }

    pub(crate) fn attrs(&self) -> &crate::attrs::Attrs {
        &self.attrs
    }

    /// Returns the foreground color of the cell.
    #[must_use]
    pub fn fgcolor(&self) -> crate::Color {
        self.attrs.fgcolor
    }

    /// Returns the background color of the cell.
    #[must_use]
    pub fn bgcolor(&self) -> crate::Color {
        self.attrs.bgcolor
    }

    /// Returns whether the cell should be rendered with the bold text
    /// attribute.
    #[must_use]
    pub fn bold(&self) -> bool {
        self.attrs.bold()
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

    /// Returns whether the cell should be rendered with the inverse text
    /// attribute.
    #[must_use]
    pub fn inverse(&self) -> bool {
        self.attrs.inverse() || self.selected
    }
}
