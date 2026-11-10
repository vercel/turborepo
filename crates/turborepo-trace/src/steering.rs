//! Repo steering files: files that steer the repository's process (review
//! ownership, CI definitions, bots, editor and git settings) and can never
//! change what an application does.
//!
//! Such a file sits inside almost every "whole directory" input: a task's
//! fallback membership (`members:<pkg>/**`), turbo's default inputs, the scope
//! of a computed read (`<dir>/**`). Treated as an ordinary change, an edit to
//! `packages/a/.vercel.approvers` reaches the package's tasks and, through
//! their outputs, tests. So a changed steering file is taken out of the change
//! set and affects an entry only in these ways ([`seed_plan`]):
//!
//! - **explicit references to the file**: every node with a resolved edge to it
//!   (an import, a `require`, a read or asset edge, a manifest edge), and
//!   transitively the nodes that reference those, through ordinary file-to-file
//!   edges. A CI-tooling test that imports `.github/scripts/x.mjs` or reads a
//!   workflow file by a resolved path is selected;
//! - **implicit references that name steering paths**: a probe, a directory
//!   membership or a covered-uncertainty scope recorded on a file counts when
//!   the file is itself a steering file, or when the probed path or listed
//!   directory is a steering path (a tooling test that globs
//!   `.github/workflows/*.yml`, the importer of a deleted
//!   `.github/scripts/x.mjs`). A config input (a file whose contents a
//!   resolution read) always names the file. Whole-package memberships and
//!   scopes of application directories never count;
//! - **tasks are terminal**: a task that references the file directly (its
//!   definition, as a `gha:<workflow>#<job>` task and its workflow file, or a
//!   declared input) or runs an affected file is reported when it is an entry,
//!   and nothing flows on from it. Membership hubs are never entered, so a
//!   task's whole-directory input does not see the file.
//!
//! When every change is a steering file, the uncertainty floor
//! (`--on-uncertain`) applies only to entries that reach an open uncertainty
//! located in a steering file: an unknown reference in application code cannot
//! be about a file that no application reads.
//!
//! `--steering <GLOB>` adds globs (relative to the graph root); `--no-steering`
//! turns the treatment off.

use std::collections::{HashMap, VecDeque};

use turborepo_trace_graph::{
    AffectedOptions, AffectedReport, Cause, Change, ChangeKind, EdgeFilter, Engine, EntryResult,
    GraphView, NodeId, NodeSet, Policy, UncertainSite, Verdict, glob::Glob,
};
use turborepo_trace_store::Graph;

/// The graph the engine runs on: the store's graph with its derived layer
/// (per-test working directories, hints). Node ids are those of the base graph.
type View<'g> = turborepo_trace_graph::Patched<'g, Graph>;

/// The default steering globs, relative to the graph root. Deliberately tight:
/// nothing a build or a test can read (no documentation, no `.gitignore`, no
/// lockfiles).
pub const DEFAULT_GLOBS: &[&str] = &[
    ".github/**",
    "CODEOWNERS",
    "docs/CODEOWNERS",
    "**/.vercel.approvers",
    ".gitattributes",
    ".git-blame-ignore-revs",
    ".editorconfig",
    ".mergify.yml",
    "renovate.json",
    ".renovaterc*",
];

/// The steering globs of one run.
pub struct Steering {
    globs: Vec<Glob>,
}

impl Steering {
    /// The defaults plus `extra`; no globs at all when not `enabled`.
    pub fn new(extra: &[String], enabled: bool) -> anyhow::Result<Steering> {
        let mut globs = Vec::new();
        if enabled {
            for p in DEFAULT_GLOBS
                .iter()
                .copied()
                .chain(extra.iter().map(String::as_str))
            {
                let p = p.trim_start_matches("./");
                globs.push(Glob::new(p).map_err(|e| anyhow::anyhow!("--steering {p:?}: {e}"))?);
            }
        }
        Ok(Steering { globs })
    }

    /// Whether root-relative `path` is a steering file.
    pub fn is_steering(&self, path: &str) -> bool {
        self.globs.iter().any(|g| g.matches(path))
    }

    /// Whether the directory `dir` holds only steering files
    /// (`.github/workflows`).
    pub fn is_steering_dir(&self, dir: &str) -> bool {
        !dir.is_empty() && (self.is_steering(dir) || self.is_steering(&format!("{dir}/_")))
    }

    /// The steering files among `changes`.
    pub fn of(&self, changes: &[Change]) -> Vec<Change> {
        if self.globs.is_empty() {
            return Vec::new();
        }
        changes
            .iter()
            .filter(|c| self.is_steering(&c.path))
            .cloned()
            .collect()
    }
}

/// The directory a membership or scope `(dir, pattern)` names: `dir` plus the
/// leading literal directories of `pattern` (`("", ".github/workflows/*.yml")`
/// → `.github/workflows`).
pub fn named_dir(dir: &str, pattern: &str) -> String {
    let mut out = dir.trim_end_matches('/').to_owned();
    let mut segs = pattern.split('/').peekable();
    while let Some(s) = segs.next() {
        // The last segment names files, not a directory.
        if segs.peek().is_none() || turborepo_trace_graph::glob::is_glob(s) || s == ".." {
            break;
        }
        if s.is_empty() || s == "." {
            continue;
        }
        if !out.is_empty() {
            out.push('/');
        }
        out.push_str(s);
    }
    out
}

/// How a changed steering file is applied: the nodes it seeds and what the
/// propagation from them may follow ([`seed_plan`]).
pub struct SeedPlan {
    /// The nodes the change seeds, each with its cause. A hub is seeded only
    /// when [`Follow::through_hubs`] is set.
    pub seeds: Vec<(NodeId, Cause)>,
    /// What the reverse propagation from the seeds follows.
    pub follow: Follow,
}

/// What the propagation from a steering change's seeds may follow.
#[derive(Copy, Clone, Debug)]
pub struct Follow {
    /// The edges it may cross (in addition to the run's own traversal filter,
    /// such as `--runtime-only`).
    pub edges: EdgeFilter,
    /// Whether it continues from a task node to the nodes that depend on the
    /// task (its dependents and the consumers of its outputs). When
    /// `false`, a reached task is reported and is terminal.
    pub through_tasks: bool,
    /// Whether it enters membership hubs (`members:<dir>/<pattern>`), and so
    /// the tasks whose whole-directory input contains a reached file.
    pub through_hubs: bool,
}

impl Follow {
    /// File-to-file edges only: tasks are terminal, hubs are not entered.
    pub const FILES: Follow = Follow {
        edges: EdgeFilter::ALL,
        through_tasks: false,
        through_hubs: false,
    };
}

/// Whether `path` defines CI jobs: a workflow (`.github/workflows/**`) or a
/// local action (`action.yml` / `action.yaml` below `.github/`). [`seed_plan`]
/// treats these separately so that a field-level reading of workflows can
/// replace the rule for them.
pub fn is_workflow_definition(path: &str) -> bool {
    let Some(rest) = path.strip_prefix(".github/") else {
        return false;
    };
    rest.starts_with("workflows/")
        || matches!(rest.rsplit('/').next(), Some("action.yml" | "action.yaml"))
}

/// The decision for one changed steering file: which nodes it seeds and which
/// edges the propagation from them may follow (see the module docs).
///
/// This is the single place that decides it. Workflow and action definitions
/// ([`is_workflow_definition`]) go through [`workflow_seed_plan`]: which jobs
/// such a file changes, and which tests those jobs run, is decided field by
/// field in [`crate::workflows`], not by the file as a whole.
pub fn seed_plan(
    g: &Graph,
    engine: &Engine<'_, View<'_>>,
    st: &Steering,
    change: &Change,
) -> SeedPlan {
    if is_workflow_definition(&change.path) {
        return workflow_seed_plan(g, engine, st, change);
    }
    default_seed_plan(g, engine, st, change)
}

/// [`seed_plan`] for workflow and local action definitions: the explicit
/// references to the file, as for every steering file, but no task is seeded
/// through its config input. Every job of a workflow reads the whole file, and
/// so does every task of the root package (through task discovery); the jobs
/// whose *definition* changed, and the tests they run,
/// come from [`crate::workflows::effects`].
pub fn workflow_seed_plan(
    g: &Graph,
    engine: &Engine<'_, View<'_>>,
    st: &Steering,
    change: &Change,
) -> SeedPlan {
    let mut plan = default_seed_plan(g, engine, st, change);
    plan.seeds.retain(|(n, _)| !g.is_task(*n));
    plan
}

/// [`seed_plan`] for every other steering file.
pub fn default_seed_plan(
    g: &Graph,
    engine: &Engine<'_, View<'_>>,
    st: &Steering,
    c: &Change,
) -> SeedPlan {
    let own = || match c.kind {
        ChangeKind::Deleted => Cause::Deleted {
            path: c.path.clone(),
        },
        _ => Cause::Modified {
            path: c.path.clone(),
        },
    };
    let mut seeds: Vec<(NodeId, Cause)> = Vec::new();
    // The file itself: its node is the start of the explicit references to it (and
    // a steering file can be an entry, such as a test of CI tooling).
    if let Some(n) = g.node_by_path(&c.path) {
        seeds.push((n, own()));
    }
    // Config inputs name the file.
    engine.match_inputs(&c.path, &mut |n| seeds.push((n, own())));
    if matches!(c.kind, ChangeKind::Added | ChangeKind::Deleted) {
        // A probe counts on a task or a steering file, or when the probed path is a
        // steering path.
        engine.match_probes(&c.path, &mut |n, probe| {
            if g.is_task(n) || st.is_steering(g.path(n)) || st.is_steering(probe) {
                seeds.push((
                    n,
                    match c.kind {
                        ChangeKind::Added => Cause::Probe {
                            added: c.path.clone(),
                            probe: probe.to_owned(),
                        },
                        _ => own(),
                    },
                ));
            }
        });
        // A membership counts on a steering file, or when it lists a steering
        // directory.
        engine.match_memberships(&c.path, &mut |n, dir, pattern| {
            if st.is_steering(g.path(n)) || st.is_steering_dir(&named_dir(dir, pattern)) {
                seeds.push((
                    n,
                    Cause::Membership {
                        path: c.path.clone(),
                        kind: c.kind,
                        dir: dir.to_owned(),
                        pattern: pattern.to_owned(),
                    },
                ));
            }
        });
    }
    // So does the scope of a covered uncertainty.
    engine.match_scopes(&c.path, &mut |n, dir, pattern| {
        if st.is_steering(g.path(n)) || st.is_steering_dir(&named_dir(dir, pattern)) {
            seeds.push((
                n,
                Cause::Scope {
                    path: c.path.clone(),
                    kind: c.kind,
                    dir: dir.to_owned(),
                    pattern: pattern.to_owned(),
                },
            ));
        }
    });
    SeedPlan {
        seeds,
        follow: Follow::FILES,
    }
}

/// What the steering changes reach.
#[derive(Default)]
pub struct Reach {
    /// Reached nodes: node → the node it was reached from (itself for a seed).
    parent: HashMap<u32, u32>,
    /// Seeds with their cause.
    cause: HashMap<u32, Cause>,
}

impl Reach {
    /// Number of nodes reached.
    pub fn len(&self) -> usize {
        self.parent.len()
    }

    /// Whether nothing was reached.
    pub fn is_empty(&self) -> bool {
        self.parent.is_empty()
    }

    fn chain(&self, n: NodeId) -> Vec<NodeId> {
        let mut out = vec![n];
        let mut cur = n.0;
        while let Some(&p) = self.parent.get(&cur) {
            if p == cur {
                break;
            }
            out.push(NodeId(p));
            cur = p;
        }
        out.reverse();
        out
    }

    /// Adds a walk's nodes. A node reached before keeps its chain; a new node's
    /// chain may continue into an earlier one (every step is still an
    /// edge).
    fn absorb(&mut self, parent: HashMap<u32, u32>, causes: Vec<(NodeId, Cause)>) {
        for (&n, &p) in &parent {
            self.parent.entry(n).or_insert(p);
        }
        for (n, c) in causes {
            if self.parent.get(&n.0) == Some(&n.0) {
                self.cause.entry(n.0).or_insert(c);
            }
        }
    }
}

struct Walk<'a, 'g> {
    g: &'g Graph,
    engine: &'a Engine<'g, View<'g>>,
    follow: Follow,
    parent: HashMap<u32, u32>,
    queue: VecDeque<u32>,
}

impl<'a, 'g> Walk<'a, 'g> {
    fn new(engine: &'a Engine<'g, View<'g>>, follow: Follow) -> Self {
        Walk {
            g: engine.graph().base(),
            engine,
            follow,
            parent: HashMap::new(),
            queue: VecDeque::default(),
        }
    }

    fn is_hub(&self, n: NodeId) -> bool {
        self.g.lang(n) == Some(turborepo_trace_store::tasks::MEMBERS_LANG)
    }

    fn enter(&mut self, n: NodeId, from: u32) -> bool {
        if (self.is_hub(n) && !self.follow.through_hubs) || self.parent.contains_key(&n.0) {
            return false;
        }
        self.parent.insert(n.0, from);
        // A task is reported; whether anything flows on from it is the plan's choice.
        if self.follow.through_tasks || !self.g.is_task(n) {
            self.queue.push_back(n.0);
        }
        true
    }

    /// Seeds `n`; `false` if it is a hub the plan does not enter, or already
    /// reached.
    fn seed(&mut self, n: NodeId) -> bool {
        self.enter(n, n.0)
    }

    /// The reverse closure of the seeds through the edges the plan follows.
    fn run(&mut self) {
        let rev = self.engine.reverse();
        let filter = self.engine.options().traversal;
        while let Some(n) = self.queue.pop_front() {
            let (srcs, attrs) = rev.neighbors(NodeId(n));
            for (&s, &a) in srcs.iter().zip(attrs) {
                if filter.accepts(a) && self.follow.edges.accepts(a) {
                    self.enter(NodeId(s), n);
                }
            }
        }
    }
}

/// The nodes the steering `changes` affect: per change, the reverse closure of
/// its [`seed_plan`].
pub fn reach(engine: &Engine<'_, View<'_>>, st: &Steering, changes: &[Change]) -> Reach {
    let g = engine.graph();
    let mut out = Reach::default();
    for c in changes {
        let plan = seed_plan(g.base(), engine, st, c);
        if plan.seeds.is_empty() {
            continue;
        }
        let mut w = Walk::new(engine, plan.follow);
        let mut causes = Vec::new();
        for (n, why) in plan.seeds {
            if w.seed(n) {
                causes.push((n, why));
            }
        }
        w.run();
        out.absorb(w.parent, causes);
    }
    out
}

/// Adds the entries `reach` contains to `report` as affected, with their
/// witness chains. Entries the engine already reports as affected by another
/// change keep that result.
pub fn merge(g: &Graph, reach: &Reach, entries: &NodeSet, report: &mut AffectedReport) {
    if reach.is_empty() {
        return;
    }
    let mut at: HashMap<u32, usize> = report
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| (e.entry.0, i))
        .collect();
    let mut hit: Vec<u32> = reach
        .parent
        .keys()
        .copied()
        .filter(|&n| entries.contains(NodeId(n)))
        .collect();
    hit.sort_unstable();
    for n in hit {
        let witness = reach.chain(NodeId(n));
        let result = EntryResult {
            entry: NodeId(n),
            verdict: Verdict::Affected,
            cause: reach.cause.get(&witness[0].0).cloned(),
            witness,
            uncertain: None,
        };
        match at.get(&n) {
            // Affected by another change already.
            Some(&i) if report.entries[i].cause.is_some() => {}
            Some(&i) => report.entries[i] = result,
            None => {
                at.insert(n, report.entries.len());
                report.entries.push(result);
            }
        }
    }
    report.seed_count += reach.cause.len();
    report.reached_count += reach.parent.len();
    report
        .entries
        .sort_by(|a, b| g.path(a.entry).cmp(g.path(b.entry)));
}

/// The uncertainty floor of a run whose changes are all steering files: entries
/// that reach (through file-to-file edges) an open uncertainty recorded on a
/// steering file get the policy's verdict, unless they are affected already.
pub fn floor(
    engine: &Engine<'_, View<'_>>,
    st: &Steering,
    opts: &AffectedOptions,
    entries: &NodeSet,
    report: &mut AffectedReport,
) {
    let g = engine.graph();
    let verdict = match opts.policy {
        Policy::Affected => Verdict::Affected,
        Policy::Unknown => Verdict::Unknown,
        Policy::Ignore => return,
    };
    let mut w = Walk::new(engine, Follow::FILES);
    for i in 0..g.node_count() {
        let n = NodeId(i);
        if !g.is_external(n)
            && g.has_uncertainty(n)
            && st.is_steering(g.path(n))
            && g.uncertainties(n).any(|u| opts.triggers(&u, g.lang(n)))
        {
            w.seed(n);
        }
    }
    if w.parent.is_empty() {
        return;
    }
    w.run();
    let sites = Reach {
        parent: w.parent,
        cause: HashMap::new(),
    };
    let have: std::collections::HashSet<u32> = report.entries.iter().map(|e| e.entry.0).collect();
    let mut hit: Vec<u32> = sites
        .parent
        .keys()
        .copied()
        .filter(|&n| entries.contains(NodeId(n)) && !have.contains(&n))
        .collect();
    hit.sort_unstable();
    for n in hit {
        let witness = sites.chain(NodeId(n));
        let site = witness[0];
        let reasons = g
            .uncertainties(site)
            .filter(|u| opts.triggers(u, g.lang(site)))
            .map(|u| (u.kind, u.line, u.detail.to_owned()))
            .collect();
        report.entries.push(EntryResult {
            entry: NodeId(n),
            verdict,
            witness,
            cause: None,
            uncertain: Some(UncertainSite {
                node: site,
                reasons,
                bound: None,
            }),
        });
    }
    report
        .entries
        .sort_by(|a, b| g.path(a.entry).cmp(g.path(b.entry)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_set() {
        let st = Steering::new(&[], true).unwrap();
        for p in [
            ".github/workflows/ci.yml",
            ".github/scripts/x.test.mjs",
            ".github/CODEOWNERS",
            ".github/dependabot.yml",
            "CODEOWNERS",
            "docs/CODEOWNERS",
            ".vercel.approvers",
            "packages/a/.vercel.approvers",
            "packages/a/src/deep/.vercel.approvers",
            ".gitattributes",
            ".git-blame-ignore-revs",
            ".editorconfig",
            ".mergify.yml",
            "renovate.json",
            ".renovaterc",
            ".renovaterc.json5",
        ] {
            assert!(st.is_steering(p), "{p}");
        }
        for p in [
            "README.md",
            "docs/README.md",
            ".gitignore",
            "pnpm-lock.yaml",
            "package.json",
            "packages/a/CODEOWNERS",
            "packages/a/.gitattributes",
            "packages/a/renovate.json",
            "src/.github.ts",
            "github/workflows/ci.yml",
            "turbo.json",
        ] {
            assert!(!st.is_steering(p), "{p}");
        }
        assert!(st.is_steering_dir(".github"));
        assert!(st.is_steering_dir(".github/workflows"));
        assert!(!st.is_steering_dir(""));
        assert!(!st.is_steering_dir("packages/a"));
    }

    #[test]
    fn extra_globs_and_off() {
        let st = Steering::new(&["./tools/ci/**".into(), "OWNERS".into()], true).unwrap();
        assert!(st.is_steering("tools/ci/check.mjs"));
        assert!(st.is_steering("OWNERS"));
        assert!(st.is_steering_dir("tools/ci"));
        let off = Steering::new(&["tools/ci/**".into()], false).unwrap();
        assert!(!off.is_steering(".github/workflows/ci.yml"));
        assert!(!off.is_steering("tools/ci/check.mjs"));
        assert!(Steering::new(&["a/{b".into()], true).is_err());
    }

    #[test]
    fn named_dirs_and_workflow_definitions() {
        assert_eq!(named_dir(".github/workflows", "*.yml"), ".github/workflows");
        assert_eq!(
            named_dir("", ".github/workflows/*.yml"),
            ".github/workflows"
        );
        assert_eq!(named_dir("", "**"), "");
        assert_eq!(named_dir("packages/a", "**"), "packages/a");
        assert_eq!(named_dir(".github", "scripts/**/*.mjs"), ".github/scripts");
        assert_eq!(named_dir("a", "x.json"), "a");
        assert!(is_workflow_definition(".github/workflows/ci.yml"));
        assert!(is_workflow_definition(".github/actions/setup/action.yml"));
        assert!(is_workflow_definition(".github/actions/setup/action.yaml"));
        assert!(!is_workflow_definition(".github/scripts/x.mjs"));
        assert!(!is_workflow_definition(".github/CODEOWNERS"));
        assert!(!is_workflow_definition("packages/a/action.yml"));
    }
}
