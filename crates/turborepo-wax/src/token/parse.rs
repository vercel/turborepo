use std::{
    borrow::Cow,
    fmt::{self, Display, Formatter},
    str::FromStr,
};

#[cfg(feature = "miette")]
use miette::{self, Diagnostic, LabeledSpan, SourceCode};
use nom::error::{VerboseError as NomError, VerboseErrorKind as NomErrorKind};
use pori::{Located, Location, Stateful};
use thiserror::Error;

use crate::{
    diagnostics::{LocatedError, Span},
    token::{
        Alternative, Archetype, Class, Evaluation, Literal, Repetition, Separator, Token,
        TokenKind, Tokenized, Wildcard,
    },
    PATHS_ARE_CASE_INSENSITIVE,
};

pub type Annotation = Span;

type Expression<'i> = Located<'i, str>;
type Input<'i> = Stateful<Expression<'i>, ParserState>;
type ErrorStack<'i> = NomError<Input<'i>>;
type ErrorMode<'i> = nom::Err<ErrorStack<'i>>;

pub const ROOT_SEPARATOR_EXPRESSION: &str = "/";

#[derive(Clone, Debug)]
pub struct ErrorEntry<'t> {
    fragment: Cow<'t, str>,
    location: usize,
    kind: NomErrorKind,
}

impl<'t> ErrorEntry<'t> {
    pub fn into_owned(self) -> ErrorEntry<'static> {
        let ErrorEntry {
            fragment,
            location,
            kind,
        } = self;
        ErrorEntry {
            fragment: fragment.into_owned().into(),
            location,
            kind,
        }
    }
}

impl<'t> From<(Input<'t>, NomErrorKind)> for ErrorEntry<'t> {
    fn from((input, kind): (Input<'t>, NomErrorKind)) -> Self {
        let location = input.location();
        ErrorEntry {
            fragment: input.into_data().into(),
            location,
            kind,
        }
    }
}

#[cfg(feature = "miette")]
impl From<ErrorEntry<'_>> for LabeledSpan {
    fn from(error: ErrorEntry<'_>) -> Self {
        let span = error.span();
        LabeledSpan::new_with_span(Some(format!("{}", error)), span)
    }
}

impl Display for ErrorEntry<'_> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self.kind {
            NomErrorKind::Char(expected) => {
                if let Some(got) = self.fragment.chars().next() {
                    write!(f, "expected `{}`, got `{}`", expected, got)
                } else {
                    write!(f, "expected `{}`, got end of input", expected)
                }
            }
            NomErrorKind::Context(context) => write!(f, "in context `{}`", context),
            NomErrorKind::Nom(parser) => write!(f, "in sub-parser `{:?}`", parser),
        }
    }
}

impl LocatedError for ErrorEntry<'_> {
    fn span(&self) -> Span {
        (self.location, 1)
    }
}

/// Describes errors that occur when parsing a glob expression.
///
/// Common examples of glob expressions that cannot be parsed are alternative
/// and repetition patterns with missing delimiters and ambiguous patterns, such
/// as `src/***/*.rs` or `{.local,.config/**/*.toml`.
#[derive(Clone, Debug, Error)]
#[error("failed to parse glob expression")]
pub struct ParseError<'t> {
    expression: Cow<'t, str>,
    locations: Vec<ErrorEntry<'t>>,
}

impl<'t> ParseError<'t> {
    fn new(expression: &'t str, error: ErrorMode<'t>) -> Self {
        match error {
            ErrorMode::Incomplete(_) => {
                panic!("unexpected parse error: incomplete input")
            }
            ErrorMode::Error(stack) | ErrorMode::Failure(stack) => ParseError {
                expression: expression.into(),
                locations: stack.errors.into_iter().map(From::from).collect(),
            },
        }
    }

    /// Clones any borrowed data into an owning instance.
    pub fn into_owned(self) -> ParseError<'static> {
        let ParseError {
            expression,
            locations,
        } = self;
        ParseError {
            expression: expression.into_owned().into(),
            locations: locations.into_iter().map(ErrorEntry::into_owned).collect(),
        }
    }

    pub fn locations(&self) -> &[ErrorEntry<'t>] {
        &self.locations
    }

    /// Gets the glob expression that failed to parse.
    pub fn expression(&self) -> &str {
        self.expression.as_ref()
    }
}

#[cfg(feature = "miette")]
#[cfg_attr(docsrs, doc(cfg(feature = "miette")))]
impl Diagnostic for ParseError<'_> {
    fn code<'a>(&'a self) -> Option<Box<dyn Display + 'a>> {
        Some(Box::new("wax::glob::parse"))
    }

    fn source_code(&self) -> Option<&dyn SourceCode> {
        Some(&self.expression)
    }

    fn labels(&self) -> Option<Box<dyn Iterator<Item = LabeledSpan> + '_>> {
        Some(Box::new(self.locations.iter().cloned().map(From::from)))
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ParserState {
    flags: FlagState,
    subexpression: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FlagState {
    is_case_insensitive: bool,
}

impl Default for FlagState {
    fn default() -> Self {
        FlagState {
            is_case_insensitive: PATHS_ARE_CASE_INSENSITIVE,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum FlagToggle {
    CaseInsensitive(bool),
}

pub fn parse(expression: &str) -> Result<Tokenized, ParseError> {
    use nom::{
        branch, bytes::complete as bytes, character::complete as character, combinator, error,
        multi, sequence, IResult, Parser,
    };

    use crate::token::parse::FlagToggle::CaseInsensitive;

    type ParseResult<'i, O> = IResult<Input<'i>, O, ErrorStack<'i>>;

    fn boe(input: Input) -> ParseResult<Input> {
        if input.state.subexpression == input.location() {
            Ok((input, input))
        } else {
            Err(ErrorMode::Error(ErrorStack {
                errors: vec![(input, NomErrorKind::Context("beginning of expression"))],
            }))
        }
    }

    fn flags<'i, F>(
        mut toggle: impl FnMut(FlagToggle) -> F,
    ) -> impl FnMut(Input<'i>) -> ParseResult<'i, ()>
    where
        F: Parser<Input<'i>, (), ErrorStack<'i>>,
    {
        move |input| {
            let (input, _) = multi::many0(sequence::delimited(
                bytes::tag("(?"),
                multi::many1(branch::alt((
                    sequence::tuple((bytes::tag("i"), toggle(CaseInsensitive(true)))),
                    sequence::tuple((bytes::tag("-i"), toggle(CaseInsensitive(false)))),
                ))),
                bytes::tag(")"),
            ))(input)?;
            Ok((input, ()))
        }
    }

    // Explicit lifetimes prevent inference errors.
    #[allow(clippy::needless_lifetimes)]
    fn flags_with_state<'i>(input: Input<'i>) -> ParseResult<'i, ()> {
        flags(move |toggle| {
            move |mut input: Input<'i>| {
                match toggle {
                    CaseInsensitive(toggle) => {
                        input.state.flags.is_case_insensitive = toggle;
                    }
                }
                Ok((input, ()))
            }
        })(input)
    }

    // Explicit lifetimes prevent inference errors.
    #[allow(clippy::needless_lifetimes)]
    fn flags_without_state<'i>(input: Input<'i>) -> ParseResult<'i, ()> {
        flags(move |_| move |input: Input<'i>| Ok((input, ())))(input)
    }

    fn literal(input: Input) -> ParseResult<TokenKind<Annotation>> {
        combinator::map(
            combinator::verify(
                bytes::escaped_transform(
                    bytes::is_not("/?*$:<>()[]{},\\"),
                    '\\',
                    branch::alt((
                        combinator::value("?", bytes::tag("?")),
                        combinator::value("*", bytes::tag("*")),
                        combinator::value("$", bytes::tag("$")),
                        combinator::value(":", bytes::tag(":")),
                        combinator::value("<", bytes::tag("<")),
                        combinator::value(">", bytes::tag(">")),
                        combinator::value("(", bytes::tag("(")),
                        combinator::value(")", bytes::tag(")")),
                        combinator::value("[", bytes::tag("[")),
                        combinator::value("]", bytes::tag("]")),
                        combinator::value("{", bytes::tag("{")),
                        combinator::value("}", bytes::tag("}")),
                        combinator::value(",", bytes::tag(",")),
                    )),
                ),
                |text: &str| !text.is_empty(),
            ),
            move |text| {
                TokenKind::Literal(Literal {
                    text: text.into(),
                    is_case_insensitive: input.state.flags.is_case_insensitive,
                })
            },
        )(input)
    }

    fn separator(input: Input) -> ParseResult<TokenKind<Annotation>> {
        combinator::value(TokenKind::Separator(Separator), bytes::tag("/"))(input)
    }

    fn wildcard<'i>(
        terminator: impl Clone + Parser<Input<'i>, Input<'i>, ErrorStack<'i>>,
    ) -> impl FnMut(Input<'i>) -> ParseResult<'i, TokenKind<'i, Annotation>> {
        branch::alt((
            error::context(
                "exactly-one",
                combinator::map(bytes::tag("?"), |_| TokenKind::from(Wildcard::One)),
            ),
            error::context(
                "tree",
                combinator::map(
                    sequence::tuple((
                        error::context(
                            "prefix",
                            combinator::map(
                                branch::alt((
                                    sequence::tuple((
                                        combinator::value(true, bytes::tag("/")),
                                        flags_with_state,
                                    )),
                                    sequence::tuple((
                                        combinator::value(false, boe),
                                        flags_with_state,
                                    )),
                                )),
                                |(prefix, _)| prefix,
                            ),
                        ),
                        sequence::terminated(
                            bytes::tag("**"),
                            error::context(
                                "postfix",
                                branch::alt((
                                    combinator::map(
                                        sequence::tuple((flags_with_state, bytes::tag("/"))),
                                        |(_, postfix)| postfix,
                                    ),
                                    terminator.clone(),
                                )),
                            ),
                        ),
                    )),
                    |(has_root, _)| Wildcard::Tree { has_root }.into(),
                ),
            ),
            error::context(
                "zero-or-more",
                combinator::map(
                    sequence::terminated(
                        bytes::tag("*"),
                        branch::alt((
                            combinator::map(
                                combinator::peek(sequence::tuple((
                                    flags_without_state,
                                    error::context("no terminating wildcard", bytes::is_not("*$")),
                                ))),
                                |(_, right)| right,
                            ),
                            terminator.clone(),
                        )),
                    ),
                    |_| Wildcard::ZeroOrMore(Evaluation::Eager).into(),
                ),
            ),
            error::context(
                "zero-or-more",
                combinator::map(
                    sequence::terminated(
                        bytes::tag("$"),
                        branch::alt((
                            combinator::map(
                                combinator::peek(sequence::tuple((
                                    flags_without_state,
                                    error::context("no terminating wildcard", bytes::is_not("*$")),
                                ))),
                                |(_, right)| right,
                            ),
                            terminator,
                        )),
                    ),
                    |_| Wildcard::ZeroOrMore(Evaluation::Lazy).into(),
                ),
            ),
        ))
    }

    fn repetition(input: Input) -> ParseResult<TokenKind<Annotation>> {
        fn bounds(input: Input) -> ParseResult<(usize, Option<usize>)> {
            type BoundResult<T> = Result<T, <usize as FromStr>::Err>;

            branch::alt((
                sequence::preceded(
                    bytes::tag(":"),
                    branch::alt((
                        error::context(
                            "range",
                            combinator::map_res(
                                sequence::separated_pair(
                                    character::digit1,
                                    bytes::tag(","),
                                    combinator::opt(character::digit1),
                                ),
                                |(lower, upper): (Input, Option<_>)| -> BoundResult<_> {
                                    let lower = lower.parse::<usize>()?;
                                    let upper =
                                        upper.map(|upper| upper.parse::<usize>()).transpose()?;
                                    Ok((lower, upper))
                                },
                            ),
                        ),
                        error::context(
                            "converged",
                            combinator::map_res(character::digit1, |n: Input| -> BoundResult<_> {
                                let n = n.parse::<usize>()?;
                                Ok((n, Some(n)))
                            }),
                        ),
                        combinator::success((1, None)),
                    )),
                ),
                combinator::success((0, None)),
            ))(input)
        }

        combinator::map(
            sequence::delimited(
                bytes::tag("<"),
                sequence::tuple((
                    error::context(
                        "sub-glob",
                        glob(move |input| {
                            combinator::peek(branch::alt((bytes::tag(":"), bytes::tag(">"))))(input)
                        }),
                    ),
                    error::context("bounds", bounds),
                )),
                bytes::tag(">"),
            ),
            |(tokens, (lower, upper))| {
                Repetition {
                    tokens,
                    lower,
                    upper,
                }
                .into()
            },
        )(input)
    }

    fn class(input: Input) -> ParseResult<TokenKind<Annotation>> {
        fn archetypes(input: Input) -> ParseResult<Vec<Archetype>> {
            let escaped_character = |input| {
                branch::alt((
                    character::none_of("[]-\\"),
                    branch::alt((
                        combinator::value('[', bytes::tag("\\[")),
                        combinator::value(']', bytes::tag("\\]")),
                        combinator::value('-', bytes::tag("\\-")),
                    )),
                ))(input)
            };

            multi::many1(branch::alt((
                combinator::map(
                    sequence::separated_pair(escaped_character, bytes::tag("-"), escaped_character),
                    Archetype::from,
                ),
                combinator::map(escaped_character, Archetype::from),
            )))(input)
        }

        combinator::map(
            sequence::delimited(
                bytes::tag("["),
                sequence::tuple((
                    combinator::opt(bytes::tag("!").or(bytes::tag("^"))),
                    archetypes,
                )),
                bytes::tag("]"),
            ),
            |(negation, archetypes)| {
                Class {
                    is_negated: negation.is_some(),
                    archetypes,
                }
                .into()
            },
        )(input)
    }

    fn alternative(input: Input) -> ParseResult<TokenKind<Annotation>> {
        sequence::delimited(
            bytes::tag("{"),
            combinator::map(
                multi::separated_list1(
                    bytes::tag(","),
                    error::context(
                        "sub-glob",
                        glob(move |input| {
                            combinator::peek(branch::alt((bytes::tag(","), bytes::tag("}"))))(input)
                        }),
                    ),
                ),
                |alternatives: Vec<Vec<_>>| Alternative::from(alternatives).into(),
            ),
            bytes::tag("}"),
        )(input)
    }

    fn glob<'i>(
        terminator: impl 'i + Clone + Parser<Input<'i>, Input<'i>, ErrorStack<'i>>,
    ) -> impl Parser<Input<'i>, Vec<Token<'i, Annotation>>, ErrorStack<'i>> {
        fn annotate<'i, F>(
            parser: F,
        ) -> impl FnMut(Input<'i>) -> ParseResult<'i, Token<'i, Annotation>>
        where
            F: 'i + Parser<Input<'i>, TokenKind<'i, Annotation>, ErrorStack<'i>>,
        {
            combinator::map(pori::span(parser), |(span, kind)| Token::new(kind, span))
        }

        move |mut input: Input<'i>| {
            input.state.subexpression = input.location();
            sequence::terminated(
                multi::many1(branch::alt((
                    annotate(error::context(
                        "literal",
                        sequence::preceded(flags_with_state, literal),
                    )),
                    annotate(error::context(
                        "repetition",
                        sequence::preceded(flags_with_state, repetition),
                    )),
                    annotate(error::context(
                        "alternative",
                        sequence::preceded(flags_with_state, alternative),
                    )),
                    annotate(error::context(
                        "wildcard",
                        sequence::preceded(flags_with_state, wildcard(terminator.clone())),
                    )),
                    annotate(error::context(
                        "class",
                        sequence::preceded(flags_with_state, class),
                    )),
                    annotate(error::context(
                        "separator",
                        sequence::preceded(flags_with_state, separator),
                    )),
                ))),
                terminator.clone(),
            )
            .parse(input)
        }
    }

    if expression.is_empty() {
        Ok(Tokenized {
            expression: expression.into(),
            tokens: vec![],
        })
    } else {
        let input = Input::new(Expression::from(expression), ParserState::default());
        let tokens = combinator::all_consuming(glob(combinator::eof))(input)
            .map(|(_, tokens)| tokens)
            .map_err(|error| ParseError::new(expression, error))?;
        Ok(Tokenized {
            expression: expression.into(),
            tokens,
        })
    }
}
