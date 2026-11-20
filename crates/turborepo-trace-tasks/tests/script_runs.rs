//! The tests a package script starts (`turborepo_trace_tasks::runs`): the
//! forms of `go test`, and a test runner started by a script file.

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
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "trace-script-runs-{name}-{}-{nanos}",
        std::process::id()
    ));
    // A directory of the test's own: never one that is already there.
    std::fs::create_dir(&dir).unwrap();
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

/// The runs of the `test` script of the package at `rel`, rendered:
/// `tests:<fw>:<dir>`, a task reference, `unknown`, `computed`.
fn runs(set: &TaskSet, root: &Path, rel: &str) -> Vec<String> {
    let t = set
        .get(&TaskId::script(rel, "test"))
        .unwrap_or_else(|| panic!("no task {rel}#test"));
    let mut out: Vec<String> = t
        .spec
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
            TestRun::Service { .. } => "service".to_string(),
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

fn manifest(name: &str, test: &str) -> String {
    serde_json::json!({ "name": name, "version": "1.0.0", "scripts": { "test": test } }).to_string()
}

/// A repository with one package per `(directory, test script)` of `scripts`,
/// each with `extra` files (relative to the package).
fn packages(name: &str, scripts: &[(&str, &str)], extra: &[(&str, &str)]) -> (PathBuf, TaskSet) {
    let mut files: Vec<(String, String)> = vec![(
        "package.json".into(),
        r#"{"name":"root","private":true,"workspaces":["services/*"]}"#.into(),
    )];
    for (dir, script) in scripts {
        files.push((
            format!("services/{dir}/package.json"),
            manifest(dir, script),
        ));
        for (rel, text) in extra {
            files.push((format!("services/{dir}/{rel}"), (*text).to_owned()));
        }
    }
    let borrowed: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let root = repo(name, &borrowed);
    let rels: Vec<String> = scripts
        .iter()
        .map(|(dir, _)| format!("services/{dir}"))
        .collect();
    let rels: Vec<&str> = rels.iter().map(String::as_str).collect();
    let set = tasks(&root, &rels);
    (root, set)
}

const GO_MOD: (&str, &str) = ("go.mod", "module example.com/service\n\ngo 1.22\n");
const TEST_SH: (&str, &str) = (
    "scripts/test.sh",
    "#!/usr/bin/env bash\nset -eu\ngo test ./...\n",
);

/// `go test` starts the tests of the directory it runs in: the package's, the
/// one a `cd` or `-C` names, or the ones of its package arguments.
#[test]
fn go_test_runs_where_the_command_says() {
    let forms: &[(&str, &str, &[&str])] = &[
        ("a", "go test -race ./...", &["tests:gotest:services/a"]),
        (
            "b",
            "cd app && go test ./...",
            &["tests:gotest:services/b/app"],
        ),
        (
            "c",
            "go -C engine test ./...",
            &["tests:gotest:services/c/engine"],
        ),
        (
            "d",
            "go test -C engine ./...",
            &["tests:gotest:services/d/engine"],
        ),
        (
            "e",
            "cd ./jobs && CGO_ENABLED=0 go test -count=1 ./...",
            &["tests:gotest:services/e/jobs"],
        ),
        (
            "f",
            "go test -run TestName ./push ./email",
            &[
                "tests:gotest:services/f/email",
                "tests:gotest:services/f/push",
            ],
        ),
        ("g", "sh -c 'go test ./...'", &["tests:gotest:services/g"]),
        ("h", "./scripts/test.sh", &["tests:gotest:services/h"]),
        (
            "i",
            "go test -json ./internal/...",
            &["tests:gotest:services/i/internal"],
        ),
    ];
    let scripts: Vec<(&str, &str)> = forms.iter().map(|(d, s, _)| (*d, *s)).collect();
    let (root, set) = packages("go-test", &scripts, &[GO_MOD, TEST_SH]);
    for (dir, script, want) in forms {
        assert_eq!(
            runs(&set, &root, &format!("services/{dir}")),
            *want,
            "{script}"
        );
    }
    std::fs::remove_dir_all(&root).unwrap();
}

/// `gotestsum` is `go test` with a formatter: without arguments of its own it
/// tests `./...`, and it can be a tool of the module.
#[test]
fn gotestsum_runs_go_test() {
    let forms: &[(&str, &str, &[&str])] = &[
        (
            "a",
            "gotestsum --format testname -- -tags=integration ./...",
            &["tests:gotest:services/a"],
        ),
        ("b", "gotestsum", &["tests:gotest:services/b"]),
        ("c", "gotestsum --format dots", &["tests:gotest:services/c"]),
        (
            "d",
            "gotestsum --packages=./internal/... -- -count=1",
            &["tests:gotest:services/d/internal"],
        ),
        (
            "e",
            "gotestsum --packages './api/... ./web/...'",
            &["tests:gotest:services/e/api", "tests:gotest:services/e/web"],
        ),
        (
            "f",
            "go tool gotestsum -- ./...",
            &["tests:gotest:services/f"],
        ),
        (
            "g",
            "go tool gotestsum --format dots -- ./internal/...",
            &["tests:gotest:services/g/internal"],
        ),
        (
            "h",
            "go run gotest.tools/gotestsum@v1.12.0 -- ./...",
            &["tests:gotest:services/h"],
        ),
        (
            "i",
            "go -C engine tool gotestsum",
            &["tests:gotest:services/i/engine"],
        ),
        (
            "j",
            "gotestsum --raw-command -- go test -json ./internal/...",
            &["tests:gotest:services/j/internal"],
        ),
        // A program of the repository's own starts the tests: the task runs whole.
        (
            "k",
            "gotestsum --raw-command -- ./scripts/test.sh",
            &["unknown"],
        ),
        // The packages come before `-args`: what follows it goes to the test binary.
        (
            "l",
            "gotestsum --packages=./internal/... -- -count=1 -args -verbose",
            &["tests:gotest:services/l/internal"],
        ),
        (
            "m",
            "gotestsum --packages './api/... ./web/...' -- --args ./fixtures",
            &["tests:gotest:services/m/api", "tests:gotest:services/m/web"],
        ),
        (
            "n",
            "gotestsum -- -args ./fixtures",
            &["tests:gotest:services/n"],
        ),
        // A gotestsum started by a gotestsum is a program of its own.
        (
            "o",
            "gotestsum --raw-command -- gotestsum --raw-command -- go test -json ./internal/...",
            &["unknown"],
        ),
        (
            "p",
            "gotestsum --raw-command -- go tool gotestsum -- ./internal/...",
            &["unknown"],
        ),
    ];
    let scripts: Vec<(&str, &str)> = forms.iter().map(|(d, s, _)| (*d, *s)).collect();
    let (root, set) = packages("gotestsum", &scripts, &[GO_MOD, TEST_SH]);
    for (dir, script, want) in forms {
        assert_eq!(
            runs(&set, &root, &format!("services/{dir}")),
            *want,
            "{script}"
        );
    }
    std::fs::remove_dir_all(&root).unwrap();
}

/// A script that nests `gotestsum --raw-command` thousands of times is read in
/// one step: its task runs whole.
#[test]
fn nested_raw_gotestsum_commands_are_read_once() {
    let script = format!(
        "{}go test ./...",
        "gotestsum --raw-command -- ".repeat(4000)
    );
    let handle = std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(move || {
            let (root, set) = packages("gotestsum-nested", &[("a", &script)], &[GO_MOD]);
            let got = runs(&set, &root, "services/a");
            std::fs::remove_dir_all(&root).unwrap();
            got
        })
        .unwrap();
    assert_eq!(handle.join().unwrap(), ["unknown"]);
}

/// The Go files a task reads: `**/*.go` below a `./...` pattern.
fn go_sources(set: &TaskSet, root: &Path, rel: &str) -> Vec<String> {
    let t = set.get(&TaskId::script(rel, "test")).unwrap();
    let mut out: Vec<String> = t
        .spec
        .memberships
        .iter()
        .filter(|m| m.pattern == "**/*.go")
        .map(|m| {
            m.dir
                .strip_prefix(root)
                .unwrap_or(&m.dir)
                .display()
                .to_string()
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The packages of the command are its inputs, also after a flag that takes no
/// value and for gotestsum's default.
#[test]
fn go_test_reads_the_packages_it_names() {
    let forms: &[(&str, &str, &[&str])] = &[
        ("a", "go test -json ./...", &["services/a"]),
        ("b", "go test -failfast ./...", &["services/b"]),
        ("c", "gotestsum", &["services/c"]),
        ("d", "go -C engine test ./...", &["services/d/engine"]),
    ];
    let scripts: Vec<(&str, &str)> = forms.iter().map(|(d, s, _)| (*d, *s)).collect();
    let (root, set) = packages("go-inputs", &scripts, &[GO_MOD]);
    for (dir, script, want) in forms {
        assert_eq!(
            go_sources(&set, &root, &format!("services/{dir}")),
            *want,
            "{script}"
        );
    }
    std::fs::remove_dir_all(&root).unwrap();
}

/// A script file without a shell extension is not read command by command:
/// its text says what it starts. `./scripts/test` is that file, not the `test`
/// builtin, and a runner named by the path of its installed binary
/// (`./node_modules/.bin/jest`) is that runner.
#[test]
fn a_script_file_names_the_runner_by_its_installed_binary() {
    let script =
        |body: &str| format!("#!/usr/bin/env bash\nset -e\ncd \"$(dirname \"$0\")/..\"\n{body}\n");
    let forms: &[(&str, &str, &[&str])] = &[
        ("a", "./node_modules/.bin/jest \"$@\"", &["tests:jest:."]),
        (
            "b",
            "exec node_modules/.bin/vitest run",
            &["tests:vitest:."],
        ),
        (
            "c",
            "\"$ROOT/node_modules/.bin/jest\" --ci",
            &["tests:jest:."],
        ),
        ("d", "jest --ci", &["tests:jest:."]),
        ("g", "go test ./...", &["tests:gotest:."]),
        // Not a runner, and a file of the runner's package that is not its binary.
        ("e", "./node_modules/.bin/tsc -p .", &[]),
        ("f", "cat ./node_modules/jest/package.json", &[]),
    ];
    let mut files: Vec<(String, String)> = vec![(
        "package.json".into(),
        r#"{"name":"root","private":true,"workspaces":["packages/*"]}"#.into(),
    )];
    for (dir, body, _) in forms {
        files.push((
            format!("packages/{dir}/package.json"),
            manifest(dir, "./scripts/test"),
        ));
        files.push((format!("packages/{dir}/scripts/test"), script(body)));
    }
    let borrowed: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let root = repo("script-file", &borrowed);
    let rels: Vec<String> = forms
        .iter()
        .map(|(dir, _, _)| format!("packages/{dir}"))
        .collect();
    let rels: Vec<&str> = rels.iter().map(String::as_str).collect();
    let set = tasks(&root, &rels);
    for (dir, body, want) in forms {
        assert_eq!(
            runs(&set, &root, &format!("packages/{dir}")),
            *want,
            "{body}"
        );
    }
    std::fs::remove_dir_all(&root).unwrap();
}
