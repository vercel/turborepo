//! `affected` end to end for a build script that reads its package relative to
//! the working directory (`fs.cpSync('src', 'dist')`): the script runs in the
//! directory of its task, not in the directory of the test that consumes what
//! the task builds.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{path::Path, process::Command};

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn run(root: &Path, args: &[&str], env: &[(&str, &str)]) -> String {
    let mut c = Command::new(support::harness());
    c.args(args).current_dir(root);
    for (k, v) in env {
        c.env(k, v);
    }
    let o = c.output().expect("run the harness");
    assert!(
        o.status.success(),
        "harness {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout).unwrap()
}

fn harness(root: &Path, args: &[&str]) -> String {
    run(root, args, &[])
}

/// The jest tests `changed` selects.
fn affected(root: &Path, changed: &str) -> Vec<String> {
    harness(root, &["affected", "--tests", "jest", "--changed", changed])
        .lines()
        .map(str::to_owned)
        .collect()
}

/// A workspace in which `app` loads `math` through its build output, and
/// `math` builds by copying its sources with a script that names them relative
/// to the working directory:
///
/// ```text
/// app/test/total.test.js         require('../../packages/math/dist')
/// packages/math/build.js         fs.cpSync('src', 'dist')
/// packages/math/src/{index,add}.js
/// packages/math/test/add.test.js require('../src/add')
/// ```
///
/// `dist/` is not in the checkout: what `app` loads is what the build writes.
fn repo(build: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(
        r,
        "package.json",
        r#"{"name":"acme","private":true,"scripts":{"build":"turbo run build","test":"turbo run test"}}
"#,
    );
    write(
        r,
        "pnpm-workspace.yaml",
        "packages:\n  - 'app'\n  - 'packages/*'\n",
    );
    write(
        r,
        "turbo.json",
        r#"{ "tasks": { "build": { "outputs": ["dist/**"] } } }
"#,
    );
    write(
        r,
        "app/package.json",
        r#"{"name":"app","version":"1.0.0","scripts":{"test":"jest"}}
"#,
    );
    write(r, "app/jest.config.js", "module.exports = {};\n");
    write(
        r,
        "app/test/total.test.js",
        "const { total } = require('../../packages/math/dist');\ntest('total', () => \
         expect(total(2, 3, 1)).toBe(7));\n",
    );
    write(
        r,
        "packages/math/package.json",
        &format!(
            r#"{{"name":"math","version":"1.0.0","main":"dist/index.js","scripts":{{"build":"{build}","test":"jest"}}}}
"#
        ),
    );
    write(
        r,
        "packages/math/build.js",
        "const fs = require('node:fs');\nfs.cpSync('src', 'dist', { recursive: true });\n",
    );
    write(r, "packages/math/jest.config.js", "module.exports = {};\n");
    write(
        r,
        "packages/math/src/add.js",
        "module.exports = (a, b) => a + b;\n",
    );
    write(
        r,
        "packages/math/src/index.js",
        "const add = require('./add');\nmodule.exports = { total: (...xs) => xs.reduce((a, b) => \
         add(a, b), 0) };\n",
    );
    write(
        r,
        "packages/math/test/add.test.js",
        "const add = require('../src/add');\ntest('add', () => expect(add(2, 3)).toBe(5));\n",
    );
    write(r, "README.md", "x\n");
    d
}

const APP: &str = "app/test/total.test.js";
const MATH: &str = "packages/math/test/add.test.js";

/// The sources the build script copies select the app that loads the build's
/// output: the script reads them in the directory of its task, wherever the
/// test that loads the output runs.
#[test]
fn a_build_script_reads_its_sources_relative_to_its_task() {
    let d = repo("node build.js");
    let r = d.path();
    // A source of the built package: the test that loads it directly, and the app
    // that loads what the build wrote.
    assert_eq!(affected(r, "packages/math/src/add.js"), [APP, MATH]);
    // A source only the build reads.
    assert_eq!(affected(r, "packages/math/src/index.js"), [APP]);
    // An unrelated change selects nothing.
    assert!(affected(r, "README.md").is_empty());

    // Without the rule the read is resolved with the test that reaches the build:
    // the app is missed (`TURBO_TRACE_DEBUG_CWD_TASK_RUNNER=0`, for A/B
    // measurements).
    let off = run(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--changed",
            "packages/math/src/index.js",
        ],
        &[("TURBO_TRACE_DEBUG_CWD_TASK_RUNNER", "0")],
    );
    assert_eq!(off, "");
}

/// What the build copies counts as a directory: a file added below the sources
/// reaches the app too.
#[test]
fn a_file_added_below_the_sources_reaches_the_consumer() {
    let d = repo("node build.js");
    let r = d.path();
    write(r, "packages/math/src/mul.js", "module.exports = 1;\n");
    assert_eq!(affected(r, "packages/math/src/mul.js"), [APP]);
}

/// The record says where the read was resolved, so `--explain` tells the
/// directory of the task from the directory of the test.
#[test]
fn explain_names_the_directory_of_the_task() {
    let d = repo("node build.js");
    let r = d.path();
    let out = harness(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--changed",
            "README.md",
            "--explain",
            APP,
        ],
    );
    assert!(
        out.contains(
            "covered packages/math/build.js:2 cwd_dependent (bounded by the path-like strings of \
             the code loaded with it): 'src'; resolved from the directory of the task that runs it"
        ),
        "{out}"
    );
}

/// `--strict` selects the dependents of a read it can't bound: the build runs
/// through a shell script, which can change directory before it starts the
/// command that reads.
#[test]
fn strict_selects_dependents_of_a_read_it_cannot_bound() {
    let d = repo("bash build.sh");
    let r = d.path();
    write(
        r,
        "packages/math/build.sh",
        "#!/usr/bin/env bash\nset -euo pipefail\nnode build.js\n",
    );
    // The assumption of the other mode: the read is resolved with the test that
    // reaches it, and the app is missed.
    assert_eq!(affected(r, "packages/math/src/add.js"), [MATH]);
    let out = harness(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--changed",
            "packages/math/src/add.js",
            "--strict",
            "--explain",
            APP,
        ],
    );
    assert!(
        out.contains(
            "affected app/test/total.test.js: uncertain (cwd_dependent) packages/math/build.js → \
             task:packages/math#build → app/test/total.test.js"
        ),
        "{out}"
    );
}
