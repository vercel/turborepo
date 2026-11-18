//! The selection document: `affected --format selection`.
//!
//! One JSON document for a CI pipeline. It carries two things side by side:
//!
//! - what the selection **would** run: per package, the `selected` test files
//!   with the class of their reason, the `unselected` ones (known and cleared)
//!   and the `unknown` ones (files that look like tests and are no entry of
//!   this evaluation);
//! - what the caller **must** run: `run`, at the top and per package. It is
//!   decided by the [`Mode`]: `shadow` (the default) makes it the full set,
//!   `enforce` makes it the selection. It is also the full set whenever
//!   everything must run for another reason: a file that steers the selection
//!   changed, the diff could not be read, an error.
//!
//! So one integration serves both positions of the switch: the pipeline always
//! reads `run`, and the report after the tests ([`crate::ci::compare`]) reads
//! `selected`.
//!
//! The document is a function of the two source trees, the repository's
//! configuration and the mode: static analysis only. Nothing an earlier run
//! produced is read for it (no test results, timings, recordings or reports); a
//! restored graph (`--restore`) is checked against the working tree and only
//! saves time. That is why end-to-end specs selected by recorded visits
//! (`--tests playwright`) are refused in this format.
//!
//! The schema is versioned ([`VERSION`]). A reader must treat an unknown
//! `version`, an unreadable document or a value of `run` it does not know as
//! "run everything".

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use turborepo_trace_graph::{
    AffectedReport, Cause, Change, EntryResult, GraphView, NodeId, NodeSet, PackageId,
    PackageVersion, Verdict, glob::Glob,
};
use turborepo_trace_store::Graph;

/// The `schema` field of the document.
pub(crate) const SCHEMA: &str = "turbotrace-selection";
/// The `version` field: bumped when a field changes meaning or goes away. New
/// fields don't bump it.
pub(crate) const VERSION: u32 = 1;
/// The debug variable of the switch ([`Mode`]): `TURBO_TRACE_DEBUG_MODE`.
pub(crate) const MODE_ENV: &str = "MODE";
/// The switch: what `run` carries.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// The full set runs; the selection is computed and recorded next to it.
    Shadow,
    /// The selection runs.
    Enforce,
}

/// Where the mode came from.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ModeSource {
    /// Neither the flag nor the environment variable was given.
    Default,
    /// `--mode`.
    Flag,
    /// `TURBO_TRACE_DEBUG_MODE`.
    Env,
}

/// The mode of this run: `--mode`, else `TURBO_TRACE_DEBUG_MODE`, else
/// `shadow`. A value of the variable that is neither `shadow` nor `enforce` is
/// `shadow`, with a note.
pub(crate) fn resolve_mode(flag: Option<Mode>) -> (Mode, ModeSource, Option<String>) {
    resolve_mode_from(
        flag,
        turborepo_trace_core::debug::var(MODE_ENV).ok().as_deref(),
    )
}

fn resolve_mode_from(flag: Option<Mode>, env: Option<&str>) -> (Mode, ModeSource, Option<String>) {
    if let Some(m) = flag {
        return (m, ModeSource::Flag, None);
    }
    match env.map(str::trim).filter(|v| !v.is_empty()) {
        None => (Mode::Shadow, ModeSource::Default, None),
        Some(v) if v.eq_ignore_ascii_case("enforce") => (Mode::Enforce, ModeSource::Env, None),
        Some(v) if v.eq_ignore_ascii_case("shadow") => (Mode::Shadow, ModeSource::Env, None),
        Some(v) => (
            Mode::Shadow,
            ModeSource::Env,
            Some(format!(
                "TURBO_TRACE_DEBUG_{MODE_ENV}={v:?} is neither `shadow` nor `enforce`: the full \
                 set runs"
            )),
        ),
    }
}

/// What must run: of the whole document, or of one package or task.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Run {
    /// Every test.
    All,
    /// The `selected` tests and the `unknown` ones; the `unselected` ones may
    /// be skipped.
    Selected,
    /// Nothing is selected and nothing is unknown: the package or task need not
    /// start. (Never the value of the document's own `run`.)
    None,
}

/// Why `run` is what it is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Reason {
    /// `shadow` or `enforce` (the mode decided), `inputs-changed` (a file that
    /// steers the selection changed), `diff-unreadable` (the base revision
    /// or the diff could not be read), `error` (anything else failed).
    pub(crate) code: String,
    /// One readable sentence.
    pub(crate) detail: String,
    /// For `inputs-changed`: the changed paths.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) paths: Vec<String>,
}

/// One selected test file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Selected {
    /// Relative to the repository root.
    pub(crate) path: String,
    /// The class of the chain `--explain` prints for it: `test-changed` (the
    /// file itself), `change` (a changed file reaches it), `dependency` (a
    /// third-party package resolves differently), `name` (a string of
    /// loaded code names a changed path), `task` (a CI job, a script or a
    /// service that runs it changed), `uncertain` (it reaches code that can't
    /// be analyzed: selected on every change), `unknown` (the same when
    /// uncertain tests are reported as unknown), `not-in-graph` (an entry that
    /// is no traced source file).
    pub(crate) reason: String,
    /// What the chain starts at: the changed path, the package, or the file
    /// with the uncertain site. Absent for `test-changed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) via: Option<String>,
    /// The frameworks that run the file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) frameworks: Vec<String>,
}

/// What one test launch of a task covers.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct Launch {
    /// `jest`, `vitest`, ...
    pub(crate) framework: String,
    /// The directory it runs in, relative to the repository root (`.`: the
    /// root).
    pub(crate) dir: String,
}

/// A package script that starts tests.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Task {
    /// The task id in the graph: `task:<package dir>#<script>`.
    pub(crate) id: String,
    /// The script name (`test`): with the package name, the id a task runner
    /// knows it by.
    pub(crate) name: String,
    /// The test launches of its command, with those of the scripts the command
    /// starts.
    pub(crate) runs: Vec<Launch>,
    /// Whether the task must start, and with what.
    pub(crate) run: Run,
    /// Known test files its launches cover.
    pub(crate) tests: usize,
    /// How many of them are selected.
    pub(crate) selected: usize,
    /// Set when the command starts tests in a way that can't be mapped to
    /// files: the task always starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) unmapped: Option<String>,
}

/// The tests of one package.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Package {
    /// The package name (`@acme/web`); the directory when it has none.
    pub(crate) name: String,
    /// Its directory relative to the repository root (`.`: the root package).
    pub(crate) dir: String,
    /// What must run: `all`, `selected` (its `selected` and `unknown` files;
    /// `unselected` may be skipped) or `none` (the package's tests need not
    /// start).
    pub(crate) run: Run,
    /// The same under `enforce`, whatever the mode of this document: what
    /// flipping the switch would make of `run`.
    pub(crate) would_run: Run,
    /// Known test files: `selected` plus `unselected`.
    pub(crate) tests: usize,
    /// What the selection would run.
    #[serde(default)]
    pub(crate) selected: Vec<Selected>,
    /// Known test files that the change can't affect.
    #[serde(default)]
    pub(crate) unselected: Vec<String>,
    /// Files that look like tests and are no entry of this evaluation: they
    /// always run.
    #[serde(default)]
    pub(crate) unknown: Vec<String>,
    /// The scripts that start tests.
    #[serde(default)]
    pub(crate) tasks: Vec<Task>,
}

/// Totals.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Counts {
    /// Known test files.
    pub(crate) tests: usize,
    /// Selected test files.
    pub(crate) selected: usize,
    /// Known test files that may be skipped under this document's `run`.
    pub(crate) skipped: usize,
    /// Files that look like tests and are not known.
    pub(crate) unknown: usize,
    /// Packages with tests.
    pub(crate) packages: usize,
    /// Packages whose tests start under this document's `run`.
    pub(crate) packages_run: usize,
    /// Packages whose tests would start under `enforce`.
    pub(crate) packages_selected: usize,
}

/// The document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Document {
    /// [`SCHEMA`].
    pub(crate) schema: String,
    /// [`VERSION`].
    pub(crate) version: u32,
    /// The trace version that wrote it.
    #[serde(default)]
    pub(crate) turbotrace: String,
    /// The position of the switch.
    pub(crate) mode: Mode,
    /// Where the mode came from.
    pub(crate) mode_source: ModeSource,
    /// What the caller must run: `all` or `selected`.
    pub(crate) run: Run,
    /// Why.
    pub(crate) reason: Reason,
    /// The base revision (`--since`), as given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) base: Option<String>,
    /// The commit the working tree was compared with: the merge base of `base`
    /// and HEAD, or `base` itself with `--exact-base`. Absent when it could
    /// not be told.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) base_commit: Option<String>,
    /// Every merge base, when the history has several (a criss-cross merge):
    /// the changes against each count.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) merge_bases: Vec<String>,
    /// The `--tests` frameworks.
    #[serde(default)]
    pub(crate) frameworks: Vec<String>,
    /// Totals.
    #[serde(default)]
    pub(crate) counts: Counts,
    /// Per package, sorted by directory.
    #[serde(default)]
    pub(crate) packages: Vec<Package>,
}

impl Document {
    /// The full-set document of a failed run: no lists, `run: all`.
    pub(crate) fn failed(
        mode: Mode,
        mode_source: ModeSource,
        code: &str,
        detail: String,
        base: Option<String>,
        frameworks: Vec<String>,
    ) -> Document {
        Document {
            schema: SCHEMA.to_owned(),
            version: VERSION,
            turbotrace: env!("CARGO_PKG_VERSION").to_owned(),
            mode,
            mode_source,
            run: Run::All,
            reason: Reason {
                code: code.to_owned(),
                detail,
                paths: Vec::new(),
            },
            base,
            base_commit: None,
            merge_bases: Vec::new(),
            frameworks,
            counts: Counts::default(),
            packages: Vec::new(),
        }
    }

    /// Parses and validates a document.
    #[cfg(test)]
    pub(crate) fn parse(text: &str) -> anyhow::Result<Document> {
        #[derive(Deserialize)]
        struct Head {
            #[serde(default)]
            schema: String,
            #[serde(default)]
            version: u32,
        }
        let head: Head = serde_json::from_str(text)
            .map_err(|e| anyhow::anyhow!("not a selection document: {e}"))?;
        if head.schema != SCHEMA {
            anyhow::bail!("schema is {:?}, not {SCHEMA:?}", head.schema);
        }
        if head.version != VERSION {
            anyhow::bail!(
                "version {} is not the version {VERSION} this build reads",
                head.version
            );
        }
        let doc: Document = serde_json::from_str(text)
            .map_err(|e| anyhow::anyhow!("not a version {VERSION} selection document: {e}"))?;
        if doc.run == Run::None {
            anyhow::bail!("the document's `run` is `none`");
        }
        Ok(doc)
    }
}

/// What [`build`] reads.
pub(crate) struct Input<'a> {
    pub(crate) g: &'a Graph,
    pub(crate) report: &'a AffectedReport,
    /// The entries of the evaluation.
    pub(crate) entries: &'a NodeSet,
    /// Entries that exist and are no graph node, with the verdict the policy
    /// gives them.
    pub(crate) missing: &'a [String],
    pub(crate) missing_verdict: Option<Verdict>,
    /// Every changed path, before any is set aside.
    pub(crate) changes: &'a [Change],
    pub(crate) frameworks: &'a [String],
    pub(crate) base: Option<&'a str>,
    /// The commit `base` resolved to, and every merge base when there are
    /// several.
    pub(crate) base_commit: Option<&'a str>,
    pub(crate) merge_bases: &'a [String],
    pub(crate) mode: Mode,
    pub(crate) mode_source: ModeSource,
    /// `--run-all-if-changed` globs.
    pub(crate) run_all: &'a [Glob],
    /// `--not-tests` globs.
    pub(crate) not_tests: &'a [Glob],
    /// `--full-package` globs over package directories.
    pub(crate) full_packages: &'a [Glob],
}

/// The files that look like tests of the asked `frameworks` and are no entry
/// of the evaluation (`entries`): the selection does not know what they read,
/// so they always run (the document's `unknown`, `UnknownTest` in `turbo
/// trace`).
pub(crate) fn unknown_tests(
    g: &Graph,
    entries: &NodeSet,
    frameworks: &[String],
    not_tests: &[Glob],
) -> Vec<NodeId> {
    let asked = |f: &str| frameworks.iter().any(|t| t == f || t == "all");
    let (js, py, go) = (
        asked("jest") || asked("vitest"),
        asked("pytest"),
        asked("gotest"),
    );
    if !(js || py || go) {
        return Vec::new();
    }
    // The tests of a framework that was not asked for are known tests, of that
    // framework: not unknown ones of this evaluation (`--tests jest` in a vitest
    // repository lists nothing).
    let mut others = NodeSet::new(g.node_count());
    // Packages where jest or vitest found tests: a config of theirs decides what
    // a test is there. Elsewhere jest and vitest run with their defaults.
    let mut configured = std::collections::HashSet::new();
    for (fw, _) in turborepo_trace_store::fw::GRAPH_FRAMEWORKS {
        let tests = g.tests(Some(*fw));
        if matches!(*fw, "jest" | "vitest") {
            configured.extend(tests.iter().map(|&t| g.package(t)));
        }
        if !asked(fw) {
            for t in tests {
                others.insert(t);
            }
        }
    }
    let defaults = JsDefaults::new(asked("jest"), asked("vitest"));
    (0..g.node_count())
        .map(NodeId)
        .filter(|&n| {
            !(entries.contains(n)
                || others.contains(n)
                || g.is_external(n)
                || g.lang(n).is_none()
                || turborepo_trace_store::tasks::is_pseudo_lang(g.lang(n)))
        })
        .filter(|&n| {
            let path = g.path(n);
            let by_default = || !configured.contains(&g.package(n)) && defaults.matches(path);
            (test_like(path, js, py, go) || by_default())
                && !not_tests.iter().any(|x| x.matches(path))
        })
        .collect()
}

/// The test files jest and vitest take without a config: jest's default
/// `testMatch` (`__tests__/**`, `test.js`, `spec.js`, `*.test.js`, ...) and
/// vitest's default `include`, for the frameworks asked for. vitest's default
/// `exclude` is not applied: it is relative to the project's root, and a file
/// it would leave out counts as one more unknown test, which runs.
struct JsDefaults(Vec<turborepo_trace_frameworks::pattern::CompiledPattern>);

impl JsDefaults {
    fn new(jest: bool, vitest: bool) -> Self {
        use turborepo_trace_frameworks::{
            TestPattern, jest as j, pattern::CompiledPattern, vitest as v,
        };
        let globs = jest
            .then_some(j::DEFAULT_TEST_MATCH)
            .into_iter()
            .chain(vitest.then_some(v::DEFAULT_INCLUDE))
            .flatten();
        JsDefaults(
            globs
                .map(|g| CompiledPattern::new(&TestPattern::Glob((*g).to_owned())))
                .collect(),
        )
    }

    /// Whether `path` (repository-relative) is a test file by these defaults.
    fn matches(&self, path: &str) -> bool {
        let abs = format!("/{path}");
        self.0.iter().any(|p| p.matches(&abs))
    }
}

/// Whether a path looks like a test file of a framework family that was asked
/// for.
fn test_like(path: &str, js: bool, py: bool, go: bool) -> bool {
    let base = path.rsplit('/').next().unwrap_or(path);
    let Some((stem, ext)) = base.rsplit_once('.') else {
        return false;
    };
    match ext {
        "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" | "mts" | "cts" => {
            js && (stem.ends_with(".test") || stem.ends_with(".spec"))
        }
        "py" => py && (stem.starts_with("test_") || stem.ends_with("_test")),
        "go" => go && stem.ends_with("_test"),
        _ => false,
    }
}

/// The class of an entry's reason and what its chain starts at.
fn reason_of(g: &Graph, e: &EntryResult) -> (&'static str, Option<String>) {
    let entry = g.path(e.entry);
    let site = || {
        e.uncertain
            .as_ref()
            .map(|u| g.path(u.node).to_owned())
            .filter(|p| p != entry)
    };
    if e.verdict == Verdict::Unknown {
        return ("unknown", site());
    }
    let first_is_task = e
        .witness
        .first()
        .is_some_and(|&n| turborepo_trace_store::tasks::is_pseudo_lang(g.lang(n)));
    let from = |path: &str| {
        if path == entry {
            ("test-changed", None)
        } else if first_is_task {
            ("task", Some(path.to_owned()))
        } else {
            ("change", Some(path.to_owned()))
        }
    };
    match (&e.cause, &e.uncertain) {
        (Some(Cause::Package { package }), _) => {
            let info = g.package_info(*package);
            (
                "dependency",
                Some(format!("{}@{}", info.name, info.version.as_str())),
            )
        }
        (Some(Cause::Named { path, .. }), _) => ("name", Some(path.clone())),
        (Some(Cause::Modified { path } | Cause::Deleted { path }), _) => from(path),
        (Some(Cause::Probe { added, .. }), _) => from(added),
        (Some(Cause::Membership { path, .. } | Cause::Scope { path, .. }), _) => from(path),
        (None, Some(_)) => ("uncertain", site()),
        (None, None) => (
            "change",
            e.witness
                .first()
                .map(|&n| g.path(n).to_owned())
                .filter(|p| p != entry),
        ),
    }
}

/// The entries of `sorted` (by path) whose path is `dir` or below it, as
/// [`under`] says: `dir` itself, then the contiguous range of paths that start
/// with `dir/`.
fn below<'s, 'p>(sorted: &'s [(&'p str, NodeId)], dir: &str) -> Vec<&'s (&'p str, NodeId)> {
    let dir = dir.trim_end_matches('/');
    if dir.is_empty() || dir == "." {
        return sorted.iter().collect();
    }
    let eq = sorted.partition_point(|(p, _)| *p < dir);
    let n = sorted[eq..].partition_point(|(p, _)| *p == dir);
    let mut out: Vec<&(&str, NodeId)> = sorted[eq..eq + n].iter().collect();
    let prefix = format!("{dir}/");
    let lo = sorted.partition_point(|(p, _)| *p < prefix.as_str());
    let len = sorted[lo..].partition_point(|(p, _)| p.starts_with(prefix.as_str()));
    out.extend(&sorted[lo..lo + len]);
    out
}

/// Whether `path` is `dir` or below it (`.` and the empty string: everything).
fn under(dir: &str, path: &str) -> bool {
    let dir = dir.trim_end_matches('/');
    dir.is_empty()
        || dir == "."
        || path == dir
        || path
            .strip_prefix(dir)
            .is_some_and(|rest| rest.starts_with('/'))
}

#[derive(Default)]
struct Acc {
    name: String,
    selected: Vec<Selected>,
    unselected: Vec<String>,
    unknown: Vec<String>,
    tasks: Vec<Task>,
}

/// Builds the document of an evaluation.
pub(crate) fn build(i: &Input<'_>) -> Document {
    let g = i.g;
    // First-party packages by directory (`.`: the root), for paths that are no
    // node.
    let mut by_dir: BTreeMap<String, String> = BTreeMap::new();
    for p in 0..g.package_count() {
        let info = g.package_info(PackageId(p));
        if matches!(info.version, PackageVersion::Workspace) {
            let dir = if info.root.is_empty() { "." } else { info.root };
            by_dir.insert(dir.to_owned(), info.name.to_owned());
        }
    }
    let dir_of_node = |n: NodeId| -> String {
        let root = g.package_info(g.package(n)).root;
        if root.is_empty() {
            ".".to_owned()
        } else {
            root.to_owned()
        }
    };
    let dir_of_path = |path: &str| -> String {
        let mut dir = path;
        while let Some((parent, _)) = dir.rsplit_once('/') {
            if by_dir.contains_key(parent) {
                return parent.to_owned();
            }
            dir = parent;
        }
        ".".to_owned()
    };
    let mut acc: BTreeMap<String, Acc> = BTreeMap::new();

    // 1. Selected and unselected entries.
    let mut hit = NodeSet::new(g.node_count());
    for e in &i.report.entries {
        if !matches!(e.verdict, Verdict::Affected | Verdict::Unknown) {
            continue;
        }
        hit.insert(e.entry);
        let (reason, via) = reason_of(g, e);
        acc.entry(dir_of_node(e.entry))
            .or_default()
            .selected
            .push(Selected {
                path: g.path(e.entry).to_owned(),
                reason: reason.to_owned(),
                via,
                frameworks: g
                    .test_frameworks(e.entry)
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
            });
    }
    for n in i.entries.iter() {
        if !hit.contains(n) {
            acc.entry(dir_of_node(n))
                .or_default()
                .unselected
                .push(g.path(n).to_owned());
        }
    }
    // Entries that are no graph node: selected unless the policy ignores
    // uncertainty.
    for m in i.missing {
        let a = acc.entry(dir_of_path(m)).or_default();
        match i.missing_verdict {
            Some(_) => a.selected.push(Selected {
                path: m.clone(),
                reason: "not-in-graph".to_owned(),
                via: None,
                frameworks: Vec::new(),
            }),
            None => a.unselected.push(m.clone()),
        }
    }

    // 2. Files that look like tests and are no entry.
    let asked = |f: &str| i.frameworks.iter().any(|t| t == f || t == "all");
    for n in unknown_tests(g, i.entries, i.frameworks, i.not_tests) {
        acc.entry(dir_of_node(n))
            .or_default()
            .unknown
            .push(g.path(n).to_owned());
    }

    // 3. The scripts that start tests.
    let runs = crate::workflows::Runs::new(g);
    let selected_paths: BTreeSet<&str> = acc
        .values()
        .flat_map(|a| a.selected.iter().map(|s| s.path.as_str()))
        .collect();
    // Per framework, its tests that are entries, by path: the tests below a
    // launch directory are one range of them (no scan of every test per task).
    let mut tests_of: BTreeMap<&str, Vec<(&str, NodeId)>> = BTreeMap::new();
    let mut tasks: Vec<(String, Task)> = Vec::new();
    for (n, rec) in g.tasks() {
        let id = rec.id.as_str();
        let Some((dir, name)) = id.strip_prefix("task:").and_then(|r| r.rsplit_once('#')) else {
            continue;
        };
        let launched = runs.of_command(n);
        if !launched.any() {
            continue;
        }
        let (mut tests, mut selected) = (0usize, 0usize);
        let mut seen = NodeSet::new(g.node_count());
        let mut list = Vec::new();
        // A launch of a runner this selection has no verdicts for (mocha, `node
        // --test`, Playwright, or a framework that was not asked for): the task
        // always starts.
        let mut foreign: Option<String> = None;
        for (fw, d) in launched.launches() {
            list.push(Launch {
                framework: fw.to_owned(),
                dir: d.to_owned(),
            });
            let fw_static: &str = match turborepo_trace_store::fw::GRAPH_FRAMEWORKS
                .iter()
                .find(|(id, _)| *id == fw)
            {
                Some((id, _)) if asked(id) => id,
                _ => {
                    foreign.get_or_insert_with(|| {
                        format!("starts {fw} tests, which this selection does not cover")
                    });
                    continue;
                }
            };
            let all = tests_of.entry(fw_static).or_insert_with(|| {
                let mut v: Vec<(&str, NodeId)> = g
                    .tests(Some(fw_static))
                    .into_iter()
                    .filter(|&t| i.entries.contains(t))
                    .map(|t| (g.path(t), t))
                    .collect();
                v.sort_unstable();
                v
            });
            for &(path, t) in below(all, d) {
                if !seen.contains(t) {
                    seen.insert(t);
                    tests += 1;
                    if selected_paths.contains(path) {
                        selected += 1;
                    }
                }
            }
        }
        let dir = if dir.is_empty() { "." } else { dir };
        tasks.push((
            dir.to_owned(),
            Task {
                id: id.to_owned(),
                name: name.to_owned(),
                runs: list,
                run: Run::All,
                tests,
                selected,
                unmapped: launched.every_test().map(str::to_owned).or(foreign),
            },
        ));
    }
    for (dir, t) in tasks {
        // A task of a directory that is no package of the graph goes to the enclosing
        // one.
        let key = if by_dir.contains_key(&dir) || acc.contains_key(&dir) {
            dir
        } else {
            dir_of_path(&format!("{dir}/x"))
        };
        acc.entry(key).or_default().tasks.push(t);
    }

    // 4. Whether everything must run, and the document.
    let mut inputs: Vec<String> = i
        .changes
        .iter()
        .filter(|c| i.run_all.iter().any(|x| x.matches(&c.path)))
        .map(|c| c.path.clone())
        .collect();
    inputs.sort();
    inputs.dedup();
    let (run, reason) = if !inputs.is_empty() {
        (
            Run::All,
            Reason {
                code: "inputs-changed".to_owned(),
                detail: format!(
                    "{} changed path(s) steer the selection itself, so everything runs",
                    inputs.len()
                ),
                paths: inputs,
            },
        )
    } else {
        match i.mode {
            Mode::Shadow => (
                Run::All,
                Reason {
                    code: "shadow".to_owned(),
                    detail: "mode shadow: the full set runs and the selection is recorded"
                        .to_owned(),
                    paths: Vec::new(),
                },
            ),
            Mode::Enforce => (
                Run::Selected,
                Reason {
                    code: "enforce".to_owned(),
                    detail: "mode enforce: the selection runs, with every test turbo trace does \
                             not know"
                        .to_owned(),
                    paths: Vec::new(),
                },
            ),
        }
    };

    let mut counts = Counts::default();
    let mut packages = Vec::new();
    for (dir, mut a) in acc {
        a.selected.sort_by(|x, y| x.path.cmp(&y.path));
        a.selected.dedup_by(|x, y| x.path == y.path);
        a.unselected.sort();
        a.unselected.dedup();
        a.unknown.sort();
        a.unknown.dedup();
        a.tasks.sort_by(|x, y| x.id.cmp(&y.id));
        a.name = by_dir.get(&dir).cloned().unwrap_or_else(|| dir.clone());
        let unmapped = a.tasks.iter().any(|t| t.unmapped.is_some());
        // Under `enforce`: nothing to run, a part, or (nothing may be skipped)
        // everything. A package named by `--full-package` runs whole or not at
        // all: a part of its tests would break something it checks over the
        // whole run (a coverage threshold).
        let whole = i.full_packages.iter().any(|x| x.matches(&dir));
        let would_run = if a.unselected.is_empty()
            || (whole && (!a.selected.is_empty() || !a.unknown.is_empty() || unmapped))
        {
            Run::All
        } else if a.selected.is_empty() && a.unknown.is_empty() && !unmapped {
            Run::None
        } else {
            Run::Selected
        };
        let pkg_run = if run == Run::All { Run::All } else { would_run };
        for t in &mut a.tasks {
            let unknown_below = a
                .unknown
                .iter()
                .any(|u| t.runs.iter().any(|l| under(&l.dir, u)));
            let would = if t.unmapped.is_some() || (t.tests > 0 && t.selected == t.tests) {
                Run::All
            } else if t.selected == 0 && !unknown_below {
                Run::None
            } else {
                Run::Selected
            };
            t.run = if pkg_run == Run::All { Run::All } else { would };
        }
        let tests = a.selected.len() + a.unselected.len();
        counts.tests += tests;
        counts.selected += a.selected.len();
        counts.unknown += a.unknown.len();
        counts.packages += 1;
        if pkg_run != Run::All {
            counts.skipped += a.unselected.len();
        }
        if pkg_run != Run::None {
            counts.packages_run += 1;
        }
        if would_run != Run::None {
            counts.packages_selected += 1;
        }
        packages.push(Package {
            name: a.name,
            dir,
            run: pkg_run,
            would_run,
            tests,
            selected: a.selected,
            unselected: a.unselected,
            unknown: a.unknown,
            tasks: a.tasks,
        });
    }
    Document {
        schema: SCHEMA.to_owned(),
        version: VERSION,
        turbotrace: env!("CARGO_PKG_VERSION").to_owned(),
        mode: i.mode,
        mode_source: i.mode_source,
        run,
        reason,
        base: i.base.map(str::to_owned),
        base_commit: i.base_commit.map(str::to_owned),
        merge_bases: if i.merge_bases.len() > 1 {
            i.merge_bases.to_vec()
        } else {
            Vec::new()
        },
        frameworks: i.frameworks.to_vec(),
        counts,
        packages,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_precedence() {
        let m = |flag, env| {
            let (mode, source, note) = resolve_mode_from(flag, env);
            (mode, source, note.is_some())
        };
        assert_eq!(m(None, None), (Mode::Shadow, ModeSource::Default, false));
        assert_eq!(
            m(None, Some("")),
            (Mode::Shadow, ModeSource::Default, false)
        );
        assert_eq!(
            m(None, Some("enforce")),
            (Mode::Enforce, ModeSource::Env, false)
        );
        assert_eq!(
            m(None, Some(" Enforce ")),
            (Mode::Enforce, ModeSource::Env, false)
        );
        assert_eq!(
            m(None, Some("shadow")),
            (Mode::Shadow, ModeSource::Env, false)
        );
        // Anything else is the full set, and says so.
        assert_eq!(m(None, Some("on")), (Mode::Shadow, ModeSource::Env, true));
        // The flag wins.
        assert_eq!(
            m(Some(Mode::Shadow), Some("enforce")),
            (Mode::Shadow, ModeSource::Flag, false)
        );
        assert_eq!(
            m(Some(Mode::Enforce), Some("shadow")),
            (Mode::Enforce, ModeSource::Flag, false)
        );
    }

    #[test]
    fn below_is_under_over_a_sorted_list() {
        let mut v: Vec<(&str, NodeId)> = [
            "a",
            "a-b/t.test.js",
            "a.b/t.test.js",
            "a/t.test.js",
            "a/x/t.test.js",
            "ab/t.test.js",
            "b/t.test.js",
            "t.test.js",
        ]
        .iter()
        .enumerate()
        .map(|(k, p)| (*p, NodeId(k as u32)))
        .collect();
        v.sort_unstable();
        for dir in ["", ".", "a", "a/", "a/x", "a-b", "b", "c", "a/t.test.js"] {
            let fast: Vec<&str> = below(&v, dir).iter().map(|(p, _)| *p).collect();
            let slow: Vec<&str> = v
                .iter()
                .filter(|(p, _)| under(dir, p))
                .map(|(p, _)| *p)
                .collect();
            assert_eq!(fast, slow, "{dir:?}");
        }
    }

    #[test]
    fn jest_and_vitest_defaults_without_a_config() {
        let jest = JsDefaults::new(true, false);
        for p in [
            "pkg/__tests__/sum.js",
            "pkg/__tests__/deep/helper.mts",
            "pkg/test.js",
            "pkg/spec.js",
            "test.js",
            "pkg/src/a.test.tsx",
            "pkg/src/a.spec.cjs",
        ] {
            assert!(jest.matches(p), "jest: {p}");
        }
        for p in [
            "pkg/src/sum.js",
            "pkg/testing.js",
            "pkg/a.test.json",
            "pkg/latest.js",
        ] {
            assert!(!jest.matches(p), "jest: {p}");
        }
        let vitest = JsDefaults::new(false, true);
        assert!(vitest.matches("pkg/src/a.test.ts"));
        assert!(vitest.matches("pkg/a.spec.mjs"));
        for p in ["pkg/test.js", "pkg/__tests__/sum.js", "pkg/spec.js"] {
            assert!(!vitest.matches(p), "vitest: {p}");
        }
        assert!(!JsDefaults::new(false, false).matches("pkg/test.js"));
    }

    #[test]
    fn test_like_names() {
        let js = |p| test_like(p, true, false, false);
        assert!(js("a/b.test.ts"));
        assert!(js("a/b.spec.tsx"));
        assert!(js("b.test.mjs"));
        assert!(!js("a/__tests__/helper.ts"));
        assert!(!js("a/test.ts"));
        assert!(!js("a/b.test.json"));
        assert!(!js("a/b_test.go"));
        assert!(test_like("a/b_test.go", false, false, true));
        assert!(test_like("t/test_api.py", false, true, false));
        assert!(!test_like("t/test_api.py", true, false, false));
    }

    #[test]
    fn reads_only_its_own_version() {
        let doc = Document::failed(
            Mode::Shadow,
            ModeSource::Default,
            "error",
            "x".to_owned(),
            None,
            vec!["jest".to_owned()],
        );
        let text = serde_json::to_string(&doc).unwrap();
        assert_eq!(Document::parse(&text).unwrap(), doc);
        let other = text.replace("\"version\":1", "\"version\":2");
        assert!(
            Document::parse(&other)
                .unwrap_err()
                .to_string()
                .contains("version 2")
        );
        let other = text.replace(SCHEMA, "something-else");
        assert!(Document::parse(&other).is_err());
        assert!(Document::parse("[]").is_err());
        // Fields a later minor adds are ignored.
        let more = text.replacen('{', "{\"later\":true,", 1);
        assert_eq!(Document::parse(&more).unwrap(), doc);
    }
}
