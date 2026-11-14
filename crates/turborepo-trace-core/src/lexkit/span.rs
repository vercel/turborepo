use std::ops::Range;

/// A half-open byte range `start..end` into a source buffer.
///
/// Offsets are `u32`, so sources larger than 4 GiB are not supported.
/// Constructors saturate instead of panicking. Line and column are computed
/// lazily with [`crate::lexkit::LineIndex`].
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Span {
    /// Inclusive start offset.
    pub start: u32,
    /// Exclusive end offset.
    pub end: u32,
}

impl Span {
    /// An empty span at offset 0, for synthesized references without a source
    /// location.
    pub const DUMMY: Span = Span { start: 0, end: 0 };

    /// Creates a span from `start` to `end`.
    #[inline]
    pub const fn new(start: u32, end: u32) -> Self {
        Span { start, end }
    }

    /// Creates a span from `usize` offsets, saturating at `u32::MAX`.
    #[inline]
    pub fn from_usize(start: usize, end: usize) -> Self {
        Span {
            start: u32::try_from(start).unwrap_or(u32::MAX),
            end: u32::try_from(end).unwrap_or(u32::MAX),
        }
    }

    /// Length in bytes (0 when `end < start`).
    #[inline]
    pub const fn len(self) -> u32 {
        self.end.saturating_sub(self.start)
    }

    /// Whether the span covers no bytes.
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }

    /// The span as a `usize` range.
    #[inline]
    pub const fn range(self) -> Range<usize> {
        self.start as usize..self.end as usize
    }

    /// Smallest span covering both `self` and `other`.
    #[inline]
    pub fn to(self, other: Span) -> Span {
        Span::new(self.start.min(other.start), self.end.max(other.end))
    }

    /// Returns the covered bytes of `src`, or `None` if the span is out of
    /// bounds or inverted.
    #[inline]
    pub fn slice(self, src: &[u8]) -> Option<&[u8]> {
        src.get(self.range())
    }
}

impl From<Range<usize>> for Span {
    fn from(r: Range<usize>) -> Self {
        Span::from_usize(r.start, r.end)
    }
}
