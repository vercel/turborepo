#![cfg(feature = "miette")]

use std::borrow::Cow;

use miette::{Diagnostic, SourceSpan};
use tardar::BoxedDiagnostic;
use thiserror::Error;

use crate::{
    diagnostics::SpanExt as _,
    token::{self, TokenKind, TokenTree, Tokenized},
};

#[derive(Clone, Debug, Diagnostic, Error)]
#[diagnostic(code(wax::glob::semantic_literal), severity(warning))]
#[error("`{literal}` has been interpreted as a literal with no semantics")]
pub struct SemanticLiteralWarning<'t> {
    #[source_code]
    expression: Cow<'t, str>,
    literal: Cow<'t, str>,
    #[label("here")]
    span: SourceSpan,
}

#[derive(Clone, Debug, Diagnostic, Error)]
#[diagnostic(code(wax::glob::terminating_separator), severity(warning))]
#[error("terminating separator may discard matches")]
pub struct TerminatingSeparatorWarning<'t> {
    #[source_code]
    expression: Cow<'t, str>,
    #[label("here")]
    span: SourceSpan,
}

pub fn diagnose<'i, 't>(
    tokenized: &'i Tokenized<'t>,
) -> impl 'i + Iterator<Item = BoxedDiagnostic<'t>> {
    None.into_iter()
        .chain(
            token::literals(tokenized.tokens())
                .filter(|(_, literal)| literal.is_semantic_literal())
                .map(|(component, literal)| {
                    Box::new(SemanticLiteralWarning {
                        expression: tokenized.expression().clone(),
                        literal: literal.text().clone(),
                        span: component
                            .tokens()
                            .iter()
                            .map(|token| *token.annotation())
                            .reduce(|left, right| left.union(&right))
                            .map(SourceSpan::from)
                            .expect("no tokens in component"),
                    }) as BoxedDiagnostic
                }),
        )
        .chain(tokenized.tokens().last().into_iter().filter_map(|token| {
            matches!(token.kind(), TokenKind::Separator(_)).then(|| {
                Box::new(TerminatingSeparatorWarning {
                    expression: tokenized.expression().clone(),
                    span: (*token.annotation()).into(),
                }) as BoxedDiagnostic
            })
        }))
}

// These tests exercise `Glob` APIs, which wrap functions in this module.
#[cfg(test)]
mod tests {
    use crate::Glob;

    // It is non-trivial to downcast `&dyn Diagnostic`, so diagnostics are
    // identified in tests by their code.
    const CODE_SEMANTIC_LITERAL: &str = "wax::glob::semantic_literal";
    const CODE_TERMINATING_SEPARATOR: &str = "wax::glob::terminating_separator";

    #[cfg(any(unix, windows))]
    #[test]
    fn diagnose_glob_semantic_literal_warning() {
        let glob = Glob::new("../foo").unwrap();
        let diagnostics: Vec<_> = glob.diagnose().collect();

        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .code()
            .map_or(false, |code| code.to_string() == CODE_SEMANTIC_LITERAL)));
    }

    #[test]
    fn diagnose_glob_terminating_separator_warning() {
        let glob = Glob::new("**/foo/").unwrap();
        let diagnostics: Vec<_> = glob.diagnose().collect();

        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .code()
            .map_or(false, |code| code.to_string() == CODE_TERMINATING_SEPARATOR)));
    }
}
