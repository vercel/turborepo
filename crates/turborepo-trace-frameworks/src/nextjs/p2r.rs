//! A path-to-regexp v6 subset, as Next.js uses it for `redirects`, `rewrites`,
//! `middleware` matchers and microfrontends paths.
//!
//! Supported: literal text, `:name`, `:name(regex)`, unnamed `(regex)` groups,
//! the modifiers `?`, `*`, `+` (a preceding `/` or `.` is the optional prefix),
//! `{...}` groups with a modifier, and `\\` escapes. Matching is
//! case-insensitive and strict (no optional trailing slash), like Next. Custom
//! regexes the Rust engine cannot run (lookaround, backreferences)
//! make the pattern unsupported.

use regex::Regex;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    Text(String),
    Param {
        name: String,
        prefix: String,
        suffix: String,
        pattern: String,
        modifier: Option<char>,
    },
}

/// A compiled pattern.
#[derive(Clone, Debug)]
pub struct PathPattern {
    source: String,
    toks: Vec<Tok>,
    re: Regex,
    names: Vec<String>,
}

/// Default parameter pattern.
const DEFAULT_PAT: &str = "[^/#?]+?";

fn parse(src: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut out: Vec<Tok> = Vec::new();
    let mut text = String::new();
    let mut unnamed = 0usize;
    let read_name = |i: &mut usize| -> String {
        let mut n = String::new();
        while *i < chars.len() && (chars[*i].is_alphanumeric() || chars[*i] == '_') {
            n.push(chars[*i]);
            *i += 1;
        }
        n
    };
    let read_pattern = |i: &mut usize| -> Result<String, String> {
        // at '('
        let mut depth = 1;
        let mut p = String::new();
        *i += 1;
        while *i < chars.len() {
            let c = chars[*i];
            if c == '\\' && *i + 1 < chars.len() {
                p.push(c);
                p.push(chars[*i + 1]);
                *i += 2;
                continue;
            }
            if c == ')' {
                depth -= 1;
                if depth == 0 {
                    *i += 1;
                    return Ok(p);
                }
            } else if c == '(' {
                depth += 1;
                if chars.get(*i + 1) != Some(&'?') {
                    return Err("capturing groups are not allowed in a parameter pattern".into());
                }
            }
            p.push(c);
            *i += 1;
        }
        Err("unbalanced pattern".into())
    };
    let modifier_at = |i: &mut usize| -> Option<char> {
        match chars.get(*i) {
            Some(&c @ ('?' | '*' | '+')) => {
                *i += 1;
                Some(c)
            }
            _ => None,
        }
    };
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' if i + 1 < chars.len() => {
                text.push(chars[i + 1]);
                i += 2;
            }
            ':' | '(' => {
                let (name, pattern) = if c == ':' {
                    i += 1;
                    let n = read_name(&mut i);
                    if n.is_empty() {
                        return Err(format!("missing parameter name at {i}"));
                    }
                    let p = if chars.get(i) == Some(&'(') {
                        read_pattern(&mut i)?
                    } else {
                        DEFAULT_PAT.to_string()
                    };
                    (n, p)
                } else {
                    let p = read_pattern(&mut i)?;
                    let n = unnamed.to_string();
                    unnamed += 1;
                    (n, p)
                };
                // The prefix: a trailing `/` or `.` of the pending text.
                let mut prefix = String::new();
                if let Some(last) = text.chars().last()
                    && (last == '/' || last == '.')
                {
                    prefix.push(last);
                    text.pop();
                }
                if !text.is_empty() {
                    out.push(Tok::Text(std::mem::take(&mut text)));
                }
                let modifier = modifier_at(&mut i);
                out.push(Tok::Param {
                    name,
                    prefix,
                    suffix: String::new(),
                    pattern,
                    modifier,
                });
            }
            '{' => {
                if !text.is_empty() {
                    out.push(Tok::Text(std::mem::take(&mut text)));
                }
                i += 1;
                let mut prefix = String::new();
                while i < chars.len() && chars[i] != ':' && chars[i] != '(' && chars[i] != '}' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        i += 1;
                    }
                    prefix.push(chars[i]);
                    i += 1;
                }
                let mut name = String::new();
                let mut pattern = String::new();
                if chars.get(i) == Some(&':') {
                    i += 1;
                    name = read_name(&mut i);
                    pattern = DEFAULT_PAT.into();
                    if chars.get(i) == Some(&'(') {
                        pattern = read_pattern(&mut i)?;
                    }
                } else if chars.get(i) == Some(&'(') {
                    pattern = read_pattern(&mut i)?;
                    name = unnamed.to_string();
                    unnamed += 1;
                }
                let mut suffix = String::new();
                while i < chars.len() && chars[i] != '}' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        i += 1;
                    }
                    suffix.push(chars[i]);
                    i += 1;
                }
                if chars.get(i) != Some(&'}') {
                    return Err("unbalanced {".into());
                }
                i += 1;
                let modifier = modifier_at(&mut i);
                if name.is_empty() {
                    // A plain optional group of text.
                    let t = format!("{prefix}{suffix}");
                    out.push(Tok::Param {
                        name: String::new(),
                        prefix: t,
                        suffix: String::new(),
                        pattern: String::new(),
                        modifier,
                    });
                } else {
                    out.push(Tok::Param {
                        name,
                        prefix,
                        suffix,
                        pattern,
                        modifier,
                    });
                }
            }
            _ => {
                text.push(c);
                i += 1;
            }
        }
    }
    if !text.is_empty() {
        out.push(Tok::Text(text));
    }
    Ok(out)
}

fn to_regex(toks: &[Tok]) -> (String, Vec<String>) {
    let mut re = String::from("(?i)^");
    let mut names = Vec::new();
    for t in toks {
        match t {
            Tok::Text(s) => re.push_str(&regex::escape(s)),
            Tok::Param {
                name,
                prefix,
                suffix,
                pattern,
                modifier,
            } => {
                let pre = regex::escape(prefix);
                let suf = regex::escape(suffix);
                if name.is_empty() {
                    re.push_str(&format!("(?:{pre}{suf})"));
                    if let Some(m) = modifier {
                        re.push(*m);
                    }
                    continue;
                }
                names.push(name.clone());
                match modifier {
                    Some('+') | Some('*') if !prefix.is_empty() || !suffix.is_empty() => {
                        let m = if *modifier == Some('*') { "?" } else { "" };
                        re.push_str(&format!(
                            "(?:{pre}((?:{pattern})(?:{suf}{pre}(?:{pattern}))*){suf}){m}"
                        ));
                    }
                    Some(m @ ('+' | '*')) => re.push_str(&format!("((?:{pattern}){m})")),
                    Some('?') => re.push_str(&format!("(?:{pre}({pattern}){suf})?")),
                    _ => re.push_str(&format!("{pre}({pattern}){suf}")),
                }
            }
        }
    }
    re.push('$');
    (re, names)
}

impl PathPattern {
    /// Compiles `source`.
    pub fn new(source: &str) -> Result<PathPattern, String> {
        let toks = parse(source)?;
        let (re, names) = to_regex(&toks);
        let re = Regex::new(&re).map_err(|e| format!("unsupported pattern: {e}"))?;
        Ok(PathPattern {
            source: source.to_string(),
            toks,
            re,
            names,
        })
    }

    /// The source.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Parameter names.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Whether the pattern matches `path` (no parameters are extracted).
    pub fn is_match(&self, path: &str) -> bool {
        self.re.is_match(path)
    }

    /// Matches `path`; returns the parameter values (in name order; `None` if
    /// absent).
    pub fn matches(&self, path: &str) -> Option<Vec<(String, Option<String>)>> {
        let c = self.re.captures(path)?;
        Some(
            self.names
                .iter()
                .enumerate()
                .map(|(i, n)| (n.clone(), c.get(i + 1).map(|m| m.as_str().to_string())))
                .collect(),
        )
    }

    /// Whether the pattern has a literal first segment (`/docs/...`), as text.
    pub fn literal_prefix(&self) -> String {
        match self.toks.first() {
            Some(Tok::Text(t)) => t.clone(),
            _ => String::new(),
        }
    }
}

/// Fills `:name` parameters of a destination (path-to-regexp `compile`, without
/// encoding). Parameters that are absent compile to nothing; repeated ones are
/// joined with `/` (they were captured as a `/`-joined string already). Returns
/// `None` if a parameter the destination needs is unknown.
pub fn compile_destination(dest: &str, params: &[(String, Option<String>)]) -> Option<String> {
    let mut out = String::new();
    let chars: Vec<char> = dest.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == ':'
            && chars
                .get(i + 1)
                .is_some_and(|c| c.is_alphanumeric() || *c == '_')
        {
            // Skip `://` of a scheme.
            let mut j = i + 1;
            let mut name = String::new();
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                name.push(chars[j]);
                j += 1;
            }
            if chars.get(j) == Some(&'(') {
                let mut d = 0;
                while j < chars.len() {
                    if chars[j] == '(' {
                        d += 1;
                    } else if chars[j] == ')' {
                        d -= 1;
                        if d == 0 {
                            j += 1;
                            break;
                        }
                    }
                    j += 1;
                }
            }
            let optional = matches!(chars.get(j), Some('?' | '*'));
            if matches!(chars.get(j), Some('?' | '*' | '+')) {
                j += 1;
            }
            match params.iter().find(|(n, _)| *n == name) {
                Some((_, Some(v))) => out.push_str(v),
                Some((_, None)) => {
                    if out.ends_with('/') && optional {
                        out.pop();
                    }
                }
                None => {
                    // Not a source parameter (for example a port `:3000`): keep the text.
                    if name.chars().all(|c| c.is_ascii_digit()) {
                        out.push(':');
                        out.push_str(&name);
                    } else {
                        return None;
                    }
                }
            }
            i = j;
            continue;
        }
        out.push(c);
        i += 1;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(p: &str, s: &str) -> Option<Vec<(String, Option<String>)>> {
        PathPattern::new(p).unwrap().matches(s)
    }

    #[test]
    fn basics() {
        assert!(m("/blog/:slug", "/blog/hello").is_some());
        assert!(m("/blog/:slug", "/blog/hello/x").is_none());
        assert!(m("/blog/:slug", "/Blog/x").is_some());
        assert_eq!(
            m("/docs/:path*", "/docs/a/b").unwrap()[0].1.as_deref(),
            Some("a/b")
        );
        assert!(m("/docs/:path*", "/docs").is_some());
        assert!(m("/docs/:path+", "/docs").is_none());
        assert!(m("/post/:id(\\d{1,})", "/post/12").is_some());
        assert!(m("/post/:id(\\d{1,})", "/post/ab").is_none());
        assert!(m("/a/:x?", "/a").is_some());
        assert!(m("/(.*)", "/anything/here").is_some());
        assert!(m("/old{/:rest}?", "/old").is_some());
        assert!(m("/old{/:rest}?", "/old/x").is_some());
        assert!(PathPattern::new("/x/:a((?!b).*)").is_err());
    }

    #[test]
    fn destination() {
        let p = PathPattern::new("/docs/:path*").unwrap();
        let ps = p.matches("/docs/a/b").unwrap();
        assert_eq!(
            compile_destination("/new-docs/:path*", &ps).as_deref(),
            Some("/new-docs/a/b")
        );
        let ps = p.matches("/docs").unwrap();
        assert_eq!(
            compile_destination("/new-docs/:path*", &ps).as_deref(),
            Some("/new-docs")
        );
        assert_eq!(
            compile_destination("https://example.com/:path*", &ps).as_deref(),
            Some("https://example.com")
        );
        assert_eq!(compile_destination("/x/:other", &ps), None);
        assert_eq!(
            compile_destination("http://localhost:3000/y", &ps).as_deref(),
            Some("http://localhost:3000/y")
        );
    }
}
