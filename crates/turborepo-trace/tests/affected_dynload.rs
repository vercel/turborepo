//! `affected` end to end for module loads the per-file analysis
//! cannot bound on its own: paths relative to the test runner's working
//! directory in shared helpers.
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
    // The hints file of these tests (`turbo trace` reads none).
    c.args(args)
        .current_dir(root)
        .env("TURBO_TRACE_DEBUG_HINTS", root.join("hints.toml"));
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().expect("run the harness")
}

fn ok_env(root: &Path, args: &[&str], env: &[(&str, &str)]) -> String {
    let o = run_env(root, args, env);
    assert!(
        o.status.success(),
        "harness {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout).unwrap()
}

fn ok(root: &Path, args: &[&str]) -> String {
    ok_env(root, args, &[])
}

fn affected(root: &Path, extra: &[&str]) -> Vec<String> {
    let mut args = vec!["affected", "--tests", "jest"];
    args.extend_from_slice(extra);
    ok(root, &args).lines().map(str::to_owned).collect()
}

/// A monorepo whose services share a test helper that loads `<cwd>/src/routes`,
/// the way a service's endpoint tests load that service's route table:
///
/// ```text
/// packages/helper/setup.js      require(process.cwd() + '/src/routes'), reads <cwd>/fixtures/seed.json
/// services/a/{jest.config.js, src/routes.js → src/handler.js, test/a.test.js, fixtures/seed.json}
/// services/b/{jest.config.js, src/routes.js, test/b.test.js}
/// services/c/{jest.config.js, test/c.test.js}          no routes (yet)
/// services/c/test/plain.test.js                        does not use the helper
/// tools/run.js                                         uses the helper; not a test
/// ```
fn services_repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(
        r,
        "packages/helper/setup.js",
        "const fs = require('fs');\nconst path = require('path');\nexports.loadRoutes = function \
         () {\n  return require(process.cwd() + '/src/routes');\n};\nexports.seed = function () \
         {\n  if (!fs.existsSync(path.join(process.cwd(), 'fixtures', 'seed.json'))) return \
         null;\n  return JSON.parse(fs.readFileSync(path.join(process.cwd(), 'fixtures', \
         'seed.json'), 'utf8'));\n};\n",
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
        write(
            r,
            &format!("services/{s}/test/{s}.test.js"),
            "const { loadRoutes } = require('../../../packages/helper/setup');\ntest('routes', () \
             => { loadRoutes(); });\n",
        );
    }
    write(
        r,
        "services/a/src/routes.js",
        "module.exports = { routes: [require('./handler')] };\n",
    );
    write(
        r,
        "services/a/src/handler.js",
        "module.exports = () => 1;\n",
    );
    write(r, "services/a/fixtures/seed.json", "{}\n");
    write(
        r,
        "services/b/src/routes.js",
        "module.exports = { routes: [] };\n",
    );
    write(
        r,
        "services/c/test/plain.test.js",
        "test('plain', () => {});\n",
    );
    write(
        r,
        "tools/run.js",
        "require('../packages/helper/setup').loadRoutes();\n",
    );
    write(r, "README.md", "x\n");
    d
}

const A: &str = "services/a/test/a.test.js";
const B: &str = "services/b/test/b.test.js";
const C: &str = "services/c/test/c.test.js";

#[test]
fn cwd_relative_loads_resolve_per_test() {
    let d = services_repo();
    let r = d.path();
    // An unrelated change selects nothing: the helper's `cwd_dependent` loads are
    // resolved for every jest test that reaches it.
    assert!(affected(r, &["--changed", "README.md"]).is_empty());
    // Without the per-test resolution (and without the bound that names give the
    // helper's reads) every test that reaches the helper is selected on every
    // change.
    let off: Vec<String> = ok_env(
        r,
        &["affected", "--tests", "jest", "--changed", "README.md"],
        &[
            ("TURBO_TRACE_DEBUG_CWD", "0"),
            ("TURBO_TRACE_DEBUG_NAMES", "0"),
        ],
    )
    .lines()
    .map(str::to_owned)
    .collect();
    assert_eq!(off, [A, B, C]);

    // A service's routes, and what they import, select that service's test only.
    assert_eq!(affected(r, &["--changed", "services/a/src/routes.js"]), [A]);
    assert_eq!(
        affected(r, &["--changed", "services/a/src/handler.js"]),
        [A]
    );
    assert_eq!(affected(r, &["--changed", "services/b/src/routes.js"]), [B]);
    // The file read below the working directory.
    assert_eq!(
        affected(r, &["--changed", "services/a/fixtures/seed.json"]),
        [A]
    );
    // The helper itself selects every test that uses it.
    assert_eq!(
        affected(r, &["--changed", "packages/helper/setup.js"]),
        [A, B, C]
    );

    // The witness names the working directory and the helper's line.
    let o = ok(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--changed",
            "services/a/src/handler.js",
            "--format",
            "json",
        ],
    );
    let v: serde_json::Value = serde_json::from_str(&o).unwrap();
    let res = &v["results"][0];
    assert_eq!(res["entry"], A);
    let witness: Vec<&str> = res["witness"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w.as_str().unwrap())
        .collect();
    assert_eq!(
        witness,
        ["services/a/src/handler.js", "services/a/src/routes.js", A]
    );

    // An entry that is not a test has no known working directory: still uncertain.
    let o = ok(
        r,
        &[
            "affected",
            "tools/run.js",
            A,
            "--changed",
            "README.md",
            "--format",
            "json",
        ],
    );
    let v: serde_json::Value = serde_json::from_str(&o).unwrap();
    let results = v["results"].as_array().unwrap();
    assert_eq!(results.len(), 1, "{o}");
    assert_eq!(results[0]["entry"], "tools/run.js");
    assert!(
        o.contains("cwd_dependent")
            && o.contains("resolved per test from the test runner's working directory"),
        "{o}"
    );
}

#[test]
fn cwd_targets_that_appear_or_disappear_select_the_test() {
    let d = services_repo();
    let r = d.path();
    ok(r, &["graph", "build"]);
    // Service c has no routes. Adding them (in any module form) selects its helper
    // test, matched through the probes recorded on the test; `plain.test.js` is
    // not selected.
    for added in [
        "services/c/src/routes.js",
        "services/c/src/routes.ts",
        "services/c/src/routes/index.js",
        "services/c/fixtures/seed.json",
    ] {
        write(r, "changes.txt", &format!("A\t{added}\n"));
        assert_eq!(
            affected(r, &["--no-refresh", "--changed-from", "changes.txt"]),
            [C],
            "{added}"
        );
    }
    write(r, "changes.txt", "A\tservices/c/src/other.js\n");
    assert!(affected(r, &["--no-refresh", "--changed-from", "changes.txt"]).is_empty());

    // With a refreshed graph the new file is a node and an edge of the test.
    write(
        r,
        "services/c/src/routes.js",
        "module.exports = { routes: [] };\n",
    );
    assert_eq!(affected(r, &["--changed", "services/c/src/routes.js"]), [C]);
    // Deleting a service's routes selects its test (the graph is refreshed first:
    // the test then probes the missing path).
    std::fs::remove_file(r.join("services/b/src/routes.js")).unwrap();
    assert_eq!(affected(r, &["--changed", "services/b/src/routes.js"]), [B]);
}

/// A helper that loads `<cwd>/src/<name>`: a pattern below the working
/// directory, in two services. From the repository root the pattern matches
/// nothing, which says nothing about the directory a test runs in. (The record
/// used to become "no files match", which counts as covered: a test was not
/// selected for a change of the file it loads.) The pattern is resolved per
/// test like a literal path: each test depends on the modules below the `src`
/// of its own working directory.
fn pattern_repo(helper: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(r, "packages/helper/load.js", helper);
    for s in ["a", "b"] {
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
        write(
            r,
            &format!("services/{s}/src/handler.js"),
            "module.exports = require('./dep');\n",
        );
        write(
            r,
            &format!("services/{s}/src/dep.js"),
            "module.exports = () => 1;\n",
        );
        write(
            r,
            &format!("services/{s}/test/{s}.test.js"),
            "const { load } = require('../../../packages/helper/load');\nconst which = \
             ['handler'];\ntest('loads', () => { load(which[0]); });\n",
        );
    }
    write(
        r,
        "services/a/test/plain.test.js",
        "test('plain', () => {});\n",
    );
    write(r, "services/a/lib/unrelated.js", "module.exports = 0;\n");
    write(r, "README.md", "x\n");
    d
}

#[test]
fn a_pattern_below_the_working_directory_is_resolved_per_test() {
    let d = pattern_repo(
        "const path = require('path');\nexports.load = (name) => require(path.join(process.cwd(), \
         'src', name));\n",
    );
    let r = d.path();
    // The file a test loads, and what that file imports, select the test of that
    // service.
    assert_eq!(
        affected(r, &["--changed", "services/a/src/handler.js"]),
        [A]
    );
    assert_eq!(affected(r, &["--changed", "services/a/src/dep.js"]), [A]);
    assert_eq!(
        affected(r, &["--changed", "services/b/src/handler.js"]),
        [B]
    );
    // Nothing else does: the load is bounded, not open.
    assert!(affected(r, &["--changed", "README.md"]).is_empty());
    assert!(affected(r, &["--changed", "services/a/lib/unrelated.js"]).is_empty());
    // A module added below the directory can be the one that is loaded.
    write(r, "changes.txt", "A\tservices/b/src/added.js\n");
    assert_eq!(
        affected(r, &["--no-refresh", "--changed-from", "changes.txt"]),
        [B]
    );
    let o = ok(r, &["floor", "--tests", "jest"]);
    assert!(o.contains("0 of 3 entries"), "{o}");
}

#[test]
fn a_pattern_over_the_working_directory_itself_stays_open() {
    // `<cwd>/<name>` names no directory below the working directory: every test
    // that reaches the helper is selected on every change.
    let d = pattern_repo(
        "const path = require('path');\nexports.load = (name) => require(path.join(process.cwd(), \
         name));\n",
    );
    let r = d.path();
    assert_eq!(affected(r, &["--changed", "README.md"]), [A, B]);
    let o = ok(r, &["floor", "--tests", "jest"]);
    assert!(
        o.contains("2 of 3 entries") && o.contains("packages/helper/load.js"),
        "{o}"
    );
}

/// The real chain behind a framework that loads route tables and imports the
/// handler modules they name by path, with the two sites the analysis cannot
/// bound declared in the hints file:
///
/// ```text
/// packages/framework/normalize.js   import(spec)               (spec: a handler path from a route table)
/// packages/helper/setup.js          for (const p of paths) require(p)   (<cwd>/src/routes, collected in a Set)
/// services/a/src/routes.js          { handler: 'services/a/src/handlers/get' }
/// services/a/src/handlers/get.js → ../lib.js
/// services/b/…                      likewise
/// ```
fn framework_repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(
        r,
        "packages/framework/normalize.js",
        "exports.normalize = function ({ spec }) {\n  return { handler: () => import(spec), \
         modulePath: spec };\n};\n",
    );
    write(
        r,
        "packages/helper/setup.js",
        "const { normalize } = require('../framework/normalize');\nexports.setupEndpoint = \
         function () {\n  const paths = new Set();\n  paths.add(process.cwd() + '/src/routes');\n  \
         const out = [];\n  for (const p of paths) {\n    const loaded = require(p);\n    for \
         (const route of loaded.routes) out.push(normalize({ spec: route.handler }));\n  }\n  \
         return out;\n};\n",
    );
    for s in ["a", "b"] {
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
        write(
            r,
            &format!("services/{s}/src/routes.js"),
            &format!(
                "module.exports = {{\n  routes: [\n    {{ method: 'GET', path: '/v1/x', handler: \
                 'services/{s}/src/handlers/get' }},\n  ],\n}};\n"
            ),
        );
        write(
            r,
            &format!("services/{s}/src/handlers/get.js"),
            "module.exports = require('../lib');\n",
        );
        write(
            r,
            &format!("services/{s}/src/lib.js"),
            "module.exports = 1;\n",
        );
        write(
            r,
            &format!("services/{s}/test/{s}.test.js"),
            "const { setupEndpoint } = \
             require('../../../packages/helper/setup');\ntest('endpoint', () => { \
             setupEndpoint(); });\n",
        );
    }
    write(
        r,
        "tools/run.js",
        "require('../packages/helper/setup').setupEndpoint();\n",
    );
    write(r, "README.md", "x\n");
    d
}

/// The two sites declared, without `depends-on`: the handler paths in the route
/// tables are module strings, which the tracer turns into edges by itself.
const SITE_HINTS: &str = r#"
[[hint]]
files = "packages/helper/setup.js"
kinds = ["computed_specifier"]
cwd-loads = ["./src/routes"]

[[hint]]
files = "packages/framework/normalize.js"
loads = "module-strings"
"#;

#[test]
fn module_strings_link_route_tables_to_handlers() {
    let d = framework_repo();
    let r = d.path();
    // A route table depends on the handlers it names, with no hint.
    let deps = ok(r, &["deps", "services/a/src/routes.js"]);
    assert!(
        deps.lines().any(|l| l == "services/a/src/handlers/get.js")
            && deps.lines().any(|l| l == "services/a/src/lib.js"),
        "{deps}"
    );
    assert!(!deps.contains("services/b/"), "{deps}");

    write(r, "hints.toml", SITE_HINTS);
    assert!(affected(r, &["--changed", "README.md"]).is_empty());
    assert_eq!(
        affected(r, &["--changed", "services/a/src/handlers/get.js"]),
        [A]
    );
    assert_eq!(affected(r, &["--changed", "services/b/src/lib.js"]), [B]);

    // A table can name a handler before its file exists: adding the file (in any
    // module form) selects the tests that load the table.
    write(
        r,
        "services/b/src/routes.js",
        "module.exports = {\n  routes: [\n    { handler: 'services/b/src/handlers/get' },\n    { \
         handler: 'services/b/src/handlers/post' },\n  ],\n};\n",
    );
    ok(r, &["graph", "build"]);
    for added in [
        "services/b/src/handlers/post.js",
        "services/b/src/handlers/post.ts",
    ] {
        write(r, "changes.txt", &format!("A\t{added}\n"));
        assert_eq!(
            affected(r, &["--no-refresh", "--changed-from", "changes.txt"]),
            [B],
            "{added}"
        );
    }
    write(r, "changes.txt", "A\tservices/b/src/handlers/other.js\n");
    assert!(affected(r, &["--no-refresh", "--changed-from", "changes.txt"]).is_empty());
}

const HINTS: &str = r#"
# The helper requires <working directory>/src/routes for every path it collected.
[[hint]]
files = "packages/helper/setup.js"
kinds = ["computed_specifier"]
cwd-loads = ["./src/routes"]

# The framework imports the handler modules that route tables name.
[[hint]]
files = "packages/framework/normalize.js"
loads = "module-strings"

[[hint]]
files = "services/*/src/routes.js"
depends-on = ["./handlers/**"]
"#;

#[test]
fn hints_bound_dynamic_loads() {
    let d = framework_repo();
    let r = d.path();
    // Without hints both tests reach two open module loads: selected on every
    // change.
    assert_eq!(affected(r, &["--changed", "README.md"]), [A, B]);

    write(r, "hints.toml", HINTS);
    assert!(affected(r, &["--changed", "README.md"]).is_empty());
    // test → helper → <cwd>/src/routes → the handlers it names → what they import.
    assert_eq!(affected(r, &["--changed", "services/a/src/routes.js"]), [A]);
    assert_eq!(
        affected(r, &["--changed", "services/a/src/handlers/get.js"]),
        [A]
    );
    assert_eq!(affected(r, &["--changed", "services/b/src/lib.js"]), [B]);
    assert_eq!(
        affected(r, &["--changed", "packages/framework/normalize.js"]),
        [A, B]
    );
    // A handler added to a service's directory (the hint's membership).
    ok(r, &["graph", "build"]);
    write(r, "changes.txt", "A\tservices/b/src/handlers/post.js\n");
    assert_eq!(
        affected(r, &["--no-refresh", "--changed-from", "changes.txt"]),
        [B]
    );
    // An entry that is not a test: the helper's load has no working directory to
    // resolve against, so it stays uncertain; the framework's site is bounded
    // for everyone.
    let o = ok(
        r,
        &[
            "affected",
            "tools/run.js",
            "--changed",
            "README.md",
            "--format",
            "json",
        ],
    );
    let v: serde_json::Value = serde_json::from_str(&o).unwrap();
    assert_eq!(v["results"][0]["entry"], "tools/run.js");
    assert_eq!(
        v["results"][0]["uncertain"]["file"],
        "packages/helper/setup.js"
    );

    // The hints file can live elsewhere (replaying commits that predate it), or be
    // off.
    std::fs::rename(r.join("hints.toml"), r.join("elsewhere.toml")).unwrap();
    let elsewhere = r.join("elsewhere.toml");
    let env = [("TURBO_TRACE_DEBUG_HINTS", elsewhere.to_str().unwrap())];
    let args = ["affected", "--tests", "jest", "--changed", "README.md"];
    assert_eq!(ok_env(r, &args, &env), "");
    assert_eq!(
        ok_env(r, &args, &[("TURBO_TRACE_DEBUG_HINTS", "")])
            .lines()
            .collect::<Vec<_>>(),
        [A, B]
    );
    // A malformed hints file is an error, not silently ignored.
    write(r, "hints.toml", "[[hint]]\nfiles = \"x\"\nload = []\n");
    let o = run_env(r, &args, &[]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("hints.toml"));
}
