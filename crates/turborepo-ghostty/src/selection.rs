//! Selecting terminal content between two endpoints.
//!
//! The start and end values are [`GridRef`] values. They are therefore
//! untracked grid references and inherit the same lifetime rules: they are
//! only safe to use until the next mutating operation on the terminal that
//! produced them, including dropping the terminal. To keep a selection valid
//! across terminal mutations, callers must maintain tracked grid references
//! for the endpoints and reconstruct a [`Selection`] from fresh snapshots
//! when needed.
//!
//! Selections can be directly obtained by calling methods such as
//! [`Terminal::select_all`], [`Terminal::select_word`], etc., but this can be
//! quite cumbersome to use for terminal emulators designed for human users.
//! For this use case, [selection gestures](self::gesture) serve as a convenient
//! way of translating common UI actions (clicking, dragging, etc.) into
//! selections, to be copied, formatted, or installed as the active selection.
use std::{marker::PhantomData, ptr::NonNull};

use crate::{
    alloc::{Allocator, Bytes},
    error::{Error, Result, from_optional_result, from_optional_result_with_len, from_result},
    ffi,
    fmt::Format,
    screen::GridRef,
    terminal::{Point, Terminal},
};

/// A snapshot selection range defined by two grid references.
///
/// # Preconditions
///
/// For every method that interacts with the terminal,
/// the selection's start and end grid refs must both be valid untracked
/// snapshots for the given terminal's currently active screen. In practice,
/// they must come from that terminal and screen, and no mutating terminal call
/// may have occurred since the refs were produced or reconstructed from
/// tracked refs. Passing refs from another terminal, another screen, or stale
/// refs violates this precondition.
#[derive(Clone, Debug)]
pub struct Selection<'t> {
    pub(crate) inner: ffi::Selection,
    _phan: PhantomData<&'t ffi::Terminal>,
}
impl<'t> Selection<'t> {
    /// Create a new selection between two endpoints.
    ///
    /// Both endpoints are inclusive. The endpoints preserve selection direction
    /// and may be reversed; callers must not assume that start is the top-left
    /// endpoint or that end is the bottom-right endpoint.
    ///
    /// When `rectangle` is false, the endpoints describe a linear selection.
    /// When `rectangle` is true, the same endpoints are interpreted as
    /// opposite corners of a rectangular/block selection.
    pub fn new(start: GridRef<'t>, end: GridRef<'t>, rectangle: bool) -> Self {
        // SAFETY: provided by the type system
        unsafe {
            Self::from_raw(ffi::Selection {
                start: start.inner,
                end: end.inner,
                rectangle,
                ..ffi::sized!(ffi::Selection)
            })
        }
    }

    /// # Safety
    ///
    /// Caller must guarantee that the selection is bound by the lifetime `'t`.
    pub(crate) unsafe fn from_raw(value: ffi::Selection) -> Self {
        Self {
            inner: value,
            _phan: PhantomData,
        }
    }

    /// Start of the selection range (inclusive).
    ///
    /// This may be before start in terminal order. It is an untracked
    /// [`GridRef`] snapshot and follows untracked grid-ref lifetime rules.
    pub fn start(&self) -> GridRef<'t> {
        unsafe { GridRef::from_raw(self.inner.start) }
    }
    /// End of the selection range (inclusive).
    ///
    /// This may be before start in terminal order. It is an untracked
    /// [`GridRef`] snapshot and follows untracked grid-ref lifetime rules.
    pub fn end(&self) -> GridRef<'t> {
        unsafe { GridRef::from_raw(self.inner.end) }
    }
    /// Whether the endpoints are interpreted as a rectangular/block
    /// selection rather than a linear selection.
    pub fn is_rectangle(&self) -> bool {
        self.inner.rectangle
    }

    /// Adjust a selection snapshot using terminal selection semantics.
    ///
    /// The logical end endpoint is always moved, regardless of whether the
    /// selection is forward or reversed visually. The input selection remains
    /// a snapshot: after adjustment, call [`Terminal::set_selection`] to
    /// install it as the terminal-owned selection if desired.
    ///
    /// See [#Preconditions](#preconditions) for the necessary preconditions.
    pub fn adjust(&mut self, terminal: &'t Terminal<'_, '_>, adjustment: Adjustment) -> Result<()> {
        let result = unsafe {
            ffi::ghostty_terminal_selection_adjust(
                terminal.inner.as_raw(),
                &raw mut self.inner,
                adjustment.into(),
            )
        };
        from_result(result)
    }

    /// Test whether a terminal point is inside a selection snapshot.
    ///
    /// This uses the same selection semantics as the terminal, including
    /// rectangular/block selections and linear selections spanning multiple
    /// rows.
    ///
    /// See [#Preconditions](#preconditions) for the necessary preconditions.
    pub fn contains(&self, terminal: &'t Terminal<'_, '_>, point: Point) -> Result<bool> {
        let mut contains = false;
        let result = unsafe {
            ffi::ghostty_terminal_selection_contains(
                terminal.inner.as_raw(),
                &self.inner,
                point.into(),
                &raw mut contains,
            )
        };
        from_result(result)?;
        Ok(contains)
    }

    /// Test whether two selection snapshots are equal.
    ///
    /// Equality uses the terminal's internal selection semantics: both endpoint
    /// pins must match and both selections must have the same rectangular/block
    /// state. This avoids requiring callers to compare raw [`GridRef`]
    /// internals.
    ///
    /// See [#Preconditions](#preconditions) for the necessary preconditions.
    pub fn equals(&self, terminal: &'t Terminal<'_, '_>, other: &Self) -> Result<bool> {
        let mut equal = false;
        let result = unsafe {
            ffi::ghostty_terminal_selection_equal(
                terminal.inner.as_raw(),
                &self.inner,
                &other.inner,
                &raw mut equal,
            )
        };
        from_result(result)?;
        Ok(equal)
    }

    /// Get the current endpoint ordering of a selection snapshot.
    ///
    /// See [#Preconditions](#preconditions) for the necessary preconditions.
    pub fn order(&self, terminal: &'t Terminal<'_, '_>) -> Result<Order> {
        let mut order = ffi::SelectionOrder::FORWARD;

        let result = unsafe {
            ffi::ghostty_terminal_selection_order(
                terminal.inner.as_raw(),
                &self.inner,
                &raw mut order,
            )
        };
        from_result(result)?;
        Order::try_from(order).map_err(|_| Error::InvalidValue)
    }

    /// Return a selection snapshot with endpoints ordered as requested.
    ///
    /// Use [`Order::Forward`] to get top-left to bottom-right bounds,
    /// and [`Order::Reverse`] to get bottom-right to top-left bounds.
    /// Mirrored desired orders are accepted but normalized the same as forward.
    /// The output selection is a fresh untracked snapshot and is not installed
    /// as the terminal's current selection.
    ///
    /// See [#Preconditions](#preconditions) for the necessary preconditions.
    pub fn to_ordered(&self, terminal: &'t Terminal<'_, '_>, desired: Order) -> Result<Self> {
        let mut selection = ffi::sized!(ffi::Selection);
        let result = unsafe {
            ffi::ghostty_terminal_selection_ordered(
                terminal.inner.as_raw(),
                &self.inner,
                desired.into(),
                &raw mut selection,
            )
        };
        from_result(result)?;
        Ok(unsafe { Self::from_raw(selection) })
    }
}

/// Methods related to [selections](crate::selection).
impl Terminal<'_, '_> {
    /// Set the active screen selection.
    ///
    /// The selection's grid references must be valid for this terminal's
    /// active screen at the time of the call.
    ///
    /// Passing `None` clears the active screen selection.
    ///
    /// This function does not take `&mut self` since it does not invalidate
    /// any state that relies on the terminal.
    pub fn set_selection(&self, selection: Option<&Selection<'_>>) -> Result<&Self> {
        self.set_optional(ffi::TerminalOption::SELECTION, selection.map(|v| &v.inner))?;
        Ok(self)
    }

    /// Derive a selection snapshot covering all selectable terminal content.
    ///
    /// The returned selection is not installed as the terminal's current
    /// selection.
    pub fn select_all(&self) -> Result<Option<Selection<'_>>> {
        let mut value = ffi::sized!(ffi::Selection);
        let result =
            unsafe { ffi::ghostty_terminal_select_all(self.inner.as_raw(), &raw mut value) };

        let sel = from_optional_result(result, value)?;
        Ok(sel.map(|v| unsafe {
            // SAFETY: Selection should be initialized and valid on success
            Selection::from_raw(v)
        }))
    }
    /// Derive a selection snapshot covering all selectable terminal content.
    ///
    /// The returned selection is not installed as the terminal's current
    /// selection.
    pub fn select_line(&self, options: SelectLineOptions) -> Result<Option<Selection<'_>>> {
        let mut value = ffi::sized!(ffi::Selection);

        let result = unsafe {
            ffi::ghostty_terminal_select_line(self.inner.as_raw(), &options.inner, &raw mut value)
        };

        let sel = from_optional_result(result, value)?;
        Ok(sel.map(|v| unsafe {
            // SAFETY: Selection should be initialized and valid on success
            Selection::from_raw(v)
        }))
    }
    /// Derive a command-output selection snapshot from a terminal grid
    /// reference.
    ///
    /// The returned selection is not installed as the terminal's current
    /// selection.
    pub fn select_output(&self, grid_ref: GridRef<'_>) -> Result<Option<Selection<'_>>> {
        let mut value = ffi::sized!(ffi::Selection);

        let result = unsafe {
            ffi::ghostty_terminal_select_output(self.inner.as_raw(), grid_ref.inner, &raw mut value)
        };

        let sel = from_optional_result(result, value)?;
        Ok(sel.map(|v| unsafe {
            // SAFETY: Selection should be initialized and valid on success
            Selection::from_raw(v)
        }))
    }
    /// Derive a word selection snapshot from a terminal grid reference.
    ///
    /// The returned selection is not installed as the terminal's current
    /// selection.
    pub fn select_word(&self, options: SelectWordOptions) -> Result<Option<Selection<'_>>> {
        let mut value = ffi::sized!(ffi::Selection);

        let result = unsafe {
            ffi::ghostty_terminal_select_word(self.inner.as_raw(), &options.inner, &raw mut value)
        };

        let sel = from_optional_result(result, value)?;
        Ok(sel.map(|v| unsafe {
            // SAFETY: Selection should be initialized and valid on success
            Selection::from_raw(v)
        }))
    }

    /// Derive the nearest word selection snapshot between two
    /// terminal grid refs.
    ///
    /// Starting at `options.start`, this searches toward `options.end`
    /// (inclusive) and returns the first selectable word found using
    /// Ghostty's word-selection rules.
    ///
    /// This is useful for implementing double-click-and-drag selection in a UI.
    /// If a user double-clicks one word and drags across spaces or punctuation
    /// toward another word, selecting only the word directly under the current
    /// pointer can flicker or collapse when the pointer is between words.
    /// Instead, ask for the nearest word between the original click and the
    /// drag point, ask again in the reverse direction, and combine the two word
    /// bounds into the drag selection.
    ///
    /// The returned selection is not installed as the terminal's current
    /// selection.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use libghostty_vt::{
    ///     Error,
    ///     terminal::{Terminal, Point, PointCoordinate},
    ///     screen::GridRef,
    ///     selection::{Selection, SelectWordBetweenOptions},
    /// };
    /// # use libghostty_vt::TerminalOptions;
    /// # fn main() -> libghostty_vt::error::Result<()> {
    /// # let terminal = Terminal::new(TerminalOptions { cols: 80, rows: 24, max_scrollback: 0 }).unwrap();
    ///
    /// // Double-click-and-drag style selection. Suppose the user double-clicks
    /// // "git" and drags to "status". The pointer may pass over whitespace, so
    /// // select the nearest word between the original click and current drag point
    /// // in both directions, then combine the outer word bounds.
    /// fn ref_at<'t>(terminal: &'t Terminal<'_, '_>, x: u16, y: u32) -> Result<GridRef<'t>, Error> {
    ///     terminal.grid_ref(Point::Active(PointCoordinate { x, y }))
    /// }
    ///
    /// let click_ref = ref_at(&terminal, 2, 0)?; // the "git" in "git status"
    /// let drag_ref = ref_at(&terminal, 6, 0)?;  // the "status" in "git status"
    ///
    /// let start_word = terminal.select_word_between(
    ///     SelectWordBetweenOptions::new(click_ref.clone(), drag_ref.clone())
    /// )?;
    ///
    /// let end_word = terminal.select_word_between(
    ///     SelectWordBetweenOptions::new(drag_ref, click_ref)
    /// )?;
    ///
    /// let drag_selection = Selection::new(
    ///     start_word.unwrap().start(),
    ///     end_word.unwrap().end(),
    ///     false,
    /// );
    /// # Ok(())}
    /// ```
    pub fn select_word_between(
        &self,
        options: SelectWordBetweenOptions,
    ) -> Result<Option<Selection<'_>>> {
        let mut value = ffi::sized!(ffi::Selection);

        let result = unsafe {
            ffi::ghostty_terminal_select_word_between(
                self.inner.as_raw(),
                &options.inner,
                &raw mut value,
            )
        };

        let sel = from_optional_result(result, value)?;
        Ok(sel.map(|v| unsafe {
            // SAFETY: Selection should be initialized and valid on success
            Selection::from_raw(v)
        }))
    }

    /// Format a terminal selection into an allocated buffer.
    ///
    /// This is a one-shot convenience API for formatting either the terminal's
    /// active selection or a caller-provided [`Selection`] without explicitly
    /// creating a [`Formatter`](crate::fmt::Formatter).
    ///
    /// The returned buffer is allocated using allocator, or the default
    /// allocator if `None` is passed. The returned bytes are not
    /// NUL-terminated. This supports plain text, VT, and HTML uniformly as
    /// byte output.
    ///
    /// If `options.selection` is `None` and the terminal has no active
    /// selection, the function returns `None`.
    pub fn format_selection_alloc<'a, 'ctx: 'a>(
        &self,
        alloc: Option<&'a Allocator<'ctx>>,
        options: FormatOptions,
    ) -> Result<Option<Bytes<'a>>> {
        let mut out = std::ptr::null_mut();
        let mut out_len = 0usize;
        let alloc = alloc.map_or(std::ptr::null(), |v| v.to_raw());

        let result = unsafe {
            ffi::ghostty_terminal_selection_format_alloc(
                self.inner.as_raw(),
                alloc,
                options.inner,
                &raw mut out,
                &raw mut out_len,
            )
        };

        let out = from_optional_result(result, out)?;
        Ok(out
            .and_then(NonNull::new)
            .map(|ptr| unsafe { Bytes::from_raw_parts(ptr, out_len, alloc) }))
    }

    /// Format a terminal selection into a caller-provided buffer.
    ///
    /// This is a one-shot convenience API for formatting either the terminal's
    /// active selection or a caller-provided [`Selection`] without explicitly
    /// creating a [`Formatter`](crate::fmt::Formatter).
    ///
    /// If `buf` is too small, this returns `Err(Error::OutOfSpace { required
    /// })` where `required` is the required size. The caller can then retry
    /// with a larger buffer.
    ///
    /// If `options.selection` is `None` and the terminal has no active
    /// selection, the function returns `None`.
    pub fn format_selection_buf(
        &self,
        options: FormatOptions,
        buf: &mut [u8],
    ) -> Result<Option<usize>> {
        let mut written = 0usize;

        let result = unsafe {
            ffi::ghostty_terminal_selection_format_buf(
                self.inner.as_raw(),
                options.inner,
                buf.as_mut_ptr(),
                buf.len(),
                &raw mut written,
            )
        };

        from_optional_result_with_len(result, written)
    }
}

/// Options for [deriving a line selection](Terminal::select_line)
/// from a terminal grid reference.
///
/// If [`with_whitespace`](Self::with_whitespace) is not called,
/// Ghostty's default line-trim whitespace codepoints are used.
#[derive(Clone, Debug)]
pub struct SelectLineOptions<'t, 'ws> {
    inner: ffi::TerminalSelectLineOptions,
    _phan: (PhantomData<&'t ffi::Terminal>, PhantomData<&'ws [char]>),
}
impl<'t, 'ws> SelectLineOptions<'t, 'ws> {
    /// Create a new set of options for [deriving a line
    /// selection](Terminal::select_line), from the given grid reference.
    pub fn new(grid_ref: GridRef<'t>) -> Self {
        Self {
            inner: ffi::TerminalSelectLineOptions {
                ref_: grid_ref.inner,
                whitespace: std::ptr::null(),
                whitespace_len: 0,
                semantic_prompt_boundary: false,
                ..ffi::sized!(ffi::TerminalSelectLineOptions)
            },
            _phan: (PhantomData, PhantomData),
        }
    }

    /// Specify the codepoints to trim from the start and end of the line.
    pub fn with_whitespace(mut self, value: &'ws [char]) -> Self {
        // Note: it's always safe to reinterpret char as a u32,
        // as long as no mutation occurs.
        self.inner.whitespace = value.as_ptr().cast();
        self.inner.whitespace_len = value.len();
        self
    }

    /// Specify whether semantic prompt state changes should bound the line
    /// selection.
    pub fn with_semantic_prompt_boundary(mut self, value: bool) -> Self {
        self.inner.semantic_prompt_boundary = value;
        self
    }
}

/// Options for [deriving a word selection](Terminal::select_word)
/// from a terminal grid reference.
///
/// If [`with_boundary_codepoints`](Self::with_boundary_codepoints)
/// is not called, Ghostty's default word-boundary codepoints are used.
#[derive(Clone, Debug)]
pub struct SelectWordOptions<'t, 'bc> {
    inner: ffi::TerminalSelectWordOptions,
    _phan: (PhantomData<&'t ffi::Terminal>, PhantomData<&'bc [char]>),
}
impl<'t, 'bc> SelectWordOptions<'t, 'bc> {
    /// Create a new set of options for [deriving a word
    /// selection](Terminal::select_word), from the given grid reference.
    pub fn new(grid_ref: GridRef<'t>) -> Self {
        Self {
            inner: ffi::TerminalSelectWordOptions {
                ref_: grid_ref.inner,
                ..ffi::sized!(ffi::TerminalSelectWordOptions)
            },
            _phan: (PhantomData, PhantomData),
        }
    }

    /// Specify the word-boundary codepoints.
    pub fn with_boundary_codepoints(mut self, value: &'bc [char]) -> Self {
        // Note: it's always safe to reinterpret char as a u32,
        // as long as no mutation occurs.
        self.inner.boundary_codepoints = value.as_ptr().cast();
        self.inner.boundary_codepoints_len = value.len();
        self
    }
}

/// Options for [deriving the nearest word
/// selection](Terminal::select_word_between) between two grid references.
///
/// If [`with_boundary_codepoints`](Self::with_boundary_codepoints)
/// is not called, Ghostty's default word-boundary codepoints are used.
#[derive(Debug)]
pub struct SelectWordBetweenOptions<'t, 'bc> {
    inner: ffi::TerminalSelectWordBetweenOptions,
    _phan: (PhantomData<&'t ffi::Terminal>, PhantomData<&'bc [char]>),
}
impl<'t, 'bc> SelectWordBetweenOptions<'t, 'bc> {
    /// Create a new set of options for
    /// [deriving the nearest word selection](Terminal::select_word_between),
    /// from the two given grid references.
    pub fn new(start: GridRef<'t>, end: GridRef<'t>) -> Self {
        Self {
            inner: ffi::TerminalSelectWordBetweenOptions {
                start: start.inner,
                end: end.inner,
                ..ffi::sized!(ffi::TerminalSelectWordBetweenOptions)
            },
            _phan: (PhantomData, PhantomData),
        }
    }

    /// Specify the word-boundary codepoints.
    pub fn with_boundary_codepoints(mut self, value: &'bc [char]) -> Self {
        // Note: it's always safe to reinterpret char as a u32,
        // as long as no mutation occurs.
        self.inner.boundary_codepoints = value.as_ptr().cast();
        self.inner.boundary_codepoints_len = value.len();
        self
    }
}

/// Options for [one-shot formatting of a terminal
/// selection](Terminal::format_selection_alloc).
///
/// If [`with_selection`](Self::with_selection) is not called, the formatter
/// defaults to formatting the terminal's active selection. If there is no
/// active selection, formatting returns `Ok(None)`.
///
/// The selection is formatted from the terminal's active screen using the same
/// formatting semantics as [`Formatter`](crate::fmt::Formatter).
/// For copy/clipboard behavior matching Ghostty's Screen.selectionString(),
/// use plain output with unwrap and trim both set to true.
#[derive(Debug)]
pub struct FormatOptions<'t, 's> {
    inner: ffi::TerminalSelectionFormatOptions,
    _phan: PhantomData<&'s Selection<'t>>,
}
impl<'t, 's> FormatOptions<'t, 's> {
    /// Create a new set of options for one-shot formatting of a
    /// terminal selection.
    pub fn new() -> Self {
        Self {
            inner: ffi::TerminalSelectionFormatOptions {
                ..ffi::sized!(ffi::TerminalSelectionFormatOptions)
            },
            _phan: PhantomData,
        }
    }
    /// Specify the output format to emit.
    pub fn with_emit_format(mut self, value: Format) -> Self {
        self.inner.emit = value.into();
        self
    }
    /// Specify whether to unwrap soft-wrapped lines.
    pub fn with_unwrap(mut self, value: bool) -> Self {
        self.inner.unwrap = value;
        self
    }
    /// Specify whether to trim trailing whitespace on non-blank lines.
    pub fn with_trim(mut self, value: bool) -> Self {
        self.inner.trim = value;
        self
    }
    /// Specify the selection to format in place of the terminal's active
    /// selection.
    ///
    /// The selection must be a [valid snapshot
    /// selection](Selection#preconditions) for this terminal.
    pub fn with_selection(mut self, value: &'s Selection<'t>) -> Self {
        self.inner.selection = &value.inner;
        self
    }
}
impl Default for FormatOptions<'_, '_> {
    fn default() -> Self {
        Self::new()
    }
}

/// Operation used to adjust a selection endpoint.
///
/// Adjustment mutates the selection's logical end endpoint, not whichever
/// endpoint is visually bottom/right. This preserves keyboard and drag behavior
/// for both forward and reversed selections.
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
#[repr(u32)]
#[non_exhaustive]
pub enum Adjustment {
    /// Move left to the previous non-empty cell, wrapping upward.
    Left = ffi::SelectionAdjust::LEFT,
    /// Move right to the next non-empty cell, wrapping downward.
    Right = ffi::SelectionAdjust::RIGHT,
    /// Move up one row at the current column,
    /// or to the beginning of the line if already at the top.
    Up = ffi::SelectionAdjust::UP,
    /// Move down to the next non-blank row at the current column,
    /// or to the end of the line if none exists.
    Down = ffi::SelectionAdjust::DOWN,
    /// Move to the top-left cell of the screen.
    Home = ffi::SelectionAdjust::HOME,
    /// Move to the right edge of the last non-blank row on the screen.
    End = ffi::SelectionAdjust::END,
    /// Move up by one terminal page height,
    /// or to home if that would move past the top.
    PageUp = ffi::SelectionAdjust::PAGE_UP,
    /// Move down by one terminal page height,
    /// or to end if that would move past the bottom.
    PageDown = ffi::SelectionAdjust::PAGE_DOWN,
    /// Move to the left edge of the current line.
    BeginningOfLine = ffi::SelectionAdjust::BEGINNING_OF_LINE,
    /// Move to the right edge of the current line.
    EndOfLine = ffi::SelectionAdjust::END_OF_LINE,
}

/// Ordering of a selection's endpoints in terminal coordinates.
///
/// Mirrored orders are only produced by rectangular selections whose start
/// and end endpoints are on opposite diagonal corners that are not simple
/// top-left-to-bottom-right or bottom-right-to-top-left orderings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
#[repr(u32)]
#[non_exhaustive]
pub enum Order {
    /// Start is before end in top-left to bottom-right order.
    Forward = ffi::SelectionOrder::FORWARD,
    /// End is before start in top-left to bottom-right order.
    Reverse = ffi::SelectionOrder::REVERSE,
    /// Rectangular selection from top-right to bottom-left.
    MirroredForward = ffi::SelectionOrder::MIRRORED_FORWARD,
    /// Rectangular selection from bottom-left to top-right.
    MirroredReverse = ffi::SelectionOrder::MIRRORED_REVERSE,
}
