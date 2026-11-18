//! Scans test (and setup) files for runner calls that create dependencies no
//! import expresses: `jest.mock('x')`, `jest.requireActual('x')`,
//! `vi.mock('x')`, `vi.importActual('x')`, ..., and per-file environment
//! pragmas in the leading comments (`@jest-environment jsdom`,
//! `@vitest-environment happy-dom`).

use std::borrow::Cow;

use turborepo_trace_core::lexkit::{EscapeRules, decode};
use turborepo_trace_lang_js::{
    Dialect,
    lexer::{Lexer, TokKind, Token},
};

/// What a runner call does with its module argument.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CallKind {
    /// Replaces the module (`jest.mock`, `jest.doMock`, `vi.mock`, `vi.doMock`,
    /// `jest.unstable_mockModule`, `jest.setMock`): a manual mock is used
    /// unless a factory is given.
    Mock,
    /// Returns the mock (`jest.requireMock`, `vi.importMock`): manual mock
    /// lookup.
    RequireMock,
    /// Loads the real module (`jest.requireActual`, `vi.importActual`,
    /// `jest.createMockFromModule`).
    RequireActual,
    /// `jest.enableAutomock()`.
    EnableAutomock,
}

/// A runner call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunnerCall {
    /// `jest.mock`, `vi.importActual`, ...
    pub call: String,
    /// What it does.
    pub kind: CallKind,
    /// The module request, if it is a literal.
    pub request: Option<String>,
    /// A second argument was given (factory or options).
    pub factory: bool,
    /// 1-based line.
    pub line: u32,
}

/// The result of [`scan`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scanned {
    /// Runner calls, in source order.
    pub calls: Vec<RunnerCall>,
    /// The environment pragma, if any.
    pub environment: Option<String>,
}

fn classify(object: &str, method: &str) -> Option<CallKind> {
    Some(match (object, method) {
        ("jest", "mock" | "doMock" | "setMock" | "unstable_mockModule") => CallKind::Mock,
        ("jest", "requireMock") => CallKind::RequireMock,
        ("jest", "requireActual" | "createMockFromModule" | "genMockFromModule") => {
            CallKind::RequireActual
        }
        ("jest", "enableAutomock") => CallKind::EnableAutomock,
        ("vi", "mock" | "doMock") => CallKind::Mock,
        ("vi", "importMock") => CallKind::RequireMock,
        ("vi", "importActual") => CallKind::RequireActual,
        _ => return None,
    })
}

/// Scans `src` for calls on `objects` (`["jest"]`, `["vi"]`) and the
/// environment pragma `@<pragma>` (`jest-environment`, `vitest-environment`).
pub fn scan(src: &[u8], dialect: Dialect, objects: &[&str], pragma: &str) -> Scanned {
    let mut out = Scanned {
        environment: environment_pragma(src, pragma),
        ..Scanned::default()
    };
    // Cheap prefilter.
    if !objects
        .iter()
        .any(|o| memfind(src, format!("{o}.").as_bytes()))
    {
        return out;
    }
    let mut lx = Lexer::new(src, dialect);
    let mut toks: Vec<Token> = Vec::new();
    loop {
        let t = lx.next_token();
        if t.kind == TokKind::Eof || toks.len() > 2_000_000 {
            break;
        }
        toks.push(t);
    }
    let text = |t: &Token| t.text(src);
    let mut line = 1u32;
    let mut line_at = 0usize;
    for i in 0..toks.len() {
        let t = toks[i];
        if t.kind != TokKind::Ident {
            continue;
        }
        let Some(obj) = objects.iter().find(|o| text(&t) == o.as_bytes()) else {
            continue;
        };
        if i > 0 && matches!(toks[i - 1].kind, TokKind::Dot | TokKind::OptChain) {
            continue;
        }
        let (Some(d), Some(m), Some(lp)) = (toks.get(i + 1), toks.get(i + 2), toks.get(i + 3))
        else {
            continue;
        };
        if d.kind != TokKind::Dot || m.kind != TokKind::Ident || lp.kind != TokKind::LParen {
            continue;
        }
        let method = String::from_utf8_lossy(text(m)).into_owned();
        let Some(kind) = classify(obj, &method) else {
            continue;
        };
        let start = t.start as usize;
        line += src[line_at..start].iter().filter(|b| **b == b'\n').count() as u32;
        line_at = start;
        // The first argument: a literal, or `import('x')` (vitest).
        let mut j = i + 4;
        let request = match toks.get(j) {
            Some(a) if matches!(a.kind, TokKind::Str | TokKind::Template) => {
                j += 1;
                a.literal_content(src)
                    .and_then(|raw| decode(raw, EscapeRules::JS).ok())
                    .map(Cow::into_owned)
            }
            Some(a)
                if text(a) == b"import"
                    && toks.get(j + 1).is_some_and(|x| x.kind == TokKind::LParen)
                    && toks
                        .get(j + 2)
                        .is_some_and(|x| matches!(x.kind, TokKind::Str | TokKind::Template))
                    && toks.get(j + 3).is_some_and(|x| x.kind == TokKind::RParen) =>
            {
                let lit = toks[j + 2];
                j += 4;
                lit.literal_content(src)
                    .and_then(|raw| decode(raw, EscapeRules::JS).ok())
                    .map(Cow::into_owned)
            }
            _ => None,
        };
        let factory = request.is_some()
            && toks.get(j).is_some_and(|c| c.kind == TokKind::Comma)
            && toks.get(j + 1).is_some_and(|n| n.kind != TokKind::RParen);
        if kind == CallKind::EnableAutomock || request.is_some() {
            out.calls.push(RunnerCall {
                call: format!("{obj}.{method}"),
                kind,
                request,
                factory,
                line,
            });
        } else if !(kind == CallKind::Mock
            && toks.get(i + 4).is_some_and(|a| a.kind == TokKind::RParen))
        {
            out.calls.push(RunnerCall {
                call: format!("{obj}.{method}"),
                kind,
                request: None,
                factory,
                line,
            });
        }
    }
    out
}

fn memfind(h: &[u8], n: &[u8]) -> bool {
    h.windows(n.len()).any(|w| w == n)
}

/// The value of `@<pragma> value` in the leading comments of `src`.
pub fn environment_pragma(src: &[u8], pragma: &str) -> Option<String> {
    let mut i = 0;
    if src.starts_with(b"#!") {
        i = src.iter().position(|b| *b == b'\n').unwrap_or(src.len());
    }
    let needle = format!("@{pragma}");
    loop {
        while i < src.len() && src[i].is_ascii_whitespace() {
            i += 1;
        }
        let (body, next) = if src[i..].starts_with(b"/*") {
            let end = src[i + 2..]
                .windows(2)
                .position(|w| w == b"*/")
                .map_or(src.len(), |p| i + 2 + p + 2);
            (&src[i..end.min(src.len())], end)
        } else if src[i..].starts_with(b"//") {
            let end = src[i..]
                .iter()
                .position(|b| *b == b'\n')
                .map_or(src.len(), |p| i + p);
            (&src[i..end], end)
        } else {
            return None;
        };
        let s = String::from_utf8_lossy(body);
        if let Some(p) = s.find(&needle) {
            let rest = &s[p + needle.len()..];
            if rest.starts_with(|c: char| c.is_whitespace()) {
                let v: String = rest
                    .trim_start()
                    .chars()
                    .take_while(|c| !c.is_whitespace() && *c != '*')
                    .collect();
                if !v.is_empty() {
                    return Some(v);
                }
            }
        }
        if next <= i {
            return None;
        }
        i = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jest_calls() {
        let src = br#"/**
 * @jest-environment jsdom
 */
import { jest } from '@jest/globals';
jest.mock('fs');
jest.mock('../lib/db', () => ({ query: jest.fn() }));
jest.mock(`./local`);
const real = jest.requireActual('../lib/db');
const m = jest.requireMock("axios");
jest.mock(name);
foo.jest.mock('nope');
jest.unstable_mockModule('esm-thing', () => ({}));
"#;
        let s = scan(src, Dialect::TS, &["jest"], "jest-environment");
        assert_eq!(s.environment.as_deref(), Some("jsdom"));
        let got: Vec<(String, Option<String>, bool, u32)> = s
            .calls
            .iter()
            .map(|c| (c.call.clone(), c.request.clone(), c.factory, c.line))
            .collect();
        assert_eq!(
            got,
            [
                ("jest.mock".into(), Some("fs".into()), false, 5),
                ("jest.mock".into(), Some("../lib/db".into()), true, 6),
                ("jest.mock".into(), Some("./local".into()), false, 7),
                (
                    "jest.requireActual".into(),
                    Some("../lib/db".into()),
                    false,
                    8
                ),
                ("jest.requireMock".into(), Some("axios".into()), false, 9),
                ("jest.mock".into(), None, false, 10),
                (
                    "jest.unstable_mockModule".into(),
                    Some("esm-thing".into()),
                    true,
                    12
                ),
            ]
        );
    }

    #[test]
    fn vi_calls_and_pragma() {
        let src = b"// @vitest-environment happy-dom\nvi.mock(import('./a'));\nvi.mock('b', { spy: true });\nawait vi.importActual('c');\n";
        let s = scan(src, Dialect::TS, &["vi"], "vitest-environment");
        assert_eq!(s.environment.as_deref(), Some("happy-dom"));
        assert_eq!(s.calls.len(), 3);
        assert_eq!(s.calls[0].request.as_deref(), Some("./a"));
        assert!(!s.calls[0].factory);
        assert!(s.calls[1].factory);
        assert_eq!(s.calls[2].kind, CallKind::RequireActual);
        assert_eq!(
            environment_pragma(b"const a = 1; // @jest-environment x", "jest-environment"),
            None
        );
    }
}
