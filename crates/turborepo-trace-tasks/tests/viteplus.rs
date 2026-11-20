//! vite-plus (`vp`, `vpr`, `vpx`) over `fixtures/tasks/vite-plus`: `vp test`
//! is a vitest run rooted where it is started, `vp run` a script of a
//! package, `vp exec` and `vpx` wrap a command. The repository has a
//! `vite.config.ts` at its root that only holds formatting settings, a package
//! with a config of its own, and packages without one.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use turborepo_trace_tasks::{
    Options, TaskSet, discover,
    registry::Recognition,
    sources::Workspace,
    spec::{TaskRef, TestRun},
};

fn set() -> (PathBuf, TaskSet) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../turborepo-trace-testkit/fixtures/tasks/vite-plus")
        .canonicalize()
        .expect("fixture dir");
    let ws = Workspace::discover(&root);
    let set = discover(&ws, &Options::default());
    (root, set)
}

fn rel(root: &Path, p: &Path) -> String {
    let d = p.strip_prefix(root).unwrap_or(p).display().to_string();
    if d.is_empty() { ".".to_string() } else { d }
}

/// The runs of task `id`, rendered: `tests:<fw>:<dir>`, a task reference,
/// `unknown`.
fn runs(set: &TaskSet, root: &Path, id: &str) -> Vec<String> {
    let t = set
        .get(&id.parse().unwrap())
        .unwrap_or_else(|| panic!("no task {id}"));
    let mut out: Vec<String> = t
        .spec
        .runs
        .iter()
        .map(|r| match r {
            TestRun::Tests { framework, dir } => format!("tests:{framework}:{}", rel(root, dir)),
            TestRun::Task { target } => match target {
                TaskRef::Local { name } => format!("local#{name}"),
                TaskRef::Upstream { name } => format!("^{name}"),
                TaskRef::Package { package, name } => format!("{package}#{name}"),
                TaskRef::All { name } => format!("*#{name}"),
                TaskRef::Id { id } => id.to_string(),
            },
            TestRun::Unknown { .. } => "unknown".to_string(),
            TestRun::Computed { .. } => "computed".to_string(),
            TestRun::Service { .. } => "service".to_string(),
        })
        .collect();
    out.dedup();
    out
}

/// The recognizer and recognition of every command of task `id`.
fn commands(set: &TaskSet, id: &str) -> Vec<(String, Recognition)> {
    let t = set
        .get(&id.parse().unwrap())
        .unwrap_or_else(|| panic!("no task {id}"));
    t.commands
        .iter()
        .map(|c| (c.recognizer.unwrap_or("-").to_string(), c.recognition))
        .collect()
}

/// The definition files of task `id`, relative to the fixture root.
fn definition(set: &TaskSet, root: &Path, id: &str) -> Vec<String> {
    let t = set.get(&id.parse().unwrap()).unwrap();
    let mut out: Vec<String> = t.spec.definition.iter().map(|p| rel(root, p)).collect();
    out.sort();
    out.dedup();
    out
}

/// `vp test run` in a package script: the vitest tests of the package, by the
/// vite-plus recognizer. The subcommands and filters only narrow the run.
#[test]
fn vp_test_in_a_package_script_runs_the_vitest_tests_of_the_package() {
    let (root, set) = set();
    for script in ["test", "test:watch", "test:one"] {
        let id = format!("task:packages/math#{script}");
        assert_eq!(
            runs(&set, &root, &id),
            ["tests:vitest:packages/math"],
            "{id}"
        );
        assert_eq!(
            commands(&set, &id),
            [("vite-plus".to_string(), Recognition::Full)],
            "{id}"
        );
    }
    // `vp test` without a subcommand runs once.
    assert_eq!(
        runs(&set, &root, "task:apps/web#test"),
        ["tests:vitest:apps/web"]
    );
}

/// The root script `vp test run` is started at the root, where the config
/// without a `test` section is the run's config.
#[test]
fn vp_test_in_a_root_script_runs_the_tests_below_the_root() {
    let (root, set) = set();
    assert_eq!(runs(&set, &root, "task:.#test"), ["tests:vitest:."]);
    assert_eq!(definition(&set, &root, "task:.#test"), ["vite.config.ts"]);
}

/// `--root` and `vp -C` say where the run is rooted; `-c` names its config.
#[test]
fn vp_test_flags_select_the_root_and_the_config() {
    let (root, set) = set();
    assert_eq!(
        runs(&set, &root, "task:apps/web#test:math"),
        ["tests:vitest:packages/math"]
    );
    assert_eq!(
        runs(&set, &root, "task:apps/web#test:billing"),
        ["tests:vitest:services/billing"]
    );
    assert_eq!(
        runs(&set, &root, "task:apps/web#test:unit"),
        ["tests:vitest:apps/web"]
    );
    assert_eq!(
        definition(&set, &root, "task:apps/web#test:unit"),
        ["apps/web/vite.unit.config.ts"]
    );
    // The package's own config, imported from `vite-plus`, with its setup file.
    assert_eq!(
        definition(&set, &root, "task:apps/web#test"),
        ["apps/web/vite.config.ts"]
    );
    let web = set.get(&"task:apps/web#test".parse().unwrap()).unwrap();
    assert!(
        web.spec
            .entries
            .iter()
            .any(|e| e.path.ends_with("apps/web/test/setup.ts")),
        "{:?}",
        web.spec.entries
    );
    assert!(
        web.spec
            .memberships
            .iter()
            .any(
                |m| m.pattern.contains("*.test.ts") && m.dir.ends_with("apps/web/src")
                    || m.pattern.contains("src/**/*.test.ts")
            ),
        "{:?}",
        web.spec.memberships
    );
}

/// A run in a package without a config: vitest up to 4 takes the config of the
/// nearest directory above, so that file defines the run; the run stays rooted
/// in the package.
#[test]
fn a_config_above_the_package_defines_the_run_and_does_not_move_it() {
    let (root, set) = set();
    for id in ["task:packages/math#test", "task:services/billing#test"] {
        assert_eq!(definition(&set, &root, id), ["vite.config.ts"], "{id}");
    }
    assert_eq!(
        runs(&set, &root, "task:services/billing#test"),
        ["tests:vitest:services/billing"]
    );
}

/// `pnpm --dir <dir> exec vp test run` in a CI step runs in that directory;
/// with `-r` it runs in every package, which is not followed: vitest tests
/// anywhere.
#[test]
fn a_ci_step_that_starts_vp_test_through_the_package_manager() {
    let (root, set) = set();
    let q = "gha:.github/workflows/ci.yml";
    assert_eq!(
        runs(&set, &root, &format!("{q}#math")),
        ["tests:vitest:packages/math"]
    );
    assert_eq!(
        runs(&set, &root, &format!("{q}#everything")),
        ["tests:vitest:."]
    );
    assert!(runs(&set, &root, &format!("{q}#lint")).is_empty());
}

/// `vp run <script>` and `vpr <script>` run package scripts; `vp exec` and
/// `vpx` run a command; the other subcommands start no tests.
#[test]
fn vp_run_exec_and_the_other_subcommands() {
    let (root, set) = set();
    assert_eq!(runs(&set, &root, "task:packages/math#ci"), ["local#test"]);
    assert_eq!(runs(&set, &root, "task:.#test:all"), ["*#test"]);
    assert_eq!(runs(&set, &root, "task:.#test:math"), ["math#test"]);
    // `-t` adds the workspace dependencies of the package, and `-w` with a
    // filter selects in two ways: the script of every package.
    assert_eq!(runs(&set, &root, "task:.#test:math:deps"), ["*#test"]);
    assert_eq!(runs(&set, &root, "task:.#test:root:math"), ["*#test"]);
    assert_eq!(
        runs(&set, &root, "task:.#test:web"),
        ["tests:vitest:apps/web"]
    );
    assert!(runs(&set, &root, "task:.#check").is_empty());
    // `vpx <file>` runs the file: it defines the task.
    assert_eq!(definition(&set, &root, "task:.#seed"), ["scripts/seed.ts"]);
    // Node's options come before the file: the file after them is the script,
    // and a preloaded module defines the task too.
    assert_eq!(
        definition(&set, &root, "task:.#seed:preloaded"),
        ["scripts/env.ts", "scripts/seed.ts"]
    );
    // A task that is no script (one of a vite config's `run.tasks`) is not read:
    // partly recognized, and no test launch without a launch word.
    assert_eq!(
        commands(&set, "task:.#test:release"),
        [("vite-plus".to_string(), Recognition::Partial)]
    );
    assert!(runs(&set, &root, "task:.#test:release").is_empty());
}
