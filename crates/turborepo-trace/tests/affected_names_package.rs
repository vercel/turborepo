//! `affected` end to end for the package assumption of the names
//! bound (`turborepo_trace_graph::names`): a read with a computed path is taken
//! to read only files of the package it runs for, unless the code says where
//! the path goes.
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
    c.args(["affected", "--tests", "jest"])
        .args(args)
        .current_dir(root);
    for (k, v) in env {
        c.env(k, v);
    }
    let o = c.output().expect("run the harness");
    assert!(
        o.status.success(),
        "turbotrace affected failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout).unwrap()
}

/// The tests a change of `changed` selects.
fn affected(root: &Path, changed: &str, more: &[&str], env: &[(&str, &str)]) -> Vec<String> {
    let mut args = vec!["--changed", changed];
    args.extend(more);
    run(root, &args, env).lines().map(str::to_owned).collect()
}

const OFF: &[(&str, &str)] = &[("TURBO_TRACE_DEBUG_NAMES_PACKAGE", "0")];

/// A workspace of four packages and the root package's own files:
///
/// ```text
/// packages/reader/index.js      readText(p): fs.readFileSync(p)          p is a parameter
/// services/a/test/a.test.js     readText(join(base(), 'fixtures', name))  base() is text from outside
/// services/b/test/b.test.js     the same, and a path written with parent steps: '../../shared/data'
/// services/c/src/dir.js         exports.DIR = join(__dirname, '../../../shared/located')
/// services/c/test/c.test.js     readText(join(DIR, name))
/// ```
///
/// Every `fixtures` directory is named by the string `fixtures` of the tests of
/// `a` and `b`; which of them a test can read is the question.
fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(
        r,
        "pnpm-workspace.yaml",
        "packages:\n  - 'services/*'\n  - 'packages/*'\n",
    );
    write(
        r,
        "packages/reader/package.json",
        r#"{"name":"reader","version":"1.0.0"}"#,
    );
    write(
        r,
        "packages/reader/index.js",
        "const fs = require('fs');\nexports.readText = function (p) {\n  return \
         fs.readFileSync(p, 'utf8');\n};\n",
    );
    for s in ["a", "b", "c"] {
        write(
            r,
            &format!("services/{s}/package.json"),
            &format!(r#"{{"name":"svc-{s}","version":"1.0.0"}}"#),
        );
        write(
            r,
            &format!("services/{s}/jest.config.js"),
            "module.exports = {};\n",
        );
        write(r, &format!("services/{s}/fixtures/x.json"), "{}\n");
    }
    let test = "const path = require('path');\nconst { readText } = \
                require('../../../packages/reader/index.js');\nconst base = () => \
                globalThis.base;\ntest('t', () => { readText(path.join(base(), 'fixtures', \
                globalThis.name)); });\n";
    write(r, "services/a/test/a.test.js", test);
    write(
        r,
        "services/b/test/b.test.js",
        &format!(
            "{test}const shared = '../../shared/data';\ntest('s', () => {{ readText(shared + '/' \
             + globalThis.name); }});\n"
        ),
    );
    write(
        r,
        "services/c/src/dir.js",
        "exports.DIR = require('path').join(__dirname, '../../../shared/located');\n",
    );
    write(
        r,
        "services/c/test/c.test.js",
        "const path = require('path');\nconst { readText } = \
         require('../../../packages/reader/index.js');\nconst { DIR } = \
         require('../src/dir');\ntest('t', () => { readText(path.join(DIR, globalThis.name)); \
         });\n",
    );
    write(r, "packages/reader/fixtures/x.json", "{}\n");
    // A file of the reader's package that no test loads knows this fixture by its
    // path, so the fixture is a file of the graph: a change to it is a
    // modification, and only names can select the tests for it.
    write(
        r,
        "packages/reader/known.js",
        "module.exports = require('fs').readFileSync(require('path').join(__dirname, \
         'fixtures/x.json'));\n",
    );
    write(r, "fixtures/x.json", "{}\n");
    write(r, "shared/data/x.json", "{}\n");
    write(r, "shared/located/x.json", "{}\n");
    write(r, "shared/other/x.json", "{}\n");
    d
}

const A: &str = "services/a/test/a.test.js";
const B: &str = "services/b/test/b.test.js";
const C: &str = "services/c/test/c.test.js";

#[test]
fn a_string_names_only_files_of_the_test_s_package_or_of_the_read_s() {
    let d = repo();
    let r = d.path();
    // The test's own package.
    assert_eq!(affected(r, "services/a/fixtures/x.json", &[], &[]), [A]);
    assert_eq!(affected(r, "services/b/fixtures/x.json", &[], &[]), [B]);
    // The package of the file with the read.
    let in_reader = "packages/reader/fixtures/x.json";
    assert_eq!(
        affected(r, in_reader, &[], &[]),
        [A, B],
        "{}",
        run(r, &["--changed", in_reader, "--explain", C], &[])
    );
    // Another package, the root package's files: assumed not to be read.
    assert!(affected(r, "services/c/fixtures/x.json", &[], &[]).is_empty());
    assert!(affected(r, "fixtures/x.json", &[], &[]).is_empty());
    // Without the assumption the string names the shape anywhere.
    for path in [
        "services/a/fixtures/x.json",
        "services/c/fixtures/x.json",
        "fixtures/x.json",
    ] {
        assert_eq!(affected(r, path, &[], OFF), [A, B], "{path}");
    }
    // `--strict` assumes nothing: the read is open, and every test that reaches it
    // is selected.
    assert_eq!(
        affected(r, "services/c/fixtures/x.json", &["--strict"], &[]),
        [A, B, C]
    );
}

#[test]
fn a_path_the_code_locates_is_named_in_any_package() {
    let d = repo();
    let r = d.path();
    // A string written with parent steps says that it leaves.
    assert_eq!(affected(r, "shared/data/x.json", &[], &[]), [B]);
    // A literal path from a file's location says where it goes, also for the module
    // that receives the value.
    assert_eq!(affected(r, "shared/located/x.json", &[], &[]), [C]);
    // Not what is beside them.
    assert!(affected(r, "shared/other/x.json", &[], &[]).is_empty());
}

#[test]
fn explain_states_the_assumption() {
    let d = repo();
    let r = d.path();
    let out = run(
        r,
        &["--changed", "services/c/fixtures/x.json", "--explain", A],
        &[],
    );
    assert!(out.starts_with(&format!("unaffected {A}")), "{out}");
    assert!(
        out.contains(
            "assumed: a read with a computed path reads only files of the package of this entry \
             (services/a/) or of the package of the file that holds the read"
        ),
        "{out}"
    );
    // Selected inside the package: the cause names the string and the read.
    let out = run(
        r,
        &["--changed", "services/a/fixtures/x.json", "--explain", A],
        &[],
    );
    assert!(
        out.contains("named by the string 'fixtures'") && out.contains("packages/reader/index.js"),
        "{out}"
    );
    // Without the assumption nothing is said about it.
    let out = run(r, &["--changed", "docs/none.md", "--explain", A], OFF);
    assert!(!out.contains("assumed:"), "{out}");
}
