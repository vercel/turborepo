use std::path::{Path, PathBuf};

use crate::{
    Part, Resolution, ResolveCx, Span, Target, Uncertainty, UncertaintyKind,
    eval::PatPart,
    fs::{DirCache, EntryKind, FileKind, LinkInfo, normalize},
    os::OsStrExt,
};

/// Visiting more than `limit * VISIT_BUDGET_FACTOR` directory entries without
/// filling the match limit also truncates, so a pattern that matches little in
/// a huge tree stays bounded.
const VISIT_BUDGET_FACTOR: usize = 100;

/// Why a pattern was not expanded.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Refusal {
    /// The pattern starts with a wildcard, so there is no directory to anchor
    /// it.
    LeadingWildcard,
    /// The pattern's static directory is the filesystem root.
    RootDirectory,
}

/// The result of [`expand`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expansion {
    /// The pattern was expanded.
    Matches {
        /// The absolute glob, with `*` for each wildcard.
        pattern: String,
        /// Matching files, sorted by path bytes.
        matches: Vec<PathBuf>,
        /// Whether expansion stopped at the limit (match count or visit
        /// budget).
        truncated: bool,
    },
    /// The pattern was not expanded.
    Refused(Refusal),
}

/// Pattern token: a literal byte or a wildcard that matches any run of bytes,
/// including `/`.
///
/// Wildcards cross directory separators on purpose: an unknown expression may
/// contain slashes, and over-approximating is the guiding principle.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Tok {
    Byte(u8),
    Star,
}

/// NFA over [`Tok`]s; states are positions `0..=toks.len()`.
struct Matcher {
    toks: Vec<Tok>,
}

impl Matcher {
    fn new(toks: Vec<Tok>) -> Self {
        Matcher { toks }
    }

    fn start(&self) -> Vec<bool> {
        let mut s = vec![false; self.toks.len() + 1];
        s[0] = true;
        self.close(&mut s);
        s
    }

    /// Epsilon closure: a star may match nothing.
    fn close(&self, s: &mut [bool]) {
        for i in 0..self.toks.len() {
            if s[i] && self.toks[i] == Tok::Star {
                s[i + 1] = true;
            }
        }
    }

    fn step(&self, s: &[bool], input: &[u8]) -> Vec<bool> {
        let mut cur = s.to_vec();
        let mut next = vec![false; cur.len()];
        for &b in input {
            next.iter_mut().for_each(|x| *x = false);
            for i in 0..self.toks.len() {
                if !cur[i] {
                    continue;
                }
                match self.toks[i] {
                    Tok::Byte(c) if c == b => next[i + 1] = true,
                    Tok::Byte(_) => {}
                    Tok::Star => next[i] = true,
                }
            }
            self.close(&mut next);
            std::mem::swap(&mut cur, &mut next);
            if !cur.iter().any(|&x| x) {
                break;
            }
        }
        cur
    }

    fn accepts(&self, s: &[bool]) -> bool {
        s[self.toks.len()]
    }

    fn alive(s: &[bool]) -> bool {
        s.iter().any(|&x| x)
    }
}

/// Expands `parts` against the filesystem, anchored at `base_dir` unless the
/// pattern is absolute.
///
/// The longest literal prefix up to its last `/` picks the directory to walk.
/// Below it, only subdirectories that can still lead to a match are entered,
/// and only files are returned. Symlinks to files match; symlinked directories
/// are not entered (avoids cycles).
///
/// A pattern starting with a wildcard is never expanded
/// ([`Refusal::LeadingWildcard`]), nor is one anchored at `/`
/// ([`Refusal::RootDirectory`]). At most `limit` matches are returned;
/// `truncated` reports whether more existed or the visit budget ran out.
pub fn expand(parts: &[PatPart], base_dir: &Path, fs: &DirCache, limit: usize) -> Expansion {
    expand_with(parts, base_dir, fs, &ExpandOptions::new(limit))
}

/// Options for [`expand_with`].
#[derive(Copy, Clone, Debug)]
pub struct ExpandOptions<'a> {
    /// Maximum number of matches.
    pub limit: usize,
    /// Directory names never entered below the static root, such as
    /// `node_modules` (nft globs ignore `<root>/**/node_modules/**`). The
    /// static root itself may contain them.
    pub skip_dirs: &'a [&'a str],
    /// Files for which this returns `false` are not matched (and do not count
    /// toward the limit), for example nft's excluded asset extensions.
    pub keep_file: Option<fn(&Path) -> bool>,
}

impl ExpandOptions<'_> {
    /// Only a match limit.
    pub const fn new(limit: usize) -> Self {
        ExpandOptions {
            limit,
            skip_dirs: &[],
            keep_file: None,
        }
    }
}

/// [`expand`] with [`ExpandOptions`].
pub fn expand_with(
    parts: &[PatPart],
    base_dir: &Path,
    fs: &DirCache,
    opts: &ExpandOptions<'_>,
) -> Expansion {
    let limit = opts.limit;
    let Some(PatPart::Lit(first)) = parts.first() else {
        return match parts.first() {
            None => Expansion::Matches {
                pattern: String::new(),
                matches: Vec::new(),
                truncated: false,
            },
            Some(_) => Expansion::Refused(Refusal::LeadingWildcard),
        };
    };
    // Split the first literal into the static directory and the start of the
    // relative pattern.
    let (dir_part, rest_first) = match first.rfind('/') {
        Some(i) => (&first[..=i], &first[i + 1..]),
        None => ("", first.as_str()),
    };
    let root = normalize(&base_dir.join(dir_part));
    if root.as_os_str() == "/" {
        return Expansion::Refused(Refusal::RootDirectory);
    }

    let mut toks: Vec<Tok> = rest_first.bytes().map(Tok::Byte).collect();
    let mut pattern = format!("{}/{}", root.display(), rest_first);
    for p in &parts[1..] {
        match p {
            PatPart::Lit(s) => {
                toks.extend(s.bytes().map(Tok::Byte));
                pattern.push_str(s);
            }
            PatPart::Wild { .. } => {
                if toks.last() != Some(&Tok::Star) {
                    toks.push(Tok::Star);
                }
                pattern.push('*');
            }
        }
    }

    let m = Matcher::new(toks);
    let mut walk = Walk {
        fs,
        m: &m,
        limit,
        budget: limit.saturating_mul(VISIT_BUDGET_FACTOR).max(1000),
        matches: Vec::new(),
        truncated: false,
        skip_dirs: opts.skip_dirs,
        keep_file: opts.keep_file,
    };
    let start = m.start();
    walk.dir(&root, &start);
    walk.matches
        .sort_unstable_by(|a, b| a.as_os_str().as_bytes().cmp(b.as_os_str().as_bytes()));
    Expansion::Matches {
        pattern,
        matches: walk.matches,
        truncated: walk.truncated,
    }
}

struct Walk<'a> {
    fs: &'a DirCache,
    m: &'a Matcher,
    limit: usize,
    budget: usize,
    matches: Vec<PathBuf>,
    truncated: bool,
    skip_dirs: &'a [&'a str],
    keep_file: Option<fn(&Path) -> bool>,
}

impl Walk<'_> {
    /// Walks `dir`, where `state` is the matcher state after the relative path
    /// of `dir` (including its trailing `/`).
    fn dir(&mut self, dir: &Path, state: &[bool]) {
        let Some(listing) = self.fs.list(dir) else {
            return;
        };
        for e in listing.iter() {
            if self.truncated {
                return;
            }
            if self.budget == 0 {
                self.truncated = true;
                return;
            }
            self.budget -= 1;
            let s = self.m.step(state, e.name().as_bytes());
            if !Matcher::alive(&s) {
                continue;
            }
            let path = dir.join(e.name());
            let is_file = match e.kind() {
                EntryKind::File => true,
                EntryKind::Symlink => matches!(
                    self.fs.link_info(&path),
                    Some(LinkInfo {
                        kind: FileKind::File,
                        ..
                    })
                ),
                EntryKind::Dir => {
                    if self
                        .skip_dirs
                        .iter()
                        .any(|d| d.as_bytes() == e.name().as_bytes())
                    {
                        continue;
                    }
                    let s2 = self.m.step(&s, b"/");
                    if Matcher::alive(&s2) {
                        self.dir(&path, &s2);
                    }
                    false
                }
                EntryKind::Other => false,
            };
            if is_file && self.m.accepts(&s) && self.keep_file.is_none_or(|keep| keep(&path)) {
                if self.matches.len() >= self.limit {
                    self.truncated = true;
                    return;
                }
                self.matches.push(path);
            }
        }
    }
}

/// Resolves a pattern specifier to a [`Target::Glob`], for use inside a
/// [`turborepo_trace_core::Resolver`].
///
/// The result always carries an uncertainty:
/// [`UncertaintyKind::PartialSpecifier`] for a successful expansion,
/// [`UncertaintyKind::PatternTooBroad`] when truncated or refused.
pub fn resolve_pattern(
    parts: &[Part<'_>],
    base_dir: &Path,
    cx: &ResolveCx<'_>,
    span: Span,
) -> Resolution {
    let parts: Vec<PatPart> = parts.iter().map(PatPart::from).collect();
    match expand(&parts, base_dir, cx.fs, cx.options.max_glob) {
        Expansion::Matches {
            pattern,
            matches,
            truncated,
        } => {
            let u = if truncated {
                Uncertainty::new(
                    UncertaintyKind::PatternTooBroad,
                    span,
                    format!(
                        "{pattern}: more than {} matches; truncated",
                        cx.options.max_glob
                    ),
                )
            } else {
                Uncertainty::new(
                    UncertaintyKind::PartialSpecifier,
                    span,
                    format!("{pattern}: {} match(es)", matches.len()),
                )
            };
            Resolution::target(Target::Glob {
                pattern: pattern.into(),
                matches: matches.iter().map(|p| cx.intern(p)).collect(),
                truncated,
            })
            .with_uncertainty(u)
        }
        Expansion::Refused(r) => {
            let why = match r {
                Refusal::LeadingWildcard => "pattern starts with a wildcard; not expanded",
                Refusal::RootDirectory => "pattern is anchored at /; not expanded",
            };
            Resolution::unresolved(Uncertainty::new(
                UncertaintyKind::PatternTooBroad,
                span,
                why,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn lit(s: &str) -> PatPart {
        PatPart::Lit(s.into())
    }
    fn wild() -> PatPart {
        PatPart::Wild { reason: "x".into() }
    }

    fn tree() -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        fs::create_dir_all(r.join("locales/sub")).unwrap();
        fs::create_dir_all(r.join("other")).unwrap();
        for f in ["en.json", "de.json", "readme.md", "sub/fr.json"] {
            fs::write(r.join("locales").join(f), "{}").unwrap();
        }
        fs::write(r.join("other/x.json"), "{}").unwrap();
        t
    }

    fn rel(r: &Path, e: &Expansion) -> (Vec<String>, bool) {
        match e {
            Expansion::Matches {
                matches, truncated, ..
            } => (
                matches
                    .iter()
                    .map(|p| p.strip_prefix(r).unwrap().to_string_lossy().into_owned())
                    .collect(),
                *truncated,
            ),
            Expansion::Refused(r) => panic!("refused: {r:?}"),
        }
    }

    #[test]
    fn expands_prefix_wild_suffix() {
        let t = tree();
        let r = t.path();
        let c = DirCache::new();
        let e = expand(&[lit("./locales/"), wild(), lit(".json")], r, &c, 1000);
        let (files, truncated) = rel(r, &e);
        // Wildcards cross `/`, so sub/fr.json matches too (over-approximation).
        assert_eq!(
            files,
            ["locales/de.json", "locales/en.json", "locales/sub/fr.json"]
        );
        assert!(!truncated);
        if let Expansion::Matches { pattern, .. } = &e {
            assert!(pattern.ends_with("/locales/*.json"), "{pattern}");
        }
    }

    #[test]
    fn partial_file_name_prefix() {
        let t = tree();
        let r = t.path();
        let c = DirCache::new();
        let e = expand(&[lit("locales/e"), wild()], r, &c, 1000);
        assert_eq!(rel(r, &e).0, ["locales/en.json"]);
        // No '/' in the literal: anchored at base_dir.
        let e = expand(&[lit("oth"), wild(), lit(".json")], r, &c, 1000);
        assert_eq!(rel(r, &e).0, ["other/x.json"]);
    }

    #[test]
    fn absolute_and_parent_patterns() {
        let t = tree();
        let r = t.path();
        let c = DirCache::new();
        let abs = format!("{}/other/", r.display());
        let e = expand(&[lit(&abs), wild()], Path::new("/unused"), &c, 1000);
        assert_eq!(rel(r, &e).0, ["other/x.json"]);
        let e = expand(&[lit("../other/"), wild()], &r.join("locales"), &c, 1000);
        assert_eq!(rel(r, &e).0, ["other/x.json"]);
    }

    #[test]
    fn refuses_leading_wildcard_and_root() {
        let c = DirCache::new();
        assert_eq!(
            expand(&[wild(), lit(".json")], Path::new("/tmp"), &c, 1000),
            Expansion::Refused(Refusal::LeadingWildcard)
        );
        assert_eq!(
            expand(&[lit("/"), wild()], Path::new("/tmp"), &c, 1000),
            Expansion::Refused(Refusal::RootDirectory)
        );
        assert!(matches!(
            expand(&[], Path::new("/tmp"), &c, 1000),
            Expansion::Matches {
                truncated: false,
                ..
            }
        ));
    }

    #[test]
    fn limit_truncates() {
        let t = tempfile::tempdir().unwrap();
        for i in 0..20 {
            fs::write(t.path().join(format!("f{i:02}.txt")), "").unwrap();
        }
        let c = DirCache::new();
        let e = expand(&[lit("./f"), wild()], t.path(), &c, 5);
        let (files, truncated) = rel(t.path(), &e);
        assert_eq!(files.len(), 5);
        assert!(truncated);
        assert_eq!(files[0], "f00.txt");
    }

    #[test]
    fn missing_dir_is_empty() {
        let t = tree();
        let c = DirCache::new();
        let e = expand(&[lit("./nope/"), wild()], t.path(), &c, 10);
        assert_eq!(rel(t.path(), &e), (vec![], false));
    }

    #[test]
    fn prunes_non_matching_dirs() {
        let t = tree();
        let r = t.path();
        let c = DirCache::new();
        let e = expand(&[lit("./locales/sub/"), wild()], r, &c, 1000);
        assert_eq!(rel(r, &e).0, ["locales/sub/fr.json"]);
        // `other/` cannot match `locales/...`, so it is never listed.
        let e = expand(&[lit("./l"), wild(), lit(".md")], r, &c, 1000);
        assert_eq!(rel(r, &e).0, ["locales/readme.md"]);
        assert!(c.list(&r.join("other")).is_some()); // listing works when asked directly
    }

    #[test]
    fn skip_dirs_and_keep_file() {
        let t = tree();
        let r = t.path();
        fs::create_dir_all(r.join("locales/node_modules/x")).unwrap();
        fs::write(r.join("locales/node_modules/x/a.json"), "{}").unwrap();
        let c = DirCache::new();
        let all = expand(&[lit("./locales/"), wild()], r, &c, 1000);
        assert!(rel(r, &all).0.iter().any(|f| f.contains("node_modules")));
        let opts = ExpandOptions {
            skip_dirs: &["node_modules"],
            keep_file: Some(|p| p.extension().is_none_or(|e| e != "md")),
            ..ExpandOptions::new(1000)
        };
        let e = expand_with(&[lit("./locales/"), wild()], r, &c, &opts);
        assert_eq!(
            rel(r, &e).0,
            ["locales/de.json", "locales/en.json", "locales/sub/fr.json"]
        );
    }

    #[test]
    fn matcher_basics() {
        let m = Matcher::new(vec![Tok::Byte(b'a'), Tok::Star, Tok::Byte(b'b')]);
        let s = m.start();
        assert!(m.accepts(&m.step(&s, b"ab")));
        assert!(m.accepts(&m.step(&s, b"a/x/b")));
        assert!(!m.accepts(&m.step(&s, b"a")));
        assert!(!Matcher::alive(&m.step(&s, b"b")));
    }
}
