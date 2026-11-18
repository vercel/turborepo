//! Whether which files are tests depends on where the repository is checked
//! out.
//!
//! jest and Playwright match their test patterns against the **absolute** path
//! of a file. `testMatch: ['**/src/**/*.test.js']` makes every `*.test.js` a
//! test in a checkout below a directory called `src`, and
//! `testPathIgnorePatterns: ['/dist/']` leaves a checkout below a directory
//! called `dist` without tests. Discovery is therefore matched as the
//! runner matches it ([`crate::inputs::DiscoveryMatcher`]), and the place of
//! the checkout is an input of a project exactly when it matters:
//!
//! - A project's discovery is kept as its config writes it, without the place
//!   ([`DiscoverySpec`]): the patterns with `<rootDir>` in them, the
//!   directories from the repository root.
//! - It is evaluated where the checkout is and at two **neutral roots**
//!   ([`NEUTRAL_ROOTS`]): the same relative paths below directories whose names
//!   no pattern spells (control characters), one deep and one directly below
//!   `/`, so that a pattern which counts directories or starts at `^/` differs
//!   from one of them.
//! - Equal answers for every file: the place decides nothing, and nothing about
//!   it is recorded. Different answers: the project is **location-dependent**
//!   ([`Verdict`]), the absolute root becomes part of its record, and `turbo
//!   trace lint` reports it.
//!
//! Most configurations are decided from the patterns alone ([`tells_apart`]):
//! an expression that is in the same state after reading the root as after
//! reading a neutral root cannot come to another answer for what follows. Only
//! when a pattern can tell the roots apart are the project's files matched
//! again.
//!
//! What cannot be kept without the place makes a spec *inexact*
//! ([`DiscoverySpec::exact`]): a pattern or directory that the config computes
//! from its own location (`__dirname`), a `rootDir` or a `<rootDir>` path that
//! climbs with `..`. Such a project records the root like a location-dependent
//! one (a graph built elsewhere evaluates it again), and is reported only when
//! its answers differ.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

use serde::{Deserialize, Serialize};

use crate::{
    inputs::{TestDiscovery, TestPattern},
    pattern::JsRegex,
};

/// The neutral roots: directories whose names no pattern spells. The deep one first: it is
/// the one differences are counted against (a checkout is rarely kept directly below `/`).
// rustfmt's `format_strings` would change this item's string literals.
#[rustfmt::skip]
pub const NEUTRAL_ROOTS: [&str; 2] = [
    "/\u{1}/\u{2}/\u{3}/\u{4}/\u{5}/\u{6}/\u{7}/\u{1}\u{2}/\u{1}\u{3}/\u{1}\u{4}/\u{1}\u{5}/\u{1}\u{6}",
    "/\u{1}\u{7}",
];

/// Stands for the repository root in a text that is not written with
/// `<rootDir>` ([`PatternSpec::text`] of an inexact spec).
const PLACE: &str = "\u{0}";

/// One pattern of a [`DiscoverySpec`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatternSpec {
    /// A glob (else a JS regex).
    pub glob: bool,
    /// The pattern: as the config writes it when [`PatternSpec::rooted`]
    /// (`<rootDir>/src/**`), else as it is matched.
    pub text: String,
    /// The flags of a regex.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub flags: String,
    /// The config writes `<rootDir>` in it: the runner puts the project's root
    /// directory there.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub rooted: bool,
}

impl PatternSpec {
    /// A pattern that is matched as it is written.
    pub fn plain(p: &TestPattern) -> Self {
        match p {
            TestPattern::Glob(g) => PatternSpec {
                glob: true,
                text: g.clone(),
                flags: String::new(),
                rooted: false,
            },
            TestPattern::Regex(r) => PatternSpec {
                glob: false,
                text: r.source().to_owned(),
                flags: r.flags().to_owned(),
                rooted: false,
            },
        }
    }

    /// A glob a jest config writes (`testMatch`), `<rootDir>` in it or not.
    pub fn jest_glob(raw: &str) -> Self {
        PatternSpec {
            glob: true,
            text: raw.to_owned(),
            flags: String::new(),
            rooted: raw.starts_with("<rootDir>"),
        }
    }

    /// An expression a jest config writes as a string (`testRegex`,
    /// `testPathIgnorePatterns`), `<rootDir>` in it or not.
    pub fn jest_regex(raw: &str) -> Self {
        PatternSpec {
            glob: false,
            text: raw.to_owned(),
            flags: String::new(),
            rooted: raw.contains("<rootDir>"),
        }
    }

    /// The pattern as the runner matches it in a checkout at `repo` whose
    /// project root is `root_dir`.
    pub fn at(&self, repo: &str, root_dir: &Path) -> TestPattern {
        match (self.glob, self.rooted) {
            (true, true) => TestPattern::Glob(jest_glob(&self.text, root_dir)),
            (false, true) => TestPattern::Regex(JsRegex::new(
                self.text.replace("<rootDir>", &root_dir.to_string_lossy()),
                "",
            )),
            (true, false) => TestPattern::Glob(self.text.replace(PLACE, repo)),
            (false, false) => TestPattern::Regex(JsRegex::new(
                self.text.replace(PLACE, repo),
                self.flags.clone(),
            )),
        }
    }
}

/// A `testMatch` glob as jest matches it: `<rootDir>` at its start is the
/// project's root directory with its glob characters escaped
/// (`escapeGlobCharacters`), so a root like `app/(site)` is matched literally;
/// the path is normalized.
pub fn jest_glob(raw: &str, root_dir: &Path) -> String {
    let joined = crate::common::replace_root_dir(raw, root_dir);
    let root_s = root_dir.to_string_lossy();
    match joined.strip_prefix(root_s.as_ref()) {
        Some(rest) if raw.starts_with("<rootDir>") => {
            let mut out = String::new();
            for c in root_s.chars() {
                if matches!(
                    c,
                    '(' | ')' | '*' | '{' | '}' | '[' | ']' | '!' | '?' | '\\'
                ) {
                    out.push('\\');
                }
                out.push(c);
            }
            out.push_str(rest);
            out
        }
        _ => joined,
    }
}

/// A project's discovery as its config writes it, without the place of the
/// checkout: what [`DiscoverySpec::at`] makes a [`TestDiscovery`] of for a
/// checkout at any root.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoverySpec {
    /// The project's root directory (jest `rootDir`, Playwright `testDir`),
    /// from the repository root (`""`: the root itself).
    pub root_dir: String,
    /// The directories searched, from the repository root.
    pub roots: Vec<String>,
    /// A file is a test if it matches any of these...
    pub include: Vec<PatternSpec>,
    /// ...and none of these.
    pub exclude: Vec<PatternSpec>,
    /// Of [`DiscoverySpec::exclude`], the patterns that leave a file out of the
    /// runner's file map altogether (jest `modulePathIgnorePatterns`: no
    /// test, no manual mock, no module), by index.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unmapped: Vec<usize>,
    /// Allowed extensions, without dot (empty: any).
    pub extensions: Vec<String>,
    /// [`TestDiscovery::absolute`].
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub absolute: bool,
    /// Whether the spec says everything the config does at every place. Not
    /// when something of it was computed from the place the config was
    /// loaded at and cannot be told from a path the config spells:
    /// [`DiscoverySpec::inexact`] says what.
    pub exact: bool,
    /// Why the spec is not exact, for a message.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub inexact: String,
}

/// What a config writes for a [`DiscoverySpec`], as the adapter read it at
/// `repo`.
pub struct Written<'a> {
    /// The repository root.
    pub repo: &'a Path,
    /// The project's root directory as loaded (absolute, normalized).
    pub root_dir: &'a Path,
    /// Whether the config writes its root directory as the same directory of
    /// the tree at every place ([`stays_inside`]): relative to the config,
    /// without stepping above the repository.
    pub root_dir_plain: bool,
    /// The directories searched, as loaded (absolute), each with whether the
    /// config writes it that way, relative to the root directory.
    pub roots: Vec<(PathBuf, bool)>,
    /// The patterns that say which files are tests.
    pub include: Vec<PatternSpec>,
    /// The patterns that leave files out.
    pub exclude: Vec<PatternSpec>,
    /// [`DiscoverySpec::unmapped`].
    pub unmapped: Vec<usize>,
    /// Allowed extensions.
    pub extensions: Vec<String>,
    /// [`TestDiscovery::absolute`].
    pub absolute: bool,
    /// Whether a source of the config spells the repository root's absolute
    /// path: a pattern with that path in it may then be the config's own
    /// text, not a path computed from its location.
    pub spells_root: bool,
}

/// Whether `text` holds the path `root` where a path starts and ends:
/// `/work/repo/src` and `x|/work/repo`, not `/work/repo2` or `/a/work/repo`.
fn names_path(text: &str, root: &str) -> bool {
    occurrences(text, root).next().is_some()
}

/// The byte offsets where `root` occurs in `text` as a path ([`names_path`]).
fn occurrences<'t>(text: &'t str, root: &'t str) -> impl Iterator<Item = usize> + 't {
    let name = |c: char| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '~' | '@');
    text.match_indices(root)
        .filter(move |(i, _)| {
            let before = text[..*i].chars().next_back();
            let after = text[*i + root.len()..].chars().next();
            !before.is_some_and(|c| name(c) || c == '/') && !after.is_some_and(name)
        })
        .map(|(i, _)| i)
}

/// `text` with [`PLACE`] for every occurrence of the path `root`.
fn placed(text: &str, root: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for i in occurrences(text, root) {
        if i < at {
            continue;
        }
        out.push_str(&text[at..i]);
        out.push_str(PLACE);
        at = i + root.len();
    }
    out.push_str(&text[at..]);
    out
}

/// `path` from `repo` (`""` for `repo` itself); `None` when it is not inside.
fn below(repo: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(repo)
        .ok()
        .map(|r| r.to_string_lossy().into_owned())
}

fn has_dot_dot(text: &str) -> bool {
    text.split('/').any(|c| c == "..")
}

/// Whether `raw`, a path that a config writes relative to the directory `dir`
/// (as it is, or after `<rootDir>` when `dir` is the project's root directory),
/// names the same directory of the tree wherever the repository at `repo` is
/// checked out: it is relative, `dir` is inside the repository, and no `..` of
/// it steps above the repository root (`../x` from the root goes through the
/// directory the checkout is in, whatever that is called).
pub fn stays_inside(repo: &Path, dir: &Path, raw: &str) -> bool {
    let raw = raw.strip_prefix("<rootDir>").unwrap_or(raw);
    let raw = raw.strip_prefix('/').unwrap_or(raw);
    if raw.starts_with('/') || raw.starts_with('<') || raw.starts_with('~') {
        return false;
    }
    let Ok(below) = dir.strip_prefix(repo) else {
        return false;
    };
    let mut depth = below.components().count();
    for c in raw.split('/') {
        match c {
            "" | "." => {}
            ".." if depth == 0 => return false,
            ".." => depth -= 1,
            _ => depth += 1,
        }
    }
    true
}

impl DiscoverySpec {
    /// The spec of what a config writes.
    pub fn of(w: Written<'_>) -> DiscoverySpec {
        let repo_s = w.repo.to_string_lossy().into_owned();
        let mut inexact: Vec<String> = Vec::new();
        let mut note = |why: String| {
            if !inexact.contains(&why) {
                inexact.push(why);
            }
        };
        // A repository at `/`: every absolute path would be "the root's".
        let placeable = repo_s.len() > 1;
        let root_dir = match below(w.repo, w.root_dir) {
            Some(r) => {
                if !w.root_dir_plain {
                    note(
                        "the project's root directory is computed from where the config is (an \
                         absolute path, or `..` above the repository)"
                            .to_owned(),
                    );
                }
                r
            }
            None => {
                note("the project's root directory is outside the repository".to_owned());
                w.root_dir.to_string_lossy().into_owned()
            }
        };
        let roots = w
            .roots
            .iter()
            .map(|(dir, plain)| match below(w.repo, dir) {
                Some(r) => {
                    if !plain {
                        note(format!(
                            "the directory {} is computed from where the config is",
                            if r.is_empty() { "." } else { &r }
                        ));
                    }
                    r
                }
                // Outside the repository: none of its files is there, at any place.
                None => dir.to_string_lossy().into_owned(),
            })
            .collect();
        let mut pattern = |mut p: PatternSpec| -> PatternSpec {
            if p.rooted {
                // `<rootDir>/../../x` from a root one level down: where it lands depends
                // on what the directory of the checkout is called.
                if has_dot_dot(&p.text) && !stays_inside(w.repo, w.root_dir, &p.text) {
                    note(format!(
                        "the pattern {} climbs above the repository with `..`",
                        p.text
                    ));
                }
            } else if placeable && names_path(&p.text, &repo_s) {
                if w.spells_root {
                    // The config's own text: matched as it is written, at every
                    // place.
                } else {
                    note(format!(
                        "the pattern {} is computed from where the config is (`__dirname`)",
                        placed(&p.text, &repo_s).replace(PLACE, "<the checkout>")
                    ));
                    p.text = placed(&p.text, &repo_s);
                }
            }
            p
        };
        let include = w.include.into_iter().map(&mut pattern).collect();
        let exclude = w.exclude.into_iter().map(&mut pattern).collect();
        DiscoverySpec {
            root_dir,
            roots,
            include,
            exclude,
            unmapped: w.unmapped,
            extensions: w.extensions,
            absolute: w.absolute,
            exact: inexact.is_empty(),
            inexact: inexact.join("; "),
        }
    }

    /// The directory `rel` (from the repository root, or absolute) in a
    /// checkout at `repo`.
    fn dir(repo: &Path, rel: &str) -> PathBuf {
        if rel.starts_with('/') {
            PathBuf::from(rel)
        } else if rel.is_empty() {
            repo.to_path_buf()
        } else {
            repo.join(rel)
        }
    }

    /// The spec of a discovery that was read once more with the config loaded
    /// as if the repository were at the deep neutral root
    /// ([`crate::jsconfig::as_if`]): `roots`, `include` and `exclude` are
    /// that load's, `root_dir` its project root, and `loaded`
    /// is the discovery as loaded where the repository is (at `repo`).
    ///
    /// In the neutral load, whatever the config computes from its own location
    /// has the neutral root in its text, and no config spells that root. So
    /// when every neutral name in the values is part of the whole neutral
    /// root, the values with the root's place marked say what the config
    /// writes at every place: `path.join(__dirname, 'src')` is the
    /// directory `src` of the repository, `` `${__dirname}/checks/.*` `` an
    /// expression below the root. `None` when a value holds a part of the
    /// neutral root only (a directory above the checkout:
    /// `path.join(__dirname, '..', 'repo')`, or the name of the checkout's
    /// directory), when a directory is not below the root, and
    /// when the spec, taken at `repo`, is not the discovery as loaded (the
    /// config decides something by the text of its location).
    #[allow(clippy::too_many_arguments)]
    pub fn proved(
        repo: &Path,
        root_dir: &Path,
        roots: &[PathBuf],
        include: &[TestPattern],
        exclude: &[TestPattern],
        unmapped: Vec<usize>,
        extensions: Vec<String>,
        loaded: &TestDiscovery,
    ) -> Option<DiscoverySpec> {
        let neutral = Path::new(NEUTRAL_ROOTS[0]);
        let neutral_s = NEUTRAL_ROOTS[0];
        let names_neutral = |t: &str| t.chars().any(|c| ('\u{1}'..='\u{7}').contains(&c));
        let dir = |p: &Path| -> Option<String> {
            let rel = below(neutral, p)?;
            (!names_neutral(&rel)).then_some(rel)
        };
        let pattern = |p: &TestPattern| -> Option<PatternSpec> {
            let mut spec = PatternSpec::plain(p);
            spec.text = placed(&spec.text, neutral_s);
            (!names_neutral(&spec.text)).then_some(spec)
        };
        let spec = DiscoverySpec {
            root_dir: dir(root_dir)?,
            roots: roots.iter().map(|r| dir(r)).collect::<Option<_>>()?,
            include: include.iter().map(pattern).collect::<Option<_>>()?,
            exclude: exclude.iter().map(pattern).collect::<Option<_>>()?,
            unmapped,
            extensions,
            absolute: loaded.absolute,
            exact: true,
            inexact: String::new(),
        };
        let here = spec.at(repo);
        (here.roots == loaded.roots
            && here.include == loaded.include
            && here.exclude == loaded.exclude
            && here.extensions == loaded.extensions)
            .then_some(spec)
    }

    /// The discovery in a checkout at `repo` (absolute, normalized).
    pub fn at(&self, repo: &Path) -> TestDiscovery {
        let repo_s = repo.to_string_lossy();
        let root_dir = Self::dir(repo, &self.root_dir);
        TestDiscovery {
            roots: self.roots.iter().map(|r| Self::dir(repo, r)).collect(),
            include: self
                .include
                .iter()
                .map(|p| p.at(&repo_s, &root_dir))
                .collect(),
            exclude: self
                .exclude
                .iter()
                .map(|p| p.at(&repo_s, &root_dir))
                .collect(),
            glob_base: None,
            extensions: self.extensions.clone(),
            absolute: self.absolute,
            choice: None,
            spec: None,
        }
    }
}

/// The discoveries of one project whose answers decide which files are its
/// tests: the configured one and, when the project's discovery was chosen among
/// two by the files that are there ([`crate::inputs::DiscoveryChoice`]), jest's
/// default one. In that order whichever applies: the text of a spec does not
/// change when the choice does.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSpec {
    /// The configured discovery.
    pub configured: DiscoverySpec,
    /// The default one, when it may stand in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<DiscoverySpec>,
}

impl ProjectSpec {
    /// The spec of a project's discovery and whether the default discovery is
    /// the one that applies; `None` when the adapter keeps none (a runner
    /// that matches from its own root: the place has no say).
    pub fn of(d: &TestDiscovery) -> Option<(ProjectSpec, bool)> {
        let own = d.spec.as_deref()?.clone();
        let Some(c) = d.choice.as_deref() else {
            return Some((
                ProjectSpec {
                    configured: own,
                    default: None,
                },
                false,
            ));
        };
        let other = c.other.spec.as_deref().cloned();
        Some(if c.fell_back {
            (
                ProjectSpec {
                    configured: other?,
                    default: Some(own),
                },
                true,
            )
        } else {
            (
                ProjectSpec {
                    configured: own,
                    default: other,
                },
                false,
            )
        })
    }

    /// The spec as text: the same for the same config at every place.
    pub fn to_text(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Reads [`ProjectSpec::to_text`].
    pub fn from_text(text: &str) -> Option<ProjectSpec> {
        serde_json::from_str(text).ok()
    }

    fn all(&self) -> impl Iterator<Item = &DiscoverySpec> {
        std::iter::once(&self.configured).chain(self.default.as_ref())
    }

    /// The discovery that applies: the default one when `fell_back` (and there
    /// is one).
    pub fn applying(&self, fell_back: bool) -> &DiscoverySpec {
        match &self.default {
            Some(d) if fell_back => d,
            _ => &self.configured,
        }
    }

    /// Whether every part of the spec is exact ([`DiscoverySpec::exact`]).
    pub fn exact(&self) -> bool {
        self.all().all(|d| d.exact)
    }
}

/// The files of the repository, for [`verdict`]: the walked files
/// (repo-relative, sorted), and whether the tree holds a chain of directories
/// called like the ones the checkout is in (computed once, when a pattern needs
/// it).
pub struct Tree<'a> {
    files: &'a [String],
    chain: Arc<OnceLock<bool>>,
}

impl<'a> Tree<'a> {
    /// `files`: repo-relative, sorted.
    pub fn new(files: &'a [String]) -> Self {
        Tree {
            files,
            chain: Arc::default(),
        }
    }

    /// [`Tree::new`] with a memo that other trees over the same files of a
    /// checkout at the same root share.
    pub fn sharing(files: &'a [String], chain: &Arc<OnceLock<bool>>) -> Self {
        Tree {
            files,
            chain: Arc::clone(chain),
        }
    }

    /// The files below `dir` (repo-relative, `""` for all).
    fn below(&self, dir: &str) -> &'a [String] {
        if dir.is_empty() {
            return self.files;
        }
        let prefix = format!("{dir}/");
        let lo = self.files.partition_point(|p| p.as_str() < prefix.as_str());
        let n = self.files[lo..].partition_point(|p| p.starts_with(prefix.as_str()));
        &self.files[lo..lo + n]
    }

    /// Whether some file's path holds the components of `repo` one after the
    /// other (`work/repo/` in `vendor/work/repo/x.js` for a checkout at
    /// `/work/repo`).
    fn holds_chain(&self, repo: &str) -> bool {
        *self.chain.get_or_init(|| {
            let chain = format!("{}/", repo.trim_start_matches('/'));
            let find = aho_corasick::AhoCorasick::new([chain.as_str()]);
            let Ok(find) = find else {
                return true;
            };
            self.files.iter().any(|f| {
                find.find_iter(f.as_str())
                    .any(|m| m.start() == 0 || f.as_bytes()[m.start() - 1] == b'/')
            })
        })
    }
}

/// What the place of the checkout decides for a project.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Verdict {
    /// Some file's answer differs between the checkout's root and a neutral
    /// root.
    pub dependent: bool,
    /// Files (repo-relative, sorted) that are tests here and none at the
    /// neutral root the difference was found against.
    pub gained: Vec<String>,
    /// Files that are tests at that neutral root and none here.
    pub lost: Vec<String>,
}

/// Whether the expression `text` (in the `regex` crate's syntax, searched
/// anywhere in a path as [`regex::Regex::is_match`] does) can come to different
/// answers for two paths that differ only in which of `starts` they begin with.
/// `Some(false)`: it cannot, for any continuation. `Some(true)`: it may.
/// `None`: not known (the expression does not compile to an automaton).
///
/// The automaton of the expression reads each start; when it passes no match
/// and ends in the same state after each, what follows decides alone. Memoized
/// per expression and starts: a repository's projects share a few patterns.
pub fn tells_apart(text: &str, starts: &[&str]) -> Option<bool> {
    type Memo = Mutex<HashMap<(String, Vec<String>), Option<bool>>>;
    static MEMO: OnceLock<Memo> = OnceLock::new();
    let key = (
        text.to_owned(),
        starts.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(),
    );
    let memo = MEMO.get_or_init(Memo::default);
    if let Some(v) = memo.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return *v;
    }
    let v = tells_apart_uncached(text, starts);
    memo.lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key, v);
    v
}

fn tells_apart_uncached(text: &str, starts: &[&str]) -> Option<bool> {
    use regex_automata::{Input, hybrid::dfa::DFA};
    let dfa = DFA::new(text).ok()?;
    let mut cache = dfa.create_cache();
    let mut first = None;
    for s in starts {
        let mut sid = dfa
            .start_state_forward(&mut cache, &Input::new(s.as_bytes()))
            .ok()?;
        for &b in s.as_bytes() {
            sid = dfa.next_state(&mut cache, sid, b).ok()?;
            if sid.is_tagged() {
                if sid.is_match() {
                    // A match inside the start: the answer is the start's.
                    return Some(true);
                }
                if sid.is_quit() || sid.is_unknown() {
                    return None;
                }
            }
        }
        match first {
            None => first = Some(sid),
            Some(f) if f != sid => return Some(true),
            Some(_) => {}
        }
    }
    // The states were compared by id: only within one cache generation.
    (cache.clear_count() == 0).then_some(false)
}

/// Whether the place may decide something for `d` in a checkout at `repo`, from
/// the patterns alone. `false`: no file's answer can differ between `repo` and
/// a neutral root.
fn suspected(d: &DiscoverySpec, repo: &Path, tree: &Tree<'_>) -> bool {
    let repo_s = repo.to_string_lossy();
    // jest's file map leaves out every path through a `node_modules` directory.
    if d.absolute && format!("{repo_s}/").contains("/node_modules/") {
        return true;
    }
    let here = format!("{repo_s}/");
    let neutral: Vec<String> = NEUTRAL_ROOTS.iter().map(|n| format!("{n}/")).collect();
    let mut starts: Vec<&str> = vec![here.as_str()];
    starts.extend(neutral.iter().map(String::as_str));
    // A path that starts with the project's root directory, written by the runner:
    // such a pattern sees the root where the file's path starts, the same at
    // every place, unless the root's own text can be found again further on.
    let plain_root = !d.root_dir.starts_with('/')
        && !has_dot_dot(&d.root_dir)
        && repo_s
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '/' | '_' | '-' | '.' | '@' | '~'));
    let overlaps = || {
        // The root's text, read as an expression (`.` is any character), found in
        // itself at a later directory.
        let r = repo_s.as_bytes();
        (1..r.len()).filter(|&i| r[i] == b'/').any(|i| {
            r[i..]
                .iter()
                .zip(r)
                .all(|(text, pat)| *pat == b'.' || pat == text)
        })
    };
    for p in d.include.iter().chain(&d.exclude) {
        if p.rooted {
            if !plain_root || has_dot_dot(&p.text) {
                return true;
            }
            if p.glob {
                // Anchored at the start of the path with the root, literally.
                continue;
            }
            let once = p.text.starts_with("<rootDir>") && p.text.matches("<rootDir>").count() == 1;
            if !once || overlaps() || tree.holds_chain(&repo_s) {
                return true;
            }
            continue;
        }
        if p.text.contains(PLACE) {
            // The root where the config computed it: at the start of the pattern it is
            // where every file's path starts, like `<rootDir>`.
            let once = p.text.starts_with(PLACE) && p.text.matches(PLACE).count() == 1;
            if !once || !plain_root {
                return true;
            }
            if p.glob {
                continue;
            }
            if overlaps() || tree.holds_chain(&repo_s) {
                return true;
            }
            continue;
        }
        let text = crate::pattern::automaton_text(&p.at(&repo_s, repo));
        match tells_apart(&text, &starts) {
            Some(false) => {}
            _ => return true,
        }
    }
    false
}

/// What the place of a checkout at `repo` decides for the project `spec`: the
/// answers of its discoveries for every walked file below their directories, at
/// `repo` and at the neutral roots. Decided from the patterns alone where they
/// cannot tell the roots apart.
///
/// `fell_back`: the default discovery is the one that applies
/// ([`ProjectSpec::of`]); the lists of the verdict are that discovery's.
pub fn verdict(spec: &ProjectSpec, fell_back: bool, repo: &Path, tree: &Tree<'_>) -> Verdict {
    if !spec.all().any(|d| suspected(d, repo, tree)) {
        return Verdict::default();
    }
    differences(spec, fell_back, repo, tree)
}

/// [`verdict`] without the shortcut: every file is matched at every root.
pub fn differences(spec: &ProjectSpec, fell_back: bool, repo: &Path, tree: &Tree<'_>) -> Verdict {
    let mut out = Verdict::default();
    let applying = spec.applying(fell_back);
    for d in spec.all() {
        let here = d.at(repo);
        let here_m = here.matcher().rooted(repo);
        // The patterns that leave a file out of the file map, at a root.
        let unmapped = |all: &TestDiscovery| -> Vec<crate::pattern::CompiledPattern> {
            d.unmapped
                .iter()
                .filter_map(|&i| all.exclude.get(i))
                .map(crate::pattern::CompiledPattern::new)
                .collect()
        };
        let left_out = |ps: &[crate::pattern::CompiledPattern], p: &Path| {
            let text = p.to_string_lossy();
            ps.iter().any(|x| x.matches(&text))
        };
        let here_u = unmapped(&here);
        for n in NEUTRAL_ROOTS {
            let n = Path::new(n);
            let there = d.at(n);
            let there_m = there.matcher().rooted(n);
            let there_u = unmapped(&there);
            let (mut gained, mut lost) = (Vec::new(), Vec::new());
            let mut differs = false;
            for root in &d.roots {
                if root.starts_with('/') {
                    continue;
                }
                for f in tree.below(root) {
                    let (a, b) = (repo.join(f), n.join(f));
                    let (ta, tb) = (here_m.is_test(&a), there_m.is_test(&b));
                    if ta && !tb {
                        gained.push(f.clone());
                    } else if tb && !ta {
                        lost.push(f.clone());
                    } else if left_out(&here_u, &a) != left_out(&there_u, &b) {
                        // No test either way, and left out of the runner's file map at
                        // one root only (its mocks, its modules).
                        differs = true;
                    }
                }
            }
            if !gained.is_empty() || !lost.is_empty() || differs {
                out.dependent = true;
                // The lists are the applying discovery's, against the first neutral root
                // that differs.
                if std::ptr::eq(d, applying) && out.gained.is_empty() && out.lost.is_empty() {
                    gained.sort();
                    gained.dedup();
                    lost.sort();
                    lost.dedup();
                    out.gained = gained;
                    out.lost = lost;
                }
            }
        }
    }
    out
}

/// The record of a project's location: what [`state`] gives for its spec.
///
/// `""`: the place decides nothing. Otherwise the absolute root: the project is
/// location-dependent there, or its spec is inexact and the answer of another
/// place cannot be derived from it.
pub fn state(spec_text: &str, repo: &Path, tree: &Tree<'_>) -> String {
    let dependent = match ProjectSpec::from_text(spec_text) {
        Some(spec) => !spec.exact() || verdict(&spec, false, repo, tree).dependent,
        // A spec this version cannot read: the place is taken to decide.
        None => true,
    };
    if dependent {
        repo.to_string_lossy().into_owned()
    } else {
        String::new()
    }
}

/// The names of the directories above (and of) the checkout at `repo` that the
/// answer for `file` (repo-relative) depends on: the ones whose renaming
/// changes it. For a message.
pub fn names(spec: &DiscoverySpec, repo: &Path, file: &str) -> Vec<String> {
    let answer = |root: &Path| -> (bool, bool) {
        let d = spec.at(root);
        let m = d.matcher().rooted(root);
        let p = root.join(file);
        (m.is_test(&p), m.left_out(&p))
    };
    // (Every pattern that leaves files out counts here: the names are for a message
    // about one file whose answer differs.)
    let here = answer(repo);
    let parts: Vec<&std::ffi::OsStr> = repo.iter().skip(1).collect();
    let mut out = Vec::new();
    for (i, name) in parts.iter().enumerate() {
        let mut renamed = PathBuf::from("/");
        for (j, c) in parts.iter().enumerate() {
            if i == j {
                renamed.push("\u{1}\u{1}");
            } else {
                renamed.push(c);
            }
        }
        if answer(&renamed) != here {
            out.push(name.to_string_lossy().into_owned());
        }
    }
    out
}

/// Why the answer for `file` (repo-relative) differs between a checkout at
/// `repo` and a neutral root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Culprit {
    /// A pattern, as the config writes it, matches the file at one root only.
    Pattern {
        /// The pattern as written.
        text: String,
        /// It is one that leaves files out.
        leaves_out: bool,
    },
    /// jest's file map leaves out every path through a `node_modules`
    /// directory, and the checkout lies below one.
    InstalledPackages,
}

/// The first pattern of `d` whose match for `file` differs between `repo` and a
/// neutral root. For a message.
pub fn culprit(d: &DiscoverySpec, repo: &Path, file: &str) -> Option<Culprit> {
    let matches = |p: &PatternSpec, root: &Path| -> bool {
        let root_s = root.to_string_lossy();
        let pattern = p.at(&root_s, &DiscoverySpec::dir(root, &d.root_dir));
        crate::pattern::CompiledPattern::new(&pattern).matches(&root.join(file).to_string_lossy())
    };
    for n in NEUTRAL_ROOTS {
        let n = Path::new(n);
        let all = d
            .include
            .iter()
            .map(|p| (p, false))
            .chain(d.exclude.iter().map(|p| (p, true)));
        for (p, leaves_out) in all {
            if matches(p, repo) != matches(p, n) {
                return Some(Culprit::Pattern {
                    text: p.text.replace(PLACE, "<the checkout>"),
                    leaves_out,
                });
            }
        }
    }
    (d.absolute && format!("{}/", repo.display()).contains("/node_modules/"))
        .then_some(Culprit::InstalledPackages)
}

/// Shares a spec between a discovery and its users.
pub fn shared(spec: DiscoverySpec) -> Option<Arc<DiscoverySpec>> {
    Some(Arc::new(spec))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jest(include: &[PatternSpec], exclude: &[PatternSpec]) -> ProjectSpec {
        ProjectSpec {
            configured: DiscoverySpec {
                root_dir: String::new(),
                roots: vec![String::new()],
                include: include.to_vec(),
                exclude: exclude.to_vec(),
                unmapped: Vec::new(),
                extensions: vec!["js".into()],
                absolute: true,
                exact: true,
                inexact: String::new(),
            },
            default: None,
        }
    }

    fn files(list: &[&str]) -> Vec<String> {
        let mut v: Vec<String> = list.iter().map(|s| (*s).to_owned()).collect();
        v.sort();
        v
    }

    const FILES: &[&str] = &[
        "src/a.test.js",
        "src/a.js",
        "tools/x.test.js",
        "dist/b.test.js",
        "fixtures/c.test.js",
        "checks/one.js",
    ];

    fn defaults() -> Vec<PatternSpec> {
        crate::jest::DEFAULT_TEST_MATCH
            .iter()
            .map(|g| PatternSpec::jest_glob(g))
            .collect()
    }

    fn ignore_modules() -> Vec<PatternSpec> {
        vec![PatternSpec::jest_regex("/node_modules/")]
    }

    /// jest's defaults: the place decides nothing at an ordinary place, and
    /// that is known from the patterns alone; below `node_modules` and
    /// below `__tests__` it does, as in jest.
    #[test]
    fn the_defaults_depend_on_the_place_only_where_jest_does() {
        let spec = jest(&defaults(), &ignore_modules());
        let fs = files(FILES);
        for repo in [
            "/work/repo",
            "/home/runner/work/x/x",
            "/tmp/.tmpAb12/repo",
            "/app",
            "/vercel/path0",
            "/work/dist/build/vendor/src/x",
            "/a/my.test/repo",
        ] {
            let tree = Tree::new(&fs);
            let repo = Path::new(repo);
            assert!(!spec.all().any(|d| suspected(d, repo, &tree)), "{repo:?}");
            assert_eq!(verdict(&spec, false, repo, &tree), Verdict::default());
            assert_eq!(differences(&spec, false, repo, &tree), Verdict::default());
            assert_eq!(state(&spec.to_text(), repo, &tree), "");
        }
        // Below `node_modules`: jest finds no test.
        let tree = Tree::new(&fs);
        let v = verdict(&spec, false, Path::new("/ci/node_modules/pkg/repo"), &tree);
        assert!(v.dependent);
        assert_eq!(v.gained, [""; 0]);
        assert_eq!(
            v.lost,
            [
                "dist/b.test.js",
                "fixtures/c.test.js",
                "src/a.test.js",
                "tools/x.test.js"
            ]
        );
        // Below `__tests__`: every file is one.
        let tree = Tree::new(&fs);
        let v = verdict(&spec, false, Path::new("/ci/__tests__/repo"), &tree);
        assert!(v.dependent);
        assert_eq!(v.gained, ["checks/one.js", "src/a.js"]);
        assert_eq!(v.lost, [""; 0]);
        assert_eq!(
            names(
                &spec.configured,
                Path::new("/ci/__tests__/repo"),
                "src/a.js"
            ),
            ["__tests__"]
        );
    }

    /// The review's patterns: a directory, a wildcard, a file name. At a place
    /// whose directories the pattern does not name, the answers are the
    /// neutral ones (and `tools/x.test.js` is no test); below a directory
    /// it names, they are jest's.
    #[test]
    fn a_pattern_that_can_match_above_the_root_does_so_only_where_it_does() {
        let fs = files(FILES);
        for p in [
            PatternSpec::jest_glob("**/src/**/*.test.js"),
            PatternSpec::jest_regex("/src/.*[.]test[.]js$"),
            PatternSpec::jest_regex("src.*[.]test[.]js$"),
        ] {
            let spec = jest(std::slice::from_ref(&p), &ignore_modules());
            let tree = Tree::new(&fs);
            let here = Path::new("/work/repo");
            assert_eq!(
                verdict(&spec, false, here, &tree),
                Verdict::default(),
                "{p:?}"
            );
            let d = spec.configured.at(here);
            let m = d.matcher();
            assert!(m.is_test(&here.join("src/a.test.js")));
            assert!(!m.is_test(&here.join("tools/x.test.js")), "{p:?}");
            // In a checkout below `src`, jest takes every `*.test.js`.
            let there = Path::new("/home/me/src/repo");
            let tree = Tree::new(&fs);
            let v = verdict(&spec, false, there, &tree);
            assert!(v.dependent, "{p:?}");
            assert_eq!(
                v.gained,
                ["dist/b.test.js", "fixtures/c.test.js", "tools/x.test.js"],
                "{p:?}"
            );
            assert_eq!(
                names(&spec.configured, there, "tools/x.test.js"),
                ["src"],
                "{p:?}"
            );
            assert_eq!(state(&spec.to_text(), there, &tree), "/home/me/src/repo");
        }
    }

    /// An expression that names the checkout's own directory, with or without a
    /// `/` before its wildcard (the reviewer's four expressions), finds
    /// what jest finds: the tests in a checkout of that name, none
    /// elsewhere.
    #[test]
    fn an_expression_that_names_the_checkout_finds_its_tests_there() {
        let fs = files(FILES);
        for re in [
            "repo/src/.*[.]test[.]js$",
            "repo.*[.]test[.]js$",
            "work.*[.]test[.]js$",
            "repo[/]src[/].*[.]test[.]js$",
            "repo.src..*[.]test[.]js$",
        ] {
            let spec = jest(&[PatternSpec::jest_regex(re)], &ignore_modules());
            let here = Path::new("/work/repo");
            let d = spec.configured.at(here);
            assert!(d.matcher().is_test(&here.join("src/a.test.js")), "{re}");
            let tree = Tree::new(&fs);
            let v = verdict(&spec, false, here, &tree);
            assert!(v.dependent, "{re}");
            assert!(v.gained.contains(&"src/a.test.js".to_owned()), "{re}");
            assert!(v.lost.is_empty(), "{re}");
            let there = Path::new("/ci/checkout");
            let d = spec.configured.at(there);
            assert!(!d.matcher().is_test(&there.join("src/a.test.js")), "{re}");
            let tree = Tree::new(&fs);
            assert_eq!(
                verdict(&spec, false, there, &tree),
                Verdict::default(),
                "{re}"
            );
        }
    }

    /// `<rootDir>` is the same directory of the tree at every place: decided
    /// from the patterns. An expression that counts directories or starts
    /// at `^/` is not.
    #[test]
    fn root_dir_patterns_anchors_and_counted_directories() {
        let fs = files(FILES);
        let spec = jest(
            &[
                PatternSpec::jest_glob("<rootDir>/src/**/*.test.js"),
                PatternSpec::jest_regex("<rootDir>/tools/.*[.]test[.]js$"),
            ],
            &[PatternSpec::jest_regex("<rootDir>/dist/")],
        );
        for repo in ["/work/repo", "/app", "/home/me/src/x", "/a.b/c"] {
            let repo = Path::new(repo);
            let tree = Tree::new(&fs);
            assert!(!spec.all().any(|d| suspected(d, repo, &tree)), "{repo:?}");
            assert_eq!(differences(&spec, false, repo, &tree), Verdict::default());
            let d = spec.configured.at(repo);
            let m = d.matcher();
            assert!(m.is_test(&repo.join("src/a.test.js")));
            assert!(m.is_test(&repo.join("tools/x.test.js")));
            assert!(!m.is_test(&repo.join("dist/b.test.js")));
            assert!(!m.is_test(&repo.join("fixtures/c.test.js")));
        }
        // The tree holds the checkout's own path again: matched file by file.
        let again = files(&["src/a.test.js", "vendor/app/dist/x.test.js"]);
        let tree = Tree::new(&again);
        assert!(suspected(&spec.configured, Path::new("/app"), &tree));
        // Two directories and then `src`: true two levels below `/` only.
        let counted = jest(
            &[PatternSpec::jest_regex("^/[^/]+/[^/]+/src/.*[.]test[.]js$")],
            &[],
        );
        let tree = Tree::new(&fs);
        let v = verdict(&counted, false, Path::new("/work/repo"), &tree);
        assert!(v.dependent);
        assert_eq!(v.gained, ["src/a.test.js"]);
        let tree = Tree::new(&fs);
        assert!(!verdict(&counted, false, Path::new("/work/a/repo"), &tree).dependent);
        // One directory: the shallow neutral root has one too, the deep one differs.
        let one = jest(
            &[PatternSpec::jest_regex("^/[^/]+/src/.*[.]test[.]js$")],
            &[],
        );
        let tree = Tree::new(&fs);
        assert!(verdict(&one, false, Path::new("/app"), &tree).dependent);
        let tree = Tree::new(&fs);
        assert!(verdict(&one, false, Path::new("/work/repo"), &tree).dependent);
    }

    /// A pattern computed from the config's location is not kept exactly; one
    /// the config spells is its own text.
    #[test]
    fn computed_and_spelled_roots() {
        let repo = Path::new("/work/repo");
        let written = |spells_root: bool| Written {
            repo,
            root_dir: repo,
            root_dir_plain: true,
            roots: vec![(repo.to_path_buf(), true)],
            include: vec![PatternSpec::jest_regex("/work/repo/src/.*[.]test[.]js$")],
            exclude: Vec::new(),
            unmapped: Vec::new(),
            extensions: vec!["js".into()],
            absolute: true,
            spells_root,
        };
        let computed = DiscoverySpec::of(written(false));
        assert!(!computed.exact);
        assert!(computed.inexact.contains("<the checkout>/src/"));
        assert_eq!(
            computed.at(repo).include,
            written(false)
                .include
                .iter()
                .map(|p| p.at("", repo))
                .collect::<Vec<_>>()
        );
        let other = Path::new("/ci/x");
        assert!(
            computed
                .at(other)
                .matcher()
                .is_test(&other.join("src/a.test.js"))
        );
        let spelled = DiscoverySpec::of(written(true));
        assert!(spelled.exact);
        assert!(
            !spelled
                .at(other)
                .matcher()
                .is_test(&other.join("src/a.test.js"))
        );
        assert!(
            spelled
                .at(repo)
                .matcher()
                .is_test(&repo.join("src/a.test.js"))
        );
        // `/work/repo2` is another directory.
        assert!(!names_path("/work/repo2/src", "/work/repo"));
        assert!(names_path("x|/work/repo/src", "/work/repo"));
    }

    #[test]
    fn states_of_an_automaton() {
        let n: Vec<String> = NEUTRAL_ROOTS.iter().map(|n| format!("{n}/")).collect();
        let starts = |here: &'static str| -> Vec<&str> {
            let mut v = vec![here];
            v.extend(n.iter().map(String::as_str));
            v
        };
        assert_eq!(
            tells_apart("/node_modules/", &starts("/work/repo/")),
            Some(false)
        );
        assert_eq!(
            tells_apart(r".*\.spec\.ts$", &starts("/work/repo/")),
            Some(false)
        );
        assert_eq!(
            tells_apart("/dist/", &starts("/work/dist/repo/")),
            Some(true)
        );
        assert_eq!(tells_apart("/dist/", &starts("/work/dist/")), Some(true));
        assert_eq!(tells_apart("^/work/", &starts("/work/repo/")), Some(true));
        assert_eq!(tells_apart("^/src/", &starts("/work/repo/")), Some(false));
        assert_eq!(tells_apart("repo.*x$", &starts("/work/repo/")), Some(true));
    }
}
