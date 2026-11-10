//! `turbotrace affected --since`: a governor of the base tree that the head
//! graph no longer names.
//!
//! `affected` reads the graph of the head. What decided a file's records at the
//! base and is not consulted for it at the head (a config that stops applying,
//! a manifest that stops bounding, a mock that goes) is found only if the head
//! graph still says where the file looked: a probe of the place, a membership
//! of the directory, an edge to the config that took over, or a diff rule that
//! reads the base value. The tsconfig case (a referenced project that stops
//! including a file) is in `affected_tsconfig.rs`; these are
//! the other governors of JS tests, each as the change that takes the governor
//! away and the one that brings it back.
//!
//! Every case asserts the tests that must be selected under `--on-uncertain
//! ignore`, the default policy and `--strict`, on the repository's graph
//! (refreshed) and on a new one, which must select the same.
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

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(support::harness())
        .args(args)
        .current_dir(root)
        .output()
        .expect("run turbotrace")
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

/// Commits the working tree as the base.
fn commit(r: &Path, message: &str) {
    if !r.join(".git").exists() {
        git(r, &["init", "-q"]);
    }
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "--allow-empty", "-m", message]);
}

fn lines(o: Output, args: &[&str]) -> Vec<String> {
    assert!(
        o.status.success(),
        "turbotrace {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// What the diff against `HEAD` selects under `--on-uncertain ignore`, the
/// default policy and `--strict`, each on the repository's graph (built at an
/// earlier state, refreshed now) and on a new graph, which must agree.
fn selections(r: &Path, scratch: &Path, what: &str) -> Vec<(&'static str, Vec<String>)> {
    static COLD: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let mut out = Vec::new();
    for (policy, flags) in [
        ("ignore", &["--on-uncertain", "ignore"][..]),
        ("default", &[][..]),
        ("strict", &["--strict"][..]),
    ] {
        let base = [
            &["affected", "--since", "HEAD"][..],
            flags,
            &["--tests", "jest", "--tests", "vitest"][..],
        ]
        .concat();
        let refreshed = lines(run(r, &base), &base);
        let n = COLD.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let graph = scratch.join(format!("g-cold-{n}"));
        let mut args = base.clone();
        args.extend(["--graph", graph.to_str().unwrap()]);
        let cold = lines(run(r, &args), &args);
        assert_eq!(refreshed, cold, "{what}: refreshed and cold ({policy})");
        out.push((policy, refreshed));
    }
    eprintln!("OBSERVED {what}: {out:?}");
    out
}

/// Asserts that the diff selects every test of `want` under every policy.
fn selects(r: &Path, scratch: &Path, what: &str, want: &[&str]) {
    for (policy, got) in selections(r, scratch, what) {
        for w in want {
            assert!(
                got.iter().any(|g| g == w),
                "{what} ({policy}): {w} is not selected: {got:?}"
            );
        }
    }
}

/// Asserts that nothing is selected when nothing changed (`--on-uncertain
/// ignore`; the other policies may select a floor), and warms the repository's
/// graph.
fn quiet(r: &Path, scratch: &Path) {
    let all = selections(r, scratch, "nothing changed");
    assert!(all[0].1.is_empty(), "nothing changed: {:?}", all[0].1);
}

fn root_files(r: &Path) {
    write(r, "package.json", r#"{"name":"root","private":true}"#);
    write(r, ".gitignore", ".turbo/trace/\nnode_modules/\n");
}

fn workspace(r: &Path) {
    root_files(r);
    write(r, "pnpm-workspace.yaml", "packages:\n  - 'packages/*'\n");
}

const VITEST_CONFIG: &str = "import { defineConfig } from 'vitest/config';\n\nexport default \
                             defineConfig({ test: { include: ['tests/**/*.test.ts'] } });\n";

fn vitest_test(import: &str, name: &str) -> String {
    format!(
        "import {{ expect, test }} from 'vitest';\nimport {{ {name} }} from \
         '{import}';\n\ntest('{name}', () => {{\n  expect({name}).toBeDefined();\n}});\n"
    )
}

// ---------------------------------------------------------------------------------------
// F1. The `package.json` that bounds the tsconfig fallback.
// ---------------------------------------------------------------------------------------

/// A file no tsconfig covers uses the nearest config at or below its package
/// root (`crates/turborepo-trace-lang-js/src/resolve/tsconfig.rs`, the
/// fallback). A nested `package.json` makes its directory the package root,
/// which cuts the config above off: the alias import does not resolve. The
/// manifest goes: the import resolves through the config's `paths`. It comes
/// back: the import no longer resolves. Covered by the file's own record of its
/// package scope (`JsResolver::nearest_package_json`: a probe of every
/// `package.json` missing on the way up, an edge to the one found).
#[test]
fn the_manifest_that_bounds_the_tsconfig_fallback_goes_and_comes() {
    let d = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let (r, s) = (d.path(), scratch.path());
    workspace(r);
    write(
        r,
        "packages/app/package.json",
        r#"{"name":"@acme/app","version":"1.0.0","scripts":{"test":"vitest run"}}"#,
    );
    write(r, "packages/app/vitest.config.ts", VITEST_CONFIG);
    write(
        r,
        "packages/app/tsconfig.json",
        "{ \"compilerOptions\": { \"paths\": { \"@lib/*\": [\"./v1/*\"] } }, \"include\": \
         [\"none\"] }\n",
    );
    write(r, "packages/app/v1/dep.ts", "export const dep = 'v1';\n");
    let bound = "packages/app/src/sub/package.json";
    write(r, bound, "{ \"type\": \"module\" }\n");
    write(
        r,
        "packages/app/src/sub/h.ts",
        "import { dep } from '@lib/dep';\n\nexport const h = dep;\n",
    );
    write(r, "packages/app/src/o.ts", "export const o = 1;\n");
    let t = "packages/app/tests/t.test.ts";
    write(r, t, &vitest_test("../src/sub/h", "h"));
    write(
        r,
        "packages/app/tests/c.test.ts",
        &vitest_test("../src/o", "o"),
    );
    commit(r, "base");
    quiet(r, s);
    std::fs::remove_file(r.join(bound)).unwrap();
    selects(r, s, "F1 the bounding manifest goes", &[t]);
    commit(r, "no bound");
    quiet(r, s);
    write(r, bound, "{ \"type\": \"module\" }\n");
    selects(r, s, "F1 the bounding manifest comes", &[t]);
}

// ---------------------------------------------------------------------------------------
// F2. The nearer runner config of a test.
// ---------------------------------------------------------------------------------------

/// A test is governed by the nearest runner config above it. The nearer one
/// goes: the config further up takes over (other setup files). It comes back.
/// Covered by the test's own record: a probe of every config candidate missing
/// on the way up, an edge to the config found
/// (`crates/turborepo-trace-store/src/fw.rs`, `Fw::config_for`).
#[test]
fn the_nearer_runner_config_goes_and_comes() {
    // jest.
    let d = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let (r, s) = (d.path(), scratch.path());
    root_files(r);
    write(r, "jest.config.js", "module.exports = {};\n");
    write(
        r,
        "packages/a/package.json",
        r#"{"name":"@acme/a","version":"1.0.0"}"#,
    );
    let near = "packages/a/jest.config.js";
    let near_text = "module.exports = { setupFiles: ['<rootDir>/setup.js'] };\n";
    write(r, near, near_text);
    write(r, "packages/a/setup.js", "global.flag = 1;\n");
    let x = "packages/a/x.test.js";
    write(
        r,
        x,
        "test('x', () => {\n  expect(global.flag).toBe(1);\n});\n",
    );
    write(
        r,
        "tests/r.test.js",
        "test('r', () => {\n  expect(1).toBe(1);\n});\n",
    );
    commit(r, "base");
    quiet(r, s);
    std::fs::remove_file(r.join(near)).unwrap();
    selects(r, s, "F2 the nearer jest config goes", &[x]);
    commit(r, "no nearer config");
    quiet(r, s);
    write(r, near, near_text);
    selects(r, s, "F2 the nearer jest config comes", &[x]);

    // vitest.
    let d = tempfile::tempdir().unwrap();
    let (r, s) = (d.path(), scratch.path());
    root_files(r);
    write(
        r,
        "vitest.config.ts",
        "import { defineConfig } from 'vitest/config';\n\nexport default defineConfig({ test: { \
         include: ['**/*.test.ts'] } });\n",
    );
    write(
        r,
        "packages/a/package.json",
        r#"{"name":"@acme/a","version":"1.0.0"}"#,
    );
    let near = "packages/a/vitest.config.ts";
    let near_text = "import { defineConfig } from 'vitest/config';\n\nexport default \
                     defineConfig({ test: { include: ['**/*.test.ts'], setupFiles: ['./setup.ts'] \
                     } });\n";
    write(r, near, near_text);
    write(r, "packages/a/setup.ts", "export {};\n");
    write(r, "packages/a/v.ts", "export const v = 1;\n");
    let x = "packages/a/x.test.ts";
    write(r, x, &vitest_test("./v", "v"));
    commit(r, "base");
    quiet(r, s);
    std::fs::remove_file(r.join(near)).unwrap();
    selects(r, s, "F2 the nearer vitest config goes", &[x]);
    commit(r, "no nearer config");
    quiet(r, s);
    write(r, near, near_text);
    selects(r, s, "F2 the nearer vitest config comes", &[x]);
}

// ---------------------------------------------------------------------------------------
// F3. A haste mock of a package.
// ---------------------------------------------------------------------------------------

/// jest replaces a package of `node_modules` by `<root>/__mocks__/<package>.js`
/// without a `jest.mock` call. The graph puts the edge on every file below the
/// project's root that imports the package, and the listing that names the mock
/// is a key-only record of that file (`FileRec::key_members`). The mock goes;
/// it comes. Covered by the tests, not by the importing file: every jest test
/// of the project has a membership of the `__mocks__` directories of its roots.
#[test]
fn a_haste_mock_goes_and_comes() {
    let d = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let (r, s) = (d.path(), scratch.path());
    root_files(r);
    write(r, "jest.config.js", "module.exports = {};\n");
    let mock = "__mocks__/left-pad.js";
    let mock_text = "module.exports = () => 'mock';\n";
    write(r, mock, mock_text);
    write(
        r,
        "src/h.js",
        "const pad = require('left-pad');\n\nmodule.exports = () => pad('a');\n",
    );
    write(r, "src/o.js", "module.exports = 1;\n");
    let t = "tests/h.test.js";
    write(
        r,
        t,
        "const h = require('../src/h');\n\ntest('h', () => {\n  expect(h()).toBe('mock');\n});\n",
    );
    write(
        r,
        "tests/c.test.js",
        "const o = require('../src/o');\n\ntest('c', () => {\n  expect(o).toBe(1);\n});\n",
    );
    commit(r, "base");
    quiet(r, s);
    std::fs::remove_file(r.join(mock)).unwrap();
    selects(r, s, "F3 the haste mock goes", &[t]);
    commit(r, "no mock");
    quiet(r, s);
    write(r, mock, mock_text);
    selects(r, s, "F3 the haste mock comes", &[t]);
}

// ---------------------------------------------------------------------------------------
// F4. A test that two projects of one config match.
// ---------------------------------------------------------------------------------------

/// jest runs a test file once in every project whose patterns match it, and the
/// graph gives a test the inputs of every project that matches (`Fw::augment`;
/// it was the first one only). A setup file of the second project is edited:
/// the test's run in that project changes.
///
/// Not the defect of a governor that the head graph forgets (the base and the
/// head agree here): the second project was never modelled.
#[test]
fn a_test_of_two_projects_depends_on_both() {
    let d = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let (r, s) = (d.path(), scratch.path());
    root_files(r);
    write(
        r,
        "jest.config.js",
        "module.exports = {\n  projects: [\n    { displayName: 'a', setupFiles: \
         ['<rootDir>/setup-a.js'] },\n    { displayName: 'b', setupFiles: \
         ['<rootDir>/setup-b.js'] },\n  ],\n};\n",
    );
    write(r, "setup-a.js", "global.flag = 'a';\n");
    write(r, "setup-b.js", "global.flag = 'b';\n");
    let t = "tests/t.test.js";
    write(
        r,
        t,
        "test('t', () => {\n  expect(global.flag).toBeDefined();\n});\n",
    );
    commit(r, "base");
    quiet(r, s);
    write(r, "setup-a.js", "global.flag = 'a2';\n");
    selects(r, s, "F4 the first project's setup file", &[t]);
    commit(r, "a2");
    quiet(r, s);
    write(r, "setup-b.js", "global.flag = 'b2';\n");
    selects(r, s, "F4 the second project's setup file", &[t]);
}

/// The same with the inline projects of a vitest config (`test.projects`).
#[test]
fn a_vitest_test_of_two_projects_depends_on_both() {
    let d = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let (r, s) = (d.path(), scratch.path());
    root_files(r);
    write(
        r,
        "vitest.config.ts",
        "import { defineConfig } from 'vitest/config';\n\nexport default defineConfig({\n  test: \
         {\n    projects: [\n      { test: { name: 'a', include: ['tests/**/*.test.ts'], \
         setupFiles: ['./setup-a.ts'] } },\n      { test: { name: 'b', include: \
         ['tests/**/*.test.ts'], setupFiles: ['./setup-b.ts'] } },\n    ],\n  },\n});\n",
    );
    write(r, "setup-a.ts", "export const flag = 'a';\n");
    write(r, "setup-b.ts", "export const flag = 'b';\n");
    write(r, "src/v.ts", "export const v = 1;\n");
    let t = "tests/t.test.ts";
    write(r, t, &vitest_test("../src/v", "v"));
    commit(r, "base");
    quiet(r, s);
    write(r, "setup-a.ts", "export const flag = 'a2';\n");
    selects(r, s, "F4v the first project's setup file", &[t]);
    commit(r, "a2");
    quiet(r, s);
    write(r, "setup-b.ts", "export const flag = 'b2';\n");
    selects(r, s, "F4v the second project's setup file", &[t]);
}

// ---------------------------------------------------------------------------------------
// F5, F6. A workspace package: its `exports`, and the globs that make it one.
// ---------------------------------------------------------------------------------------

const LIB_MANIFEST: &str = "packages/lib/package.json";

fn lib_manifest(exports: &str) -> String {
    format!("{{\"name\":\"@acme/lib\",\"version\":\"1.0.0\",\"exports\":{exports}}}\n")
}

const BOTH: &str = r#"{"./feature":{"import":"./src/esm.ts","default":"./src/cjs.ts"}}"#;

/// `packages/app` imports `@acme/lib/feature` in a helper; `packages/lib` is
/// linked into its `node_modules` as an install leaves it.
fn linked(r: &Path) -> &'static str {
    workspace(r);
    write(r, LIB_MANIFEST, &lib_manifest(BOTH));
    write(r, "packages/lib/src/esm.ts", "export const x = 'esm';\n");
    write(r, "packages/lib/src/cjs.ts", "export const x = 'cjs';\n");
    write(
        r,
        "packages/app/package.json",
        r#"{"name":"@acme/app","version":"1.0.0","scripts":{"test":"vitest run"},"dependencies":{"@acme/lib":"workspace:*"}}"#,
    );
    let link = r.join("packages/app/node_modules/@acme/lib");
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    turborepo_trace_core::os::symlink("../../../lib", link).unwrap();
    write(r, "packages/app/vitest.config.ts", VITEST_CONFIG);
    write(
        r,
        "packages/app/src/f.ts",
        "import { x } from '@acme/lib/feature';\n\nexport const f = x;\n",
    );
    write(r, "packages/app/src/o.ts", "export const o = 1;\n");
    let t = "packages/app/tests/t.test.ts";
    write(r, t, &vitest_test("../src/f", "f"));
    write(
        r,
        "packages/app/tests/c.test.ts",
        &vitest_test("../src/o", "o"),
    );
    commit(r, "base");
    t
}

/// The `exports` of a workspace package answer a subpath by condition. The
/// condition the importer used goes (the request falls to `default`: another
/// file); the subpath goes (the request is refused); it comes back. Covered by
/// `crates/turborepo-trace/src/exportsdiff.rs`: both manifests answer the
/// requests the head graph recorded, and an importer whose answer differs is
/// seeded.
#[test]
fn the_exports_of_a_workspace_package_answer_differently() {
    let d = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let (r, s) = (d.path(), scratch.path());
    let t = linked(r);
    quiet(r, s);
    write(
        r,
        LIB_MANIFEST,
        &lib_manifest(r#"{"./feature":{"default":"./src/cjs.ts"}}"#),
    );
    selects(r, s, "F5 the import condition goes", &[t]);
    commit(r, "default only");
    quiet(r, s);
    write(
        r,
        LIB_MANIFEST,
        &lib_manifest(r#"{"./other":"./src/cjs.ts"}"#),
    );
    selects(r, s, "F5 the subpath goes", &[t]);
    commit(r, "no subpath");
    quiet(r, s);
    write(r, LIB_MANIFEST, &lib_manifest(BOTH));
    selects(r, s, "F5 the subpath comes back", &[t]);
}

/// The workspace globs decide which directories are packages of the graph. A
/// package that stays linked in `node_modules` and leaves the globs (no install
/// ran): no test run reads the globs, so nothing has to be selected; the
/// refreshed graph and a new one must agree, and bringing the package back must
/// agree too.
#[test]
fn the_workspace_globs_stop_listing_a_linked_package() {
    let d = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let (r, s) = (d.path(), scratch.path());
    linked(r);
    quiet(r, s);
    write(r, "pnpm-workspace.yaml", "packages:\n  - 'packages/app'\n");
    selections(r, s, "F6 the globs stop listing the package");
    commit(r, "lib is not listed");
    quiet(r, s);
    write(r, "pnpm-workspace.yaml", "packages:\n  - 'packages/*'\n");
    selections(r, s, "F6 the globs list the package again");
}
