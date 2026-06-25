//! Terminal cell style attributes.
//!
//! A style describes the visual attributes of a terminal cell, including
//! foreground, background, and underline colors, as well as flags for bold,
//! italic, underline, and other text decorations.
use std::mem::MaybeUninit;

use crate::{
    error::{Error, Result},
    ffi,
};

/// Style identifier type.
///
/// Used to look up the full style from a grid reference.
/// Obtain this from a cell via [`Cell::style_id`][crate::screen::Cell::style_id].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Id(pub(crate) ffi::StyleId);

/// Terminal cell style attributes.
///
/// A style describes the visual attributes of a terminal cell, including
/// foreground, background, and underline colors, as well as flags for bold,
/// italic, underline, and other text decorations.
#[expect(
    clippy::struct_excessive_bools,
    reason = "style attributes should be just a bunch of bools"
)]
#[expect(missing_docs, reason = "self-explanatory")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    pub fg_color: StyleColor,
    pub bg_color: StyleColor,
    pub underline_color: StyleColor,
    pub bold: bool,
    pub italic: bool,
    pub faint: bool,
    pub blink: bool,
    pub inverse: bool,
    pub invisible: bool,
    pub strikethrough: bool,
    pub overline: bool,
    pub underline: Underline,
}

impl Style {
    /// Check if a style is the default style.
    ///
    /// Returns true if all colors are unset and all flags are off.
    #[must_use]
    pub fn is_default(self) -> bool {
        let raw = ffi::Style::from(self);
        unsafe { ffi::ghostty_style_is_default(&raw const raw) }
    }
}

impl Default for Style {
    fn default() -> Self {
        let mut style = MaybeUninit::zeroed();
        unsafe {
            ffi::ghostty_style_default(style.as_mut_ptr());
        }

        // SAFETY: We trust the function above to initialize everything correctly
        Self::try_from(unsafe { style.assume_init() })
            .expect("ghostty_style_default to init valid Style")
    }
}

/// A color used in a style attribute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StyleColor {
    /// Unset.
    None,
    /// Palette index.
    Palette(PaletteIndex),
    /// Direct RGB value.
    Rgb(RgbColor),
}

/// RGB color value.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct RgbColor {
    /// Red color component (0-255)
    pub r: u8,
    /// Green color component (0-255)
    pub g: u8,
    /// Blue color component (0-255)
    pub b: u8,
}

/// Palette color index (0-255).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PaletteIndex(pub ffi::ColorPaletteIndex);

impl PaletteIndex {
    #![expect(missing_docs, reason = "self-explanatory")]
    pub const BLACK: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_BLACK);
    pub const RED: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_RED);
    pub const GREEN: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_GREEN);
    pub const YELLOW: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_YELLOW);
    pub const BLUE: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_BLUE);
    pub const MAGENTA: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_MAGENTA);
    pub const CYAN: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_CYAN);
    pub const WHITE: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_WHITE);
    pub const BRIGHT_BLACK: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_BRIGHT_BLACK);
    pub const BRIGHT_RED: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_BRIGHT_RED);
    pub const BRIGHT_GREEN: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_BRIGHT_GREEN);
    pub const BRIGHT_YELLOW: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_BRIGHT_YELLOW);
    pub const BRIGHT_BLUE: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_BRIGHT_BLUE);
    pub const BRIGHT_MAGENTA: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_BRIGHT_MAGENTA);
    pub const BRIGHT_CYAN: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_BRIGHT_CYAN);
    pub const BRIGHT_WHITE: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_BRIGHT_WHITE);
}

/// Underline style types.
#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, int_enum::IntEnum)]
#[non_exhaustive]
#[expect(missing_docs, reason = "self-explanatory")]
pub enum Underline {
    None = ffi::SgrUnderline::NONE,
    Single = ffi::SgrUnderline::SINGLE,
    Double = ffi::SgrUnderline::DOUBLE,
    Curly = ffi::SgrUnderline::CURLY,
    Dotted = ffi::SgrUnderline::DOTTED,
    Dashed = ffi::SgrUnderline::DASHED,
}

//----------------------------------
// Conversion to and from FFI types
//----------------------------------

impl TryFrom<ffi::Style> for Style {
    type Error = Error;
    fn try_from(value: ffi::Style) -> Result<Self> {
        Ok(Self {
            fg_color: StyleColor::try_from(value.fg_color)?,
            bg_color: StyleColor::try_from(value.bg_color)?,
            underline_color: StyleColor::try_from(value.underline_color)?,
            bold: value.bold,
            italic: value.italic,
            faint: value.faint,
            blink: value.blink,
            inverse: value.inverse,
            invisible: value.invisible,
            strikethrough: value.strikethrough,
            overline: value.overline,
            #[expect(clippy::cast_sign_loss, reason = "bindgen ain't perfect")]
            underline: Underline::try_from(value.underline as u32)
                .map_err(|_| Error::InvalidValue)?,
        })
    }
}

impl From<Style> for ffi::Style {
    fn from(value: Style) -> Self {
        Self {
            size: std::mem::size_of::<Self>(),
            fg_color: value.fg_color.into(),
            bg_color: value.bg_color.into(),
            underline_color: value.underline_color.into(),
            bold: value.bold,
            italic: value.italic,
            faint: value.faint,
            blink: value.blink,
            inverse: value.inverse,
            invisible: value.invisible,
            strikethrough: value.strikethrough,
            overline: value.overline,
            #[expect(clippy::cast_possible_wrap, reason = "bindgen ain't perfect")]
            underline: u32::from(value.underline) as i32,
        }
    }
}

impl TryFrom<ffi::StyleColor> for StyleColor {
    type Error = Error;
    fn try_from(value: ffi::StyleColor) -> Result<Self> {
        Ok(match value.tag {
            ffi::StyleColorTag::NONE => Self::None,
            ffi::StyleColorTag::PALETTE => {
                Self::Palette(PaletteIndex(unsafe { value.value.palette }))
            }
            ffi::StyleColorTag::RGB => Self::Rgb(unsafe { value.value.rgb }.into()),
            _ => return Err(Error::InvalidValue),
        })
    }
}

impl From<StyleColor> for ffi::StyleColor {
    fn from(value: StyleColor) -> Self {
        match value {
            StyleColor::None => Self {
                tag: ffi::StyleColorTag::NONE,
                value: ffi::StyleColorValue::default(),
            },
            StyleColor::Palette(PaletteIndex(palette)) => Self {
                tag: ffi::StyleColorTag::PALETTE,
                value: ffi::StyleColorValue { palette },
            },
            StyleColor::Rgb(rgb) => Self {
                tag: ffi::StyleColorTag::RGB,
                value: ffi::StyleColorValue { rgb: rgb.into() },
            },
        }
    }
}

impl From<ffi::ColorRgb> for RgbColor {
    fn from(value: ffi::ColorRgb) -> Self {
        let ffi::ColorRgb { r, g, b } = value;
        Self { r, g, b }
    }
}

impl From<RgbColor> for ffi::ColorRgb {
    fn from(value: RgbColor) -> Self {
        let RgbColor { r, g, b } = value;
        Self { r, g, b }
    }
}
