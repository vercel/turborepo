//! Patterns from runner configs: JS regexes and micromatch globs, both run on
//! the `regex` crate.
//!
//! - [`JsRegex`]: a JS regex source plus flags. Translation handles `\/`, the
//!   `i`, `s`, `m` flags and named groups. Lookaround and backreferences, which
//!   the `regex` crate leaves out, run on `fancy-regex` (a backtracking engine
//!   with the same syntax otherwise), so `^#/(?!generated/)(.*)$` matches what
//!   it matches in JS. A regex neither engine compiles is [not
//!   supported](JsRegex::is_supported): it never matches, and adapters report
//!   it as an uncertainty.
//! - Globs ([`glob_to_regex`]): the picomatch subset runners use: `*`, `?`,
//!   `**`, `[...]`, `{a,b}` (nested), extglobs `?(...)`, `*(...)`, `+(...)`,
//!   `@(...)`, plain groups `(a|b)`, and a leading `./` (stripped). `*` matches
//!   dotfiles, as with `dot: true` in jest and vitest. `!(...)` needs
//!   lookahead: it is approximated by `*` (over-matching), and
//!   [`glob_to_regex`] reports it.

use std::sync::OnceLock;

use regex::Regex;
use serde::{Serialize, Serializer};

use crate::inputs::TestPattern;

/// A JS regex.
#[derive(Clone, Debug)]
pub struct JsRegex {
    source: String,
    flags: String,
    compiled: OnceLock<Option<Engine>>,
}

/// The engine a [`JsRegex`] runs on.
#[derive(Clone, Debug)]
enum Engine {
    /// The `regex` crate (linear time): everything without lookaround or
    /// backreferences.
    Plain(Regex),
    /// `fancy-regex` (backtracking): lookaround and backreferences.
    Fancy(Box<fancy_regex::Regex>),
}

/// One match of a [`JsRegex`]: its byte range and groups.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegexMatch {
    /// Start of the match.
    pub start: usize,
    /// End of the match.
    pub end: usize,
    /// The groups (index 0 is the whole match); `None` for a group that did not
    /// take part.
    pub groups: Vec<Option<String>>,
}

impl PartialEq for JsRegex {
    fn eq(&self, o: &Self) -> bool {
        self.source == o.source && self.flags == o.flags
    }
}

impl Eq for JsRegex {}

impl Serialize for JsRegex {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if self.flags.is_empty() {
            s.serialize_str(&self.source)
        } else {
            s.serialize_str(&format!("/{}/{}", self.source, self.flags))
        }
    }
}

impl JsRegex {
    /// A regex from its JS source and flags.
    pub fn new(source: impl Into<String>, flags: impl Into<String>) -> Self {
        JsRegex {
            source: source.into(),
            flags: flags.into(),
            compiled: OnceLock::new(),
        }
    }

    /// The JS source.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The JS flags.
    pub fn flags(&self) -> &str {
        &self.flags
    }

    fn engine(&self) -> Option<&Engine> {
        self.compiled
            .get_or_init(|| {
                let t = translate(&self.source, &self.flags);
                match Regex::new(&t) {
                    Ok(r) => Some(Engine::Plain(r)),
                    Err(_) => fancy_regex::Regex::new(&t)
                        .ok()
                        .map(|r| Engine::Fancy(Box::new(r))),
                }
            })
            .as_ref()
    }

    /// Whether an engine can run it.
    pub fn is_supported(&self) -> bool {
        self.engine().is_some()
    }

    /// `regex.test(s)`. A match the backtracking engine gives up on (its step
    /// limit) counts as no match.
    pub fn is_match(&self, s: &str) -> bool {
        match self.engine() {
            Some(Engine::Plain(r)) => r.is_match(s),
            Some(Engine::Fancy(r)) => r.is_match(s).unwrap_or(false),
            None => false,
        }
    }

    /// The first match at or after byte `from`.
    pub fn find_at(&self, s: &str, from: usize) -> Option<RegexMatch> {
        match self.engine()? {
            Engine::Plain(r) => {
                let c = r.captures_at(s, from)?;
                let m = c.get(0)?;
                Some(RegexMatch {
                    start: m.start(),
                    end: m.end(),
                    groups: c
                        .iter()
                        .map(|g| g.map(|g| g.as_str().to_string()))
                        .collect(),
                })
            }
            Engine::Fancy(r) => {
                let c = r.captures_from_pos(s, from).ok()??;
                let m = c.get(0)?;
                Some(RegexMatch {
                    start: m.start(),
                    end: m.end(),
                    groups: c
                        .iter()
                        .map(|g| g.map(|g| g.as_str().to_string()))
                        .collect(),
                })
            }
        }
    }

    /// The groups of the first match (index 0 is the whole match).
    pub fn captures(&self, s: &str) -> Option<Vec<Option<String>>> {
        self.find_at(s, 0).map(|m| m.groups)
    }

    /// `s.replace(regex, replacement)`: the first match (every match with the
    /// `g` flag) is replaced, with JS replacement patterns (`$$`, `$&`, ``
    /// $` ``, `$'`, `$1`..`$99`; `$<name>` is left as written). `None` when
    /// nothing matches.
    pub fn replace(&self, s: &str, replacement: &str) -> Option<String> {
        let global = self.flags.contains('g');
        let mut out = String::with_capacity(s.len() + replacement.len());
        let mut pos = 0;
        let mut matched = false;
        while pos <= s.len() {
            let Some(m) = self.find_at(s, pos) else {
                break;
            };
            matched = true;
            out.push_str(&s[pos..m.start]);
            expand_replacement(replacement, s, &m, &mut out);
            pos = m.end;
            if m.end == m.start {
                // An empty match: copy one character and go on after it.
                match s[m.end..].chars().next() {
                    Some(c) => {
                        out.push(c);
                        pos += c.len_utf8();
                    }
                    None => pos = s.len() + 1,
                }
            }
            if !global {
                break;
            }
        }
        if !matched {
            return None;
        }
        if pos < s.len() {
            out.push_str(&s[pos..]);
        }
        Some(out)
    }
}

/// Appends `replacement` for the match `m` in `s`, expanding JS replacement
/// patterns.
fn expand_replacement(replacement: &str, s: &str, m: &RegexMatch, out: &mut String) {
    let b = replacement.as_bytes();
    let groups = m.groups.len().saturating_sub(1);
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'$' || i + 1 >= b.len() {
            let ch = replacement[i..].chars().next().unwrap_or('$');
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        match b[i + 1] {
            b'$' => {
                out.push('$');
                i += 2;
            }
            b'&' => {
                out.push_str(&s[m.start..m.end]);
                i += 2;
            }
            b'`' => {
                out.push_str(&s[..m.start]);
                i += 2;
            }
            b'\'' => {
                out.push_str(&s[m.end..]);
                i += 2;
            }
            d if d.is_ascii_digit() => {
                // Two digits when that group exists, else one digit, else literal text.
                let one = usize::from(d - b'0');
                let two = b
                    .get(i + 2)
                    .filter(|x| x.is_ascii_digit())
                    .map(|x| one * 10 + usize::from(x - b'0'));
                let (n, len) = match two {
                    Some(n) if (1..=groups).contains(&n) => (n, 3),
                    _ if (1..=groups).contains(&one) => (one, 2),
                    _ => {
                        out.push('$');
                        i += 1;
                        continue;
                    }
                };
                if let Some(Some(g)) = m.groups.get(n) {
                    out.push_str(g);
                }
                i += len;
            }
            _ => {
                out.push('$');
                i += 1;
            }
        }
    }
}

/// JS regex syntax to `regex` syntax.
fn translate(src: &str, flags: &str) -> String {
    let mut out = String::with_capacity(src.len() + 8);
    let mut fl = String::new();
    for f in flags.chars() {
        if matches!(f, 'i' | 's' | 'm') {
            fl.push(f);
        }
    }
    if !fl.is_empty() {
        out.push_str(&format!("(?{fl})"));
    }
    let mut chars = src.chars().peekable();
    let mut in_class = false;
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                // Escaped characters that need no escape in Rust (and some that may not
                // be escaped there).
                Some(
                    e @ ('/' | '-' | '_' | '"' | '\'' | '`' | ',' | ':' | '=' | '!' | '<' | '>'
                    | '@' | '%' | ';'),
                ) => {
                    if in_class && e == '-' {
                        out.push_str("\\-");
                    } else {
                        out.push_str(&regex::escape(&e.to_string()));
                    }
                }
                Some(e) => {
                    out.push('\\');
                    out.push(e);
                }
                None => out.push_str("\\\\"),
            },
            '[' if !in_class => {
                in_class = true;
                out.push('[');
                // `[^]` / `[]` edge cases are rare; `[` inside a class must be escaped.
                if chars.peek() == Some(&'^') {
                    out.push('^');
                    chars.next();
                }
            }
            '[' if in_class => out.push_str("\\["),
            ']' if in_class => {
                in_class = false;
                out.push(']');
            }
            '&' | '~' if in_class => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// A glob compiled to a regex. Returns the regex source (anchored) and whether
/// the translation over-approximates (`!(...)`).
pub fn glob_to_regex(glob: &str) -> (String, bool) {
    let g = glob.strip_prefix("./").unwrap_or(glob);
    let mut approx = false;
    let body = glob_body(g, &mut approx);
    (format!("^{body}$"), approx)
}

fn glob_body(g: &str, approx: &mut bool) -> String {
    let b: Vec<char> = g.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        // Extglobs.
        if matches!(c, '?' | '*' | '+' | '@' | '!')
            && b.get(i + 1) == Some(&'(')
            && let Some(close) = find_close(&b, i + 1, '(', ')')
        {
            let inner: String = b[i + 2..close].iter().collect();
            let alts: Vec<String> = split_top(&inner, '|')
                .iter()
                .map(|a| glob_body(a, approx))
                .collect();
            let group = format!("(?:{})", alts.join("|"));
            match c {
                '?' => out.push_str(&format!("{group}?")),
                '*' => out.push_str(&format!("{group}*")),
                '+' => out.push_str(&format!("{group}+")),
                '@' => out.push_str(&group),
                _ => {
                    *approx = true;
                    out.push_str("[^/]*");
                }
            }
            i = close + 1;
            continue;
        }
        match c {
            '*' => {
                if b.get(i + 1) == Some(&'*') {
                    let at_seg_start = i == 0 || b[i - 1] == '/';
                    let mut j = i + 2;
                    while b.get(j) == Some(&'*') {
                        j += 1;
                    }
                    if at_seg_start && b.get(j) == Some(&'/') {
                        out.push_str("(?:.*/)?");
                        i = j + 1;
                    } else if at_seg_start && j == b.len() {
                        out.push_str(".*");
                        i = j;
                    } else {
                        out.push_str("[^/]*");
                        i = j;
                    }
                    continue;
                }
                out.push_str("[^/]*");
            }
            '?' => out.push_str("[^/]"),
            '[' => match find_close(&b, i, '[', ']') {
                Some(close) if close > i + 1 => {
                    let mut cls: String = b[i + 1..close].iter().collect();
                    if let Some(rest) = cls.strip_prefix('!') {
                        cls = format!("^{rest}");
                    }
                    out.push('[');
                    out.push_str(&cls.replace('\\', "\\\\").replace('[', "\\["));
                    out.push(']');
                    i = close + 1;
                    continue;
                }
                _ => out.push_str("\\["),
            },
            '{' => match find_close(&b, i, '{', '}') {
                Some(close) => {
                    let inner: String = b[i + 1..close].iter().collect();
                    let parts = split_top(&inner, ',');
                    if parts.len() > 1 {
                        let alts: Vec<String> =
                            parts.iter().map(|a| glob_body(a, approx)).collect();
                        out.push_str(&format!("(?:{})", alts.join("|")));
                    } else {
                        out.push_str(&regex::escape(&format!("{{{inner}}}")));
                    }
                    i = close + 1;
                    continue;
                }
                None => out.push_str("\\{"),
            },
            // A plain group: picomatch keeps parentheses as a regex group, with `|`
            // alternatives (`*.(test|spec).ts`).
            '(' => match find_close(&b, i, '(', ')') {
                Some(close) => {
                    let inner: String = b[i + 1..close].iter().collect();
                    let alts: Vec<String> = split_top(&inner, '|')
                        .iter()
                        .map(|a| glob_body(a, approx))
                        .collect();
                    out.push_str(&format!("(?:{})", alts.join("|")));
                    i = close + 1;
                    continue;
                }
                None => out.push_str("\\("),
            },
            '\\' if i + 1 < b.len() => {
                out.push_str(&regex::escape(&b[i + 1].to_string()));
                i += 2;
                continue;
            }
            c => out.push_str(&regex::escape(&c.to_string())),
        }
        i += 1;
    }
    out
}

fn find_close(b: &[char], open_at: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0;
    let mut i = open_at;
    while i < b.len() {
        if b[i] == '\\' {
            i += 2;
            continue;
        }
        if b[i] == open {
            depth += 1;
        } else if b[i] == close {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Splits on `sep` outside `{}`, `()` and `[]`.
fn split_top(s: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '{' | '(' | '[' => depth += 1,
            '}' | ')' | ']' => depth -= 1,
            _ => {}
        }
        if c == sep && depth == 0 {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

/// Whether the translation of `glob` is exact: it uses only constructs
/// [`glob_to_regex`] implements with picomatch's meaning. Not exact: a negated
/// extglob (approximated), a brace range (`{1..3}`), a POSIX class
/// (`[[:alpha:]]`), an unbalanced bracket, a backslash at the end, a `|`
/// outside a group (picomatch passes it to its regex, where `**/*.test.ts|**/*.
/// spec.ts` matches `a.test.ts`; the translator takes it literally). An adapter
/// may only conclude "this pattern matches no file" from an exact one.
pub fn glob_is_exact(glob: &str) -> bool {
    let (re, approx) = glob_to_regex(glob);
    if approx
        || Regex::new(&re).is_err()
        || glob.contains("[[:")
        || glob.ends_with('\\')
        || split_top(glob, '|').len() > 1
    {
        return false;
    }
    let b: Vec<char> = glob.chars().collect();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            '\\' => i += 1,
            '{' => match find_close(&b, i, '{', '}') {
                Some(close) => {
                    let inner: String = b[i + 1..close].iter().collect();
                    if inner.contains("..") {
                        return false;
                    }
                }
                None => return false,
            },
            '(' if find_close(&b, i, '(', ')').is_none() => return false,
            '[' if find_close(&b, i, '[', ']').is_none() => return false,
            _ => {}
        }
        i += 1;
    }
    true
}

/// Whether `s` contains glob syntax.
pub fn is_glob(s: &str) -> bool {
    s.contains(['*', '?', '[', '{']) || s.contains("+(") || s.contains("@(") || s.contains("!(")
}

/// A [`TestPattern`] ready to match.
pub struct CompiledPattern {
    glob: Option<Option<Regex>>,
    regex: Option<JsRegex>,
}

impl CompiledPattern {
    /// Compiles `p`.
    pub fn new(p: &TestPattern) -> Self {
        match p {
            TestPattern::Glob(g) => CompiledPattern {
                glob: Some(Regex::new(&glob_to_regex(g).0).ok()),
                regex: None,
            },
            TestPattern::Regex(r) => CompiledPattern {
                glob: None,
                regex: Some(r.clone()),
            },
        }
    }

    /// Whether the pattern is a glob (matched against relative paths when a
    /// base is set).
    pub fn is_glob(&self) -> bool {
        self.glob.is_some()
    }

    /// Whether `s` matches.
    pub fn matches(&self, s: &str) -> bool {
        match (&self.glob, &self.regex) {
            (Some(Some(re)), _) => re.is_match(s),
            (_, Some(r)) => r.is_match(s),
            _ => false,
        }
    }
}

/// The pattern as an expression in the `regex` crate's syntax, uncompiled: what
/// [`crate::location::tells_apart`] builds an automaton of (an expression with
/// lookaround or backreferences has none).
pub(crate) fn automaton_text(p: &TestPattern) -> String {
    match p {
        TestPattern::Glob(g) => glob_to_regex(g).0,
        TestPattern::Regex(r) => translate(r.source(), r.flags()),
    }
}

/// Matches a glob against a path (convenience for tests and callers).
pub fn glob_matches(glob: &str, path: &str) -> bool {
    Regex::new(&glob_to_regex(glob).0).is_ok_and(|r| r.is_match(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jest_default_test_match() {
        let a = "**/__tests__/**/*.?([mc])[jt]s?(x)";
        let b = "**/?(*.)+(spec|test).?([mc])[jt]s?(x)";
        for p in [
            "/r/src/__tests__/a.ts",
            "/r/src/__tests__/deep/a.mjs",
            "/r/__tests__/b.tsx",
        ] {
            assert!(glob_matches(a, p), "{p}");
        }
        for p in [
            "/r/src/a.test.ts",
            "/r/a.spec.cjs",
            "/r/x/test.js",
            "/r/x/a.b.test.jsx",
        ] {
            assert!(glob_matches(b, p), "{p}");
        }
        for p in ["/r/src/a.ts", "/r/src/atest.ts", "/r/src/a.test.py"] {
            assert!(!glob_matches(b, p), "{p}");
        }
        assert!(!glob_matches(a, "/r/src/tests/a.ts"));
    }

    #[test]
    fn vitest_default_include_and_braces() {
        let g = "**/*.{test,spec}.?(c|m)[jt]s?(x)";
        assert!(glob_matches(g, "src/a.test.ts"));
        assert!(glob_matches(g, "a.spec.mjs"));
        assert!(!glob_matches(g, "src/a.ts"));
        assert!(glob_matches("./**/*.test.{ts,tsx}", "a/b.test.tsx"));
        assert!(glob_matches("**/{a,b{c,d}}/*.js", "x/bd/y.js"));
        assert!(glob_matches("**/node_modules/**", "a/node_modules/b/c.js"));
        assert!(glob_matches("**/.{idea,git,cache}/**", ".git/x"));
        assert!(glob_matches("src/**", "src/a/b"));
        assert!(!glob_matches("src/*.ts", "src/a/b.ts"));
        let (_, approx) = glob_to_regex("**/!(x).ts");
        assert!(approx);
    }

    /// Plain parentheses are a group in picomatch (`*.(test|spec).[jt]s?(x)`),
    /// not literal characters.
    #[test]
    fn plain_groups_and_exactness() {
        let g = "**/*.(test|spec).[jt]s?(x)";
        assert!(glob_matches(g, "/r/src/a.test.ts"));
        assert!(glob_matches(g, "/r/src/a.spec.jsx"));
        assert!(!glob_matches(g, "/r/src/a.e2e.ts"));
        assert!(!glob_matches(g, "/r/src/a.(test|spec).ts"));
        assert!(glob_matches(
            "**/__tests__/**/*.(test|spec).[jt]s?(x)",
            "/r/src/__tests__/x/a.test.js"
        ));
        // A literal parenthesis needs an escape, as in picomatch.
        assert!(glob_matches("app/\\(auth\\)/*.ts", "app/(auth)/a.ts"));
        assert!(glob_matches("app/(auth)/*.ts", "app/auth/a.ts"));
        for exact in [
            "<rootDir>/**/*.test.ts",
            "**/*.(test|spec).[jt]s?(x)",
            "**/?(*.)+(spec|test).?([mc])[jt]s?(x)",
            "src/**/*.{test,spec}.ts",
        ] {
            assert!(glob_is_exact(exact), "{exact}");
        }
        for inexact in [
            "**/!(x).ts",
            "a/{1..3}.ts",
            "a/[[:alpha:]].ts",
            "a/(b.ts",
            "a/[b.ts",
            "**/*.test.ts|**/*.spec.ts",
        ] {
            assert!(!glob_is_exact(inexact), "{inexact}");
        }
    }

    #[test]
    fn js_regexes() {
        let r = JsRegex::new(r"^(\.{1,2}/.*)\.js$", "");
        assert_eq!(
            r.captures("../a/b.js").unwrap()[1].as_deref(),
            Some("../a/b")
        );
        assert!(JsRegex::new(r"\/node_modules\/", "").is_match("/r/node_modules/x"));
        assert!(JsRegex::new(r"\.SPEC\.ts$", "i").is_match("a.spec.ts"));
        assert!(JsRegex::new(r"^.+\.(t|j)sx?$", "").is_match("a/b.tsx"));
        assert!(JsRegex::new(r"[\-a]", "").is_match("-"));
    }

    /// Lookaround and backreferences run on the backtracking engine, with JS
    /// semantics.
    #[test]
    fn js_regexes_with_lookaround() {
        let la = JsRegex::new(r"^(?!.*node_modules/(jose/)).*node_modules/", "");
        assert!(la.is_supported());
        assert!(la.is_match("a/node_modules/x"));
        assert!(!la.is_match("a/node_modules/jose/x"));
        // A negative lookahead built from a list of files (a vite alias `find`).
        let alias = JsRegex::new(r"^#/(?!(?:app/routes\.mjs|lib/valid\.mjs)$)(.*)$", "");
        assert!(alias.is_supported());
        assert_eq!(
            alias.captures("#/lib/util").unwrap()[1].as_deref(),
            Some("lib/util")
        );
        assert!(alias.captures("#/app/routes.mjs").is_none());
        assert!(alias.captures("#/app/routes.mjs.map").is_some());
        assert!(JsRegex::new(r"(?<=\.)test\.ts$", "").is_match("a.test.ts"));
        assert!(!JsRegex::new(r"(?<=\.)test\.ts$", "").is_match("atest.ts"));
        assert_eq!(
            JsRegex::new(r"(?<name>a)\k<name>", "")
                .captures("aa")
                .unwrap()[1]
                .as_deref(),
            Some("a")
        );
        // Neither engine compiles it: not supported, never matches.
        let bad = JsRegex::new(r"(unclosed", "");
        assert!(!bad.is_supported());
        assert!(!bad.is_match("(unclosed"));
    }

    #[test]
    fn js_replace() {
        let r =
            |re: &str, flags: &str, s: &str, with: &str| JsRegex::new(re, flags).replace(s, with);
        // Only the matched part is replaced.
        assert_eq!(r("^~", "", "~/x/y", "/src").as_deref(), Some("/src/x/y"));
        assert_eq!(
            r(
                r"^@/app/\(site\)(/.*)?$",
                "",
                "@/app/(site)/a/b",
                "/r/site$1"
            )
            .as_deref(),
            Some("/r/site/a/b")
        );
        // An unmatched group is empty; `$$`, `$&`, the text before and after the match.
        assert_eq!(
            r(r"^@/app/\(site\)(/.*)?$", "", "@/app/(site)", "/r/site$1").as_deref(),
            Some("/r/site")
        );
        assert_eq!(r("b", "", "abc", "[$$$&$`$']").as_deref(), Some("a[$bac]c"));
        // `$10` is group 1 followed by `0` when there is no group 10; `$2` without a
        // second group is literal.
        assert_eq!(r("(b)", "", "abc", "$10$2").as_deref(), Some("ab0$2c"));
        // The first match only, unless `g`.
        assert_eq!(r("a", "", "aaa", "b").as_deref(), Some("baa"));
        assert_eq!(r("a", "g", "aaa", "b").as_deref(), Some("bbb"));
        assert_eq!(r("x*", "g", "ab", "-").as_deref(), Some("-a-b-"));
        assert_eq!(r("z", "", "abc", "y"), None);
    }
}
