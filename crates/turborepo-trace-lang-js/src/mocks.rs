//! Module mocks of the test runners (jest, vitest): the table of their APIs and
//! what a file says with them, as a per-file fact.
//!
//! A test that replaces a module with a mock (`jest.mock('./db', () => fake)`)
//! never runs the module's code, so a change that reaches the test only through
//! that module cannot change its outcome (`turborepo_trace_graph::mocks`).
//! Whether a replacement holds depends on what the test, its setup files and
//! the helpers it loads do with the same APIs: one `jest.requireActual('./db')`
//! anywhere and the real module runs after all. [`scan`] finds every use of the
//! APIs in [`MOCK_APIS`] in a file; [`MockScan::encode`] is the fact
//! (`mock:{...}`) the extractor records for first-party files, so that the
//! graph layer reads it without the source.
//!
//! # Matching
//!
//! A call that *removes* a dependency (a replacement) is matched strictly: the
//! runner's object by its usual name (`jest`, `vi`, `vitest`), the method by
//! name, a literal module request, and for the hoisted calls a statement at the
//! top level of the file. Anything that *adds* one (loading the real module,
//! undoing a mock) is matched loosely: the distinctive method names count on
//! any receiver and in any position ([`MockScan::loose`]), because a false
//! match there only keeps a dependency.

use std::borrow::Cow;

use turborepo_trace_core::lexkit::{EscapeRules, decode};

use crate::lexer::{Dialect, Kw, Lexer, NEWLINE_BEFORE, TokKind, Token};

/// What a runner API does to the module registry of a test file.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum MockRole {
    /// Replaces the module for every load of the test file. The runner's
    /// transform hoists the call above the file's imports (`jest.mock`,
    /// `vi.mock`).
    Replace,
    /// Replaces the module from the call on; not hoisted (`jest.doMock`,
    /// `jest.setMock`, `jest.unstable_mockModule`, `vi.doMock`).
    ReplaceLate,
    /// Loads the real module whatever is registered (`jest.requireActual`,
    /// `vi.importActual`, `jest.createMockFromModule`).
    Actual,
    /// Returns the registered mock, or an automock of the real module
    /// (`jest.requireMock`, `vi.importMock`).
    MockOf,
    /// Undoes a replacement (`jest.unmock`, `jest.dontMock`, `vi.unmock`,
    /// `vi.doUnmock`).
    Unmock,
    /// Switches automocking of every module for the file
    /// (`jest.enableAutomock`).
    Automock,
    /// Clears the module registry. Registered replacements stay
    /// (`jest.resetModules`, `jest.isolateModules`, `vi.resetModules`).
    Registry,
}

/// One API of a runner.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct MockApi {
    /// The runner (`jest`, `vitest`): the framework id.
    pub runner: &'static str,
    /// The object the method is called on, as usually written.
    pub object: &'static str,
    /// The method.
    pub method: &'static str,
    /// What it does.
    pub role: MockRole,
}

const fn api(
    runner: &'static str,
    object: &'static str,
    method: &'static str,
    role: MockRole,
) -> MockApi {
    MockApi {
        runner,
        object,
        method,
        role,
    }
}

/// The mock APIs of jest and vitest. `vitest` is the second name vitest exports
/// for `vi`.
pub const MOCK_APIS: &[MockApi] = &[
    api("jest", "jest", "mock", MockRole::Replace),
    api("jest", "jest", "doMock", MockRole::ReplaceLate),
    api("jest", "jest", "setMock", MockRole::ReplaceLate),
    api("jest", "jest", "unstable_mockModule", MockRole::ReplaceLate),
    api("jest", "jest", "requireActual", MockRole::Actual),
    api("jest", "jest", "createMockFromModule", MockRole::Actual),
    api("jest", "jest", "genMockFromModule", MockRole::Actual),
    api("jest", "jest", "requireMock", MockRole::MockOf),
    api("jest", "jest", "unmock", MockRole::Unmock),
    api("jest", "jest", "dontMock", MockRole::Unmock),
    api("jest", "jest", "deepUnmock", MockRole::Unmock),
    api("jest", "jest", "unstable_unmockModule", MockRole::Unmock),
    api("jest", "jest", "enableAutomock", MockRole::Automock),
    api("jest", "jest", "disableAutomock", MockRole::Automock),
    api("jest", "jest", "resetModules", MockRole::Registry),
    api("jest", "jest", "isolateModules", MockRole::Registry),
    api("jest", "jest", "isolateModulesAsync", MockRole::Registry),
    api("vitest", "vi", "mock", MockRole::Replace),
    api("vitest", "vi", "doMock", MockRole::ReplaceLate),
    api("vitest", "vi", "importActual", MockRole::Actual),
    api("vitest", "vi", "importMock", MockRole::MockOf),
    api("vitest", "vi", "unmock", MockRole::Unmock),
    api("vitest", "vi", "doUnmock", MockRole::Unmock),
    api("vitest", "vi", "resetModules", MockRole::Registry),
    api("vitest", "vitest", "mock", MockRole::Replace),
    api("vitest", "vitest", "doMock", MockRole::ReplaceLate),
    api("vitest", "vitest", "importActual", MockRole::Actual),
    api("vitest", "vitest", "importMock", MockRole::MockOf),
    api("vitest", "vitest", "unmock", MockRole::Unmock),
    api("vitest", "vitest", "doUnmock", MockRole::Unmock),
    api("vitest", "vitest", "resetModules", MockRole::Registry),
];

/// Method names that are not ordinary words: used on any receiver, as a value
/// or in a way [`scan`] cannot read as a call of [`MOCK_APIS`], they still
/// count ([`MockScan::loose`]). `mock` and `doMock` count on an unknown
/// receiver only when called with a literal request.
fn distinctive(name: &[u8]) -> bool {
    MOCK_APIS.iter().any(|a| {
        a.method.as_bytes() == name
            && !matches!(a.role, MockRole::Registry)
            && !matches!(a.method, "mock" | "doMock")
    })
}

/// The second argument of a replacing call.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum MockArg {
    /// None: an automock or a manual mock.
    Bare,
    /// An inline function without parameters: the factory's value is the
    /// module.
    Factory,
    /// An inline function with a parameter (vitest hands it `importOriginal`).
    FactoryWithParam,
    /// An object literal (vitest options, `{ spy: true }`).
    Options,
    /// Anything else (a factory by reference, a spread).
    Other,
}

impl MockArg {
    fn code(self) -> &'static str {
        match self {
            MockArg::Bare => "b",
            MockArg::Factory => "f",
            MockArg::FactoryWithParam => "p",
            MockArg::Options => "o",
            MockArg::Other => "x",
        }
    }

    fn of_code(s: &str) -> Option<MockArg> {
        Some(match s {
            "b" => MockArg::Bare,
            "f" => MockArg::Factory,
            "p" => MockArg::FactoryWithParam,
            "o" => MockArg::Options,
            "x" => MockArg::Other,
            _ => return None,
        })
    }
}

/// One call of an API of [`MOCK_APIS`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MockCall {
    /// The API.
    pub api: &'static MockApi,
    /// The module request, when it is a literal (`'./db'`, `import('./db')`).
    pub request: Option<String>,
    /// 1-based line.
    pub line: u32,
    /// The call is a statement at the top level of the file.
    pub top_level: bool,
    /// What follows the request.
    pub arg: MockArg,
    /// The literal request of the replacing call (`jest.mock`, `doMock`, ...)
    /// in whose arguments this call stands, the innermost one: the call
    /// runs when that mock's factory does, which is when its module is
    /// requested.
    pub within: Option<String>,
}

impl MockCall {
    /// `jest.mock`, `vi.importActual`, ...
    pub fn name(&self) -> String {
        format!("{}.{}", self.api.object, self.api.method)
    }
}

/// A distinctive method name used in a way that is not a call [`scan`] can
/// read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LooseUse {
    /// The name (`requireActual`, `unmock`; `mock` for a call on an unknown
    /// receiver).
    pub name: String,
    /// The literal request it is called with, if any.
    pub request: Option<String>,
    /// 1-based line.
    pub line: u32,
}

/// What a file does with the mock APIs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MockScan {
    /// Calls, in source order. Calls of [`MockRole::Registry`] are left out.
    pub calls: Vec<MockCall>,
    /// Other uses of the distinctive names.
    pub loose: Vec<LooseUse>,
}

/// The tag of the fact ([`MockScan::encode`]).
pub const MOCK_FACT_TAG: &str = "mock:";

impl MockScan {
    /// Whether the file uses none of the APIs.
    pub fn is_empty(&self) -> bool {
        self.calls.is_empty() && self.loose.is_empty()
    }

    /// The scan as a tagged fact string (`mock:{"calls":[...],"loose":[...]}`).
    pub fn encode(&self) -> String {
        let calls: Vec<serde_json::Value> = self
            .calls
            .iter()
            .map(|c| {
                serde_json::json!([
                    c.api.object,
                    c.api.method,
                    c.request,
                    c.line,
                    c.top_level,
                    c.arg.code(),
                    c.within
                ])
            })
            .collect();
        let loose: Vec<serde_json::Value> = self
            .loose
            .iter()
            .map(|l| serde_json::json!([l.name, l.request, l.line]))
            .collect();
        format!(
            "{MOCK_FACT_TAG}{}",
            serde_json::json!({ "calls": calls, "loose": loose })
        )
    }

    /// Parses a fact string. `None` for facts of other kinds and for malformed
    /// input (also a call of an API this table does not have).
    pub fn decode(s: &str) -> Option<MockScan> {
        let v: serde_json::Value = serde_json::from_str(s.strip_prefix(MOCK_FACT_TAG)?).ok()?;
        let mut out = MockScan::default();
        for c in v.get("calls")?.as_array()? {
            let c = c.as_array()?;
            let (object, method) = (c.first()?.as_str()?, c.get(1)?.as_str()?);
            out.calls.push(MockCall {
                api: MOCK_APIS
                    .iter()
                    .find(|a| a.object == object && a.method == method)?,
                request: c.get(2)?.as_str().map(str::to_owned),
                line: c.get(3)?.as_u64()? as u32,
                top_level: c.get(4)?.as_bool()?,
                arg: MockArg::of_code(c.get(5)?.as_str()?)?,
                within: c.get(6).and_then(|w| w.as_str()).map(str::to_owned),
            });
        }
        for l in v.get("loose")?.as_array()? {
            let l = l.as_array()?;
            out.loose.push(LooseUse {
                name: l.first()?.as_str()?.to_owned(),
                request: l.get(1)?.as_str().map(str::to_owned),
                line: l.get(2)?.as_u64()? as u32,
            });
        }
        Some(out)
    }
}

/// Whether `src` can use a mock API: a cheap test that never misses a file
/// [`scan`] reports something for. [`scan`] only reports identifiers that are a
/// method name of [`MOCK_APIS`] (other than the calls that clear the registry),
/// and every such name contains one of three words (a test below checks it):
/// the file passes when one of their occurrences is such a name as a whole
/// word. `mockReturnValue`, `mockFn` or `Actually` in a comment are
/// not, and most test files have nothing else.
pub fn may_mock(src: &[u8]) -> bool {
    // A byte of an identifier (any non-ASCII byte counts: the word is then no API
    // name).
    let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 0x80;
    [&b"mock"[..], b"Mock", b"Actual"].iter().any(|needle| {
        memchr::memmem::find_iter(src, needle).any(|at| {
            let mut start = at;
            while start > 0 && ident(src[start - 1]) {
                start -= 1;
            }
            let mut end = at + needle.len();
            while end < src.len() && ident(src[end]) {
                end += 1;
            }
            let word = &src[start..end];
            MOCK_APIS
                .iter()
                .any(|a| a.role != MockRole::Registry && a.method.as_bytes() == word)
        })
    })
}

/// The literal request at `toks[j]`: a string, a template without
/// substitutions, or `import('x')`. Returns it with the index after it.
fn literal_request(src: &[u8], toks: &[Token], j: usize) -> Option<(String, usize)> {
    let lit = |t: &Token| {
        t.literal_content(src)
            .and_then(|raw| decode(raw, EscapeRules::JS).ok())
            .map(Cow::into_owned)
    };
    let a = toks.get(j)?;
    if matches!(a.kind, TokKind::Str | TokKind::Template) {
        return Some((lit(a)?, j + 1));
    }
    if a.text(src) == b"import"
        && toks.get(j + 1)?.kind == TokKind::LParen
        && matches!(toks.get(j + 2)?.kind, TokKind::Str | TokKind::Template)
        && toks.get(j + 3)?.kind == TokKind::RParen
    {
        return Some((lit(&toks[j + 2])?, j + 4));
    }
    None
}

/// The index of the `(` that opens the arguments of a call whose callee ends at
/// `toks[j - 1]`: `toks[j]` itself, or the token after TypeScript type
/// arguments (`requireActual<typeof import('./a')>('./a')`).
fn call_paren(toks: &[Token], j: usize) -> Option<usize> {
    let t = toks.get(j)?;
    if t.kind == TokKind::LParen {
        return Some(j);
    }
    if t.kind != TokKind::Lt {
        return None;
    }
    let mut angle = 1usize;
    let mut k = j + 1;
    // Type arguments are short; a comparison chain that happens to follow the name
    // is not followed far.
    while k < toks.len() && k < j + 200 {
        match toks[k].kind {
            TokKind::Lt => angle += 1,
            TokKind::Gt => {
                angle -= 1;
                if angle == 0 {
                    return (toks.get(k + 1)?.kind == TokKind::LParen).then_some(k + 1);
                }
            }
            // `>>` and other operators, statements: not type arguments this reads.
            TokKind::Op | TokKind::Semi | TokKind::Assign | TokKind::LBrace | TokKind::RBrace => {
                return None;
            }
            _ => {}
        }
        k += 1;
    }
    None
}

/// What follows the request of a replacing call: `toks[j]` is the token after
/// the request.
fn second_arg(src: &[u8], toks: &[Token], j: usize) -> MockArg {
    let Some(c) = toks.get(j) else {
        return MockArg::Other;
    };
    if c.kind == TokKind::RParen {
        return MockArg::Bare;
    }
    if c.kind != TokKind::Comma {
        return MockArg::Other;
    }
    let mut k = j + 1;
    let Some(mut t) = toks.get(k) else {
        return MockArg::Other;
    };
    if t.kind == TokKind::RParen {
        return MockArg::Bare;
    }
    if t.kind == TokKind::LBrace {
        return MockArg::Options;
    }
    if t.kind == TokKind::Ident && t.text(src) == b"async" {
        k += 1;
        match toks.get(k) {
            Some(n) => t = n,
            None => return MockArg::Other,
        }
    }
    let params = |open: usize| -> Option<MockArg> {
        let close = toks.get(open + 1)?;
        if close.kind == TokKind::RParen {
            Some(MockArg::Factory)
        } else {
            Some(MockArg::FactoryWithParam)
        }
    };
    if t.kind == TokKind::LParen {
        // `() =>` or `(a) =>`: an arrow function when the matching `)` is followed by
        // `=>`.
        let depth = t.depth;
        let mut c = k + 1;
        while c < toks.len() && !(toks[c].kind == TokKind::RParen && toks[c].depth == depth) {
            c += 1;
        }
        if toks.get(c + 1).is_some_and(|a| a.kind == TokKind::Arrow) {
            return params(k).unwrap_or(MockArg::Other);
        }
        return MockArg::Other;
    }
    if t.kind == TokKind::Ident && t.text(src) == b"function" {
        let mut open = k + 1;
        if toks.get(open).is_some_and(|n| n.kind == TokKind::Star) {
            return MockArg::Other;
        }
        if toks.get(open).is_some_and(|n| n.kind == TokKind::Ident) {
            open += 1;
        }
        if toks.get(open).is_some_and(|n| n.kind == TokKind::LParen) {
            return params(open).unwrap_or(MockArg::Other);
        }
        return MockArg::Other;
    }
    // `x => ...`: one parameter.
    if t.kind == TokKind::Ident && toks.get(k + 1).is_some_and(|a| a.kind == TokKind::Arrow) {
        return MockArg::FactoryWithParam;
    }
    MockArg::Other
}

/// Whether the statement that starts with `toks[i]` (at depth 0) is at the top
/// level: the token before it ends a statement.
fn statement_start(toks: &[Token], i: usize) -> bool {
    let t = &toks[i];
    if t.depth != 0 {
        return false;
    }
    let Some(p) = i.checked_sub(1).map(|p| &toks[p]) else {
        return true;
    };
    match p.kind {
        TokKind::Semi | TokKind::RBrace => true,
        TokKind::RParen
        | TokKind::RBracket
        | TokKind::Str
        | TokKind::Template
        | TokKind::TemplateTail
        | TokKind::Number
        | TokKind::Regex
        | TokKind::Jsx => t.flags & NEWLINE_BEFORE != 0,
        TokKind::Ident => t.flags & NEWLINE_BEFORE != 0 && p.kw == Kw::None,
        _ => false,
    }
}

/// Scans `src` for uses of the mock APIs.
pub fn scan(src: &[u8], dialect: Dialect) -> MockScan {
    let mut out = MockScan::default();
    if !may_mock(src) {
        return out;
    }
    let mut lx = Lexer::new(src, dialect);
    let mut toks: Vec<Token> = Vec::new();
    loop {
        let t = lx.next_token();
        if t.kind == TokKind::Eof || toks.len() > 2_000_000 {
            break;
        }
        if t.kind != TokKind::TripleSlash {
            toks.push(t);
        }
    }
    let text = |t: &Token| t.text(src);
    let mut line = 1u32;
    let mut line_at = 0usize;
    let mut line_of = |t: &Token| {
        let start = (t.start as usize).min(src.len());
        if start >= line_at {
            line += memchr::memchr_iter(b'\n', &src[line_at..start]).count() as u32;
            line_at = start;
        }
        line
    };
    // Method tokens that are part of a call read below.
    let mut read = vec![false; toks.len()];
    // The replacing calls read so far: the tokens of their arguments (open and
    // closing parenthesis) and their request.
    let mut replacing: Vec<(usize, usize, Option<String>)> = Vec::new();
    for i in 0..toks.len() {
        let t = toks[i];
        if t.kind != TokKind::Ident {
            continue;
        }
        let after_dot = i > 0 && matches!(toks[i - 1].kind, TokKind::Dot | TokKind::OptChain);
        let name = text(&t);
        if !after_dot {
            // `<object>.<method>(`.
            let (Some(d), Some(m)) = (toks.get(i + 1), toks.get(i + 2)) else {
                continue;
            };
            if d.kind == TokKind::Dot && m.kind == TokKind::Ident {
                let method = text(m);
                if let Some(api) = MOCK_APIS
                    .iter()
                    .find(|a| a.object.as_bytes() == name && a.method.as_bytes() == method)
                    && let Some(lp) = call_paren(&toks, i + 3)
                {
                    read[i + 2] = true;
                    if api.role == MockRole::Registry {
                        continue;
                    }
                    let (request, arg) = match literal_request(src, &toks, lp + 1) {
                        Some((r, next)) => (Some(r), second_arg(src, &toks, next)),
                        None if toks.get(lp + 1).is_some_and(|a| a.kind == TokKind::RParen) => {
                            (None, MockArg::Bare)
                        }
                        None => {
                            // A computed request: the argument after it, if the request is
                            // one token.
                            (None, second_arg(src, &toks, lp + 2))
                        }
                    };
                    // The innermost replacing call this one stands in (they are read in
                    // source order: an inner one comes later).
                    let within = replacing
                        .iter()
                        .rev()
                        .find(|(open, close, _)| *open < i && i < *close)
                        .and_then(|(_, _, r)| r.clone());
                    if matches!(api.role, MockRole::Replace | MockRole::ReplaceLate) {
                        let depth = toks[lp].depth;
                        let close = (lp + 1..toks.len())
                            .find(|&c| toks[c].kind == TokKind::RParen && toks[c].depth == depth)
                            .unwrap_or(toks.len());
                        replacing.push((lp, close, request.clone()));
                    }
                    out.calls.push(MockCall {
                        api,
                        request,
                        line: line_of(&t),
                        top_level: statement_start(&toks, i),
                        arg,
                        within,
                    });
                    continue;
                }
            }
        }
        if read[i] {
            continue;
        }
        if distinctive(name) {
            let request = call_paren(&toks, i + 1)
                .and_then(|lp| literal_request(src, &toks, lp + 1))
                .map(|(r, _)| r);
            out.loose.push(LooseUse {
                name: String::from_utf8_lossy(name).into_owned(),
                request,
                line: line_of(&t),
            });
        } else if after_dot
            && (name == b"mock" || name == b"doMock")
            && let Some(lp) = call_paren(&toks, i + 1)
            && let Some((request, _)) = literal_request(src, &toks, lp + 1)
        {
            // A replacing call on a receiver this scan does not know (an alias of the
            // runner's object, a chained call).
            out.loose.push(LooseUse {
                name: String::from_utf8_lossy(name).into_owned(),
                request: Some(request),
                line: line_of(&t),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calls(src: &str) -> Vec<(String, Option<String>, bool, MockArg, u32)> {
        scan(src.as_bytes(), Dialect::TS)
            .calls
            .into_iter()
            .map(|c| (c.name(), c.request, c.top_level, c.arg, c.line))
            .collect()
    }

    #[test]
    fn forms_of_a_replacement() {
        let src = "import { a } from './a';\njest.mock('./a', () => ({ a: 1 \
                   }));\njest.mock('./b');\njest.mock('./c', function () { return {}; \
                   });\njest.mock('./d', factory);\njest.mock('./e', () => ({}), { virtual: true \
                   });\nvi.mock('./f', async (importOriginal) => ({ ...(await importOriginal()) \
                   }));\nvi.mock('./g', { spy: true });\nvi.mock(import('./h'), () => \
                   ({}));\nvi.mock('./i', async () => ({}));\nvi.mock('./j', orig => \
                   orig());\nvitest.mock(`./k`, () => ({}));\n";
        let got = calls(src);
        let want = [
            ("jest.mock", "./a", MockArg::Factory),
            ("jest.mock", "./b", MockArg::Bare),
            ("jest.mock", "./c", MockArg::Factory),
            ("jest.mock", "./d", MockArg::Other),
            ("jest.mock", "./e", MockArg::Factory),
            ("vi.mock", "./f", MockArg::FactoryWithParam),
            ("vi.mock", "./g", MockArg::Options),
            ("vi.mock", "./h", MockArg::Factory),
            ("vi.mock", "./i", MockArg::Factory),
            ("vi.mock", "./j", MockArg::FactoryWithParam),
            ("vitest.mock", "./k", MockArg::Factory),
        ];
        assert_eq!(got.len(), want.len(), "{got:?}");
        for (g, w) in got.iter().zip(want) {
            assert_eq!((g.0.as_str(), g.1.as_deref(), g.3), (w.0, Some(w.1), w.2));
            assert!(g.2, "{g:?} is a top-level statement");
        }
        assert_eq!(got[0].4, 2);
        assert_eq!(got[10].4, 12);
    }

    #[test]
    fn only_statements_at_the_top_level_are_top_level() {
        let src = "jest.mock('./a', () => ({}))\nconst x = 1\njest.mock('./b', () => \
                   ({}))\nbeforeAll(() => { jest.mock('./c', () => ({})); });\nif (x) { \
                   jest.mock('./d', () => ({})); }\nconst y = jest.mock('./e', () => ({}));\nx && \
                   jest.mock('./f', () => ({}));\nfunction f() { jest.mock('./g', () => ({})); \
                   }\ndescribe('x', () => {\n  jest.mock('./h', () => ({}));\n});\njest\n  \
                   .mock('./i', () => ({}));\nawait vi.mock('./j', () => ({}));\n";
        let top: Vec<(String, bool)> = calls(src)
            .into_iter()
            .map(|c| (c.1.unwrap(), c.2))
            .collect();
        let want = [
            ("./a", true),
            ("./b", true),
            ("./c", false),
            ("./d", false),
            ("./e", false),
            ("./f", false),
            ("./g", false),
            ("./h", false),
            ("./i", true),
            ("./j", false),
        ];
        assert_eq!(
            top,
            want.map(|(a, b)| (a.to_owned(), b)),
            "top-level statements"
        );
    }

    #[test]
    fn loads_of_the_real_module_and_loose_uses() {
        let src = "const real = jest.requireActual<typeof import('./a')>('./a');\nconst r2 = \
                   await vi.importActual('./b');\nconst r3 = jest.requireActual(name);\nconst { \
                   requireActual } = \
                   jest;\nhelper.unmock('./c');\nj.mock('./d');\njest.mock('./e', () => \
                   ({})).mock('./f', () => \
                   ({}));\njest.resetModules();\nfn.mock.calls[0];\njest.unmock('./g');\njest.\
                   requireMock('./h');\njest.enableAutomock();\n";
        let s = scan(src.as_bytes(), Dialect::TS);
        let got: Vec<(String, Option<&str>)> = s
            .calls
            .iter()
            .map(|c| (c.name(), c.request.as_deref()))
            .collect();
        assert_eq!(
            got,
            [
                ("jest.requireActual".to_owned(), Some("./a")),
                ("vi.importActual".to_owned(), Some("./b")),
                ("jest.requireActual".to_owned(), None),
                ("jest.mock".to_owned(), Some("./e")),
                ("jest.unmock".to_owned(), Some("./g")),
                ("jest.requireMock".to_owned(), Some("./h")),
                ("jest.enableAutomock".to_owned(), None),
            ]
        );
        let loose: Vec<(&str, Option<&str>, u32)> = s
            .loose
            .iter()
            .map(|l| (l.name.as_str(), l.request.as_deref(), l.line))
            .collect();
        assert_eq!(
            loose,
            [
                ("requireActual", None, 4),
                ("unmock", Some("./c"), 5),
                ("mock", Some("./d"), 6),
                ("mock", Some("./f"), 7),
            ]
        );
    }

    #[test]
    fn the_prefilter_knows_every_api() {
        // Calls that clear the registry are not reported.
        for a in MOCK_APIS.iter().filter(|a| a.role != MockRole::Registry) {
            assert!(may_mock(a.method.as_bytes()), "{}", a.method);
        }
        assert!(!may_mock(b"const lock = block(socket);\n"));
        // An API name as a whole word, however it is used; not a longer word around it.
        for src in [
            "jest\n  .mock('./a')",
            "x.mock ('./a')",
            "const { requireActual: real } = jest;",
            "vi.importActual<typeof import('./a')>('./a')",
            "obj['unmock']",
            "#mock",
        ] {
            assert!(may_mock(src.as_bytes()), "{src}");
        }
        for src in [
            "jest.fn().mockReturnValue(1); const mockFn = jest.fn(); expect(mockFn.mock.calls)",
            "// Actually, the Mock server is mocked by mockServer()",
            "import { MockAgent } from 'undici'; const mock_ = 1; const $mock = 2; remocked",
            "requireActualThing(); remock(); mock\u{e9}()",
        ] {
            // `mockFn.mock.calls` has the word `mock`: only that one passes.
            assert_eq!(may_mock(src.as_bytes()), src.contains(".mock."), "{src}");
            assert!(scan(src.as_bytes(), Dialect::TS).is_empty(), "{src}");
        }
    }

    #[test]
    fn a_call_in_a_factory_knows_its_mock() {
        let src = "jest.mock('./a', () => ({ ...jest.requireActual('./a'), x: \
                   jest.requireActual('./b') }));\njest.requireActual('./a');\njest.doMock(name, \
                   () => jest.requireActual('./c'));\njest.mock('./d', () => { jest.mock('./e', \
                   () => jest.requireActual('./e')); return jest.requireActual('./d'); });\n";
        let s = scan(src.as_bytes(), Dialect::TS);
        let got: Vec<(String, Option<&str>, Option<&str>)> = s
            .calls
            .iter()
            .map(|c| (c.name(), c.request.as_deref(), c.within.as_deref()))
            .collect();
        let call = |n: &str, r: Option<&'static str>, w: Option<&'static str>| (n.to_owned(), r, w);
        assert_eq!(
            got,
            [
                call("jest.mock", Some("./a"), None),
                call("jest.requireActual", Some("./a"), Some("./a")),
                call("jest.requireActual", Some("./b"), Some("./a")),
                // Outside any mock call; in the factory of a mock of a computed module.
                call("jest.requireActual", Some("./a"), None),
                call("jest.doMock", None, None),
                call("jest.requireActual", Some("./c"), None),
                // The innermost one.
                call("jest.mock", Some("./d"), None),
                call("jest.mock", Some("./e"), Some("./d")),
                call("jest.requireActual", Some("./e"), Some("./e")),
                call("jest.requireActual", Some("./d"), Some("./d")),
            ]
        );
        assert_eq!(MockScan::decode(&s.encode()), Some(s));
    }

    #[test]
    fn the_fact_round_trips() {
        let src = "jest.mock('./a', () => ({}));\nconst { requireActual } = jest;\nvi.doMock(x);\n";
        let s = scan(src.as_bytes(), Dialect::JS);
        assert_eq!(s.calls.len(), 2);
        let f = s.encode();
        assert!(f.starts_with("mock:{"), "{f}");
        assert_eq!(MockScan::decode(&f), Some(s));
        assert_eq!(MockScan::decode("env:{}"), None);
        assert!(scan(b"const a = require('./a');\n", Dialect::JS).is_empty());
        assert!(
            scan(
                b"jest.fn().mockReturnValue(1); x.mock.calls;\n",
                Dialect::JS
            )
            .is_empty()
        );
    }
}
