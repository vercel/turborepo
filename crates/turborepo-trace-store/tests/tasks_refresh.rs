//! Incremental equals cold for build tasks: after each kind of edit to
//! task-defining files (workflows, local actions, package scripts, Makefiles,
//! turbo config), the graph of a refreshed store is byte-identical (JSON export
//! and shard keys) to a fresh build of the same tree. Regression tests for the
//! first workflow file of a repository, which a refresh did not pick up when
//! the root package had no task before.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use turborepo_trace_core::Language;
use turborepo_trace_store::{Externals, Graph, GraphOptions, build};

fn langs() -> Vec<Arc<dyn Language>> {
    vec![Arc::new(turborepo_trace_lang_js::JsLanguage)]
}

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, contents).unwrap();
}

fn remove(root: &Path, rel: &str) {
    std::fs::remove_file(root.join(rel)).unwrap();
}

fn opts(root: &Path, out: &Path) -> GraphOptions {
    GraphOptions {
        root: root.to_path_buf(),
        out: Some(out.to_path_buf()),
        externals: Externals::Opaque,
        threads: 2,
        ..Default::default()
    }
}

/// The JSON export, the shard keys and the task ids of the store at `out`.
fn export(out: &Path) -> (String, Vec<String>, Vec<String>) {
    let g = Graph::open(out).unwrap();
    let mut buf = Vec::new();
    turborepo_trace_graph::json::write_json(&g, Some(g.root()), true, &mut buf).unwrap();
    let keys = g
        .manifest()
        .shards
        .iter()
        .map(|s| format!("{} {} {}", s.kind, s.root, s.key))
        .collect();
    let mut tasks: Vec<String> = g
        .tasks()
        .iter()
        .map(|(_, t)| t.id.as_str().to_owned())
        .collect();
    tasks.sort();
    (String::from_utf8(buf).unwrap(), keys, tasks)
}

/// A repository under test: its root and the incrementally refreshed store.
struct Repo {
    _t: tempfile::TempDir,
    root: PathBuf,
    out: PathBuf,
}

impl Repo {
    fn new(files: &[(&str, &str)]) -> Repo {
        let t = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(t.path()).unwrap();
        write(&root, ".gitignore", "node_modules\n.turbotrace\n");
        for (rel, contents) in files {
            write(&root, rel, contents);
        }
        let out = root.join(".turbotrace");
        let r = Repo { _t: t, root, out };
        build(&opts(&r.root, &r.out), &langs()).unwrap();
        r
    }

    /// A copy of `fixtures/tasks/<name>`.
    fn fixture(name: &str) -> Repo {
        fn copy(from: &Path, to: &Path) {
            std::fs::create_dir_all(to).unwrap();
            for e in std::fs::read_dir(from).unwrap() {
                let e = e.unwrap();
                let t = to.join(e.file_name());
                if e.file_type().unwrap().is_dir() {
                    copy(&e.path(), &t);
                } else {
                    std::fs::copy(e.path(), &t).unwrap();
                }
            }
        }
        let t = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(t.path()).unwrap();
        let src = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../turborepo-trace-testkit/fixtures/tasks")
            .join(name);
        copy(&src, &root);
        write(&root, ".gitignore", "node_modules\n.turbotrace\n");
        let out = root.join(".turbotrace");
        let r = Repo { _t: t, root, out };
        build(&opts(&r.root, &r.out), &langs()).unwrap();
        r
    }

    /// Refreshes the store and compares it with a fresh build of the same tree.
    /// Returns the task ids.
    fn refreshed_equals_cold(&self, what: &str) -> Vec<String> {
        self.compare(what).unwrap_or_else(|e| panic!("{e}"))
    }

    /// [`Repo::refreshed_equals_cold`], with the difference as an error.
    fn compare(&self, what: &str) -> Result<Vec<String>, String> {
        build(&opts(&self.root, &self.out), &langs()).unwrap();
        let fresh = tempfile::tempdir().unwrap();
        build(&opts(&self.root, fresh.path()), &langs()).unwrap();
        let (inc_json, inc_keys, inc_tasks) = export(&self.out);
        let (fresh_json, fresh_keys, fresh_tasks) = export(fresh.path());
        if inc_tasks != fresh_tasks {
            return Err(format!(
                "{what}: the refreshed graph's tasks differ from a fresh build's: {inc_tasks:?} \
                 vs {fresh_tasks:?}"
            ));
        }
        if inc_json != fresh_json {
            // Lines only one side has (the export is one record per line group).
            let inc: std::collections::BTreeSet<&str> = inc_json.lines().collect();
            let fr: std::collections::BTreeSet<&str> = fresh_json.lines().collect();
            let only = |a: &std::collections::BTreeSet<&str>,
                        b: &std::collections::BTreeSet<&str>,
                        side: &str|
             -> Vec<String> {
                a.difference(b)
                    .take(8)
                    .map(|l| format!("only {side}: {}", l.trim()))
                    .collect()
            };
            let mut diff = only(&inc, &fr, "refreshed");
            diff.extend(only(&fr, &inc, "fresh"));
            if diff.is_empty() {
                // The same lines in another order.
                diff.extend(
                    inc_json
                        .lines()
                        .zip(fresh_json.lines())
                        .enumerate()
                        .filter(|(_, (a, b))| a != b)
                        .take(4)
                        .map(|(i, (a, b))| {
                            format!("line {i}: refreshed {} | fresh {}", a.trim(), b.trim())
                        }),
                );
            }
            return Err(format!(
                "{what}: the refreshed graph differs from a fresh build ({} vs {} lines):\n{}",
                inc_json.lines().count(),
                fresh_json.lines().count(),
                diff.join("\n")
            ));
        }
        if inc_keys != fresh_keys {
            return Err(format!("{what}: shard keys differ from a fresh build's"));
        }
        Ok(inc_tasks)
    }

    /// Starts the incremental store over from a fresh build (after a reported
    /// difference).
    fn rebuild(&self) {
        std::fs::remove_dir_all(&self.out).unwrap();
        build(&opts(&self.root, &self.out), &langs()).unwrap();
    }
}

const WORKFLOW: &str = "name: ci\non: [push]\njobs:\n  test:\n    runs-on: ubuntu-latest\n    \
                        steps:\n      - uses: actions/checkout@v4\n      - run: node \
                        tools/check.js\n";
const SOURCES: &[(&str, &str)] = &[
    ("package.json", r#"{"name":"root","private":true}"#),
    ("src/index.js", "module.exports = 1;\n"),
    ("tools/check.js", "require('../src/index.js');\n"),
];

fn gha(tasks: &[String]) -> Vec<&str> {
    tasks
        .iter()
        .map(String::as_str)
        .filter(|t| t.starts_with("gha:"))
        .collect()
}

#[test]
fn the_first_workflow_of_a_repository_is_picked_up() {
    // No task at all before: the root package has no script and there is no
    // workflow.
    let r = Repo::new(SOURCES);
    assert!(r.refreshed_equals_cold("no change").is_empty());
    write(&r.root, ".github/workflows/ci.yml", WORKFLOW);
    let tasks = r.refreshed_equals_cold("first workflow added");
    assert_eq!(gha(&tasks), ["gha:.github/workflows/ci.yml#test"]);
    // A second one, an edit, and the removal of each.
    write(
        &r.root,
        ".github/workflows/deploy.yaml",
        "on: [push]\njobs:\n  deploy:\n    runs-on: x\n    steps:\n      - run: echo deploy\n",
    );
    let tasks = r.refreshed_equals_cold("second workflow added");
    assert_eq!(gha(&tasks).len(), 2);
    write(
        &r.root,
        ".github/workflows/ci.yml",
        &WORKFLOW.replace("  test:", "  unit:"),
    );
    let tasks = r.refreshed_equals_cold("job renamed");
    assert!(gha(&tasks).contains(&"gha:.github/workflows/ci.yml#unit"));
    remove(&r.root, ".github/workflows/deploy.yaml");
    r.refreshed_equals_cold("one workflow deleted");
    remove(&r.root, ".github/workflows/ci.yml");
    let tasks = r.refreshed_equals_cold("last workflow deleted");
    assert!(gha(&tasks).is_empty(), "{tasks:?}");
    // And the first one again, now that the directory exists and is empty.
    write(&r.root, ".github/workflows/ci.yml", WORKFLOW);
    let tasks = r.refreshed_equals_cold("first workflow added to an empty directory");
    assert_eq!(gha(&tasks), ["gha:.github/workflows/ci.yml#test"]);
    std::fs::remove_dir_all(r.root.join(".github")).unwrap();
    let tasks = r.refreshed_equals_cold(".github deleted");
    assert!(gha(&tasks).is_empty(), "{tasks:?}");
}

#[test]
fn the_first_workflow_next_to_package_scripts() {
    let mut files = SOURCES.to_vec();
    files[0] = (
        "package.json",
        r#"{"name":"root","private":true,"scripts":{"check":"node tools/check.js"}}"#,
    );
    let r = Repo::new(&files);
    assert_eq!(r.refreshed_equals_cold("no change"), ["task:.#check"]);
    write(&r.root, ".github/workflows/ci.yml", WORKFLOW);
    let tasks = r.refreshed_equals_cold("first workflow added");
    assert_eq!(gha(&tasks), ["gha:.github/workflows/ci.yml#test"]);
}

#[test]
fn the_first_local_action_and_reusable_workflow() {
    let mut files = SOURCES.to_vec();
    files.push((
        ".github/workflows/ci.yml",
        "on: [push]\njobs:\n  test:\n    runs-on: x\n    steps:\n      - uses: \
         ./.github/actions/setup\n  call:\n    uses: ./.github/workflows/reuse.yml\n",
    ));
    let r = Repo::new(&files);
    r.refreshed_equals_cold("no change");
    // The action a step names appears.
    write(
        &r.root,
        ".github/actions/setup/action.yml",
        "runs:\n  using: composite\n  steps:\n    - run: node tools/check.js\n      shell: bash\n",
    );
    r.refreshed_equals_cold("first local action added");
    write(
        &r.root,
        ".github/actions/setup/action.yml",
        "runs:\n  using: composite\n  steps:\n    - run: node src/index.js\n      shell: bash\n",
    );
    r.refreshed_equals_cold("local action edited");
    // The reusable workflow a job calls appears.
    write(
        &r.root,
        ".github/workflows/reuse.yml",
        "on:\n  workflow_call:\njobs:\n  inner:\n    runs-on: x\n    steps:\n      - run: echo \
         inner\n",
    );
    let tasks = r.refreshed_equals_cold("first reusable workflow added");
    assert!(gha(&tasks).contains(&"gha:.github/workflows/reuse.yml#inner"));
    remove(&r.root, ".github/actions/setup/action.yml");
    r.refreshed_equals_cold("local action deleted");
    remove(&r.root, ".github/workflows/reuse.yml");
    let tasks = r.refreshed_equals_cold("reusable workflow deleted");
    assert!(!gha(&tasks).contains(&"gha:.github/workflows/reuse.yml#inner"));
}

#[test]
fn the_first_task_file_of_each_kind() {
    // A package without any task gets its first Makefile, script, turbo config and
    // Dockerfile, one after the other.
    let mut files = SOURCES.to_vec();
    files.push(("packages/a/package.json", r#"{"name":"a"}"#));
    files.push(("packages/a/src/a.js", "module.exports = 2;\n"));
    files[0] = (
        "package.json",
        r#"{"name":"root","private":true,"workspaces":["packages/*"]}"#,
    );
    let r = Repo::new(&files);
    assert!(r.refreshed_equals_cold("no change").is_empty());
    write(&r.root, "Makefile", "build:\n\tnode tools/check.js\n");
    let tasks = r.refreshed_equals_cold("first Makefile added");
    assert!(tasks.iter().any(|t| t.starts_with("make:")), "{tasks:?}");
    write(
        &r.root,
        "packages/a/package.json",
        r#"{"name":"a","scripts":{"build":"node src/a.js"}}"#,
    );
    let tasks = r.refreshed_equals_cold("first script added");
    assert!(
        tasks.iter().any(|t| t == "task:packages/a#build"),
        "{tasks:?}"
    );
    write(
        &r.root,
        "turbo.json",
        r#"{"tasks":{"build":{"outputs":["dist/**"]}}}"#,
    );
    r.refreshed_equals_cold("turbo.json added");
    write(
        &r.root,
        "packages/a/Dockerfile",
        "FROM node:22\nCOPY src ./src\n",
    );
    let tasks = r.refreshed_equals_cold("first Dockerfile added");
    assert!(tasks.iter().any(|t| t.starts_with("docker:")), "{tasks:?}");
    remove(&r.root, "Makefile");
    remove(&r.root, "turbo.json");
    remove(&r.root, "packages/a/Dockerfile");
    let tasks = r.refreshed_equals_cold("task files deleted");
    assert_eq!(tasks, ["task:packages/a#build"]);
}

/// Whether a file of a task fixture defines tasks or is run or read by one as a
/// task file: manifests, task-runner configs, workflows and actions,
/// Dockerfiles, shell scripts.
fn is_task_file(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    matches!(
        name,
        "package.json"
            | "turbo.json"
            | "turbo.jsonc"
            | "Makefile"
            | "makefile"
            | "GNUmakefile"
            | "justfile"
            | "Justfile"
            | "tox.ini"
            | "noxfile.py"
            | "project.json"
            | "nx.json"
            | "Cargo.toml"
            | "config.toml"
            | "pyproject.toml"
            | "setup.cfg"
            | "setup.py"
            | "MANIFEST.in"
            | ".dockerignore"
    ) || name.starts_with("Dockerfile")
        || name.starts_with("Taskfile")
        || [".yml", ".yaml", ".mk", ".sh"]
            .iter()
            .any(|e| name.ends_with(e))
}

fn files_below(dir: &Path, base: &Path, out: &mut Vec<String>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let e = e.unwrap();
        let p = e.path();
        if e.file_name() == ".turbotrace" {
            continue;
        }
        if e.file_type().unwrap().is_dir() {
            files_below(&p, base, out);
        } else {
            out.push(
                p.strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
}

/// Fixtures left out of the general check, with the reason. Remove an entry
/// when its cause is fixed.
///
/// None now. `codegen` was skipped: its JS config files name YAML and JSON
/// files in strings that the JS plugin resolves like module strings (`input:
/// './openapi.yaml'`), and while such a file existed nothing was recorded for
/// the string, so deleting it or adding it back did not re-trace the config.
/// The lookup now records the file's membership of its directory.
const SKIPPED_FIXTURES: &[&str] = &[];

/// The general check: in every task fixture, deleting a task-defining file and
/// putting it back each leave a refreshed store equal to a fresh build. The
/// first and the last file of a kind in a package are among these edits.
#[test]
fn every_task_fixture_refreshes_like_a_cold_build() {
    let fixtures =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../turborepo-trace-testkit/fixtures/tasks");
    let mut names: Vec<String> = std::fs::read_dir(&fixtures)
        .unwrap()
        .map(|e| e.unwrap())
        .filter(|e| e.file_type().unwrap().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let mut edits = 0;
    let mut differences: Vec<String> = Vec::new();
    for name in names
        .iter()
        .filter(|n| !SKIPPED_FIXTURES.contains(&n.as_str()))
    {
        let r = Repo::fixture(name);
        r.refreshed_equals_cold(&format!("{name}: no change"));
        let mut files = Vec::new();
        files_below(&r.root, &r.root, &mut files);
        files.sort();
        let mut check = |what: String| {
            if let Err(e) = r.compare(&what) {
                differences.push(e);
                r.rebuild();
            }
        };
        for rel in files.iter().filter(|f| is_task_file(f)) {
            let p = r.root.join(rel);
            let contents = std::fs::read(&p).unwrap();
            std::fs::remove_file(&p).unwrap();
            check(format!("{name}: {rel} deleted"));
            std::fs::write(&p, &contents).unwrap();
            check(format!("{name}: {rel} added back"));
            edits += 2;
        }
    }
    assert!(
        edits > 100,
        "task files found in {} fixtures: {edits} edits",
        names.len()
    );
    assert!(
        differences.is_empty(),
        "{} of {edits} edits leave a refreshed store that differs from a fresh build:\n{}",
        differences.len(),
        differences.join("\n")
    );
}

#[test]
fn edits_of_the_workflow_fixture() {
    let r = Repo::fixture("gha-runs");
    r.refreshed_equals_cold("no change");
    let edit = |rel: &str, from: &str, to: &str| {
        let p = r.root.join(rel);
        let s = std::fs::read_to_string(&p).unwrap();
        assert!(s.contains(from), "{rel} contains {from:?}");
        std::fs::write(p, s.replacen(from, to, 1)).unwrap();
    };
    edit(
        ".github/workflows/quality.yml",
        "runs-on: ubuntu-latest",
        "runs-on: ubuntu-24.04",
    );
    r.refreshed_equals_cold("job key edited");
    edit(
        ".github/workflows/checks.yml",
        "turbo ci:checks $FILTERS",
        "turbo ci:checks --filter=@acme/a",
    );
    r.refreshed_equals_cold("step command edited");
    edit(
        ".github/actions/setup-node/action.yml",
        "actions/setup-node@v4",
        "actions/setup-node@v5",
    );
    r.refreshed_equals_cold("nested action edited");
    edit(".github/scripts/weights.mjs", "a: 2", "a: 3");
    r.refreshed_equals_cold("script module edited");
    // A script starts launching tests: what the job runs changes.
    edit(
        "scripts/scoring/index.mjs",
        "turbo run build",
        "turbo run test",
    );
    r.refreshed_equals_cold("script text edited");
    write(
        &r.root,
        ".github/workflows/extra.yml",
        "on: [push]\njobs:\n  extra:\n    runs-on: x\n    steps:\n      - run: pnpm --filter \
         @acme/b test\n",
    );
    r.refreshed_equals_cold("workflow added");
    remove(&r.root, ".github/workflows/nightly.yml");
    r.refreshed_equals_cold("workflow deleted");
    remove(&r.root, ".github/actions/install/action.yml");
    r.refreshed_equals_cold("action deleted");
    edit(
        "turbo.json",
        r#""dependsOn": ["test", "typecheck"]"#,
        r#""dependsOn": ["typecheck"]"#,
    );
    r.refreshed_equals_cold("turbo dependsOn edited");
    edit(
        "packages/b/package.json",
        "jest -c jest.ci.config.js",
        "jest",
    );
    r.refreshed_equals_cold("package script edited");
    remove(&r.root, ".nvmrc");
    r.refreshed_equals_cold("named file deleted");
}
