//! Terminal screen cell and row types.
//!
//! These types represent the contents of a terminal screen.
//! A [`Cell`] is a single grid cell and a [`Row`] is a single row.
//! Both are opaque values whose fields are accessed via their methods.
use std::{marker::PhantomData, mem::MaybeUninit, ptr::NonNull};

use crate::{
    error::{Error, Result, from_optional_result_uninit, from_result, from_result_with_len},
    ffi,
    style::{self, PaletteIndex, RgbColor, Style},
    terminal::{Point, PointCoordinate, PointSpace, Terminal},
};

/// Terminal screen identifier.
///
/// Identifies which screen buffer is active in the terminal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum Screen {
    /// The primary (normal) screen.
    #[default]
    Primary = ffi::TerminalScreen::PRIMARY,
    /// The alternate screen.
    Alternate = ffi::TerminalScreen::ALTERNATE,
}

/// Resolved reference to a terminal cell position.
///
/// A grid reference is a resolved reference to a specific cell position in
/// the terminal's internal page structure. Obtain a grid reference from
/// [`Terminal::grid_ref`][crate::Terminal::grid_ref], then extract the cell
/// or row via [`GridRef::cell`] and [`GridRef::row`].
///
/// A grid reference is only valid until the next update to the terminal
/// instance. There is no guarantee that a grid reference will remain valid
/// after ANY operation, even if a seemingly unrelated part of the grid is
/// changed, so any information related to the grid reference should be read
/// and cached immediately after obtaining the grid reference.
///
/// This API is not meant to be used as the core of render loop.
/// It isn't built to sustain the framerates needed for rendering large screens.
/// Use the render state API for that.
#[derive(Clone, Debug)]
pub struct GridRef<'t> {
    pub(crate) inner: ffi::GridRef,
    pub(crate) _phan: PhantomData<&'t ffi::Terminal>,
}

impl GridRef<'_> {
    pub(crate) unsafe fn from_raw(inner: ffi::GridRef) -> Self {
        Self {
            inner,
            _phan: PhantomData,
        }
    }

    /// Get the row from a grid reference.
    pub fn row(&self) -> Result<Row> {
        let mut v = ffi::Row::default();
        let result =
            unsafe { ffi::ghostty_grid_ref_row(std::ptr::from_ref(&self.inner), &raw mut v) };
        from_result(result)?;
        Ok(Row(v))
    }
    /// Get the cell from a grid reference.
    pub fn cell(&self) -> Result<Cell> {
        let mut v = ffi::Cell::default();
        let result =
            unsafe { ffi::ghostty_grid_ref_cell(std::ptr::from_ref(&self.inner), &raw mut v) };
        from_result(result)?;
        Ok(Cell(v))
    }
    /// Get the style of the cell at the grid reference's position.
    pub fn style(&self) -> Result<Style> {
        let mut v = ffi::Style::default();
        let result =
            unsafe { ffi::ghostty_grid_ref_style(std::ptr::from_ref(&self.inner), &raw mut v) };
        from_result(result)?;
        Style::try_from(v)
    }

    /// Get the grapheme cluster codepoints for the cell at the grid
    /// reference's position.
    ///
    /// Writes the full grapheme cluster (the cell's primary codepoint
    /// followed by any combining codepoints) into the provided buffer.
    /// If the cell has no text, `Ok(0)` is returned.
    ///
    /// If the buffer is too small, the function returns
    /// `Err(Error::OutOfSpace { required })` where `required` is the
    /// required number of codepoints. The caller can then retry with
    /// a sufficiently sized buffer.
    pub fn graphemes(&self, buf: &mut [char]) -> Result<usize> {
        let mut len = 0;
        let result = unsafe {
            ffi::ghostty_grid_ref_graphemes(
                std::ptr::from_ref(&self.inner),
                std::ptr::from_mut(buf).cast(),
                buf.len(),
                &raw mut len,
            )
        };
        from_result_with_len(result, len)
    }

    /// Get the hyperlink URI for the cell at the grid reference's position.
    ///
    /// Writes the URI bytes into the provided buffer.
    /// If the cell has no hyperlink, `Ok(0)` is returned.
    ///
    /// If the buffer is too small, the function returns
    /// `Err(Error::OutOfSpace { required })` where `required` is the
    /// required number of codepoints. The caller can then retry with
    /// a sufficiently sized buffer.
    pub fn hyperlink_uri(&self, buf: &mut [u8]) -> Result<usize> {
        let mut len = 0;
        let result = unsafe {
            ffi::ghostty_grid_ref_hyperlink_uri(
                std::ptr::from_ref(&self.inner),
                std::ptr::from_mut(buf).cast(),
                buf.len(),
                &raw mut len,
            )
        };
        from_result_with_len(result, len)
    }
}

/// Owned grid references that move with the terminal.
///
/// A tracked grid reference follows its cell across normal screen operations.
/// For example scrolling, scrollback pruning, resize/reflow, and other
/// terminal mutations update the tracked reference automatically.
///
/// A tracked reference can still lose its original semantic location.
/// This can happen when the underlying grid is reset, pruned, or otherwise
/// discarded in a way that cannot be mapped to a meaningful new cell.
/// In that state, [`TrackedGridRef::has_value`] returns `false` and
/// [`TrackedGridRef::snapshot`] / [`TrackedGridRef::point`] return `Ok(None)`.
/// The handle remains valid, and callers may move it to a new point with
/// [`TrackedGridRef::set`].
///
/// To read cell data from a tracked reference, first snapshot it with
/// [`TrackedGridRef::snapshot`]. The returned [`GridRef`] is again an
/// untracked reference and follows the same short lifetime rules as any
/// other untracked grid reference.
///
/// A tracked reference belongs to the terminal screen/page-list that was
/// active when it was created or last set. Converting it to a point uses that
/// owning screen/page-list, even if the terminal has since switched between
/// primary and alternate screens. Calling [`TrackedGridRef::set`] resolves
/// the new point against the terminal's currently active screen/page-list
/// and may move the tracked reference between screens.
///
/// If the tracked grid reference outlives the terminal it is created from,
/// it remains valid, but all APIs return either `false` or `Ok(None)`.
///
/// Each tracked reference adds bookkeeping to terminal mutations. Use them
/// sparingly for long-lived anchors such as selections, search state, marks,
/// or application-side bookmarks.
#[derive(Debug)]
pub struct TrackedGridRef {
    inner: NonNull<ffi::TrackedGridRefImpl>,
    terminal: NonNull<ffi::TerminalImpl>,
}

impl TrackedGridRef {
    pub(crate) fn new(
        inner: NonNull<ffi::TrackedGridRefImpl>,
        terminal: NonNull<ffi::TerminalImpl>,
    ) -> Self {
        Self { inner, terminal }
    }

    /// Whether a tracked grid reference currently has a meaningful value.
    ///
    /// If the terminal that created the tracked reference has been dropped,
    /// this returns false.
    pub fn has_value(&self) -> bool {
        unsafe { ffi::ghostty_tracked_grid_ref_has_value(self.inner.as_ptr()) }
    }

    /// Snapshot a tracked grid reference into a regular [`GridRef`].
    ///
    /// The returned [`GridRef`] is an untracked snapshot and has the same
    /// lifetime rules as [`Terminal::grid_ref`]: it is only valid until the
    /// next terminal update. Snapshot immediately before calling
    /// [`GridRef::cell`], [`GridRef::row`], [`GridRef::graphemes`],
    /// [`GridRef::hyperlink_uri`], or [`GridRef::style`],
    ///
    /// If the tracked reference no longer has a meaningful value, this returns
    /// `Ok(None)`. This includes references whose owning terminal has been
    /// dropped.
    pub fn snapshot<'t>(&self, terminal: &'t Terminal<'_, '_>) -> Result<Option<GridRef<'t>>> {
        // The C ghostty_tracked_grid_ref_snapshot does not take a terminal, so
        // we validate the pairing here to keep the returned GridRef's lifetime
        // soundly tied to a terminal that actually owns the underlying pin.
        if self.terminal != terminal.inner.ptr {
            return Err(Error::InvalidValue);
        }
        let mut grid_ref = MaybeUninit::new(ffi::sized!(ffi::GridRef));
        let result = unsafe {
            ffi::ghostty_tracked_grid_ref_snapshot(self.inner.as_ptr(), grid_ref.as_mut_ptr())
        };

        from_optional_result_uninit(result, grid_ref).map(|value| {
            value.map(|raw| unsafe {
                // SAFETY: A successful libghostty snapshot initializes a
                // short-lived untracked grid reference for the provided
                // terminal. The returned Rust lifetime is tied to that
                // terminal borrow.
                GridRef::from_raw(raw)
            })
        })
    }

    /// Convert a tracked grid reference to a point in the requested coordinate
    /// space.
    ///
    /// This is the tracked equivalent of [`Terminal::point_from_grid_ref`].
    /// Unlike snapshotting, this does not expose an intermediate untracked
    /// [`GridRef`].
    ///
    /// A tracked reference is resolved against the terminal screen/page-list
    /// that currently owns the reference. If the terminal has switched between
    /// primary and alternate screens since the reference was created or last
    /// set, this may be different from the terminal's currently active screen.
    ///
    /// If the tracked reference no longer has a meaningful value, this returns
    /// `Ok(None)`. `Ok(None` is also returned when the reference cannot be
    /// represented in the requested coordinate space, including after the
    /// terminal that created the tracked reference has been dropped.
    pub fn point(&self, space: PointSpace) -> Result<Option<PointCoordinate>> {
        let mut point = MaybeUninit::<ffi::PointCoordinate>::zeroed();
        let result = unsafe {
            ffi::ghostty_tracked_grid_ref_point(
                self.inner.as_ptr(),
                space.into_raw(),
                point.as_mut_ptr(),
            )
        };

        from_optional_result_uninit(result, point).map(|value| value.map(Into::into))
    }

    /// Move an existing tracked grid reference to a new terminal point.
    ///
    /// On success, the tracked reference begins tracking the new point and any
    /// prior "no value" state is cleared. On `Err(Error::OutOfMemory)`, the
    /// original tracked reference is left unchanged.
    ///
    /// The terminal must be the same terminal that created the tracked
    /// reference. The point is resolved against the terminal
    /// screen/page-list that is active at the time this function is called.
    /// If the terminal has switched between primary and alternate screens,
    /// this may move the tracked reference from one screen/page-list to the
    /// other.
    pub fn set(&mut self, terminal: &mut Terminal<'_, '_>, point: Point) -> Result<&mut Self> {
        // The C layer validates the terminal/tracked-ref pairing and returns
        // GHOSTTY_INVALID_VALUE on mismatch, so we don't duplicate the check
        // on the Rust side.
        let result = unsafe {
            ffi::ghostty_tracked_grid_ref_set(
                self.inner.as_ptr(),
                terminal.inner.as_raw(),
                point.into(),
            )
        };
        from_result(result)?;
        Ok(self)
    }
}

impl Drop for TrackedGridRef {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_tracked_grid_ref_free(self.inner.as_ptr()) }
    }
}

/// Represents a single terminal row.
///
/// The internal layout is opaque and must be queried via its methods.
/// Obtain cell values from terminal query APIs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row(pub(crate) ffi::Row);

impl Row {
    fn get<T>(&self, tag: ffi::RowData::Type) -> Result<T> {
        let mut value = MaybeUninit::<T>::zeroed();
        let result = unsafe { ffi::ghostty_row_get(self.0, tag, value.as_mut_ptr().cast()) };
        // Since we manually model every possible query, this should never fail.
        from_result(result)?;
        // SAFETY: Value should be initialized after successful call.
        Ok(unsafe { value.assume_init() })
    }

    /// Whether this row is soft-wrapped.
    pub fn is_wrapped(self) -> Result<bool> {
        self.get(ffi::RowData::WRAP)
    }
    /// Whether this row is a continuation of a soft-wrapped row.
    pub fn is_wrap_continuation(self) -> Result<bool> {
        self.get(ffi::RowData::WRAP_CONTINUATION)
    }
    /// Whether any cells in this row have grapheme clusters.
    pub fn has_grapheme_cluster(self) -> Result<bool> {
        self.get(ffi::RowData::GRAPHEME)
    }
    /// Whether any cells in this row have styling (may have false positives).
    pub fn is_styled(self) -> Result<bool> {
        self.get(ffi::RowData::STYLED)
    }
    /// Whether any cells in this row have hyperlinks (may have false
    /// positives).
    pub fn has_hyperlink(self) -> Result<bool> {
        self.get(ffi::RowData::HYPERLINK)
    }
    /// The semantic prompt state of this row.
    pub fn semantic_prompt(self) -> Result<RowSemanticPrompt> {
        self.get::<ffi::RowSemanticPrompt::Type>(ffi::RowData::SEMANTIC_PROMPT)
            .and_then(|v| v.try_into().map_err(|_| Error::InvalidValue))
    }
    /// Whether this row contains a Kitty virtual placeholder.
    pub fn has_kitty_virtual_placeholder(self) -> Result<bool> {
        self.get(ffi::RowData::KITTY_VIRTUAL_PLACEHOLDER)
    }
    /// Whether this row is dirty and requires a redraw.
    pub fn is_dirty(self) -> Result<bool> {
        self.get(ffi::RowData::DIRTY)
    }
}

/// Represents a single terminal cell.
///
/// The internal layout is opaque and must be queried via its methods.
/// Obtain cell values from terminal query APIs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell(pub(crate) ffi::Cell);

impl Cell {
    fn get<T>(&self, tag: ffi::CellData::Type) -> Result<T> {
        let mut value = MaybeUninit::<T>::zeroed();
        let result = unsafe { ffi::ghostty_cell_get(self.0, tag, value.as_mut_ptr().cast()) };
        // Since we manually model every possible query, this should never fail.
        from_result(result)?;
        // SAFETY: Value should be initialized after successful call.
        Ok(unsafe { value.assume_init() })
    }

    /// The codepoint of the cell (0 if empty or bg-color-only).
    pub fn codepoint(self) -> Result<u32> {
        self.get(ffi::CellData::CODEPOINT)
    }
    /// The content tag describing what kind of content is in the cell.
    pub fn content_tag(self) -> Result<CellContentTag> {
        self.get::<ffi::CellContentTag::Type>(ffi::CellData::CONTENT_TAG)
            .and_then(|v| v.try_into().map_err(|_| Error::InvalidValue))
    }
    /// The wide property of the cell.
    pub fn wide(self) -> Result<CellWide> {
        self.get::<ffi::CellWide::Type>(ffi::CellData::WIDE)
            .and_then(|v| v.try_into().map_err(|_| Error::InvalidValue))
    }
    /// Whether the cell has text to render.
    pub fn has_text(self) -> Result<bool> {
        self.get(ffi::CellData::HAS_TEXT)
    }
    /// Whether the cell has non-default styling.
    pub fn has_styling(self) -> Result<bool> {
        self.get(ffi::CellData::HAS_STYLING)
    }
    /// The style ID for the cell (for use with style lookups).
    pub fn style_id(self) -> Result<style::Id> {
        self.get(ffi::CellData::STYLE_ID).map(style::Id)
    }
    /// Whether the cell has a hyperlink.
    pub fn has_hyperlink(self) -> Result<bool> {
        self.get(ffi::CellData::HAS_HYPERLINK)
    }
    /// Whether the cell is protected.
    pub fn is_protected(self) -> Result<bool> {
        self.get(ffi::CellData::PROTECTED)
    }
    /// The semantic content type of the cell (from OSC 133).
    pub fn semantic_content(self) -> Result<CellSemanticContent> {
        self.get::<ffi::CellSemanticContent::Type>(ffi::CellData::SEMANTIC_CONTENT)
            .and_then(|v| v.try_into().map_err(|_| Error::InvalidValue))
    }

    /// The palette index for the cell's background color.
    ///
    /// Only valid when [`Cell::content_tag`] is
    /// [`CellContentTag::BgColorPalette`].
    pub fn bg_color_palette(self) -> Result<PaletteIndex> {
        self.get(ffi::CellData::COLOR_PALETTE).map(PaletteIndex)
    }
    /// The RGB color value for the cell's background color.
    ///
    /// Only valid when [`Cell::content_tag`] is [`CellContentTag::BgColorRgb`].
    pub fn bg_color_rgb(self) -> Result<RgbColor> {
        Ok(self.get::<ffi::ColorRgb>(ffi::CellData::COLOR_RGB)?.into())
    }
}

/// Row semantic prompt state.
///
/// Indicates whether any cells in a row are part of a shell prompt, as reported
/// by OSC 133 sequences.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
pub enum RowSemanticPrompt {
    /// No prompt cells in this row.
    None = ffi::RowSemanticPrompt::NONE,
    /// Prompt cells exist and this is a primary prompt line.
    Prompt = ffi::RowSemanticPrompt::PROMPT,
    /// Prompt cells exist and this is a continuation line.
    Continuation = ffi::RowSemanticPrompt::PROMPT_CONTINUATION,
}

/// Cell content tag.
///
/// Describes what kind of content a cell holds.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
pub enum CellContentTag {
    /// A single codepoint (may be zero for empty).
    Codepoint = ffi::CellContentTag::CODEPOINT,
    /// A codepoint that is part of a multi-codepoint grapheme cluster.
    CodepointGrapheme = ffi::CellContentTag::CODEPOINT_GRAPHEME,
    /// No text; background color from palette.
    BgColorPalette = ffi::CellContentTag::BG_COLOR_PALETTE,
    /// No text; background color as RGB.
    BgColorRgb = ffi::CellContentTag::BG_COLOR_RGB,
}

/// Cell wide property.
///
/// Describes the width behavior of a cell.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
pub enum CellWide {
    /// Not a wide character, cell width 1.
    Narrow = ffi::CellWide::NARROW,
    /// Wide character, cell width 2.  
    Wide = ffi::CellWide::WIDE,
    /// Spacer after wide character. Do not render.
    SpacerTail = ffi::CellWide::SPACER_TAIL,
    /// Spacer at end of soft-wrapped line for a wide character.
    SpacerHead = ffi::CellWide::SPACER_HEAD,
}

/// Semantic content type of a cell.
///
/// Set by semantic prompt sequences (OSC 133) to distinguish between
/// command output, user input, and shell prompt text.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
pub enum CellSemanticContent {
    /// Regular output content, such as command output.
    Output = ffi::CellSemanticContent::OUTPUT,
    /// Content that is part of user input.
    Input = ffi::CellSemanticContent::INPUT,
    /// Content that is part of a shell prompt.
    Prompt = ffi::CellSemanticContent::PROMPT,
}
