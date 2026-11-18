//! The "runs" relation (`turborepo_trace_tasks::runs`) and job definitions
//! (`sources::gha::def`) over `fixtures/tasks/gha-runs`: a monorepo whose unit
//! tests run in a reusable workflow called once per computed shard (`turbo
//! <pipeline task> $FILTERS`), with an e2e job, a lint job, deploy jobs and a
//! job whose script starts tests itself.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use turborepo_trace_tasks::{
    Options, TaskSet, discover,
    sources::{
        Workspace,
        gha::def::{Defs, changed_jobs},
    },
    spec::{TaskRef, TestRun},
};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../turborepo-trace-testkit/fixtures/tasks/gha-runs")
        .canonicalize()
        .expect("fixture dir")
}

fn set() -> (PathBuf, TaskSet) {
    let root = fixture();
    let ws = Workspace::discover(&root);
    let set = discover(&ws, &Options::default());
    (root, set)
}

/// The runs of task `id`, rendered: `tests:<fw>:<dir>`, `task:<ref>`,
/// `unknown`.
fn runs(set: &TaskSet, root: &Path, id: &str) -> Vec<String> {
    let t = set
        .get(&id.parse().unwrap())
        .unwrap_or_else(|| panic!("no task {id}"));
    t.spec
        .runs
        .iter()
        .map(|r| match r {
            TestRun::Tests { framework, dir } => {
                let d = dir.strip_prefix(root).unwrap_or(dir).display().to_string();
                format!("tests:{framework}:{}", if d.is_empty() { "." } else { &d })
            }
            TestRun::Task { target } => match target {
                TaskRef::Local { name } => format!("local#{name}"),
                TaskRef::Upstream { name } => format!("^{name}"),
                TaskRef::Package { package, name } => format!("{package}#{name}"),
                TaskRef::All { name } => format!("*#{name}"),
                TaskRef::Id { id } => id.to_string(),
            },
            TestRun::Unknown { .. } => "unknown".to_string(),
            TestRun::Computed { .. } => "computed".to_string(),
            TestRun::Service { target } => match target {
                TaskRef::Id { id } => format!("service:{id}"),
                other => format!("service:{other:?}"),
            },
        })
        .collect()
}

#[test]
fn package_scripts_run_their_tests() {
    let (root, set) = set();
    assert_eq!(
        runs(&set, &root, "task:packages/a#test"),
        ["tests:jest:packages/a"]
    );
    assert_eq!(
        runs(&set, &root, "task:tools/e2e#test"),
        ["tests:jest:tools/e2e"]
    );
    assert!(runs(&set, &root, "task:packages/a#typecheck").is_empty());
    assert!(runs(&set, &root, "task:.#check-format").is_empty());
}

#[test]
fn jobs_run_what_their_steps_start() {
    let (root, set) = set();
    let q = "gha:.github/workflows/quality.yml";
    // `pnpm --filter @acme/e2e test`.
    assert_eq!(runs(&set, &root, &format!("{q}#e2e")), ["@acme/e2e#test"]);
    // `pnpm check-format`: a script that starts no tests.
    assert_eq!(
        runs(&set, &root, &format!("{q}#lint")),
        ["local#check-format"]
    );
    // A reusable workflow call runs the called jobs; `needs:` runs nothing.
    assert_eq!(
        runs(&set, &root, &format!("{q}#test-packages")),
        ["gha:.github/workflows/checks.yml#checks"]
    );
    assert!(runs(&set, &root, &format!("{q}#summary")).is_empty());
    // A script without launch words.
    assert!(runs(&set, &root, &format!("{q}#metadata")).is_empty());
    // `turbo ci:checks $FILTERS`: the pipeline task, what it depends on in
    // turbo.json, and the computed word (which could name any task).
    let checks = runs(&set, &root, "gha:.github/workflows/checks.yml#checks");
    for want in ["*#ci:checks", "*#test", "*#typecheck", "computed"] {
        assert!(checks.iter().any(|r| r == want), "{want} in {checks:?}");
    }
    assert!(!checks.iter().any(|r| r == "unknown"), "{checks:?}");
    // Deploy jobs: unknown system tools, and a turbo build whose computed word
    // after `--` is an argument of the task, not a task name. No tests.
    let d = "gha:.github/workflows/deploy.yml";
    assert_eq!(runs(&set, &root, &format!("{d}#deploy")), ["@acme/a#build"]);
    assert!(runs(&set, &root, &format!("{d}#notify")).is_empty());
    // A script that spawns `pnpm run … test:…` itself.
    assert_eq!(
        runs(&set, &root, "gha:.github/workflows/nightly.yml#smoke"),
        ["unknown"]
    );
    // A script that calls `turbo run build`, and a computed task name: no evidence
    // of a test launch.
    assert_eq!(
        runs(&set, &root, "gha:.github/workflows/scoring.yml#score"),
        ["computed"]
    );
}

/// The definition files of task `id`, relative to the fixture root.
fn definition(set: &TaskSet, root: &Path, id: &str) -> Vec<String> {
    set.get(&id.parse().unwrap())
        .unwrap_or_else(|| panic!("no task {id}"))
        .spec
        .definition
        .iter()
        .map(|p| p.strip_prefix(root).unwrap_or(p).display().to_string())
        .collect()
}

#[test]
fn definition_files_are_what_a_command_is_not_what_it_reads() {
    let (root, set) = set();
    // The script the step runs; not the application module or the helper it imports
    // (the helper counts at query time, inside the script's directory).
    assert_eq!(
        definition(&set, &root, "gha:.github/workflows/scoring.yml#score"),
        [".nvmrc", "scripts/scoring/index.mjs"]
    );
    // A shell script, and the file named by `node-version-file` in the nested
    // action.
    assert_eq!(
        definition(&set, &root, "gha:.github/workflows/deploy.yml#deploy"),
        [".github/scripts/deploy.sh", ".nvmrc"]
    );
    // A test runner's config, named or discovered.
    assert_eq!(
        definition(&set, &root, "task:packages/b#test"),
        ["packages/b/jest.ci.config.js"]
    );
    assert_eq!(
        definition(&set, &root, "task:packages/a#test"),
        ["packages/a/jest.config.js"]
    );
    // A type check reads sources: inputs, no definition file.
    assert!(definition(&set, &root, "task:packages/a#typecheck").is_empty());
}

#[test]
fn definitions_follow_local_actions() {
    let root = fixture();
    let read = |p: &str| std::fs::read_to_string(root.join(p)).ok();
    let mut defs = Defs::new(&read);
    let quality = defs.workflow(".github/workflows/quality.yml").unwrap();
    let files =
        |job: &str| -> Vec<&str> { quality[job].files.iter().map(String::as_str).collect() };
    assert_eq!(
        files("e2e"),
        [
            ".github/actions/install/action.yml",
            ".github/actions/setup-node/action.yml",
            ".github/workflows/quality.yml",
        ]
    );
    assert_eq!(files("metadata"), [".github/workflows/quality.yml"]);

    // Editing the nested action changes the jobs that install, in every workflow.
    let nested = ".github/actions/setup-node/action.yml";
    let read_new = |p: &str| {
        let s = std::fs::read_to_string(root.join(p)).ok()?;
        Some(if p == nested {
            s.replace("actions/setup-node@v4", "actions/setup-node@v5")
        } else {
            s
        })
    };
    let mut new_defs = Defs::new(&read_new);
    let mut changed = Vec::new();
    for wf in ["quality", "checks", "deploy", "nightly"] {
        let rel = format!(".github/workflows/{wf}.yml");
        let old = defs.workflow(&rel).unwrap();
        let new = new_defs.workflow(&rel).unwrap();
        for j in changed_jobs(Some(&old), &new) {
            changed.push(format!("{wf}#{j}"));
        }
    }
    assert_eq!(
        changed,
        [
            "quality#e2e",
            "quality#lint",
            "checks#checks",
            "deploy#deploy",
            "nightly#smoke"
        ]
    );
}
