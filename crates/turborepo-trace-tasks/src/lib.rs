//! Build tooling for `turbo trace`: package scripts, task runners and
//! code generators, **recognized, never executed**.
//!
//! # Pipeline
//!
//! 1. [`sources`]: a [`sources::TaskSource`] finds the tasks a package declares
//!    (`package.json` scripts, `turbo.json`, Makefiles, Dockerfiles, ...) as
//!    [`sources::DiscoveredTask`]s: an id, command lines, declared spec parts.
//! 2. [`shell`]: each command line is split into simple [`spec::Command`]s
//!    (quoting, `&&`, `cd dir &&`, redirections, known variables; unsupported
//!    syntax becomes uncertainty).
//! 3. [`registry`]: each command goes to the first matching
//!    [`registry::Recognizer`], which adds inputs, outputs, configs, entries,
//!    memberships, task deps and toolchain packages to the task's
//!    [`spec::TaskSpec`]. Unknown programs make the task depend on its whole
//!    package directory, with an `UnknownCommand` uncertainty.
//! 4. [`graph`]: task references (`^build`, `pnpm -F web run build`) are
//!    resolved to [`spec::TaskId`]s over the workspace.
//!
//! # Example
//!
//! ```no_run
//! use turborepo_trace_tasks::{discover, Options, sources::Workspace};
//!
//! let ws = Workspace::discover(std::path::Path::new("/repo"));
//! let set = discover(&ws, &Options::default());
//! for t in &set.tasks {
//!     println!("{} inputs={} outputs={}", t.id, t.spec.inputs.len(), t.spec.outputs.len());
//! }
//! let build = set.get(&"task:apps/web#build".parse().unwrap());
//! # let _ = build;
//! ```
//!
//! # Extension
//!
//! Recognizer and source modules plug in through
//! [`recognizers::codegen::recognizers`] and each source module's `sources()` /
//! `recognizers()`, so no central file needs editing.

// Every public item is documented.
#![warn(missing_docs)]

pub mod fallback;
pub mod format;
pub mod glob;
pub mod graph;
pub mod recognizers;
pub mod registry;
pub mod runs;
pub mod scanners;
pub mod shell;
pub mod sources;
pub mod spec;
pub mod specs;

use std::{collections::BTreeMap, path::PathBuf, str::FromStr};

use serde::Serialize;

pub use crate::spec::{Command, TaskRef};
use crate::{
    registry::{CommandReport, Recognition, RecognizeCx, registry},
    sources::{Contribution, Package, SourceCx, Workspace},
    spec::{TaskId, TaskSpec},
};

impl FromStr for TaskId {
    type Err = String;
    fn from_str(s: &str) -> Result<TaskId, String> {
        TaskId::parse(s).ok_or_else(|| format!("invalid task id `{s}`"))
    }
}

/// Options for [`discover`].
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Known environment variables for expansion in command lines. Empty by
    /// default: nothing from the real environment is used, so results are
    /// reproducible.
    pub env: BTreeMap<String, String>,
}

/// One analyzed task.
#[derive(Clone, Debug, Serialize)]
pub struct Task {
    /// Id.
    pub id: TaskId,
    /// Display name (`build`).
    pub name: String,
    /// Package root, repo-relative (`.` for the root).
    pub package: String,
    /// Package name, if the manifest has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_name: Option<String>,
    /// Ids of the sources that contributed (`npm`, `turbo`).
    pub sources: Vec<&'static str>,
    /// Files that define the task (also in `spec.configs`).
    pub files: Vec<PathBuf>,
    /// Command lines as written.
    pub lines: Vec<String>,
    /// Per-command recognition results.
    pub commands: Vec<CommandReport>,
    /// Per command line (parallel to [`Task::lines`]): the worst recognition of
    /// its commands (`Ignored` counts as recognized), at least `Partial`
    /// when the line uses unsupported shell syntax, `Ignored` for a line
    /// without commands.
    pub line_recognition: Vec<Recognition>,
    /// Inputs, outputs, deps, ... (declared plus recognized).
    pub spec: TaskSpec,
    /// Resolved task dependencies ([`graph::resolve`]); empty before
    /// resolution.
    pub deps: Vec<TaskId>,
    /// Filesystem observations made while recognizing this task's commands
    /// (config discovery probes, config reads), recorded through the shared
    /// `DirCache` when the caller runs inside
    /// [`turborepo_trace_frameworks::fsx::with_fs`] with a recording cache.
    /// Observations of task *discovery* (reading `package.json`, `turbo.json`)
    /// go to the caller's recording frame instead, since they are shared by
    /// every task of the package.
    #[serde(skip)]
    pub observations: turborepo_trace_core::Observations,
    /// What discovery computed the task from without it being an input of the
    /// task ([`sources::DiscoveredTask::decided_by`]), from every
    /// contribution. A graph store keys it with the package (it decides
    /// which tasks and memberships the package has); nothing depends on it.
    #[serde(skip)]
    pub decided_by: turborepo_trace_core::Observations,
}

impl Task {
    /// The worst recognition over the task's commands (`Full` with no
    /// commands).
    pub fn recognition(&self) -> Recognition {
        let mut r = Recognition::Full;
        for c in &self.commands {
            r = match (r, c.recognition) {
                (_, Recognition::Unknown) | (Recognition::Unknown, _) => Recognition::Unknown,
                (_, Recognition::Partial) | (Recognition::Partial, _) => Recognition::Partial,
                (x, _) => x,
            };
        }
        r
    }
}

/// The tasks of a workspace.
#[derive(Clone, Debug, Default, Serialize)]
pub struct TaskSet {
    /// Repository root.
    pub root: PathBuf,
    /// Tasks, sorted by id.
    pub tasks: Vec<Task>,
}

impl TaskSet {
    /// The task with id `id`.
    pub fn get(&self, id: &TaskId) -> Option<&Task> {
        self.tasks
            .binary_search_by(|t| t.id.cmp(id))
            .ok()
            .map(|i| &self.tasks[i])
    }

    /// `id` and every task it depends on, transitively (after
    /// [`graph::resolve`]).
    pub fn closure(&self, id: &TaskId) -> Vec<&Task> {
        let mut seen = std::collections::BTreeSet::new();
        let mut out = Vec::new();
        let mut todo = vec![id.clone()];
        while let Some(i) = todo.pop() {
            if !seen.insert(i.clone()) {
                continue;
            }
            if let Some(t) = self.get(&i) {
                todo.extend(t.deps.iter().cloned());
                out.push(t);
            }
        }
        out
    }

    /// Tasks whose linking outputs ([`spec::Output::links_producer`]) or output
    /// packages cover `file` (absolute): the producers of a generated file.
    pub fn producers_of(&self, file: &std::path::Path) -> Vec<&Task> {
        self.tasks
            .iter()
            .filter(|t| {
                t.spec
                    .outputs
                    .iter()
                    .any(|o| o.links_producer() && o.covers(file))
                    || {
                        let pkg = turborepo_trace_frameworks::fsx::join(&self.root, &t.package);
                        t.spec
                            .output_packages
                            .iter()
                            .any(|p| p.covers(file, &pkg, &self.root))
                    }
            })
            .collect()
    }
}

/// Discovers and analyzes every task of `ws`, with dependencies resolved.
pub fn discover(ws: &Workspace, opts: &Options) -> TaskSet {
    let mut tasks = Vec::new();
    for p in &ws.packages {
        tasks.extend(analyze_package(ws, p, opts));
    }
    tasks.sort_by(|a, b| a.id.cmp(&b.id));
    let mut set = TaskSet {
        root: ws.root.clone(),
        tasks,
    };
    graph::resolve(ws, &mut set);
    set
}

/// Discovers and analyzes the tasks of one package (dependencies unresolved).
/// This is the unit the graph store caches per package shard.
pub fn analyze_package(ws: &Workspace, pkg: &Package, opts: &Options) -> Vec<Task> {
    let cx = SourceCx {
        workspace: ws,
        package: pkg,
    };
    // Merge contributions by id: definitions first, then augmentations.
    let mut defs: BTreeMap<TaskId, (Vec<&'static str>, Vec<sources::DiscoveredTask>)> =
        BTreeMap::new();
    let mut augments: Vec<(&'static str, sources::DiscoveredTask)> = Vec::new();
    for src in sources::sources() {
        // Discovery runs in the package's shared frame: everything it records becomes
        // an observation of every task of the package, and the store links a
        // task to the producer of any output inside a directory it listed.
        // Directory walks that look for config files would therefore make every
        // task depend on every generator in the package. A source that names
        // its config files gets its listings replaced by one narrow listing (`*
        // */{names}`), so adding such a file still changes the package's tasks.
        // Reads and probes stay shared (config files are inputs of every task);
        // listings a task's own spec needs are expressed as memberships.
        // Sources without names (`gha`) record their own narrow listings.
        let names = src.config_file_names();
        let found = if names.is_empty() {
            src.discover(&cx)
        } else {
            let found = turborepo_trace_core::record::pause_listings(|| src.discover(&cx));
            let pattern = if names.len() == 1 {
                format!("**/{}", names[0])
            } else {
                format!("**/{{{}}}", names.join(","))
            };
            turborepo_trace_core::record::listing(&pkg.dir, &pattern);
            found
        };
        for d in found {
            let Some(id) = d.id.clone() else { continue };
            match d.mode {
                Contribution::Defines => {
                    let e = defs.entry(id).or_default();
                    e.0.push(src.id());
                    e.1.push(d);
                }
                Contribution::Augments => augments.push((src.id(), d)),
            }
        }
    }
    for (sid, d) in augments {
        if let Some(e) = d.id.as_ref().and_then(|id| defs.get_mut(id)) {
            e.0.push(sid);
            e.1.push(d);
        }
    }
    let scripts: Vec<String> = defs
        .keys()
        .filter_map(|id| match id {
            TaskId::Script { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();
    let reg = registry();
    let analyze_task =
        |id: TaskId, mut srcs: Vec<&'static str>, parts: Vec<sources::DiscoveredTask>| -> Task {
            srcs.dedup();
            let ci = srcs.iter().any(|s| {
                sources::sources()
                    .iter()
                    .any(|src| src.id() == *s && src.ci_side_effects())
            });
            let mut spec = TaskSpec::default();
            let mut files = Vec::new();
            let mut lines = Vec::new();
            let mut commands = Vec::new();
            let mut line_recognition = Vec::new();
            let mut name = String::new();
            // Whether a command may write files the spec doesn't name (see below).
            let mut opaque = false;
            let mut decided_by = turborepo_trace_core::Observations::default();
            let frame = turborepo_trace_core::record::start();
            for d in parts {
                if name.is_empty() {
                    name = d.name.clone();
                }
                decided_by.extend_from(&d.decided_by);
                files.extend(d.files.iter().cloned());
                spec.merge(d.spec);
                let mut env = opts.env.clone();
                env.extend(d.env);
                for line in &d.lines {
                    lines.push(line.clone());
                    let parsed = shell::parse(line, &d.cwd, &env);
                    if !parsed.uncertainties.is_empty() {
                        // Skipped constructs (`case`, `eval`, functions) may run anything.
                        spec.membership(pkg.dir.clone(), "**", "unsupported shell syntax");
                        runs::scan_unsupported_shell(line, &ws.root, &mut spec);
                    }
                    let shell_issue = !parsed.uncertainties.is_empty();
                    opaque |= shell_issue;
                    let mut recs = Vec::new();
                    spec.uncertainties.extend(parsed.uncertainties);
                    for cmd in &parsed.commands {
                        let rcx = RecognizeCx {
                            repo_root: &ws.root,
                            package_root: &pkg.dir,
                            env: &env,
                            task_name: Some(&name),
                            scripts: &scripts,
                        };
                        let before = (spec.parts.len(), footprint(&spec));
                        let rep = reg.recognize(cmd, &rcx, &mut spec);
                        // A command is transparent when it has no file effect, when it only
                        // runs other tasks (`pnpm typegen`: their writes are theirs), or when
                        // it is made of parts. Anything else (`node patch-dist.js`) may write
                        // files the spec doesn't name.
                        opaque |= spec.parts.len() == before.0
                            && rep.recognition != Recognition::Ignored
                            && (rep.recognition != Recognition::Full
                                || footprint(&spec) != before.1);
                        recs.push(rep.recognition);
                        commands.push(rep);
                    }
                    line_recognition.push(line_recognition_of(&recs, shell_issue));
                }
            }
            let observations = frame.finish();
            // Whether a task this one depends on or runs can write into this package: a
            // task of the same package. Tasks of other packages (turbo `^build`) write
            // their own packages, as declared outputs link only inside their package.
            let local_writer = spec
                .task_deps
                .iter()
                .any(|d| may_write_into(&d.target, pkg, ws, &scripts));
            if !spec.parts.is_empty() {
                if opaque && pkg.rel == "." {
                    // A root script may write anywhere in the repository: another command of
                    // the task may rewrite what a part wrote (`tsc -b && node patch-dists.js`)
                    // or generate what it reads. Importers of the outputs depend on the whole
                    // task, as without parts.
                    spec.drop_parts();
                } else if opaque || local_writer {
                    // Another command of the task (`bunchee && tsc -b`,
                    // `tsc -b && node patch-dist.js`) or a task of the same package that it
                    // depends on or runs (`dependsOn: ["typegen"]`, a `pre` hook,
                    // `pnpm typegen && tsc -b`) may rewrite what the package's own projects
                    // emit, or generate their sources without declaring them: the outputs of
                    // those projects also stay outputs of the task. A project of another
                    // workspace package is fed and written by that package's tasks (as for
                    // declared outputs, which link only inside their package), not by this
                    // task's other commands or dependencies. For the same reason a
                    // dependency on other packages' tasks alone (turbo `^build`) does not
                    // make the task a producer of its own projects' outputs: what a project
                    // emits depends on those packages only through what its sources import,
                    // and the sources' own edges carry that. (Linked to the task, an importer
                    // of `dist/index.js` inherited the whole dependency chain of the task:
                    // every upstream build script and its uncertainty.)
                    let nested: Vec<&std::path::Path> = ws
                        .packages
                        .iter()
                        .filter(|p| p.dir != pkg.dir && p.dir.starts_with(&pkg.dir))
                        .map(|p| p.dir.as_path())
                        .collect();
                    let own = |part: &std::path::Path| {
                        part.starts_with(&pkg.dir) && !nested.iter().any(|n| part.starts_with(n))
                    };
                    let copies: Vec<spec::Output> = spec
                        .outputs
                        .iter()
                        .filter(|o| o.part.as_deref().is_some_and(own))
                        .map(|o| spec::Output {
                            part: None,
                            ..o.clone()
                        })
                        .collect();
                    spec.outputs.extend(copies);
                }
            }
            if !spec.uncertainties.is_empty() {
                // Every uncertain task falls back to its whole package (also for uncertainties
                // declared by task sources).
                spec.membership(pkg.dir.clone(), "**", "uncertain task");
            }
            if !line_recognition.is_empty()
                && line_recognition.iter().all(|r| *r == Recognition::Ignored)
                && spec.uncertainties.is_empty()
            {
                // Every command has no file effect (`echo "dummy target for caching"`): the
                // task writes nothing, so its declared outputs are only what
                // turbo restores from the cache. A root task such as
                // `//#cache-tsbuildinfo-files` declares every package's
                // `dist/tsbuildinfo.json` that way.
                for o in &mut spec.outputs {
                    if o.kind == spec::OutputKind::Declared {
                        o.kind = spec::OutputKind::CacheOnly;
                    }
                }
            }
            if !spec.parts.is_empty() && !opaque && !local_writer && spec.uncertainties.is_empty() {
                // Every command of the task is made of parts, has no file effect or only
                // runs tasks of other packages: the task writes exactly what its parts
                // write, and nothing it depends on writes into this package. What its
                // declared outputs name beyond that (turbo `outputs: ["dist"]` on a
                // `tsc -b` type-check, there to restore the `.d.ts` files from the cache)
                // is produced by the tasks that write it, which link on their own.
                for o in &mut spec.outputs {
                    if o.kind == spec::OutputKind::Declared && o.part.is_none() {
                        o.kind = spec::OutputKind::CacheOnly;
                    }
                }
            }
            if spec.persistent {
                // A persistent task (`tsc -b -w`, a dev server): no task can depend on it and
                // builds don't run it, so nothing built or tested reads what it writes.
                for o in &mut spec.outputs {
                    if o.links_producer() {
                        o.kind = spec::OutputKind::Persistent;
                    }
                }
            }
            if ci {
                // CI and container tasks are never producers: whatever their steps write
                // (`mv tmp package.json` in a bump job) is recorded but does not link.
                spec.classify_all_outputs(spec::OutputKind::CiSideEffect);
            }
            // A write to a path that is not a known place in the repository produces no
            // file anyone can name: recorded, not linked ([`spec::unplaced`]).
            for o in &mut spec.outputs {
                if o.links_producer() && spec::unplaced(&o.path, &ws.root) {
                    o.kind = spec::OutputKind::Unplaced;
                }
            }
            // A task that produces outputs does not depend on a formatting script it calls.
            format::drop_in_place_deps(&mut spec);
            // The whole-package fallback of a root task: the root package's own files,
            // unless the task works on the workspace.
            fallback::settle(ws, pkg, &lines, &mut spec);
            spec.configs.extend(files.iter().cloned());
            files.sort();
            files.dedup();
            spec.normalize();
            Task {
                id,
                name,
                package: pkg.rel.clone(),
                package_name: pkg.name.clone(),
                sources: srcs,
                files,
                lines,
                commands,
                line_recognition,
                spec,
                deps: Vec::new(),
                observations,
                decided_by,
            }
        };
    /// One task to analyze: its id, the sources that define it and their parts.
    type TaskDef = (TaskId, (Vec<&'static str>, Vec<sources::DiscoveredTask>));
    let defs: Vec<TaskDef> = defs.into_iter().collect();
    // Tasks are independent (each records in its own frame): a package with many
    // (the root of a big monorepo: hundreds of workflow jobs and scripts) is
    // analyzed on several threads, each with this thread's shared dir cache and
    // pause state, in task order.
    const PARALLEL_MIN: usize = 16;
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(16);
    if defs.len() < PARALLEL_MIN || threads < 2 {
        return defs
            .into_iter()
            .map(|(id, (srcs, parts))| analyze_task(id, srcs, parts))
            .collect();
    }
    let shared = {
        let (a, b) = (
            turborepo_trace_frameworks::fsx::cache(),
            turborepo_trace_frameworks::fsx::cache(),
        );
        std::sync::Arc::ptr_eq(&a, &b).then_some(a)
    };
    let pauses = turborepo_trace_core::record::pauses();
    let n = defs.len();
    let work: Vec<std::sync::Mutex<Option<TaskDef>>> = defs
        .into_iter()
        .map(|d| std::sync::Mutex::new(Some(d)))
        .collect();
    let results: Vec<std::sync::Mutex<Option<Task>>> =
        (0..n).map(|_| std::sync::Mutex::new(None)).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..threads.min(n) {
            scope.spawn(|| {
                let run = || loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if i >= n {
                        break;
                    }
                    let Some((id, (srcs, parts))) =
                        work[i].lock().unwrap_or_else(|e| e.into_inner()).take()
                    else {
                        continue;
                    };
                    let t = turborepo_trace_core::record::with_pauses(pauses, || {
                        analyze_task(id, srcs, parts)
                    });
                    *results[i].lock().unwrap_or_else(|e| e.into_inner()) = Some(t);
                };
                match &shared {
                    Some(fs) => turborepo_trace_frameworks::fsx::with_fs(fs, run),
                    None => run(),
                }
            });
        }
    });
    results
        .into_iter()
        .filter_map(|r| r.into_inner().unwrap_or_else(|e| e.into_inner()))
        .collect()
}

/// Whether the task(s) `target` names, as a dependency of a task of `pkg`, can
/// write into `pkg`'s directory: a task of the same package that exists
/// (`scripts`: the package's task names). Tasks of other packages write their
/// own packages. A reference that cannot be placed (a glob over packages, an id
/// without a directory) counts.
fn may_write_into(
    target: &TaskRef,
    pkg: &sources::Package,
    ws: &Workspace,
    scripts: &[String],
) -> bool {
    match target {
        TaskRef::Local { name } | TaskRef::All { name } => scripts.contains(name),
        TaskRef::Upstream { .. } => false,
        TaskRef::Package { package, name } => {
            if glob::is_glob(package) {
                scripts.contains(name)
            } else {
                ws.package(package)
                    .is_some_and(|p| p.rel == pkg.rel && scripts.contains(name))
            }
        }
        TaskRef::Id { id } => match id.dir() {
            Some(d) => pkg.rel == "." || d == pkg.rel || d.starts_with(&format!("{}/", pkg.rel)),
            None => true,
        },
    }
}

/// The sizes of the parts of a spec that describe file effects (everything but
/// task dependencies, configs, toolchain and env names): unchanged by a command
/// that only runs other tasks.
fn footprint(spec: &TaskSpec) -> [usize; 7] {
    [
        spec.inputs.len(),
        spec.data_inputs.len(),
        spec.outputs.len(),
        spec.output_packages.len(),
        spec.entries.len(),
        spec.memberships.len(),
        spec.uncertainties.len(),
    ]
}

/// A line's recognition from its commands': `Unknown` if every command is
/// unknown, `Partial` if some are unknown or partial (or the shell syntax is
/// not fully supported), `Ignored` if every command is ignored, else `Full`.
pub fn line_recognition_of(recs: &[Recognition], shell_issue: bool) -> Recognition {
    use Recognition::*;
    if recs.is_empty() {
        return if shell_issue { Partial } else { Ignored };
    }
    if recs.iter().all(|r| *r == Unknown) {
        return Unknown;
    }
    if shell_issue || recs.iter().any(|r| matches!(r, Unknown | Partial)) {
        return Partial;
    }
    if recs.iter().all(|r| *r == Ignored) {
        return Ignored;
    }
    Full
}
