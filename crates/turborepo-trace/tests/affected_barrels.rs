//! `turbotrace affected --since`: export-level tracing through barrel files. A
//! modified re-exported file is skipped for a test only when the test imports
//! none of its names through the barrel, the file is side-effect free at both
//! revisions, and the test runner doesn't type-check.
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

fn ok(root: &Path, args: &[&str]) -> String {
    let o = run(root, args);
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

const JEST_CONFIG: &str = "module.exports = { testMatch: ['**/test/**/*.test.ts'] };\n";

/// A jest repo: `src/index.ts` re-exports `a.ts` and `b.ts`; `test/a.test.ts`
/// imports `{ a }` and `test/b.test.ts` imports `{ b }` through it. `extra`
/// files are written too. Everything is committed.
fn repo(extra: &[(&str, &str)]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(r, ".gitignore", ".turbo/trace/\n");
    write(r, "jest.config.js", JEST_CONFIG);
    write(
        r,
        "src/index.ts",
        "export * from './a';\nexport * from './b';\n",
    );
    write(
        r,
        "src/a.ts",
        "export function a(): number {\n  return 1;\n}\n",
    );
    write(
        r,
        "src/b.ts",
        "export const b = 2;\nexport type B = number;\n",
    );
    write(
        r,
        "test/a.test.ts",
        "import { a } from '../src';\ntest('a', () => expect(a()).toBe(1));\n",
    );
    write(
        r,
        "test/b.test.ts",
        "import { b } from '../src';\ntest('b', () => expect(b).toBe(2));\n",
    );
    for (p, c) in extra {
        write(r, p, c);
    }
    git(r, &["init", "-q"]);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "base"]);
    d
}

fn affected(root: &Path) -> Vec<String> {
    ok(root, &["affected", "--tests", "jest", "--since", "HEAD"])
        .lines()
        .map(str::to_owned)
        .collect()
}

fn explain(root: &Path, entry: &str) -> String {
    ok(
        root,
        &[
            "affected",
            "--tests",
            "jest",
            "--since",
            "HEAD",
            "--explain",
            entry,
        ],
    )
}

#[test]
fn unused_reexported_file_is_skipped() {
    let d = repo(&[]);
    let r = d.path();
    write(
        r,
        "src/b.ts",
        "export const b = 3;\nexport type B = string;\n",
    );
    assert_eq!(affected(r), ["test/b.test.ts"]);
    let e = explain(r, "test/a.test.ts");
    assert!(
        e.contains("unaffected test/a.test.ts")
            && e.contains("src/b.ts skipped: not imported through src/index.ts, side-effect free"),
        "{e}"
    );
    // JSON names the skipped file and its barrel.
    let o = ok(
        r,
        &[
            "affected", "--tests", "jest", "--since", "HEAD", "--format", "json",
        ],
    );
    let v: serde_json::Value = serde_json::from_str(&o).unwrap();
    assert_eq!(v["barrels"]["candidates"], serde_json::json!(["src/b.ts"]));
    assert_eq!(v["barrels"]["skipped"][0]["path"], "src/b.ts");
    assert_eq!(
        v["barrels"]["skipped"][0]["barrels"],
        serde_json::json!(["src/index.ts"])
    );
    assert_eq!(v["barrels"]["skipped"][0]["entries_skipped"], 1);
    assert_eq!(
        v["unaffected_entries"],
        serde_json::json!(["test/a.test.ts"])
    );
}

#[test]
fn a_test_file_that_reexports_the_changed_file_is_selected() {
    // What loads a test file may read what it exports (a plugin that turns every
    // export of a file into a test does): a test file is never a barrel on the
    // way to an importer.
    let d = repo(&[
        (
            "test/c.test.ts",
            "export { a } from '../src/a';\ntest('c', () => {});\n",
        ),
        (
            "test/d.test.ts",
            "export { b as renamed } from '../src';\ntest('d', () => {});\n",
        ),
    ]);
    let r = d.path();
    write(
        r,
        "src/a.ts",
        "export function a(): number {\n  return 2;\n}\n",
    );
    assert_eq!(affected(r), ["test/a.test.ts", "test/c.test.ts"]);
    let e = explain(r, "test/d.test.ts");
    assert!(
        e.contains("unaffected test/d.test.ts")
            && e.contains("src/a.ts skipped: not imported through src/index.ts, side-effect free"),
        "{e}"
    );
    git(r, &["checkout", "-q", "."]);
    write(
        r,
        "src/b.ts",
        "export const b = 3;\nexport type B = number;\n",
    );
    assert_eq!(affected(r), ["test/b.test.ts", "test/d.test.ts"]);
}

#[test]
fn a_change_of_what_linking_asks_for_selects() {
    // Where modules are linked before they run (jest's ES module mode, a browser, a
    // bundler that checks exports), `export { b } from './b'` in the barrel is
    // an error for every importer of the barrel once `./b` stops exporting `b`,
    // whatever name the importer uses: under jest with ES modules the test of
    // `a` fails with "does not provide an export named 'b'". A file whose
    // exported or imported names changed is not skipped.
    let barrel = "export { a } from './a';\nexport { b } from './b';\n";
    let both = ["test/a.test.ts", "test/b.test.ts"];
    let d = repo(&[("src/index.ts", barrel)]);
    let r = d.path();
    write(r, "src/b.ts", "export const c = 2;\n");
    assert_eq!(affected(r), both);
    // With changed exports the file is compared: `b` is gone, and the walk ends at
    // the barrel that asks for it. Without (barrel pruning alone) nothing says
    // which names changed, and a file whose names differ is not skipped at all.
    let without = |r: &Path, format: &str| -> String {
        let o = Command::new(support::harness())
            .args(["affected", "--tests", "jest", "--since", "HEAD"])
            .args(["--format", format])
            .env("TURBO_TRACE_DEBUG_EXPORTS", "0")
            .current_dir(r)
            .output()
            .expect("run turbotrace");
        String::from_utf8(o.stdout).unwrap()
    };
    let v: serde_json::Value = serde_json::from_str(&without(r, "json")).unwrap();
    assert_eq!(v["barrels"]["candidates"], serde_json::json!([]));
    assert_eq!(
        v["barrels"]["not_skippable"][0]["reason"],
        "exported or imported names differ between base and head"
    );
    let e = explain(r, "test/a.test.ts");
    assert!(
        e.contains(
            "  src/index.ts re-exports b (added, removed or redeclared) of src/b.ts: linking it \
             can fail\n"
        ),
        "{e}"
    );
    // An export that is new breaks nothing: no file asked for it and linked.
    // Without the comparison the file is not skipped.
    git(r, &["checkout", "-q", "."]);
    write(
        r,
        "src/b.ts",
        "export const b = 2;\nexport type B = number;\nexport const c = 3;\n",
    );
    assert_eq!(affected(r), Vec::<String>::new());
    assert_eq!(without(r, "list").lines().collect::<Vec<_>>(), both);
    // A named import that is new: the module it names may not export it.
    let b = "import { c } from './c';\nexport function b(): number {\n  return c;\n}\n";
    let d = repo(&[
        ("src/index.ts", barrel),
        ("src/c.ts", "export const c = 1;\n"),
        ("src/b.ts", b),
    ]);
    let r = d.path();
    write(r, "src/b.ts", &b.replace("{ c }", "{ c, d }"));
    assert_eq!(affected(r), both);
    // The same names, another body: skipped for the test that does not use the
    // file.
    write(r, "src/b.ts", &b.replace("return c;", "return c + 1;"));
    assert_eq!(affected(r), ["test/b.test.ts"]);
    // A namespace import asks for no name.
    let b = "import * as c from './c';\nexport function b(): number {\n  return c.c;\n}\n";
    let d = repo(&[
        ("src/index.ts", barrel),
        ("src/c.ts", "export const c = 1;\nexport const d = 2;\n"),
        ("src/b.ts", b),
    ]);
    let r = d.path();
    write(r, "src/b.ts", &b.replace("return c.c;", "return c.d;"));
    assert_eq!(affected(r), ["test/b.test.ts"]);
}

#[test]
fn without_a_base_revision_nothing_is_skipped() {
    let d = repo(&[]);
    let r = d.path();
    write(r, "src/b.ts", "export const b = 3;\n");
    let o = ok(r, &["affected", "--tests", "jest", "--changed", "src/b.ts"]);
    assert_eq!(
        o.lines().collect::<Vec<_>>(),
        ["test/a.test.ts", "test/b.test.ts"]
    );
}

#[test]
fn top_level_side_effects_select() {
    // At the head.
    let d = repo(&[]);
    let r = d.path();
    write(
        r,
        "src/b.ts",
        "export const b = 3;\nconsole.log('loaded');\n",
    );
    assert_eq!(affected(r), ["test/a.test.ts", "test/b.test.ts"]);
    let o = ok(
        r,
        &[
            "affected", "--tests", "jest", "--since", "HEAD", "--format", "json",
        ],
    );
    let v: serde_json::Value = serde_json::from_str(&o).unwrap();
    assert_eq!(v["barrels"]["not_skippable"][0]["path"], "src/b.ts");
    assert_eq!(
        v["barrels"]["not_skippable"][0]["reason"],
        "head: top-level side effects"
    );
    // At the base only.
    let d = repo(&[("src/b.ts", "export const b = Date.now();\n")]);
    let r = d.path();
    write(r, "src/b.ts", "export const b = 3;\n");
    assert_eq!(affected(r), ["test/a.test.ts", "test/b.test.ts"]);
    // An empty collection is a value, not a side effect: only `b` changed.
    let d = repo(&[("src/b.ts", "export const b = new Map();\n")]);
    let r = d.path();
    write(r, "src/b.ts", "export const b = 3;\n");
    assert_eq!(affected(r), ["test/b.test.ts"]);
    // A new module load (its side effects run when the barrel is imported).
    let d = repo(&[(
        "src/polyfill.ts",
        "(globalThis as any).x = 1;\nexport {};\n",
    )]);
    let r = d.path();
    write(r, "src/b.ts", "import './polyfill';\nexport const b = 3;\n");
    assert_eq!(affected(r), ["test/a.test.ts", "test/b.test.ts"]);
}

#[test]
fn syntax_errors_select() {
    // Even inside a function body: the transpiler fails on the whole file.
    let d = repo(&[]);
    let r = d.path();
    write(
        r,
        "src/b.ts",
        "export const b = 3;\nexport function f() { let x = 1; let x = 2; }\n",
    );
    assert_eq!(affected(r), ["test/a.test.ts", "test/b.test.ts"]);
}

#[test]
fn used_names_select() {
    let d = repo(&[]);
    let r = d.path();
    write(
        r,
        "src/a.ts",
        "export function a(): number {\n  return 2;\n}\n",
    );
    assert_eq!(affected(r), ["test/a.test.ts"]);
    let e = explain(r, "test/a.test.ts");
    assert!(e.contains("src/a.ts") && e.contains("src/index.ts"), "{e}");
}

#[test]
fn whole_module_imports_select() {
    for (name, test) in [
        (
            "namespace",
            "import * as m from '../src';\ntest('n', () => expect(Object.keys(m)).toBeTruthy());\n",
        ),
        (
            "dynamic",
            "test('d', async () => { const m = await import('../src'); expect(m.a()).toBe(1); \
             });\n",
        ),
        (
            "require",
            "const m = require('../src');\ntest('r', () => expect(m.a()).toBe(1));\n",
        ),
    ] {
        let d = repo(&[("test/n.test.ts", test)]);
        let r = d.path();
        write(r, "src/b.ts", "export const b = 3;\n");
        assert_eq!(
            affected(r),
            ["test/b.test.ts", "test/n.test.ts"],
            "{name} import of the barrel"
        );
    }
    // Static members of a namespace import and destructured requires are names.
    for test in [
        "import * as m from '../src';\ntest('n', () => expect(m.a()).toBe(1));\n",
        "const { a } = require('../src');\ntest('n', () => expect(a()).toBe(1));\n",
    ] {
        let d = repo(&[("test/n.test.ts", test)]);
        let r = d.path();
        write(r, "src/b.ts", "export const b = 3;\n");
        assert_eq!(affected(r), ["test/b.test.ts"], "{test}");
    }
}

#[test]
fn mocks_of_the_barrel_select() {
    let d = repo(&[(
        "test/m.test.ts",
        "import { a } from '../src';\njest.mock('../src', () => ({ a: () => 2 }));\ntest('m', () \
         => expect(a()).toBe(2));\n",
    )]);
    let r = d.path();
    write(r, "src/b.ts", "export const b = 3;\n");
    // For export-level tracing a mock of the barrel uses every name of it ...
    let o = Command::new(support::harness())
        .args(["affected", "--tests", "jest", "--since", "HEAD"])
        .env("TURBO_TRACE_DEBUG_MOCKS", "0")
        .current_dir(r)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(o.stdout)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["test/b.test.ts", "test/m.test.ts"]
    );
    // ... but the test replaces the barrel with a factory: it loads neither the
    // barrel nor what it re-exports (mock-aware loading, `affected_mocks.rs`).
    assert_eq!(affected(r), ["test/b.test.ts"]);
}

#[test]
fn type_checking_runners_select() {
    let ts_jest = [
        (
            "node_modules/ts-jest/package.json",
            r#"{"name":"ts-jest","version":"29.0.0","main":"index.js"}"#,
        ),
        ("node_modules/ts-jest/index.js", "module.exports = {};\n"),
    ];
    fn config<'a>(base: &[(&'a str, &'a str)], cfg: &'a str) -> Vec<(&'a str, &'a str)> {
        let mut v = base.to_vec();
        v.push(("jest.config.js", cfg));
        v
    }
    let config = |cfg: &'static str| config(&ts_jest, cfg);
    let checked = "module.exports = {\n  testMatch: ['**/test/**/*.test.ts'],\n  transform: { \
                   '^.+\\\\.ts$': 'ts-jest' },\n};\n";
    let d = repo(&config(checked));
    let r = d.path();
    write(r, "src/b.ts", "export const b = 3;\n");
    assert_eq!(
        affected(r),
        ["test/a.test.ts", "test/b.test.ts"],
        "ts-jest type-checks"
    );
    let transpiled = "module.exports = {\n  testMatch: ['**/test/**/*.test.ts'],\n  transform: { \
                      '^.+\\\\.ts$': ['ts-jest', { diagnostics: false }] },\n};\n";
    let d = repo(&config(transpiled));
    let r = d.path();
    write(r, "src/b.ts", "export const b = 3;\n");
    assert_eq!(
        affected(r),
        ["test/b.test.ts"],
        "ts-jest without diagnostics"
    );
}

#[test]
fn export_star_collisions_select() {
    // b.ts starts exporting `a`, which a.ts also exports: ambiguous (ESM) or first
    // wins (CommonJS); either way test/a.test.ts can see it. `b` itself did not
    // change, so test/b.test.ts cannot (export-level comparison; without it
    // both are selected).
    let d = repo(&[]);
    let r = d.path();
    write(r, "src/b.ts", "export const b = 2;\nexport const a = 5;\n");
    assert_eq!(affected(r), ["test/a.test.ts"]);
    let all = |r: &Path| {
        let o = Command::new(support::harness())
            .args(["affected", "--tests", "jest", "--since", "HEAD"])
            .env("TURBO_TRACE_DEBUG_EXPORT_DIFF", "0")
            .current_dir(r)
            .output()
            .expect("run turbotrace");
        String::from_utf8(o.stdout).unwrap()
    };
    assert_eq!(
        all(r).lines().collect::<Vec<_>>(),
        ["test/a.test.ts", "test/b.test.ts"]
    );
    // ... and stops exporting it.
    let d = repo(&[("src/b.ts", "export const b = 2;\nexport const a = 5;\n")]);
    let r = d.path();
    write(r, "src/b.ts", "export const b = 2;\n");
    assert_eq!(affected(r), ["test/a.test.ts"]);
}

#[test]
fn deleted_and_renamed_reexported_files_select() {
    let d = repo(&[]);
    let r = d.path();
    std::fs::remove_file(r.join("src/b.ts")).unwrap();
    assert_eq!(affected(r), ["test/a.test.ts", "test/b.test.ts"]);
}

#[test]
fn files_imported_besides_the_barrel_select() {
    // src/c.ts is re-exported too and imports `b`: test/c.test.ts uses `c`.
    let d = repo(&[
        (
            "src/index.ts",
            "export * from './a';\nexport * from './b';\nexport * from './c';\n",
        ),
        (
            "src/c.ts",
            "import { b } from './b';\nexport function c(): number {\n  return b;\n}\n",
        ),
        (
            "test/c.test.ts",
            "import { c } from '../src';\ntest('c', () => expect(c()).toBe(2));\n",
        ),
    ]);
    let r = d.path();
    write(r, "src/b.ts", "export const b = 3;\n");
    assert_eq!(affected(r), ["test/b.test.ts", "test/c.test.ts"]);
}
