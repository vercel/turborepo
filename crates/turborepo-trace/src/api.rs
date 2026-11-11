//! The typed API of `turbo trace`: a request in, a report out.

use std::{io::Write, path::PathBuf, sync::Arc};

use serde::Serialize;
use turbopath::AbsoluteSystemPath;
use turborepo_scm::SCM;
use turborepo_trace_graph::{ChangeList, ChangeSource, GivenChanges};

use crate::{
    Error,
    adapter::{Packages, ScmChanges, members, package_name},
    affected::{AffectedCmd, AffectedFormat},
    graph::GraphLoc,
    selection::{Document, Mode, Run},
};

/// What turbo knows about the repository (from its `Run`, which the caller
/// may drop once these are taken).
pub struct Repo<'a> {
    /// The repository root (`run.repo_root()`).
    pub root: &'a AbsoluteSystemPath,
    /// The invocation directory: relative paths in a request are relative to
    /// it.
    pub cwd: &'a AbsoluteSystemPath,
    /// From the package graph (`Packages::new(run.pkg_dep_graph())`): the
    /// JavaScript workspace members, and the package names of the report.
    pub packages: &'a Packages,
    /// The SCM layer (`run.scm()`): the changed files.
    pub scm: &'a SCM,
    /// The root `turbo.json` turbo reads (`run.opts().repo_opts
    /// .root_turbo_json_path`, `--root-turbo-json`). The trace reads
    /// `<root>/turbo.json` (or `turbo.jsonc`) only: with another one,
    /// `affected` selects every test ([`Reason::AllTestsChanged`]), and `why`
    /// and `lint` fail.
    pub root_turbo_json: &'a AbsoluteSystemPath,
}

/// Why the trace cannot read the repository's task configuration as turbo
/// does, if it cannot: a root `turbo.json` other than `<root>/turbo.json` or
/// `<root>/turbo.jsonc` (`--root-turbo-json`).
fn unsupported_config(repo: &Repo<'_>) -> Option<String> {
    let default = ["turbo.json", "turbo.jsonc"].iter().any(|name| {
        repo.root.join_component(name).as_std_path() == repo.root_turbo_json.as_std_path()
    });
    (!default).then(|| {
        format!(
            "the root turbo.json is {} (--root-turbo-json), and the trace reads only {}",
            repo.root_turbo_json,
            repo.root.join_component("turbo.json")
        )
    })
}

/// Where the changes come from.
#[derive(Clone, Debug)]
pub enum Changes {
    /// `SCM::changed_files` from `base` (resolved by turbo: default the GitHub
    /// Actions base, else `main` or `master`; the merge base with HEAD, as
    /// `turbo run --affected`) to `head` (default: the working tree,
    /// uncommitted and untracked files included). A ref that starts with `-`
    /// is refused ([`Error::InvalidRequest`]); any other SCM error selects
    /// every test ([`Reason::AllTestsChanged`]).
    Scm {
        /// `--base`.
        base: Option<String>,
        /// `--head`.
        head: Option<String>,
    },
    /// `--changed`: paths relative to [`Repo::cwd`], modified if the path
    /// exists and deleted otherwise. No base revision is read.
    Files(Vec<String>),
}

/// `turbo trace affected`.
#[derive(Clone, Debug)]
pub struct AffectedRequest {
    /// The changes.
    pub changes: Changes,
    /// `--tests`: frameworks (`jest`, `vitest`, `gotest`, `cargo-test`, `all`).
    /// Empty: `all`.
    pub tests: Vec<String>,
    /// `--strict`: every uncertainty selects, and no assumption is made.
    pub strict: bool,
    /// The output filter: package names (`--filter`, resolved by turbo).
    /// `None`: every package. The selection itself is never filtered.
    pub packages: Option<Vec<String>>,
}

/// The result of `turbo trace affected`.
#[derive(Clone, Debug, Serialize)]
pub struct AffectedTests {
    /// The selected tests, sorted by path.
    pub items: Vec<AffectedTest>,
    /// The tasks that start tests the trace cannot map to files (a runner it
    /// does not cover, a command it cannot read): each must run whole, sorted
    /// by task.
    pub uncovered_tasks: Vec<UncoveredTask>,
}

impl AffectedTests {
    /// turbo's `--exit-code` contract: 0 when nothing must run, 1 when a test
    /// is selected or a task must run whole (errors are 2, from the caller).
    pub fn exit_code(&self) -> i32 {
        i32::from(!self.items.is_empty() || !self.uncovered_tasks.is_empty())
    }
}

/// A task whose tests the trace cannot map to files: it runs whole.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UncoveredTask {
    /// The turbo task (`<package>#<script>`).
    pub task: String,
    /// turbo's name of its package (`//`: the root).
    pub package: String,
    /// Why its tests cannot be mapped.
    pub description: String,
}

/// One selected test.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AffectedTest {
    /// Relative to the repository root, `/`-separated.
    pub path: String,
    /// turbo's name of its package (`//`: the root).
    pub package: String,
    /// The turbo task that runs it (`<package>#<script>`), when one is known.
    pub task: Option<String>,
    /// Why it is selected.
    pub reason: Reason,
}

/// Why a test is selected (modelled on turbo's `TaskChangeReason`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "__typename")]
pub enum Reason {
    /// The test file itself changed.
    TestChanged,
    /// A changed file reaches it through what it loads.
    #[serde(rename_all = "camelCase")]
    FileChanged {
        /// The changed file the chain starts at.
        file_path: String,
    },
    /// A third-party package it uses resolves differently.
    #[serde(rename_all = "camelCase")]
    PackageDependencyChanged {
        /// The package.
        package_name: String,
    },
    /// A string in loaded code names a changed path.
    #[serde(rename_all = "camelCase")]
    NameReferenced {
        /// The changed path.
        file_path: String,
    },
    /// A CI job, a script or a service that runs it changed.
    #[serde(rename_all = "camelCase")]
    TaskChanged {
        /// The changed file.
        file_path: String,
    },
    /// It reaches code that cannot be analyzed statically: selected on every
    /// change.
    #[serde(rename_all = "camelCase")]
    Uncertain {
        /// The file with the uncertain site.
        file_path: String,
    },
    /// A test file that is not in the graph (not a traced source file).
    NotInGraph,
    /// A file that looks like a test, of a package whose list of tests the
    /// trace cannot tell (a runner's defaults it does not read, a pattern
    /// that leaves it out, a framework it does not cover): always selected.
    UnknownTest,
    /// Every test is selected: the changes could not be read, or a file that
    /// steers the selection changed.
    AllTestsChanged {
        /// Why.
        description: String,
    },
}

/// `Err(Error::Unsupported)` off Unix.
pub fn supported() -> Result<(), Error> {
    if cfg!(unix) {
        Ok(())
    } else {
        Err(Error::Unsupported)
    }
}

/// The per-machine directory of the graph caches ([`host_defaults`]): the
/// user's cache directory's `turbo/trace`, else the temporary directory's
/// `turbo-trace`. `TURBO_TRACE_DEBUG_MACHINE_CACHE` overrides it (tests).
pub(crate) fn machine_cache_dir() -> PathBuf {
    if let Some(dir) =
        turborepo_trace_core::debug::var_os("MACHINE_CACHE").filter(|d| !d.is_empty())
    {
        return PathBuf::from(dir);
    }
    match turborepo_dirs::cache_dir() {
        Ok(Some(dir)) => dir.as_std_path().join("turbo").join("trace"),
        _ => std::env::temp_dir().join("turbo-trace"),
    }
}

/// What every command of `turbo trace` sets first: the graph's caches in a
/// per-machine directory, not below `.turbo/trace` (some are keyed by this
/// machine's file stamps, and `.turbo` travels: CI caches it with
/// `actions/cache`).
fn host_defaults() {
    turborepo_trace_store::set_machine_cache(Some(machine_cache_dir()));
}

/// The frameworks `--tests` takes: those whose tests `turbo trace` lists by
/// file. (End-to-end specs selected by recorded visits, `playwright`, and the
/// Python language, `pytest`, are not part of `turbo trace` yet.)
pub const FRAMEWORKS: &[&str] = &["jest", "vitest", "gotest", "cargo-test", "all"];

fn check_tests(req: &AffectedRequest) -> Result<(), Error> {
    match req.tests.iter().find(|t| !FRAMEWORKS.contains(&t.as_str())) {
        Some(t) => Err(Error::InvalidRequest(format!(
            "unknown test framework {t:?} for --tests: use {}",
            FRAMEWORKS.join(", ")
        ))),
        None => Ok(()),
    }
}

/// `--base` and `--head` as turbo takes refs: one that git would read as an
/// option (`-...`) is refused.
fn check_refs(req: &AffectedRequest) -> Result<(), Error> {
    if let Changes::Scm { base, head } = &req.changes {
        for r in base.iter().chain(head) {
            crate::adapter::validate_ref(r).map_err(Error::InvalidRequest)?;
        }
    }
    Ok(())
}

fn abs(repo: &Repo<'_>, p: &str) -> PathBuf {
    turborepo_trace_core::fs::normalize(&repo.cwd.as_std_path().join(p))
}

fn command(repo: &Repo<'_>, req: &AffectedRequest) -> AffectedCmd {
    AffectedCmd {
        tests: if req.tests.is_empty() {
            vec!["all".to_owned()]
        } else {
            req.tests.clone()
        },
        strict: req.strict,
        loc: GraphLoc {
            root: Some(repo.root.as_std_path().to_path_buf()),
            graph: None,
            members: members(repo.packages),
        },
        ..AffectedCmd::default()
    }
}

/// The selection document of `cmd` (`--format selection --mode enforce`).
/// The document is handed over in process, not written and parsed again.
fn selection(mut cmd: AffectedCmd) -> Result<Document, Error> {
    cmd.format = AffectedFormat::Selection;
    cmd.mode = Some(Mode::Enforce);
    let sink = crate::affected::SelectionSink::default();
    let host = crate::affected::Host {
        sink: Some(sink.clone()),
    };
    let mut out = std::io::sink();
    crate::commands::finish(crate::affected::cmd_affected_for(
        cmd,
        &mut out,
        Some(&host),
    ))?;
    let doc = sink.lock().unwrap_or_else(|e| e.into_inner()).take();
    doc.ok_or_else(|| Error::Failed("the selection produced no document".to_owned()))
}

/// Points `cmd` at the changes from `base` (resolved by turbo's SCM layer)
/// to `head`, or to the working tree: turbo's `SCM::changed_files` lists
/// them (from the merge base to `head`, and from `head` to the working tree),
/// with the status layer of [`ScmChanges`]. An error here, or later in the
/// diff, selects every test.
fn scm_changes(
    repo: &Repo<'_>,
    cmd: &mut AffectedCmd,
    base: Option<&str>,
    head: Option<&str>,
) -> Result<(), String> {
    let source = ScmChanges::new(repo.root, repo.scm, base, head);
    source.changes()?;
    cmd.change_source = Some(Arc::new(source));
    Ok(())
}

/// The `SCM::changed_files` queries [`affected`] and [`explain`] make for
/// `changes`, for `RunBuilder::prefetch_changed_files`: started while turbo
/// builds its package graph, the git scans overlap the build, and the
/// command's own queries take their results. Empty for
/// [`Changes::Files`].
pub fn prefetch_requests(
    root: &AbsoluteSystemPath,
    changes: &Changes,
) -> Vec<turborepo_scm::git::ChangedFilesRequest> {
    match changes {
        Changes::Scm { base, head } => {
            crate::adapter::prefetch_requests(root, base.as_deref(), head.as_deref())
        }
        Changes::Files(_) => Vec::new(),
    }
}

/// Selects the tests a change affects.
pub fn affected(repo: &Repo<'_>, req: &AffectedRequest) -> Result<AffectedTests, Error> {
    supported()?;
    check_tests(req)?;
    check_refs(req)?;
    host_defaults();
    if let Some(why) = unsupported_config(repo) {
        return everything(repo, req, &why);
    }
    let mut cmd = command(repo, req);
    match &req.changes {
        Changes::Files(files) => {
            cmd.changed = files
                .iter()
                .map(|f| abs(repo, f).to_string_lossy().into_owned())
                .collect();
        }
        Changes::Scm { base, head } => {
            if let Err(e) = scm_changes(repo, &mut cmd, base.as_deref(), head.as_deref()) {
                return everything(repo, req, &format!("the changes cannot be read: {e}"));
            }
        }
    }
    let doc = selection(cmd)?;
    if doc.run == Run::All {
        if doc.packages.is_empty() {
            // The selection failed open. When it was the diff, every test is listed; any
            // other failure (a bad request, a graph that cannot be built) is an
            // error.
            if doc.reason.code == "diff-unreadable" {
                return everything(repo, req, &doc.reason.detail);
            }
            return Err(Error::Failed(doc.reason.detail));
        }
        return Ok(report(repo, req, &doc, Some(&doc.reason.detail)));
    }
    Ok(report(repo, req, &doc, None))
}

/// Every test, each with `AllTestsChanged { description }`.
fn everything(
    repo: &Repo<'_>,
    req: &AffectedRequest,
    description: &str,
) -> Result<AffectedTests, Error> {
    let mut cmd = command(repo, req);
    cmd.change_source = Some(Arc::new(GivenChanges {
        base: None,
        list: ChangeList::default(),
    }) as Arc<dyn ChangeSource>);
    let doc = selection(cmd)?;
    if doc.run == Run::All && doc.packages.is_empty() {
        return Err(Error::Failed(format!(
            "{description}; the tests cannot be listed either: {}",
            doc.reason.detail
        )));
    }
    Ok(report(repo, req, &doc, Some(description)))
}

/// The items of a selection document; with `all`, every known test, for that
/// reason.
fn report(
    repo: &Repo<'_>,
    req: &AffectedRequest,
    doc: &Document,
    all: Option<&str>,
) -> AffectedTests {
    let mut items = Vec::new();
    let mut uncovered = Vec::new();
    for pkg in &doc.packages {
        let package = package_name(repo.packages, &pkg.dir).unwrap_or_else(|| pkg.name.clone());
        if req
            .packages
            .as_ref()
            .is_some_and(|keep| !keep.contains(&package))
        {
            continue;
        }
        let mut tasks: Vec<&crate::selection::Task> = pkg.tasks.iter().collect();
        tasks.sort_by(|a, b| a.id.cmp(&b.id));
        let task_of = |path: &str, frameworks: &[String]| {
            tasks
                .iter()
                .find(|t| {
                    t.runs.iter().any(|l| {
                        (frameworks.is_empty() || frameworks.contains(&l.framework))
                            && under(&l.dir, path)
                    })
                })
                .map(|t| format!("{package}#{}", t.name))
        };
        let mut push = |path: &str, frameworks: &[String], reason: Reason| {
            items.push(AffectedTest {
                path: path.to_owned(),
                package: package.clone(),
                task: task_of(path, frameworks),
                reason,
            });
        };
        match all {
            Some(why) => {
                for s in &pkg.selected {
                    push(&s.path, &s.frameworks, everything_reason(why));
                }
                for p in pkg.unselected.iter().chain(&pkg.unknown) {
                    push(p, &[], everything_reason(why));
                }
            }
            None => {
                for s in &pkg.selected {
                    push(&s.path, &s.frameworks, reason(&s.reason, s.via.as_deref()));
                }
                // Files that look like tests and that the selection does not know: they
                // always run (the selection document's `unknown`).
                for p in &pkg.unknown {
                    push(p, &[], Reason::UnknownTest);
                }
            }
        }
        for t in &pkg.tasks {
            if let Some(why) = &t.unmapped {
                uncovered.push(UncoveredTask {
                    task: format!("{package}#{}", t.name),
                    package: package.clone(),
                    description: why.clone(),
                });
            }
        }
    }
    items.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.package.cmp(&b.package)));
    items.dedup_by(|a, b| a.path == b.path);
    uncovered.sort_by(|a, b| a.task.cmp(&b.task));
    uncovered.dedup();
    AffectedTests {
        items,
        uncovered_tasks: uncovered,
    }
}

fn everything_reason(why: &str) -> Reason {
    Reason::AllTestsChanged {
        description: why.to_owned(),
    }
}

/// `path` (root-relative) is `dir` or below it (`.`: the root).
fn under(dir: &str, path: &str) -> bool {
    dir == "." || dir.is_empty() || path.strip_prefix(dir).is_some_and(|r| r.starts_with('/'))
}

/// The selection document's class of a chain as a [`Reason`].
fn reason(class: &str, via: Option<&str>) -> Reason {
    let via = via.unwrap_or_default().to_owned();
    match class {
        "test-changed" => Reason::TestChanged,
        "change" => Reason::FileChanged { file_path: via },
        "dependency" => Reason::PackageDependencyChanged { package_name: via },
        "name" => Reason::NameReferenced { file_path: via },
        "task" => Reason::TaskChanged { file_path: via },
        "not-in-graph" => Reason::NotInGraph,
        // `uncertain`, `unknown`, and any class added later: the conservative reading.
        _ => Reason::Uncertain { file_path: via },
    }
}

/// `turbo trace affected --explain <test>...`: the readable reason for each
/// named test (paths relative to [`Repo::cwd`]), as text.
pub fn explain(
    repo: &Repo<'_>,
    req: &AffectedRequest,
    tests: &[String],
    out: &mut dyn Write,
) -> Result<(), Error> {
    supported()?;
    check_tests(req)?;
    check_refs(req)?;
    host_defaults();
    if let Some(why) = unsupported_config(repo) {
        return Err(Error::Failed(format!("{why} (every test is selected)")));
    }
    let mut cmd = command(repo, req);
    cmd.explain = tests
        .iter()
        .map(|t| abs(repo, t).to_string_lossy().into_owned())
        .collect();
    match &req.changes {
        Changes::Files(files) => {
            cmd.changed = files
                .iter()
                .map(|f| abs(repo, f).to_string_lossy().into_owned())
                .collect();
        }
        Changes::Scm { base, head } => {
            if let Err(e) = scm_changes(repo, &mut cmd, base.as_deref(), head.as_deref()) {
                return Err(Error::Failed(format!(
                    "the changes cannot be read: {e} (every test is selected)"
                )));
            }
        }
    }
    let host = crate::affected::Host::default();
    crate::commands::finish(crate::affected::cmd_affected_for(cmd, out, Some(&host)))?;
    Ok(())
}

/// `turbo trace why <test> <file>`: the chain of references from the test to
/// the file, as text (paths relative to [`Repo::cwd`]). An error when the file
/// is not in the test's trace.
pub fn why(repo: &Repo<'_>, test: &str, file: &str, out: &mut dyn Write) -> Result<(), Error> {
    supported()?;
    host_defaults();
    if let Some(why) = unsupported_config(repo) {
        return Err(Error::Failed(why));
    }
    let cwd = repo.cwd.as_std_path().to_path_buf();
    let cmd = crate::why::WhyCmd {
        entry: abs(repo, test),
        file: abs(repo, file),
        opts: crate::why::TraceOpts {
            base: Some(cwd.clone()),
            cwd: Some(cwd),
            ..Default::default()
        },
    };
    crate::commands::finish(crate::why::cmd_why(cmd, out))?;
    Ok(())
}

/// `turbo trace lint`.
#[derive(Clone, Debug, Default)]
pub struct LintRequest {
    /// `--json`: the findings as JSON.
    pub json: bool,
}

/// Runs the lints and writes the findings to `out`. Returns the exit code: 0
/// without new findings, 1 with.
pub fn lint(repo: &Repo<'_>, req: &LintRequest, out: &mut dyn Write) -> Result<i32, Error> {
    supported()?;
    host_defaults();
    if let Some(why) = unsupported_config(repo) {
        return Err(Error::Failed(why));
    }
    let cmd = crate::lint::LintCmd {
        format: if req.json {
            crate::lint::LintFormat::Json
        } else {
            crate::lint::LintFormat::Text
        },
        loc: GraphLoc {
            root: Some(repo.root.as_std_path().to_path_buf()),
            graph: None,
            members: members(repo.packages),
        },
        // Options are flags only: no baseline file is read (every finding is new).
        no_baseline: true,
        ..Default::default()
    };
    let code = crate::commands::finish(crate::lint::cmd_lint(cmd, out))?;
    Ok(i32::from(code.0))
}
