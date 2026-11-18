//! turbo's native packages (`sources::native`): the tasks turbo derives for
//! Cargo crates and the test tasks of uv projects, with their `turbo.json`
//! definitions.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use turborepo_trace_tasks::{
    Options, TaskSet, discover,
    sources::Workspace,
    spec::{TaskId, TaskRef, TestRun},
};

/// A new repository below the temporary directory, resolved (macOS links it).
fn repo(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("trace-native-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (rel, text) in files {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    dir.canonicalize().unwrap()
}

fn tasks(root: &Path, rels: &[&str]) -> TaskSet {
    discover(&Workspace::from_roots(root, rels), &Options::default())
}

fn names(set: &TaskSet, rel: &str) -> Vec<String> {
    let mut v: Vec<String> = set
        .tasks
        .iter()
        .filter(|t| matches!(&t.id, TaskId::Script { package, .. } if package == rel))
        .map(|t| t.name.clone())
        .collect();
    v.sort();
    v
}

fn get<'a>(set: &'a TaskSet, rel: &str, name: &str) -> &'a turborepo_trace_tasks::Task {
    set.get(&TaskId::script(rel, name))
        .unwrap_or_else(|| panic!("no task {rel}#{name}"))
}

const CARGO_TURBO: &str = r#"{
  "futureFlags": { "experimentalCargoWorkspaces": true },
  "tasks": {
    "build": { "dependsOn": ["^build"], "outputs": ["dist/**"] },
    "app#build": { "outputs": ["out/**"] },
    "test": {}
  }
}"#;

fn cargo_files(turbo: &'static str) -> Vec<(&'static str, &'static str)> {
    vec![
        ("turbo.json", turbo),
        (
            "Cargo.toml",
            "[workspace]\nmembers = [\"crates/*\"]\n\n[workspace.metadata]\nname = \"acme\"\n",
        ),
        (
            "crates/app/Cargo.toml",
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nlib-a = { path = \
             \"../lib-a\" }\n",
        ),
        ("crates/app/src/main.rs", "fn main() {}\n"),
        (
            "crates/lib-a/Cargo.toml",
            "[package]\nname = \"lib-a\"\nversion = \"0.1.0\"\n",
        ),
        ("crates/lib-a/src/lib.rs", "pub fn f() {}\n"),
    ]
}

#[test]
fn crates_have_the_tasks_turbo_derives() {
    let root = repo("cargo", &cargo_files(CARGO_TURBO));
    let set = tasks(&root, &[".", "crates/app", "crates/lib-a"]);
    assert_eq!(
        names(&set, "crates/app"),
        ["build", "check", "dev", "format", "lint", "run", "test"]
    );
    // A library has no binary to run.
    assert_eq!(
        names(&set, "crates/lib-a"),
        ["build", "check", "format", "lint", "test"]
    );
    let build = get(&set, "crates/app", "build");
    assert_eq!(build.lines, ["cargo build --package=app --locked"]);
    // `turbo.json`: the generic definition and the crate's own (`app#build`), and
    // what the command writes.
    assert!(
        build
            .spec
            .task_deps
            .iter()
            .any(|d| matches!(&d.target, TaskRef::Upstream { name } if name == "build")),
        "{:?}",
        build.spec.task_deps
    );
    let outputs: Vec<&Path> = build
        .spec
        .outputs
        .iter()
        .map(|o| o.path.as_path())
        .collect();
    for o in [
        root.join("crates/app/dist"),
        root.join("crates/app/out"),
        root.join("target"),
    ] {
        assert!(outputs.contains(&o.as_path()), "{o:?} not in {outputs:?}");
    }
    // The test task starts the crate's tests.
    let test = get(&set, "crates/lib-a", "test");
    assert_eq!(test.lines, ["cargo test --package=lib-a --locked"]);
    // The crate's tests, not the workspace's.
    assert!(
        test.spec.runs.iter().any(|r| matches!(
            r,
            TestRun::Tests { framework, dir } if framework == "cargo-test" && *dir == root.join("crates/lib-a")
        )),
        "{:?}",
        test.spec.runs
    );
}

#[test]
fn crates_have_no_tasks_without_the_future_flag() {
    let root = repo(
        "cargo-off",
        &cargo_files(r#"{ "tasks": { "build": { "outputs": ["dist/**"] } } }"#),
    );
    let set = tasks(&root, &[".", "crates/app", "crates/lib-a"]);
    assert!(names(&set, "crates/app").is_empty());
    assert!(names(&set, "crates/lib-a").is_empty());
}

#[test]
fn uv_projects_that_declare_pytest_have_a_test_task() {
    let root = repo(
        "uv",
        &[
            (
                "turbo.json",
                r#"{ "futureFlags": { "experimentalPythonWorkspaces": true }, "tasks": { "test": {} } }"#,
            ),
            (
                "pyproject.toml",
                "[tool.turbo]\nname = \"acme\"\n\n[tool.uv.workspace]\nmembers = \
                 [\"packages/*\"]\n",
            ),
            (
                "packages/py-app/pyproject.toml",
                "[project]\nname = \"py-app\"\nversion = \"0.1.0\"\n\n[dependency-groups]\ndev = \
                 [\"pytest>=8\"]\n",
            ),
            (
                "packages/py-lib/pyproject.toml",
                "[project]\nname = \"py-lib\"\nversion = \"0.1.0\"\ndependencies = \
                 [\"requests\"]\n",
            ),
        ],
    );
    let set = tasks(&root, &[".", "packages/py-app", "packages/py-lib"]);
    assert_eq!(names(&set, "packages/py-app"), ["test"]);
    assert!(names(&set, "packages/py-lib").is_empty());
    // The root declares no pytest: the workspace has no test task.
    assert!(names(&set, ".").is_empty());
    let test = get(&set, "packages/py-app", "test");
    assert_eq!(
        test.lines,
        ["uv run --active --frozen --package py-app pytest packages/py-app"]
    );
    assert!(
        test.spec
            .runs
            .iter()
            .any(|r| matches!(r, TestRun::Tests { framework, .. } if framework == "pytest")),
        "{:?}",
        test.spec.runs
    );
}
