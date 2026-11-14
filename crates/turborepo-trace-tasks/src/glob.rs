//! A small glob matcher for task globs (turbo `inputs`/`outputs`, tsconfig
//! `include`, `--tasks` patterns).
//!
//! Syntax: `*` (any run without `/`), `**` (any run, including `/`; `**/` also
//! matches zero directories), `?` (one byte except `/`), `[abc]`/`[a-z]`/`[!a]`
//! classes and `{a,b}` alternatives (expanded up front, nested allowed).
//! Patterns match the whole path; a leading `./` is ignored. Matching is a DP
//! over (pattern, input), so it is polynomial.

/// Whether `path` (relative, `/`-separated) matches `pattern`.
pub fn matches(pattern: &str, path: &str) -> bool {
    let path = path.strip_prefix("./").unwrap_or(path);
    expand_braces(pattern).iter().any(|p| {
        match_one(
            p.strip_prefix("./").unwrap_or(p).as_bytes(),
            path.as_bytes(),
        )
    })
}

/// Whether `s` contains glob syntax.
pub fn is_glob(s: &str) -> bool {
    s.bytes().any(|b| matches!(b, b'*' | b'?' | b'[' | b'{'))
}

/// The longest leading run of `pattern`'s `/`-segments without glob syntax
/// (`src/a/**/*.ts` → `src/a`), and the rest.
pub fn split_static_prefix(pattern: &str) -> (&str, &str) {
    let p = pattern.strip_prefix("./").unwrap_or(pattern);
    let mut end = 0;
    for (i, seg) in p.split('/').enumerate() {
        if is_glob(seg) {
            break;
        }
        let seg_end = if i == 0 {
            seg.len()
        } else {
            end + 1 + seg.len()
        };
        if seg_end >= p.len() {
            // The whole pattern is literal.
            return (p, "");
        }
        end = seg_end;
    }
    if end == 0 {
        ("", p)
    } else {
        (&p[..end], &p[end + 1..])
    }
}

/// Expands `{a,b}` alternatives (at most 256 results; the rest is dropped).
pub fn expand_braces(p: &str) -> Vec<String> {
    let b = p.as_bytes();
    let Some(open) = b.iter().position(|&c| c == b'{') else {
        return vec![p.to_string()];
    };
    let mut depth = 0;
    let mut close = None;
    let mut commas = Vec::new();
    for (i, &c) in b.iter().enumerate().skip(open) {
        match c {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(i);
                    break;
                }
            }
            b',' if depth == 1 => commas.push(i),
            _ => {}
        }
    }
    let Some(close) = close else {
        return vec![p.to_string()];
    };
    let mut out = Vec::new();
    let mut start = open + 1;
    let head = &p[..open];
    let tail = &p[close + 1..];
    for end in commas.iter().copied().chain([close]) {
        let alt = &p[start..end];
        for rest in expand_braces(&format!("{head}{alt}{tail}")) {
            if out.len() >= 256 {
                return out;
            }
            out.push(rest);
        }
        start = end + 1;
    }
    out
}

#[derive(Clone, Debug)]
enum Tok {
    Byte(u8),
    Any,
    Star,
    DoubleStar,
    DirsSlash,
    Class(Vec<(u8, u8)>, bool),
}

fn compile(p: &[u8]) -> Vec<Tok> {
    let mut toks = Vec::new();
    let mut i = 0;
    while i < p.len() {
        match p[i] {
            b'*' if p.get(i + 1) == Some(&b'*') => {
                if p.get(i + 2) == Some(&b'/') {
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
            b'[' => {
                let mut j = i + 1;
                let neg = matches!(p.get(j), Some(b'!' | b'^'));
                if neg {
                    j += 1;
                }
                let start = j;
                while j < p.len() && (p[j] != b']' || j == start) {
                    j += 1;
                }
                if j >= p.len() {
                    toks.push(Tok::Byte(b'['));
                    i += 1;
                    continue;
                }
                let body = &p[start..j];
                let mut ranges = Vec::new();
                let mut k = 0;
                while k < body.len() {
                    if k + 2 < body.len() && body[k + 1] == b'-' {
                        ranges.push((body[k], body[k + 2]));
                        k += 3;
                    } else {
                        ranges.push((body[k], body[k]));
                        k += 1;
                    }
                }
                toks.push(Tok::Class(ranges, neg));
                i = j + 1;
            }
            b'\\' if i + 1 < p.len() => {
                toks.push(Tok::Byte(p[i + 1]));
                i += 2;
            }
            c => {
                toks.push(Tok::Byte(c));
                i += 1;
            }
        }
    }
    toks
}

fn match_one(pattern: &[u8], s: &[u8]) -> bool {
    let toks = compile(pattern);
    // dp[j]: tokens[..i] match s[..j].
    let mut dp = vec![false; s.len() + 1];
    dp[0] = true;
    for t in &toks {
        let mut next = vec![false; s.len() + 1];
        match t {
            Tok::Star | Tok::DoubleStar => {
                let cross = matches!(t, Tok::DoubleStar);
                for j in 0..=s.len() {
                    if dp[j] || (j > 0 && next[j - 1] && (cross || s[j - 1] != b'/')) {
                        next[j] = true;
                    }
                }
            }
            Tok::DirsSlash => {
                // Empty, or any run ending in '/'.
                let mut reach = false;
                for j in 0..=s.len() {
                    if dp[j] {
                        next[j] = true;
                        reach = true;
                    }
                    if reach && j < s.len() && s[j] == b'/' {
                        next[j + 1] = true;
                    }
                }
            }
            _ => {
                for j in 0..s.len() {
                    if !dp[j] {
                        continue;
                    }
                    let c = s[j];
                    let ok = match t {
                        Tok::Byte(b) => *b == c,
                        Tok::Any => c != b'/',
                        Tok::Class(r, neg) => {
                            c != b'/' && (r.iter().any(|(a, b)| (*a..=*b).contains(&c)) != *neg)
                        }
                        _ => unreachable!(),
                    };
                    if ok {
                        next[j + 1] = true;
                    }
                }
            }
        }
        dp = next;
    }
    dp[s.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basics() {
        assert!(matches("src/**/*.ts", "src/a.ts"));
        assert!(matches("src/**/*.ts", "src/x/y/a.ts"));
        assert!(!matches("src/*.ts", "src/x/a.ts"));
        assert!(matches("**/*.{ts,tsx}", "a/b.tsx"));
        assert!(matches("dist/**", "dist/a/b.js"));
        assert!(matches("apps/*#build", "apps/web#build"));
        assert!(!matches("apps/*#build", "apps/web#test"));
        assert!(matches("[a-c]?.md", "b1.md"));
        assert!(matches("./a.txt", "a.txt"));
        assert!(matches("**", "x/y"));
    }

    #[test]
    fn prefixes() {
        assert_eq!(split_static_prefix("src/a/**/*.ts"), ("src/a", "**/*.ts"));
        assert_eq!(split_static_prefix("**/*.ts"), ("", "**/*.ts"));
        assert_eq!(split_static_prefix("src/index.ts"), ("src/index.ts", ""));
        assert_eq!(split_static_prefix("./src/*"), ("src", "*"));
    }
}
