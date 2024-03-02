#![cfg(feature = "miette")]

use std::borrow::Cow;

use miette::{Diagnostic, SourceSpan};
use tardar::{
    BoxedDiagnostic, DiagnosticResult, DiagnosticResultExt as _, IteratorExt as _, ResultExt as _,
};
use thiserror::Error;

use crate::{
    diagnostics::SpanExt as _,
    rule,
    token::{self, TokenKind, TokenTree, Tokenized},
    Checked, Glob,
};

/// APIs for diagnosing globs.
impl<'t> Glob<'t> {
    /// Constructs a [`Glob`] from a glob expression with diagnostics.
    ///
    /// This function is the same as [`Glob::new`], but additionally returns
    /// detailed diagnostics on both success and failure.
    ///
    /// See [`Glob::diagnose`].
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tardar::DiagnosticResultExt as _;
    /// use wax::Glob;
    ///
    /// let result = Glob::diagnosed("(?i)readme.{md,mkd,markdown}");
    /// for diagnostic in result.diagnostics() {
    ///     eprintln!("{}", diagnostic);
    /// }
    /// if let Some(glob) = result.ok_output() { /* ... */ }
    /// ```
    ///
    /// [`Glob`]: crate::Glob
    /// [`Glob::diagnose`]: crate::Glob::diagnose
    /// [`Glob::new`]: crate::Glob::new
    pub fn diagnosed(expression: &'t str) -> DiagnosticResult<'t, Self> {
        parse_and_diagnose(expression).and_then_diagnose(|tree| {
            Glob::compile(tree.as_ref().tokens())
                .into_error_diagnostic()
                .map_output(|program| Glob { tree, program })
        })
    }

    /// Gets **non-error** [`Diagnostic`]s.
    ///
    /// This function requires a receiving [`Glob`] and so does not report
    /// error-level [`Diagnostic`]s. It can be used to get non-error
    /// diagnostics after constructing or [partitioning][`Glob::partition`]
    /// a [`Glob`].
    ///
    /// See [`Glob::diagnosed`].
    ///
    /// [`Diagnostic`]: miette::Diagnostic
    /// [`Glob`]: crate::Glob
    /// [`Glob::diagnosed`]: crate::Glob::diagnosed
    /// [`Glob::partition`]: crate::Glob::partition
    pub fn diagnose(&self) -> impl Iterator<Item = Box<dyn Diagnostic + '_>> {
        diagnose(self.tree.as_ref())
    }
}

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

fn parse_and_diagnose(expression: &str) -> DiagnosticResult<Checked<Tokenized>> {
    token::parse(expression)
        .into_error_diagnostic()
        .and_then_diagnose(|tokenized| rule::check(tokenized).into_error_diagnostic())
        .and_then_diagnose(|checked| {
            // TODO: This should accept `&Checked`.
            diagnose(checked.as_ref())
                .into_non_error_diagnostic()
                .map_output(|_| checked)
        })
}

fn diagnose<'i, 't>(
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
        .chain(
            tokenized
                .tokens()
                .last()
                .into_iter()
                .filter(|token| matches!(token.kind(), TokenKind::Separator(_)))
                .map(|token| {
                    Box::new(TerminatingSeparatorWarning {
                        expression: tokenized.expression().clone(),
                        span: (*token.annotation()).into(),
                    }) as BoxedDiagnostic
                }),
        )
}

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
