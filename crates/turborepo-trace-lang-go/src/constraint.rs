//! Build constraints: `//go:build` expressions, legacy `// +build` lines,
//! file-name `_GOOS_GOARCH` suffixes, and the [`BuildContext`] they are
//! evaluated against (the target platform from
//! [`turborepo_trace_core::Platform`], `-tags`, cgo, the Go release).
//!
//! Header rules follow `go/build`'s `parseFileHeader`: constraint comments
//! count only in the leading comment block, before the first line of code.
//! `//go:build` lines may appear anywhere in that block; `// +build` lines only
//! before the last blank line preceding the code, and they are ignored when a
//! `//go:build` line exists (Go 1.17+ semantics).

use turborepo_trace_core::Platform;

/// The newest Go release whose `go1.N` release tags are satisfied by default.
pub const LATEST_GO_MINOR: u32 = 27;

/// What build constraints are evaluated against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildContext {
    /// Target `GOOS`/`GOARCH`.
    pub platform: Platform,
    /// Extra tags (`-tags`).
    pub tags: Vec<String>,
    /// `CGO_ENABLED`: satisfies the `cgo` tag and keeps files that `import
    /// "C"`.
    pub cgo: bool,
    /// The Go release minor version: tags `go1.1` … `go1.<go_minor>` are
    /// satisfied.
    pub go_minor: u32,
    /// Include files for every target, flagged as conditional, instead of
    /// filtering.
    pub all_targets: bool,
}

impl BuildContext {
    /// A context for `platform` with cgo on, no extra tags and the latest Go
    /// release.
    pub fn new(platform: Platform) -> Self {
        BuildContext {
            platform,
            tags: Vec::new(),
            cgo: true,
            go_minor: LATEST_GO_MINOR,
            all_targets: false,
        }
    }

    /// Whether `tag` is satisfied (`go/build`'s `matchTag`).
    pub fn tag(&self, tag: &str) -> bool {
        if self.tags.iter().any(|t| t == tag) {
            return true;
        }
        if let Some(m) = self.platform.go_tag_matches(tag) {
            return m;
        }
        match tag {
            "cgo" => self.cgo,
            "gc" => true,
            "gccgo" => false,
            _ => tag
                .strip_prefix("go1.")
                .and_then(|n| n.parse::<u32>().ok())
                .is_some_and(|n| n <= self.go_minor),
        }
    }

    /// Whether a file name passes the `_GOOS`/`_GOARCH` suffix rule.
    pub fn file_name_ok(&self, name: &str) -> bool {
        self.platform.go_file_matches(name)
    }
}

/// A build-constraint expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    /// A tag.
    Tag(String),
    /// `!x`.
    Not(Box<Expr>),
    /// `x && y`.
    And(Box<Expr>, Box<Expr>),
    /// `x || y`.
    Or(Box<Expr>, Box<Expr>),
}

impl Expr {
    /// Evaluates with `tag` deciding each tag.
    pub fn eval(&self, tag: &mut impl FnMut(&str) -> bool) -> bool {
        match self {
            Expr::Tag(t) => tag(t),
            Expr::Not(x) => !x.eval(tag),
            Expr::And(a, b) => a.eval(tag) && b.eval(tag),
            Expr::Or(a, b) => a.eval(tag) || b.eval(tag),
        }
    }

    /// Evaluates against a build context.
    pub fn matches(&self, cx: &BuildContext) -> bool {
        self.eval(&mut |t| cx.tag(t))
    }
}

/// Why a constraint expression is malformed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxError(pub String);

impl std::fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Maximum nesting depth of a `//go:build` expression (Go uses 1000; deeper
/// input is an error, not a stack overflow).
const MAX_DEPTH: usize = 100;

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    depth: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\r') {
            self.i += 1;
        }
    }

    fn eat(&mut self, tok: &[u8]) -> bool {
        self.ws();
        if self.s[self.i..].starts_with(tok) {
            self.i += tok.len();
            true
        } else {
            false
        }
    }

    fn or(&mut self) -> Result<Expr, SyntaxError> {
        let mut x = self.and()?;
        while self.eat(b"||") {
            x = Expr::Or(Box::new(x), Box::new(self.and()?));
        }
        Ok(x)
    }

    fn and(&mut self) -> Result<Expr, SyntaxError> {
        let mut x = self.not()?;
        while self.eat(b"&&") {
            x = Expr::And(Box::new(x), Box::new(self.not()?));
        }
        Ok(x)
    }

    fn not(&mut self) -> Result<Expr, SyntaxError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(SyntaxError("expression nests too deeply".into()));
        }
        let r = if self.eat(b"!") {
            self.not().map(|x| Expr::Not(Box::new(x)))
        } else {
            self.atom()
        };
        self.depth -= 1;
        r
    }

    fn atom(&mut self) -> Result<Expr, SyntaxError> {
        if self.eat(b"(") {
            let x = self.or()?;
            if !self.eat(b")") {
                return Err(SyntaxError("missing )".into()));
            }
            return Ok(x);
        }
        self.ws();
        let start = self.i;
        while self.i < self.s.len()
            && (self.s[self.i].is_ascii_alphanumeric() || matches!(self.s[self.i], b'_' | b'.'))
        {
            self.i += 1;
        }
        if start == self.i {
            return Err(SyntaxError(if self.i >= self.s.len() {
                "unexpected end of expression".into()
            } else {
                format!("unexpected {:?}", self.s[self.i] as char)
            }));
        }
        Ok(Expr::Tag(
            String::from_utf8_lossy(&self.s[start..self.i]).into_owned(),
        ))
    }
}

/// Parses the expression of a `//go:build` line (the text after `//go:build`).
pub fn parse_expr(s: &str) -> Result<Expr, SyntaxError> {
    let mut p = Parser {
        s: s.as_bytes(),
        i: 0,
        depth: 0,
    };
    let x = p.or()?;
    p.ws();
    if p.i != p.s.len() {
        return Err(SyntaxError(format!(
            "unexpected {:?}",
            String::from_utf8_lossy(&p.s[p.i..])
        )));
    }
    Ok(x)
}

/// Parses one `// +build` line's arguments: space-separated options (OR) of
/// comma-separated terms (AND), each `tag` or `!tag`.
pub fn parse_plus_build(args: &str) -> Result<Expr, SyntaxError> {
    let mut or: Option<Expr> = None;
    for opt in args.split_ascii_whitespace() {
        let mut and: Option<Expr> = None;
        for term in opt.split(',') {
            let (neg, tag) = match term.strip_prefix('!') {
                Some(t) => (true, t),
                None => (false, term),
            };
            if tag.is_empty()
                || tag.starts_with('!')
                || !tag
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.')
            {
                return Err(SyntaxError(format!("bad +build term {term:?}")));
            }
            let mut x = Expr::Tag(tag.to_owned());
            if neg {
                x = Expr::Not(Box::new(x));
            }
            and = Some(match and {
                None => x,
                Some(a) => Expr::And(Box::new(a), Box::new(x)),
            });
        }
        if let Some(a) = and {
            or = Some(match or {
                None => a,
                Some(o) => Expr::Or(Box::new(o), Box::new(a)),
            });
        }
    }
    or.ok_or_else(|| SyntaxError("empty +build line".into()))
}

/// The constraint comments of a file header.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeaderConstraints {
    /// The text after `//go:build`, with its byte range in the file.
    pub go_build: Option<(String, std::ops::Range<usize>)>,
    /// Arguments of the `// +build` lines that count.
    pub plus_build: Vec<String>,
    /// Byte offset of the first line of code (the header's end).
    pub code_start: usize,
    /// More than one `//go:build` line (a Go error); the first is used.
    pub multiple_go_build: bool,
}

impl HeaderConstraints {
    /// Whether the header has any constraint.
    pub fn is_constrained(&self) -> bool {
        self.go_build.is_some() || !self.plus_build.is_empty()
    }

    /// The effective expression: `//go:build` if present, else the AND of the
    /// `+build` lines. `Ok(None)` when unconstrained.
    pub fn expr(&self) -> Result<Option<Expr>, SyntaxError> {
        if let Some((s, _)) = &self.go_build {
            return parse_expr(s).map(Some);
        }
        let mut all: Option<Expr> = None;
        for l in &self.plus_build {
            let x = parse_plus_build(l)?;
            all = Some(match all {
                None => x,
                Some(a) => Expr::And(Box::new(a), Box::new(x)),
            });
        }
        Ok(all)
    }

    /// Display form of the constraint for reports.
    pub fn display(&self) -> String {
        match &self.go_build {
            Some((s, _)) => s.trim().to_owned(),
            None => self
                .plus_build
                .iter()
                .map(|l| format!("+build {l}"))
                .collect::<Vec<_>>()
                .join("; "),
        }
    }

    /// Whether the file builds in `cx`. A malformed constraint counts as
    /// satisfied (Go reports an error; over-approximating keeps the file).
    pub fn matches(&self, cx: &BuildContext) -> bool {
        match self.expr() {
            Ok(Some(x)) => x.matches(cx),
            Ok(None) | Err(_) => true,
        }
    }
}

fn is_go_build_line(line: &[u8]) -> bool {
    line.strip_prefix(b"//go:build")
        .is_some_and(|r| r.is_empty() || matches!(r[0], b' ' | b'\t'))
}

/// Reads the constraint comments of a file header (`go/build`'s
/// `parseFileHeader`), for Go, C and assembly files alike.
pub fn header_constraints(src: &[u8]) -> HeaderConstraints {
    let mut out = HeaderConstraints::default();
    let mut end = 0usize; // offset after the most recent blank line in the header
    let mut in_slash_star = false;
    let mut pos = 0usize;
    let mut go_build_seen = false;
    let mut code_start = src.len();
    // `+build` candidates with their line offsets; only those before `end` count.
    let mut plus: Vec<(usize, String)> = Vec::new();
    'lines: while pos < src.len() {
        let line_start = pos;
        let nl = memchr::memchr(b'\n', &src[pos..]).map_or(src.len(), |i| pos + i);
        pos = (nl + 1).min(src.len());
        let raw = &src[line_start..nl];
        let line = raw.trim_ascii();
        if line.is_empty() && !in_slash_star {
            end = line_start;
            continue;
        }
        if !in_slash_star && is_go_build_line(line) {
            let off = line_start + (raw.len() - raw.trim_ascii_start().len());
            let text = String::from_utf8_lossy(&line[b"//go:build".len()..]).into_owned();
            if go_build_seen {
                out.multiple_go_build = true;
            } else {
                go_build_seen = true;
                out.go_build = Some((text, off..off + line.len()));
            }
        }
        if !in_slash_star && let Some(rest) = line.strip_prefix(b"//") {
            let rest = rest.trim_ascii();
            if let Some(args) = rest.strip_prefix(b"+build")
                && (args.is_empty() || matches!(args[0], b' ' | b'\t'))
            {
                plus.push((line_start, String::from_utf8_lossy(args).trim().to_owned()));
            }
        }
        // Skip comments; stop at the first code.
        let mut l = line;
        while !l.is_empty() {
            if in_slash_star {
                match memchr::memmem::find(l, b"*/") {
                    Some(i) => {
                        in_slash_star = false;
                        l = l[i + 2..].trim_ascii();
                        continue;
                    }
                    None => continue 'lines,
                }
            }
            if l.starts_with(b"//") {
                continue 'lines;
            }
            if l.starts_with(b"/*") {
                in_slash_star = true;
                l = l[2..].trim_ascii();
                continue;
            }
            code_start = line_start;
            break 'lines;
        }
    }
    out.code_start = code_start.min(src.len());
    if !go_build_seen {
        out.plus_build = plus
            .into_iter()
            .filter(|(off, _)| *off < end)
            .map(|(_, a)| a)
            .collect();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cx(target: &str) -> BuildContext {
        BuildContext::new(Platform::parse(target).unwrap())
    }

    fn eval(expr: &str, target: &str) -> bool {
        parse_expr(expr).unwrap().matches(&cx(target))
    }

    #[test]
    fn expressions() {
        assert!(eval("linux", "linux/amd64"));
        assert!(!eval("linux", "darwin/arm64"));
        assert!(eval("linux && amd64", "linux/amd64"));
        assert!(!eval("linux && !amd64", "linux/amd64"));
        assert!(eval("darwin || (linux && arm64)", "linux/arm64"));
        assert!(eval("!windows", "darwin/arm64"));
        assert!(eval("unix", "darwin/arm64"));
        assert!(!eval("unix", "windows/amd64"));
        assert!(eval("cgo", "linux/amd64"));
        assert!(eval("go1.18 && !go1.99", "linux/amd64"));
        assert!(!eval("ignore", "linux/amd64"));
        assert!(eval("gc && !gccgo", "linux/amd64"));
        assert!(!eval("android", "linux/amd64"));
        // Precedence: && binds tighter than ||.
        assert!(eval("windows && 386 || linux", "linux/amd64"));
        assert!(!eval("!(linux || darwin)", "linux/amd64"));
        let mut c = cx("linux/amd64");
        c.tags.push("integration".into());
        c.cgo = false;
        assert!(parse_expr("integration && !cgo").unwrap().matches(&c));
    }

    #[test]
    fn expression_errors() {
        for bad in [
            "",
            "linux &&",
            "(linux",
            "linux)",
            "a b",
            "!",
            "a || || b",
            "a & b",
        ] {
            assert!(parse_expr(bad).is_err(), "{bad:?}");
        }
        let deep = format!("{}a{}", "(".repeat(500), ")".repeat(500));
        assert!(parse_expr(&deep).is_err());
        let nots = format!("{}a", "!".repeat(500));
        assert!(parse_expr(&nots).is_err());
    }

    #[test]
    fn plus_build_lines() {
        let x = parse_plus_build("linux,amd64 darwin,!cgo").unwrap();
        assert!(x.matches(&cx("linux/amd64")));
        assert!(!x.matches(&cx("darwin/arm64")));
        assert!(!x.matches(&cx("linux/arm64")));
        assert!(parse_plus_build("").is_err());
        assert!(parse_plus_build("!!a").is_err());
    }

    #[test]
    fn headers() {
        let h = header_constraints(b"// Copyright\n\n//go:build linux && !cgo\n\npackage x\n");
        assert_eq!(h.go_build.as_ref().unwrap().0.trim(), "linux && !cgo");
        assert!(!h.matches(&cx("linux/amd64")));
        // After code, nothing counts.
        let h = header_constraints(b"package x\n//go:build ignore\n");
        assert!(!h.is_constrained());
        // `+build` must be followed by a blank line before the code.
        let h = header_constraints(b"// +build linux\npackage x\n");
        assert!(h.plus_build.is_empty());
        let h = header_constraints(b"// +build linux\n// +build amd64\n\npackage x\n");
        assert_eq!(h.plus_build, ["linux", "amd64"]);
        assert!(!h.matches(&cx("linux/arm64")));
        // `//go:build` wins over `+build`.
        let h = header_constraints(b"//go:build darwin\n// +build linux\n\npackage x\n");
        assert!(h.plus_build.is_empty());
        assert!(!h.matches(&cx("linux/amd64")));
        // Inside a block comment it does not count.
        let h = header_constraints(b"/*\n//go:build ignore\n*/\npackage x\n");
        assert!(!h.is_constrained());
        // Block comments are part of the header.
        let h = header_constraints(b"/* lic */\n//go:build windows\npackage x\n");
        assert!(h.go_build.is_some());
        // `//go:buildx` is not a directive.
        assert!(!header_constraints(b"//go:buildx a\npackage x\n").is_constrained());
        // Assembly header.
        let h = header_constraints(b"//go:build amd64 && !purego\n\n#include \"textflag.h\"\n");
        assert!(h.matches(&cx("linux/amd64")));
        assert!(!h.matches(&cx("linux/arm64")));
        let h = header_constraints(b"//go:build a\n//go:build b\npackage x");
        assert!(h.multiple_go_build);
        assert!(header_constraints(b"").expr().unwrap().is_none());
    }

    #[test]
    fn file_names() {
        let c = cx("linux/amd64");
        assert!(c.file_name_ok("a_linux.go"));
        assert!(c.file_name_ok("a_linux_amd64.go"));
        assert!(!c.file_name_ok("a_linux_arm64.go"));
        assert!(!c.file_name_ok("a_windows.go"));
        assert!(c.file_name_ok("linux.go"));
        assert!(!c.file_name_ok("a_darwin_test.go"));
        assert!(c.file_name_ok("a_amd64.s"));
        assert!(!c.file_name_ok("a_arm64.s"));
    }
}
