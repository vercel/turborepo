//! Managing [render states](RenderState) of the terminal.

use std::{convert::Into, marker::PhantomData, mem::MaybeUninit};

pub use ffi::RenderStateRowSelection as RowSelection;

use crate::{
    alloc::{Allocator, Object},
    error::{Error, Result, from_optional_result, from_result},
    ffi,
    screen::{Cell, Row},
    style::{RgbColor, Style},
    terminal::Terminal,
};

/// Represents the state required to render a visible screen (a viewport) of
/// a terminal instance.
///
/// This is stateful and optimized for repeated updates from a single terminal
/// instance and only updating dirty regions of the screen.
///
/// The key design principle of this API is that it only needs read/write
/// access to the terminal instance during the update call. This allows the
/// render state to minimally impact terminal IO performance and also allows
/// the renderer to be safely multi-threaded (as long as a lock is held
/// during the update call to ensure exclusive access to the terminal instance).
///
/// The basic usage of this API is:
///
///  1. Create an empty render state
///  2. Update it from a terminal instance whenever you need.
///  3. Read from the render state to get the data needed to draw your frame.
///
/// # Dirty Tracking
///
/// Dirty tracking is a key feature of the render state that allows renderers
/// to efficiently determine what parts of the screen have changed and only
/// redraw changed regions.
///
/// The render state API keeps track of dirty state at two independent layers:
/// a global dirty state that indicates whether the entire frame is clean,
/// partially dirty, or fully dirty, and a per-row dirty state that allows
/// tracking which rows in a partially dirty frame have changed.
///
/// The user of the render state API is expected to unset both of these.
/// The update call does not unset dirty state, it only updates it.
///
/// An extremely important detail: **setting one dirty state doesn't unset
/// the other.** For example, setting the global dirty state to false does
/// not reset the row-level dirty flags. So, the caller of the render state
/// API must be careful to manage both layers of dirty state correctly.
///
/// # Examples
///
/// ## Creating and updating render state
///
/// ```rust
/// // Create a terminal and render state, then update the render state
/// // from the terminal. The render state captures a snapshot of everything
/// // needed to draw a frame.
/// use libghostty_vt::{Terminal, TerminalOptions, RenderState};
///
/// let mut terminal = Terminal::new(TerminalOptions {
///     cols: 40,
///     rows: 5,
///     max_scrollback: 10000,
/// }).unwrap();
///
/// let mut render_state = RenderState::new().unwrap();
///
/// // Feed some styled content into the terminal.
/// terminal.vt_write(b"Hello, \x1b[1;32mworld\x1b[0m!\r\n");
/// terminal.vt_write(b"\x1b[4munderlined\x1b[0m text\r\n");
/// terminal.vt_write(b"\x1b[38;2;255;128;0morange\x1b[0m\r\n");
///
/// assert!(render_state.update(&terminal).is_ok());
/// ```
///
/// ## Checking dirty state
///
/// ```rust
/// // Check the global dirty state to decide how much work the renderer
/// // needs to do. After rendering, reset it to false.
/// # use libghostty_vt::{Terminal, TerminalOptions, RenderState, render::Dirty};
/// # let terminal = Terminal::new(TerminalOptions {
/// #     cols: 80,
/// #     rows: 25,
/// #     max_scrollback: 10000,
/// # }).unwrap();
/// # let mut render_state = RenderState::new().unwrap();
/// let snapshot = render_state.update(&terminal).unwrap();
///
/// match snapshot.dirty().unwrap() {
///     Dirty::Clean => println!("Frame is clean, nothing to draw."),
///     Dirty::Partial => println!("Partial redraw needed."),
///     Dirty::Full => println!("Full redraw needed."),
/// }
/// ```
///
/// ## Reading colors
///
/// ```rust
/// // Retrieve colors (background, foreground, palette) from the render
/// // state. These are needed to resolve palette-indexed cell colors.
/// # use libghostty_vt::{Terminal, TerminalOptions, RenderState};
/// # let terminal = Terminal::new(TerminalOptions {
/// #     cols: 80,
/// #     rows: 25,
/// #     max_scrollback: 10000,
/// # }).unwrap();
/// # let mut render_state = RenderState::new().unwrap();
/// let snapshot = render_state.update(&terminal).unwrap();
/// let colors = snapshot.colors().unwrap();
///
/// println!(
///     "Background: {:02x}{:02x}{:02x}",
///     colors.background.r, colors.background.g, colors.background.b
/// );
/// println!(
///     "Foreground: {:02x}{:02x}{:02x}",
///     colors.background.r, colors.background.g, colors.background.b
/// );
/// ```
///
/// ## Reading cursor state
///
/// ```rust
/// // Read cursor position and visual style from the render state.
/// use libghostty_vt::render::CursorViewport;
/// # use libghostty_vt::{Terminal, TerminalOptions, RenderState};
/// # let terminal = Terminal::new(TerminalOptions {
/// #     cols: 80,
/// #     rows: 25,
/// #     max_scrollback: 10000,
/// # }).unwrap();
/// # let mut render_state = RenderState::new().unwrap();
/// let snapshot = render_state.update(&terminal).unwrap();
///
/// if snapshot.cursor_visible().unwrap() {
///     if let Some(CursorViewport { x, y, .. }) = snapshot.cursor_viewport().unwrap() {
///         let style = snapshot.cursor_visual_style().unwrap();
///         println!("Cursor at ({x}, {y}), style {style:?}");
///     }
/// }
/// ```
///
/// ## Iterating rows and cells
///
/// ```rust
/// // Iterate rows via the row iterator. For each dirty row, iterate its
/// // cells, read codepoints/graphemes and styles, and emit ANSI-colored
/// // output as a simple "renderer".
/// use libghostty_vt::{Terminal, TerminalOptions, RenderState};
/// use libghostty_vt::style::Underline;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// # let terminal = Terminal::new(TerminalOptions {
/// #     cols: 80,
/// #     rows: 25,
/// #     max_scrollback: 10000,
/// # }).unwrap();
/// # let mut render_state = RenderState::new()?;
/// use libghostty_vt::render::{RowIterator, CellIterator};
///
/// // During setup:
/// let mut rows = RowIterator::new()?;
/// let mut cells = CellIterator::new()?;
///
/// // On each frame:
/// let snapshot = render_state.update(&terminal)?;
/// let colors = snapshot.colors()?;
///
/// let mut row_iter = rows.update(&snapshot)?;
/// let mut row_index = 0;
///
/// while let Some(row) = row_iter.next() {
///     // Check per-row dirty state; a real renderer would skip clean rows.
///     print!(
///         "Row {row_index} [{}]",
///         if row.dirty()? { "dirty" } else { "clean" }
///     );
///
///     // Get cells for this row (reuses the same cells handle).
///     let mut cell_iter = cells.update(&row)?;
///     while let Some(cell) = cell_iter.next() {
///         let graphemes = cell.graphemes()?;
///
///         if graphemes.is_empty() {
///             print!(" ");
///             continue;
///         }
///
///         // Resolve foreground color for this cell.
///         let fg = cell.fg_color()?.unwrap_or(colors.foreground);
///         // Emit ANSI true-color escape for the foreground.
///         print!("\x1b[38;2;{};{};{}m", fg.r, fg.g, fg.b);
///
///         // Read the style for this cell. Returns the default style for
///         // cells that have no explicit styling.
///         let style = cell.style()?;
///         if style.bold {
///             print!("\x1b[1m");
///         }
///         if style.underline != Underline::None {
///             print!("\x1b[4m");
///         }
///
///         for grapheme in graphemes {
///             print!("{}", grapheme.escape_default());
///         }
///         print!("\x1b[0m"); // Reset style after each cell.
///     }
///     println!();
///
///     // Clear per-row dirty flag after "rendering" it.
///     row.set_dirty(false);
///
///     row_index += 1;
/// }
/// # Ok(())}
/// ```
#[derive(Debug)]
pub struct RenderState<'alloc>(Object<'alloc, ffi::RenderStateImpl>);

/// A snapshot of the render state after an update.
///
/// This struct exists to guard data accessed from the render state from
/// being accidentally modified after an update. If you find yourself unable
/// to update the render state due to borrow checker errors, make sure to
/// drop the active snapshot (and data that depends on it) before updating.
#[derive(Debug)]
pub struct Snapshot<'alloc, 's>(&'s mut RenderState<'alloc>);

/// Opaque handle to a render-state row iterator.
///
/// The row iterator must be [updated](RowIterator::update) from a snapshot of
/// the render state in order to function, as most data is only accessible
/// per [iteration](RowIteration).
#[derive(Debug)]
pub struct RowIterator<'alloc>(Object<'alloc, ffi::RenderStateRowIteratorImpl>);

/// An active iteration over the rows in the render state.
///
/// Row iterations are created by [updating](RowIterator::update) row iterators
/// with a snapshot of the render state. The borrow checker statically
/// guarantees that all accesses of the data do not outlive the given snapshot,
/// at the cost of added lifetime annotations.
#[derive(Debug)]
pub struct RowIteration<'alloc, 's> {
    iter: &'s mut RowIterator<'alloc>,
    // NOTE: While in theory the snapshot borrow should have its own
    // lifetime 'ss where 'rs: 'ss, but it gets very unwieldy and honestly
    // one wouldn't run into too many situations where this simpler constraint
    // isn't enough.
    _phan: PhantomData<&'s Snapshot<'alloc, 's>>,
}

/// Opaque handle to a render state cell iterator.
///
/// The cell iterator must be [updated](CellIterator::update) from a
/// [row](RowIteration) in order to function, as most data is only
/// accessible per [iteration](CellIteration).
#[derive(Debug)]
pub struct CellIterator<'alloc>(Object<'alloc, ffi::RenderStateRowCellsImpl>);

/// An active iteration over the cells on a given row
/// within the render state.
///
/// Cell iterations are created by [updating](CellIterator::update) row
/// iterators at a given [row](RowIteration). The borrow checker statically
/// guarantees that all accesses of the data do not outlive the given snapshot,
/// at the cost of added lifetime annotations.
#[derive(Debug)]
pub struct CellIteration<'alloc, 's> {
    iter: &'s mut CellIterator<'alloc>,
    _phan: PhantomData<&'s RowIteration<'alloc, 's>>,
}

//--------------------------
// Impl blocks
//--------------------------

impl<'alloc> RenderState<'alloc> {
    /// Create a new render state instance.
    pub fn new() -> Result<Self> {
        // SAFETY: A NULL allocator is always valid
        unsafe { Self::new_inner(std::ptr::null()) }
    }

    /// Create a new render state instance with a custom allocator.
    ///
    /// See the [crate-level
    /// documentation](crate#memory-management-and-lifetimes)
    /// regarding custom memory management and lifetimes.
    pub fn new_with_alloc<'ctx: 'alloc>(alloc: &'alloc Allocator<'ctx>) -> Result<Self> {
        // SAFETY: Borrow checking should forbid invalid allocators
        unsafe { Self::new_inner(alloc.to_raw()) }
    }

    unsafe fn new_inner(alloc: *const ffi::Allocator) -> Result<Self> {
        let mut raw: ffi::RenderState = std::ptr::null_mut();
        let result = unsafe { ffi::ghostty_render_state_new(alloc, &raw mut raw) };
        from_result(result)?;
        Ok(Self(Object::new(raw)?))
    }

    /// Update a render state instance from a terminal,
    /// returning a new [snapshot](Snapshot).
    ///
    /// This consumes terminal/screen dirty state in the same way as the
    /// internal render state update path.
    ///
    /// # Errors
    ///
    /// Returns `Err(Error::OutOfMemory)` if updating the state requires
    /// allocation and that allocation fails.
    pub fn update<'cb>(
        &mut self,
        terminal: &Terminal<'alloc, 'cb>,
    ) -> Result<Snapshot<'alloc, '_>> {
        let result =
            unsafe { ffi::ghostty_render_state_update(self.0.as_raw(), terminal.inner.as_raw()) };
        from_result(result)?;
        Ok(Snapshot(self))
    }
}

impl Drop for RenderState<'_> {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_render_state_free(self.0.as_raw()) }
    }
}

impl Snapshot<'_, '_> {
    fn get<T>(&self, tag: ffi::RenderStateData::Type) -> Result<T> {
        let mut value = MaybeUninit::<T>::zeroed();
        let result = unsafe {
            ffi::ghostty_render_state_get(self.0.0.as_raw(), tag, value.as_mut_ptr().cast())
        };
        // Since we manually model every possible query, this should never fail.
        from_result(result)?;
        // SAFETY: Value should be initialized after successful call.
        Ok(unsafe { value.assume_init() })
    }

    fn set<T>(&self, tag: ffi::RenderStateOption::Type, value: &T) -> Result<()> {
        let result = unsafe {
            ffi::ghostty_render_state_set(self.0.0.as_raw(), tag, std::ptr::from_ref(value).cast())
        };
        // Since we manually model every possible query, this should never fail.
        from_result(result)
    }

    /// Get the current dirty state.
    pub fn dirty(&self) -> Result<Dirty> {
        self.get::<ffi::RenderStateDirty::Type>(ffi::RenderStateData::DIRTY)
            .and_then(|v| v.try_into().map_err(|_| Error::InvalidValue))
    }

    /// Get the viewport width.
    pub fn cols(&self) -> Result<u16> {
        self.get(ffi::RenderStateData::COLS)
    }

    /// Get the viewport height.
    pub fn rows(&self) -> Result<u16> {
        self.get(ffi::RenderStateData::ROWS)
    }

    /// Get the cursor color that may have been explicitly set by the terminal
    /// state.
    pub fn cursor_color(&self) -> Result<Option<RgbColor>> {
        let has_value = self.get(ffi::RenderStateData::COLOR_CURSOR_HAS_VALUE)?;
        if has_value {
            let color = self.get(ffi::RenderStateData::COLOR_CURSOR)?;
            Ok(Some(color))
        } else {
            Ok(None)
        }
    }

    /// Whether the cursor is currently visible based on terminal modes.
    pub fn cursor_visible(&self) -> Result<bool> {
        self.get(ffi::RenderStateData::CURSOR_VISIBLE)
    }

    /// Whether the cursor is currently blinking based on terminal modes.
    pub fn cursor_blinking(&self) -> Result<bool> {
        self.get(ffi::RenderStateData::CURSOR_BLINKING)
    }

    /// Whether the cursor is at a password input field.
    pub fn cursor_password_input(&self) -> Result<bool> {
        self.get(ffi::RenderStateData::CURSOR_PASSWORD_INPUT)
    }

    /// Get the visual style of the cursor.
    pub fn cursor_visual_style(&self) -> Result<CursorVisualStyle> {
        self.get::<ffi::RenderStateCursorVisualStyle::Type>(
            ffi::RenderStateData::CURSOR_VISUAL_STYLE,
        )
        .and_then(|v| v.try_into().map_err(|_| Error::InvalidValue))
    }

    /// Get the relative position of the cursor and other information
    /// if it is currently visible within the viewport.
    pub fn cursor_viewport(&self) -> Result<Option<CursorViewport>> {
        let has_value = self.get(ffi::RenderStateData::CURSOR_VIEWPORT_HAS_VALUE)?;
        if has_value {
            let x = self.get(ffi::RenderStateData::CURSOR_VIEWPORT_X)?;
            let y = self.get(ffi::RenderStateData::CURSOR_VIEWPORT_Y)?;
            let at_wide_tail = self.get(ffi::RenderStateData::CURSOR_VIEWPORT_WIDE_TAIL)?;
            Ok(Some(CursorViewport { x, y, at_wide_tail }))
        } else {
            Ok(None)
        }
    }

    /// Get the current color information from a render state.
    pub fn colors(&self) -> Result<Colors> {
        let mut colors = ffi::sized!(ffi::RenderStateColors);
        let result =
            unsafe { ffi::ghostty_render_state_colors_get(self.0.0.as_raw(), &raw mut colors) };
        from_result(result)?;

        Ok(Colors {
            background: colors.background.into(),
            foreground: colors.foreground.into(),
            cursor: if colors.cursor_has_value {
                Some(colors.cursor.into())
            } else {
                None
            },
            palette: colors.palette.map(Into::into),
        })
    }

    /// Set dirty state.
    pub fn set_dirty(&self, dirty: Dirty) -> Result<()> {
        self.set(
            ffi::RenderStateOption::DIRTY,
            &(dirty as ffi::RenderStateDirty::Type),
        )
    }
}

impl<'alloc> RowIterator<'alloc> {
    /// Create a new row iterator instance.
    pub fn new() -> Result<Self> {
        // SAFETY: A NULL allocator is always valid
        unsafe { Self::new_inner(std::ptr::null()) }
    }

    /// Create a new cell iterator instance with a custom allocator.
    ///
    /// See the [crate-level
    /// documentation](crate#memory-management-and-lifetimes)
    /// regarding custom memory management and lifetimes.
    pub fn new_with_alloc<'ctx: 'alloc>(alloc: &'alloc Allocator<'ctx>) -> Result<Self> {
        // SAFETY: Borrow checking should forbid invalid allocators
        unsafe { Self::new_inner(alloc.to_raw()) }
    }

    unsafe fn new_inner(alloc: *const ffi::Allocator) -> Result<Self> {
        let mut raw: ffi::RenderStateRowIterator = std::ptr::null_mut();
        let result = unsafe { ffi::ghostty_render_state_row_iterator_new(alloc, &raw mut raw) };
        from_result(result)?;
        Ok(Self(Object::new(raw)?))
    }

    /// Update the row iterator for a snapshot of the render state,
    /// returning a new row iteration.
    pub fn update(
        &mut self,
        snapshot: &'_ Snapshot<'alloc, '_>,
    ) -> Result<RowIteration<'alloc, '_>> {
        let result = unsafe {
            ffi::ghostty_render_state_get(
                snapshot.0.0.as_raw(),
                ffi::RenderStateData::ROW_ITERATOR,
                std::ptr::from_mut(&mut self.0.ptr).cast(),
            )
        };
        from_result(result)?;

        Ok(RowIteration {
            iter: self,
            _phan: PhantomData,
        })
    }
}

impl Drop for RowIterator<'_> {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_render_state_row_iterator_free(self.0.as_raw()) }
    }
}

impl RowIteration<'_, '_> {
    /// Move a row iteration to the next row.
    ///
    /// Returns `Some(row)` if the iteration moved successfully and row
    /// data is available to read at the new position using `row`.
    #[expect(
        clippy::should_implement_trait,
        reason = "lending `next` cannot implement trait"
    )]
    pub fn next(&mut self) -> Option<&Self> {
        if unsafe { ffi::ghostty_render_state_row_iterator_next(self.iter.0.as_raw()) } {
            Some(self)
        } else {
            None
        }
    }

    fn get<T>(&self, tag: ffi::RenderStateRowData::Type) -> Result<T> {
        let mut value = MaybeUninit::<T>::zeroed();
        let result = unsafe {
            ffi::ghostty_render_state_row_get(self.iter.0.as_raw(), tag, value.as_mut_ptr().cast())
        };
        // Since we manually model every possible query, this should never fail.
        from_result(result)?;
        // SAFETY: Value should be initialized after successful call.
        Ok(unsafe { value.assume_init() })
    }

    fn set<T>(&self, tag: ffi::RenderStateRowOption::Type, value: &T) -> Result<()> {
        let result = unsafe {
            ffi::ghostty_render_state_row_set(
                self.iter.0.as_raw(),
                tag,
                std::ptr::from_ref(value).cast(),
            )
        };
        from_result(result)
    }

    /// Whether the current row is dirty.
    pub fn dirty(&self) -> Result<bool> {
        self.get(ffi::RenderStateRowData::DIRTY)
    }

    /// The raw row value.
    pub fn raw_row(&self) -> Result<Row> {
        self.get(ffi::RenderStateRowData::RAW).map(Row)
    }

    /// Set dirty state for the current row.
    pub fn set_dirty(&self, dirty: bool) -> Result<()> {
        self.set(ffi::RenderStateRowOption::DIRTY, &dirty)
    }

    /// Row-local selected cell range.
    pub fn selection(&self) -> Result<Option<RowSelection>> {
        let mut value = ffi::sized!(RowSelection);
        let result = unsafe {
            ffi::ghostty_render_state_row_get(
                self.iter.0.as_raw(),
                ffi::RenderStateRowData::SELECTION,
                std::ptr::from_mut(&mut value).cast(),
            )
        };
        // Since we manually model every possible query, this should never fail.
        // SAFETY: Value should be initialized after successful call.
        from_optional_result(result, value)
    }
}

impl<'alloc> CellIterator<'alloc> {
    /// Create a new cell iterator instance.
    pub fn new() -> Result<Self> {
        // SAFETY: A NULL allocator is always valid
        unsafe { Self::new_inner(std::ptr::null()) }
    }

    /// Create a new cell iterator instance with a custom allocator.
    ///
    /// See the [crate-level
    /// documentation](crate#memory-management-and-lifetimes)
    /// regarding custom memory management and lifetimes.
    pub fn new_with_alloc<'ctx: 'alloc>(alloc: &'alloc Allocator<'ctx>) -> Result<Self> {
        // SAFETY: Borrow checking should forbid invalid allocators
        unsafe { Self::new_inner(alloc.to_raw()) }
    }

    unsafe fn new_inner(alloc: *const ffi::Allocator) -> Result<Self> {
        let mut raw: ffi::RenderStateRowCells = std::ptr::null_mut();
        let result = unsafe { ffi::ghostty_render_state_row_cells_new(alloc, &raw mut raw) };
        from_result(result)?;
        Ok(Self(Object::new(raw)?))
    }

    /// Update the cell iterator for a new row iteration,
    /// returning a new cell iteration.
    pub fn update(
        &mut self,
        row: &'_ RowIteration<'alloc, '_>,
    ) -> Result<CellIteration<'alloc, '_>> {
        let result = unsafe {
            ffi::ghostty_render_state_row_get(
                row.iter.0.as_raw(),
                ffi::RenderStateRowData::CELLS,
                std::ptr::from_mut(&mut self.0.ptr).cast(),
            )
        };
        from_result(result)?;

        Ok(CellIteration {
            iter: self,
            _phan: PhantomData,
        })
    }
}

impl Drop for CellIterator<'_> {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_render_state_row_cells_free(self.0.as_raw()) }
    }
}

impl CellIteration<'_, '_> {
    /// Move a cell iteration to the next cell.
    ///
    /// Returns `Some(cell)` if the iteration moved successfully and cell
    /// data is available to read at the new position using `cell`.
    #[expect(
        clippy::should_implement_trait,
        reason = "lending `next` cannot implement trait"
    )]
    pub fn next(&mut self) -> Option<&Self> {
        if unsafe { ffi::ghostty_render_state_row_cells_next(self.iter.0.as_raw()) } {
            Some(self)
        } else {
            None
        }
    }

    /// Move a cell iteration to a specific column.
    ///
    /// Positions the iteration at the given x (column) index so that
    /// subsequent reads return data for that cell.
    pub fn select(&mut self, x: u16) -> Result<()> {
        let result = unsafe { ffi::ghostty_render_state_row_cells_select(self.iter.0.as_raw(), x) };
        from_result(result)
    }

    fn get<T>(&self, tag: ffi::RenderStateRowCellsData::Type) -> Result<T> {
        let mut value = MaybeUninit::<T>::zeroed();
        let result = unsafe {
            ffi::ghostty_render_state_row_cells_get(
                self.iter.0.as_raw(),
                tag,
                value.as_mut_ptr().cast(),
            )
        };
        from_result(result)?;
        // SAFETY: Value should be initialized after successful call.
        Ok(unsafe { value.assume_init() })
    }

    /// The raw cell value.
    pub fn raw_cell(&self) -> Result<Cell> {
        self.get(ffi::RenderStateRowCellsData::RAW).map(Cell)
    }

    /// The style for the current cell.
    pub fn style(&self) -> Result<Style> {
        let mut value = ffi::sized!(ffi::Style);
        let result = unsafe {
            ffi::ghostty_render_state_row_cells_get(
                self.iter.0.as_raw(),
                ffi::RenderStateRowCellsData::STYLE,
                std::ptr::from_mut(&mut value).cast(),
            )
        };
        from_result(result)?;
        Style::try_from(value)
    }

    /// The resolved foreground color of the cell.
    ///
    /// Resolves palette indices through the palette. Bold color handling
    /// is not applied; the caller should handle bold styling separately.
    ///
    /// Returns `None` if the cell has no explicit foreground color, in which
    /// case the caller should use whatever default foreground color it want
    /// (e.g. the terminal foreground).
    pub fn fg_color(&self) -> Result<Option<RgbColor>> {
        let res = self.get::<ffi::ColorRgb>(ffi::RenderStateRowCellsData::FG_COLOR);
        match res {
            Ok(o) => Ok(Some(o.into())),
            Err(Error::InvalidValue) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// The resolved background color of the cell.
    ///
    /// Flattens the three possible sources: [`Cell::bg_color_rgb`],
    /// [`Cell::bg_color_palette`] (looked up in the palette), or the
    /// style's [`bg_color`][Style::bg_color].
    ///
    /// Returns `None` if the cell has no background color, in which case the
    /// caller should use whatever default background color it wants
    /// (e.g. the terminal background).
    pub fn bg_color(&self) -> Result<Option<RgbColor>> {
        let res = self.get::<ffi::ColorRgb>(ffi::RenderStateRowCellsData::BG_COLOR);
        match res {
            Ok(o) => Ok(Some(o.into())),
            Err(Error::InvalidValue) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Get the grapheme codepoints.
    ///
    /// The base codepoint is placed first, followed by any extra codepoints.
    pub fn graphemes(&self) -> Result<Vec<char>> {
        let len = self.graphemes_len()?;
        let mut graphemes = vec!['\0'; len];
        self.graphemes_buf(&mut graphemes)?;
        Ok(graphemes)
    }

    /// The total number of grapheme codepoints including the base codepoint.
    ///
    /// Returns 0 if the cell has no text.
    pub fn graphemes_len(&self) -> Result<usize> {
        self.get(ffi::RenderStateRowCellsData::GRAPHEMES_LEN)
    }

    /// Write grapheme codepoints into a caller-provided buffer.
    ///
    /// The buffer must be at least [`CellIteration::graphemes_len`] elements.
    /// The base codepoint is written first, followed by any extra codepoints.
    pub fn graphemes_buf(&self, buf: &mut [char]) -> Result<()> {
        let result = unsafe {
            ffi::ghostty_render_state_row_cells_get(
                self.iter.0.as_raw(),
                ffi::RenderStateRowCellsData::GRAPHEMES_BUF,
                buf.as_mut_ptr().cast(),
            )
        };
        from_result(result)
    }

    /// Encode the current cell's full grapheme cluster as UTF-8 into a
    /// caller-provided string buffer.
    ///
    /// The base codepoint is encoded first, followed by any extra grapheme
    /// codepoints.
    ///
    /// May grow the buffer if more space is required.
    pub fn graphemes_utf8(&self, buf: &mut String) -> Result<()> {
        // SAFETY: String comes with some very stringent safety requirements,
        // so we'll detail them here. The safety protocol for the C API is
        // essentially that, in case of an error, no data will be written
        // to the String's underlying buffer, and the buffer should appear
        // as if unmodified. As such, we should be fine to operate on the
        // original buffer directly and not cause any UB or break any
        // invariants with the String's internal state.
        //
        // Since Strings do not have a `set_len` method like Vecs, in the
        // happy path we have to recombine the entire string from its
        // constituents, i.e. its pointer, length and capacity. This should
        // be fine as the pointer indeed came from the original String,
        // and that we do not attempt to copy the pointer anywhere and
        // potentially cause aliasing issues. As for the remaining factors,
        // we have to trust that the API will not cause length and capacity
        // to have nonsensical values, and that the underlying bytes are
        // indeed UTF-8.
        //
        // TODO: Use `String::into_raw_parts` to make this slightly simpler

        let cbuf = loop {
            // Save the old length of the String for later
            let len = buf.len();
            let mut cbuf = ffi::Buffer {
                ptr: buf.as_mut_ptr(),
                cap: buf.capacity(),
                len,
            };

            let result = unsafe {
                ffi::ghostty_render_state_row_cells_get(
                    self.iter.0.as_raw(),
                    ffi::RenderStateRowCellsData::GRAPHEMES_UTF8,
                    std::ptr::from_mut(&mut cbuf).cast(),
                )
            };
            match result {
                ffi::Result::SUCCESS => break Ok(cbuf),
                ffi::Result::OUT_OF_MEMORY => break Err(Error::OutOfMemory),
                ffi::Result::OUT_OF_SPACE => {
                    // When OutOfSpace is returned, the new length is written
                    // to `cbuf.len`, so we reserve additional space for that
                    buf.reserve(cbuf.len - len);
                    continue;
                }
                ffi::Result::NO_VALUE | ffi::Result::INVALID_VALUE | _ => {
                    break Err(Error::InvalidValue);
                }
            };
        }?;

        // Reconstitute the original String
        // WITHOUT DROPPING THE EXISTING STRING OBJECT (!!)
        // Otherwise, memory corruption, double frees, etc. WILL happen.
        unsafe {
            std::ptr::write(buf, String::from_raw_parts(cbuf.ptr, cbuf.len, cbuf.cap));
        }
        Ok(())
    }

    /// Whether the cell is contained within the current selection.
    ///
    /// This returns true when the cell's column is within the current row's
    /// row-local selection range, and false otherwise. Rendering policy for
    /// selected cells (colors, inversion, etc.) is left to the caller.
    ///
    /// Renderers that can draw cells in spans may be more efficient calling
    /// [`RowIteration::selection`] once per row and applying that range
    /// directly, avoiding one C API call per cell for selection state.
    pub fn is_selected(&self) -> Result<bool> {
        self.get(ffi::RenderStateRowCellsData::SELECTED)
    }

    /// Whether the cell has any explicit styling.
    ///
    /// This is equivalent to querying the raw cell's [`Cell::has_styling`]
    /// value, but avoids materializing the raw [`Cell`] for renderers that
    /// only need to know whether fetching the full style is necessary.
    pub fn has_styling(&self) -> Result<bool> {
        self.get(ffi::RenderStateRowCellsData::HAS_STYLING)
    }
}

//---------------------------
// Auxiliary types
//---------------------------

/// Cursor viewport position information.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorViewport {
    /// Cursor viewport x position in cells.
    pub x: u16,
    /// Cursor viewport y position in cells.
    pub y: u16,
    /// Whether the cursor is on the tail of a wide character.
    pub at_wide_tail: bool,
}

/// Render-state color information.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Colors {
    /// The default/current background color for the render state.
    pub background: RgbColor,
    /// The default/current foreground color for the render state.
    pub foreground: RgbColor,
    /// The cursor color which may be explicitly set by terminal state.
    pub cursor: Option<RgbColor>,
    /// The active 256-color palette for this render state.
    pub palette: [RgbColor; 256],
}

/// Dirty state of a render state after update.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
pub enum Dirty {
    /// Not dirty at all; rendering can be skipped.
    Clean = ffi::RenderStateDirty::FALSE,
    /// Some rows changed; renderer can redraw incrementally.
    Partial = ffi::RenderStateDirty::PARTIAL,
    /// Global state changed; renderer should redraw everything.
    Full = ffi::RenderStateDirty::FULL,
}

/// Visual style of the cursor.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
#[non_exhaustive]
pub enum CursorVisualStyle {
    /// Bar cursor (DECSCUSR 5, 6).
    Bar = ffi::RenderStateCursorVisualStyle::BAR,
    /// Block cursor (DECSCUSR 1, 2).
    Block = ffi::RenderStateCursorVisualStyle::BLOCK,
    /// Underline cursor (DECSCUSR 3, 4).
    Underline = ffi::RenderStateCursorVisualStyle::UNDERLINE,
    /// Hollow block cursor.
    BlockHollow = ffi::RenderStateCursorVisualStyle::BLOCK_HOLLOW,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::{Options, Terminal};

    /// Guards the `set_dirty` → `update` → `dirty()` round-trip. If
    /// `Snapshot::set(value: &T)` calls `from_ref(&value)`, the result has
    /// type `*const &T` (a pointer to the local reference), not `*const T`.
    /// C reads stack-address bytes into the dirty field, the next `update`
    /// propagates them, and `dirty()` fails enum decoding.
    #[test]
    fn dirty_decodes_after_set_dirty_then_update() {
        let terminal = Terminal::new(Options {
            cols: 8,
            rows: 3,
            max_scrollback: 0,
        })
        .unwrap();
        let mut state = RenderState::new().unwrap();

        state
            .update(&terminal)
            .unwrap()
            .set_dirty(Dirty::Clean)
            .unwrap();

        assert!(state.update(&terminal).unwrap().dirty().is_ok());
    }
}
