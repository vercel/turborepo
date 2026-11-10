//! The entry set of `--tasks <glob>`: which of the matching build tasks are
//! reported.
//!
//! `affected --tasks` answers "what has to be built, checked and tested again".
//! A package has scripts that are not part of that: a dev server, a watch mode,
//! the `--fix` variant of a check. They depend on the same files as their
//! counterparts (the fix variant runs the check), so every change that selects
//! the check selected them too, and on a large repository they are a fifth of
//! the report.
//!
//! By default a glob therefore leaves out the tasks that are **run by hand by
//! their nature**: the name has a `dev`, `start`, `serve`, `preview`, `watch`
//! or `fix` segment (`dev:ts`, `unit-test-watch`, `clean-code:fix`). A task
//! that the repository's automation runs stays: a CI job or an install
//! lifecycle script reaches it through task dependencies or by naming it (`pnpm
//! format:fix` in a workflow step). A pattern that names the task (no wildcard
//! at all, or none in the name after `#`: `**#dev`) always gets it, and
//! `--all-tasks` switches the rule off.
//!
//! Command-line flags are not read: `"lint": "eslint --fix ."` and
//! `"format": "prettier --write ."` are often a package's only lint and format
//! scripts.
//!
//! The rule goes by what a task is, not by whether something is seen to run it.
//! The relation "the automation runs it" ([`turborepo_trace_store`]'s tracked
//! outputs) was measured as the default and is wrong in both directions: one
//! launcher with a computed task name reaches every script of a repository, and
//! a repository whose checks run on a CI system that is not read has
//! its lint and type-check tasks run by nothing.

use std::collections::{HashMap, HashSet};

use turborepo_trace_graph::{GraphView, NodeId, NodeSet};
use turborepo_trace_store::Graph;

/// Name segments of a task that is interactive, long-running or mutating by
/// nature.
const HAND_RUN: &[&str] = &["dev", "start", "serve", "preview", "watch", "fix"];

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

/// Whether a task with this name is run by hand by its nature.
pub(crate) fn hand_run(name: &str) -> bool {
    name.split([':', '-', '_', '.', '/'])
        .any(|seg| HAND_RUN.contains(&seg))
}

/// The task nodes that a CI job or an install lifecycle script reaches through
/// task dependencies and explicit launches (`id:` and `script:` entries of the
/// runs relation; a script name counts in every package). A launch with a
/// computed name reaches nothing.
fn automated(g: &Graph) -> HashSet<u32> {
    let tasks = g.tasks();
    let mut by_name: HashMap<&str, Vec<u32>> = HashMap::new();
    let mut rec_of: HashMap<u32, usize> = HashMap::new();
    for (i, (n, t)) in tasks.iter().enumerate() {
        rec_of.insert(n.0, i);
        if t.id.as_str().starts_with("task:") {
            by_name.entry(t.name.as_str()).or_default().push(n.0);
        }
    }
    let mut run: HashSet<u32> = HashSet::new();
    let mut todo: Vec<u32> = Vec::new();
    for (n, t) in &tasks {
        let id = t.id.as_str();
        if (id.starts_with("gha:") || LIFECYCLE.contains(&t.name.as_str())) && run.insert(n.0) {
            todo.push(n.0);
        }
    }
    while let Some(n) = todo.pop() {
        let t = tasks[rec_of[&n]].1;
        let mut next: Vec<u32> = Vec::new();
        for d in t.deps.iter() {
            next.extend(g.node_by_path(d.as_str()).map(|x| x.0));
        }
        for r in t.runs.iter() {
            let r = r.as_str();
            if let Some(id) = r.strip_prefix("id:") {
                next.extend(g.node_by_path(id).map(|x| x.0));
            } else if let Some((_, name)) = r
                .strip_prefix("script:")
                .and_then(|rest| rest.rsplit_once('#'))
            {
                next.extend(by_name.get(name).into_iter().flatten());
            }
        }
        for m in next {
            if rec_of.contains_key(&m) && run.insert(m) {
                todo.push(m);
            }
        }
    }
    run
}

/// Task nodes matching any of `globs`: a glob matches the id without the kind
/// prefix (`apps/*#build`; `**#test` crosses directories), or, when it contains
/// a `:`, the full id as well (`make:*`; a script name may have a colon too:
/// `apps/web#lint:fix`). Unless `all`, a wildcard pattern leaves out the tasks
/// that are run by hand by their nature (see the module docs).
pub(crate) fn collect(g: &Graph, globs: &[String], all: bool) -> NodeSet {
    let mut set = NodeSet::new(g.node_count());
    if globs.is_empty() {
        return set;
    }
    let mut auto: Option<HashSet<u32>> = None;
    for (n, t) in g.tasks() {
        let id = g.path(n);
        let short = id.split_once(':').map_or(id, |(_, r)| r);
        let matches = |p: &String| {
            turborepo_trace_tasks::glob::matches(p, short)
                || (p.contains(':') && turborepo_trace_tasks::glob::matches(p, id))
        };
        // A pattern whose task name is literal (`apps/web#dev`, `**#dev`) asks for it.
        let exact = globs.iter().any(|p| {
            let name = p.rsplit_once('#').map_or(p.as_str(), |x| x.1);
            (!turborepo_trace_tasks::glob::is_glob(p)
                || (p.contains('#') && !turborepo_trace_tasks::glob::is_glob(name)))
                && matches(p)
        });
        if !exact && !globs.iter().any(matches) {
            continue;
        }
        if all
            || exact
            || !hand_run(t.name.as_str())
            || auto.get_or_insert_with(|| automated(g)).contains(&n.0)
        {
            set.insert(NodeId(n.0));
        }
    }
    set
}

#[cfg(test)]
mod tests {
    use super::hand_run;

    #[test]
    fn names_of_tasks_run_by_hand() {
        for name in [
            "dev",
            "dev:ts",
            "start",
            "unit-test-watch",
            "test:watch",
            "clean-code:fix",
            "format-fix",
            "serve",
        ] {
            assert!(hand_run(name), "{name}");
        }
        for name in [
            "build",
            "test",
            "typecheck",
            "clean-code",
            "lint:check-env",
            "fixture:read",
            "devx-report",
            "restart-check",
            "ci:build",
            "lint",
            "format",
        ] {
            assert!(!hand_run(name), "{name}");
        }
    }
}
