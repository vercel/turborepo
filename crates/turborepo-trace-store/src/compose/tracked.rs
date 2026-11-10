//! Tracked outputs: files a task writes that are committed to the repository.
//!
//! Generated-file linking makes every node that an output of a task covers
//! depend on the task: whoever uses the file is affected by the task's inputs,
//! and inherits the open uncertainty of the code the task runs. For a file that
//! is not in the checkout (a gitignored `dist/`, generated types) that is the
//! only way to know what the file will contain. For a **committed** file it is
//! only true if something regenerates the file before the tests run. A
//! generator that a developer runs by hand, and whose output is checked in,
//! does not run in CI: the tests read the committed bytes. An edit of the
//! generator, or of anything it reads, changes nothing for them until the
//! output is regenerated, and that is a change of the committed file itself.
//!
//! So an existing, walked (tracked and not ignored) file that an output of a
//! package script covers is not linked to the script when **nothing in the
//! repository's automation runs it**. That can only be said of a repository
//! whose automation is readable: one with a CI job (a GitHub Actions workflow
//! job) that runs tests or tasks. Without one, the tests run somewhere this
//! model cannot see (another CI system, a dashboard setting), what runs
//! before them is unknown, and every link stays.
//!
//! What the automation runs is everything reachable from its starting points:
//!
//! - starting points: CI jobs and every other task that is not a package
//!   script, install lifecycle scripts (`postinstall`, `prepare`, …: the
//!   package manager runs them by itself), and every task that runs tests
//!   (`"test": "node gen.js && jest"` runs the generator on every test run;
//!   launches found in opaque text count);
//! - from a task to the tasks it depends on or runs (turbo `dependsOn` and
//!   `^task`, a `pre`/`post` hook, `pnpm gen && …`, `turbo run gen`, a workflow
//!   step, a Makefile prerequisite), to every script of a name its runs
//!   relation names ([`crate::shard::TaskRec::runs`]; in any package), and,
//!   where it starts a task whose name is computed (`pnpm run "$TASK"`), to
//!   every script of its package (of the repository, for a root script or a CI
//!   job).
//!
//! A script only reached from scripts that nothing runs (a `dev` launcher that
//! takes the task as an argument) is itself run by hand.
//!
//! Files of the output that are not in the checkout keep their links (probes,
//! listings): somebody has to run the task for them to exist. One kind of probe
//! does not count: a candidate path that a reference looked for inside the
//! output before it resolved to an existing file (`./types/entry` tries
//! `entry.js` before it finds `entry.ts`), or that the analysis of a source
//! file looked up without needing it (a `package.json` or `tsconfig.json` next
//! to the file). The file works without it, and a task nothing runs
//! will not create it; if a commit adds it, the probe matches the added path as
//! for any other file. Parts (`tsc -b` projects) are not tasks and keep their
//! links.
//!
//! The assumption is the one of the runs relation: the repository's automation
//! starts a task by naming it where the task model can see it. A generator that
//! CI regenerates through a path the model does not read (an action of another
//! repository, a second CI system next to the workflows) would need the link;
//! the committed file would then usually be checked for drift by that same job.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use turborepo_trace_graph::{EdgeAttr, RefKind};

use super::{Graph, NONE};
use crate::{
    normalize::expand_probe,
    paths::join_repo,
    shard::{ArchivedTargetRec, FILE_LEVEL},
};

/// Scripts the package manager runs by itself.
const LIFECYCLE: &[&str] = &[
    "preinstall",
    "install",
    "postinstall",
    "prepare",
    "prepublish",
    "prepublishOnly",
    "prepack",
    "postpack",
];

/// Id prefix of CI jobs the task model reads (GitHub Actions workflow jobs).
const CI_JOB: &str = "gha:";

/// Whether a task id names a package script (`task:<package>#<name>`), as
/// opposed to a CI job, a container build or a Makefile target.
fn is_script(id: &str) -> bool {
    id.starts_with("task:")
}

/// The package directory of a script task id (`task:<package>#<name>`; `.` for
/// the root).
fn package_of(id: &str) -> &str {
    let rest = id.strip_prefix("task:").unwrap_or(id);
    rest.rsplit_once('#').map_or(rest, |x| x.0)
}

/// The package scripts nothing in the repository's automation runs, and the
/// snapshot's walked files.
pub(super) struct Tracked {
    /// Task nodes of scripts with outputs that nothing runs (see the module
    /// docs).
    manual: HashSet<u32>,
    /// The walked files (repo-relative, sorted); empty when no task is manual.
    walked: Vec<String>,
}

impl Tracked {
    /// `deps`: the task dependency edges (task node, dependency node). `out`:
    /// the graph directory (for the walked-file list).
    pub(super) fn new(g: &Graph, out: &Path, deps: &[(u32, u32)]) -> Tracked {
        // Every task: node and record.
        let mut tasks = Vec::new();
        for (si, s) in g.shards.iter().enumerate() {
            for t in s.get().tasks.iter() {
                if let Some(n) = g.file_node(si, t.file.to_native())
                    && g.is_task(n)
                {
                    tasks.push((n.0, t));
                }
            }
        }
        // Scripts by name and by package.
        let mut by_name: HashMap<&str, Vec<u32>> = HashMap::new();
        let mut by_package: HashMap<&str, Vec<u32>> = HashMap::new();
        let mut rec_of: HashMap<u32, usize> = HashMap::new();
        for (i, (n, t)) in tasks.iter().enumerate() {
            rec_of.insert(*n, i);
            if is_script(t.id.as_str()) {
                by_name.entry(t.name.as_str()).or_default().push(*n);
                by_package
                    .entry(package_of(t.id.as_str()))
                    .or_default()
                    .push(*n);
            }
        }
        let mut dep_of: HashMap<u32, Vec<u32>> = HashMap::new();
        for &(t, d) in deps {
            dep_of.entry(t).or_default().push(d);
        }
        // Is the automation readable: does a CI job run tests or tasks?
        let readable = tasks.iter().any(|(_, t)| {
            t.id.as_str().starts_with(CI_JOB) && t.runs.iter().any(|r| !r.starts_with("def:"))
        });
        if !readable {
            return Tracked {
                manual: HashSet::new(),
                walked: Vec::new(),
            };
        }
        // The starting points, then everything they depend on or run.
        let mut run: HashSet<u32> = HashSet::new();
        let mut todo: Vec<u32> = Vec::new();
        for (n, t) in &tasks {
            let start = !is_script(t.id.as_str())
                || LIFECYCLE.contains(&t.name.as_str())
                || t.runs
                    .iter()
                    .any(|r| r.starts_with("tests:") || r.starts_with("unknown:"));
            if start && run.insert(*n) {
                todo.push(*n);
            }
        }
        while let Some(n) = todo.pop() {
            let mut next: Vec<u32> = dep_of.get(&n).cloned().unwrap_or_default();
            let t = tasks[rec_of[&n]].1;
            let id = t.id.as_str();
            for r in t.runs.iter() {
                let r = r.as_str();
                if r.starts_with("computed:") {
                    // A task name chosen at run time: any script of the package, or of
                    // the repository for a root script or a CI job.
                    if is_script(id) && package_of(id) != "." {
                        next.extend(by_package.get(package_of(id)).into_iter().flatten());
                    } else {
                        next.extend(by_package.values().flatten());
                    }
                } else if let Some(id) = r.strip_prefix("id:") {
                    next.extend(g.lookup(id));
                } else if let Some((_, name)) = r
                    .strip_prefix("script:")
                    .and_then(|rest| rest.rsplit_once('#'))
                {
                    // Whatever the package: a superset.
                    next.extend(by_name.get(name).into_iter().flatten());
                }
            }
            for m in next {
                if rec_of.contains_key(&m) && run.insert(m) {
                    todo.push(m);
                }
            }
        }
        let manual: HashSet<u32> = tasks
            .iter()
            .filter(|(n, t)| is_script(t.id.as_str()) && !t.outputs.is_empty() && !run.contains(n))
            .map(|(n, _)| *n)
            .collect();
        let walked = if manual.is_empty() {
            Vec::new()
        } else {
            g.manifest
                .files
                .as_deref()
                .and_then(|n| crate::manifest::read_files(&out.join("store"), n))
                .unwrap_or_default()
        };
        Tracked { manual, walked }
    }

    /// Whether `path`, covered by an output of the task node `task`, is a
    /// committed file that nothing regenerates: it is not linked to the
    /// task.
    pub(super) fn committed(&self, task: u32, path: &str) -> bool {
        self.manual.contains(&task)
            && self
                .walked
                .binary_search_by(|w| w.as_str().cmp(path))
                .is_ok()
    }

    /// Whether node `n` only looked for the missing path `path`, covered by an
    /// output of the task node `task`, without needing it: `task` is one
    /// that nothing runs, and every reference of `n` that probed `path`
    /// resolved to a concrete target (or is a lookup of the file's
    /// analysis). Such a probe does not link `n` to the task.
    pub(super) fn passing_probe(&self, g: &Graph, task: u32, n: u32, path: &str) -> bool {
        if !self.manual.contains(&task) {
            return false;
        }
        let node = g.nodes[n as usize];
        if node.shard == NONE {
            return false;
        }
        let a = g.shards[node.shard as usize].get();
        let Some(file) = a.files.get(node.file as usize) else {
            return false;
        };
        let root = a.package.root.as_str();
        let mut scratch = Vec::new();
        let mut any = false;
        for r in file.refs.iter() {
            let looks = r.probes.iter().any(|pi| {
                scratch.clear();
                expand_probe(&a.probes[pi.to_native() as usize], &mut scratch);
                scratch
                    .iter()
                    .any(|rel| join_repo(root, rel).as_deref() == Some(path))
            });
            if !looks {
                continue;
            }
            // The file-level group of a source file holds lookups that are not requirements
            // (the nearest manifest, a tsconfig): the file is what it is without them. That
            // of a task or part holds its missing inputs: kept.
            let resolved = if r.index.to_native() == FILE_LEVEL {
                !g.is_pseudo(n as usize)
            } else {
                r.edges.iter().any(|e| {
                    EdgeAttr(e.attr.to_native()).kind() != RefKind::Manifest
                        && !matches!(e.target, ArchivedTargetRec::None)
                })
            };
            if !resolved {
                return false;
            }
            any = true;
        }
        any
    }
}
