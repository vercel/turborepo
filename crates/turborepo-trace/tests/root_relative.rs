//! A relative path on the command line (`--changed`, `--explain`, an entry) is
//! relative to the working directory when that is inside the repository, and to
//! `--root` when the command runs somewhere else: a CI step that names the
//! repository with `--root` from another directory used to get "not in the
//! graph" for `--changed src/lib.js`.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{path::Path, process::Command};

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn run(cwd: &Path, args: &[&str]) -> String {
    let o = Command::new(support::harness())
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run the harness");
    assert!(
        o.status.success(),
        "harness {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout).unwrap()
}

#[test]
fn a_relative_path_is_relative_to_the_root_when_the_command_runs_elsewhere() {
    let d = tempfile::tempdir().unwrap();
    let t = turborepo_trace_core::fs::normalize(&std::fs::canonicalize(d.path()).unwrap());
    let (repo, elsewhere) = (t.join("repo"), t.join("elsewhere"));
    std::fs::create_dir_all(&elsewhere).unwrap();
    write(&repo, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(&repo, "jest.config.js", "module.exports = {};\n");
    write(&repo, "src/lib.js", "module.exports = 1;\n");
    write(&repo, "src/other.js", "module.exports = 2;\n");
    write(
        &repo,
        "src/a.test.js",
        "require('./lib');\ntest('a', () => {});\n",
    );
    let root = repo.to_str().unwrap();
    run(&repo, &["graph", "build"]);
    // From another directory: the paths are the repository's.
    let out = run(
        &elsewhere,
        &[
            "affected",
            "--root",
            root,
            "--tests",
            "jest",
            "--changed",
            "src/lib.js",
            "--explain",
            "src/a.test.js",
        ],
    );
    assert!(out.contains("affected src/a.test.js"), "{out}");
    let out = run(
        &elsewhere,
        &[
            "affected",
            "--root",
            root,
            "--tests",
            "jest",
            "--changed",
            "src/other.js",
        ],
    );
    assert_eq!(out, "");
    let out = run(
        &elsewhere,
        &[
            "affected",
            "--root",
            root,
            "--entries",
            "src/a.test.js",
            "--changed",
            "src/lib.js",
        ],
    );
    assert_eq!(out.trim(), "src/a.test.js");
    // From a directory of the repository: relative to that directory, as before.
    let out = run(
        &repo.join("src"),
        &[
            "affected",
            "--root",
            root,
            "--tests",
            "jest",
            "--changed",
            "lib.js",
            "--explain",
            "a.test.js",
        ],
    );
    assert!(out.contains("affected src/a.test.js"), "{out}");
    let out = run(
        &repo,
        &["affected", "--tests", "jest", "--changed", "src/lib.js"],
    );
    assert_eq!(out.trim(), "src/a.test.js");
}
