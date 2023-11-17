//! Rules and limitations for token sequences.
//!
//! This module provides the `check` function, which examines a token sequence
//! and emits an error if the sequence violates rules. Rules are invariants that
//! are difficult or impossible to enforce when parsing text and primarily
//! detect and reject token sequences that produce anomalous, meaningless, or
//! unexpected globs (regular expressions) when compiled.
//!
//! Most rules concern alternatives, which have complex interactions with
//! neighboring tokens.

// TODO: The `check` function fails fast and either report no errors or exactly
//       one error. To better support diagnostics, `check` should probably
//       perform an exhaustive analysis and report zero or more errors.

#[cfg(feature = "miette")]
use std::fmt::Display;
use std::{borrow::Cow, convert::Infallible, iter::Fuse, path::PathBuf, slice};

use itertools::Itertools as _;
#[cfg(feature = "miette")]
use miette::{Diagnostic, LabeledSpan, SourceCode};
use thiserror::Error;

use crate::{
    diagnostics::{CompositeSpan, CorrelatedSpan, SpanExt as _},
    token::{self, InvariantSize, Token, TokenKind, TokenTree, Tokenized},
    Any, BuildError, Glob, Pattern,
};

/// Maximum invariant size.
///
/// This size is equal to or greater than the maximum size of a path on
/// supported platforms. The primary purpose of this limit is to mitigate
/// malicious or mistaken expressions that encode very large invariant text,
/// namely via repetitions.
///
/// This limit is independent of the back end encoding. This code does not rely
/// on errors in the encoder by design, such as size limitations.
const MAX_INVARIANT_SIZE: InvariantSize = InvariantSize::new(0x10000);

trait IteratorExt: Iterator + Sized {
    fn adjacent(self) -> Adjacent<Self>
    where
        Self::Item: Clone;
}

impl<I> IteratorExt for I
where
    I: Iterator,
{
    fn adjacent(self) -> Adjacent<Self>
    where
        Self::Item: Clone,
    {
        Adjacent::new(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Adjacency<T> {
    Only { item: T },
    First { item: T, right: T },
    Middle { left: T, item: T, right: T },
    Last { left: T, item: T },
}

impl<T> Adjacency<T> {
    pub fn into_tuple(self) -> (Option<T>, T, Option<T>) {
        match self {
            Adjacency::Only { item } => (None, item, None),
            Adjacency::First { item, right } => (None, item, Some(right)),
            Adjacency::Middle { left, item, right } => (Some(left), item, Some(right)),
            Adjacency::Last { left, item } => (Some(left), item, None),
        }
    }
}

struct Adjacent<I>
where
    I: Iterator,
{
    input: Fuse<I>,
    adjacency: Option<Adjacency<I::Item>>,
}

impl<I> Adjacent<I>
where
    I: Iterator,
{
    fn new(input: I) -> Self {
        let mut input = input.fuse();
        let adjacency = match (input.next(), input.next()) {
            (Some(item), Some(right)) => Some(Adjacency::First { item, right }),
            (Some(item), None) => Some(Adjacency::Only { item }),
            (None, None) => None,
            // The input iterator is fused, so this cannot occur.
            (None, Some(_)) => unreachable!(),
        };
        Adjacent { input, adjacency }
    }
}

impl<I> Iterator for Adjacent<I>
where
    I: Iterator,
    I::Item: Clone,
{
    type Item = Adjacency<I::Item>;

    fn next(&mut self) -> Option<Self::Item> {
        let next = self.input.next();
        self.adjacency.take().map(|adjacency| {
            self.adjacency = match adjacency.clone() {
                Adjacency::First {
                    item: left,
                    right: item,
                }
                | Adjacency::Middle {
                    item: left,
                    right: item,
                    ..
                } => {
                    if let Some(right) = next {
                        Some(Adjacency::Middle { left, item, right })
                    } else {
                        Some(Adjacency::Last { left, item })
                    }
                }
                Adjacency::Only { .. } | Adjacency::Last { .. } => None,
            };
            adjacency
        })
    }
}

trait SliceExt<T> {
    fn terminals(&self) -> Option<Terminals<&T>>;
}

impl<T> SliceExt<T> for [T] {
    fn terminals(&self) -> Option<Terminals<&T>> {
        match self.len() {
            0 => None,
            1 => Some(Terminals::Only(self.first().unwrap())),
            _ => Some(Terminals::StartEnd(
                self.first().unwrap(),
                self.last().unwrap(),
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Terminals<T> {
    Only(T),
    StartEnd(T, T),
}

impl<T> Terminals<T> {
    pub fn map<U, F>(self, mut f: F) -> Terminals<U>
    where
        F: FnMut(T) -> U,
    {
        match self {
            Terminals::Only(only) => Terminals::Only(f(only)),
            Terminals::StartEnd(start, end) => Terminals::StartEnd(f(start), f(end)),
        }
    }
}

/// Describes errors concerning rules and patterns in a glob expression.
///
/// Patterns must follow rules described in the [repository
/// documentation](https://github.com/olson-sean-k/wax/blob/master/README.md). These rules are
/// designed to avoid nonsense glob expressions and ambiguity. If a glob
/// expression parses but violates these rules or is otherwise malformed, then
/// this error is returned by some APIs.
#[derive(Debug, Error)]
#[error("malformed glob expression: {kind}")]
pub struct RuleError<'t> {
    expression: Cow<'t, str>,
    kind: RuleErrorKind,
    location: CompositeSpan,
}

impl<'t> RuleError<'t> {
    fn new(expression: Cow<'t, str>, kind: RuleErrorKind, location: CompositeSpan) -> Self {
        RuleError {
            expression,
            kind,
            location,
        }
    }

    /// Clones any borrowed data into an owning instance.
    pub fn into_owned(self) -> RuleError<'static> {
        let RuleError {
            expression,
            kind,
            location,
        } = self;
        RuleError {
            expression: expression.into_owned().into(),
            kind,
            location,
        }
    }

    pub fn locations(&self) -> &[CompositeSpan] {
        slice::from_ref(&self.location)
    }

    /// Gets the glob expression that violated pattern rules.
    pub fn expression(&self) -> &str {
        self.expression.as_ref()
    }
}

#[cfg(feature = "miette")]
#[cfg_attr(docsrs, doc(cfg(feature = "miette")))]
impl Diagnostic for RuleError<'_> {
    fn code<'a>(&'a self) -> Option<Box<dyn 'a + Display>> {
        Some(Box::new(String::from(match self.kind {
            RuleErrorKind::RootedSubGlob => "wax::glob::rooted_sub_glob",
            RuleErrorKind::SingularTree => "wax::glob::singular_tree",
            RuleErrorKind::SingularZeroOrMore => "wax::glob::singular_zero_or_more",
            RuleErrorKind::AdjacentBoundary => "wax::glob::adjacent_boundary",
            RuleErrorKind::AdjacentZeroOrMore => "wax::glob::adjacent_zero_or_more",
            RuleErrorKind::OversizedInvariant => "wax::glob::oversized_invariant",
            RuleErrorKind::IncompatibleBounds => "wax::glob::incompatible_bounds",
        })))
    }

    fn help<'a>(&'a self) -> Option<Box<dyn 'a + Display>> {
        match self.kind {
            RuleErrorKind::OversizedInvariant => Some(Box::new(String::from(
                "this error typically occurs when a repetition has a convergent bound that is too \
                 large",
            ))),
            _ => None,
        }
    }

    fn source_code(&self) -> Option<&dyn SourceCode> {
        Some(&self.expression)
    }

    fn labels(&self) -> Option<Box<dyn Iterator<Item = LabeledSpan>>> {
        Some(Box::new(self.location.labels().into_iter()))
    }
}

#[derive(Clone, Debug, Error)]
#[non_exhaustive]
enum RuleErrorKind {
    #[error("rooted sub-glob in group")]
    RootedSubGlob,
    #[error("singular tree wildcard `**` in group")]
    SingularTree,
    #[error("singular zero-or-more wildcard `*` or `$` in group")]
    SingularZeroOrMore,
    #[error("adjacent component boundaries `/` or `**`")]
    AdjacentBoundary,
    #[error("adjacent zero-or-more wildcards `*` or `$`")]
    AdjacentZeroOrMore,
    #[error("oversized invariant expression")]
    OversizedInvariant,
    #[error("incompatible repetition bounds")]
    IncompatibleBounds,
}

#[derive(Clone, Copy, Debug)]
pub struct Checked<T> {
    inner: T,
}

impl<T> Checked<T> {
    pub fn release(self) -> T {
        self.inner
    }
}

impl<'t> Checked<Token<'t, ()>> {
    pub fn any<T, I>(trees: I) -> Self
    where
        T: TokenTree<'t>,
        I: IntoIterator<Item = Checked<T>>,
    {
        Checked {
            // `token::any` constructs an alternative from the input token trees. The alternative
            // is not checked, but the `any` combinator is explicitly allowed to ignore the subset
            // of rules that may be violated by this construction. In particular, branches may or
            // may not have roots such that the alternative can match overlapping directory trees.
            inner: token::any(
                trees
                    .into_iter()
                    .map(Checked::release)
                    .map(TokenTree::into_tokens),
            ),
        }
    }
}

impl<'t, A> Checked<Token<'t, A>> {
    pub fn into_owned(self) -> Checked<Token<'static, A>> {
        Checked {
            inner: self.release().into_owned(),
        }
    }
}

impl<'t, A> Checked<Tokenized<'t, A>> {
    pub fn into_owned(self) -> Checked<Tokenized<'static, A>> {
        Checked {
            inner: self.release().into_owned(),
        }
    }
}

impl<'t> Checked<Tokenized<'t>> {
    pub fn partition(self) -> (PathBuf, Self) {
        let tokenized = self.release();
        // `Tokenized::partition` does not violate rules.
        let (path, tokenized) = tokenized.partition();
        (path, Checked { inner: tokenized })
    }
}

impl<T> AsRef<T> for Checked<T> {
    fn as_ref(&self) -> &T {
        &self.inner
    }
}

impl<'t, T> Pattern<'t> for Checked<T>
where
    T: TokenTree<'t>,
{
    type Tokens = T;
    type Error = Infallible;
}

impl<'t> From<Any<'t>> for Checked<Token<'t, ()>> {
    fn from(any: Any<'t>) -> Self {
        let Any { tree, .. } = any;
        tree
    }
}

impl<'t> From<Glob<'t>> for Checked<Tokenized<'t>> {
    fn from(glob: Glob<'t>) -> Self {
        let Glob { tree, .. } = glob;
        tree
    }
}

impl<'t> TryFrom<&'t str> for Checked<Tokenized<'t>> {
    type Error = BuildError;

    fn try_from(expression: &'t str) -> Result<Self, Self::Error> {
        crate::parse_and_check(expression)
    }
}

pub fn check(tokenized: Tokenized) -> Result<Checked<Tokenized>, RuleError> {
    boundary(&tokenized)?;
    bounds(&tokenized)?;
    group(&tokenized)?;
    size(&tokenized)?;
    Ok(Checked { inner: tokenized })
}

fn boundary<'t>(tokenized: &Tokenized<'t>) -> Result<(), RuleError<'t>> {
    if let Some((left, right)) = tokenized
        .walk()
        .group_by(|(position, _)| *position)
        .into_iter()
        .flat_map(|(_, group)| {
            group
                .map(|(_, token)| token)
                .tuple_windows::<(_, _)>()
                .filter(|(left, right)| {
                    left.is_component_boundary() && right.is_component_boundary()
                })
                .map(|(left, right)| (*left.annotation(), *right.annotation()))
        })
        .next()
    {
        Err(RuleError::new(
            tokenized.expression().clone(),
            RuleErrorKind::AdjacentBoundary,
            CompositeSpan::spanned("here", left.union(&right)),
        ))
    } else {
        Ok(())
    }
}

fn group<'t>(tokenized: &Tokenized<'t>) -> Result<(), RuleError<'t>> {
    use Terminals::{Only, StartEnd};

    use crate::token::{
        TokenKind::{Separator, Wildcard},
        Wildcard::{Tree, ZeroOrMore},
    };

    struct CorrelatedError {
        kind: RuleErrorKind,
        location: CorrelatedSpan,
    }

    impl CorrelatedError {
        fn new(kind: RuleErrorKind, outer: Option<&Token>, inner: &Token) -> Self {
            CorrelatedError {
                kind,
                location: CorrelatedSpan::split_some(
                    outer.map(Token::annotation).copied().map(From::from),
                    *inner.annotation(),
                ),
            }
        }
    }

    #[derive(Clone, Copy, Default)]
    struct Outer<'i, 't> {
        left: Option<&'i Token<'t>>,
        right: Option<&'i Token<'t>>,
    }

    impl<'i, 't> Outer<'i, 't> {
        pub fn push(self, left: Option<&'i Token<'t>>, right: Option<&'i Token<'t>>) -> Self {
            Outer {
                left: left.or(self.left),
                right: right.or(self.right),
            }
        }
    }

    fn has_starting_component_boundary<'t>(token: Option<&'t Token<'t>>) -> bool {
        token.map_or(false, |token| {
            token
                .walk()
                .starting()
                .any(|(_, token)| token.is_component_boundary())
        })
    }

    fn has_ending_component_boundary<'t>(token: Option<&'t Token<'t>>) -> bool {
        token.map_or(false, |token| {
            token
                .walk()
                .ending()
                .any(|(_, token)| token.is_component_boundary())
        })
    }

    fn has_starting_zom_token<'t>(token: Option<&'t Token<'t>>) -> bool {
        token.map_or(false, |token| {
            token
                .walk()
                .starting()
                .any(|(_, token)| matches!(token.kind(), Wildcard(ZeroOrMore(_))))
        })
    }

    fn has_ending_zom_token<'t>(token: Option<&'t Token<'t>>) -> bool {
        token.map_or(false, |token| {
            token
                .walk()
                .ending()
                .any(|(_, token)| matches!(token.kind(), Wildcard(ZeroOrMore(_))))
        })
    }

    fn diagnose<'i, 't>(
        // This is a somewhat unusual API, but it allows the lifetime `'t` of the `Cow` to be
        // properly forwarded to output values (`RuleError`).
        #[allow(clippy::ptr_arg)] expression: &'i Cow<'t, str>,
        token: &'i Token<'t>,
        label: &'static str,
    ) -> impl 'i + Copy + Fn(CorrelatedError) -> RuleError<'t>
    where
        't: 'i,
    {
        move |CorrelatedError { kind, location }| {
            RuleError::new(
                expression.clone(),
                kind,
                CompositeSpan::correlated(label, *token.annotation(), location),
            )
        }
    }

    fn recurse<'i, 't, I>(
        // This is a somewhat unusual API, but it allows the lifetime `'t` of the `Cow` to be
        // properly forwarded to output values (`RuleError`).
        #[allow(clippy::ptr_arg)] expression: &Cow<'t, str>,
        tokens: I,
        outer: Outer<'i, 't>,
    ) -> Result<(), RuleError<'t>>
    where
        I: IntoIterator<Item = &'i Token<'t>>,
        't: 'i,
    {
        for (left, token, right) in tokens.into_iter().adjacent().map(Adjacency::into_tuple) {
            match token.kind() {
                TokenKind::Alternative(ref alternative) => {
                    let outer = outer.push(left, right);
                    let diagnose = diagnose(expression, token, "in this alternative");
                    for tokens in alternative.branches() {
                        if let Some(terminals) = tokens.terminals() {
                            check_group(terminals, outer).map_err(diagnose)?;
                            check_group_alternative(terminals, outer).map_err(diagnose)?;
                        }
                        recurse(expression, tokens.iter(), outer)?;
                    }
                }
                TokenKind::Repetition(ref repetition) => {
                    let outer = outer.push(left, right);
                    let diagnose = diagnose(expression, token, "in this repetition");
                    let tokens = repetition.tokens();
                    if let Some(terminals) = tokens.terminals() {
                        check_group(terminals, outer).map_err(diagnose)?;
                        check_group_repetition(terminals, outer, repetition.bounds())
                            .map_err(diagnose)?;
                    }
                    recurse(expression, tokens.iter(), outer)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn check_group<'t>(
        terminals: Terminals<&Token>,
        outer: Outer<'t, 't>,
    ) -> Result<(), CorrelatedError> {
        let Outer { left, right } = outer;
        match terminals.map(|token| (token, token.kind())) {
            // The group is preceded by component boundaries; disallow leading separators.
            //
            // For example, `foo/{bar,/}`.
            Only((inner, Separator(_))) | StartEnd((inner, Separator(_)), _)
                if has_ending_component_boundary(left) =>
            {
                Err(CorrelatedError::new(
                    RuleErrorKind::AdjacentBoundary,
                    left,
                    inner,
                ))
            }
            // The group is followed by component boundaries; disallow trailing
            // separators.
            //
            // For example, `{foo,/}/bar`.
            Only((inner, Separator(_))) | StartEnd(_, (inner, Separator(_)))
                if has_starting_component_boundary(right) =>
            {
                Err(CorrelatedError::new(
                    RuleErrorKind::AdjacentBoundary,
                    right,
                    inner,
                ))
            }
            // Disallow singular tree tokens.
            //
            // For example, `{foo,bar,**}`.
            Only((inner, Wildcard(Tree { .. }))) => Err(CorrelatedError::new(
                RuleErrorKind::SingularTree,
                None,
                inner,
            )),
            // The group is preceded by component boundaries; disallow leading tree tokens.
            //
            // For example, `foo/{bar,**/baz}`.
            StartEnd((inner, Wildcard(Tree { .. })), _) if has_ending_component_boundary(left) => {
                Err(CorrelatedError::new(
                    RuleErrorKind::AdjacentBoundary,
                    left,
                    inner,
                ))
            }
            // The group is followed by component boundaries; disallow trailing
            // tree tokens.
            //
            // For example, `{foo,bar/**}/baz`.
            StartEnd(_, (inner, Wildcard(Tree { .. })))
                if has_starting_component_boundary(right) =>
            {
                Err(CorrelatedError::new(
                    RuleErrorKind::AdjacentBoundary,
                    right,
                    inner,
                ))
            }
            // The group is prefixed by a zero-or-more token; disallow leading
            // zero-or-more tokens.
            //
            // For example, `foo*{bar,*,baz}`.
            Only((inner, Wildcard(ZeroOrMore(_))))
            | StartEnd((inner, Wildcard(ZeroOrMore(_))), _)
                if has_ending_zom_token(left) =>
            {
                Err(CorrelatedError::new(
                    RuleErrorKind::AdjacentZeroOrMore,
                    left,
                    inner,
                ))
            }
            // The group is followed by a zero-or-more token; disallow trailing
            // zero-or-more tokens.
            //
            // For example, `{foo,*,bar}*baz`.
            Only((inner, Wildcard(ZeroOrMore(_))))
            | StartEnd(_, (inner, Wildcard(ZeroOrMore(_))))
                if has_starting_zom_token(right) =>
            {
                Err(CorrelatedError::new(
                    RuleErrorKind::AdjacentZeroOrMore,
                    right,
                    inner,
                ))
            }
            _ => Ok(()),
        }
    }

    fn check_group_alternative<'t>(
        terminals: Terminals<&Token>,
        outer: Outer<'t, 't>,
    ) -> Result<(), CorrelatedError> {
        let Outer { left, .. } = outer;
        match terminals.map(|token| (token, token.kind())) {
            // The alternative is preceded by a termination; disallow rooted sub-globs.
            //
            // For example, `{foo,/}` or `{foo,/bar}`.
            Only((inner, Separator(_))) | StartEnd((inner, Separator(_)), _) if left.is_none() => {
                Err(CorrelatedError::new(
                    RuleErrorKind::RootedSubGlob,
                    left,
                    inner,
                ))
            }
            // The alternative is preceded by a termination; disallow rooted
            // sub-globs.
            //
            // For example, `{/**/foo,bar}`.
            Only((inner, Wildcard(Tree { has_root: true })))
            | StartEnd((inner, Wildcard(Tree { has_root: true })), _)
                if left.is_none() =>
            {
                Err(CorrelatedError::new(
                    RuleErrorKind::RootedSubGlob,
                    left,
                    inner,
                ))
            }
            _ => Ok(()),
        }
    }

    fn check_group_repetition<'t>(
        terminals: Terminals<&Token>,
        outer: Outer<'t, 't>,
        bounds: (usize, Option<usize>),
    ) -> Result<(), CorrelatedError> {
        let Outer { left, .. } = outer;
        let (lower, _) = bounds;
        match terminals.map(|token| (token, token.kind())) {
            // The repetition is preceded by a termination; disallow rooted sub-globs with a zero
            // lower bound.
            //
            // For example, `</foo:0,>`.
            Only((inner, Separator(_))) | StartEnd((inner, Separator(_)), _)
                if left.is_none() && lower == 0 =>
            {
                Err(CorrelatedError::new(
                    RuleErrorKind::RootedSubGlob,
                    left,
                    inner,
                ))
            }
            // The repetition is preceded by a termination; disallow rooted
            // sub-globs with a zero lower bound.
            //
            // For example, `</**/foo>`.
            Only((inner, Wildcard(Tree { has_root: true })))
            | StartEnd((inner, Wildcard(Tree { has_root: true })), _)
                if left.is_none() && lower == 0 =>
            {
                Err(CorrelatedError::new(
                    RuleErrorKind::RootedSubGlob,
                    left,
                    inner,
                ))
            }
            // The repetition begins and ends with a separator.
            //
            // For example, `</foo/bar/:1,>`.
            StartEnd((left, _), (right, _))
                if left.is_component_boundary() && right.is_component_boundary() =>
            {
                Err(CorrelatedError::new(
                    RuleErrorKind::AdjacentBoundary,
                    Some(left),
                    right,
                ))
            }
            // The repetition is a singular separator.
            //
            // For example, `</:1,>`.
            Only((token, Separator(_))) => Err(CorrelatedError::new(
                RuleErrorKind::AdjacentBoundary,
                None,
                token,
            )),
            // The repetition is a singular zero-or-more wildcard.
            //
            // For example, `<*:1,>`.
            Only((token, Wildcard(ZeroOrMore(_)))) => Err(CorrelatedError::new(
                RuleErrorKind::SingularZeroOrMore,
                None,
                token,
            )),
            _ => Ok(()),
        }
    }

    recurse(tokenized.expression(), tokenized.tokens(), Outer::default())
}

fn bounds<'t>(tokenized: &Tokenized<'t>) -> Result<(), RuleError<'t>> {
    if let Some((_, token)) = tokenized.walk().find(|(_, token)| match token.kind() {
        TokenKind::Repetition(ref repetition) => {
            let (lower, upper) = repetition.bounds();
            upper.map_or(false, |upper| upper < lower || upper == 0)
        }
        _ => false,
    }) {
        Err(RuleError::new(
            tokenized.expression().clone(),
            RuleErrorKind::IncompatibleBounds,
            CompositeSpan::spanned("here", *token.annotation()),
        ))
    } else {
        Ok(())
    }
}

fn size<'t>(tokenized: &Tokenized<'t>) -> Result<(), RuleError<'t>> {
    if let Some((_, token)) = tokenized
        .walk()
        // TODO: This is expensive. For each token tree encountered, the tree is traversed to
        //       determine its variance. If variant, the tree is traversed and queried again,
        //       revisiting the same tokens to recompute their local variance.
        .find(|(_, token)| {
            token
                .variance::<InvariantSize>()
                .as_invariance()
                .map_or(false, |size| *size >= MAX_INVARIANT_SIZE)
        })
    {
        Err(RuleError::new(
            tokenized.expression().clone(),
            RuleErrorKind::OversizedInvariant,
            CompositeSpan::spanned("here", *token.annotation()),
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::rule::{Adjacency, IteratorExt as _};

    #[test]
    fn adjacent() {
        let mut adjacent = Option::<i32>::None.into_iter().adjacent();
        assert_eq!(adjacent.next(), None);

        let mut adjacent = Some(0i32).into_iter().adjacent();
        assert_eq!(adjacent.next(), Some(Adjacency::Only { item: 0 }));
        assert_eq!(adjacent.next(), None);

        let mut adjacent = (0i32..3).adjacent();
        assert_eq!(
            adjacent.next(),
            Some(Adjacency::First { item: 0, right: 1 })
        );
        assert_eq!(
            adjacent.next(),
            Some(Adjacency::Middle {
                left: 0,
                item: 1,
                right: 2
            })
        );
        assert_eq!(adjacent.next(), Some(Adjacency::Last { left: 1, item: 2 }));
        assert_eq!(adjacent.next(), None);
    }
}
