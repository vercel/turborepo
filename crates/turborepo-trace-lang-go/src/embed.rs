//! `//go:embed` patterns (the `embed` package rules, as implemented by
//! `cmd/go/internal/load.resolveEmbed`).
//!
//! - A pattern is a `path.Match` glob relative to the package directory,
//!   optionally prefixed with `all:`. It must be a valid relative slash path:
//!   no `.`/`..`/empty elements, no leading or trailing `/`.
//! - A file matched directly is embedded, whatever its name.
//! - A directory matched is embedded recursively, except files and directories
//!   whose names begin with `.` or `_` (unless `all:`).
//! - Never embedded: names that version-control tools reserve (`.git`, `.hg`,
//!   `.svn`, `.bzr`, `CVS`), irregular files (symlinks, devices), and anything
//!   inside a directory with its own `go.mod` (another module).
//! - A pattern with no match, or a matched directory with nothing embeddable,
//!   is a build error.

use std::path::{Path, PathBuf};

use turborepo_trace_core::{DirCache, fs::EntryKind};

/// A parsed `//go:embed` pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbedPattern<'p> {
    /// The glob without `all:`.
    pub glob: &'p str,
    /// `all:` was given: include `.` and `_` files in matched directories.
    pub all: bool,
}

/// Why a pattern cannot be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmbedError {
    /// Not a valid pattern (syntax or a `.`/`..`/absolute path).
    Invalid(String),
    /// Nothing matches.
    NoMatch,
    /// A matched directory holds nothing embeddable.
    EmptyDir(PathBuf),
}

impl std::fmt::Display for EmbedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmbedError::Invalid(why) => write!(f, "invalid pattern: {why}"),
            EmbedError::NoMatch => f.write_str("no matching files found"),
            EmbedError::EmptyDir(d) => {
                write!(
                    f,
                    "cannot embed directory {}: contains no embeddable files",
                    d.display()
                )
            }
        }
    }
}

/// Parses and validates a pattern.
pub fn parse(pattern: &str) -> Result<EmbedPattern<'_>, EmbedError> {
    let (glob, all) = match pattern.strip_prefix("all:") {
        Some(g) => (g, true),
        None => (pattern, false),
    };
    if glob.is_empty() || glob == "." {
        return Err(EmbedError::Invalid("empty or \".\"".into()));
    }
    if glob.starts_with('/') || glob.ends_with('/') {
        return Err(EmbedError::Invalid("must be a relative path".into()));
    }
    for elem in glob.split('/') {
        if elem.is_empty() || elem == "." || elem == ".." {
            return Err(EmbedError::Invalid(format!("bad path element {elem:?}")));
        }
    }
    // Syntax check: `path.Match` reports ErrBadPattern for malformed classes and
    // escapes.
    for elem in glob.split('/') {
        if match_elem(elem, "").is_none() {
            return Err(EmbedError::Invalid(format!("malformed glob {elem:?}")));
        }
    }
    Ok(EmbedPattern { glob, all })
}

/// Whether a pattern element has `path.Match` metacharacters.
pub fn has_meta(s: &str) -> bool {
    s.bytes().any(|b| matches!(b, b'*' | b'?' | b'[' | b'\\'))
}

/// `path.Match` for one path element (no `/` in `name`). `None` for a malformed
/// pattern (Go's `ErrBadPattern`).
pub fn match_elem(pattern: &str, name: &str) -> Option<bool> {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    // Iterative glob matching with single-star backtracking (linear in practice).
    let (mut pi, mut ni) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    let mut syntax_ok = check_syntax(&p);
    loop {
        if pi < p.len() {
            match p[pi] {
                '*' => {
                    star = Some((pi, ni));
                    pi += 1;
                    continue;
                }
                '?' if ni < n.len() => {
                    pi += 1;
                    ni += 1;
                    continue;
                }
                '[' if ni < n.len() => match class(&p, pi, n[ni]) {
                    Some((true, next)) => {
                        pi = next;
                        ni += 1;
                        continue;
                    }
                    Some((false, _)) => {}
                    None => syntax_ok = false,
                },
                '\\' if pi + 1 < p.len() && ni < n.len() && p[pi + 1] == n[ni] => {
                    pi += 2;
                    ni += 1;
                    continue;
                }
                c if c != '\\' && c != '[' && c != '?' && ni < n.len() && c == n[ni] => {
                    pi += 1;
                    ni += 1;
                    continue;
                }
                _ => {}
            }
        } else if ni == n.len() {
            return syntax_ok.then_some(true);
        }
        // Mismatch: backtrack to the last star.
        match star {
            Some((sp, sn)) if sn < n.len() => {
                star = Some((sp, sn + 1));
                pi = sp + 1;
                ni = sn + 1;
            }
            _ => return syntax_ok.then_some(false),
        }
    }
}

/// Whether every `[...]` class and `\` escape in `p` is well formed.
fn check_syntax(p: &[char]) -> bool {
    let mut i = 0;
    while i < p.len() {
        match p[i] {
            '\\' => {
                if i + 1 >= p.len() {
                    return false;
                }
                i += 2;
            }
            '[' => match class_end(p, i) {
                Some(e) => i = e,
                None => return false,
            },
            _ => i += 1,
        }
    }
    true
}

/// A parsed `[...]` class: its ranges, whether it is negated, and the index
/// after its `]`.
type Class = (Vec<(char, char)>, bool, usize);

/// Parses the class at `start` (Go's `path.Match` rules). `None` if malformed.
fn parse_class(p: &[char], start: usize) -> Option<Class> {
    fn get_esc(p: &[char], i: &mut usize) -> Option<char> {
        let mut c = *p.get(*i)?;
        if c == '-' || c == ']' {
            return None;
        }
        if c == '\\' {
            *i += 1;
            c = *p.get(*i)?;
        }
        *i += 1;
        Some(c)
    }
    let mut i = start + 1;
    let negated = p.get(i) == Some(&'^');
    if negated {
        i += 1;
    }
    let mut ranges = Vec::new();
    loop {
        if p.get(i) == Some(&']') && !ranges.is_empty() {
            return Some((ranges, negated, i + 1));
        }
        let lo = get_esc(p, &mut i)?;
        let mut hi = lo;
        if p.get(i) == Some(&'-') {
            i += 1;
            hi = get_esc(p, &mut i)?;
        }
        ranges.push((lo, hi));
    }
}

/// Index after the `]` closing the class at `start`, if well formed.
fn class_end(p: &[char], start: usize) -> Option<usize> {
    parse_class(p, start).map(|(_, _, e)| e)
}

/// Matches `ch` against the class at `start`: `(matched, index after the
/// class)`, or `None` if malformed.
fn class(p: &[char], start: usize, ch: char) -> Option<(bool, usize)> {
    let (ranges, negated, end) = parse_class(p, start)?;
    let matched = ranges.iter().any(|&(lo, hi)| lo <= ch && ch <= hi);
    Some((matched != negated, end))
}

/// The `go/build` reserved names that are never embedded.
fn is_bad_name(name: &str) -> bool {
    matches!(name, ".bzr" | ".hg" | ".git" | ".svn" | "CVS")
}

/// The core-glob form of a pattern for membership records: `[…]` becomes `?`,
/// escapes are dropped. `*` and `?` keep their meaning (no `/`).
pub fn membership_glob(glob: &str) -> String {
    let mut out = String::with_capacity(glob.len());
    let mut it = glob.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '[' => {
                for d in it.by_ref() {
                    if d == ']' {
                        break;
                    }
                }
                out.push('?');
            }
            '\\' => out.extend(it.next()),
            c => out.push(c),
        }
    }
    out
}

/// The result of expanding one pattern.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Expansion {
    /// Embedded files, sorted, deduplicated.
    pub files: Vec<PathBuf>,
    /// Directories embedded recursively (each is a `(dir, "**")` membership).
    pub dirs: Vec<PathBuf>,
    /// Stopped at the limit.
    pub truncated: bool,
}

/// Expands pattern `p` in package directory `pkg_dir` through the dir cache.
/// Listings are read with [`DirCache::list`]; callers that record memberships
/// themselves should pause listing recording around this call.
pub fn expand(
    fs: &DirCache,
    pkg_dir: &Path,
    p: &EmbedPattern<'_>,
    limit: usize,
) -> Result<Expansion, EmbedError> {
    let mut out = Expansion::default();
    // Glob element by element.
    let mut cur: Vec<PathBuf> = vec![pkg_dir.to_path_buf()];
    let elems: Vec<&str> = p.glob.split('/').collect();
    for (k, elem) in elems.iter().enumerate() {
        let last = k + 1 == elems.len();
        let mut next = Vec::new();
        for dir in &cur {
            // A directory with its own go.mod belongs to another module (except the package
            // directory itself).
            if dir != pkg_dir && has_go_mod(fs, dir) {
                continue;
            }
            let Some(listing) = fs.list(dir) else {
                continue;
            };
            for e in listing.iter() {
                let Ok(name) = std::str::from_utf8(e.name_bytes()) else {
                    continue;
                };
                if match_elem(elem, name) != Some(true) {
                    continue;
                }
                let path = dir.join(name);
                if !last && e.kind() != EntryKind::Dir {
                    continue;
                }
                next.push((path, e.kind()));
            }
        }
        if last {
            for (path, kind) in next {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default();
                if is_bad_name(name) {
                    continue;
                }
                match kind {
                    EntryKind::File => out.files.push(path),
                    EntryKind::Dir => {
                        if has_go_mod(fs, &path) {
                            continue;
                        }
                        let before = out.files.len();
                        walk(fs, &path, p.all, limit, &mut out);
                        if out.files.len() == before && !out.truncated {
                            return Err(EmbedError::EmptyDir(path));
                        }
                        out.dirs.push(path);
                    }
                    // Symlinks and other irregular files cannot be embedded.
                    _ => {}
                }
                if out.files.len() >= limit {
                    out.truncated = true;
                    break;
                }
            }
        } else {
            cur = next.into_iter().map(|(p, _)| p).collect();
        }
    }
    if out.files.is_empty() && !out.truncated {
        return Err(EmbedError::NoMatch);
    }
    out.files.sort();
    out.files.dedup();
    out.dirs.sort();
    out.dirs.dedup();
    Ok(out)
}

fn has_go_mod(fs: &DirCache, dir: &Path) -> bool {
    fs.list(dir).is_some_and(|l| {
        l.get(b"go.mod")
            .is_some_and(|e| e.kind() == EntryKind::File)
    })
}

fn walk(fs: &DirCache, dir: &Path, all: bool, limit: usize, out: &mut Expansion) {
    let Some(listing) = fs.list(dir) else {
        return;
    };
    for e in listing.iter() {
        if out.files.len() >= limit {
            out.truncated = true;
            return;
        }
        let Ok(name) = std::str::from_utf8(e.name_bytes()) else {
            continue;
        };
        if is_bad_name(name) || (!all && (name.starts_with('.') || name.starts_with('_'))) {
            continue;
        }
        let path = dir.join(name);
        match e.kind() {
            EntryKind::File => out.files.push(path),
            EntryKind::Dir if !has_go_mod(fs, &path) => walk(fs, &path, all, limit, out),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_match() {
        let m = |p: &str, n: &str| match_elem(p, n).unwrap();
        assert!(m("*", "abc"));
        assert!(m("*", ".hidden"));
        assert!(m("a*c", "abbbc"));
        assert!(!m("a*c", "abd"));
        assert!(m("?.txt", "a.txt"));
        assert!(!m("?.txt", "ab.txt"));
        assert!(m("[abc].go", "b.go"));
        assert!(!m("[^abc].go", "b.go"));
        assert!(m("[a-c]x", "bx"));
        assert!(!m("[a-c]x", "dx"));
        assert!(m("\\*", "*"));
        assert!(!m("\\*", "a"));
        assert!(m("*.html", "index.html"));
        assert!(m("é*", "été"));
        assert!(m("*a*b*c*", "xxaxxbxxcxx"));
        assert!(match_elem("[", "a").is_none());
        assert!(match_elem("a\\", "a").is_none());
        assert!(match_elem("[]", "]").is_none());
        assert!(match_elem("[a-]", "a").is_none());
        assert!(m("[\\]]", "]"));
    }

    #[test]
    fn parse_patterns() {
        assert_eq!(
            parse("all:static").unwrap(),
            EmbedPattern {
                glob: "static",
                all: true
            }
        );
        for bad in ["", ".", "/abs", "a/../b", "a//b", "./a", "a/", "[x", "all:"] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
        assert!(parse("a/*.txt").is_ok());
        assert!(parse(".hidden").is_ok());
    }

    #[test]
    fn membership_globs() {
        assert_eq!(membership_glob("a/[xy]*.txt"), "a/?*.txt");
        assert_eq!(membership_glob("a\\*b"), "a*b");
    }

    fn tree(files: &[&str]) -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        for f in files {
            let p = t.path().join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "x").unwrap();
        }
        t
    }

    fn names(root: &Path, e: &Expansion) -> Vec<String> {
        e.files
            .iter()
            .map(|f| f.strip_prefix(root).unwrap().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn expansion_rules() {
        let t = tree(&[
            "static/a.css",
            "static/.hidden",
            "static/_skip/x",
            "static/sub/b.js",
            "static/.git/config",
            "static/nested/go.mod",
            "static/nested/c.txt",
            "tmpl/x.html",
            "tmpl/.y.html",
            "empty/.only",
            "z.txt",
        ]);
        let fs = DirCache::new();
        let root = t.path();
        let e = expand(&fs, root, &parse("static").unwrap(), 100).unwrap();
        assert_eq!(names(root, &e), ["static/a.css", "static/sub/b.js"]);
        assert_eq!(e.dirs, [root.join("static")]);
        let e = expand(&fs, root, &parse("all:static").unwrap(), 100).unwrap();
        assert_eq!(
            names(root, &e),
            [
                "static/.hidden",
                "static/_skip/x",
                "static/a.css",
                "static/sub/b.js"
            ]
        );
        // `*` matches dot files directly (`image/*` embeds `image/.tempfile`).
        let e = expand(&fs, root, &parse("tmpl/*").unwrap(), 100).unwrap();
        assert_eq!(names(root, &e), ["tmpl/.y.html", "tmpl/x.html"]);
        let e = expand(&fs, root, &parse("*.txt").unwrap(), 100).unwrap();
        assert_eq!(names(root, &e), ["z.txt"]);
        assert_eq!(
            expand(&fs, root, &parse("nope/*").unwrap(), 100),
            Err(EmbedError::NoMatch)
        );
        assert!(matches!(
            expand(&fs, root, &parse("empty").unwrap(), 100),
            Err(EmbedError::EmptyDir(_))
        ));
        // Another module's files are never embedded.
        assert_eq!(
            expand(&fs, root, &parse("static/nested/c.txt").unwrap(), 100),
            Err(EmbedError::NoMatch)
        );
        let e = expand(&fs, root, &parse("static").unwrap(), 1).unwrap();
        assert!(e.truncated);
        assert_eq!(e.files.len(), 1);
    }
}
