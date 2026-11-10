//! Services a task starts before the tests it runs.
//!
//! Integration tests often talk over the network to a service of the same
//! repository that the CI job (or a package script) starts first: `docker
//! compose up --wait`, then the test command. No import connects the tests to
//! the service's code: a Go service tested by TypeScript tests, a database
//! seeded from a mounted SQL file, a mock server with mounted expectations.
//! Without this module, an edit to the service selects none of those tests.
//!
//! The model uses what the task layer records where the dependency is declared:
//!
//! 1. **Services.** A command that starts a compose project and leaves it
//!    running records `service:<task id>` in its task's runs
//!    ([`turborepo_trace_tasks::spec::TestRun::Service`]). The task
//!    `docker:<compose file>` holds what the project is made of: the compose
//!    file and its `.env`, the build contexts of its services (the `COPY`
//!    sources of their Dockerfiles), bind-mounted files, env files, config and
//!    secret files.
//! 2. **Changed services.** A service changed when a change reaches its task:
//!    the same evaluation as for any task entry, with uncertainty ignored (a
//!    service is never "affected" because something else is uncertain).
//! 3. **Whose tests.** A task that starts a changed service selects the tests
//!    it runs ([`crate::workflows::Runs`]): the steps of a job after `docker
//!    compose up`, the rest of a script line, the scripts and turbo tasks they
//!    launch. A task also starts the services of the tasks it runs or depends
//!    on when those run no tests themselves (a `services:up` script, a turbo
//!    `dependsOn` that provisions a database). A task that runs other tasks
//!    with tests of their own does not inherit their services: `turbo run test`
//!    over the repository does not connect one package's database to every
//!    other package's tests.
//!
//! What this does not see: services started by code (a test's global setup that
//! spawns a process, a script that calls the Docker API), and the service a
//! test talks to when a task starts several. Both err on the side of the task:
//! every test it runs is selected.
//!
//! Cost: nothing for a repository without such commands; otherwise one more
//! evaluation over the service tasks, and a walk up from the tasks that start a
//! changed service.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use turborepo_trace_graph::{
    AffectedOptions, AffectedReport, Cause, ChangeSet, Engine, EntryResult, GraphView, NodeId,
    NodeSet, Policy, Verdict,
};
use turborepo_trace_store::Graph;

use crate::workflows::Runs;

/// The graph the engine runs on (node ids are those of the base graph).
type View<'g> = turborepo_trace_graph::Patched<'g, Graph>;

/// A test selected because its task starts a changed service.
#[derive(Clone, Debug)]
pub struct Hit {
    /// The chain from the changed node to the service's task, both inclusive.
    pub service: Vec<NodeId>,
    /// The task that starts the service and runs the test.
    pub task: NodeId,
    /// Why the first node of the chain changed.
    pub cause: Cause,
}

/// What changed services add to an `affected` query.
#[derive(Debug, Default)]
pub struct Effects {
    /// Tests of tasks that start a changed service.
    pub tests: BTreeMap<NodeId, Hit>,
    /// Readable notes for `--format json` and `--stats`.
    pub notes: Vec<String>,
}

/// Computes the tests selected through changed services (see the module docs).
/// `opts` are the options of the main evaluation.
pub fn effects(
    g: &Graph,
    engine: &Engine<'_, View<'_>>,
    cs: &ChangeSet,
    opts: &AffectedOptions,
) -> Effects {
    let mut fx = Effects::default();
    let mut runs = Runs::new(g);
    // 1. The tasks that start services, with the services.
    let mut starters: Vec<(NodeId, Vec<NodeId>)> = runs
        .tasks()
        .filter_map(|n| {
            let s = runs.own_services(n);
            (!s.is_empty()).then_some((n, s))
        })
        .collect();
    if starters.is_empty() {
        return fx;
    }
    starters.sort_unstable();
    let mut services = NodeSet::new(g.node_count());
    for (_, own) in &starters {
        for &s in own {
            services.insert(s);
        }
    }
    // 2. The services a change reaches.
    let report = engine.affected(cs, &services, &opts.clone().policy(Policy::Ignore));
    let changed: HashMap<NodeId, &EntryResult> = report
        .entries
        .iter()
        .filter(|e| e.verdict == Verdict::Affected && e.cause.is_some())
        .map(|e| (e.entry, e))
        .collect();
    if changed.is_empty() {
        return fx;
    }
    // 3. The tasks that start them and run tests: from each starter of a changed
    //    service
    // up through the tasks that run or depend on it, as long as the task itself
    // runs no tests. The first task with tests on each way is the one whose
    // tests talk to the service; nothing is inherited beyond it.
    let index = runs.parent_index();
    let mut roots: BTreeMap<NodeId, NodeId> = BTreeMap::new();
    let mut runs_tests: HashMap<NodeId, bool> = HashMap::new();
    for (starter, own) in &starters {
        let Some(&service) = own.iter().find(|s| changed.contains_key(s)) else {
            continue;
        };
        let mut seen: BTreeSet<NodeId> = BTreeSet::new();
        let mut todo = vec![*starter];
        while let Some(t) = todo.pop() {
            if !seen.insert(t) || seen.len() > 4096 {
                continue;
            }
            let any = *runs_tests.entry(t).or_insert_with(|| runs.of(t).any());
            if any {
                roots.entry(t).or_insert(service);
            } else {
                todo.extend(runs.parents(t, &index));
            }
        }
    }
    let mut unmapped: Option<String> = None;
    for (&task, service) in &roots {
        let hit = changed[service];
        let Some(cause) = hit.cause.clone() else {
            continue;
        };
        let r = runs.of(task);
        if let Some(reason) = r.every_test() {
            unmapped.get_or_insert_with(|| format!("{} ({reason})", g.path(task)));
        }
        for test in runs.tests_of(&r) {
            fx.tests.entry(test).or_insert_with(|| Hit {
                service: hit.witness.clone(),
                task,
                cause: cause.clone(),
            });
        }
    }
    let mut names: Vec<&str> = changed.keys().map(|&s| g.path(s)).collect();
    names.sort_unstable();
    names.truncate(3);
    fx.notes.push(format!(
        "services: {} changed service definition(s) ({}); {} task(s) that start them run {} \
         test(s){}",
        changed.len(),
        names.join(", "),
        roots.len(),
        fx.tests.len(),
        match unmapped {
            Some(u) => format!("; every test, because of a launch that can't be mapped: {u}"),
            None => String::new(),
        }
    ));
    fx
}

impl Effects {
    /// Adds the verdicts to `report`: among `entries`, the tests of tasks that
    /// start a changed service are affected.
    pub fn apply(&self, g: &Graph, entries: &NodeSet, report: &mut AffectedReport) {
        if self.tests.is_empty() {
            return;
        }
        let mut at: HashMap<NodeId, usize> = HashMap::new();
        for (i, e) in report.entries.iter().enumerate() {
            at.insert(e.entry, i);
        }
        let mut added = false;
        for (&test, hit) in &self.tests {
            if !entries.contains(test) {
                continue;
            }
            let mut witness = hit.service.clone();
            witness.push(hit.task);
            witness.push(test);
            let result = EntryResult {
                entry: test,
                verdict: Verdict::Affected,
                witness,
                cause: Some(hit.cause.clone()),
                uncertain: None,
            };
            match at.get(&test) {
                // A change is a stronger reason than uncertainty.
                Some(&i) if report.entries[i].cause.is_none() => report.entries[i] = result,
                Some(_) => {}
                None => {
                    at.insert(test, report.entries.len());
                    report.entries.push(result);
                    added = true;
                }
            }
        }
        if added {
            report
                .entries
                .sort_by(|a, b| g.path(a.entry).cmp(g.path(b.entry)));
        }
    }
}
