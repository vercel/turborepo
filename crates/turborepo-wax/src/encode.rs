use std::borrow::{Borrow, Cow};
#[cfg(feature = "miette")]
use std::fmt::Display;

use const_format::formatcp;
use itertools::{Itertools as _, Position};
#[cfg(feature = "miette")]
use miette::Diagnostic;
use regex::{Error as RegexError, Regex};
use thiserror::Error;

use crate::token::Token;

/// A regular expression that never matches.
///
/// This expression is formed from a character class that intersects completely
/// disjoint characters. Unlike an empty regular expression, which always
/// matches, this yields an empty character class, which never matches (even
/// against empty strings).
const NEVER_EXPRESSION: &str = "[a&&b]";

#[cfg(windows)]
const SEPARATOR_CLASS_EXPRESSION: &str = "/\\\\";
#[cfg(unix)]
const SEPARATOR_CLASS_EXPRESSION: &str = "/";

// This only encodes the platform's main separator, so any additional separators
// will be missed. It may be better to have explicit platform support and invoke
// `compile_error!` on unsupported platforms, as this could cause very aberrant
// behavior. Then again, it seems that platforms using more than one separator
// are rare. GS/OS, OS/2, and Windows are likely the best known examples
// and of those only Windows is a supported Rust target at the time of writing
// (and is already supported by Wax).
#[cfg(not(any(windows, unix)))]
const SEPARATOR_CLASS_EXPRESSION: &str = main_separator_class_expression();

#[cfg(not(any(windows, unix)))]
const fn main_separator_class_expression() -> &'static str {
    use std::path::MAIN_SEPARATOR;

    // TODO: This is based upon `regex_syntax::is_meta_character`, but that function
    // is not       `const`. Perhaps that can be changed upstream.
    const fn escape(x: char) -> &'static str {
        match x {
            '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$'
            | '#' | '&' | '-' | '~' => "\\",
            _ => "",
        }
    }

    formatcp!("{0}{1}", escape(MAIN_SEPARATOR), MAIN_SEPARATOR)
}

macro_rules! sepexpr {
    ($fmt:expr) => {
        formatcp!($fmt, formatcp!("[{0}]", SEPARATOR_CLASS_EXPRESSION))
    };
}

macro_rules! nsepexpr {
    ($fmt:expr) => {
        formatcp!($fmt, formatcp!("[^{0}]", SEPARATOR_CLASS_EXPRESSION))
    };
}

/// Describes errors that occur when compiling a glob expression.
///
/// **This error only occurs when the size of the compiled program is too
/// large.** All other compilation errors are considered internal bugs and will
/// panic.
#[derive(Clone, Debug, Error)]
#[error("failed to compile glob: {kind}")]
pub struct CompileError {
    kind: CompileErrorKind,
}

#[derive(Clone, Copy, Debug, Error)]
#[non_exhaustive]
enum CompileErrorKind {
    #[error("oversized program")]
    OversizedProgram,
}

#[cfg(feature = "miette")]
#[cfg_attr(docsrs, doc(cfg(feature = "miette")))]
impl Diagnostic for CompileError {
    fn code<'a>(&'a self) -> Option<Box<dyn 'a + Display>> {
        Some(Box::new(String::from(match self.kind {
            CompileErrorKind::OversizedProgram => "wax::glob::oversized_program",
        })))
    }
}

trait Escaped {
    fn escaped(&self) -> String;
}

impl Escaped for char {
    fn escaped(&self) -> String {
        regex::escape(&self.to_string())
    }
}

impl Escaped for str {
    fn escaped(&self) -> String {
        regex::escape(self)
    }
}

#[derive(Clone, Copy, Debug)]
enum Grouping {
    Capture,
    NonCapture,
}

impl Grouping {
    fn push_open(&self, pattern: &mut String) {
        match self {
            Grouping::Capture => pattern.push('('),
            Grouping::NonCapture => pattern.push_str("(?:"),
        }
    }

    fn push_close(pattern: &mut String) {
        pattern.push(')');
    }

    fn push_str(&self, pattern: &mut String, encoding: &str) {
        self.push_open(pattern);
        pattern.push_str(encoding);
        Grouping::push_close(pattern);
    }

    fn push_with<'p, F>(&self, pattern: &mut String, f: F)
    where
        F: Fn() -> Cow<'p, str>,
    {
        self.push_open(pattern);
        pattern.push_str(f().as_ref());
        Grouping::push_close(pattern);
    }
}

pub fn case_folded_eq(left: &str, right: &str) -> bool {
    let Ok(regex) = Regex::new(&format!("(?i){}", regex::escape(left))) else {
        return false;
    };
    if let Some(matched) = regex.find(right) {
        matched.start() == 0 && matched.end() == right.len()
    } else {
        false
    }
}

fn class_matches_anything(pattern: &str) -> bool {
    let Ok(hir) = regex_syntax::Parser::new().parse(pattern) else {
        return false;
    };
    !matches!(
        hir.kind(),
        regex_syntax::hir::HirKind::Class(class) if class.is_empty()
    )
}

pub fn compile<'t, A, T>(tokens: impl IntoIterator<Item = T>) -> Result<Regex, CompileError>
where
    T: Borrow<Token<'t, A>>,
{
    let mut pattern = String::new();
    pattern.push('^');
    encode(Grouping::Capture, None, &mut pattern, tokens);
    pattern.push('$');
    Regex::new(&pattern).map_err(|error| match error {
        RegexError::CompiledTooBig(_) => CompileError {
            kind: CompileErrorKind::OversizedProgram,
        },
        _ => panic!("failed to compile glob"),
    })
}

fn encode<'t, A, T>(
    grouping: Grouping,
    superposition: Option<Position>,
    pattern: &mut String,
    tokens: impl IntoIterator<Item = T>,
) where
    T: Borrow<Token<'t, A>>,
{
    use itertools::Position::{First, Last, Middle, Only};

    use crate::token::{
        Archetype::{Character, Range},
        Evaluation::{Eager, Lazy},
        TokenKind::{Alternative, Class, Literal, Repetition, Separator, Wildcard},
        Wildcard::{One, Tree, ZeroOrMore},
    };

    fn encode_intermediate_tree(grouping: Grouping, pattern: &mut String) {
        let invariant_grouping = Grouping::NonCapture;
        invariant_grouping.push_open(pattern);
        pattern.push_str(sepexpr!("{0}|{0}"));
        grouping.push_str(pattern, sepexpr!(".*{0}"));
        Grouping::push_close(pattern);
    }
    let mut is_case_insensitive = None;
    for (position, token) in tokens.into_iter().with_position() {
        match (position, token.borrow().kind()) {
            (_, Literal(literal)) => {
                // TODO: Should Unicode support also be toggled by casing flags?
                let case_insensitive = literal.is_case_insensitive();
                if is_case_insensitive != Some(case_insensitive) {
                    pattern.push_str(if case_insensitive { "(?i)" } else { "(?-i)" });
                    is_case_insensitive = Some(case_insensitive);
                }
                pattern.push_str(&literal.text().escaped());
            }
            (_, Separator(_)) => pattern.push_str(sepexpr!("{0}")),
            (position, Alternative(alternative)) => {
                let encodings: Vec<_> = alternative
                    .branches()
                    .iter()
                    .map(|tokens| {
                        let mut pattern = String::new();
                        let invariant_grouping = Grouping::NonCapture;
                        invariant_grouping.push_open(&mut pattern);
                        encode(
                            invariant_grouping,
                            superposition.or(Some(position)),
                            &mut pattern,
                            tokens.iter(),
                        );
                        Grouping::push_close(&mut pattern);
                        pattern
                    })
                    .collect();
                grouping.push_str(pattern, &encodings.join("|"));
            }
            (position, Repetition(repetition)) => {
                let encoding = {
                    let (lower, upper) = repetition.bounds();
                    let mut pattern = String::new();
                    let invariant_grouping = Grouping::NonCapture;
                    invariant_grouping.push_open(&mut pattern);
                    encode(
                        invariant_grouping,
                        superposition.or(Some(position)),
                        &mut pattern,
                        repetition.tokens().iter(),
                    );
                    Grouping::push_close(&mut pattern);
                    pattern.push_str(&if let Some(upper) = upper {
                        format!("{{{},{}}}", lower, upper)
                    } else {
                        format!("{{{},}}", lower)
                    });
                    pattern
                };
                grouping.push_str(pattern, &encoding);
            }
            (_, Class(class)) => {
                grouping.push_with(pattern, || {
                    use crate::token::Class as ClassToken;

                    fn encode_class_archetypes(class: &ClassToken, pattern: &mut String) {
                        for archetype in class.archetypes() {
                            match archetype {
                                Character(literal) => pattern.push_str(&literal.escaped()),
                                Range(left, right) => {
                                    pattern.push_str(&left.escaped());
                                    pattern.push('-');
                                    pattern.push_str(&right.escaped());
                                }
                            }
                        }
                    }

                    let mut pattern = String::new();
                    pattern.push('[');
                    if class.is_negated() {
                        pattern.push('^');
                        encode_class_archetypes(class, &mut pattern);
                        pattern.push_str(SEPARATOR_CLASS_EXPRESSION);
                    } else {
                        encode_class_archetypes(class, &mut pattern);
                        pattern.push_str(nsepexpr!("&&{0}"));
                    }
                    pattern.push(']');
                    // Parse the class without compiling a complete `Regex`. The result may be
                    // empty if separator subtraction removes every character in the class.
                    if class_matches_anything(&pattern) {
                        pattern.into()
                    } else {
                        // If parsing fails or the class is empty, use `NEVER_EXPRESSION`, which
                        // matches nothing.
                        NEVER_EXPRESSION.into()
                    }
                });
            }
            (_, Wildcard(One)) => grouping.push_str(pattern, nsepexpr!("{0}")),
            (_, Wildcard(ZeroOrMore(Eager))) => grouping.push_str(pattern, nsepexpr!("{0}*")),
            (_, Wildcard(ZeroOrMore(Lazy))) => grouping.push_str(pattern, nsepexpr!("{0}*?")),
            (First, Wildcard(Tree { has_root })) => {
                if let Some(Middle | Last) = superposition {
                    encode_intermediate_tree(grouping, pattern);
                } else if *has_root {
                    grouping.push_str(pattern, sepexpr!("{0}.*{0}?"));
                } else {
                    let invariant_grouping = Grouping::NonCapture;
                    invariant_grouping.push_open(pattern);
                    pattern.push_str(sepexpr!("{0}?|"));
                    grouping.push_str(pattern, sepexpr!(".*{0}"));
                    Grouping::push_close(pattern);
                }
            }
            (Middle, Wildcard(Tree { .. })) => {
                encode_intermediate_tree(grouping, pattern);
            }
            (Last, Wildcard(Tree { .. })) => {
                if let Some(First | Middle) = superposition {
                    encode_intermediate_tree(grouping, pattern);
                } else {
                    let invariant_grouping = Grouping::NonCapture;
                    invariant_grouping.push_open(pattern);
                    pattern.push_str(sepexpr!("{0}?|{0}"));
                    grouping.push_str(pattern, ".*");
                    Grouping::push_close(pattern);
                }
            }
            (Only, Wildcard(Tree { .. })) => grouping.push_str(pattern, ".*"),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{encode, token::TokenTree};
    #[test]
    fn class_matches_anything_checks_syntax_and_empty_intersections() {
        assert!(encode::class_matches_anything("[ab&&[^a]]"));
        assert!(!encode::class_matches_anything("[a&&[^a]]"));
        assert!(!encode::class_matches_anything("[z-a]"));
    }

    #[test]
    fn class_restricted_to_separators_matches_nothing() {
        let tokens = crate::token::parse("[/]").unwrap();
        let regex = encode::compile(tokens.tokens().iter()).unwrap();

        assert!(!regex.is_match("/"));
    }

    #[test]
    fn case_folded_eq() {
        assert!(encode::case_folded_eq("a", "a"));
        assert!(encode::case_folded_eq("a", "A"));

        assert!(!encode::case_folded_eq("a", "b"));
        assert!(!encode::case_folded_eq("aa", "a"));
        assert!(!encode::case_folded_eq("a", "aa"));
    }

    #[test]
    fn only_encodes_changes_to_casing_flags() {
        let tokens = crate::token::parse("(?i)a(?i)b(?-i)c(?-i)d").unwrap();
        let regex = encode::compile(tokens.tokens().iter()).unwrap();

        assert_eq!(regex.as_str(), "^(?i)ab(?-i)cd$");
    }

    #[test]
    fn casing_flags_in_groups_do_not_change_outer_state() {
        let tokens = crate::token::parse("(?-i)a{(?i)b,(?-i)c}(?-i)d").unwrap();
        let regex = encode::compile(tokens.tokens().iter()).unwrap();

        assert!(regex.is_match("aBd"));
        assert!(!regex.is_match("aBD"));
        assert!(!regex.is_match("Abd"));
    }
}
