//! Call argument helpers: literal decoding, uncertainty details, and
//! [`evaluate`], a context-free entry point to the partial evaluator
//! ([`super::evaluator`]).
//!
//! The recognizer evaluates arguments with the file's constants and bindings in
//! scope; [`evaluate`] runs the same evaluator with an empty scope (no
//! constants, no `path` binding), so it understands literals, concatenation,
//! templates, ternaries, `__dirname`, `import.meta.url`, `new URL(x,
//! import.meta.url)` and `process.cwd()`.

use std::{borrow::Cow, collections::HashMap};

use turborepo_trace_core::{
    Span, Specifier, Uncertainty, UncertaintyKind,
    lexkit::{EscapeRules, decode},
};

use super::{
    api::BaseRule,
    evaluator::{self, JsVal, Scope},
};
use crate::lexer::Token;

/// Maximum number of argument tokens captured. Longer arguments are reported as
/// computed with their text truncated.
pub const MAX_ARG_TOKENS: usize = 256;

/// Maximum number of source bytes recorded in a computed-specifier uncertainty.
const MAX_DETAIL: usize = 200;

/// The value of an argument.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Evaluated<'s> {
    /// What the argument names.
    pub spec: Specifier<'s>,
    /// Why it is not fully known, if it is not.
    pub uncertainty: Option<Uncertainty>,
}

/// Decodes a string literal or substitution-free template token. `None` for any
/// other token.
pub fn literal<'s>(src: &'s [u8], t: &Token) -> Option<Result<Cow<'s, str>, Uncertainty>> {
    let raw = t.literal_content(src)?;
    Some(decode(raw, EscapeRules::JS).map_err(|e| {
        Uncertainty::new(
            UncertaintyKind::ParseIncomplete,
            t.span(),
            format!("undecodable string literal: {e}"),
        )
    }))
}

/// The span and source text (capped, `…` when truncated) of argument tokens,
/// for uncertainty records.
pub fn arg_detail(src: &[u8], toks: &[Token], truncated: bool) -> (Span, String) {
    let (Some(first), Some(last)) = (toks.first(), toks.last()) else {
        return (Span::default(), String::new());
    };
    let span = Span::new(first.start, last.end.max(first.start));
    let text = span.slice(src).unwrap_or_default();
    let mut detail =
        String::from_utf8_lossy(&text[..floor_boundary(text, MAX_DETAIL)]).into_owned();
    if truncated || text.len() > MAX_DETAIL {
        detail.push('…');
    }
    (span, detail)
}

/// Evaluates the tokens of one module-specifier argument (as for `require` /
/// `import()`) without file context. `truncated` says the capture stopped at
/// [`MAX_ARG_TOKENS`] or at the end of input. Alternatives with different
/// anchors are not representable here and become unknown.
pub fn evaluate<'s>(src: &'s [u8], toks: &[Token], truncated: bool) -> Evaluated<'s> {
    let (span, detail) = arg_detail(src, toks, truncated);
    if toks.is_empty() {
        return Evaluated {
            spec: Specifier::Unknown,
            uncertainty: None,
        };
    }
    let computed = |kind| Evaluated {
        spec: Specifier::Unknown,
        uncertainty: Some(Uncertainty::new(kind, span, detail.clone())),
    };
    if truncated {
        return computed(UncertaintyKind::ComputedSpecifier);
    }
    let consts = HashMap::new();
    let sc = Scope {
        src,
        consts: &consts,
        apis: &[],
        loaders: &[],
        file_name: None,
    };
    let out = evaluator::evaluate(&sc, toks);
    if let JsVal::Str(turborepo_trace_core::eval::Value::Str(_)) = &out.val
        && let [t] = toks
        && let Some(lit) = literal(src, t)
    {
        // Keep plain literals borrowed.
        return match lit {
            Ok(s) => Evaluated {
                spec: Specifier::Literal(s),
                uncertainty: None,
            },
            Err(u) => Evaluated {
                spec: Specifier::Unknown,
                uncertainty: Some(u),
            },
        };
    }
    let env = out.env;
    let lowered = evaluator::lower(out.val, BaseRule::Module);
    let same_base = lowered.leaves.windows(2).all(|w| w[0].base == w[1].base);
    if lowered.leaves.is_empty() || !same_base {
        return computed(if env {
            UncertaintyKind::EnvDependent
        } else {
            UncertaintyKind::ComputedSpecifier
        });
    }
    let value =
        turborepo_trace_core::eval::Value::alternatives(lowered.leaves.into_iter().map(|l| l.spec));
    let mut reasons = lowered.dropped;
    evaluator::wild_reasons(&value, &mut reasons);
    let uncertainty = if env {
        Some(Uncertainty::new(
            UncertaintyKind::EnvDependent,
            span,
            detail,
        ))
    } else if !reasons.is_empty() {
        Some(Uncertainty::new(
            UncertaintyKind::PartialSpecifier,
            span,
            reasons.join(", "),
        ))
    } else {
        None
    };
    Evaluated {
        spec: value.to_specifier(),
        uncertainty,
    }
}

/// The largest `n <= max` that does not split a UTF-8 sequence in `s`.
fn floor_boundary(s: &[u8], max: usize) -> usize {
    if s.len() <= max {
        return s.len();
    }
    let mut n = max;
    while n > 0 && (s[n] & 0xC0) == 0x80 {
        n -= 1;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::{Dialect, Lexer, TokKind};

    fn ev(src: &str) -> Evaluated<'_> {
        let mut lx = Lexer::new(src.as_bytes(), Dialect::JS);
        let mut toks = Vec::new();
        loop {
            let t = lx.next_token();
            if t.kind == TokKind::Eof {
                break;
            }
            toks.push(t);
        }
        evaluate(src.as_bytes(), &toks, false)
    }

    #[test]
    fn context_free_evaluation() {
        let e = ev("'./a'");
        assert!(matches!(e.spec, Specifier::Literal(Cow::Borrowed("./a"))));
        assert_eq!(ev("'u' + 'rl'").spec, Specifier::literal("url"));
        let e = ev("'./p/' + x");
        assert_eq!(e.spec.to_string(), "./p/*");
        assert_eq!(
            e.uncertainty.unwrap().kind,
            UncertaintyKind::PartialSpecifier
        );
        let e = ev("x");
        assert_eq!(e.spec, Specifier::Unknown);
        assert_eq!(e.uncertainty.unwrap().detail.as_ref(), "x");
        assert_eq!(ev("c ? 'a' : 'b'").spec.to_string(), "a | b");
    }
}
