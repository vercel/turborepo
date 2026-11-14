//! Per-test working directories: [`Graph::cwd_patch`].
//!
//! A reference relative to the process working directory
//! (`require(process.cwd() + '/src/routes')`,
//! `readFileSync(path.resolve('fixtures/a.json'))`) is stored on its file
//! with [`turborepo_trace_core::Base::Cwd`], resolved against the repository
//! root, and carries a `cwd_dependent` uncertainty: what it names depends on
//! who runs the file. Under jest and vitest that is known: a test runs in one
//! of a few directories ([`crate::shard::TestRec::cwds`]). The reference
//! usually sits in a shared helper outside the test's package, so the
//! resolution belongs to the pair (test, reference), not to the helper's file.
//! No shard can hold it: which tests reach the helper depends on every shard
//! between them. It is derived here, from the composed graph.
//!
//! For every test `T` with known working directories, every file `F` that `T`
//! runs (its closure over the edges uncertainty follows,
//! [`turborepo_trace_graph::EdgeFilter::for_uncertainty`]: a file that `T` or a
//! task only reads as data does not run in `T`'s working directory) and
//! every `cwd_dependent` reference `R` of `F` with a literal specifier, for
//! each working directory `C` of `T`:
//!
//! | `R` | Added to `T` |
//! |---|---|
//! | a module load (`require`, `import()`, worker, child process) of a JS file | an edge to every existing candidate of `C/<spec>` (the path itself, with each module extension, and `index` files below it), a config input for existing candidates that are not nodes, a probe for every missing candidate and for `C/<spec>/package.json` |
//! | a file read or asset path | an edge to `C/<spec>` when it is a node, a config input when it exists without being one, a probe when it is missing; for a directory, edges and inputs for the files below it plus a `**` membership |
//! | a directory listing | a `*` membership of `C/<spec>` (and a probe when the directory is missing) |
//!
//! "Exists" is decided against the snapshot's walked-file list and the graph's
//! nodes, with no file system access: the patch is a function of the manifest.
//! Edges added to `T` can bring new files into its closure (the routes module a
//! helper loads has its own imports), so the pairs are computed to a fixed
//! point.
//!
//! The `cwd_dependent` record stays on `F`, marked *entry-resolved*
//! ([`turborepo_trace_graph::GraphView::is_entry_resolved`]): it no longer
//! counts for tests with known working directories, and counts as before for
//! every other entry (a build task, a file given as an entry, a test of another
//! framework). A record is only marked when every reference that raised it
//! could be resolved for every test and directory that needed it. A module load
//! whose specifier is a pattern below a directory of the working directory
//! (`require(path.join(process.cwd(), 'src', name))`: `./src/*`) is resolved
//! too: an edge to every walked file below `C/src` that the pattern matches (as
//! written, with a module extension, or as an `index` file), a config input for
//! a match that is not a node, and a `**` membership of `C/src`, so that files
//! added there count. The directory may hold at most [`MAX_DIR_FILES`] files,
//! and the pattern has to name a directory below `C`: a pattern over the
//! working directory itself is not resolved.
//!
//! Not resolved (the record stays open for everyone): any other specifier that
//! is not literal (a read through a pattern is bounded by names instead), a
//! value read from a mutable binding (a guess), a path outside the repository,
//! a module path that is a directory with a `package.json` (its `main` is not
//! known here), a directory with more than [`MAX_DIR_FILES`] files, and module
//! loads in files of other languages.
//!
//! Soundness rests on [`crate::shard::TestRec::cwds`] naming every directory
//! the runner is started in (see `fw::test_cwds`); the repository root is
//! covered by the edges and probes the tracer recorded on `F` itself.
//!
//! # The repository root, and a path that climbs above it
//!
//! The root is a working directory of every test whose runner does not change
//! into a directory of its own ([`ROOT_RUNNERS`]: jest, vitest, pytest and
//! Playwright can be started there with a path filter; `go test` and `cargo
//! test` run a test in its package's directory). The tracer's edges on `F`
//! cover it for a path that stays inside the repository. One that climbs above
//! the root from there (`../repo/data/x.json`) is no path of the repository for
//! the tracer: it has one reading, the repository seen from another
//! place ([`turborepo_trace_core::fs::seen_from_elsewhere`]: `data/x.json`, in
//! a checkout whose directory is called `repo`), and that reading is added
//! here, per test, like the resolutions from the other directories.
//!
//! Except when the reference is *explained* ([`explained`]): from another
//! working directory of the test, or from the directory of the package that
//! holds the reference (a script is written for the directory of its package),
//! the same path stays inside the repository and goes through an existing
//! directory where the reading from the root takes the names of the directories
//! above the checkout (`../../packages/ui/dist/a.svg`, written for a runner
//! started in `apps/web`: `packages/ui` exists; from the root it would name
//! `dist/a.svg` in a checkout kept below `…/packages/ui/`). The reading from
//! the root is then not taken. This is an assumption: a repository is not
//! checked out below directories whose names repeat a path inside itself.
//! [`Graph::set_cwd_root_always`] turns it off (`--strict`,
//! `TURBO_TRACE_DEBUG_CWD_EXPLAINED=0`), and [`Graph::cwd_assumed`] says where
//! it decided.

use std::collections::{HashMap, HashSet};

use turborepo_trace_core::{Phase, RefFlags, RefKind, UncertaintyKind};
use turborepo_trace_graph::{self as q, EdgeAttr, NodeId, NodeSet, Patch};

use super::{Graph, NONE, span_of};
use crate::{
    paths::{join_repo, join_simple},
    shard::{ArchivedBaseRec, ArchivedSpecTok, kind},
};

/// A directory read through a working-directory-relative path is only resolved
/// per test when it holds at most this many files.
pub const MAX_DIR_FILES: usize = 512;

/// Extensions a JS module path is tried with (the resolver's JS and TS lists).
pub(super) const MODULE_EXTS: &[&str] = &[
    ".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs", ".json", ".node",
];

/// Appended to the detail of a record resolved per test.
pub const RESOLVED_NOTE: &str = "resolved per test from the test runner's working directory";

/// One `cwd_dependent` reference.
pub(super) struct SiteRef {
    pub(super) kind: RefKind,
    pub(super) attr: u16,
    /// Literal specifiers (alternatives); `None`: not resolvable.
    pub(super) specs: Option<Vec<String>>,
    pub(super) line: u32,
    /// Span of the uncertainty record the reference raised.
    pub(super) unc_span: [u32; 2],
    /// The file is JS/TS (module paths are resolved with the JS candidates).
    pub(super) js: bool,
}

/// A module load declared for a file (a hint): `specs` are loaded relative to
/// the working directory wherever the file runs.
pub(super) struct ExtraSite {
    /// The file's node.
    pub node: u32,
    /// Specifiers relative to the working directory (`./src/routes`).
    pub specs: Vec<String>,
    /// Line shown in edge labels (0: none).
    pub line: u32,
    /// Identifies the declaration in the failed set (not a real span).
    pub key: [u32; 2],
}

/// A part of a specifier pattern: literal text, or anything (also across `/`).
#[derive(Clone, Debug, PartialEq, Eq)]
enum PatTok {
    Lit(String),
    Wild,
}

/// Most parts of a pattern that is resolved per test.
const MAX_PATTERN_PARTS: usize = 8;

/// A file with `cwd_dependent` references.
struct Site {
    node: u32,
    /// The directory of the package that holds the file, from the root (empty:
    /// the root package).
    pkg: String,
    refs: Vec<SiteRef>,
    /// Parallel to `refs`: the pattern of a module load that has one instead of
    /// literal specifiers.
    patterns: Vec<Option<Vec<PatTok>>>,
    /// Nodes that reach the file.
    reach: NodeSet,
}

/// What one reference resolves to in one working directory.
#[derive(Default)]
pub(super) struct Resolution {
    pub(super) edges: Vec<u32>,
    pub(super) probes: Vec<String>,
    pub(super) inputs: Vec<String>,
    pub(super) members: Vec<(String, String)>,
}

fn literals(toks: &[ArchivedSpecTok]) -> Option<Vec<String>> {
    match toks {
        [ArchivedSpecTok::Literal(s)] => Some(vec![s.to_string()]),
        [ArchivedSpecTok::Alt(n), rest @ ..] if rest.len() == n.to_native() as usize => rest
            .iter()
            .map(|t| match t {
                ArchivedSpecTok::Literal(s) => Some(s.to_string()),
                _ => None,
            })
            .collect(),
        _ => None,
    }
}

/// The parts of a pattern specifier that starts with literal text.
fn pattern(toks: &[ArchivedSpecTok]) -> Option<Vec<PatTok>> {
    let [ArchivedSpecTok::Pattern(n), parts @ ..] = toks else {
        return None;
    };
    if parts.len() != n.to_native() as usize || parts.len() > MAX_PATTERN_PARTS {
        return None;
    }
    let out: Vec<PatTok> = parts
        .iter()
        .map(|t| match t {
            ArchivedSpecTok::Lit(s) => Some(PatTok::Lit(s.to_string())),
            ArchivedSpecTok::Wild(_) => Some(PatTok::Wild),
            _ => None,
        })
        .collect::<Option<_>>()?;
    matches!(out.first(), Some(PatTok::Lit(_))).then_some(out)
}

/// Whether `s` is `parts` with each wildcard standing for any text.
fn wild_match(parts: &[PatTok], s: &str) -> bool {
    match parts.split_first() {
        None => s.is_empty(),
        Some((PatTok::Lit(l), rest)) => s
            .strip_prefix(l.as_str())
            .is_some_and(|t| wild_match(rest, t)),
        Some((PatTok::Wild, [])) => true,
        Some((PatTok::Wild, rest)) => (0..=s.len())
            .filter(|&i| s.is_char_boundary(i))
            .any(|i| wild_match(rest, &s[i..])),
    }
}

/// A pattern as text: `./src/*`.
fn show_pattern(parts: &[PatTok]) -> String {
    parts
        .iter()
        .map(|p| match p {
            PatTok::Lit(l) => l.as_str(),
            PatTok::Wild => "*",
        })
        .collect()
}

pub(super) fn is_module_kind(k: RefKind) -> bool {
    matches!(
        k,
        RefKind::Require
            | RefKind::DynamicImport
            | RefKind::Import
            | RefKind::Worker
            | RefKind::ChildProcess
    )
}

/// Whether the record's detail says the value was a guess (a mutable binding's
/// initial value, an unknown object's `.join` taken as `path.join`).
fn is_guess(detail: &str) -> bool {
    detail.contains("initial value is used") || detail.contains("assumed to be `path.")
}

/// The label of an edge of the per-test working-directory layer: the test's
/// working directory, what the site names, and the site (the file and line
/// whose reference is resolved there).
fn cwd_edge_label(cwd: &str, specs: &str, site: &str, line: u32) -> String {
    format!(
        "cwd {}: {specs} ({site}:{line})",
        if cwd.is_empty() { "." } else { cwd }
    )
}

/// The site file of an edge of the per-test working-directory layer, from its
/// label ([`cwd_edge_label`]): the edge runs from a test to what the site loads
/// in that test's working directory, so the code that holds the loaded module
/// is the site's. `None` for any other label.
pub fn cwd_edge_site(label: &str) -> Option<&str> {
    let rest = label.strip_prefix("cwd ")?.strip_suffix(')')?;
    let (_, site) = rest.rsplit_once(" (")?;
    let (path, line) = site.rsplit_once(':')?;
    (!path.is_empty() && line.bytes().all(|b| b.is_ascii_digit())).then_some(path)
}

/// `spec`, a path that a file writes relative to the working directory, as a
/// path from the repository root when the file runs in `cwd` (a directory of
/// the repository, from the root): [`join_repo`]. A path that climbs above the
/// root from there and comes back down names what follows the directories above
/// the root, the repository seen from another
/// place ([`turborepo_trace_core::fs::seen_from_elsewhere`]:
/// `../repo/data/x.json` in the root is `data/x.json` when the checkout's
/// directory is called `repo`): here the working directory is known, so the
/// number of levels is. `None` for an absolute path (the resolver has read it)
/// and for a path that stays above the root.
fn join_cwd(cwd: &str, spec: &str) -> Option<String> {
    if let Some(p) = join_repo(cwd, spec) {
        return Some(p);
    }
    if spec.starts_with('/') {
        return None;
    }
    let depth = cwd.split('/').filter(|s| !s.is_empty()).count();
    match turborepo_trace_core::fs::seen_from_elsewhere(Some(depth), spec).as_slice() {
        [reading] => Some(reading.rel.clone()),
        _ => None,
    }
}

/// The frameworks whose runner can be started in the repository root for a test
/// of any package (with a path filter, `--projects`, a root config):
/// `process.cwd()` in such a test may be the root. Not `go test` and `cargo
/// test`, which run every test in the directory of its package.
pub const ROOT_RUNNERS: &[&str] = &["jest", "vitest", "pytest"];

/// A relative path as the levels it climbs and the names that follow them
/// (`../../packages/ui/dist/a.svg`: 2 and `packages`, `ui`, `dist`, `a.svg`).
/// `None` for an absolute path.
fn climb(spec: &str) -> Option<(usize, Vec<&str>)> {
    if spec.starts_with('/') {
        return None;
    }
    let mut up = 0;
    let mut parts: Vec<&str> = Vec::new();
    for c in spec.split('/') {
        match c {
            "" | "." => {}
            ".." if !parts.is_empty() => {
                parts.pop();
            }
            ".." => up += 1,
            c => parts.push(c),
        }
    }
    Some((up, parts))
}

/// Whether `spec`, read from the repository root, climbs above it and comes
/// back down: it has a reading there ([`join_cwd`] with the root), and no path
/// of the repository.
fn climbs_back(spec: &str) -> bool {
    climb(spec).is_some_and(|(up, parts)| up > 0 && parts.len() > up && !parts.contains(&".git"))
}

/// Whether the working directory `cwd` (not the root) explains `spec`, a path
/// that climbs back from the root ([`climbs_back`]): from `cwd` the path stays
/// inside the repository, and the names that the reading from the root takes
/// for the directories above the checkout are an existing directory (or file)
/// there. `exists`: whether a path from the root is a walked file or has walked
/// files below it. Returns that path.
fn explained(cwd: &str, spec: &str, exists: impl Fn(&str) -> bool) -> Option<String> {
    let (up, parts) = climb(spec)?;
    let dirs: Vec<&str> = cwd.split('/').filter(|s| !s.is_empty()).collect();
    if dirs.is_empty() || dirs.len() < up || parts.len() <= up {
        return None;
    }
    let mut at: Vec<&str> = dirs[..dirs.len() - up].to_vec();
    at.extend(&parts[..up]);
    let at = at.join("/");
    exists(&at).then_some(at)
}

/// `TURBO_TRACE_DEBUG_CWD_EXPLAINED=0`: the reading from the repository root is
/// taken whatever another working directory explains (for A/B measurements;
/// `--strict` does the same).
pub(super) fn root_always_by_switch() -> bool {
    turborepo_trace_core::debug::var_os("CWD_EXPLAINED").is_some_and(|v| v == "0")
}

/// A reference whose reading from the repository root was not taken because
/// another directory explains its path ([`explained`]): the checkable side of
/// the assumption ([`Graph::cwd_dropped`], the lint rule `cwd-path-explained`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CwdDropped {
    /// The file that holds the reference (repo-relative).
    pub site: String,
    /// Its line.
    pub line: u32,
    /// The path as written, relative to the working directory.
    pub spec: String,
    /// The directory the path was taken to be written for (repo-relative).
    pub dir: String,
    /// The existing directory the path goes through from there.
    pub at: String,
    /// What the path names from the repository root (repo-relative)...
    pub names: String,
    /// ...when this holds, as the end of a sentence (`the repository is checked
    /// out in a directory called ui`).
    pub when: String,
}

impl CwdDropped {
    fn new(site: &str, line: u32, spec: &str, dir: &str, at: &str) -> Self {
        let reading = turborepo_trace_core::fs::seen_from_elsewhere(Some(0), spec);
        let (names, when) = reading
            .first()
            .map(|e| (e.rel.clone(), e.when.clone()))
            .unwrap_or_default();
        CwdDropped {
            site: site.to_owned(),
            line,
            spec: spec.to_owned(),
            dir: dir.to_owned(),
            at: at.to_owned(),
            names,
            when,
        }
    }
}

/// One reading from the root that was not taken, as a sentence for `--explain`.
fn assumed_sentence(site: &str, line: u32, spec: &str, dir: &str, at: &str) -> String {
    let reading = turborepo_trace_core::fs::seen_from_elsewhere(Some(0), spec);
    let (rel, when) = reading
        .first()
        .map(|e| (e.rel.as_str(), e.when.as_str()))
        .unwrap_or_default();
    format!(
        "{site}:{line} reads {spec} relative to the working directory; from {dir}/ that path goes \
         through {at}/, which exists, so it is not also read from the repository root, where it \
         would name {rel} if {when}"
    )
}

impl Graph {
    /// The snapshot's walked-file list as stored (see
    /// [`crate::manifest::files_in`]).
    pub(super) fn walked_files(&self) -> Option<Vec<u8>> {
        let out = self.graph_dir.as_deref()?;
        crate::manifest::read_files_raw(&out.join("store"), self.manifest.files.as_deref()?)
    }

    /// The tests with known working directories, by node: the directories the
    /// test runner may run each in (repository relative, `""` for the
    /// root).
    pub fn tests_with_cwds(&self) -> HashMap<u32, Vec<&str>> {
        let mut out: HashMap<u32, Vec<&str>> = HashMap::new();
        for (ni, node) in self.nodes.iter().enumerate() {
            if node.shard == NONE {
                continue;
            }
            // Test records are sorted by file.
            let tests = &self.shards[node.shard as usize].get().tests;
            let lo = tests.partition_point(|t| t.file.to_native() < node.file);
            for t in tests[lo..]
                .iter()
                .take_while(|t| t.file.to_native() == node.file)
            {
                let v = out.entry(ni as u32).or_default();
                for c in t.cwds.iter() {
                    if !v.contains(&c.as_str()) {
                        v.push(c.as_str());
                    }
                }
            }
        }
        // Entries whose runner is not a graph framework (Playwright specs).
        for (n, c) in &self.entry_cwds {
            let v = out.entry(*n).or_default();
            if !v.contains(&c.as_str()) {
                v.push(c.as_str());
            }
        }
        out.retain(|_, v| !v.is_empty());
        out
    }

    /// Of [`Graph::tests_with_cwds`], the tests whose runner may also be
    /// started in the repository root ([`ROOT_RUNNERS`], and the entries of
    /// [`Graph::set_entry_cwds`]).
    fn tests_run_from_root(&self) -> HashSet<u32> {
        let mut out = HashSet::new();
        for (ni, node) in self.nodes.iter().enumerate() {
            if node.shard == NONE {
                continue;
            }
            let tests = &self.shards[node.shard as usize].get().tests;
            let lo = tests.partition_point(|t| t.file.to_native() < node.file);
            if tests[lo..]
                .iter()
                .take_while(|t| t.file.to_native() == node.file)
                .any(|t| ROOT_RUNNERS.contains(&t.framework.as_str()))
            {
                out.insert(ni as u32);
            }
        }
        out.extend(self.entry_cwds.iter().map(|(n, _)| *n));
        out
    }

    /// Whether the reading from the repository root of a working-directory path
    /// that climbs above it is taken even where another working directory
    /// of the test explains the path (see the module documentation):
    /// `--strict`, `TURBO_TRACE_DEBUG_CWD_EXPLAINED=0`.
    pub fn set_cwd_root_always(&mut self, always: bool) {
        self.cwd_root_always = always || root_always_by_switch();
    }

    /// Where the per-test resolution did not take the reading from the
    /// repository root of a path in `entry`'s closure, because another
    /// working directory of the entry explains the path (see the module
    /// documentation): one sentence each, for `--explain`. Filled
    /// by [`Graph::cwd_patch`].
    pub fn cwd_assumed(&self, entry: NodeId) -> Vec<String> {
        let notes = self
            .cwd_notes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut out: Vec<String> = notes
            .iter()
            .filter(|(n, _)| *n == entry.0)
            .map(|(_, s)| s.clone())
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Every reference whose reading from the repository root the per-test
    /// resolution did not take because another directory explains the path
    /// (see the module documentation), each once and sorted. Filled by
    /// [`Graph::cwd_patch`].
    pub fn cwd_dropped(&self) -> Vec<CwdDropped> {
        self.cwd_dropped
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Declares the working directories (repo-relative, empty for the root) of
    /// entries that are not tests of a graph framework, so that
    /// [`Graph::cwd_patch`] resolves working-directory-relative references
    /// for them as it does for jest and vitest tests. The caller vouches
    /// that the runner is started in one of them or in the repository root.
    pub fn set_entry_cwds(&mut self, entries: impl IntoIterator<Item = (NodeId, String)>) {
        self.entry_cwds = entries.into_iter().map(|(n, c)| (n.0, c)).collect();
    }

    /// The first-party files with `cwd_dependent` references (without reach
    /// sets), and the records that cannot be resolved: `(node, span)`.
    // Invariant: the table lists every kind (unit-tested).
    #[allow(clippy::expect_used)]
    fn cwd_sites(&self) -> (Vec<Site>, HashSet<(u32, [u32; 2])>) {
        let cwd_kind = crate::convert::UNCERTAINTY_KINDS
            .iter()
            .position(|k| *k == UncertaintyKind::CwdDependent)
            .expect("listed kind") as u8;
        let mut sites = Vec::new();
        let mut failed = HashSet::new();
        for (ni, node) in self.nodes.iter().enumerate() {
            if node.shard == NONE {
                continue;
            }
            let a = self.shards[node.shard as usize].get();
            if a.package.kind != kind::FIRST_PARTY {
                continue;
            }
            let f = &a.files[node.file as usize];
            let Some(ex) = f.extraction.as_ref() else {
                continue;
            };
            let js = f.lang.as_ref().is_some_and(|l| l.as_str() == "js");
            let mut refs = Vec::new();
            let mut patterns = Vec::new();
            for r in ex.refs.iter() {
                let Some(u) = r.uncertainty.as_ref() else {
                    continue;
                };
                if !matches!(r.base, ArchivedBaseRec::Cwd) || u.kind != cwd_kind {
                    continue;
                }
                let Some(&k) = RefKind::ALL.get(r.kind as usize) else {
                    continue;
                };
                let unc_span = [u.span[0].to_native(), u.span[1].to_native()];
                let guess = is_guess(u.detail.as_str());
                let specs = literals(&r.spec).filter(|_| !guess);
                // A module load of a JS file through a pattern below the working directory.
                let pat = if specs.is_none() && !guess && js && is_module_kind(k) {
                    pattern(&r.spec)
                } else {
                    None
                };
                if specs.is_none() && pat.is_none() {
                    failed.insert((ni as u32, unc_span));
                }
                patterns.push(pat);
                let phase = if r.phase == 0 {
                    Phase::Runtime
                } else {
                    Phase::Build
                };
                refs.push(SiteRef {
                    kind: k,
                    attr: crate::convert::edge_attr(
                        k,
                        phase,
                        RefFlags::from_bits_truncate(r.flags.to_native()),
                    ),
                    specs,
                    line: r.line.to_native(),
                    unc_span,
                    js,
                });
            }
            if !refs.is_empty() {
                sites.push(Site {
                    node: ni as u32,
                    pkg: a.package.root.as_str().trim_matches('/').to_owned(),
                    refs,
                    patterns,
                    reach: NodeSet::new(0),
                });
            }
        }
        (sites, failed)
    }

    /// `r` resolved in working directory `cwd`; `None` when it cannot be (see
    /// the module documentation).
    pub(super) fn resolve_in_cwd(
        &self,
        walked: &[&str],
        cwd: &str,
        r: &SiteRef,
    ) -> Option<Resolution> {
        let is_file = |p: &str| walked.binary_search(&p).is_ok();
        // The walked files below directory `p`.
        let below = |p: &str| -> &[&str] {
            if p.is_empty() {
                return walked;
            }
            let prefix = format!("{p}/");
            let lo = walked.partition_point(|f| *f < prefix.as_str());
            let n = walked[lo..].partition_point(|f| f.starts_with(prefix.as_str()));
            &walked[lo..lo + n]
        };
        let mut out = Resolution::default();
        // An existing path: an edge when it is a node, else a config input.
        let hit = |out: &mut Resolution, p: &str| match self.lookup(p) {
            Some(n) => out.edges.push(n),
            None => out.inputs.push(p.to_owned()),
        };
        for spec in r.specs.as_ref()? {
            let p = join_cwd(cwd, spec)?;
            if is_module_kind(r.kind) {
                if !r.js {
                    return None;
                }
                let mut cands: Vec<String> = vec![p.clone()];
                cands.extend(MODULE_EXTS.iter().map(|e| format!("{p}{e}")));
                let direct = cands.iter().any(|c| is_file(c));
                let manifest = join_simple(&p, "package.json");
                if !direct && is_file(&manifest) {
                    // A package directory: its `main` decides, and is not known here.
                    return None;
                }
                let dir = !below(&p).is_empty();
                cands.extend(MODULE_EXTS.iter().map(|e| format!("{p}/index{e}")));
                for c in &cands {
                    if is_file(c) {
                        hit(&mut out, c);
                    } else if !(dir && *c == p) {
                        // A probe on an existing directory would match every file added
                        // below it.
                        out.probes.push(c.clone());
                    }
                }
                if !is_file(&manifest) {
                    out.probes.push(manifest);
                }
                continue;
            }
            match r.kind {
                RefKind::FsRead | RefKind::Asset | RefKind::Embed => {
                    if is_file(&p) {
                        hit(&mut out, &p);
                        continue;
                    }
                    let files = below(&p);
                    if files.is_empty() {
                        // Missing: a file created there, or below it, changes the read.
                        if p.is_empty() {
                            return None;
                        }
                        out.probes.push(p);
                    } else if files.len() > MAX_DIR_FILES {
                        return None;
                    } else {
                        for f in files {
                            hit(&mut out, f);
                        }
                        out.members.push((p, "**".to_owned()));
                    }
                }
                RefKind::FsReadDir => {
                    if is_file(&p) {
                        hit(&mut out, &p);
                        continue;
                    }
                    if below(&p).is_empty() {
                        if p.is_empty() {
                            return None;
                        }
                        out.probes.push(p.clone());
                    }
                    out.members.push((p, "*".to_owned()));
                }
                _ => return None,
            }
        }
        Some(out)
    }

    /// A module pattern resolved in working directory `cwd` (see the module
    /// documentation); `None` when it cannot be: it names no directory
    /// below `cwd`, or the directory holds more than [`MAX_DIR_FILES`]
    /// files.
    fn resolve_pattern_in_cwd(
        &self,
        walked: &[&str],
        cwd: &str,
        parts: &[PatTok],
    ) -> Option<Resolution> {
        let (PatTok::Lit(first), rest) = parts.split_first()? else {
            return None;
        };
        // The directory the pattern starts in: the literal start up to its last `/`.
        let cut = first.rfind('/')?;
        let dir = join_cwd(cwd, &first[..cut])?;
        if dir.is_empty() || dir == cwd.trim_matches('/') {
            // A pattern over the whole working directory (or the repository) bounds
            // nothing.
            return None;
        }
        let prefix = format!("{dir}/");
        let lo = walked.partition_point(|f| *f < prefix.as_str());
        let n = walked[lo..].partition_point(|f| f.starts_with(prefix.as_str()));
        let files = &walked[lo..lo + n];
        if files.len() > MAX_DIR_FILES {
            return None;
        }
        // The pattern relative to the directory, as written and as a module path that
        // is completed by an extension or an `index` file.
        let mut toks: Vec<PatTok> = Vec::with_capacity(rest.len() + 1);
        if cut + 1 < first.len() {
            toks.push(PatTok::Lit(first[cut + 1..].to_owned()));
        }
        toks.extend(rest.iter().cloned());
        let mut forms = vec![toks.clone()];
        if !matches!(toks.last(), Some(PatTok::Wild)) {
            for tail in MODULE_EXTS
                .iter()
                .map(|e| (*e).to_owned())
                .chain(MODULE_EXTS.iter().map(|e| format!("/index{e}")))
            {
                let mut f = toks.clone();
                f.push(PatTok::Lit(tail));
                forms.push(f);
            }
        }
        let mut out = Resolution::default();
        for f in files {
            let rel = &f[prefix.len()..];
            if forms.iter().any(|form| wild_match(form, rel)) {
                match self.lookup(f) {
                    Some(n) => out.edges.push(n),
                    None => out.inputs.push((*f).to_owned()),
                }
            }
        }
        // Files added below the directory can match.
        out.members.push((dir, "**".to_owned()));
        Some(out)
    }

    /// The per-test resolution of working-directory-relative references (see
    /// the module documentation): apply it with
    /// [`turborepo_trace_graph::Patched`].
    pub fn cwd_patch(&self) -> Patch {
        self.cwd_patch_with(&[]).0
    }

    /// [`Graph::cwd_patch`] with declared working-directory-relative module
    /// loads (hints) in addition to the extracted references. Also returns
    /// the keys of the references that could not be resolved for some test:
    /// `(node, key)`.
    pub(super) fn cwd_patch_with(&self, extra: &[ExtraSite]) -> (Patch, HashSet<(u32, [u32; 2])>) {
        let mut patch = Patch::new();
        let tests = self.tests_with_cwds();
        if tests.is_empty() {
            return (patch, HashSet::new());
        }
        turborepo_trace_core::timing::mark("derive: tests");
        let (mut sites, mut failed) = self.cwd_sites();
        turborepo_trace_core::timing::mark("derive: sites");
        for x in extra {
            let r = SiteRef {
                kind: RefKind::Require,
                attr: crate::convert::edge_attr(RefKind::Require, Phase::Runtime, RefFlags::LAZY),
                specs: Some(x.specs.clone()),
                line: x.line,
                unc_span: x.key,
                js: true,
            };
            match sites.iter_mut().find(|s| s.node == x.node) {
                Some(s) => {
                    s.refs.push(r);
                    s.patterns.push(None);
                }
                None => sites.push(Site {
                    node: x.node,
                    pkg: String::new(),
                    refs: vec![r],
                    patterns: vec![None],
                    reach: NodeSet::new(0),
                }),
            }
        }
        if sites.is_empty() {
            return (patch, failed);
        }
        let Some(walked_buf) = self.walked_files() else {
            // Nothing can be resolved: every declared load fails.
            failed.extend(extra.iter().map(|x| (x.node, x.key)));
            return (patch, failed);
        };
        let walked: Vec<&str> = crate::manifest::files_in(&walked_buf).collect();
        turborepo_trace_core::timing::mark("derive: walked files");
        let n = self.nodes.len() as u32;
        let rev = self.fwd.transpose();
        turborepo_trace_core::timing::mark("derive: transpose");
        let mut stack: Vec<u32> = Vec::new();
        let runs = q::EdgeFilter::ALL.for_uncertainty();
        for s in &mut sites {
            let mut reach = NodeSet::new(n);
            reach.insert(NodeId(s.node));
            stack.clear();
            stack.push(s.node);
            while let Some(x) = stack.pop() {
                let (srcs, attrs) = rev.neighbors(NodeId(x));
                for (&p, &a) in srcs.iter().zip(attrs) {
                    // Only what runs the file runs it in its own working directory: a task
                    // that reads the file as an input, or a file that reads its text, does
                    // not (the edges uncertainty does not follow either).
                    if runs.accepts(a) && reach.insert(NodeId(p)) {
                        stack.push(p);
                    }
                }
            }
            s.reach = reach;
        }
        turborepo_trace_core::timing::mark("derive: reach");
        // Tests of one project share their working directories: what a site adds to a
        // test is computed once per (set of working directories, site).
        let mut test_nodes: Vec<u32> = tests.keys().copied().collect();
        test_nodes.sort_unstable();
        // The root is one more working directory of a test whose runner can be started
        // there (see the module documentation).
        let rooted = self.tests_run_from_root();
        let assume = !self.cwd_root_always;
        let mut set_ids: HashMap<(&[&str], bool), u32> = HashMap::new();
        let mut sets: Vec<(&[&str], bool)> = Vec::new();
        let mut set_of: Vec<u32> = vec![u32::MAX; n as usize];
        for &t in &test_nodes {
            let dirs = tests[&t].as_slice();
            let key = (dirs, rooted.contains(&t));
            let id = *set_ids.entry(key).or_insert_with(|| {
                sets.push(key);
                sets.len() as u32 - 1
            });
            set_of[t as usize] = id;
        }
        let exists = |p: &str| {
            if walked.binary_search(&p).is_ok() {
                return true;
            }
            let prefix = format!("{p}/");
            let lo = walked.partition_point(|f| *f < prefix.as_str());
            walked
                .get(lo)
                .is_some_and(|f| f.starts_with(prefix.as_str()))
        };
        let mut notes: Vec<(u32, String)> = Vec::new();
        /// What one site adds to every test of one set of working directories.
        #[derive(Default)]
        struct Bundle {
            /// `(target, attr, label)`.
            edges: Vec<(u32, u16, u32)>,
            probes: Vec<u32>,
            inputs: Vec<u32>,
            members: Vec<(u32, u32)>,
            /// Sites the edges' targets reach.
            through: Vec<u32>,
            /// Readings from the root that were not taken ([`explained`]): a
            /// sentence each.
            assumed: Vec<String>,
            /// The same, as records ([`Graph::cwd_dropped`]).
            dropped: Vec<CwdDropped>,
        }
        let mut dropped: Vec<CwdDropped> = Vec::new();
        let mut bundles: rustc_hash::FxHashMap<(u32, u32), Bundle> = Default::default();
        let mut resolutions = 0usize;
        // (test, site) pairs, to a fixed point: an edge added to a test can reach more
        // sites.
        let mut work: Vec<(u32, u32)> = Vec::new();
        for (si, s) in sites.iter().enumerate() {
            for &t in &test_nodes {
                if s.reach.contains(NodeId(t)) {
                    work.push((t, si as u32));
                }
            }
        }
        let mut done: Vec<NodeSet> = sites.iter().map(|_| NodeSet::new(n)).collect();
        let mut pairs = 0usize;
        while let Some((t, si)) = work.pop() {
            if !done[si as usize].insert(NodeId(t)) {
                continue;
            }
            pairs += 1;
            let key = (set_of[t as usize], si);
            let b = match bundles.entry(key) {
                std::collections::hash_map::Entry::Occupied(o) => o.into_mut(),
                std::collections::hash_map::Entry::Vacant(slot) => {
                    let site = &sites[si as usize];
                    let mut b = Bundle::default();
                    let (dirs, rooted) = sets[key.0 as usize];
                    // The root as a working directory that the set does not name.
                    let implicit = rooted && !dirs.contains(&"");
                    for (r, pat) in site.refs.iter().zip(&site.patterns) {
                        for cwd in dirs.iter().copied().chain(implicit.then_some("")) {
                            // From the root, what a path that climbs above it and comes
                            // back names is a reading, not a path of the repository: not
                            // taken where another working directory explains the path. The
                            // root that the set does not name is read for such paths only
                            // (the tracer's edges on the file cover every other path).
                            let mut at_root: Option<SiteRef> = None;
                            if cwd.is_empty() {
                                let all: Vec<String> = match pat {
                                    Some(p) => match p.first() {
                                        Some(PatTok::Lit(first)) => first
                                            .rfind('/')
                                            .map(|cut| first[..cut].to_owned())
                                            .into_iter()
                                            .collect(),
                                        _ => Vec::new(),
                                    },
                                    None => r.specs.clone().unwrap_or_default(),
                                };
                                let mut kept: Vec<String> = Vec::new();
                                for spec in &all {
                                    let back = climbs_back(spec);
                                    if !back && implicit {
                                        continue;
                                    }
                                    // A test whose only working directory is the
                                    // root runs the reference there and nowhere else:
                                    // no other directory can be the one the path was
                                    // written for, and the reading is the only one.
                                    let elsewhere = dirs.iter().any(|d| !d.is_empty());
                                    let by = (back && assume && elsewhere)
                                        .then(|| {
                                            // The test's own directories, and the
                                            // directory of the package that holds the
                                            // reference (a script is written for it).
                                            dirs.iter()
                                                .copied()
                                                .chain(std::iter::once(site.pkg.as_str()))
                                                .find_map(|d| {
                                                    explained(d, spec, exists).map(|at| (d, at))
                                                })
                                        })
                                        .flatten();
                                    match by {
                                        Some((dir, at)) => {
                                            b.assumed.push(assumed_sentence(
                                                self.path_of(site.node as usize),
                                                r.line,
                                                spec,
                                                dir,
                                                &at,
                                            ));
                                            b.dropped.push(CwdDropped::new(
                                                self.path_of(site.node as usize),
                                                r.line,
                                                spec,
                                                dir,
                                                &at,
                                            ));
                                        }
                                        None => kept.push(spec.clone()),
                                    }
                                }
                                if kept.len() != all.len() || implicit {
                                    if kept.is_empty() {
                                        continue;
                                    }
                                    if pat.is_none() {
                                        at_root = Some(SiteRef {
                                            kind: r.kind,
                                            attr: r.attr,
                                            specs: Some(kept),
                                            line: r.line,
                                            unc_span: r.unc_span,
                                            js: r.js,
                                        });
                                    }
                                }
                            }
                            resolutions += 1;
                            let res = match pat {
                                Some(p) => self.resolve_pattern_in_cwd(&walked, cwd, p),
                                None => {
                                    self.resolve_in_cwd(&walked, cwd, at_root.as_ref().unwrap_or(r))
                                }
                            };
                            let Some(res) = res else {
                                failed.insert((site.node, r.unc_span));
                                continue;
                            };
                            if !res.edges.is_empty() {
                                let shown = match pat {
                                    Some(p) => show_pattern(p),
                                    None => r.specs.as_deref().unwrap_or_default().join(" | "),
                                };
                                let label = patch.intern(&cwd_edge_label(
                                    cwd,
                                    &shown,
                                    self.path_of(site.node as usize),
                                    r.line,
                                ));
                                b.edges
                                    .extend(res.edges.iter().map(|&x| (x, r.attr, label)));
                            }
                            b.probes.extend(res.probes.iter().map(|x| patch.intern(x)));
                            b.inputs.extend(res.inputs.iter().map(|x| patch.intern(x)));
                            b.members.extend(
                                res.members
                                    .iter()
                                    .map(|(d, x)| (patch.intern(d), patch.intern(x))),
                            );
                        }
                    }
                    b.edges.sort_unstable();
                    b.edges.dedup();
                    b.probes.sort_unstable();
                    b.probes.dedup();
                    b.inputs.sort_unstable();
                    b.inputs.dedup();
                    b.members.sort_unstable();
                    b.members.dedup();
                    for (sj, other) in sites.iter().enumerate() {
                        // A file the test only reads does not run in the test either.
                        if b.edges.iter().any(|e| {
                            runs.accepts(EdgeAttr(e.1)) && other.reach.contains(NodeId(e.0))
                        }) {
                            b.through.push(sj as u32);
                        }
                    }
                    slot.insert(b)
                }
            };
            let from = NodeId(t);
            notes.extend(b.assumed.iter().map(|s| (t, s.clone())));
            dropped.extend(b.dropped.iter().cloned());
            for &(x, attr, label) in &b.edges {
                patch.edge_id(from, NodeId(x), EdgeAttr(attr), label);
            }
            for &x in &b.probes {
                patch.probe_id(from, x);
            }
            for &x in &b.inputs {
                patch.input_id(from, x);
            }
            for &(d, x) in &b.members {
                patch.membership_id(from, d, x);
            }
            for &sj in &b.through {
                if !done[sj as usize].contains(from) {
                    work.push((t, sj));
                }
            }
        }
        if turborepo_trace_core::timing::enabled() {
            turborepo_trace_core::timing::mark(&format!(
                "derive: {pairs} pair(s), {} bundle(s), {resolutions} resolution(s), {} site \
                 file(s)",
                bundles.len(),
                sites.len()
            ));
        }
        // Records every one of whose references resolved, for every pair.
        let cwd_kind = q::UncertaintyKind::CwdDependent;
        for site in &sites {
            let node = self.nodes[site.node as usize];
            let a = self.shards[node.shard as usize].get();
            let f = &a.files[node.file as usize];
            for u in f.uncertainties.iter() {
                let span = [u.span[0].to_native(), u.span[1].to_native()];
                if crate::convert::query_unc_kind(u.kind) != cwd_kind
                    || u.scope.is_some()
                    || failed.contains(&(site.node, span))
                    || !site.refs.iter().any(|r| r.unc_span == span)
                {
                    continue;
                }
                let detail = a.details[u.detail.to_native() as usize].as_str();
                patch.resolve_for_entries(
                    NodeId(site.node),
                    cwd_kind,
                    u.line.to_native(),
                    span_of(&u.span),
                    &format!("{detail}; {RESOLVED_NOTE}"),
                );
            }
        }
        let mut entries = NodeSet::new(n);
        for &t in &test_nodes {
            entries.insert(NodeId(t));
        }
        patch.set_resolved_entries(entries);
        *self
            .cwd_notes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = notes;
        dropped.sort();
        dropped.dedup();
        *self
            .cwd_dropped
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = dropped;
        (patch, failed)
    }
}

#[cfg(test)]
mod root_tests {
    use super::{assumed_sentence, climb, climbs_back, explained};

    /// A path that climbs above the root from the root and comes back in is
    /// explained by a working directory from which it stays inside and goes
    /// through an existing directory where the reading from the root takes
    /// the names above the checkout.
    #[test]
    fn a_working_directory_explains_a_climb_through_an_existing_directory() {
        assert_eq!(
            climb("../../packages/ui/dist/a.svg"),
            Some((2, vec!["packages", "ui", "dist", "a.svg"]))
        );
        assert_eq!(climb("a/../../b/./c"), Some((1, vec!["b", "c"])));
        assert_eq!(climb("/abs/x"), None);
        assert!(climbs_back("../repo/data/x.json"));
        assert!(climbs_back("../../packages/ui/dist/a.svg"));
        for inside_or_above in ["data/x.json", "../repo", "../..", "../.git/x", "/abs/x"] {
            assert!(!climbs_back(inside_or_above), "{inside_or_above}");
        }
        let dirs = ["packages/ui", "apps/web", "data"];
        let exists = |p: &str| dirs.contains(&p);
        // From apps/web the path goes through packages/ui, which exists.
        assert_eq!(
            explained("apps/web", "../../packages/ui/dist/a.svg", exists),
            Some("packages/ui".to_owned())
        );
        // From packages/a, `../w1/…` goes through packages/w1, which does not.
        assert_eq!(explained("packages/a", "../w1/data/x.json", exists), None);
        // From a directory the path leaves the repository from, nothing is explained,
        // and the root explains nothing.
        assert_eq!(
            explained("apps", "../../packages/ui/dist/a.svg", exists),
            None
        );
        assert_eq!(explained("", "../data/x.json", exists), None);
        // A deeper directory: the climb lands below the root.
        assert_eq!(
            explained("apps/web/e2e", "../../packages/ui/x", |p| p
                == "apps/packages/ui"),
            Some("apps/packages/ui".to_owned())
        );
        assert_eq!(
            assumed_sentence(
                "apps/web/src/gen.test.js",
                3,
                "../../packages/ui/dist/gen.json",
                "apps/web",
                "packages/ui"
            ),
            "apps/web/src/gen.test.js:3 reads ../../packages/ui/dist/gen.json relative to the \
             working directory; from apps/web/ that path goes through packages/ui/, which exists, \
             so it is not also read from the repository root, where it would name dist/gen.json \
             if the repository is checked out in a directory called packages/ui"
        );
    }
}

#[cfg(test)]
mod label_tests {
    use super::{cwd_edge_label, cwd_edge_site};

    #[test]
    fn the_site_of_a_working_directory_edge_is_read_back_from_its_label() {
        let l = cwd_edge_label("services/a", "./src/routes", "packages/helper/setup.ts", 89);
        assert_eq!(cwd_edge_site(&l), Some("packages/helper/setup.ts"));
        let l = cwd_edge_label("", "a | b (c)", "app/(group)/load.ts", 7);
        assert_eq!(cwd_edge_site(&l), Some("app/(group)/load.ts"));
        assert_eq!(cwd_edge_site("hint turbotrace.toml:3"), None);
        assert_eq!(cwd_edge_site("cwd services/a: ./x"), None);
    }
}
