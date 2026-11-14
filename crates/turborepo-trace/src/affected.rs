//! `turbo trace affected`, and the fingerprint of a selection's inputs.

use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::{Context, Result, bail};
use turborepo_trace_graph::{
    AffectedOptions, Change, ChangeKind, ChangeSet, EdgeFilter, Engine, EngineOptions, GraphView,
    NodeId, NodeSet, Policy, RefFlags, Verdict,
    glob::{Glob, is_glob},
};
use turborepo_trace_store::Graph;

use crate::{
    ExitCode,
    graph::{GraphLoc, open_report},
    lockdiff::{self, Git},
};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
/// The verdict for entries that reach first-party uncertainty
/// (`--on-uncertain`).
pub enum OnUncertain {
    /// Affected (sound: the default).
    Affected,
    /// Unknown: reported apart (`--exit-code` 2).
    Unknown,
    /// Not affected (unsound: for measurements).
    Ignore,
}

/// The output of `affected`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AffectedFormat {
    /// The affected entries, one per line.
    List,
    /// JSON with a verdict and a witness chain per entry.
    Json,
    /// The selection document for a CI pipeline (crate::selection,
    /// `docs/ci.md`).
    Selection,
}

/// `affected`.
#[derive(Debug)]
pub struct AffectedCmd {
    /// Entry files or globs (relative to the current directory; globs match
    /// graph files).
    pub entries: Vec<String>,
    /// More entry files or globs. Repeatable.
    pub entries_opt: Vec<String>,
    /// Read entries from a file (`-`: stdin), one per line, relative to the
    /// graph root.
    pub entries_from: Vec<String>,
    /// Use the test files of a framework as entries: jest, vitest, pytest,
    /// gotest, cargo-test, or `all`. Repeatable.
    pub tests: Vec<String>,
    /// Use build tasks as entries: a glob over task ids without the kind prefix
    /// (`apps/*#build`, `*#test`), or over full ids (`task:apps/*#build`,
    /// `make:*`). Repeatable. Task ids can also be passed as entries
    /// (`task:apps/web#build`).
    pub tasks: Vec<String>,
    /// With --tasks: also the tasks a wildcard leaves out because they are run
    /// by hand by their nature: dev servers, watch modes and the fix
    /// variants of checks (a `dev`, `start`, `serve`, `preview`, `watch` or
    /// `fix` segment in the name), unless a CI job or an install script
    /// runs them. A task named exactly is always an entry.
    pub all_tasks: bool,
    /// Changed paths (relative to the current directory): modified if the path
    /// exists, deleted otherwise. Repeatable.
    pub changed: Vec<String>,
    /// Read changes from a file (`-`: stdin): newline- or NUL-separated paths
    /// relative to the graph root, or `git diff --name-status` lines
    /// (`A`/`M`/`D`/`R`...).
    pub changed_from: Vec<String>,
    /// The revision the branch is compared with, usually the default branch
    /// (`origin/main`). The base is the merge base of <REV> and HEAD: the
    /// latest commit of <REV> that the branch contains. Without other
    /// change input, the changes are the difference between that commit's
    /// tree and the working tree, plus untracked files: the branch's own
    /// changes, not what <REV> gained since, and not what a merge of
    /// <REV> into the branch brought in. The base is also the source of old
    /// lockfiles, manifests and configs. It is printed on stderr. When it
    /// can't be told (a shallow checkout that lacks the history between the
    /// two), the command fails; with --format selection the document then
    /// says that everything runs.
    pub since: Option<String>,
    /// Compare with exactly the --since revision, without the merge-base step:
    /// for a caller that has resolved the base itself, such as a CI system
    /// that hands over the base commit of a pull request (the environment
    /// variable TURBO_TRACE_DEBUG_EXACT_BASE=1 does the same).
    pub exact_base: bool,
    /// Debug: print to stderr the N files whose names select the most entries
    /// (one reachability pass per naming file: slow).
    pub names_report: Option<usize>,
    /// Verdict for entries that reach first-party uncertainty.
    pub on_uncertain: OnUncertain,
    /// Let `eval`, `new Function` and other code evaluation (`eval_like`)
    /// trigger --on-uncertain. By default it is reported but ignored.
    pub strict_eval: bool,
    /// Let every uncertainty trigger --on-uncertain, including the ones that
    /// are covered or ignored by default: JS/TS imports that don't resolve
    /// (covered by the paths their resolution probed), committed generated
    /// files, file reads whose path comes from the environment or the
    /// command line, and code evaluation (--strict-eval).
    pub strict: bool,
    /// Ignore type-only edges (pure test execution; typecheck reads them).
    pub runtime_only: bool,
    /// Output format: the affected entries one per line, JSON with a verdict
    /// and a witness chain per entry, or `selection`: the document a CI
    /// pipeline reads: per package, the test files the selection would run
    /// and what the caller must run under the mode.
    pub format: AffectedFormat,
    /// With --format selection: the switch. `shadow` (the default): the caller
    /// must run the full set, and the selection is recorded next to it.
    /// `enforce`: the caller runs the selection. Without the flag the
    /// environment variable TURBO_TRACE_DEBUG_MODE decides.
    pub mode: Option<crate::selection::Mode>,
    /// With --format selection: a changed path that matches makes the caller
    /// run the full set in every mode, because it steers the selection
    /// itself (a glob relative to the repository root; repeatable).
    pub run_all_if_changed: Vec<String>,
    /// With --format selection: files that look like tests (`*.test.*`,
    /// `*.spec.*`) and belong to a runner that was not asked for, such as
    /// end-to-end specs. They are not listed as unknown tests (a glob
    /// relative to the repository root; repeatable).
    pub not_tests: Vec<String>,
    /// With --format selection: packages that run all their tests or none,
    /// never a part, because they check something over the whole run, such
    /// as a coverage threshold (a glob over package directories relative to
    /// the repository root; repeatable).
    pub full_package: Vec<String>,
    /// With --format selection: exit with an error when the selection can't be
    /// computed. By default the document then says that the full set runs
    /// and why, and the exit code is 0, so that a pipeline never stops
    /// because of the selection step.
    pub no_fail_open: bool,
    /// Print a readable reason for these entries instead of the list (they are
    /// checked as entries too). Repeatable.
    pub explain: Vec<String>,
    /// Exit 0 if nothing is affected, 1 if an entry is affected, 2 if one is
    /// unknown.
    pub exit_code: bool,
    /// Where the graph lives.
    pub loc: GraphLoc,
    /// More repo steering files, as a glob relative to the graph root.
    /// Repeatable. Steering files (by default `.github/**`, `CODEOWNERS`,
    /// `.vercel.approvers`, `.gitattributes`, `.git-blame-ignore-revs`,
    /// `.editorconfig`, `.mergify.yml`, `renovate.json`, `.renovaterc*`)
    /// can't change what an application does: a changed one affects only the
    /// entries that reference it explicitly.
    pub steering: Vec<String>,
    /// Treat repo steering files like any other changed file.
    pub no_steering: bool,
    /// Don't derive changed CI jobs and the tests they run from the changes
    /// (README "CI workflow changes"): an A/B switch for measuring what
    /// that rule selects.
    pub no_workflows: bool,
    /// Use the stored graph as is, without refreshing it.
    pub no_refresh: bool,
    /// Print timings to stderr.
    pub stats: bool,
    /// Changes with their status from a host (`turbo trace`: Turborepo's SCM
    /// layer), instead of `changed`, `changed_from` and `since`; the
    /// source's base is the base revision.
    pub change_source: Option<std::sync::Arc<dyn turborepo_trace_graph::ChangeSource>>,
}

/// The selection document of an `affected` run, handed over in process.
pub(crate) type SelectionSink =
    std::sync::Arc<std::sync::Mutex<Option<crate::selection::Document>>>;

/// A host that calls `affected` in process (`turbo trace`): it says what the
/// base is itself, and takes the selection document from `sink` (when set)
/// instead of reading it as JSON.
#[derive(Clone, Default)]
pub(crate) struct Host {
    pub(crate) sink: Option<SelectionSink>,
}

/// Writes the selection document `doc` to `sink`, else as JSON to `out`.
fn emit_selection(
    doc: crate::selection::Document,
    sink: Option<&SelectionSink>,
    out: &mut dyn Write,
) -> Result<()> {
    match sink {
        Some(sink) => {
            *sink.lock().unwrap_or_else(|e| e.into_inner()) = Some(doc);
        }
        None => {
            serde_json::to_writer_pretty(&mut *out, &doc)?;
            writeln!(out)?;
        }
    }
    Ok(())
}

impl Default for AffectedCmd {
    fn default() -> Self {
        AffectedCmd {
            entries: Default::default(),
            entries_opt: Default::default(),
            entries_from: Default::default(),
            tests: Default::default(),
            tasks: Default::default(),
            all_tasks: Default::default(),
            changed: Default::default(),
            changed_from: Default::default(),
            since: Default::default(),
            exact_base: Default::default(),
            names_report: Default::default(),
            on_uncertain: OnUncertain::Affected,
            strict_eval: Default::default(),
            strict: Default::default(),
            runtime_only: Default::default(),
            format: AffectedFormat::List,
            mode: Default::default(),
            run_all_if_changed: Default::default(),
            not_tests: Default::default(),
            full_package: Default::default(),
            no_fail_open: Default::default(),
            explain: Default::default(),
            exit_code: Default::default(),
            loc: Default::default(),
            steering: Default::default(),
            no_steering: Default::default(),
            no_workflows: Default::default(),
            no_refresh: Default::default(),
            stats: Default::default(),
            change_source: None,
        }
    }
}

fn cwd() -> Result<PathBuf> {
    let c = std::env::current_dir().context("cannot read current directory")?;
    Ok(std::fs::canonicalize(&c).unwrap_or(c))
}

/// `p` (relative to `base`, or absolute) as a root-relative path.
///
/// `base` is the working directory. When that is not inside the repository
/// (`--root` names the repository from elsewhere: a CI step that runs in
/// another directory), a relative path is relative to the root: there is
/// nothing else it could name.
fn rel_to_root(root: &Path, base: &Path, p: &str) -> Option<String> {
    let base = if base.starts_with(root) { base } else { root };
    let abs = turborepo_trace_core::fs::normalize(&base.join(p));
    let rel = match abs.strip_prefix(root) {
        Ok(r) => r.to_path_buf(),
        // A symlinked prefix (macOS /tmp): compare canonical forms.
        Err(_) => std::fs::canonicalize(&abs)
            .ok()?
            .strip_prefix(root)
            .ok()?
            .to_path_buf(),
    };
    let s = rel.to_string_lossy().replace('\\', "/");
    Some(if s.is_empty() { ".".to_owned() } else { s })
}

fn read_input(src: &str) -> Result<String> {
    let mut s = String::new();
    if src == "-" {
        std::io::stdin().read_to_string(&mut s)?;
    } else {
        s = std::fs::read_to_string(src).with_context(|| format!("cannot read {src}"))?;
    }
    Ok(s)
}

/// Parses `--changed-from` text: NUL- or newline-separated paths or name-status
/// lines.
fn parse_changes(text: &str, root: &Path, bare: &mut Vec<String>) -> Result<Vec<Change>> {
    let mut out = Vec::new();
    let nul = text.contains('\0');
    let items: Vec<&str> = if nul {
        text.split('\0').collect()
    } else {
        text.lines().collect()
    };
    let mut i = 0;
    while i < items.len() {
        let line = items[i].trim_end_matches('\r');
        i += 1;
        if line.trim().is_empty() {
            continue;
        }
        // NUL-separated `git diff --name-status -z`: status and paths are separate
        // items.
        let line = if nul && is_status(line) && !line.contains('\t') {
            let n = if line.starts_with('R') || line.starts_with('C') {
                2
            } else {
                1
            };
            let mut l = line.to_owned();
            for _ in 0..n {
                if i < items.len() {
                    l.push('\t');
                    l.push_str(items[i]);
                    i += 1;
                }
            }
            l
        } else {
            line.to_owned()
        };
        let first = line.split(['\t', ' ']).next().unwrap_or("");
        if is_status(first) && line.len() > first.len() {
            let cs = ChangeSet::from_name_status(&line).map_err(|e| anyhow::anyhow!("{e}"))?;
            out.extend(cs.files);
        } else {
            // A path without a status: what it was before is not known (`bare`).
            let p = line.trim();
            let kind = if root.join(p).exists() {
                ChangeKind::Modified
            } else {
                ChangeKind::Deleted
            };
            bare.push(p.to_owned());
            out.push(Change::new(kind, p));
        }
    }
    Ok(out)
}

fn is_status(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 4
        && s.chars().next().is_some_and(|c| "AMDTRCU".contains(c))
        && s[1..].bytes().all(|b| b.is_ascii_digit())
}

/// Resolved entries: graph nodes plus entries that exist but are not graph
/// nodes.
pub(crate) struct Entries {
    pub(crate) set: NodeSet,
    pub(crate) missing: Vec<String>,
}

pub(crate) fn collect_entries(
    g: &Graph,
    root: &Path,
    globs_or_files: &[String],
    from: &[String],
    tests: &[String],
) -> Result<Entries> {
    let cwd = cwd()?;
    let mut set = NodeSet::new(g.node_count());
    let mut missing = BTreeSet::new();
    let add_path = |rel: String, set: &mut NodeSet, missing: &mut BTreeSet<String>| match g
        .node_by_path(&rel)
    {
        Some(n) => {
            set.insert(n);
        }
        None => {
            if root.join(&rel).is_file() {
                missing.insert(rel);
            }
        }
    };
    for e in globs_or_files {
        if is_task_id(e) {
            // A task id, or a glob over task ids.
            let set_tasks = collect_tasks(g, std::slice::from_ref(e), true)?;
            if set_tasks.is_empty() {
                bail!("{e}: no such task in the graph");
            }
            for n in set_tasks.iter() {
                set.insert(n);
            }
            continue;
        }
        // A spec that names a file is that file, whatever characters its name has: a
        // route directory `[slug]` is a character class to the matcher and
        // `{a,b}` an alternation, so as a pattern the path of
        // `app/[slug]/page.test.tsx` matches nothing.
        let named = is_glob(e)
            .then(|| rel_to_root(root, &cwd, e))
            .flatten()
            .filter(|rel| g.node_by_path(rel).is_some() || root.join(rel).is_file());
        if let Some(rel) = named {
            add_path(rel, &mut set, &mut missing);
        } else if is_glob(e) {
            let prefix = rel_to_root(root, &cwd, ".").unwrap_or_default();
            let pat = if prefix.is_empty() || prefix == "." {
                e.clone()
            } else {
                format!("{prefix}/{e}")
            };
            let glob = Glob::new(&pat).map_err(|e| anyhow::anyhow!("{e}"))?;
            for i in 0..g.node_count() {
                let n = NodeId(i);
                if !g.is_external(n)
                    && g.lang(n).is_some()
                    && !turborepo_trace_store::tasks::is_pseudo_lang(g.lang(n))
                    && glob.matches(g.path(n))
                {
                    set.insert(n);
                }
            }
        } else {
            let rel = rel_to_root(root, &cwd, e)
                .with_context(|| format!("{e} is outside the graph root {}", root.display()))?;
            add_path(rel, &mut set, &mut missing);
        }
    }
    for src in from {
        for line in read_input(src)?.lines() {
            let l = line.trim();
            if l.is_empty() {
                continue;
            }
            let rel = if Path::new(l).is_absolute() {
                match rel_to_root(root, root, l) {
                    Some(r) => r,
                    None => continue,
                }
            } else {
                turborepo_trace_graph::Change::new(ChangeKind::Modified, l).path
            };
            add_path(rel, &mut set, &mut missing);
        }
    }
    for t in tests {
        let fw = if t == "all" { None } else { Some(t.as_str()) };
        if let Some(f) = fw
            && !turborepo_trace_store::fw::GRAPH_FRAMEWORKS
                .iter()
                .any(|(id, _)| *id == f)
        {
            bail!(
                "unknown framework {f:?}: use one of {} or all",
                turborepo_trace_store::fw::GRAPH_FRAMEWORKS
                    .iter()
                    .map(|(id, _)| *id)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        for n in g.tests(fw) {
            set.insert(n);
        }
    }
    Ok(Entries {
        set,
        missing: missing.into_iter().collect(),
    })
}

/// Whether an entry argument names a task (`task:…`, `make:…`, `docker:…`,
/// `<kind>:…`) rather than a path.
fn is_task_id(e: &str) -> bool {
    turborepo_trace_tasks::spec::TaskId::parse(e).is_some() && !Path::new(e).exists()
}

/// Task nodes matching any of `globs`: globs with a `:` match full ids, others
/// match ids without the kind prefix (`apps/*#build`; `**#test` crosses
/// directories). Unless `all`, a wildcard leaves out the tasks that are run by
/// hand by their nature ([`crate::task_entries`]).
pub(crate) fn collect_tasks(g: &Graph, globs: &[String], all: bool) -> Result<NodeSet> {
    Ok(crate::task_entries::collect(g, globs, all))
}

/// One line about how a changed config file was applied (`--stats`,
/// `--explain`).
fn config_note(r: &crate::configdiff::Report) -> String {
    let fields = if r.fields.is_empty() {
        String::new()
    } else {
        format!(" ({})", r.fields.join(", "))
    };
    if r.field_level {
        let named = if r.named.is_empty() {
            String::new()
        } else {
            format!("; {}", r.named.join("; "))
        };
        format!(
            "config {}{fields}: {}; seeded {} reader(s) of the file, {} task(s), {} dependency \
             link(s), {} importer(s) (+{} for type-checking entries only){named}",
            r.path, r.reason, r.readers, r.tasks, r.links, r.importers, r.type_importers
        )
    } else {
        format!("config {}{fields}: whole-file change: {}", r.path, r.reason)
    }
}

/// Whether a read with a computed path is assumed to stay inside the package it
/// runs for (`turborepo_trace_graph::names`, "The package assumption").
/// `TURBO_TRACE_DEBUG_NAMES_PACKAGE=0` turns the assumption off for A/B
/// measurements: strings then name changed paths anywhere in the repository.
pub(crate) fn names_in_package() -> bool {
    turborepo_trace_core::debug::var_os("NAMES_PACKAGE").is_none_or(|v| v != "0")
}

/// Whether a process that an entry reaches only through a build task takes its
/// working directory and package from that task
/// (`turborepo_trace_graph::names`, "Who runs a process").
/// `TURBO_TRACE_DEBUG_NAMES_TASK_RUNNER=0` turns it off for A/B measurements:
/// such a process is then named as if the entry ran it.
pub(crate) fn names_by_runner() -> bool {
    turborepo_trace_core::debug::var_os("NAMES_TASK_RUNNER").is_none_or(|v| v != "0")
}

/// Whether an unresolved import in a Python file is covered by what its
/// resolution recorded (`AffectedOptions::python_declared`): the places a
/// module of that name could appear in the repository, and the dependency
/// declarations and lockfiles next to the file or above it, from which its
/// environment is assumed to be installed. `TURBO_TRACE_DEBUG_PY_DECLARED=0`
/// turns it off for A/B measurements: such an import is open again.
pub(crate) fn python_declared() -> bool {
    turborepo_trace_core::debug::var_os("PY_DECLARED").is_none_or(|v| v != "0")
}

/// Whether a read below a temporary directory that the code writes out
/// (`/tmp/…`) is taken to name no file of the repository
/// (`AffectedOptions::temp_reads`). `TURBO_TRACE_DEBUG_TEMP_READS=0`
/// turns the assumption off for A/B measurements: such a read is an open
/// record, as under `--strict`.
pub(crate) fn temp_reads() -> bool {
    turborepo_trace_core::debug::var_os("TEMP_READS").is_none_or(|v| v != "0")
}

/// Whether a read with a computed directory directly below a place the
/// repository may be mounted at (`` `/app/${dir}/x.json` ``) is taken to name
/// no directory of the repository (`AffectedOptions::mount_reads`).
/// `TURBO_TRACE_DEBUG_MOUNT_READS=0` turns the assumption off for
/// A/B measurements: such a read is an open record, as under `--strict`.
pub(crate) fn mount_reads() -> bool {
    turborepo_trace_core::debug::var_os("MOUNT_READS").is_none_or(|v| v != "0")
}

/// Whether reads with a computed path are bounded by names (crate::names)
/// instead of being open.
pub(crate) fn names_enabled() -> bool {
    crate::names::enabled()
}

pub(crate) fn engine_options(runtime_only: bool, policy: Policy) -> EngineOptions {
    EngineOptions {
        traversal: if runtime_only {
            EdgeFilter::ALL.without_flags(RefFlags::TYPE_ONLY)
        } else {
            EdgeFilter::ALL
        },
        // The graph is refreshed from the working tree (or, with --no-refresh, may be from
        // either side of the change), so deleted paths must also match probes.
        affected: AffectedOptions::default().policy(policy).after_change(true),
    }
}

/// The options `affected`, `uncertainty` and `lint` evaluate with:
/// [`engine_options`] with every rule that an environment variable can switch
/// off taken from it (the names of computed reads, their package assumption and
/// who runs a process; the declared packages of module loads and the resolution
/// step; what covers an unresolved Python import). One function, so that the
/// three commands cannot differ in what they take a record to be. A
/// command adds its own strict flags.
pub(crate) fn selection_options(root: &Path, runtime_only: bool, policy: Policy) -> EngineOptions {
    let mut opts = engine_options(runtime_only, policy);
    opts.affected = opts
        .affected
        .named(names_enabled())
        .named_in_package(names_in_package())
        .named_by_runner(names_by_runner())
        .loads_declared(crate::loads::enabled(root))
        .loads_step(crate::loads::step())
        .python_declared(python_declared())
        .temp_reads(temp_reads())
        .mount_reads(mount_reads());
    opts
}

/// A failure to read the base revision or the diff (a shallow clone without the
/// base commit, no repository): the selection document names it apart from
/// other errors.
#[derive(Debug)]
struct DiffUnreadable;

impl std::fmt::Display for DiffUnreadable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the base revision or the diff could not be read")
    }
}

/// The lines of `--explain` for the readings from the repository root that the
/// per-test working-directory layer did not take for `entry`
/// (`turborepo_trace_store::Graph::cwd_assumed`): the assumption, where it
/// decided.
fn write_cwd_assumed(
    g: &turborepo_trace_store::Graph,
    entry: turborepo_trace_graph::NodeId,
    out: &mut dyn Write,
) -> Result<()> {
    let notes = g.cwd_assumed(entry);
    for s in notes.iter().take(5) {
        writeln!(
            out,
            "  assumed: {s} (a repository is not checked out below directories whose names repeat \
             a path inside it; --strict does not assume it)"
        )?;
    }
    if notes.len() > 5 {
        writeln!(
            out,
            "  assumed: the same for {} more path(s)",
            notes.len() - 5
        )?;
    }
    Ok(())
}

pub fn cmd_affected(cmd: AffectedCmd, out: &mut dyn Write) -> Result<ExitCode> {
    cmd_affected_for(cmd, out, None)
}

/// [`cmd_affected`] for a `host` in process.
pub(crate) fn cmd_affected_for(
    cmd: AffectedCmd,
    out: &mut dyn Write,
    host: Option<&Host>,
) -> Result<ExitCode> {
    if cmd.format != AffectedFormat::Selection {
        if cmd.mode.is_some()
            || !cmd.run_all_if_changed.is_empty()
            || !cmd.not_tests.is_empty()
            || !cmd.full_package.is_empty()
            || cmd.no_fail_open
        {
            bail!(
                "--mode, --run-all-if-changed, --not-tests, --full-package and --no-fail-open \
                 need --format selection"
            );
        }
        return affected(cmd, out, host);
    }
    // The selection document: whatever fails, the caller gets a document that says
    // the full set runs and why.
    let (mode, mode_source, note) = crate::selection::resolve_mode(cmd.mode);
    if let Some(n) = note {
        eprintln!("turbo trace: {n}");
    }
    let base = cmd.since.clone().filter(|s| !s.is_empty());
    let frameworks = cmd.tests.clone();
    let fail_open = !cmd.no_fail_open;
    let sink = host.and_then(|h| h.sink.clone());
    let mut buf: Vec<u8> = Vec::new();
    match affected(cmd, &mut buf, host) {
        Ok(code) => {
            out.write_all(&buf)?;
            Ok(code)
        }
        Err(e) => {
            let code = if e.downcast_ref::<DiffUnreadable>().is_some() {
                "diff-unreadable"
            } else {
                "error"
            };
            let doc = crate::selection::Document::failed(
                mode,
                mode_source,
                code,
                format!("{e:#}"),
                base,
                frameworks,
            );
            emit_selection(doc, sink.as_ref(), out)?;
            if fail_open {
                eprintln!("turbo trace: the selection failed, the full set runs: {e:#}");
                Ok(ExitCode::SUCCESS)
            } else {
                out.flush()?;
                Err(e)
            }
        }
    }
}

fn affected(cmd: AffectedCmd, out: &mut dyn Write, host: Option<&Host>) -> Result<ExitCode> {
    let t0 = Instant::now();
    turborepo_trace_core::timing::mark("affected: start");
    let (root, _) = cmd.loc.resolve()?;
    let root = std::fs::canonicalize(&root).unwrap_or(root);
    if cmd.entries.is_empty()
        && cmd.entries_opt.is_empty()
        && cmd.entries_from.is_empty()
        && cmd.tests.is_empty()
        && cmd.tasks.is_empty()
        && cmd.explain.is_empty()
    {
        bail!(
            "no entries: pass files or globs, --entries-from <file>, --tests <framework> or \
             --tasks <glob>"
        );
    }
    // A host's change source: its base is the merge base its changes are against
    // (or the ref, for a history with several), resolved here as for `--since`.
    let source_base = cmd.change_source.as_ref().and_then(|s| s.base());
    let since = if cmd.change_source.is_some() {
        source_base.as_deref()
    } else {
        cmd.since.as_deref().filter(|s| !s.is_empty())
    };
    // The base: the merge base of the revision and HEAD (lockdiff::Git). Everything
    // below that reads the base reads it through `git`.
    let git = match since {
        Some(rev) => Some(
            Git::new(&root, rev, lockdiff::exact_base(cmd.exact_base))
                .map_err(|e| e.context(DiffUnreadable))?,
        ),
        None => None,
    };
    // (A host in process, `turbo trace`, says nothing about the base here.)
    if let Some(g) = git
        .as_ref()
        .filter(|g| !g.exact && cmd.change_source.is_none() && host.is_none())
    {
        eprintln!("turbo trace: {}", g.describe());
    }
    // The revision as given, and the commit it resolved to.
    let since_given = since;
    let since = git.as_ref().map(|g| g.rev.as_str());

    // 1. Changes.
    let mut changes: Vec<Change> = Vec::new();
    // The paths that were given without a status (`--changed`, a plain line of
    // `--changed-from`): the tool knows whether each is there now, and nothing
    // about before (crate::dirchange::Origin).
    let mut bare: Vec<String> = Vec::new();
    let cwd = cwd()?;
    for c in &cmd.changed {
        let rel = rel_to_root(&root, &cwd, c)
            .with_context(|| format!("{c} is outside the graph root {}", root.display()))?;
        let kind = if root.join(&rel).exists() {
            ChangeKind::Modified
        } else {
            ChangeKind::Deleted
        };
        bare.push(rel.clone());
        changes.push(Change::new(kind, rel));
    }
    let mut stated: std::collections::HashSet<String> = std::collections::HashSet::new();
    for src in &cmd.changed_from {
        let mut plain = Vec::new();
        let given = parse_changes(&read_input(src)?, &root, &mut plain)?;
        stated.extend(
            given
                .iter()
                .filter(|c| !plain.contains(&c.path))
                .map(|c| c.path.clone()),
        );
        bare.extend(plain);
        changes.extend(given);
    }
    // (A path that some line gives with a status has that status.)
    let bare: std::collections::HashSet<String> =
        bare.into_iter().filter(|p| !stated.contains(p)).collect();
    // The source's changes have their status, like `--changed-from` name-status
    // lines.
    if let Some(src) = &cmd.change_source {
        let list = src
            .changes()
            .map_err(|e| anyhow::anyhow!("{e}").context(DiffUnreadable))?;
        if let Some(g) = &git {
            g.set_links_at_base(list.links_at_base);
        }
        changes.extend(list.changes);
    }
    let explicit =
        !cmd.changed.is_empty() || !cmd.changed_from.is_empty() || cmd.change_source.is_some();
    if !explicit && git.is_none() {
        bail!("no changes: pass --changed, --changed-from or --since");
    }

    // 2. Graph (refreshed while git computes the tracked diff), then untracked
    //    files.
    // The git thread also parses changed lockfiles (old and new sides) ahead of
    // step 3.
    let root_ref = &root;
    let stats = cmd.stats;
    // A refresh reports the walked files: the untracked scan then needs git's
    // tracked list.
    let prefetch_ls = !cmd.no_refresh;
    let (mut g, report, tracked) = std::thread::scope(|s| {
        let diff = (!explicit)
            .then(|| {
                git.as_ref().map(|g| {
                    s.spawn(move || {
                        let t0 = Instant::now();
                        // The tracked list for the untracked scan, alongside the diff.
                        let ls = std::thread::scope(|s2| {
                            let ls = prefetch_ls
                                .then(|| s2.spawn(|| lockdiff::Git::tracked_files(root_ref)));
                            let t = g.tracked_changes();
                            (
                                t,
                                ls.map(|h| {
                                    h.join().unwrap_or_else(|p| std::panic::resume_unwind(p))
                                }),
                            )
                        });
                        let (t, ls) = ls;
                        let t_diff = t0.elapsed();
                        let pre = match &t {
                            Ok(t) => lockdiff::preparse(root_ref, t, Some(g)),
                            Err(_) => Default::default(),
                        };
                        if stats {
                            eprintln!(
                                "git diff {} ms, lockfile parse (old and new) {} ms",
                                t_diff.as_millis(),
                                (t0.elapsed() - t_diff).as_millis()
                            );
                        }
                        (t, pre, ls)
                    })
                })
            })
            .flatten();
        let opened = open_report(&cmd.loc, cmd.no_refresh, cmd.stats);
        turborepo_trace_core::timing::mark("affected: refresh + load");
        let tracked = diff.map(|h| h.join().unwrap_or_else(|p| std::panic::resume_unwind(p)));
        opened.map(|(g, r)| (g, r, tracked))
    })?;
    // A path given without a status can't say "added": an existing path the graph
    // has no node for may be new. Only as an addition does it match the
    // listings that have no edge for it (a listing truncated at `--max-glob`, a
    // listing resolved per test). (Whether its directory is new is another
    // question, and one that a node does not answer: the refreshed graph has a
    // node for a source file that is new. See crate::dirchange.)
    for c in changes.iter_mut() {
        if c.kind == ChangeKind::Modified
            && bare.contains(&c.path)
            && g.node_by_path(&c.path).is_none()
        {
            c.kind = ChangeKind::Added;
        }
    }
    let mut pre = lockdiff::PreParsed::default();
    if let (Some(gitx), Some((tracked, parsed, ls))) = (&git, tracked) {
        changes = tracked.map_err(|e| e.context(DiffUnreadable))?;
        pre = parsed;
        let walked = report
            .as_ref()
            .map(|r| (r.walked.as_slice(), r.walked_links.as_slice()));
        // The graph directory is not part of the change set.
        let (_, out_dir) = cmd.loc.resolve()?;
        let out_rel = std::fs::canonicalize(&out_dir)
            .unwrap_or(out_dir)
            .strip_prefix(&root)
            .ok()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .filter(|p| !p.is_empty());
        changes.extend(gitx.untracked_with(&root, walked, out_rel.as_deref(), ls)?);
        if let Some(p) = &out_rel {
            let o = format!("{p}/");
            changes.retain(|c| !c.path.starts_with(&o));
        }
    }
    // The graph directory is not part of a source's changes either (an untracked
    // `.turbotrace` is an added path to git).
    if cmd.change_source.is_some() {
        let (_, out_dir) = cmd.loc.resolve()?;
        if let Some(p) = std::fs::canonicalize(&out_dir)
            .unwrap_or(out_dir)
            .strip_prefix(&root)
            .ok()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .filter(|p| !p.is_empty())
        {
            let o = format!("{p}/");
            changes.retain(|c| !c.path.starts_with(&o));
        }
    }
    changes.extend(turbo_section_changes(&root, &changes, git.as_ref()));
    changes.sort();
    changes.dedup();
    // Repo steering files leave the change set (crate::steering).
    let steer = crate::steering::Steering::new(&cmd.steering, !cmd.no_steering)?;
    let steering_changes = steer.of(&changes);
    let t_changes = t0.elapsed();
    turborepo_trace_core::timing::mark("affected: changes");

    let policy = match cmd.on_uncertain {
        OnUncertain::Affected => Policy::Affected,
        OnUncertain::Unknown => Policy::Unknown,
        OnUncertain::Ignore => Policy::Ignore,
    };
    let mut opts = selection_options(&root, cmd.runtime_only, policy);
    opts.affected = opts
        .affected
        .strict_eval(cmd.strict_eval)
        .strict(cmd.strict);
    let aopts = opts.affected.clone();
    // Under --strict a working-directory path keeps its reading from the repository
    // root (`turborepo_trace_store::Graph::set_cwd_root_always`).
    g.set_cwd_root_always(cmd.strict);
    // Working-directory-relative references resolved per test (same node ids as
    // `g`).
    let dg = crate::graph::derived(&g, cmd.stats)?;
    turborepo_trace_core::timing::mark("affected: derived layer");
    let engine = Engine::with_options(&dg, opts);

    // 3. Lockfiles and manifests, while the graph's tables and the engine's indexes
    //    load.
    let (fx, t_fx, t_warm, t_prepare) = std::thread::scope(|s| {
        let warm = s.spawn(|| {
            let t = Instant::now();
            g.warm();
            let t_warm = t.elapsed();
            engine.prepare();
            (t_warm, t.elapsed() - t_warm)
        });
        let t = Instant::now();
        let fx = lockdiff::effects(&g, &changes, git.as_ref(), pre);
        let t_fx = t.elapsed();
        let (t_warm, t_prepare) = warm.join().unwrap_or_else(|p| std::panic::resume_unwind(p));
        (fx, t_fx, t_warm, t_prepare)
    });
    turborepo_trace_core::timing::mark("affected: lockfiles | tables + engine indexes");
    // A vitest config whose alias entries changed selects the tests that can load a
    // file with an affected request (crate::aliasdiff). For the workflow layer
    // below, such a config is not a changed definition of the script that runs
    // the tests.
    let (alias_drop, alias_seeds, alias_notes) = {
        let a = crate::aliasdiff::effects(&engine, &root, &changes, git.as_ref(), &|p| {
            fx.drop.contains(p) || steer.is_steering(p)
        });
        (a.drop, a.seeds, a.notes)
    };
    // Workflow and action files: which jobs changed and which tests they run
    // (crate::workflows), instead of a file change that reaches every task.
    let kept: Vec<Change> = changes
        .iter()
        .filter(|c| !fx.drop.contains(&c.path) && !alias_drop.contains(&c.path))
        .cloned()
        .collect();
    let wf = if cmd.no_workflows {
        crate::workflows::Effects::default()
    } else {
        crate::workflows::effects(&g, &engine, &root, &kept, git.as_ref())
    };
    turborepo_trace_core::timing::mark("affected: workflows");
    let mut fx = fx;
    fx.notes.extend(alias_notes);
    fx.notes.extend(wf.notes.iter().cloned());
    fx.drop.extend(wf.drop.iter().cloned());
    // Config files whose changed fields decide who is affected (crate::configdiff).
    let lockfile_diffed = fx.drop.iter().any(|p| {
        matches!(
            p.rsplit('/').next(),
            Some(
                "pnpm-lock.yaml"
                    | "package-lock.json"
                    | "npm-shrinkwrap.json"
                    | "yarn.lock"
                    | "bun.lock"
            )
        )
    });
    let mut cfx = crate::configdiff::effects(
        &engine,
        &changes,
        git.as_ref(),
        &fx.importer_deps,
        lockfile_diffed,
        cmd.strict,
        &|p| fx.drop.contains(p) || steer.is_steering(p),
    );
    // A config module that links a package the lockfile diff changed counts as
    // changed for the nodes that read its values too
    // (crate::configdiff::config_imports).
    {
        let scoped: Vec<_> = fx
            .packages
            .iter()
            .map(|p| (p.package, p.importers.as_slice(), p.source.as_str()))
            .collect();
        crate::configdiff::package_links(&engine, &scoped, cmd.strict, &mut cfx);
    }
    fx.notes.extend(cfx.notes.iter().cloned());
    // A `next.config` whose route rules changed is no input change for the jest
    // tests of a `next/jest` project (crate::nextconfigdiff).
    let (next_drop, next_seeds) = {
        let n = crate::nextconfigdiff::effects(
            &engine,
            &root,
            &changes,
            git.as_ref(),
            cmd.strict,
            &|p| fx.drop.contains(p) || cfx.drop.contains(p) || steer.is_steering(p),
        );
        fx.notes.extend(n.notes);
        (n.drop, n.seeds)
    };
    // A tsconfig whose project references changed is no input change for tests that
    // only transpile (crate::tsconfigdiff).
    let tsfx = crate::tsconfigdiff::effects(&engine, &root, &changes, git.as_ref(), &|p| {
        fx.drop.contains(p)
            || cfx.drop.contains(p)
            || next_drop.contains(p)
            || alias_drop.contains(p)
            || steer.is_steering(p)
    });
    // A referenced tsconfig project that governed a file at the base and does not
    // now is no input of the file in this graph (crate::tsgovern).
    let tsfx = {
        let mut tsfx = tsfx;
        let packages: Vec<crate::tsgovern::ChangedPackage<'_>> = fx
            .packages
            .iter()
            .map(|p| crate::tsgovern::ChangedPackage {
                name: g.package_info(p.package).name,
                relinked: false,
                cause: turborepo_trace_graph::Cause::Package { package: p.package },
            })
            .chain(
                fx.importer_deps
                    .iter()
                    .map(|d| crate::tsgovern::ChangedPackage {
                        name: &d.name,
                        relinked: true,
                        cause: turborepo_trace_graph::Cause::Modified {
                            path: d.source.clone(),
                        },
                    }),
            )
            .collect();
        let gov = crate::tsgovern::effects(&engine, &root, &changes, git.as_ref(), &packages);
        tsfx.seeds.extend(gov.seeds);
        tsfx.notes.extend(gov.notes);
        tsfx
    };
    fx.notes.extend(tsfx.notes.iter().cloned());
    turborepo_trace_core::timing::mark("affected: config diffs");
    let mut cs = ChangeSet::new();
    for c in &changes {
        if !fx.drop.contains(&c.path)
            && !cfx.drop.contains(&c.path)
            && !next_drop.contains(&c.path)
            && !alias_drop.contains(&c.path)
            && !tsfx.drop.contains(&c.path)
            && !steer.is_steering(&c.path)
        {
            cs.files.push(c.clone());
        }
    }
    cs.seeds.extend(cfx.seeds.iter().cloned());
    // Readers of a changed manifest that got the same of it at both revisions.
    cs.spared.extend(cfx.spared.iter().cloned());
    cs.seeds.extend(next_seeds);
    cs.seeds.extend(alias_seeds);
    cs.seeds.extend(tsfx.seeds.iter().cloned());
    cs.type_seeds.extend(cfx.type_seeds.iter().cloned());
    cs.type_seeds.extend(tsfx.type_seeds.iter().cloned());
    for p in &fx.packages {
        cs.push_scoped(p.package, p.importers.clone());
    }
    // Directories the change made or removed: no file of the diff names them, and a
    // lookup that asked for one by name has another answer (crate::dirchange). Of
    // every change: a file that a rule above took out of the change set is
    // there or gone all the same.
    let origin = crate::dirchange::Origin {
        bare: &bare,
        before: &[],
    };
    cs.dirs = crate::dirchange::dirs(&root, &changes, git.as_ref(), &origin, &|d| {
        let mut any = false;
        engine.match_dir(d, &mut |_, _, _| any = true);
        any
    });
    turborepo_trace_core::timing::mark("affected: directories of the change");
    // Only steering files changed: the uncertainty floor is theirs alone
    // (crate::steering).
    let steering_only = !steering_changes.is_empty() && cs == ChangeSet::new();
    let eval_opts = if steering_only {
        aopts.clone().policy(Policy::Ignore)
    } else {
        aopts.clone()
    };

    // 4. Entries and evaluation.
    let t_lock = t0.elapsed() - t_changes;
    let mut specs = cmd.entries.clone();
    specs.extend(cmd.entries_opt.iter().cloned());
    // Entries to explain are entries too.
    specs.extend(cmd.explain.iter().cloned());
    let t_entries0 = Instant::now();
    let mut entries = collect_entries(&g, &root, &specs, &cmd.entries_from, &cmd.tests)?;
    let task_entries = collect_tasks(&g, &cmd.tasks, cmd.all_tasks)?;
    for n in task_entries.iter() {
        entries.set.insert(n);
    }
    let t_entries = t_entries0.elapsed();
    turborepo_trace_core::timing::mark("affected: entries");
    let t_eval0 = Instant::now();
    // With a base revision, modified files that only reach entries through barrels
    // whose names they don't provide are skipped (crate::barrel).
    let outcome = crate::barrel::evaluate(
        &engine,
        &root,
        &cs,
        &entries.set,
        &eval_opts,
        git.as_ref(),
        None,
        // What was traced with each file: the mock calls.
        Some(crate::barrel::GraphFacts {
            mock_files: &|| {
                g.map_facts(turborepo_trace_lang_js::mocks::MOCK_FACT_TAG, &|n, _| {
                    Some(n)
                })
            },
            mocks: &|needles| {
                g.map_facts(turborepo_trace_lang_js::mocks::MOCK_FACT_TAG, &|n, f| {
                    // A call with a computed module name has `null` after its method;
                    // any other call matters only when its text has one of the names.
                    if !f.contains("\",null,") && !needles.iter().any(|n| f.contains(n.as_str())) {
                        return None;
                    }
                    let scan = turborepo_trace_lang_js::mocks::MockScan::decode(f)?;
                    // Also the uses that are not a call the scan can read: a mock call
                    // on another receiver names a module just the same.
                    let requests = scan
                        .calls
                        .iter()
                        .map(|c| c.request.clone())
                        .chain(scan.loose.iter().map(|l| l.request.clone()))
                        .collect();
                    Some((n, requests))
                })
            },
        }),
    );
    // Erased-equal files and per-entry edge modes (crate::erase,
    // query::barrel::modes).
    let (pruning_stats, pruning_json) = (outcome.stats_line(), outcome.pruning_json(&g));
    // What `--explain` says about changed exports (crate::barrel).
    let export_notes = outcome.export_notes();
    let crate::barrel::Outcome {
        mut report,
        pruned,
        candidates: barrel_candidates,
        rejected: barrel_rejected,
        scanned: barrel_scanned,
        elapsed: barrel_elapsed,
        ..
    } = outcome;
    turborepo_trace_core::timing::mark("affected: evaluate (barrels, erase, modes, passes)");
    // What the steering changes reach through explicit references.
    let steering_reach = crate::steering::reach(&engine, &steer, &steering_changes);
    crate::steering::merge(&g, &steering_reach, &entries.set, &mut report);
    if steering_only {
        // The loads of steering files are open here, as before: nothing else changed,
        // so no package is affected that could bound them (crate::loads).
        crate::steering::floor(
            &engine,
            &steer,
            &aopts.clone().loads_declared(false),
            &entries.set,
            &mut report,
        );
        report.policy = policy;
        // An entry outside the graph can only be affected if it is a steering file
        // itself.
        entries.missing.retain(|m| steer.is_steering(m));
    }
    // Changed workflow jobs and the tests they run (crate::workflows).
    wf.apply(&g, &entries.set, &mut report);
    // Tests of tasks that start a changed service before running them
    // (crate::services).
    let svc = crate::services::effects(&g, &engine, &cs, &eval_opts);
    svc.apply(&g, &entries.set, &mut report);
    // Tests whose Next.js config loads something the change affects
    // (crate::nextconfigdiff). The config and what it loads are evaluated as
    // entries of their own here; the names and the loads layer below run for
    // them too, and the tests are selected after those.
    let mut next_loads = crate::nextconfigdiff::prepare(
        &engine,
        &root,
        &cs,
        &eval_opts,
        git.as_ref(),
        &entries.set,
        cmd.strict,
    );
    fx.notes.extend(svc.notes.iter().cloned());
    // Changed paths that code loaded with a computed read names (crate::names).
    let names_stats = if aopts.named && !engine.named_sites(&aopts).is_empty() {
        // Repo steering files count only through explicit references (crate::steering):
        // like the uncertainty floor, a name is not one.
        let named_changes: Vec<Change> = changes
            .iter()
            .filter(|c| !steer.is_steering(&c.path))
            .cloned()
            .collect();
        let cache_dir = turborepo_trace_store::cache_dir(&cmd.loc.resolve()?.1);
        // No analysis closes tables yet (the cross-file binding will pass its tables
        // here).
        let layer = crate::names::build(
            &g,
            &engine,
            &root,
            &cache_dir,
            &named_changes,
            &crate::xfile::closed_spans(),
        );
        let t = Instant::now();
        // The entries the engine evaluated without type-only edges.
        let runtime = pruned
            .runtime_entries
            .clone()
            .unwrap_or_else(|| turborepo_trace_graph::NodeSet::new(g.node_count()));
        let t_runtime = t.elapsed();
        if let Some(top) = cmd.names_report {
            // Which names select how many entries, before the names are merged in.
            let already: std::collections::HashSet<u32> = report
                .entries
                .iter()
                .filter(|e| e.cause.is_some())
                .map(|e| e.entry.0)
                .collect();
            let rows = engine.named_report(
                &layer.index,
                &named_changes,
                &entries.set,
                Some(&runtime),
                &aopts,
            );
            let union: std::collections::HashSet<u32> = rows
                .iter()
                .flat_map(|r| r.entries.iter().copied())
                .collect();
            eprintln!(
                "names report: {} file(s) name a changed path and select {} entr(ies) between \
                 them, {} of those not selected by a change; entries selected by each \
                 (inclusive), of those not selected by a change (new), and entries no other name \
                 selects (exclusive):",
                rows.len(),
                union.len(),
                union.iter().filter(|e| !already.contains(e)).count()
            );
            eprintln!("  inclusive      new  exclusive  file:line  names (a changed path)");
            for r in rows.iter().take(top) {
                let new = r.entries.iter().filter(|e| !already.contains(e)).count();
                eprintln!(
                    "  {:9} {:8} {:10}  {}:{}  {} ({})",
                    r.inclusive,
                    new,
                    r.exclusive,
                    g.path(r.node),
                    r.line,
                    r.by.join(", "),
                    r.path
                );
            }
        }
        if let Some(top) = cmd.names_report {
            // Which reads the names are paired with.
            let rows = engine.named_site_report(
                &layer.index,
                &named_changes,
                &entries.set,
                Some(&runtime),
                &aopts,
            );
            // How many of the selected entries have k such files.
            let mut per_entry: std::collections::HashMap<u32, u32> =
                std::collections::HashMap::new();
            for r in &rows {
                for &e in &r.entries {
                    *per_entry.entry(e).or_default() += 1;
                }
            }
            let mut counts: Vec<u32> = per_entry.values().copied().collect();
            counts.sort_unstable();
            let pct = |p: usize| counts.get(counts.len() * p / 100).copied().unwrap_or(0);
            eprintln!(
                "names sites: {} file(s) with a named read are paired with a name; files with a \
                 named read per selected entry: min {} median {} p90 {} max {}",
                rows.len(),
                counts.first().copied().unwrap_or(0),
                pct(50),
                pct(90),
                counts.last().copied().unwrap_or(0)
            );
            eprintln!("  inclusive  exclusive  file with the named read(s)");
            for r in rows.iter().take(top) {
                eprintln!("  {:9} {:10}  {}", r.inclusive, r.exclusive, g.path(r.node));
            }
        }
        // The same for what Next.js configs load (crate::nextconfigdiff), when a read
        // that names bound runs while one of them loads: in the same pass.
        let mut also = None;
        if let Some(p) = next_loads.as_mut()
            && p.holds(&engine, &engine.named_sites(&aopts))
        {
            also = Some((&p.set, &mut p.named));
        }
        let added = crate::names::apply(
            &g,
            &engine,
            &layer,
            &named_changes,
            &entries.set,
            &runtime,
            &aopts,
            &mut report,
            also,
        );
        format!(
            "names: {} name(s) of {} source file(s) and {} data file(s) ({} rescanned) checked \
             against the changed paths in {:.1} ms, {} can name one; {} entr(ies) selected \
             through a name only, {:.1} ms (of which runner modes {:.1} ms)",
            layer.names,
            layer.files,
            layer.data_files,
            layer.data_scanned,
            layer.elapsed.as_secs_f64() * 1e3,
            layer.index.len(),
            added,
            t.elapsed().as_secs_f64() * 1e3,
            t_runtime.as_secs_f64() * 1e3
        )
    } else {
        String::new()
    };
    // Loads with an unknown target select an entry only when the change can reach
    // them (crate::loads). Like the floor and the names, repo steering files do
    // not count.
    let loads_applied = {
        let load_paths: Vec<Change> = changes
            .iter()
            .filter(|c| !steer.is_steering(&c.path))
            .cloned()
            .collect();
        let runtime = pruned
            .runtime_entries
            .clone()
            .unwrap_or_else(|| NodeSet::new(g.node_count()));
        // A manifest or a layout file before the change (crate::loads::lent).
        let base = |path: &str| git.as_ref().and_then(|g| g.show(path));
        // The same for what Next.js configs load (crate::nextconfigdiff), when a load
        // with an unknown target runs while one of them loads.
        if let Some(p) = next_loads.as_mut()
            && p.holds(&engine, &engine.declared_sites(&eval_opts))
        {
            crate::loads::apply(
                &engine,
                &root,
                &cs,
                &load_paths,
                &base,
                &p.set,
                &p.set,
                &eval_opts,
                &mut p.bounded,
            );
        }
        crate::loads::apply(
            &engine,
            &root,
            &cs,
            &load_paths,
            &base,
            &entries.set,
            &runtime,
            &eval_opts,
            &mut report,
        )
    };
    // Tests whose Next.js config loads something the change affects
    // (crate::nextconfigdiff).
    if let Some(p) = next_loads {
        let loaded = p.finish(&engine, &cs, &aopts, &eval_opts);
        loaded.apply(&g, &entries.set, &mut report);
        fx.notes.extend(loaded.notes);
    }
    let t_eval = t_eval0.elapsed();
    turborepo_trace_core::timing::mark("affected: steering + workflow runs");
    // Entries that exist but are not in the graph cannot be cleared.
    let missing_verdict = match policy {
        Policy::Affected => Some(Verdict::Affected),
        Policy::Unknown => Some(Verdict::Unknown),
        Policy::Ignore => None,
    };

    if cmd.stats {
        eprintln!(
            "affected: {} change(s) ({} dropped as diffed lockfiles), {} changed package link(s); \
             {} entr(ies); {} seed(s), {} reached; changes+graph {} ms, lockfiles+indexes {} ms \
             (lockfiles {} ms | tables {} ms, engine indexes {} ms), entries {} ms, eval {:.1} \
             ms, total {} ms",
            changes.len(),
            fx.drop.len() - wf.drop.len(),
            fx.packages.len(),
            report.entry_count + entries.missing.len(),
            report.seed_count,
            report.reached_count,
            t_changes.as_millis(),
            t_lock.as_millis(),
            t_fx.as_millis(),
            t_warm.as_millis(),
            t_prepare.as_millis(),
            t_entries.as_millis(),
            t_eval.as_secs_f64() * 1e3,
            t0.elapsed().as_millis()
        );
        for n in &fx.notes {
            eprintln!("  {n}");
        }
        for r in &cfx.reports {
            eprintln!("  {}", config_note(r));
        }
        if cfx.env_readers > 0 {
            eprintln!(
                "  version: {} source file(s) name npm_package_version, seeded",
                cfx.env_readers
            );
        }
        if !fx.importer_deps.is_empty() {
            eprintln!(
                "  lockfile: {} direct dependenc(ies) re-resolved for their importer, {} linking \
                 node(s) seeded",
                fx.importer_deps.len(),
                cfx.lock_links
            );
        }
        eprintln!(
            "steering: {} of {} change(s) are repo steering files (they count only through \
             explicit references; {} node(s) reached{})",
            steering_changes.len(),
            changes.len(),
            steering_reach.len(),
            if steering_only {
                "; no other change, so only uncertainty in steering files counts"
            } else {
                ""
            }
        );
        let u = engine.uncertainty_counts(&aopts);
        eprintln!(
            "uncertainty: {} open, {} covered (by a scope or, for {} unresolved specifier(s), by \
             resolution probes; not counted), {} ignored by default (--strict counts them)",
            u.open, u.covered, u.probed, u.ignored
        );
        eprintln!(
            "barrels: {} candidate(s), {} modified JS file(s) not skippable, {} deferred, {} \
             entr(ies) skipped, {} shape(s) scanned, {:.1} ms",
            barrel_candidates.len(),
            barrel_rejected.len(),
            pruned.deferred,
            pruned.unaffected.len(),
            barrel_scanned,
            barrel_elapsed.as_secs_f64() * 1e3
        );
        if !pruning_stats.is_empty() {
            eprintln!("{pruning_stats}");
        }
        if !names_stats.is_empty() {
            eprintln!("{names_stats}");
        }
        if let Some(l) = &loads_applied {
            eprintln!("{}", l.stats_line());
        }
    }

    let affected: Vec<&str> = report
        .with_verdict(Verdict::Affected)
        .map(|e| g.path(e.entry))
        .collect();
    let unknown: Vec<&str> = report
        .with_verdict(Verdict::Unknown)
        .map(|e| g.path(e.entry))
        .collect();
    let (mut n_aff, mut n_unk) = (affected.len(), unknown.len());
    match missing_verdict {
        Some(Verdict::Affected) => n_aff += entries.missing.len(),
        Some(Verdict::Unknown) => n_unk += entries.missing.len(),
        _ => {}
    }

    if !cmd.explain.is_empty() {
        let by_path: BTreeMap<&str, &turborepo_trace_graph::EntryResult> = report
            .entries
            .iter()
            .map(|e| (g.path(e.entry), e))
            .collect();
        // `--explain` makes the files it names entries. The selection, which runs
        // without them, lists a file that looks like a test and is no entry as an
        // unknown test: it always runs, whatever its trace would say.
        let unknown_tests: NodeSet = {
            let mut plain = cmd.entries.clone();
            plain.extend(cmd.entries_opt.iter().cloned());
            let mut selection = collect_entries(&g, &root, &plain, &cmd.entries_from, &cmd.tests)?;
            for n in task_entries.iter() {
                selection.set.insert(n);
            }
            let not_tests = cmd
                .not_tests
                .iter()
                .map(|p| Glob::new(p).map_err(|e| anyhow::anyhow!("{p}: {e}")))
                .collect::<Result<Vec<_>>>()?;
            let mut unknown = NodeSet::new(g.node_count());
            for n in crate::selection::unknown_tests(&g, &selection.set, &cmd.tests, &not_tests) {
                unknown.insert(n);
            }
            unknown
        };
        for x in &cmd.explain {
            let rel = rel_to_root(&root, &cwd, x).unwrap_or_else(|| x.clone());
            if g.node_by_path(&rel)
                .is_some_and(|n| unknown_tests.contains(n))
            {
                writeln!(
                    out,
                    "affected {rel}: an unknown test, which always runs: it looks like a test \
                     file, and no configuration of the test frameworks asked for lists it"
                )?;
                // Not a test here, and one in a checkout elsewhere: the place decides.
                if let Some(note) = crate::lint::above_root_note(&root, &rel) {
                    writeln!(out, "  assumed: {note}")?;
                }
                continue;
            }
            match by_path.get(rel.as_str()) {
                Some(e) => {
                    let single = turborepo_trace_graph::AffectedReport {
                        policy: report.policy,
                        entry_count: 1,
                        entries: vec![(*e).clone()],
                        seed_count: report.seed_count,
                        reached_count: report.reached_count,
                        unmatched: Vec::new(),
                    };
                    turborepo_trace_graph::output::write_affected_explain(&dg, &single, out)?;
                    // Which exports changed, and which file reads one of them.
                    if e.verdict == turborepo_trace_graph::Verdict::Affected {
                        export_notes.write(&g, &pruned, &e.witness, out)?;
                    }
                    if pruned.unaffected.contains(&e.entry)
                        || pruned.type_only.contains(&e.entry)
                        || pruned.mocked.iter().any(|m| m.entry == e.entry)
                        || pruned.unregistered.iter().any(|u| u.entry == e.entry)
                    {
                        pruned.write_reasons(&engine, e.entry, out)?;
                        export_notes.write_assumption(&engine, &pruned, e.entry, out)?;
                    }
                    let notes = engine.uncertainty_notes(e.entry, &aopts);
                    turborepo_trace_graph::output::write_uncertainty_notes(&g, &notes, 20, out)?;
                    turborepo_trace_graph::output::write_names_assumption(
                        &g, e.entry, &notes, &aopts, out,
                    )?;
                    turborepo_trace_graph::output::write_loads_assumption(
                        &g, e.entry, &notes, &aopts, out,
                    )?;
                    turborepo_trace_graph::output::write_temp_assumption(&notes, out)?;
                    turborepo_trace_graph::output::write_mount_assumption(&notes, out)?;
                    write_cwd_assumed(&g, e.entry, out)?;
                    // A test only by a pattern's reading above the repository root.
                    if let Some(note) = crate::lint::above_root_note(&root, &rel) {
                        writeln!(out, "  assumed: {note}")?;
                    }
                }
                None if entries.missing.contains(&rel) => writeln!(
                    out,
                    "affected {rel}: not in the graph (not a traced source file)"
                )?,
                // Only an entry has a verdict: a file the entry specs did not select is not
                // "unaffected" (nothing was evaluated for it).
                None if g
                    .node_by_path(&rel)
                    .is_some_and(|n| entries.set.contains(n)) =>
                {
                    writeln!(out, "unaffected {rel}")?;
                    if let Some(n) = g.node_by_path(&rel) {
                        pruned.write_reasons(&engine, n, out)?;
                        export_notes.write_assumption(&engine, &pruned, n, out)?;
                        let notes = engine.uncertainty_notes(n, &aopts);
                        turborepo_trace_graph::output::write_uncertainty_notes(
                            &g, &notes, 20, out,
                        )?;
                        turborepo_trace_graph::output::write_names_assumption(
                            &g, n, &notes, &aopts, out,
                        )?;
                        turborepo_trace_graph::output::write_loads_assumption(
                            &g, n, &notes, &aopts, out,
                        )?;
                        turborepo_trace_graph::output::write_temp_assumption(&notes, out)?;
                        turborepo_trace_graph::output::write_mount_assumption(&notes, out)?;
                        write_cwd_assumed(&g, n, out)?;
                        if let Some(note) = crate::lint::above_root_note(&root, &rel) {
                            writeln!(out, "  assumed: {note}")?;
                        }
                    }
                }
                None => {
                    writeln!(out, "{rel}: not an entry in the graph")?;
                    // No test here, and one in a checkout elsewhere: the place decides.
                    if let Some(note) = crate::lint::above_root_note(&root, &rel) {
                        writeln!(out, "  assumed: {note}")?;
                    }
                }
            }
        }
        for r in &cfx.reports {
            writeln!(out, "  note: {}", config_note(r))?;
        }
        for n in cfx.notes.iter().chain(&tsfx.notes) {
            writeln!(out, "  note: {n}")?;
        }
        if !steering_changes.is_empty() {
            let shown: Vec<&str> = steering_changes
                .iter()
                .take(3)
                .map(|c| c.path.as_str())
                .collect();
            writeln!(
                out,
                "  note: {} changed repo steering file(s) ({}{}) count only through explicit \
                 references to them{}",
                steering_changes.len(),
                shown.join(", "),
                if steering_changes.len() > shown.len() {
                    ", ..."
                } else {
                    ""
                },
                if steering_only {
                    "; nothing else changed, so uncertainty outside steering files does not count"
                } else {
                    ""
                }
            )?;
        }
    } else {
        match cmd.format {
            AffectedFormat::Selection => {
                let (mode, mode_source, _) = crate::selection::resolve_mode(cmd.mode);
                let globs = |list: &[String]| -> Result<Vec<Glob>> {
                    list.iter()
                        .map(|p| Glob::new(p).map_err(|e| anyhow::anyhow!("{p}: {e}")))
                        .collect()
                };
                let run_all = cmd.run_all_if_changed.clone();
                let doc = crate::selection::build(&crate::selection::Input {
                    g: &g,
                    report: &report,
                    entries: &entries.set,
                    missing: &entries.missing,
                    missing_verdict,
                    changes: &changes,
                    frameworks: &cmd.tests,
                    base: since_given,
                    base_commit: since,
                    merge_bases: git.as_ref().map_or(&[][..], |g| g.bases.as_slice()),
                    mode,
                    mode_source,
                    run_all: &globs(&run_all)?,
                    not_tests: &globs(&cmd.not_tests)?,
                    full_packages: &globs(&cmd.full_package)?,
                });
                emit_selection(doc, host.and_then(|h| h.sink.as_ref()), out)?;
            }
            AffectedFormat::List => {
                let mut all: Vec<&str> = affected.clone();
                if missing_verdict == Some(Verdict::Affected) {
                    all.extend(entries.missing.iter().map(String::as_str));
                }
                all.sort_unstable();
                for p in all {
                    writeln!(out, "{p}")?;
                }
                if !unknown.is_empty()
                    || (missing_verdict == Some(Verdict::Unknown) && !entries.missing.is_empty())
                {
                    eprintln!(
                        "note: {} entr(ies) unknown (uncertain dependencies); see --format json",
                        n_unk
                    );
                }
            }
            AffectedFormat::Json => {
                let mut results: Vec<serde_json::Value> = report
                    .entries
                    .iter()
                    .map(|e| turborepo_trace_graph::output::affected_entry_value(&dg, e))
                    .collect();
                pruned.annotate(&g, &mut results);
                if let Some(v) = missing_verdict {
                    for m in &entries.missing {
                        results.push(serde_json::json!({
                            "entry": m,
                            "verdict": v,
                            "witness": [],
                            "reason": "not in the graph (not a traced source file)",
                        }));
                    }
                }
                let mut hit = NodeSet::new(g.node_count());
                for e in &report.entries {
                    hit.insert(e.entry);
                }
                let unaffected: Vec<&str> = entries
                    .set
                    .iter()
                    .filter(|n| !hit.contains(*n))
                    .map(|n| g.path(n))
                    .collect();
                let packages: Vec<serde_json::Value> = fx
                    .packages
                    .iter()
                    .map(|p| {
                        let info = g.package_info(p.package);
                        serde_json::json!({
                            "name": info.name,
                            "version": info.version.as_str(),
                            "importers": p.importers.iter().map(|i| g.package_info(*i).root).collect::<Vec<_>>(),
                            "source": p.source,
                            "reason": p.reason,
                        })
                    })
                    .collect();
                let v = serde_json::json!({
                    "policy": report.policy,
                    "base": git.as_ref().map(|g| serde_json::json!({
                        "given": g.given,
                        "commit": g.rev,
                        "merge_bases": g.bases,
                        "exact": g.exact,
                    })),
                    "entries": report.entry_count + entries.missing.len(),
                    "affected": n_aff,
                    "unknown": n_unk,
                    "unaffected": unaffected.len(),
                    "changes": cs.files,
                    "dropped_changes": fx.drop,
                    "configs": cfx.reports,
                    "steering": {
                        "changes": steering_changes,
                        "reached": steering_reach.len(),
                        "only": steering_only,
                    },
                    "packages": packages,
                    "notes": fx.notes,
                    "seeds": report.seed_count,
                    "reached": report.reached_count,
                    "results": results,
                    "unaffected_entries": unaffected,
                    "unmatched": report.unmatched,
                    "barrels": {
                        "candidates": barrel_candidates.iter().map(|c| &c.path).collect::<Vec<_>>(),
                        "not_skippable": barrel_rejected
                            .iter()
                            .map(|(p, why)| serde_json::json!({ "path": p, "reason": why }))
                            .collect::<Vec<_>>(),
                        "skipped": pruned
                            .skipped
                            .iter()
                            .map(|s| serde_json::json!({
                                "path": g.path(s.node),
                                "barrels": s.barrels.iter().map(|&b| g.path(b)).collect::<Vec<_>>(),
                                "relevant_importers": s.relevant,
                                "entries_skipped": s.entries,
                            }))
                            .collect::<Vec<_>>(),
                    },
                    "pruning": pruning_json,
                    "uncertainty": engine.uncertainty_counts(&aopts),
                });
                serde_json::to_writer_pretty(&mut *out, &v)?;
                writeln!(out)?;
            }
        }
    }
    turborepo_trace_core::timing::mark("affected: output");
    if cmd.exit_code {
        out.flush()?;
        return Ok(ExitCode::from(if n_unk > 0 {
            2
        } else if n_aff > 0 {
            1
        } else {
            0
        }));
    }
    Ok(ExitCode::SUCCESS)
}

/// For every changed turbo config (`turbo.json`, `turbo.jsonc`), the sections
/// of it that changed, as changes of the section pseudo-paths that tasks have
/// as inputs (`turbo.json#tasks/build`, `turbo.json#global`; see
/// `turborepo_trace_tasks::sources::turbo`). With the old contents (`--since`),
/// only the task entries that differ; without them, or when the config was
/// added, deleted or does not parse, the global section, which every
/// task the config governs depends on.
fn turbo_section_changes(
    root: &Path,
    changes: &[Change],
    git: Option<&lockdiff::Git>,
) -> Vec<Change> {
    use turborepo_trace_tasks::sources::turbo;
    let mut out = Vec::new();
    for c in changes.iter().filter(|c| turbo::is_config(&c.path)) {
        let old = (c.kind != ChangeKind::Added)
            .then(|| git.and_then(|g| g.show(&c.path)))
            .flatten();
        let new = (c.kind != ChangeKind::Deleted)
            .then(|| std::fs::read_to_string(root.join(&c.path)).ok())
            .flatten();
        for s in turbo::changed_sections(old.as_deref(), new.as_deref()) {
            out.push(Change::new(ChangeKind::Modified, format!("{}{s}", c.path)));
        }
    }
    out
}
