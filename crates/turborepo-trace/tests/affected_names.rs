//! `turbotrace affected` end to end for reads whose path cannot be evaluated:
//! they are bounded by the path-like strings and file locations of the code
//! loaded with them (`turborepo_trace_graph::names`), instead of selecting
//! their tests on every change.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{
    path::Path,
    process::{Command, Output},
};

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn run_env(root: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut c = Command::new(support::harness());
    c.args(args).current_dir(root);
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().expect("run turbotrace")
}

fn ok_env(root: &Path, args: &[&str], env: &[(&str, &str)]) -> String {
    let o = run_env(root, args, env);
    assert!(
        o.status.success(),
        "turbotrace {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout).unwrap()
}

fn affected_env(root: &Path, extra: &[&str], env: &[(&str, &str)]) -> Vec<String> {
    let mut args = vec!["affected", "--tests", "jest"];
    args.extend_from_slice(extra);
    ok_env(root, &args, env)
        .lines()
        .map(str::to_owned)
        .collect()
}

fn affected(root: &Path, extra: &[&str]) -> Vec<String> {
    affected_env(root, extra, &[])
}

/// A shared reader whose path is a parameter (the shape of a token or config
/// reader in a shared package), used by the tests of several services:
///
/// ```text
/// packages/reader/index.js            readText(p): fs.readFileSync(p)       p is a parameter
///                                     readAll(dir): readdirSync(dir) + readFileSync(join(dir, name))
/// services/a/test/paths.js            the string '/secrets/' (line 3), base from the environment
/// services/a/test/token.test.js       readText(tokenPath('dev'))            reads services/a/secrets/dev.token
/// services/b/test/env.test.js         readText(process.env.TOKEN_FILE)      no name in its code
/// services/c/test/here.test.js        readAll(__dirname)                    its own directory
/// services/d/test/table.test.js       readText(base + routes.json[i].file)  names from a data file
/// services/e/test/none.test.js        does not use the reader
/// ```
fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(r, "README.md", "readme\n");
    write(
        r,
        "packages/reader/package.json",
        r#"{"name":"reader","version":"1.0.0"}"#,
    );
    write(
        r,
        "packages/reader/index.js",
        "const fs = require('fs');\nconst path = require('path');\nexports.readText = function \
         (p) {\n  return fs.readFileSync(p, 'utf8');\n};\nexports.readAll = function (dir) {\n  \
         return fs.readdirSync(dir).map((name) => fs.readFileSync(path.join(dir, name), \
         'utf8'));\n};\n",
    );
    for s in ["a", "b", "c", "d", "e"] {
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
    }
    write(
        r,
        "services/a/test/paths.js",
        "const base = process.env.SERVICE_ROOT;\nexports.tokenPath = function (name) {\n  return \
         base + '/secrets/' + name + '.token';\n};\n",
    );
    write(
        r,
        "services/a/test/token.test.js",
        "const { readText } = require('../../../packages/reader');\nconst { tokenPath } = \
         require('./paths');\ntest('token', () => { readText(tokenPath('dev')); });\n",
    );
    write(r, "services/a/secrets/dev.token", "t\n");
    write(
        r,
        "services/b/test/env.test.js",
        "const { readText } = require('../../../packages/reader');\ntest('env', () => { \
         readText(process.env.TOKEN_FILE); });\n",
    );
    write(
        r,
        "services/c/test/here.test.js",
        "const { readAll } = require('../../../packages/reader');\ntest('here', () => { \
         readAll(__dirname); });\n",
    );
    write(r, "services/c/test/notes.txt", "n\n");
    write(
        r,
        "services/d/test/table.test.js",
        "const { readText } = require('../../../packages/reader');\nconst routes = \
         require('../routes.json');\ntest('table', () => {\n  for (const r of routes) \
         readText(process.env.SERVICE_ROOT + '/' + r.file);\n});\n",
    );
    write(
        r,
        "services/d/routes.json",
        r#"[{"file": "handlers/get.sql"}, {"file": "handlers/put.sql"}]"#,
    );
    write(r, "services/d/handlers/get.sql", "select 1\n");
    write(
        r,
        "services/e/test/none.test.js",
        "test('none', () => {});\n",
    );
    write(r, "services/e/src/x.js", "module.exports = 1;\n");
    d
}

const READERS: [&str; 4] = [
    "services/a/test/token.test.js",
    "services/b/test/env.test.js",
    "services/c/test/here.test.js",
    "services/d/test/table.test.js",
];

#[test]
fn an_unrelated_change_selects_no_reader() {
    let d = repo();
    let r = d.path();
    // Fidelity: without the names layer the computed reads are open, and every test
    // that reaches the reader is selected on any change.
    assert_eq!(
        affected_env(
            r,
            &["--changed", "README.md"],
            &[("TURBO_TRACE_DEBUG_NAMES", "0")]
        ),
        READERS
    );
    // With it, a change that nothing names selects nothing.
    assert!(affected(r, &["--changed", "README.md"]).is_empty());
    assert!(affected(r, &["--changed", "services/e/src/x.js"]).is_empty());
    // The floor report: no test reaches open uncertainty; the reads are bounded by
    // names.
    let report = ok_env(r, &["floor", "--tests", "jest"], &[]);
    assert!(
        report.contains("uncertainty floor: 0 of 5 entries"),
        "{report}"
    );
    let report = ok_env(
        r,
        &["floor", "--tests", "jest"],
        &[("TURBO_TRACE_DEBUG_NAMES", "0")],
    );
    assert!(
        report.contains("uncertainty floor: 4 of 5 entries"),
        "{report}"
    );
}

#[test]
fn a_change_to_a_named_file_selects_the_tests_that_load_the_name() {
    let d = repo();
    let r = d.path();
    // 'secrets' is a string of services/a/test/paths.js, loaded with the reader by
    // a's test.
    assert_eq!(
        affected(r, &["--changed", "services/a/secrets/dev.token"]),
        ["services/a/test/token.test.js"]
    );
    // The directory of services/c's test is named by its use of __dirname.
    assert_eq!(
        affected(r, &["--changed", "services/c/test/notes.txt"]),
        ["services/c/test/here.test.js"]
    );
    // services/d's route table names handlers/get.sql.
    assert_eq!(
        affected(r, &["--changed", "services/d/handlers/get.sql"]),
        ["services/d/test/table.test.js"]
    );
}

#[test]
fn a_file_added_later_is_named_too() {
    let d = repo();
    let r = d.path();
    ok_env(r, &["graph", "build"], &[]);
    // Not in the graph, and not next to anything the graph knows: only its shape
    // matches.
    write(r, "services/a/secrets/new.token", "n\n");
    assert_eq!(
        affected(
            r,
            &["--changed", "services/a/secrets/new.token", "--no-refresh"]
        ),
        ["services/a/test/token.test.js"]
    );
    write(r, "elsewhere/secrets/deep/x.bin", "n\n");
    assert_eq!(
        affected(
            r,
            &["--changed", "elsewhere/secrets/deep/x.bin", "--no-refresh"]
        ),
        ["services/a/test/token.test.js"]
    );
    // A deleted file is named like any other change.
    std::fs::remove_file(r.join("services/d/handlers/get.sql")).unwrap();
    assert_eq!(
        affected(
            r,
            &["--changed", "services/d/handlers/get.sql", "--no-refresh"]
        ),
        ["services/d/test/table.test.js"]
    );
    // The route table lists handlers/put.sql, which did not exist.
    write(r, "services/d/handlers/put.sql", "select 2\n");
    assert_eq!(
        affected(
            r,
            &["--changed", "services/d/handlers/put.sql", "--no-refresh"]
        ),
        ["services/d/test/table.test.js"]
    );
}

#[test]
fn explain_names_the_string_and_the_read() {
    let d = repo();
    let r = d.path();
    let out = ok_env(
        r,
        &[
            "affected",
            "--changed",
            "services/a/secrets/dev.token",
            "--explain",
            "services/a/test/token.test.js",
        ],
        &[],
    );
    assert!(
        out.contains(
            "affected services/a/test/token.test.js: changed services/a/secrets/dev.token, named \
             by the string 'secrets' in services/a/test/paths.js:3 (loaded with the read at \
             packages/reader/index.js:4, whose path is computed): services/a/test/paths.js → \
             services/a/test/token.test.js"
        ),
        "{out}"
    );
    let out = ok_env(
        r,
        &[
            "affected",
            "--changed",
            "services/d/handlers/get.sql",
            "--explain",
            "services/d/test/table.test.js",
        ],
        &[],
    );
    assert!(
        out.contains("named by the string 'handlers/get.sql' in services/d/routes.json:1"),
        "{out}"
    );
    // An entry no change reaches says how its computed reads are bounded.
    let out = ok_env(
        r,
        &[
            "affected",
            "--changed",
            "README.md",
            "--explain",
            "services/b/test/env.test.js",
        ],
        &[],
    );
    assert!(
        out.contains("bounded by the path-like strings of the code loaded with it"),
        "{out}"
    );
}

#[test]
fn strict_keeps_the_reads_open() {
    let d = repo();
    let r = d.path();
    assert_eq!(
        affected(r, &["--changed", "README.md", "--strict"]),
        READERS
    );
}
