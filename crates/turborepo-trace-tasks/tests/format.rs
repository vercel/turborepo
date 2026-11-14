//! In-place formatting scripts as task dependencies
//! ([`turborepo_trace_tasks::format`]). Fixture: `fixtures/tasks/format-deps/`,
//! with the shape of a real service whose `generate` script formats what it
//! generated through a root script (`pnpm -w fmt-select "$ROOT/src/gen"`):
//! the root script, a formatter without path arguments, depends on the whole
//! repository.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use turborepo_trace_tasks::{
    Options, Task, TaskSet, discover,
    sources::Workspace,
    spec::{TaskId, TaskRef},
};

fn set() -> (PathBuf, TaskSet) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../turborepo-trace-testkit/fixtures/tasks/format-deps")
        .canonicalize()
        .expect("fixture dir");
    let ws = Workspace::discover(&root);
    let set = discover(&ws, &Options::default());
    (root, set)
}

fn task<'a>(set: &'a TaskSet, id: &str) -> &'a Task {
    set.get(&id.parse::<TaskId>().unwrap())
        .unwrap_or_else(|| panic!("no task {id}"))
}

/// The root scripts `id` depends on, with whether the dependency is in place.
fn root_deps(set: &TaskSet, id: &str) -> Vec<(String, bool)> {
    task(set, id)
        .spec
        .task_deps
        .iter()
        .filter_map(|d| match &d.target {
            TaskRef::Package { package, name } if package == "." => {
                Some((name.clone(), d.in_place))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn the_root_format_script_depends_on_the_repository() {
    let (root, set) = set();
    // The formatter is recognized (no unknown command); without path arguments its
    // file set is its working directory: the repository.
    for id in ["task:.#fmt-select", "task:.#fmt", "task:.#fmt:check"] {
        let t = task(&set, id);
        assert!(t.spec.uncertainties.is_empty(), "{id}: {:?}", t.spec);
        assert!(
            t.spec
                .memberships
                .iter()
                .any(|m| m.dir == root && m.pattern == "**"),
            "{id}: {:?}",
            t.spec.memberships
        );
    }
    // Write mode rewrites in place: recorded, never a producer.
    assert!(
        task(&set, "task:.#fmt-select")
            .spec
            .outputs
            .iter()
            .all(|o| !o.links_producer())
    );
    assert!(task(&set, "task:.#fmt:check").spec.outputs.is_empty());
    // A wrapper around the format script only formats too: it has no outputs of its
    // own, so it keeps the dependency (and with it the formatter's file set).
    assert_eq!(
        task(&set, "task:.#fmt:all")
            .spec
            .task_deps
            .iter()
            .map(|d| (&d.target, d.in_place))
            .collect::<Vec<_>>(),
        [(
            &TaskRef::Local {
                name: "fmt-select".into()
            },
            true
        )]
    );
}

#[test]
fn a_generator_does_not_depend_on_the_formatter_it_calls() {
    let (_, set) = set();
    // The real shape: the generate script runs a shell script that formats its
    // output directory through the root script.
    let tun = task(&set, "task:svc/tun#generate");
    assert!(
        tun.spec.outputs.iter().any(|o| o.links_producer()),
        "{:?}",
        tun.spec.outputs
    );
    assert_eq!(root_deps(&set, "task:svc/tun#generate"), []);
    // `gen && pnpm -w fmt`, with a formatter that takes the whole repository.
    assert_eq!(root_deps(&set, "task:svc/api#generate"), []);
}

#[test]
fn checks_linters_and_tasks_without_outputs_keep_the_dependency() {
    let (_, set) = set();
    // `--check` fails the caller on any unformatted file.
    assert_eq!(
        root_deps(&set, "task:svc/chk#generate"),
        [("fmt:check".to_owned(), false)]
    );
    // A linter with `--fix` reports what it can't fix.
    assert_eq!(
        root_deps(&set, "task:svc/chk#generate:fix"),
        [("lint:fix".to_owned(), false)]
    );
    // A task without outputs: nothing links to it, and its verdict is the
    // formatter's.
    assert_eq!(
        root_deps(&set, "task:svc/chk#format"),
        [("fmt-select".to_owned(), true)]
    );
}
