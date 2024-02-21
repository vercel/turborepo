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
    pub fn push_str(&self, pattern: &mut String, encoding: &str) {
        self.push_with(pattern, || encoding.into());
    }

    pub fn push_with<'p, F>(&self, pattern: &mut String, f: F)
    where
        F: Fn() -> Cow<'p, str>,
    {
        match self {
            Grouping::Capture => pattern.push('('),
            Grouping::NonCapture => pattern.push_str("(?:"),
        }
        pattern.push_str(f().as_ref());
        pattern.push(')');
    }
}

pub fn case_folded_eq(left: &str, right: &str) -> bool {
    let regex = Regex::new(&format!("(?i){}", regex::escape(left)))
        .expect("failed to compile literal regular expression");
    if let Some(matched) = regex.find(right) {
        matched.start() == 0 && matched.end() == right.len()
    } else {
        false
    }
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

// TODO: Some versions of `const_format` in `^0.2.0` fail this lint in
// `formatcp`. See       https://github.com/rodrimati1992/const_format_crates/issues/38
#[allow(clippy::double_parens)]
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
        pattern.push_str(sepexpr!("(?:{0}|{0}"));
        grouping.push_str(pattern, sepexpr!(".*{0}"));
        pattern.push(')');
    }

    // TODO: Use `Grouping` everywhere a group is encoded. For invariant groups that
    // ignore       `grouping`, construct a local `Grouping` instead.
    for (position, token) in tokens.into_iter().with_position() {
        match (position, token.borrow().kind()) {
            (_, Literal(literal)) => {
                // TODO: Only encode changes to casing flags.
                // TODO: Should Unicode support also be toggled by casing flags?
                if literal.is_case_insensitive() {
                    pattern.push_str("(?i)");
                } else {
                    pattern.push_str("(?-i)");
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
                        pattern.push_str("(?:");
                        encode(
                            Grouping::NonCapture,
                            superposition.or(Some(position)),
                            &mut pattern,
                            tokens.iter(),
                        );
                        pattern.push(')');
                        pattern
                    })
                    .collect();
                grouping.push_str(pattern, &encodings.join("|"));
            }
            (position, Repetition(repetition)) => {
                let encoding = {
                    let (lower, upper) = repetition.bounds();
                    let mut pattern = String::new();
                    pattern.push_str("(?:");
                    encode(
                        Grouping::NonCapture,
                        superposition.or(Some(position)),
                        &mut pattern,
                        repetition.tokens().iter(),
                    );
                    pattern.push_str(&if let Some(upper) = upper {
                        format!("){{{},{}}}", lower, upper)
                    } else {
                        format!("){{{},}}", lower)
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
                    // TODO: The compiled `Regex` is discarded. Is there a way to check the
                    //       correctness of the expression but do less work (i.e., don't build a
                    //       complete `Regex`)?
                    // Compile the character class sub-expression. This may fail if the subtraction
                    // of the separator pattern yields an empty character class (meaning that the
                    // glob expression matches only separator characters on the target platform).
                    if Regex::new(&pattern).is_ok() {
                        pattern.into()
                    } else {
                        // If compilation fails, then use `NEVER_EXPRESSION`, which matches
                        // nothing.
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
                    pattern.push_str(sepexpr!("(?:{0}?|"));
                    grouping.push_str(pattern, sepexpr!(".*{0}"));
                    pattern.push(')');
                }
            }
            (Middle, Wildcard(Tree { .. })) => {
                encode_intermediate_tree(grouping, pattern);
            }
            (Last, Wildcard(Tree { .. })) => {
                if let Some(First | Middle) = superposition {
                    encode_intermediate_tree(grouping, pattern);
                } else {
                    pattern.push_str(sepexpr!("(?:{0}?|{0}"));
                    grouping.push_str(pattern, ".*");
                    pattern.push(')');
                }
            }
            (Only, Wildcard(Tree { .. })) => grouping.push_str(pattern, ".*"),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::encode;

    #[test]
    fn case_folded_eq() {
        assert!(encode::case_folded_eq("a", "a"));
        assert!(encode::case_folded_eq("a", "A"));

        assert!(!encode::case_folded_eq("a", "b"));
        assert!(!encode::case_folded_eq("aa", "a"));
        assert!(!encode::case_folded_eq("a", "aa"));
    }
}
