//! Minimal glob matcher for `--ignore` patterns.
//!
//! Syntax: `*` matches any run of bytes except `/`, `**` matches any run
//! including `/`, `**/` matches zero or more whole directories, `?` matches one
//! byte except `/`. Everything else is literal. A pattern without `/` is
//! matched against the file name only; a pattern containing `/` is matched
//! against the whole path relative to the trace base.
//!
//! Matching uses dynamic programming over (pattern position, input position),
//! so it runs in polynomial time for any pattern; no exponential backtracking.
//! The literal bytes a pattern starts and ends with are compared first, so a
//! path outside the pattern's directory (the common case when a file is checked
//! against many `<dir>/**/*` patterns) is rejected without running the table.

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Tok {
    Byte(u8),
    Any,
    Star,
    DoubleStar,
    /// `**/`: empty, or any run ending in `/`.
    DirsSlash,
}

/// A compiled glob pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Glob {
    source: Box<str>,
    /// The tokens between the literal prefix and the literal suffix.
    toks: Vec<Tok>,
    /// Literal bytes before the first wildcard.
    prefix: Box<[u8]>,
    /// Literal bytes after the last wildcard (empty for a pattern without
    /// wildcards: it is all prefix).
    suffix: Box<[u8]>,
    basename_only: bool,
}

impl Glob {
    /// Compiles `pattern`. Never fails; every string is a valid pattern.
    pub fn new(pattern: &str) -> Self {
        let b = pattern.as_bytes();
        let b = b.strip_prefix(b"./").unwrap_or(b);
        let mut toks = Vec::with_capacity(b.len());
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                b'*' if b.get(i + 1) == Some(&b'*') => {
                    if b.get(i + 2) == Some(&b'/') {
                        toks.push(Tok::DirsSlash);
                        i += 3;
                    } else {
                        toks.push(Tok::DoubleStar);
                        i += 2;
                    }
                }
                b'*' => {
                    toks.push(Tok::Star);
                    i += 1;
                }
                b'?' => {
                    toks.push(Tok::Any);
                    i += 1;
                }
                c => {
                    toks.push(Tok::Byte(c));
                    i += 1;
                }
            }
        }
        let literal = |t: &Tok| match t {
            Tok::Byte(c) => Some(*c),
            _ => None,
        };
        let prefix: Box<[u8]> = toks.iter().map_while(literal).collect();
        let mut suffix: Vec<u8> = toks[prefix.len()..]
            .iter()
            .rev()
            .map_while(literal)
            .collect();
        suffix.reverse();
        let toks = toks[prefix.len()..toks.len() - suffix.len()].to_vec();
        Glob {
            source: pattern.into(),
            basename_only: !b.contains(&b'/'),
            toks,
            prefix,
            suffix: suffix.into(),
        }
    }

    /// The pattern as written.
    pub fn as_str(&self) -> &str {
        &self.source
    }

    /// Whether `rel_path` (relative, `/`-separated) matches.
    pub fn matches(&self, rel_path: &[u8]) -> bool {
        let input = if self.basename_only {
            rel_path
                .iter()
                .rposition(|&b| b == b'/')
                .map_or(rel_path, |i| &rel_path[i + 1..])
        } else {
            rel_path
        };
        self.match_bytes(input)
    }

    fn match_bytes(&self, s: &[u8]) -> bool {
        // The pattern is prefix, tokens, suffix, and what a token matches does not
        // depend on its neighbours: the input matches iff it starts with the
        // prefix, the rest ends with the suffix, and the tokens match what is
        // between.
        let Some(s) = s.strip_prefix(&*self.prefix) else {
            return false;
        };
        let Some(s) = s.strip_suffix(&*self.suffix) else {
            return false;
        };
        match self.toks.as_slice() {
            [] => s.is_empty(),
            [Tok::DoubleStar] => true,
            [Tok::Star] => !s.contains(&b'/'),
            // `<dir>/**/*`: every path below the directory.
            [Tok::DirsSlash, Tok::Star] => true,
            toks => Self::table(toks, s),
        }
    }

    /// The dynamic-programming match of `toks` against `s`.
    fn table(toks: &[Tok], s: &[u8]) -> bool {
        let n = s.len();
        let m = toks.len();
        // dp[j] for the current pattern index i: does toks[i..] match s[j..]?
        let mut next = vec![false; n + 1];
        next[n] = true; // empty pattern matches empty input
        let mut cur = vec![false; n + 1];
        for i in (0..m).rev() {
            match toks[i] {
                Tok::Byte(c) => {
                    for j in 0..=n {
                        cur[j] = j < n && s[j] == c && next[j + 1];
                    }
                }
                Tok::Any => {
                    for j in 0..=n {
                        cur[j] = j < n && s[j] != b'/' && next[j + 1];
                    }
                }
                Tok::Star => {
                    // cur[j] = next[j] || (s[j] != '/' && cur[j+1])
                    cur[n] = next[n];
                    for j in (0..n).rev() {
                        cur[j] = next[j] || (s[j] != b'/' && cur[j + 1]);
                    }
                }
                Tok::DoubleStar => {
                    cur[n] = next[n];
                    for j in (0..n).rev() {
                        cur[j] = next[j] || cur[j + 1];
                    }
                }
                Tok::DirsSlash => {
                    // Empty, or consume s[j..=k] where s[k] == '/' then continue at k+1.
                    // any_slash_after[j] = exists k >= j with s[k]=='/' && next[k+1].
                    let mut acc = false;
                    cur[n] = next[n];
                    for j in (0..n).rev() {
                        if s[j] == b'/' && next[j + 1] {
                            acc = true;
                        }
                        cur[j] = next[j] || acc;
                    }
                }
            }
            std::mem::swap(&mut cur, &mut next);
        }
        next[0]
    }
}

/// A set of globs; matches if any member matches.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GlobSet {
    globs: Vec<Glob>,
}

impl GlobSet {
    /// Compiles every pattern.
    pub fn new<S: AsRef<str>>(patterns: impl IntoIterator<Item = S>) -> Self {
        GlobSet {
            globs: patterns
                .into_iter()
                .map(|p| Glob::new(p.as_ref()))
                .collect(),
        }
    }

    /// Whether the set is empty.
    pub fn is_empty(&self) -> bool {
        self.globs.is_empty()
    }

    /// Whether any glob matches `rel_path`.
    pub fn matches(&self, rel_path: &[u8]) -> bool {
        self.globs.iter().any(|g| g.matches(rel_path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(p: &str, s: &str) -> bool {
        Glob::new(p).matches(s.as_bytes())
    }

    #[test]
    fn literals_and_wildcards() {
        assert!(m("a.txt", "a.txt"));
        assert!(m("a.txt", "dir/a.txt")); // basename-only
        assert!(!m("a.txt", "a.txt.bak"));
        assert!(m("*.md", "docs/readme.md"));
        assert!(m("?.md", "x.md"));
        assert!(!m("?.md", "xy.md"));
        assert!(m("*", ""));
        assert!(m("", ""));
        assert!(!m("", "a"));
    }

    #[test]
    fn path_patterns() {
        assert!(m("src/*.rs", "src/main.rs"));
        assert!(!m("src/*.rs", "src/a/main.rs"));
        assert!(m("src/**", "src/a/b/c"));
        assert!(m("**/test/**", "a/test/b"));
        assert!(m("**/test/**", "test/b"));
        assert!(m("src/**/*.rs", "src/main.rs"));
        assert!(m("src/**/*.rs", "src/a/b/main.rs"));
        assert!(!m("src/**/*.rs", "lib/a.rs"));
        assert!(m("./src/*.rs", "src/x.rs"));
        assert!(m("node_modules/**", "node_modules/a/index.js"));
    }

    /// The table over the whole pattern, without the literal prefix and suffix
    /// shortcut.
    fn reference(p: &str, s: &str) -> bool {
        let g = Glob::new(p);
        let mut toks: Vec<Tok> = g.prefix.iter().map(|&c| Tok::Byte(c)).collect();
        toks.extend(g.toks.iter().copied());
        toks.extend(g.suffix.iter().map(|&c| Tok::Byte(c)));
        let s = s.as_bytes();
        let input = if g.basename_only {
            s.iter()
                .rposition(|&b| b == b'/')
                .map_or(s, |i| &s[i + 1..])
        } else {
            s
        };
        Glob::table(&toks, input)
    }

    #[test]
    fn literal_ends_agree_with_the_table() {
        let pats = [
            "",
            "a",
            "a/b",
            "*",
            "**",
            "**/",
            "**/*",
            "a/**/*",
            "a/**",
            "a/*",
            "a/*/b",
            "a/**/b",
            "/x/y/**/*",
            "/x/y/**/*.ts",
            "/x/y/*.d.ts",
            "**/*.ts",
            "a?c",
            "?",
            "a/**/",
            "**/a",
            "a**",
            "**a",
            "a/b/**/c/*.js",
            "*.md",
            "x*y*z",
            "a/**/**/b",
            "a/*/**",
            "aa*a",
            "a*aa",
        ];
        let inputs = [
            "",
            "a",
            "b",
            "a/",
            "a/b",
            "a/b/c",
            "a/x/b",
            "a/x/y/b",
            "/x/y",
            "/x/y/",
            "/x/y/a.ts",
            "/x/y/z/a.ts",
            "/x/yz/a.ts",
            "/x/y/a.d.ts",
            "/x/y/z/a.d.ts",
            "abc",
            "a/c",
            "ac",
            "a/b/q/c/m.js",
            "a/b/c/m.js",
            "x.md",
            "d/x.md",
            "xyz",
            "x1y2z",
            "x/y/z",
            "aa",
            "aaa",
            "aaaa",
            "ab",
            "a/b/",
            "a//b",
            "/",
        ];
        for p in pats {
            for s in inputs {
                assert_eq!(m(p, s), reference(p, s), "pattern {p:?}, input {s:?}");
            }
        }
    }

    #[test]
    fn many_stars_are_fast() {
        let p = "*a*a*a*a*a*a*a*a*a*a*a*a*b";
        let s = "a".repeat(200);
        assert!(!m(p, &s));
    }

    #[test]
    fn set() {
        let s = GlobSet::new(["*.log", "tmp/**"]);
        assert!(s.matches(b"x/y.log"));
        assert!(s.matches(b"tmp/a/b"));
        assert!(!s.matches(b"src/a.rs"));
        assert!(GlobSet::default().is_empty());
    }
}
