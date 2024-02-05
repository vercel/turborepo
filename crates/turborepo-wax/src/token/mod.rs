mod parse;
mod variance;

use std::{
    borrow::Cow,
    cmp,
    collections::VecDeque,
    mem,
    ops::Deref,
    path::{PathBuf, MAIN_SEPARATOR},
    slice, str,
};

use itertools::Itertools as _;

pub use crate::token::{
    parse::{parse, Annotation, ParseError, ROOT_SEPARATOR_EXPRESSION},
    variance::{
        invariant_text_prefix, is_exhaustive, Boundedness, InvariantSize, InvariantText, Variance,
    },
};
use crate::{
    token::variance::{
        CompositeBreadth, CompositeDepth, ConjunctiveVariance, DisjunctiveVariance,
        IntoInvariantText, Invariance, UnitBreadth, UnitDepth, UnitVariance,
    },
    StrExt as _, PATHS_ARE_CASE_INSENSITIVE,
};

pub trait TokenTree<'t>: Sized {
    type Annotation;

    fn into_tokens(self) -> Vec<Token<'t, Self::Annotation>>;

    fn tokens(&self) -> &[Token<'t, Self::Annotation>];
}

#[derive(Clone, Debug)]
pub struct Tokenized<'t, A = Annotation> {
    expression: Cow<'t, str>,
    tokens: Vec<Token<'t, A>>,
}

impl<'t, A> Tokenized<'t, A> {
    pub fn into_owned(self) -> Tokenized<'static, A> {
        let Tokenized { expression, tokens } = self;
        Tokenized {
            expression: expression.into_owned().into(),
            tokens: tokens.into_iter().map(Token::into_owned).collect(),
        }
    }

    pub fn expression(&self) -> &Cow<'t, str> {
        &self.expression
    }

    pub fn variance<T>(&self) -> Variance<T>
    where
        T: Invariance,
        for<'i> &'i Token<'t, A>: UnitVariance<T>,
    {
        self.tokens().iter().conjunctive_variance()
    }

    pub fn walk(&self) -> Walk<'_, 't, A> {
        Walk::from(&self.tokens)
    }
}

impl<'t> Tokenized<'t, Annotation> {
    pub fn partition(self) -> (PathBuf, Self) {
        fn pop_expression_bytes(expression: &str, n: usize) -> &str {
            let n = cmp::min(expression.len(), n);
            str::from_utf8(&expression.as_bytes()[n..])
                .expect("span offset split UTF-8 byte sequence")
        }

        let Tokenized {
            expression,
            mut tokens,
        } = self;

        // Get the invariant prefix and its upper bound for the token sequence.
        let prefix = variance::invariant_text_prefix(tokens.iter()).into();
        let n = variance::invariant_text_prefix_upper_bound(&tokens);
        let mut offset: usize = tokens
            .iter()
            .take(n)
            .map(|token| token.annotation().1)
            .sum();

        // Drain invariant tokens from the beginning of the token sequence and unroot
        // any tokens at the beginning of the variant sequence (tree wildcards).
        // Finally, translate spans and discard the corresponding invariant
        // bytes in the expression.
        tokens.drain(0..n);
        if tokens.first_mut().map_or(false, Token::unroot) {
            // TODO: The relationship between roots, the unrooting operation, and the span
            // in an       expression that represents such a root (if any) is
            // not captured by these APIs       very well. Perhaps `unroot`
            // should do more here? Pop additional bytes for the root separator
            // expression if the initial token has lost a root.
            offset += ROOT_SEPARATOR_EXPRESSION.len();
        }
        for token in tokens.iter_mut() {
            let start = token.annotation().0.saturating_sub(offset);
            token.annotation.0 = start;
        }
        let expression = match expression {
            Cow::Borrowed(expression) => pop_expression_bytes(expression, offset).into(),
            Cow::Owned(expression) => {
                String::from(pop_expression_bytes(&expression, offset)).into()
            }
        };

        (prefix, Tokenized { expression, tokens })
    }
}

impl<'t, A> TokenTree<'t> for Tokenized<'t, A> {
    type Annotation = A;

    fn into_tokens(self) -> Vec<Token<'t, Self::Annotation>> {
        let Tokenized { tokens, .. } = self;
        tokens
    }

    fn tokens(&self) -> &[Token<'t, Self::Annotation>] {
        &self.tokens
    }
}

#[derive(Clone, Debug)]
pub struct Token<'t, A = Annotation> {
    kind: TokenKind<'t, A>,
    annotation: A,
}

impl<'t, A> Token<'t, A> {
    fn new(kind: TokenKind<'t, A>, annotation: A) -> Self {
        Token { kind, annotation }
    }

    pub fn into_owned(self) -> Token<'static, A> {
        let Token { kind, annotation } = self;
        Token {
            kind: kind.into_owned(),
            annotation,
        }
    }

    pub fn unannotate(self) -> Token<'t, ()> {
        let Token { kind, .. } = self;
        Token {
            kind: kind.unannotate(),
            annotation: (),
        }
    }

    pub fn unroot(&mut self) -> bool {
        self.kind.unroot()
    }

    pub fn kind(&self) -> &TokenKind<'t, A> {
        self.as_ref()
    }

    pub fn annotation(&self) -> &A {
        self.as_ref()
    }

    pub fn walk(&self) -> Walk<'_, 't, A> {
        Walk::from(self)
    }

    pub fn has_root(&self) -> bool {
        self.walk().starting().any(|(_, token)| {
            matches!(
                token.kind(),
                TokenKind::Separator(_) | TokenKind::Wildcard(Wildcard::Tree { has_root: true }),
            )
        })
    }

    pub fn has_component_boundary(&self) -> bool {
        self.walk().any(|(_, token)| token.is_component_boundary())
    }
}

impl<'t, A> AsRef<TokenKind<'t, A>> for Token<'t, A> {
    fn as_ref(&self) -> &TokenKind<'t, A> {
        &self.kind
    }
}

impl<'t, A> AsRef<A> for Token<'t, A> {
    fn as_ref(&self) -> &A {
        &self.annotation
    }
}

impl<'t, A> Deref for Token<'t, A> {
    type Target = TokenKind<'t, A>;

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl<'t> From<TokenKind<'t, ()>> for Token<'t, ()> {
    fn from(kind: TokenKind<'t, ()>) -> Self {
        Token {
            kind,
            annotation: (),
        }
    }
}

impl<'t, A> TokenTree<'t> for Token<'t, A> {
    type Annotation = A;

    fn into_tokens(self) -> Vec<Token<'t, Self::Annotation>> {
        vec![self]
    }

    fn tokens(&self) -> &[Token<'t, Self::Annotation>] {
        slice::from_ref(self)
    }
}

impl<'i, 't, A> UnitBreadth for &'i Token<'t, A> {
    fn unit_breadth(self) -> Boundedness {
        self.kind.unit_breadth()
    }
}

impl<'i, 't, A> UnitDepth for &'i Token<'t, A> {
    fn unit_depth(self) -> Boundedness {
        self.kind.unit_depth()
    }
}

impl<'i, 't, A, T> UnitVariance<T> for &'i Token<'t, A>
where
    &'i TokenKind<'t, A>: UnitVariance<T>,
    T: Invariance,
{
    fn unit_variance(self) -> Variance<T> {
        self.kind.unit_variance()
    }
}

#[derive(Clone, Debug)]
pub enum TokenKind<'t, A = ()> {
    Alternative(Alternative<'t, A>),
    Class(Class),
    Literal(Literal<'t>),
    Repetition(Repetition<'t, A>),
    Separator(Separator),
    Wildcard(Wildcard),
}

impl<'t, A> TokenKind<'t, A> {
    pub fn into_owned(self) -> TokenKind<'static, A> {
        match self {
            TokenKind::Alternative(alternative) => alternative.into_owned().into(),
            TokenKind::Class(class) => TokenKind::Class(class),
            TokenKind::Literal(Literal {
                text,
                is_case_insensitive,
            }) => TokenKind::Literal(Literal {
                text: text.into_owned().into(),
                is_case_insensitive,
            }),
            TokenKind::Repetition(repetition) => repetition.into_owned().into(),
            TokenKind::Separator(_) => TokenKind::Separator(Separator),
            TokenKind::Wildcard(wildcard) => TokenKind::Wildcard(wildcard),
        }
    }

    pub fn unannotate(self) -> TokenKind<'t, ()> {
        match self {
            TokenKind::Alternative(alternative) => TokenKind::Alternative(alternative.unannotate()),
            TokenKind::Class(class) => TokenKind::Class(class),
            TokenKind::Literal(literal) => TokenKind::Literal(literal),
            TokenKind::Repetition(repetition) => TokenKind::Repetition(repetition.unannotate()),
            TokenKind::Separator(_) => TokenKind::Separator(Separator),
            TokenKind::Wildcard(wildcard) => TokenKind::Wildcard(wildcard),
        }
    }

    pub fn unroot(&mut self) -> bool {
        match self {
            TokenKind::Wildcard(Wildcard::Tree { ref mut has_root }) => {
                mem::replace(has_root, false)
            }
            _ => false,
        }
    }

    pub fn variance<T>(&self) -> Variance<T>
    where
        T: Invariance,
        for<'i> &'i TokenKind<'t, A>: UnitVariance<T>,
    {
        self.unit_variance()
    }

    pub fn has_sub_tokens(&self) -> bool {
        // It is not necessary to detect empty branches or sub-expressions.
        matches!(self, TokenKind::Alternative(_) | TokenKind::Repetition(_))
    }

    pub fn is_component_boundary(&self) -> bool {
        matches!(
            self,
            TokenKind::Separator(_) | TokenKind::Wildcard(Wildcard::Tree { .. })
        )
    }

    pub fn is_capturing(&self) -> bool {
        use TokenKind::{Alternative, Class, Repetition, Wildcard};

        matches!(
            self,
            Alternative(_) | Class(_) | Repetition(_) | Wildcard(_),
        )
    }
}

impl<'t, A> From<Alternative<'t, A>> for TokenKind<'t, A> {
    fn from(alternative: Alternative<'t, A>) -> Self {
        TokenKind::Alternative(alternative)
    }
}

impl<A> From<Class> for TokenKind<'_, A> {
    fn from(class: Class) -> Self {
        TokenKind::Class(class)
    }
}

impl<'t, A> From<Repetition<'t, A>> for TokenKind<'t, A> {
    fn from(repetition: Repetition<'t, A>) -> Self {
        TokenKind::Repetition(repetition)
    }
}

impl<A> From<Wildcard> for TokenKind<'static, A> {
    fn from(wildcard: Wildcard) -> Self {
        TokenKind::Wildcard(wildcard)
    }
}

impl<'i, 't, A> UnitBreadth for &'i TokenKind<'t, A> {
    fn unit_breadth(self) -> Boundedness {
        match self {
            TokenKind::Alternative(ref alternative) => alternative.unit_breadth(),
            TokenKind::Class(ref class) => class.unit_breadth(),
            TokenKind::Literal(ref literal) => literal.unit_breadth(),
            TokenKind::Repetition(ref repetition) => repetition.unit_breadth(),
            TokenKind::Separator(ref separator) => separator.unit_breadth(),
            TokenKind::Wildcard(ref wildcard) => wildcard.unit_breadth(),
        }
    }
}

impl<'i, 't, A> UnitDepth for &'i TokenKind<'t, A> {
    fn unit_depth(self) -> Boundedness {
        match self {
            TokenKind::Alternative(ref alternative) => alternative.unit_depth(),
            TokenKind::Class(ref class) => class.unit_depth(),
            TokenKind::Literal(ref literal) => literal.unit_depth(),
            TokenKind::Repetition(ref repetition) => repetition.unit_depth(),
            TokenKind::Separator(ref separator) => separator.unit_depth(),
            TokenKind::Wildcard(ref wildcard) => wildcard.unit_depth(),
        }
    }
}

impl<'i, 't, A, T> UnitVariance<T> for &'i TokenKind<'t, A>
where
    &'i Class: UnitVariance<T>,
    &'i Literal<'t>: UnitVariance<T>,
    &'i Separator: UnitVariance<T>,
    T: Invariance,
{
    fn unit_variance(self) -> Variance<T> {
        match self {
            TokenKind::Alternative(ref alternative) => alternative.unit_variance(),
            TokenKind::Class(ref class) => class.unit_variance(),
            TokenKind::Literal(ref literal) => literal.unit_variance(),
            TokenKind::Repetition(ref repetition) => repetition.unit_variance(),
            TokenKind::Separator(ref separator) => separator.unit_variance(),
            TokenKind::Wildcard(_) => Variance::Variant(Boundedness::Open),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Alternative<'t, A = ()>(Vec<Vec<Token<'t, A>>>);

impl<'t, A> Alternative<'t, A> {
    pub fn into_owned(self) -> Alternative<'static, A> {
        Alternative(
            self.0
                .into_iter()
                .map(|tokens| tokens.into_iter().map(Token::into_owned).collect())
                .collect(),
        )
    }

    pub fn unannotate(self) -> Alternative<'t, ()> {
        let Alternative(branches) = self;
        Alternative(
            branches
                .into_iter()
                .map(|branch| branch.into_iter().map(Token::unannotate).collect())
                .collect(),
        )
    }

    pub fn branches(&self) -> &Vec<Vec<Token<'t, A>>> {
        &self.0
    }
}

impl<'t, A> From<Vec<Vec<Token<'t, A>>>> for Alternative<'t, A> {
    fn from(alternatives: Vec<Vec<Token<'t, A>>>) -> Self {
        Alternative(alternatives)
    }
}

impl<'i, 't, A> UnitBreadth for &'i Alternative<'t, A> {
    fn unit_breadth(self) -> Boundedness {
        self.branches()
            .iter()
            .map(|tokens| tokens.iter().composite_breadth())
            .composite_breadth()
    }
}

impl<'i, 't, A> UnitDepth for &'i Alternative<'t, A> {
    fn unit_depth(self) -> Boundedness {
        self.branches()
            .iter()
            .map(|tokens| tokens.iter().composite_depth())
            .composite_depth()
    }
}

impl<'i, 't, A, T> UnitVariance<T> for &'i Alternative<'t, A>
where
    T: Invariance,
    &'i Token<'t, A>: UnitVariance<T>,
{
    fn unit_variance(self) -> Variance<T> {
        self.branches()
            .iter()
            .map(|tokens| tokens.iter().conjunctive_variance())
            .disjunctive_variance()
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Archetype {
    Character(char),
    Range(char, char),
}

impl Archetype {
    fn domain_variance(&self) -> Variance<char> {
        match self {
            Archetype::Character(x) => {
                if PATHS_ARE_CASE_INSENSITIVE {
                    Variance::Variant(Boundedness::Closed)
                } else {
                    Variance::Invariant(*x)
                }
            }
            Archetype::Range(a, b) => {
                if (a != b) || PATHS_ARE_CASE_INSENSITIVE {
                    Variance::Variant(Boundedness::Closed)
                } else {
                    Variance::Invariant(*a)
                }
            }
        }
    }
}

impl From<char> for Archetype {
    fn from(literal: char) -> Self {
        Archetype::Character(literal)
    }
}

impl From<(char, char)> for Archetype {
    fn from(range: (char, char)) -> Self {
        Archetype::Range(range.0, range.1)
    }
}

impl<'i, 't> UnitVariance<InvariantText<'t>> for &'i Archetype {
    fn unit_variance(self) -> Variance<InvariantText<'t>> {
        self.domain_variance()
            .map_invariance(|invariance| invariance.to_string().into_nominal_text())
    }
}

impl<'i> UnitVariance<InvariantSize> for &'i Archetype {
    fn unit_variance(self) -> Variance<InvariantSize> {
        // This is pessimistic and assumes that the code point will require four bytes
        // when encoded as UTF-8. This is technically possible, but most
        // commonly only one or two bytes will be required.
        self.domain_variance().map_invariance(|_| 4.into())
    }
}

#[derive(Clone, Debug)]
pub struct Class {
    is_negated: bool,
    archetypes: Vec<Archetype>,
}

impl Class {
    pub fn archetypes(&self) -> &[Archetype] {
        &self.archetypes
    }

    pub fn is_negated(&self) -> bool {
        self.is_negated
    }
}

impl<'i> UnitBreadth for &'i Class {}

impl<'i> UnitDepth for &'i Class {}

impl<'i, T> UnitVariance<T> for &'i Class
where
    &'i Archetype: UnitVariance<T>,
    T: Invariance,
{
    fn unit_variance(self) -> Variance<T> {
        if self.is_negated {
            // It is not feasible to encode a character class that matches all
            // UTF-8 text and therefore nothing when negated, and so a character
            // class must be variant if it is negated.
            Variance::Variant(Boundedness::Closed)
        } else {
            // TODO: This ignores casing groups, such as in the pattern `[aA]`.
            self.archetypes().iter().disjunctive_variance()
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Evaluation {
    Eager,
    Lazy,
}

#[derive(Clone, Debug)]
pub struct Literal<'t> {
    text: Cow<'t, str>,
    is_case_insensitive: bool,
}

impl<'t> Literal<'t> {
    pub fn text(&self) -> &str {
        self.text.as_ref()
    }

    fn domain_variance(&self) -> Variance<&Cow<'t, str>> {
        if self.has_variant_casing() {
            Variance::Variant(Boundedness::Closed)
        } else {
            Variance::Invariant(&self.text)
        }
    }

    pub fn is_case_insensitive(&self) -> bool {
        self.is_case_insensitive
    }

    pub fn has_variant_casing(&self) -> bool {
        // If path case sensitivity agrees with the literal case sensitivity, then the
        // literal is not variant. Otherwise, the literal is variant if it
        // contains characters with casing.
        (PATHS_ARE_CASE_INSENSITIVE != self.is_case_insensitive) && self.text.has_casing()
    }
}

impl<'i, 't> UnitBreadth for &'i Literal<'t> {}

impl<'i, 't> UnitDepth for &'i Literal<'t> {}

impl<'i, 't> UnitVariance<InvariantText<'t>> for &'i Literal<'t> {
    fn unit_variance(self) -> Variance<InvariantText<'t>> {
        self.domain_variance()
            .map_invariance(|invariance| invariance.clone().into_nominal_text())
    }
}

impl<'i, 't> UnitVariance<InvariantSize> for &'i Literal<'t> {
    fn unit_variance(self) -> Variance<InvariantSize> {
        self.domain_variance()
            .map_invariance(|invariance| invariance.as_bytes().len().into())
    }
}

#[derive(Clone, Debug)]
pub struct Repetition<'t, A = ()> {
    tokens: Vec<Token<'t, A>>,
    lower: usize,
    // This representation is not ideal, as it does not statically enforce the invariant that the
    // upper bound is greater than or equal to the lower bound. For example, this field could
    // instead be a summand. However, tokens must closely resemble their glob expression
    // representations so that errors in expressions can be deferred and presented more clearly.
    // Failures in the parser are difficult to describe.
    upper: Option<usize>,
}

impl<'t, A> Repetition<'t, A> {
    pub fn into_owned(self) -> Repetition<'static, A> {
        let Repetition {
            tokens,
            lower,
            upper,
        } = self;
        Repetition {
            tokens: tokens.into_iter().map(Token::into_owned).collect(),
            lower,
            upper,
        }
    }

    pub fn unannotate(self) -> Repetition<'t, ()> {
        let Repetition {
            tokens,
            lower,
            upper,
        } = self;
        Repetition {
            tokens: tokens.into_iter().map(Token::unannotate).collect(),
            lower,
            upper,
        }
    }

    pub fn tokens(&self) -> &Vec<Token<'t, A>> {
        &self.tokens
    }

    pub fn bounds(&self) -> (usize, Option<usize>) {
        (self.lower, self.upper)
    }

    pub fn is_converged(&self) -> bool {
        self.upper.map_or(false, |upper| self.lower == upper)
    }

    fn walk(&self) -> Walk<'_, 't, A> {
        Walk::from(&self.tokens)
    }
}

impl<'i, 't, A> UnitBreadth for &'i Repetition<'t, A> {
    fn unit_breadth(self) -> Boundedness {
        self.tokens().iter().composite_breadth()
    }
}

impl<'i, 't, A> UnitDepth for &'i Repetition<'t, A> {
    fn unit_depth(self) -> Boundedness {
        let (_, upper) = self.bounds();
        if upper.is_none() && self.walk().any(|(_, token)| token.is_component_boundary()) {
            Boundedness::Open
        } else {
            Boundedness::Closed
        }
    }
}

impl<'i, 't, A, T> UnitVariance<T> for &'i Repetition<'t, A>
where
    T: Invariance,
    &'i Token<'t, A>: UnitVariance<T>,
{
    fn unit_variance(self) -> Variance<T> {
        use Boundedness::Open;
        use TokenKind::Separator;
        use Variance::Variant;

        let variance = self
            .tokens()
            .iter()
            // Coalesce tokens with open variance with separators. This isn't destructive and
            // doesn't affect invariance, because this only happens in the presence of open
            // variance, which means that the repetition is variant (and has no invariant size or
            // text).
            .coalesce(|left, right| {
                match (
                    (left.kind(), left.unit_variance()),
                    (right.kind(), right.unit_variance()),
                ) {
                    ((Separator(_), _), (_, Variant(Open))) => Ok(right),
                    ((_, Variant(Open)), (Separator(_), _)) => Ok(left),
                    _ => Err((left, right)),
                }
            })
            .conjunctive_variance();
        match self.upper {
            // Repeating invariance can cause overflows, very large allocations,
            // and very inefficient comparisons (e.g., comparing very large
            // strings). This is detected by both `encode::compile` and
            // `rule::check` (in distinct but similar ways). Querying token
            // trees for their invariance must be done with care (after using
            // these functions) to avoid expanding pathological invariant
            // expressions like `<long:9999999999999>`.
            Some(_) if self.is_converged() => {
                variance.map_invariance(|invariance| invariance * self.lower)
            }
            _ => variance + Variant(Open),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Separator;

impl Separator {
    pub fn invariant_text() -> String {
        MAIN_SEPARATOR.to_string()
    }
}

impl<'i> UnitBreadth for &'i Separator {}

impl<'i> UnitDepth for &'i Separator {}

impl<'i, 't> UnitVariance<InvariantText<'t>> for &'i Separator {
    fn unit_variance(self) -> Variance<InvariantText<'t>> {
        Variance::Invariant(Separator::invariant_text().into_structural_text())
    }
}

impl<'i> UnitVariance<InvariantSize> for &'i Separator {
    fn unit_variance(self) -> Variance<InvariantSize> {
        Variance::Invariant(Separator::invariant_text().as_bytes().len().into())
    }
}

#[derive(Clone, Debug)]
pub enum Wildcard {
    One,
    ZeroOrMore(Evaluation),
    Tree { has_root: bool },
}

impl<'i> UnitBreadth for &'i Wildcard {
    fn unit_breadth(self) -> Boundedness {
        match self {
            Wildcard::One => Boundedness::Closed,
            _ => Boundedness::Open,
        }
    }
}

impl<'i> UnitDepth for &'i Wildcard {
    fn unit_depth(self) -> Boundedness {
        match self {
            Wildcard::Tree { .. } => Boundedness::Open,
            _ => Boundedness::Closed,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Position {
    Conjunctive { depth: usize },
    Disjunctive { depth: usize, branch: usize },
}

impl Position {
    pub fn depth(&self) -> usize {
        match self {
            Position::Conjunctive { ref depth } | Position::Disjunctive { ref depth, .. } => *depth,
        }
    }

    // This may appear to operate in place.
    #[must_use]
    fn converge(self) -> Self {
        match self {
            Position::Conjunctive { depth } | Position::Disjunctive { depth, .. } => {
                Position::Conjunctive { depth: depth + 1 }
            }
        }
    }

    // This may appear to operate in place.
    #[must_use]
    fn diverge(self, branch: usize) -> Self {
        match self {
            Position::Conjunctive { depth } | Position::Disjunctive { depth, .. } => {
                Position::Disjunctive {
                    depth: depth + 1,
                    branch,
                }
            }
        }
    }
}

impl Default for Position {
    fn default() -> Self {
        Position::Conjunctive { depth: 0 }
    }
}

#[derive(Clone, Debug)]
pub struct Walk<'i, 't, A> {
    buffer: VecDeque<(Position, &'i Token<'t, A>)>,
}

impl<'i, 't, A> Walk<'i, 't, A>
where
    't: 'i,
    A: 't,
{
    pub fn starting(self) -> impl 'i + Iterator<Item = (Position, &'i Token<'t, A>)> {
        self.peekable().batching(|tokens| {
            if let Some((position, token)) = tokens.next() {
                tokens
                    .peeking_take_while(|(next, _)| *next == position)
                    .for_each(drop);
                Some((position, token))
            } else {
                None
            }
        })
    }

    pub fn ending(self) -> impl 'i + Iterator<Item = (Position, &'i Token<'t, A>)> {
        self.peekable().batching(|tokens| {
            if let Some((position, _)) = tokens.peek().copied() {
                tokens
                    .peeking_take_while(|(next, _)| *next == position)
                    .last()
            } else {
                None
            }
        })
    }
}

impl<'i, 't, A> From<&'i Token<'t, A>> for Walk<'i, 't, A> {
    fn from(token: &'i Token<'t, A>) -> Self {
        Walk {
            buffer: Some((Position::default(), token)).into_iter().collect(),
        }
    }
}

impl<'i, 't, A> From<&'i Vec<Token<'t, A>>> for Walk<'i, 't, A> {
    fn from(tokens: &'i Vec<Token<'t, A>>) -> Self {
        Walk {
            buffer: tokens
                .iter()
                .map(|token| (Position::default(), token))
                .collect(),
        }
    }
}

impl<'i, 't, A> Iterator for Walk<'i, 't, A>
where
    't: 'i,
    A: 't,
{
    type Item = (Position, &'i Token<'t, A>);

    fn next(&mut self) -> Option<Self::Item> {
        if let Some((position, token)) = self.buffer.pop_front() {
            match token.kind() {
                TokenKind::Alternative(ref alternative) => {
                    self.buffer
                        .extend(alternative.branches().iter().enumerate().flat_map(
                            |(branch, tokens)| {
                                tokens
                                    .iter()
                                    .map(move |token| (position.diverge(branch), token))
                            },
                        ));
                }
                TokenKind::Repetition(ref repetition) => {
                    self.buffer.extend(
                        repetition
                            .tokens()
                            .iter()
                            .map(|token| (position.converge(), token)),
                    );
                }
                _ => {}
            }
            Some((position, token))
        } else {
            None
        }
    }
}

#[derive(Clone, Debug)]
pub struct LiteralSequence<'i, 't>(Vec<&'i Literal<'t>>);

impl<'i, 't> LiteralSequence<'i, 't> {
    pub fn literals(&self) -> &[&'i Literal<'t>] {
        self.0.as_ref()
    }

    pub fn text(&self) -> Cow<'t, str> {
        if self.literals().len() == 1 {
            self.literals().first().unwrap().text.clone()
        } else {
            self.literals()
                .iter()
                .map(|literal| &literal.text)
                .join("")
                .into()
        }
    }

    #[cfg(any(unix, windows))]
    pub fn is_semantic_literal(&self) -> bool {
        matches!(self.text().as_ref(), "." | "..")
    }

    #[cfg(not(any(unix, windows)))]
    pub fn is_semantic_literal(&self) -> bool {
        false
    }
}

#[derive(Debug)]
pub struct Component<'i, 't, A = ()>(Vec<&'i Token<'t, A>>);

impl<'i, 't, A> Component<'i, 't, A> {
    pub fn tokens(&self) -> &[&'i Token<'t, A>] {
        self.0.as_ref()
    }

    pub fn literal(&self) -> Option<LiteralSequence<'i, 't>> {
        if self.0.is_empty() {
            None
        } else {
            self.tokens()
                .iter()
                .all(|token| matches!(token.kind(), TokenKind::Literal(_)))
                .then(|| {
                    LiteralSequence(
                        self.tokens()
                            .iter()
                            .map(|token| match token.kind() {
                                TokenKind::Literal(ref literal) => literal,
                                _ => unreachable!(), // See predicate above.
                            })
                            .collect(),
                    )
                })
        }
    }

    pub fn variance<T>(&self) -> Variance<T>
    where
        T: Invariance,
        &'i Token<'t, A>: UnitVariance<T>,
    {
        self.0.iter().copied().conjunctive_variance()
    }

    pub fn depth(&self) -> Boundedness {
        self.0.iter().copied().composite_depth()
    }
}

impl<'i, 't, A> Clone for Component<'i, 't, A> {
    fn clone(&self) -> Self {
        Component(self.0.clone())
    }
}

pub fn any<'t, A, I>(tokens: I) -> Token<'t, ()>
where
    I: IntoIterator,
    I::Item: IntoIterator<Item = Token<'t, A>>,
{
    Token {
        kind: Alternative(
            tokens
                .into_iter()
                .map(|tokens| tokens.into_iter().map(Token::unannotate).collect())
                .collect(),
        )
        .into(),
        annotation: (),
    }
}

pub fn components<'i, 't, A, I>(tokens: I) -> impl Iterator<Item = Component<'i, 't, A>>
where
    't: 'i,
    A: 't,
    I: IntoIterator<Item = &'i Token<'t, A>>,
{
    tokens.into_iter().peekable().batching(|tokens| {
        let mut first = tokens.next();
        while matches!(first.map(Token::kind), Some(TokenKind::Separator(_))) {
            first = tokens.next();
        }
        first.map(|first| match first.kind() {
            TokenKind::Wildcard(Wildcard::Tree { .. }) => Component(vec![first]),
            _ => Component(
                Some(first)
                    .into_iter()
                    .chain(tokens.peeking_take_while(|token| !token.is_component_boundary()))
                    .collect(),
            ),
        })
    })
}

// TODO: This implementation allocates many `Vec`s.
pub fn literals<'i, 't, A, I>(
    tokens: I,
) -> impl Iterator<Item = (Component<'i, 't, A>, LiteralSequence<'i, 't>)>
where
    't: 'i,
    A: 't,
    I: IntoIterator<Item = &'i Token<'t, A>>,
{
    components(tokens).flat_map(|component| {
        if let Some(literal) = component.literal() {
            vec![(component, literal)]
        } else {
            component
                .tokens()
                .iter()
                .filter_map(|token| match token.kind() {
                    TokenKind::Alternative(ref alternative) => Some(
                        alternative
                            .branches()
                            .iter()
                            .flat_map(literals)
                            .collect::<Vec<_>>(),
                    ),
                    TokenKind::Repetition(ref repetition) => {
                        Some(literals(repetition.tokens()).collect::<Vec<_>>())
                    }
                    _ => None,
                })
                .flatten()
                .collect::<Vec<_>>()
        }
    })
}

#[cfg(test)]
mod tests {
    use crate::token::{self, TokenKind, TokenTree};

    #[test]
    fn literal_case_insensitivity() {
        let tokenized = token::parse("(?-i)../foo/(?i)**/bar/**(?-i)/baz/*(?i)qux").unwrap();
        let literals: Vec<_> = tokenized
            .tokens()
            .iter()
            .filter_map(|token| match token.kind {
                TokenKind::Literal(ref literal) => Some(literal),
                _ => None,
            })
            .collect();

        assert!(!literals[0].is_case_insensitive); // `..`
        assert!(!literals[1].is_case_insensitive); // `foo`
        assert!(literals[2].is_case_insensitive); // `bar`
        assert!(!literals[3].is_case_insensitive); // `baz`
        assert!(literals[4].is_case_insensitive); // `qux`
    }
}
