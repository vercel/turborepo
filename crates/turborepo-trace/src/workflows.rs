//! CI workflow changes in `affected`: which jobs changed, and which tests they
//! run.
//!
//! A workflow file can't change what the application does, but it can change
//! how CI runs the tests: the runner image, tool versions, environment,
//! services, the test command. Treating the file as an input of everything
//! selects every test for every deploy-job edit; ignoring it skips tests whose
//! job changed. The model here:
//!
//! 1. **Changed jobs.** A job (`gha:<workflow>#<job>`) changes when
//!    - its *effective definition* differs between the base revision and the
//!      working tree ([`turborepo_trace_tasks::sources::gha::def`]: the job's
//!      keys, the workflow-level keys that affect execution, and the local
//!      actions its steps use; triggers, path filters, names, concurrency, a
//!      job-level `if`, comments and formatting are not part of it). Without a
//!      base revision, every job whose definition is read from a changed file;
//!    - one of its *definition files* changed (`TaskRec::runs`, `def:`
//!      entries): the script a step runs with an interpreter, a shell script
//!      and the files it sources, a file a step names (`node-version-file:
//!      .nvmrc`), a local action's code, a test runner's config; or a file such
//!      a script imports *inside the CI tooling area*: below `.github/` or
//!      below the script's own directory;
//!    - a job it needs, or a job of a reusable workflow it calls, changed (the
//!      job consumes their outputs, artifacts and results).
//!
//!    Application code is not part of a job's definition. A source file that a
//!    job's script imports from a package, or that a job's build or test
//!    command reads, is an *input* of the job: the job is an affected task (the
//!    graph says so), but how it runs its tests did not change, so it selects
//!    none here. Directory listings, `paths:` filters, `hashFiles` and
//!    whole-package fallbacks are never followed.
//! 2. **What a changed job selects.** The tests it runs (`TaskRec::runs`,
//!    [`turborepo_trace_tasks::runs`]): test-runner invocations of its steps,
//!    the package scripts and turbo tasks they start (transitively), and the
//!    jobs of reusable workflows it calls. Not the jobs it needs. A job that
//!    starts no tests selects none. Every test is selected only on evidence of
//!    a test launch that can't be mapped (a task runner named with a test task
//!    in text the recognizers can't see through), or for a computed task name
//!    in a job that is otherwise known to run tests. Nothing else flows out of
//!    a job.
//!
//!    The same holds for a package script: when a definition file of a script
//!    that itself launches tests changes (`"test": "jest -c
//!    jest.ci.config.js"`, a wrapper script that starts the runner), the tests
//!    that script's command runs are selected: its own test runs and the tasks
//!    the command starts. The tasks the script merely depends on (a `dependsOn`
//!    entry that provisions a database) are not: they run before it, as
//!    processes of their own, and their definition did not change.
//! 3. **The files themselves.** A modified workflow or action file is not a
//!    file change for the graph (it would reach every task of the root package
//!    and every whole-directory listing). Code that reads it through an
//!    explicit reference still counts: entries that reach the file through
//!    file-to-file edges are affected.
//!
//! [`effects`] computes this for a change set; [`Effects::apply`] adds the
//! verdicts to the engine's report.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::Path,
};

use turborepo_trace_graph::{
    AffectedReport, Cause, Change, ChangeKind, Engine, EntryResult, GraphView, NodeId, NodeSet,
    PackageId, PackageVersion, RefKind, Verdict,
};
use turborepo_trace_store::Graph;
use turborepo_trace_tasks::sources::gha::def::{self, Defs, JobDef};

use crate::lockdiff::Git;

/// The graph the engine runs on: the store's graph with its derived layer. Node
/// ids are those of the base graph.
type View<'g> = turborepo_trace_graph::Patched<'g, Graph>;

/// What workflow changes add to an `affected` query.
#[derive(Debug, Default)]
pub struct Effects {
    /// Modified workflow and action files handled here: not file changes for
    /// the engine.
    pub drop: BTreeSet<String>,
    /// Changed jobs: the task node and the changed file that caused it.
    pub jobs: BTreeMap<NodeId, String>,
    /// Tests the changed jobs (and changed test-launching scripts) run: the
    /// test, the job or script task that runs it, and the changed file.
    pub tests: BTreeMap<NodeId, (NodeId, String)>,
    /// Entries that reach a dropped file through file-to-file edges: the chain
    /// from the file to the entry, both inclusive.
    pub readers: BTreeMap<NodeId, Vec<NodeId>>,
    /// Readable notes for `--format json` and `--stats`.
    pub notes: Vec<String>,
}

/// `gha:<workflow>#<job>` → (workflow path, job id).
fn job_of(id: &str) -> Option<(&str, &str)> {
    id.strip_prefix("gha:")?.rsplit_once('#')
}

fn is_pseudo(g: &Graph, n: NodeId) -> bool {
    turborepo_trace_store::tasks::is_pseudo_lang(g.lang(n))
}

/// Whether `path` is `dir` or below it (`.`: everything).
fn under(dir: &str, path: &str) -> bool {
    let dir = dir.trim_end_matches('/');
    dir.is_empty()
        || dir == "."
        || path == dir
        || path
            .strip_prefix(dir)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Resolves the `runs` records of task nodes to tests.
pub(crate) struct Runs<'g> {
    g: &'g Graph,
    /// Task records by node.
    recs: HashMap<NodeId, &'g turborepo_trace_store::shard::ArchivedTaskRec>,
    /// Package scripts by name: (package root, `.` for the repository root;
    /// node).
    scripts: HashMap<String, Vec<(String, NodeId)>>,
    /// First-party package names by root.
    names: HashMap<String, String>,
    /// Test nodes per framework.
    tests: HashMap<String, Vec<NodeId>>,
}

/// Which tasks name a task ([`Runs::parent_index`]).
#[derive(Default)]
pub(crate) struct ParentIndex<'g> {
    /// Script name → (referencing task, its package selector).
    by_script: HashMap<&'g str, Vec<(NodeId, &'g str)>>,
    /// Task id → referencing tasks.
    by_id: HashMap<&'g str, Vec<NodeId>>,
}

/// What one job runs.
#[derive(Default)]
pub(crate) struct JobRuns {
    /// Test runs: (framework, directory).
    tests: BTreeSet<(String, String)>,
    /// The first test launch that can't be mapped.
    unknown: Option<String>,
    /// The first task runner with a computed task name.
    computed: Option<String>,
}

impl JobRuns {
    /// Why every test counts, if it does: a launch that can't be mapped, or a
    /// computed task name in a task that is known to run tests.
    pub(crate) fn every_test(&self) -> Option<&str> {
        self.unknown
            .as_deref()
            .or_else(|| self.computed.as_deref().filter(|_| !self.tests.is_empty()))
    }

    /// Whether the task runs any test.
    pub(crate) fn any(&self) -> bool {
        !self.tests.is_empty() || self.unknown.is_some()
    }

    /// The test launches that are mapped: (framework, directory).
    pub(crate) fn launches(&self) -> impl Iterator<Item = (&str, &str)> {
        self.tests
            .iter()
            .map(|(fw, dir)| (fw.as_str(), dir.as_str()))
    }
}

impl<'g> Runs<'g> {
    pub(crate) fn new(g: &'g Graph) -> Self {
        let mut recs = HashMap::new();
        let mut scripts: HashMap<String, Vec<(String, NodeId)>> = HashMap::new();
        for (n, t) in g.tasks() {
            recs.insert(n, t);
            if let Some((root, name)) =
                t.id.as_str()
                    .strip_prefix("task:")
                    .and_then(|r| r.rsplit_once('#'))
            {
                scripts
                    .entry(name.to_owned())
                    .or_default()
                    .push((root.to_owned(), n));
            }
        }
        let mut names = HashMap::new();
        for i in 0..g.package_count() {
            let info = g.package_info(PackageId(i));
            if matches!(info.version, PackageVersion::Workspace) {
                let root = if info.root.is_empty() { "." } else { info.root };
                names.insert(root.to_owned(), info.name.to_owned());
            }
        }
        Runs {
            g,
            recs,
            scripts,
            names,
            tests: HashMap::new(),
        }
    }

    /// The script tasks `script:<sel>#<name>` names.
    fn scripts_of(&self, sel: &str, name: &str) -> Vec<NodeId> {
        let Some(list) = self.scripts.get(name) else {
            return Vec::new();
        };
        if sel == "*" {
            return list.iter().map(|(_, n)| *n).collect();
        }
        list.iter()
            .filter(|(root, _)| self.selects(sel, root))
            .map(|(_, n)| *n)
            .collect()
    }

    /// Whether the selector of a `script:<sel>#<name>` record names the package
    /// at `root`.
    fn selects(&self, sel: &str, root: &str) -> bool {
        if sel == "*" {
            return true;
        }
        let dir = turborepo_trace_tasks::sources::norm_rel(sel);
        let pkg = self.names.get(root).map(String::as_str);
        root == dir
            || pkg == Some(sel)
            || (turborepo_trace_tasks::glob::is_glob(sel)
                && (turborepo_trace_tasks::glob::matches(sel, root)
                    || turborepo_trace_tasks::glob::matches(&dir, root)
                    || pkg.is_some_and(|p| turborepo_trace_tasks::glob::matches(sel, p))))
    }

    /// The references of every task, by what they name: which tasks run a
    /// script of a given name (with their selector), and which name a task
    /// by id. The inverse of what [`Runs::of`] follows: run records, and
    /// for tasks other than workflow jobs their task dependencies.
    pub(crate) fn parent_index(&self) -> ParentIndex<'g> {
        let mut ix = ParentIndex::default();
        for (&n, &rec) in &self.recs {
            for r in rec.runs.iter() {
                let r = r.as_str();
                if let Some(rest) = r.strip_prefix("script:") {
                    if let Some((sel, name)) = rest.rsplit_once('#') {
                        ix.by_script.entry(name).or_default().push((n, sel));
                    }
                } else if let Some(id) = r.strip_prefix("id:") {
                    ix.by_id.entry(id).or_default().push(n);
                }
            }
            if !rec.id.as_str().starts_with("gha:") {
                for d in rec.deps.iter() {
                    ix.by_id.entry(d.as_str()).or_default().push(n);
                }
            }
        }
        ix
    }

    /// The tasks that run or depend on the task at `n` directly.
    pub(crate) fn parents(&self, n: NodeId, ix: &ParentIndex<'g>) -> Vec<NodeId> {
        let Some(rec) = self.recs.get(&n) else {
            return Vec::new();
        };
        let id = rec.id.as_str();
        let mut out: Vec<NodeId> = ix.by_id.get(id).cloned().unwrap_or_default();
        if let Some((root, name)) = id.strip_prefix("task:").and_then(|r| r.rsplit_once('#')) {
            for &(p, sel) in ix.by_script.get(name).into_iter().flatten() {
                if self.selects(sel, root) {
                    out.push(p);
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The services the task at `n` starts itself (`service:<task id>`
    /// records): the nodes of the tasks that define them.
    pub(crate) fn own_services(&self, n: NodeId) -> Vec<NodeId> {
        let Some(rec) = self.recs.get(&n) else {
            return Vec::new();
        };
        rec.runs
            .iter()
            .filter_map(|r| r.as_str().strip_prefix("service:"))
            .filter_map(|id| self.g.node_by_path(id))
            .collect()
    }

    /// Every task node with a record.
    pub(crate) fn tasks(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.recs.keys().copied()
    }

    /// Everything the task at `start` runs: its own records, the tasks they
    /// name, and for tasks other than workflow jobs their task dependencies
    /// (a job's dependencies include the jobs it needs, which it does not
    /// run).
    pub(crate) fn of(&self, start: NodeId) -> JobRuns {
        self.walk(start, true)
    }

    /// What the command of the package script at `start` runs: its own records
    /// and the tasks they name, with everything those run ([`Runs::of`]).
    /// Not the script's own task dependencies: a task runner starts them
    /// before the script, as processes of their own, and nothing the
    /// script's command is made of reaches them.
    pub(crate) fn of_command(&self, start: NodeId) -> JobRuns {
        self.walk(start, false)
    }

    /// [`Runs::of`]; with `own_deps` off, without the task dependencies of
    /// `start` itself.
    fn walk(&self, start: NodeId, own_deps: bool) -> JobRuns {
        let mut out = JobRuns::default();
        let mut seen: BTreeSet<NodeId> = BTreeSet::new();
        let mut todo = vec![start];
        while let Some(n) = todo.pop() {
            if !seen.insert(n) {
                continue;
            }
            let Some(rec) = self.recs.get(&n) else {
                continue;
            };
            for r in rec.runs.iter() {
                let r = r.as_str();
                if let Some(rest) = r.strip_prefix("tests:") {
                    if let Some((fw, dir)) = rest.split_once(':') {
                        out.tests.insert((fw.to_owned(), dir.to_owned()));
                    }
                } else if let Some(rest) = r.strip_prefix("script:") {
                    if let Some((sel, name)) = rest.rsplit_once('#') {
                        todo.extend(self.scripts_of(sel, name));
                    }
                } else if let Some(id) = r.strip_prefix("id:") {
                    todo.extend(self.g.node_by_path(id));
                } else if let Some(reason) = r.strip_prefix("unknown:") {
                    out.unknown.get_or_insert_with(|| reason.to_owned());
                } else if let Some(reason) = r.strip_prefix("computed:") {
                    out.computed.get_or_insert_with(|| reason.to_owned());
                }
            }
            if !rec.id.as_str().starts_with("gha:") && (own_deps || n != start) {
                for d in rec.deps.iter() {
                    todo.extend(self.g.node_by_path(d.as_str()));
                }
            }
        }
        out
    }

    /// Whether the task at `n` launches tests itself: one of its own commands
    /// is a test runner or a test launch that can't be mapped.
    fn launches(&self, n: NodeId) -> bool {
        self.recs.get(&n).is_some_and(|rec| {
            rec.runs.iter().any(|r| {
                let r = r.as_str();
                r.starts_with("tests:") || r.starts_with("unknown:")
            })
        })
    }

    /// The test nodes of `runs`.
    pub(crate) fn tests_of(&mut self, runs: &JobRuns) -> Vec<NodeId> {
        if runs.every_test().is_some() {
            return self.g.tests(None);
        }
        let mut out = Vec::new();
        for (fw, dir) in &runs.tests {
            let g = self.g;
            let all = self
                .tests
                .entry(fw.clone())
                .or_insert_with(|| g.tests(Some(fw)));
            out.extend(all.iter().copied().filter(|&n| under(dir, g.path(n))));
        }
        out
    }
}

/// Computes the workflow effects of `changes` (see the module docs). `git`
/// supplies the base revision's files.
pub fn effects(
    g: &Graph,
    engine: &Engine<'_, View<'_>>,
    root: &Path,
    changes: &[Change],
    git: Option<&Git>,
) -> Effects {
    let mut fx = Effects::default();
    // Workflow jobs in the graph.
    let jobs: Vec<(NodeId, &str, &str)> = g
        .tasks()
        .into_iter()
        .filter_map(|(n, t)| job_of(t.id.as_str()).map(|(wf, job)| (n, wf, job)))
        .collect();
    if jobs.is_empty() || changes.is_empty() {
        return fx;
    }
    let changed: BTreeMap<&str, ChangeKind> =
        changes.iter().map(|c| (c.path.as_str(), c.kind)).collect();
    // Seed jobs: (node, the changed file).
    let mut seeds: BTreeMap<NodeId, String> = BTreeMap::new();

    // 1. Definitions.
    if changes
        .iter()
        .any(|c| def::is_workflow_file(&c.path) || def::is_action_file(&c.path))
    {
        let read_new = |p: &str| std::fs::read_to_string(root.join(p)).ok();
        let read_old = |p: &str| match changed.get(p) {
            Some(ChangeKind::Added) => None,
            Some(_) => git.and_then(|g| g.show(p)),
            None => read_new(p),
        };
        let mut new_defs = Defs::new(&read_new);
        let mut old_defs = Defs::new(&read_old);
        let workflows: BTreeSet<&str> = jobs.iter().map(|(_, wf, _)| *wf).collect();
        let mut definition_files: BTreeSet<String> = BTreeSet::new();
        let (mut compared, mut differing) = (0usize, 0usize);
        for wf in workflows {
            let nodes = |job: Option<&str>| -> Vec<NodeId> {
                jobs.iter()
                    .filter(|(_, w, j)| *w == wf && job.is_none_or(|x| x == *j))
                    .map(|(n, _, _)| *n)
                    .collect()
            };
            let Some(new) = new_defs.workflow(wf) else {
                // Unreadable now: whatever it defined is unknown.
                if changed.contains_key(wf) {
                    definition_files.insert(wf.to_owned());
                    for n in nodes(None) {
                        seeds.entry(n).or_insert_with(|| wf.to_owned());
                    }
                }
                continue;
            };
            // The changed files this workflow's jobs are defined by.
            let touched = |d: &JobDef| -> Option<String> {
                d.files
                    .iter()
                    .find(|f| changed.contains_key(f.as_str()))
                    .cloned()
            };
            if !new.values().any(|d| touched(d).is_some()) {
                continue;
            }
            for d in new.values() {
                definition_files.extend(d.files.iter().cloned());
            }
            let old = git.and_then(|_| old_defs.workflow(wf));
            if let Some(o) = &old {
                for d in o.values() {
                    definition_files.extend(d.files.iter().cloned());
                }
            }
            for (job, d) in &new {
                compared += 1;
                let differs = match (&old, git) {
                    // No base revision: every job defined by a changed file.
                    (_, None) => touched(d).is_some(),
                    (Some(o), Some(_)) => o.get(job).map(|x| &x.value) != Some(&d.value),
                    (None, Some(_)) => true,
                };
                if differs {
                    differing += 1;
                    let why = touched(d).unwrap_or_else(|| wf.to_owned());
                    for n in nodes(Some(job)) {
                        seeds.entry(n).or_insert_with(|| why.clone());
                    }
                }
            }
        }
        for c in changes {
            if c.kind == ChangeKind::Modified
                && (def::is_workflow_file(&c.path) || definition_files.contains(&c.path))
            {
                fx.drop.insert(c.path.clone());
            }
        }
        fx.notes.push(format!(
            "workflows: {} changed definition file(s); {differing} of {compared} job \
             definition(s) in the affected workflows differ{}",
            fx.drop.len(),
            if git.is_some() {
                ""
            } else {
                " (no base revision: every job defined by a changed file)"
            }
        ));
    }

    // 2. Definition files: what a task's commands *are* (the script a step runs, a
    //    file a
    // step names, a test runner's config), recorded as `def:` entries. For a job,
    // also what such a script imports inside the CI tooling area. Application
    // code that a job script imports, or that a build or test command reads, is
    // an input of the job, not a change of how it runs its tests: it is not
    // followed here.
    let rev = engine.reverse();
    let fwd = engine.forward();
    let mut runs = Runs::new(g);
    let job_set: BTreeSet<NodeId> = jobs.iter().map(|(n, _, _)| *n).collect();
    let changed_ix: HashMap<&str, usize> = changes
        .iter()
        .enumerate()
        .map(|(i, c)| (c.path.as_str(), i))
        .collect();
    // A changed file that the definition file `def` imports inside the tooling
    // area.
    let mut memo: HashMap<String, Option<usize>> = HashMap::new();
    let mut tooling_hit = |def: &str| -> Option<usize> {
        if let Some(h) = memo.get(def) {
            return *h;
        }
        let dir = def.rsplit_once('/').map(|(d, _)| format!("{d}/"));
        let inside =
            |p: &str| p.starts_with(".github/") || dir.as_deref().is_some_and(|d| p.starts_with(d));
        let mut hit = None;
        let mut seen: BTreeSet<u32> = BTreeSet::new();
        let mut todo: Vec<u32> = g.node_by_path(def).map(|n| n.0).into_iter().collect();
        while let Some(n) = todo.pop() {
            if !seen.insert(n) || seen.len() > 4096 {
                continue;
            }
            let (succs, attrs) = fwd.neighbors(NodeId(n));
            for (&s, attr) in succs.iter().zip(attrs) {
                // A directory listing or a manifest consulted during resolution is not a
                // reference to the file's contents.
                if matches!(attr.kind(), RefKind::FsReadDir | RefKind::Manifest) {
                    continue;
                }
                let sn = NodeId(s);
                if is_pseudo(g, sn) || g.is_external(sn) || !inside(g.path(sn)) {
                    continue;
                }
                if let Some(&i) = changed_ix.get(g.path(sn)) {
                    hit = Some(i);
                    todo.clear();
                    break;
                }
                todo.push(s);
            }
        }
        memo.insert(def.to_owned(), hit);
        hit
    };
    // Package scripts (and other tasks) whose own definition file changed and that
    // launch tests themselves: the task and the changed file.
    let mut launchers: BTreeMap<NodeId, String> = BTreeMap::new();
    let mut nodes: Vec<NodeId> = runs.recs.keys().copied().collect();
    nodes.sort_unstable();
    for n in nodes {
        let is_job = job_set.contains(&n);
        if is_job && seeds.contains_key(&n) {
            continue;
        }
        let rec = runs.recs[&n];
        let defs = rec
            .runs
            .iter()
            .filter_map(|r| r.as_str().strip_prefix("def:"));
        let mut hit = None;
        for d in defs {
            hit = match changed_ix.get(d) {
                Some(&i) => Some(i),
                None if is_job => tooling_hit(d),
                None => None,
            };
            if hit.is_some() {
                break;
            }
        }
        let Some(i) = hit else { continue };
        if is_job {
            seeds.insert(n, changes[i].path.clone());
        } else if runs.launches(n) {
            launchers.insert(n, changes[i].path.clone());
        }
    }
    // Entries that read a dropped file through explicit references (file-to-file
    // edges).
    if !fx.drop.is_empty() {
        // node → predecessor on the way from a dropped file (itself for the file).
        let mut reach: HashMap<u32, u32> = HashMap::new();
        let mut queue: std::collections::VecDeque<u32> = Default::default();
        for c in changes.iter().filter(|c| fx.drop.contains(&c.path)) {
            if let Some(n) = g.node_by_path(&c.path)
                && !is_pseudo(g, n)
                && !reach.contains_key(&n.0)
            {
                reach.insert(n.0, n.0);
                queue.push_back(n.0);
            }
        }
        while let Some(n) = queue.pop_front() {
            let (preds, attrs) = rev.neighbors(NodeId(n));
            for (&p, attr) in preds.iter().zip(attrs) {
                let pn = NodeId(p);
                if matches!(attr.kind(), RefKind::FsReadDir | RefKind::Manifest) {
                    continue;
                }
                if is_pseudo(g, pn) || g.is_external(pn) || reach.contains_key(&p) {
                    continue;
                }
                reach.insert(p, n);
                queue.push_back(p);
            }
        }
        for &n in reach.keys() {
            let mut chain = vec![NodeId(n)];
            let mut at = n;
            while let Some(&prev) = reach.get(&at) {
                if prev == at {
                    break;
                }
                chain.push(NodeId(prev));
                at = prev;
            }
            chain.reverse();
            fx.readers.insert(NodeId(n), chain);
        }
    }

    // 3. Jobs that need or call a changed job.
    let mut todo: Vec<NodeId> = seeds.keys().copied().collect();
    fx.jobs = seeds;
    while let Some(k) = todo.pop() {
        let why = fx.jobs[&k].clone();
        for &p in rev.neighbors(k).0 {
            let pn = NodeId(p);
            if job_set.contains(&pn) && !fx.jobs.contains_key(&pn) {
                fx.jobs.insert(pn, why.clone());
                todo.push(pn);
            }
        }
    }
    if fx.jobs.is_empty() && launchers.is_empty() {
        return fx;
    }

    // 4. The tests the changed jobs (and changed test-launching scripts) run.
    let mut unmapped: Vec<String> = Vec::new();
    let changed_tasks: Vec<(NodeId, String)> = fx
        .jobs
        .iter()
        .chain(launchers.iter())
        .map(|(n, p)| (*n, p.clone()))
        .collect();
    for (j, path) in changed_tasks {
        // A job runs its steps and everything they start in the environment it defines.
        // A package script's definition files are those of its own command.
        let r = if job_set.contains(&j) {
            runs.of(j)
        } else {
            runs.of_command(j)
        };
        if let Some(reason) = r.every_test()
            && unmapped.len() < 3
        {
            unmapped.push(format!("{} ({reason})", g.path(j)));
        }
        for t in runs.tests_of(&r) {
            fx.tests.entry(t).or_insert_with(|| (j, path.clone()));
        }
    }
    fx.notes.push(format!(
        "workflows: {} changed job(s){} run {} test(s){}",
        fx.jobs.len(),
        if launchers.is_empty() {
            String::new()
        } else {
            format!(" and {} changed test-launching script(s)", launchers.len())
        },
        fx.tests.len(),
        if unmapped.is_empty() {
            String::new()
        } else {
            format!(
                "; every test, because of launches that can't be mapped: {}",
                unmapped.join("; ")
            )
        }
    ));
    fx
}

impl Effects {
    /// Adds the verdicts to `report`: among `entries`, the changed jobs, the
    /// tests they run and the readers of dropped files are affected.
    pub fn apply(&self, g: &Graph, entries: &NodeSet, report: &mut AffectedReport) {
        if self.jobs.is_empty() && self.tests.is_empty() && self.readers.is_empty() {
            return;
        }
        let mut at: HashMap<NodeId, usize> = HashMap::new();
        for (i, e) in report.entries.iter().enumerate() {
            at.insert(e.entry, i);
        }
        let mut add = |entry: NodeId, witness: Vec<NodeId>, path: &str| {
            if !entries.contains(entry) {
                return;
            }
            let result = EntryResult {
                entry,
                verdict: Verdict::Affected,
                witness,
                cause: Some(Cause::Modified {
                    path: path.to_owned(),
                }),
                uncertain: None,
            };
            match at.get(&entry) {
                // A change is a stronger reason than uncertainty.
                Some(&i) if report.entries[i].cause.is_none() => report.entries[i] = result,
                Some(_) => {}
                None => {
                    at.insert(entry, report.entries.len());
                    report.entries.push(result);
                }
            }
        };
        for (&j, path) in &self.jobs {
            add(j, vec![j], path);
        }
        for (&t, (j, path)) in &self.tests {
            add(t, vec![*j, t], path);
        }
        for (&n, chain) in &self.readers {
            if let Some(first) = chain.first() {
                let path = g.path(*first).to_owned();
                add(n, chain.clone(), &path);
            }
        }
        report
            .entries
            .sort_by(|a, b| g.path(a.entry).cmp(g.path(b.entry)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(
            job_of("gha:.github/workflows/ci.yml#test"),
            Some((".github/workflows/ci.yml", "test"))
        );
        assert_eq!(job_of("task:apps/web#build"), None);
        assert!(under(".", "a/b.test.ts"));
        assert!(under("apps/web", "apps/web/x.test.ts"));
        assert!(under("apps/web/x.test.ts", "apps/web/x.test.ts"));
        assert!(!under("apps/we", "apps/web/x.test.ts"));
    }
}
