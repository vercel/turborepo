use std::{
    borrow::Cow,
    collections::VecDeque,
    ops::{Add, Mul},
};

use itertools::Itertools as _;

use crate::{
    encode,
    token::{self, Separator, Token},
    PATHS_ARE_CASE_INSENSITIVE,
};

pub trait Invariance:
    Add<Self, Output = Self> + Eq + Mul<usize, Output = Self> + PartialEq<Self> + Sized
{
    fn empty() -> Self;
}

pub trait UnitVariance<T> {
    fn unit_variance(self) -> Variance<T>;
}

impl<T> UnitVariance<T> for Variance<T> {
    fn unit_variance(self) -> Variance<T> {
        self
    }
}

pub trait ConjunctiveVariance<T>: Iterator + Sized
where
    Self::Item: UnitVariance<T>,
    T: Invariance,
{
    fn conjunctive_variance(self) -> Variance<T> {
        self.map(UnitVariance::unit_variance)
            .reduce(Add::add)
            .unwrap_or_else(|| Variance::Invariant(T::empty()))
    }
}

impl<T, I> ConjunctiveVariance<T> for I
where
    I: Iterator,
    I::Item: UnitVariance<T>,
    T: Invariance,
{
}

pub trait DisjunctiveVariance<T>: Iterator + Sized
where
    Self::Item: UnitVariance<T>,
    T: Invariance,
{
    fn disjunctive_variance(self) -> Variance<T> {
        // TODO: This implementation is incomplete. Unbounded variance (and
        //       unbounded depth) are "infectious" when disjunctive. If any unit
        //       variance is variant and unbounded (open), then the disjunctive
        //       variance should be the same.
        // There are three distinct possibilities for disjunctive variance.
        //
        //   - The iterator is empty and there are no unit variances to consider. The
        //     disjunctive variance is the empty invariant.
        //   - The iterator is non-empty and all unit variances are equal. The
        //     disjunctive variance is the same as any of the like unit variances.
        //   - The iterator is non-empty and the unit variances are **not** all equal.
        //     The disjunctive variance is variant and bounded (closed).
        let mut variances = self.map(UnitVariance::unit_variance).fuse();
        let first = variances
            .next()
            .unwrap_or_else(|| Variance::Invariant(T::empty()));
        if variances.all(|variance| first == variance) {
            first
        } else {
            Variance::Variant(Boundedness::Closed)
        }
    }
}

impl<T, I> DisjunctiveVariance<T> for I
where
    I: Iterator,
    I::Item: UnitVariance<T>,
    T: Invariance,
{
}

pub trait UnitDepth: Sized {
    fn unit_depth(self) -> Boundedness {
        Boundedness::Closed
    }
}

impl UnitDepth for Boundedness {
    fn unit_depth(self) -> Boundedness {
        self
    }
}

pub trait CompositeDepth: Iterator + Sized {
    fn composite_depth(self) -> Boundedness;
}

impl<I> CompositeDepth for I
where
    I: Iterator,
    I::Item: UnitDepth,
{
    fn composite_depth(self) -> Boundedness {
        if self.map(UnitDepth::unit_depth).any(|depth| depth.is_open()) {
            Boundedness::Open
        } else {
            Boundedness::Closed
        }
    }
}

pub trait UnitBreadth: Sized {
    fn unit_breadth(self) -> Boundedness {
        Boundedness::Closed
    }
}

impl UnitBreadth for Boundedness {
    fn unit_breadth(self) -> Boundedness {
        self
    }
}

pub trait CompositeBreadth: Iterator + Sized {
    fn composite_breadth(self) -> Boundedness;
}

impl<I> CompositeBreadth for I
where
    I: Iterator,
    I::Item: UnitBreadth,
{
    fn composite_breadth(self) -> Boundedness {
        if self
            .map(UnitBreadth::unit_breadth)
            .any(|breadth| breadth.is_open())
        {
            Boundedness::Open
        } else {
            Boundedness::Closed
        }
    }
}

pub trait IntoInvariantText<'t> {
    fn into_nominal_text(self) -> InvariantText<'t>;

    fn into_structural_text(self) -> InvariantText<'t>;
}

impl<'t> IntoInvariantText<'t> for Cow<'t, str> {
    fn into_nominal_text(self) -> InvariantText<'t> {
        InvariantFragment::Nominal(self).into()
    }

    fn into_structural_text(self) -> InvariantText<'t> {
        InvariantFragment::Structural(self).into()
    }
}

impl IntoInvariantText<'static> for String {
    fn into_nominal_text(self) -> InvariantText<'static> {
        InvariantFragment::Nominal(self.into()).into()
    }

    fn into_structural_text(self) -> InvariantText<'static> {
        InvariantFragment::Structural(self.into()).into()
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct InvariantSize(usize);

impl InvariantSize {
    pub const fn new(n: usize) -> Self {
        InvariantSize(n)
    }
}

impl Add for InvariantSize {
    type Output = Self;

    fn add(self, other: Self) -> Self::Output {
        InvariantSize(self.0 + other.0)
    }
}

impl From<InvariantSize> for usize {
    fn from(size: InvariantSize) -> Self {
        size.0
    }
}

impl From<usize> for InvariantSize {
    fn from(n: usize) -> Self {
        InvariantSize(n)
    }
}

impl Invariance for InvariantSize {
    fn empty() -> Self {
        InvariantSize(0)
    }
}

impl Mul<usize> for InvariantSize {
    type Output = Self;

    fn mul(self, n: usize) -> Self::Output {
        InvariantSize(
            self.0
                .checked_mul(n)
                .expect("overflow determining invariant size"),
        )
    }
}

// TODO: The derived `PartialEq` implementation is incomplete and does not
// detect contiguous like       fragments that are equivalent to an aggregated
// fragment. This works, but relies on       constructing `InvariantText` by
// consistently appending fragments.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvariantText<'t> {
    fragments: VecDeque<InvariantFragment<'t>>,
}

impl<'t> InvariantText<'t> {
    pub fn new() -> Self {
        InvariantText {
            fragments: VecDeque::new(),
        }
    }

    pub fn into_owned(self) -> InvariantText<'static> {
        let InvariantText { fragments } = self;
        InvariantText {
            fragments: fragments
                .into_iter()
                .map(InvariantFragment::into_owned)
                .collect(),
        }
    }

    pub fn to_string(&self) -> Cow<'t, str> {
        self.fragments
            .iter()
            .map(|fragment| fragment.as_string().clone())
            .reduce(|text, fragment| text + fragment)
            .unwrap_or(Cow::Borrowed(""))
    }

    pub fn repeat(self, n: usize) -> Self {
        if n == 0 {
            self
        } else {
            let InvariantText { fragments } = self;
            let n = (n - 1)
                .checked_mul(fragments.len())
                .expect("overflow determining invariant text");
            let first = fragments.clone();
            InvariantText {
                fragments: first
                    .into_iter()
                    .chain(fragments.into_iter().cycle().take(n))
                    .collect(),
            }
        }
    }
}

impl<'t> Add for InvariantText<'t> {
    type Output = Self;

    fn add(self, other: Self) -> Self::Output {
        let InvariantText {
            fragments: mut left,
        } = self;
        let InvariantText {
            fragments: mut right,
        } = other;
        let end = left.pop_back();
        let start = right.pop_front();
        let InvariantText { fragments: middle } = match (end, start) {
            (Some(left), Some(right)) => left + right,
            (Some(middle), None) | (None, Some(middle)) => middle.into(),
            (None, None) => InvariantText::new(),
        };
        InvariantText {
            fragments: left.into_iter().chain(middle).chain(right).collect(),
        }
    }
}

impl<'t> Add<InvariantFragment<'t>> for InvariantText<'t> {
    type Output = Self;

    fn add(self, fragment: InvariantFragment<'t>) -> Self::Output {
        self + Self::from(fragment)
    }
}

impl<'t> Default for InvariantText<'t> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'t> From<InvariantFragment<'t>> for InvariantText<'t> {
    fn from(fragment: InvariantFragment<'t>) -> Self {
        InvariantText {
            fragments: [fragment].into_iter().collect(),
        }
    }
}

impl<'t> Invariance for InvariantText<'t> {
    fn empty() -> Self {
        InvariantText::new()
    }
}

impl<'t> Mul<usize> for InvariantText<'t> {
    type Output = Self;

    fn mul(self, n: usize) -> Self::Output {
        self.repeat(n)
    }
}

#[derive(Clone, Debug, Eq)]
enum InvariantFragment<'t> {
    Nominal(Cow<'t, str>),
    Structural(Cow<'t, str>),
}

impl<'t> InvariantFragment<'t> {
    pub fn into_owned(self) -> InvariantFragment<'static> {
        use InvariantFragment::{Nominal, Structural};

        match self {
            Nominal(text) => Nominal(text.into_owned().into()),
            Structural(text) => Structural(text.into_owned().into()),
        }
    }

    pub fn as_string(&self) -> &Cow<'t, str> {
        match self {
            InvariantFragment::Nominal(ref text) | InvariantFragment::Structural(ref text) => text,
        }
    }
}

impl<'t> Add for InvariantFragment<'t> {
    type Output = InvariantText<'t>;

    fn add(self, other: Self) -> Self::Output {
        use InvariantFragment::{Nominal, Structural};

        match (self, other) {
            (Nominal(left), Nominal(right)) => InvariantText {
                fragments: [Nominal(left + right)].into_iter().collect(),
            },
            (Structural(left), Structural(right)) => InvariantText {
                fragments: [Structural(left + right)].into_iter().collect(),
            },
            (left, right) => InvariantText {
                fragments: [left, right].into_iter().collect(),
            },
        }
    }
}

impl<'t> PartialEq for InvariantFragment<'t> {
    fn eq(&self, other: &Self) -> bool {
        use InvariantFragment::{Nominal, Structural};

        match (self, other) {
            (Nominal(ref left), Nominal(ref right)) => {
                if PATHS_ARE_CASE_INSENSITIVE {
                    // This comparison uses Unicode simple case folding. It
                    // would be better to use full case folding (and better
                    // still to use case folding appropriate for the language of
                    // the text), but this approach is used to have consistent
                    // results with the regular expression encoding of compiled
                    // globs. A more comprehensive alternative would be to use
                    // something like the `focaccia` crate. See also
                    // `CharExt::has_casing`.
                    encode::case_folded_eq(left.as_ref(), right.as_ref())
                } else {
                    left == right
                }
            }
            (Structural(ref left), Structural(ref right)) => left == right,
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Boundedness {
    Closed,
    Open,
}

impl Boundedness {
    pub fn is_closed(&self) -> bool {
        matches!(self, Boundedness::Closed)
    }

    pub fn is_open(&self) -> bool {
        matches!(self, Boundedness::Open)
    }
}

#[derive(Clone, Debug, Eq)]
pub enum Variance<T> {
    Invariant(T),
    // In this context, _boundedness_ refers to whether or not a variant token or expression is
    // _constrained_ or _unconstrained_. For example, the expression `**` is unconstrained and
    // matches _any and all_, while the expression `a*z` is constrained and matches _some_. Note
    // that both expressions match an infinite number of components, but the constrained
    // expression does *not* match any component. Boundedness does **not** consider length,
    // only whether or not some part of an expression is constrained to a known set of
    // matches. As such, both the expressions `?` and `*` are variant with open bounds.
    Variant(Boundedness),
}

impl<T> Variance<T> {
    pub fn map_invariance<U>(self, mut f: impl FnMut(T) -> U) -> Variance<U> {
        match self {
            Variance::Invariant(invariant) => Variance::Invariant(f(invariant)),
            Variance::Variant(boundedness) => Variance::Variant(boundedness),
        }
    }

    pub fn as_invariance(&self) -> Option<&T> {
        match self {
            Variance::Invariant(ref invariant) => Some(invariant),
            _ => None,
        }
    }

    pub fn boundedness(&self) -> Boundedness {
        match self {
            Variance::Variant(ref boundedness) => *boundedness,
            _ => Boundedness::Closed,
        }
    }

    pub fn is_invariant(&self) -> bool {
        matches!(self, Variance::Invariant(_))
    }

    pub fn is_variant(&self) -> bool {
        matches!(self, Variance::Variant(_))
    }
}

impl<T> Add for Variance<T>
where
    T: Add<T, Output = T>,
{
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        use Boundedness::{Closed, Open};
        use Variance::{Invariant, Variant};

        match (self, rhs) {
            (Invariant(left), Invariant(right)) => Invariant(left + right),
            (Variant(Open), Variant(Open)) => Variant(Open),
            (Invariant(_) | Variant(_), Variant(_)) | (Variant(_), Invariant(_)) => Variant(Closed),
        }
    }
}

impl<T> PartialEq for Variance<T>
where
    T: PartialEq<T>,
{
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Variance::Invariant(ref left), Variance::Invariant(ref right)) => left == right,
            (Variance::Variant(ref left), Variance::Variant(ref right)) => left == right,
            _ => false,
        }
    }
}

// TODO: Is there some way to unify this with
// `invariant_text_prefix_upper_bound`?
pub fn invariant_text_prefix<'t, A, I>(tokens: I) -> String
where
    A: 't,
    I: IntoIterator<Item = &'t Token<'t, A>>,
{
    let separator = &Separator::invariant_text();
    let mut tokens = tokens.into_iter().peekable();
    let mut prefix = String::new();
    if tokens
        .peek()
        .map_or(false, |token| !token.has_sub_tokens() && token.has_root())
    {
        // Push a preceding separator if the first token has a root and is not a group.
        // This ensures that initiating separators and tree wildcards express a
        // root in invariant prefixes.
        prefix.push_str(separator);
    }
    // TODO: Replace `map`, `take_while`, and `flatten` with `map_while` when it
    // stabilizes.
    prefix.push_str(
        &token::components(tokens)
            .map(|component| {
                component
                    .variance::<InvariantText>()
                    .as_invariance()
                    .map(InvariantText::to_string)
                    .map(Cow::into_owned)
            })
            .take_while(Option::is_some)
            .flatten()
            .join(separator),
    );
    prefix
}

pub fn invariant_text_prefix_upper_bound<'t, A, I>(tokens: I) -> usize
where
    A: 't,
    I: IntoIterator<Item = &'t Token<'t, A>>,
{
    use crate::token::{
        TokenKind::{Separator, Wildcard},
        Wildcard::Tree,
    };

    let mut m = 0usize;
    let mut separator = None;
    for (n, token) in tokens.into_iter().map(Token::kind).enumerate() {
        m = n;
        match token {
            Separator(_) => {
                separator = Some(n);
            }
            Wildcard(Tree { .. }) => {
                return n;
            }
            _ => {
                if token.variance::<InvariantText>().is_invariant() {
                    continue;
                }
                return match separator {
                    Some(n) => n + 1,
                    None => 0,
                };
            }
        }
    }
    m + 1
}

/// Returns `true` if the token tree is exhaustive.
///
/// A glob expression and its token tree are exhaustive if the terminal
/// component has unbounded depth and unbounded variance.
pub fn is_exhaustive<'i, 't, A, I>(tokens: I) -> bool
where
    't: 'i,
    A: 't,
    I: IntoIterator<Item = &'i Token<'t, A>>,
{
    let component = token::components(tokens).last();
    matches!(
        component.map(|component| {
            (
                component.depth(),
                component.variance::<InvariantText>().boundedness(),
            )
        }),
        Some((Boundedness::Open, Boundedness::Open)),
    )
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use crate::token::{
        self,
        variance::{self, Boundedness, InvariantSize, Variance},
        TokenTree,
    };

    #[test]
    fn invariant_text_prefix() {
        fn invariant_path_prefix(expression: &str) -> PathBuf {
            variance::invariant_text_prefix(token::parse(expression).unwrap().tokens()).into()
        }

        assert_eq!(invariant_path_prefix("/a/b"), Path::new("/a/b"));
        assert_eq!(invariant_path_prefix("a/b"), Path::new("a/b"));
        assert_eq!(invariant_path_prefix("a/*"), Path::new("a"));
        assert_eq!(invariant_path_prefix("a/*b"), Path::new("a"));
        assert_eq!(invariant_path_prefix("a/b*"), Path::new("a"));
        assert_eq!(invariant_path_prefix("a/b/*/c"), Path::new("a/b"));

        #[cfg(any(unix, windows))]
        let prefix = invariant_path_prefix("../foo/(?i)bar/(?-i)baz");
        #[cfg(unix)]
        assert_eq!(prefix, Path::new("../foo"));
        #[cfg(windows)]
        assert_eq!(prefix, Path::new("../foo/bar"));

        assert_eq!(invariant_path_prefix("**"), Path::new(""));
        assert_eq!(invariant_path_prefix("a*"), Path::new(""));
        assert_eq!(invariant_path_prefix("*/b"), Path::new(""));
        assert_eq!(invariant_path_prefix("a?/b"), Path::new(""));
    }

    #[test]
    fn tree_expression_variance() {
        use Boundedness::{Closed, Open};
        use Variance::Variant;

        let tokenized = token::parse("**").unwrap();
        assert!(matches!(
            tokenized.variance::<InvariantSize>(),
            Variant(Open)
        ));
        let tokenized = token::parse("<*/>*").unwrap();
        assert!(matches!(
            tokenized.variance::<InvariantSize>(),
            Variant(Open)
        ));
        let tokenized = token::parse("<<?>/>*").unwrap();
        assert!(matches!(
            tokenized.variance::<InvariantSize>(),
            Variant(Open)
        ));

        let tokenized = token::parse("foo/**").unwrap();
        assert!(matches!(
            tokenized.variance::<InvariantSize>(),
            Variant(Closed)
        ));
        let tokenized = token::parse("<foo*/>*").unwrap();
        assert!(matches!(
            tokenized.variance::<InvariantSize>(),
            Variant(Closed)
        ));
    }

    #[test]
    fn exhaustiveness() {
        assert!(token::is_exhaustive(token::parse("**").unwrap().tokens()));
        assert!(token::is_exhaustive(token::parse("a/**").unwrap().tokens()));
        assert!(token::is_exhaustive(
            token::parse("a/<*/>*").unwrap().tokens()
        ));
        assert!(token::is_exhaustive(
            token::parse("a/<<?>/>*").unwrap().tokens()
        ));

        assert!(!token::is_exhaustive(
            token::parse("a/**/b").unwrap().tokens()
        ));
        assert!(!token::is_exhaustive(token::parse("a/*").unwrap().tokens()));
        assert!(!token::is_exhaustive(
            token::parse("a/<?>").unwrap().tokens()
        ));
        assert!(!token::is_exhaustive(
            token::parse("a</**/b>").unwrap().tokens()
        ));
        assert!(!token::is_exhaustive(
            token::parse("**/a").unwrap().tokens()
        ));
        assert!(!token::is_exhaustive(token::parse("").unwrap().tokens()));
    }
}
