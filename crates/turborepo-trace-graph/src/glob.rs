//! A small path glob matcher: `*`, `?`, `[abc]`, `[!a-z]`, `{a,b}` and `**`
//! segments.
//!
//! `*`, `?` and classes never match `/`. `**` as a whole segment matches zero
//! or more segments (`a/**/b` matches `a/b` and `a/x/y/b`; `a/**` matches
//! everything below `a`). Braces are expanded at compile time and may nest. A
//! backslash takes the next character literally, everywhere (`\[id\]` is the
//! name `[id]`, `\{a,b\}` the name `{a,b}`).

/// Whether `s` contains glob metacharacters.
pub fn is_glob(s: &str) -> bool {
    s.bytes().any(|b| matches!(b, b'*' | b'?' | b'[' | b'{'))
}

/// The characters [`escape`] puts a backslash before: the ones that start glob
/// syntax ([`is_glob`]) and the backslash itself.
const SPECIAL: [char; 5] = ['*', '?', '[', '{', '\\'];

/// The pattern that matches exactly the name `name` and nothing else: the name
/// with a backslash before each character that has a meaning in a pattern. A
/// name without such a character is its own pattern.
///
/// A directory called `[id]` (a route segment of Next.js) or `{slug}` is a name
/// like any other, and the record of a lookup that asked for it by name has to
/// say that name: read as a pattern, `[id]` matches `i` and `d`.
pub fn escape(name: &str) -> String {
    if !name.contains(SPECIAL) {
        return name.to_owned();
    }
    let mut out = String::with_capacity(name.len() + 4);
    for c in name.chars() {
        if SPECIAL.contains(&c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The one name that `pattern` matches when no glob syntax is left once its
/// escapes are read (the inverse of [`escape`]); `None` for a pattern with a
/// wildcard, a class or braces, and for one that ends in a lone backslash.
pub fn literal(pattern: &str) -> Option<String> {
    let mut out = String::with_capacity(pattern.len());
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push(chars.next()?),
            '*' | '?' | '[' | '{' => return None,
            c => out.push(c),
        }
    }
    Some(out)
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    Lit(Box<[u8]>),
    Any,
    Star,
    /// `**/`: empty, or any string ending in `/`.
    DirStar,
    /// Trailing `**`: anything.
    AnyAll,
    Class {
        negated: bool,
        ranges: Box<[(u8, u8)]>,
    },
}

/// A compiled glob.
#[derive(Clone, Debug)]
pub struct Glob {
    source: String,
    alts: Vec<Vec<Tok>>,
}

/// A glob syntax error.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message} at byte {offset}")]
pub struct GlobError {
    /// Byte offset into the pattern.
    pub offset: usize,
    /// What is wrong.
    pub message: String,
}

impl Glob {
    /// Compiles `pattern`.
    pub fn new(pattern: &str) -> Result<Glob, GlobError> {
        let expanded = expand_braces(pattern)?;
        let alts = expanded
            .iter()
            .map(|p| compile(p))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Glob {
            source: pattern.to_owned(),
            alts,
        })
    }

    /// The source pattern.
    pub fn as_str(&self) -> &str {
        &self.source
    }

    /// Whether `path` matches.
    pub fn matches(&self, path: &str) -> bool {
        let s = path.as_bytes();
        self.alts.iter().any(|t| match_toks(t, s))
    }

    /// The literal prefix of each alternative: every match starts with one of
    /// these.
    pub fn literal_prefixes(&self) -> Vec<&[u8]> {
        self.alts
            .iter()
            .map(|t| match t.first() {
                Some(Tok::Lit(l)) => &l[..],
                _ => &[][..],
            })
            .collect()
    }
}

fn expand_braces(p: &str) -> Result<Vec<String>, GlobError> {
    let b = p.as_bytes();
    // Find the first top-level `{` and its matching `}`.
    let mut i = 0;
    let mut in_class = false;
    while i < b.len() {
        match b[i] {
            // An escaped character opens and closes nothing.
            b'\\' => i += 1,
            b'[' if !in_class => in_class = true,
            b']' if in_class => in_class = false,
            b'{' if !in_class => {
                let mut depth = 0;
                let mut parts = Vec::new();
                let mut start = i + 1;
                let mut j = i;
                while j < b.len() {
                    match b[j] {
                        b'\\' => j += 1,
                        b'{' => depth += 1,
                        b'}' => {
                            depth -= 1;
                            if depth == 0 {
                                parts.push(&p[start..j]);
                                break;
                            }
                        }
                        b',' if depth == 1 => {
                            parts.push(&p[start..j]);
                            start = j + 1;
                        }
                        _ => {}
                    }
                    j += 1;
                }
                if j >= b.len() {
                    return Err(GlobError {
                        offset: i,
                        message: "unclosed '{'".into(),
                    });
                }
                let (head, tail) = (&p[..i], &p[j + 1..]);
                let mut out = Vec::new();
                for part in parts {
                    for e in expand_braces(&format!("{head}{part}{tail}"))? {
                        out.push(e);
                    }
                }
                return Ok(out);
            }
            _ => {}
        }
        i += 1;
    }
    Ok(vec![p.to_owned()])
}

fn compile(p: &str) -> Result<Vec<Tok>, GlobError> {
    let b = p.as_bytes();
    let mut toks = Vec::new();
    let mut lit = Vec::new();
    let flush = |lit: &mut Vec<u8>, toks: &mut Vec<Tok>| {
        if !lit.is_empty() {
            toks.push(Tok::Lit(std::mem::take(lit).into()));
        }
    };
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'*' => {
                flush(&mut lit, &mut toks);
                let double = b.get(i + 1) == Some(&b'*');
                let seg_start = i == 0 || b[i - 1] == b'/';
                if double && seg_start {
                    match b.get(i + 2) {
                        Some(b'/') => {
                            toks.push(Tok::DirStar);
                            i += 3;
                            continue;
                        }
                        None => {
                            toks.push(Tok::AnyAll);
                            i += 2;
                            continue;
                        }
                        _ => {}
                    }
                }
                if toks.last() != Some(&Tok::Star) {
                    toks.push(Tok::Star);
                }
                i += if double { 2 } else { 1 };
            }
            b'?' => {
                flush(&mut lit, &mut toks);
                toks.push(Tok::Any);
                i += 1;
            }
            b'[' => {
                flush(&mut lit, &mut toks);
                let start = i;
                i += 1;
                let negated = matches!(b.get(i), Some(b'!' | b'^'));
                if negated {
                    i += 1;
                }
                let mut ranges = Vec::new();
                let mut first = true;
                loop {
                    let Some(&c) = b.get(i) else {
                        return Err(GlobError {
                            offset: start,
                            message: "unclosed '['".into(),
                        });
                    };
                    if c == b']' && !first {
                        i += 1;
                        break;
                    }
                    first = false;
                    if b.get(i + 1) == Some(&b'-') && b.get(i + 2).is_some_and(|&e| e != b']') {
                        ranges.push((c, b[i + 2]));
                        i += 3;
                    } else {
                        ranges.push((c, c));
                        i += 1;
                    }
                }
                toks.push(Tok::Class {
                    negated,
                    ranges: ranges.into(),
                });
            }
            b'\\' if i + 1 < b.len() => {
                lit.push(b[i + 1]);
                i += 2;
            }
            c => {
                lit.push(c);
                i += 1;
            }
        }
    }
    flush(&mut lit, &mut toks);
    Ok(toks)
}

fn match_toks(t: &[Tok], s: &[u8]) -> bool {
    let Some((first, rest)) = t.split_first() else {
        return s.is_empty();
    };
    match first {
        Tok::Lit(l) => s.starts_with(l) && match_toks(rest, &s[l.len()..]),
        Tok::Any => s.first().is_some_and(|&c| c != b'/') && match_toks(rest, &s[1..]),
        Tok::Class { negated, ranges } => {
            s.first().is_some_and(|&c| {
                c != b'/' && ranges.iter().any(|&(lo, hi)| lo <= c && c <= hi) != *negated
            }) && match_toks(rest, &s[1..])
        }
        Tok::Star => {
            if rest.is_empty() {
                return !s.contains(&b'/');
            }
            for i in 0..=s.len() {
                if match_toks(rest, &s[i..]) {
                    return true;
                }
                if i < s.len() && s[i] == b'/' {
                    break;
                }
            }
            false
        }
        Tok::DirStar => {
            if match_toks(rest, s) {
                return true;
            }
            for (i, &c) in s.iter().enumerate() {
                if c == b'/' && match_toks(rest, &s[i + 1..]) {
                    return true;
                }
            }
            false
        }
        Tok::AnyAll => {
            if rest.is_empty() {
                return true;
            }
            (0..=s.len()).any(|i| match_toks(rest, &s[i..]))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(p: &str, s: &str) -> bool {
        Glob::new(p).unwrap().matches(s)
    }

    #[test]
    fn basics() {
        assert!(m("src/*.ts", "src/a.ts"));
        assert!(!m("src/*.ts", "src/x/a.ts"));
        assert!(m("src/**/*.ts", "src/a.ts"));
        assert!(m("src/**/*.ts", "src/x/y/a.ts"));
        assert!(!m("src/**/*.ts", "lib/a.ts"));
        assert!(m("**", "a/b/c"));
        assert!(m("**/*.json", "a.json"));
        assert!(m("a/**", "a/b/c"));
        assert!(!m("a/**", "ab/c"));
        assert!(m("?.js", "a.js"));
        assert!(!m("?.js", "/.js"));
        assert!(m("[a-c].js", "b.js"));
        assert!(!m("[!a-c].js", "b.js"));
        assert!(m("*.{js,ts}", "x.ts"));
        assert!(m("{a,b/{c,d}}/x", "b/d/x"));
        assert!(!m("*.{js,ts}", "x.md"));
        assert!(m("a\\*b", "a*b"));
        assert!(m("*", "abc"));
        assert!(!m("*", "a/b"));
    }

    #[test]
    fn an_escaped_bracket_or_brace_is_a_character() {
        assert!(m(r"\[id\]", "[id]"));
        assert!(!m(r"\[id\]", "i"));
        assert!(m(r"\{a,b\}", "{a,b}"));
        assert!(!m(r"\{a,b\}", "a"));
        assert!(m(r"\{x\}/*.{js,ts}", "{x}/a.ts"));
        assert!(m(r"{a\,b,c}", "a,b"));
        assert!(m(r"{\{,x}y", "{y"));
        assert!(m(r"{\{,x}y", "xy"));
        assert!(Glob::new(r"a\{b}").is_ok());
        assert!(Glob::new(r"a{b\}").is_err());
    }

    #[test]
    fn a_name_as_a_pattern() {
        for name in [
            "base",
            "[id]",
            "[...slug]",
            "[[...all]]",
            "{a,b}",
            "a*b",
            "what?",
            r"back\slash",
            "a]b",
            "a}b",
            "!x",
            "(group)",
            "@modal",
            "a,b",
            r"\",
        ] {
            let p = escape(name);
            assert_eq!(literal(&p).as_deref(), Some(name), "{p}");
            assert!(m(&p, name), "{p}");
            assert_eq!(is_glob(name) || name.contains('\\'), p != name, "{p}");
        }
        // The pattern of `[id]` matches that name alone.
        assert_eq!(escape("[id]"), r"\[id]");
        for other in ["i", "d", "id", "[id", "x[id]", r"\[id]"] {
            assert!(!m(&escape("[id]"), other), "{other}");
        }
        for p in ["*", "a*", "[id]", "{a,b}", "x?", r"a\"] {
            assert_eq!(literal(p), None, "{p}");
        }
        assert_eq!(literal("plain.json").as_deref(), Some("plain.json"));
    }

    #[test]
    fn prefixes_and_errors() {
        let g = Glob::new("src/{a,b}/*.ts").unwrap();
        assert_eq!(g.literal_prefixes(), vec![&b"src/a/"[..], &b"src/b/"[..]]);
        assert_eq!(
            Glob::new("**/x").unwrap().literal_prefixes(),
            vec![&b""[..]]
        );
        assert!(Glob::new("a{b").is_err());
        assert!(Glob::new("a[b").is_err());
        assert!(is_glob("a/*") && !is_glob("a/b.ts"));
    }
}
