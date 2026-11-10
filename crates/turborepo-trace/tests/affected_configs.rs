//! `turbotrace affected --since`: a changed `package.json` is applied field by
//! field. Fields that no resolution reads (`version`, `description`, ...) and
//! scripts select only the readers of the file and the tasks of the changed
//! scripts; resolution fields (`exports`, `main`, `type`, `imports`, `browser`)
//! select the package's files and its importers, as a whole-file change does.
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

fn git(root: &Path, args: &[&str]) {
    let o = Command::new("git")
        .args(["-c", "user.email=t@t", "-c", "user.name=t"])
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
}

const A_TEST: &str = "packages/a/test/a.test.js";
const VER_TEST: &str = "packages/a/test/ver.test.js";
const NAME_TEST: &str = "packages/a/test/name.test.js";
const B_TEST: &str = "packages/b/test/b.test.js";
const DIST_TEST: &str = "packages/b/test/dist.test.js";
const ENTRIES: [&str; 5] = [A_TEST, VER_TEST, NAME_TEST, B_TEST, DIST_TEST];
/// Every entry, in output order.
const ALL: [&str; 5] = [A_TEST, NAME_TEST, VER_TEST, B_TEST, DIST_TEST];

const A_MANIFEST: &str = r#"{
  "name": "@acme/a",
  "version": "1.0.0",
  "description": "a",
  "main": "./src/index.js",
  "exports": { ".": "./src/index.js", "./util": "./src/util.js" },
  "scripts": { "build": "acme-bundle", "gen": "acme-gen" },
  "dependencies": { "left-pad": "^1.0.0" }
}
"#;

const B_MANIFEST: &str = r#"{
  "name": "@acme/b",
  "version": "1.0.0",
  "main": "src/index.js",
  "scripts": { "build": "acme-bundle --out dist", "lint": "acme-lint", "stamp": "acme-stamp $npm_package_version" },
  "dependencies": { "@acme/a": "workspace:*" }
}
"#;

/// A workspace: `@acme/a` (`src/index.js` requires `./util`; `src/ver.js`
/// requires the manifest, `src/name.js` reads it through `fs`; one test for
/// each) and `@acme/b`, which requires `@acme/a` through its `node_modules`
/// link and whose `build` task (a command turbotrace doesn't know, outputs
/// `dist/**`) produces the file `dist.test.js` loads.
fn repo(a_manifest: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"root","private":true}"#);
    write(r, "pnpm-workspace.yaml", "packages:\n  - 'packages/*'\n");
    write(r, ".gitignore", ".turbo/trace/\ndist/\n");
    write(
        r,
        "turbo.json",
        r#"{"tasks":{"build":{"outputs":["dist/**"]}}}"#,
    );
    write(r, "packages/a/package.json", a_manifest);
    write(
        r,
        "packages/a/src/index.js",
        "const u = require('./util');\nmodule.exports = { a: () => u.u() };\n",
    );
    write(
        r,
        "packages/a/src/util.js",
        "module.exports = { u: () => 1 };\n",
    );
    write(
        r,
        "packages/a/src/ver.js",
        "module.exports = require('../package.json').version;\n",
    );
    write(
        r,
        "packages/a/src/name.js",
        "const fs = require('fs');\nconst path = require('path');\nmodule.exports = \
         JSON.parse(fs.readFileSync(path.join(__dirname, '../package.json'), 'utf8')).name;\n",
    );
    write(
        r,
        A_TEST,
        "const a = require('../src/index');\ntest('a', () => expect(a.a()).toBe(1));\n",
    );
    write(
        r,
        VER_TEST,
        "const v = require('../src/ver');\ntest('v', () => expect(v).toBe('1.0.0'));\n",
    );
    write(
        r,
        NAME_TEST,
        "const n = require('../src/name');\ntest('n', () => expect(n).toBe('@acme/a'));\n",
    );
    write(r, "packages/b/package.json", B_MANIFEST);
    write(
        r,
        "packages/b/src/index.js",
        "const a = require('@acme/a');\nmodule.exports = { b: () => a.a() + 1 };\n",
    );
    write(
        r,
        B_TEST,
        "const b = require('../src/index');\ntest('b', () => expect(b.b()).toBe(2));\n",
    );
    write(
        r,
        DIST_TEST,
        "const out = require('../dist/out.js');\ntest('d', () => expect(out).toBeTruthy());\n",
    );
    std::fs::create_dir_all(r.join("packages/b/node_modules/@acme")).unwrap();
    #[cfg(unix)]
    turborepo_trace_core::os::symlink("../../../a", r.join("packages/b/node_modules/@acme/a"))
        .unwrap();
    git(r, &["init", "-q"]);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "base"]);
    d
}

/// The affected entries among [`ENTRIES`] for the working tree against `HEAD`,
/// ignoring uncertainty.
fn since_env(r: &Path, env: &[(&str, &str)]) -> Vec<String> {
    let mut args = vec!["affected", "--since", "HEAD", "--on-uncertain", "ignore"];
    args.extend(ENTRIES);
    ok_env(r, &args, env).lines().map(str::to_owned).collect()
}

fn since(r: &Path) -> Vec<String> {
    since_env(r, &[])
}

fn tasks(r: &Path) -> Vec<String> {
    ok_env(
        r,
        &[
            "affected",
            "--since",
            "HEAD",
            "--on-uncertain",
            "ignore",
            "--tasks",
            "**",
        ],
        &[],
    )
    .lines()
    .map(str::to_owned)
    .collect()
}

fn edit_a(r: &Path, from: &str, to: &str) {
    assert!(A_MANIFEST.contains(from), "{from}");
    write(r, "packages/a/package.json", &A_MANIFEST.replace(from, to));
}

const OFF: [(&str, &str); 1] = [("TURBO_TRACE_DEBUG_CONFIG_FIELDS", "0")];

#[test]
fn inert_fields_select_only_readers_of_the_file() {
    let d = repo(A_MANIFEST);
    let r = d.path();
    // As a whole-file change, the manifest selects every test of the package and of
    // its importer (and, through the importer, what the importer's build
    // produces).
    edit_a(r, r#""version": "1.0.0""#, r#""version": "1.0.1""#);
    assert_eq!(since_env(r, &OFF), ALL);
    // Field by field: `version` selects the tests that load or read the file
    // itself.
    assert_eq!(since(r), [NAME_TEST, VER_TEST]);
    // The package's tasks read its whole directory, the two readers of the manifest
    // included, so they are affected; the importer's tasks are not.
    let t = tasks(r);
    assert!(
        !t.is_empty() && t.iter().all(|t| t.starts_with("task:packages/a#")),
        "{t:?}"
    );
    // `description`, `files`, `license`, a formatting-only change: the same
    // readers.
    edit_a(
        r,
        r#""description": "a""#,
        r#""description": "A", "files": ["src"], "license": "MIT""#,
    );
    assert_eq!(since(r), [NAME_TEST, VER_TEST]);
    write(
        r,
        "packages/a/package.json",
        &A_MANIFEST.replace("\n  ", "\n    "),
    );
    assert_eq!(since(r), [NAME_TEST, VER_TEST]);

    // Reported with the changed fields.
    edit_a(r, r#""version": "1.0.0""#, r#""version": "2.0.0""#);
    let mut args = vec![
        "affected",
        "--since",
        "HEAD",
        "--on-uncertain",
        "ignore",
        "--format",
        "json",
    ];
    args.extend(ENTRIES);
    let v: serde_json::Value = serde_json::from_str(&ok_env(r, &args, &[])).unwrap();
    let c = &v["configs"][0];
    assert_eq!(c["path"], "packages/a/package.json");
    assert_eq!(c["field_level"], true);
    assert_eq!(c["fields"], serde_json::json!(["version"]));
    assert_eq!(c["readers"], 2);
    assert_eq!(v["changes"], serde_json::json!([]));
    let explain = ok_env(
        r,
        &[
            "affected",
            "--since",
            "HEAD",
            "--on-uncertain",
            "ignore",
            "--explain",
            VER_TEST,
            "--explain",
            A_TEST,
        ],
        &[],
    );
    assert!(
        explain.contains(&format!(
            "affected {VER_TEST}: modified packages/a/package.json (read or probed) at \
             packages/a/src/ver.js → {VER_TEST}"
        )),
        "{explain}"
    );
    assert!(
        explain.contains(&format!("unaffected {A_TEST}")),
        "{explain}"
    );
    assert!(
        explain.contains(
            "note: config packages/a/package.json (version): no changed field is read by \
             resolution"
        ),
        "{explain}"
    );

    // Without a base revision there is no old version: a whole-file change.
    let mut args = vec![
        "affected",
        "--changed",
        "packages/a/package.json",
        "--on-uncertain",
        "ignore",
    ];
    args.extend(ENTRIES);
    let all: Vec<String> = ok_env(r, &args, &[]).lines().map(str::to_owned).collect();
    assert_eq!(all, ALL);
    git(r, &["checkout", "-q", "--", "."]);

    // A package without readers of its manifest: a `version` bump selects no test,
    // and only the task whose script names `$npm_package_version`.
    write(
        r,
        "packages/b/package.json",
        &B_MANIFEST.replace(r#""version": "1.0.0""#, r#""version": "1.1.0""#),
    );
    assert!(since(r).is_empty());
    assert_eq!(tasks(r), ["task:packages/b#stamp"]);
    assert_eq!(since_env(r, &OFF), [B_TEST, DIST_TEST]);
}

#[test]
fn other_fields_select_the_package_and_its_importers() {
    let d = repo(A_MANIFEST);
    let r = d.path();
    for (from, to) in [
        (
            r#""description": "a""#,
            r#""description": "a", "type": "commonjs""#,
        ),
        (
            r#""description": "a""#,
            r#""description": "a", "browser": {"./src/util.js": false}"#,
        ),
        (r#""name": "@acme/a""#, r#""name": "@acme/a2""#),
        (
            r#""description": "a""#,
            r#""description": "a", "somethingNew": true"#,
        ),
        // Mixed with an inert field.
        (
            r#""version": "1.0.0",
  "description": "a""#,
            r#""version": "1.0.1",
  "description": "a", "sideEffects": false"#,
        ),
    ] {
        edit_a(r, from, to);
        assert_eq!(since(r), ALL, "{to}");
    }
    // Resolution fields are applied entry by entry (tests/affected_exports.rs): an
    // added subpath that nobody requests selects only the readers of the file.
    edit_a(
        r,
        r#""./util": "./src/util.js""#,
        r#""./util": "./src/util.js", "./x": "./src/x.js""#,
    );
    assert_eq!(since(r), [NAME_TEST, VER_TEST]);
}

/// The affected entries among `entries`, ignoring uncertainty.
fn since_of(r: &Path, entries: &[&str], env: &[(&str, &str)]) -> Vec<String> {
    let mut args = vec!["affected", "--since", "HEAD", "--on-uncertain", "ignore"];
    args.extend(entries);
    ok_env(r, &args, env).lines().map(str::to_owned).collect()
}

#[test]
fn dependency_edits_select_the_nodes_that_link_the_dependency() {
    const USES_X: &str = "packages/c/test/uses-x.test.js";
    const USES_A: &str = "packages/c/test/uses-a.test.js";
    const OTHER: &str = "packages/c/test/other.test.js";
    const C_TESTS: [&str; 3] = [USES_X, USES_A, OTHER];
    const C_MANIFEST: &str = r#"{
  "name": "@acme/c",
  "version": "1.0.0",
  "dependencies": { "@acme/a": "workspace:*", "dep-x": "^1.0.0", "preset-y": "^1.0.0" }
}
"#;
    let d = repo(A_MANIFEST);
    let r = d.path();
    // `@acme/c`: one file requires the installed third-party `dep-x`, one the
    // workspace package `@acme/a`, one neither; `preset-y` is a dependency no
    // file names.
    write(r, "packages/c/package.json", C_MANIFEST);
    write(
        r,
        "packages/c/node_modules/dep-x/package.json",
        r#"{"name":"dep-x","version":"1.0.0","main":"index.js"}"#,
    );
    write(
        r,
        "packages/c/node_modules/dep-x/index.js",
        "module.exports = 1;\n",
    );
    std::fs::create_dir_all(r.join("packages/c/node_modules/@acme")).unwrap();
    #[cfg(unix)]
    turborepo_trace_core::os::symlink("../../../a", r.join("packages/c/node_modules/@acme/a"))
        .unwrap();
    write(
        r,
        "packages/c/src/x.js",
        "module.exports = require('dep-x');\n",
    );
    write(
        r,
        "packages/c/src/a.js",
        "module.exports = require('@acme/a');\n",
    );
    write(r, "packages/c/src/other.js", "module.exports = 2;\n");
    write(r, USES_X, "require('../src/x');\ntest('x', () => {});\n");
    write(r, USES_A, "require('../src/a');\ntest('a', () => {});\n");
    write(r, OTHER, "require('../src/other');\ntest('o', () => {});\n");
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "c"]);
    let edit_c = |from: &str, to: &str| {
        assert!(C_MANIFEST.contains(from), "{from}");
        write(r, "packages/c/package.json", &C_MANIFEST.replace(from, to));
    };

    // A third-party range edit: the file that links the package, not the others.
    edit_c(r#""dep-x": "^1.0.0""#, r#""dep-x": "^1.1.0""#);
    assert_eq!(since_of(r, &C_TESTS, &OFF), [OTHER, USES_A, USES_X]);
    assert_eq!(since_of(r, &C_TESTS, &[]), [USES_X]);
    // Removed while still required (and still installed here).
    edit_c(r#", "dep-x": "^1.0.0""#, "");
    assert_eq!(since_of(r, &C_TESTS, &[]), [USES_X]);
    // Moved to another map.
    edit_c(
        r#""dep-x": "^1.0.0", "preset-y": "^1.0.0" }"#,
        r#""preset-y": "^1.0.0" }, "devDependencies": { "dep-x": "^1.0.0" }"#,
    );
    assert_eq!(since_of(r, &C_TESTS, &[]), [USES_X]);
    // A workspace dependency: the file that links the workspace package.
    edit_c(r#""@acme/a": "workspace:*""#, r#""@acme/a": "workspace:^""#);
    assert_eq!(since_of(r, &C_TESTS, &[]), [USES_A]);
    // Both, and an inert field.
    edit_c(
        r#""version": "1.0.0",
  "dependencies": { "@acme/a": "workspace:*", "dep-x": "^1.0.0""#,
        r#""version": "1.0.1",
  "dependencies": { "@acme/a": "workspace:^", "dep-x": "^2.0.0""#,
    );
    assert_eq!(since_of(r, &C_TESTS, &[]), [USES_A, USES_X]);
    // Kept as a whole-file change on request.
    assert_eq!(
        since_of(
            r,
            &C_TESTS,
            &[("TURBO_TRACE_DEBUG_CONFIG_FIELDS", "nodeps")]
        ),
        [OTHER, USES_A, USES_X]
    );

    // A dependency that nothing in the package links or names, edited without a
    // lockfile change: nothing says what an install would do, so the whole file
    // counts. The same for an added one. (With a lockfile diff:
    // tests/affected_deps.rs.)
    edit_c(r#""preset-y": "^1.0.0""#, r#""preset-y": "^1.2.0""#);
    assert_eq!(since_of(r, &C_TESTS, &[]), [OTHER, USES_A, USES_X]);
    edit_c(
        r#""preset-y": "^1.0.0""#,
        r#""preset-y": "^1.0.0", "plugin-z": "1.0.0""#,
    );
    assert_eq!(since_of(r, &C_TESTS, &[]), [OTHER, USES_A, USES_X]);
    let o = run_env(
        r,
        &[
            "affected",
            "--since",
            "HEAD",
            "--on-uncertain",
            "ignore",
            "--stats",
            OTHER,
        ],
        &[],
    );
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.contains(
            "config packages/c/package.json (dependencies.plugin-z): whole-file change: changed \
             dependenc(ies) whose consumers are not known: plugin-z"
        ),
        "{err}"
    );

    // A dependency that was required but not installed, now removed from the
    // manifest: the file that fails to resolve it probed its location.
    write(
        r,
        "packages/c/src/x.js",
        "module.exports = require('gone-w');\n",
    );
    write(
        r,
        "packages/c/package.json",
        &C_MANIFEST.replace(
            r#""preset-y": "^1.0.0""#,
            r#""preset-y": "^1.0.0", "gone-w": "1.0.0""#,
        ),
    );
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "gone-w"]);
    write(r, "packages/c/package.json", C_MANIFEST);
    assert_eq!(since_of(r, &C_TESTS, &[]), [USES_X]);

    // The first package of the fixture: `left-pad` is named by no file.
    git(r, &["checkout", "-q", "--", "."]);
    edit_a(r, r#""left-pad": "^1.0.0""#, r#""left-pad": "^1.3.0""#);
    let got = since(r);
    for t in [A_TEST, NAME_TEST, VER_TEST] {
        assert!(got.iter().any(|g| g == t), "{t} in {got:?}");
    }
}

#[test]
fn script_edits_select_their_tasks() {
    let d = repo(A_MANIFEST);
    let r = d.path();
    // An edited script: its task, and no test.
    write(
        r,
        "packages/b/package.json",
        &B_MANIFEST.replace(r#""lint": "acme-lint""#, r#""lint": "acme-lint --fix""#),
    );
    assert_eq!(tasks(r), ["task:packages/b#lint"]);
    assert!(since(r).is_empty());
    // In a package with readers of the manifest, those tests too.
    edit_a(r, r#""gen": "acme-gen""#, r#""gen": "acme-gen --all""#);
    assert_eq!(since(r), [NAME_TEST, VER_TEST]);
    git(r, &["checkout", "-q", "--", "."]);

    // The script of a task whose output a test loads: the task and that test, not
    // the tests that import the package's sources. (The package's other tasks
    // read its whole directory, the build output included, so they follow the
    // build task.)
    write(
        r,
        "packages/b/package.json",
        &B_MANIFEST.replace("acme-bundle --out dist", "acme-bundle --minify --out dist"),
    );
    assert!(tasks(r).iter().any(|t| t == "task:packages/b#build"));
    assert_eq!(since(r), [DIST_TEST]);
    assert_eq!(since_env(r, &OFF), [B_TEST, DIST_TEST]);
    // An added script: its new task.
    write(
        r,
        "packages/b/package.json",
        &B_MANIFEST.replace(
            r#""lint": "acme-lint""#,
            r#""lint": "acme-lint", "fmt": "acme-fmt""#,
        ),
    );
    assert_eq!(tasks(r), ["task:packages/b#fmt"]);
    assert!(since(r).is_empty());
    // A removed script: every task that read the manifest (what ran it no longer
    // finds it).
    write(
        r,
        "packages/b/package.json",
        &B_MANIFEST.replace(r#", "lint": "acme-lint""#, ""),
    );
    let t = tasks(r);
    assert!(t.iter().any(|t| t == "task:packages/b#build"), "{t:?}");
    assert_eq!(since(r), [DIST_TEST]);
}

#[test]
fn version_reaches_code_that_reads_it_from_the_environment() {
    const ENV_TEST: &str = "packages/b/test/env.test.js";
    let d = repo(A_MANIFEST);
    let r = d.path();
    // A package manager exports the package's version to its scripts as
    // `npm_package_version`: code that names the variable reads the field.
    write(
        r,
        "packages/b/src/env.js",
        "module.exports = process.env.npm_package_version;\n",
    );
    write(
        r,
        ENV_TEST,
        "const v = require('../src/env');\ntest('v', () => expect(v).toBeDefined());\n",
    );
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "env"]);
    write(
        r,
        "packages/b/package.json",
        &B_MANIFEST.replace(r#""version": "1.0.0""#, r#""version": "1.1.0""#),
    );
    assert_eq!(since_of(r, &[ENV_TEST, B_TEST], &[]), [ENV_TEST]);
    // Another inert field does not.
    write(
        r,
        "packages/b/package.json",
        &B_MANIFEST.replace(
            r#""version": "1.0.0""#,
            r#""version": "1.0.0", "license": "MIT""#,
        ),
    );
    assert!(since_of(r, &[ENV_TEST, B_TEST], &[]).is_empty());
}

#[test]
fn imports_a_module_mapper_rewrites_do_not_read_the_manifest() {
    const A: &str = "apps/site/test/a.test.js";
    const X: &str = "apps/site/test/x.test.js";
    const STYLED: &str = "apps/site/test/styled.test.js";
    const NAME: &str = "apps/site/test/name.test.js";
    const TESTS: [&str; 4] = [A, X, STYLED, NAME];
    const MANIFEST: &str = r#"{
  "name": "site",
  "version": "1.0.0",
  "dependencies": { "dep-x": "^1.0.0", "dep-y": "^1.0.0" },
  "devDependencies": { "identity-obj-proxy": "^3.0.0", "jest": "^29.0.0" }
}
"#;
    // An app whose jest config maps `#/x` to `<rootDir>/x` and CSS modules to a
    // package. Nothing is installed: `dep-x` and `identity-obj-proxy` do not
    // resolve.
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(
        r,
        "package.json",
        r#"{"name":"root","private":true,"workspaces":["apps/*"]}"#,
    );
    write(r, ".gitignore", ".turbo/trace/\n");
    write(r, "apps/site/package.json", MANIFEST);
    write(
        r,
        "apps/site/jest.config.js",
        "module.exports = {\n  moduleNameMapper: {\n    '^#/(.*)$': '<rootDir>/$1',\n    \
         '\\\\.module\\\\.css$': 'identity-obj-proxy',\n  },\n};\n",
    );
    write(r, "apps/site/lib/a.js", "module.exports = 1;\n");
    write(
        r,
        "apps/site/lib/uses-x.js",
        "module.exports = require('dep-x');\n",
    );
    write(
        r,
        "apps/site/lib/styled.js",
        "module.exports = require('./styled.module.css');\n",
    );
    write(r, "apps/site/lib/styled.module.css", ".a { color: red; }\n");
    write(
        r,
        "apps/site/lib/name.js",
        "module.exports = require('#/package.json').name;\n",
    );
    write(r, A, "require('#/lib/a');\ntest('a', () => {});\n");
    write(r, X, "require('#/lib/uses-x');\ntest('x', () => {});\n");
    write(
        r,
        STYLED,
        "require('#/lib/styled');\ntest('styled', () => {});\n",
    );
    write(r, NAME, "require('#/lib/name');\ntest('name', () => {});\n");
    git(r, &["init", "-q"]);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "base"]);
    let edit = |from: &str, to: &str| {
        assert!(MANIFEST.contains(from), "{from}");
        write(r, "apps/site/package.json", &MANIFEST.replace(from, to));
    };

    // One dependency changes. Every test reaches the manifest through its mapped
    // imports (their resolution asks it for the package's module type); only
    // the file that uses the dependency consumes it. The file that loads the
    // manifest through the mapper reads it.
    edit(r#""dep-x": "^1.0.0""#, r#""dep-x": "^1.1.0""#);
    assert_eq!(since_of(r, &TESTS, &OFF), [A, NAME, STYLED, X]);
    assert_eq!(since_of(r, &TESTS, &[]), [NAME, X]);
    // The package the jest config rewrites CSS modules to: the config names it, so
    // every test of the config may use it.
    edit(
        r#""identity-obj-proxy": "^3.0.0""#,
        r#""identity-obj-proxy": "^3.0.1""#,
    );
    assert_eq!(since_of(r, &TESTS, &[]), [A, NAME, STYLED, X]);
    // A field nothing resolves with: the reader of the file.
    edit(r#""version": "1.0.0""#, r#""version": "1.0.1""#);
    assert_eq!(since_of(r, &TESTS, &[]), [NAME]);
    // The module type is what the mapped imports asked the manifest for:
    // everything.
    edit(
        r#""version": "1.0.0","#,
        r#""version": "1.0.0", "type": "module","#,
    );
    assert_eq!(since_of(r, &TESTS, &[]), [A, NAME, STYLED, X]);
}

#[test]
fn unparsable_manifests_are_whole_file_changes() {
    // The new side does not parse.
    let d = repo(A_MANIFEST);
    let r = d.path();
    write(
        r,
        "packages/a/package.json",
        &A_MANIFEST.replace("\"version\": \"1.0.0\",", "\"version\": "),
    );
    let got = since(r);
    for t in [A_TEST, NAME_TEST, VER_TEST] {
        assert!(got.iter().any(|g| g == t), "{t} in {got:?}");
    }
    // The old side does not parse.
    let d = repo(&A_MANIFEST.replace("\"version\": \"1.0.0\",", "\"version\": "));
    let r = d.path();
    write(r, "packages/a/package.json", A_MANIFEST);
    let got = since(r);
    for t in [A_TEST, NAME_TEST, VER_TEST, B_TEST] {
        assert!(got.iter().any(|g| g == t), "{t} in {got:?}");
    }
    let o = run_env(
        r,
        &[
            "affected",
            "--since",
            "HEAD",
            "--on-uncertain",
            "ignore",
            "--stats",
            A_TEST,
        ],
        &[],
    );
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.contains(
            "config packages/a/package.json: whole-file change: one side is not a JSON object"
        ),
        "{err}"
    );
}
