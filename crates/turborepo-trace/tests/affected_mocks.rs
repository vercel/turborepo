//! `turbotrace affected`: mock-aware loading. A jest or vitest test is not
//! selected by a change it reaches only through a module it replaces with a
//! mock that does not load the module; every form that loads the real module
//! after all keeps it selected. The runtime behaviour behind each case is
//! measured by a runtime mock oracle.
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
    let o = Command::new(support::harness())
        .args(args)
        .envs(env.iter().copied())
        .current_dir(root)
        .output()
        .expect("run turbotrace");
    assert!(
        o.status.success(),
        "turbotrace {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout).unwrap()
}

const JEST: &str = "module.exports = { testMatch: ['**/test/**/*.test.ts'] };\n";
const VITEST: &str = "export default { test: { include: ['test/**/*.test.ts'] } };\n";

/// `test/a.test.ts → src/user.ts → src/m.ts → src/deep.ts`, with `test` as the
/// test file's text after its import of the subject, and `extra` files (a later
/// file of the same path replaces an earlier one).
fn repo(config: (&str, &str), test: &str, extra: &[(&str, &str)]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(r, ".gitignore", ".turbo/trace/\n");
    write(r, config.0, config.1);
    write(r, "src/deep.ts", "export const d = 1;\n");
    write(
        r,
        "src/m.ts",
        "import { d } from './deep';\nexport const v = d;\n",
    );
    write(
        r,
        "src/user.ts",
        "import { v } from './m';\nexport const use = () => v;\n",
    );
    write(
        r,
        "test/a.test.ts",
        &format!("import {{ use }} from '../src/user';\n{test}\ntest('a', () => {{ use(); }});\n"),
    );
    for (p, text) in extra {
        write(r, p, text);
    }
    d
}

/// The tests a modification of `changed` selects.
fn selected(root: &Path, runner: &str, changed: &str, env: &[(&str, &str)]) -> Vec<String> {
    run(
        root,
        &["affected", "--tests", runner, "--changed", changed],
        env,
    )
    .lines()
    .map(str::to_owned)
    .collect()
}

const FACTORY: &str = "jest.mock('../src/m', () => ({ v: 2 }));";
const TEST: &str = "test/a.test.ts";

#[test]
fn a_replaced_module_does_not_select_the_test() {
    let d = repo(("jest.config.js", JEST), FACTORY, &[]);
    let r = d.path();
    for changed in ["src/m.ts", "src/deep.ts"] {
        assert!(selected(r, "jest", changed, &[]).is_empty(), "{changed}");
        // The switch.
        assert_eq!(
            selected(r, "jest", changed, &[("TURBO_TRACE_DEBUG_MOCKS", "0")]),
            [TEST],
            "{changed}"
        );
    }
    for changed in ["src/user.ts", TEST] {
        assert_eq!(selected(r, "jest", changed, &[]), [TEST], "{changed}");
    }
    let why = run(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--changed",
            "src/deep.ts",
            "--explain",
            TEST,
        ],
        &[],
    );
    assert!(
        why.contains(
            "not selected through src/m.ts: replaced by the mock at test/a.test.ts:2 (loaded by \
             src/user.ts)"
        ),
        "{why}"
    );
    let json = run(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--changed",
            "src/deep.ts",
            "--format",
            "json",
        ],
        &[],
    );
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        v["pruning"]["mock_skipped"][0]["module"], "src/m.ts",
        "{json}"
    );
    assert_eq!(v["pruning"]["mock_skipped"][0]["mock"], "test/a.test.ts:2");
}

#[test]
fn what_loads_the_real_module_keeps_the_test_selected() {
    for undo in [
        // The factory loads the real module.
        "jest.mock('../src/m', () => ({ ...jest.requireActual('../src/m'), w: 1 }));",
        "jest.mock('../src/m', () => { const real = jest.requireActual<typeof \
         import('../src/m')>('../src/m'); return real; });",
        // An automock is built from the real module.
        "jest.mock('../src/m');",
        // Not hoisted above the import.
        "jest.doMock('../src/m', () => ({ v: 2 }));",
        "beforeAll(() => { jest.mock('../src/m', () => ({ v: 2 })); });",
        "if (process.env.X) { jest.mock('../src/m', () => ({ v: 2 })); }",
        // A factory turbotrace cannot read.
        "jest.mock('../src/m', factory);",
        // Undone later.
        "jest.mock('../src/m', () => ({ v: 2 }));\njest.unmock('../src/m');",
        "jest.mock('../src/m', () => ({ v: 2 }));\nafterAll(() => jest.dontMock('../src/m'));",
        "jest.mock('../src/m', () => ({ v: 2 }));\nconst real = jest.requireActual('../src/m');",
        "jest.mock('../src/m', () => ({ v: 2 }));\njest.doMock('../src/m');",
        "jest.mock('../src/m', () => ({ v: 2 }));\njest.unmock(name);",
        "jest.mock('../src/m', () => ({ v: 2 }));\nconst { requireActual } = jest;",
        "jest.mock('../src/m', () => ({ v: 2 }));\njest.enableAutomock();",
    ] {
        let d = repo(("jest.config.js", JEST), undo, &[]);
        assert_eq!(
            selected(d.path(), "jest", "src/deep.ts", &[]),
            [TEST],
            "{undo}"
        );
    }
    // The registry is cleared, the mock stays; a later factory.
    for keep in [
        "jest.mock('../src/m', () => ({ v: 2 }));\nbeforeEach(() => jest.resetModules());",
        "jest.mock('../src/m', () => ({ v: 2 }));\njest.isolateModules(() => { \
         require('../src/user'); });",
        "jest.mock('../src/m', () => ({ v: 2 }));\nbeforeEach(() => { jest.doMock('../src/m', () \
         => ({ v: 3 })); });",
        "const mockV = 2;\njest.mock('../src/m', () => ({ get v() { return mockV; } }));",
        "jest.mock('../src/m', () => ({ v: 2 }), { virtual: true });",
    ] {
        let d = repo(("jest.config.js", JEST), keep, &[]);
        assert!(
            selected(d.path(), "jest", "src/deep.ts", &[]).is_empty(),
            "{keep}"
        );
    }
}

#[test]
fn another_way_to_the_change_keeps_the_test_selected() {
    // A second import path that no mock intercepts.
    let d = repo(
        ("jest.config.js", JEST),
        "import { o } from '../src/other';\njest.mock('../src/m', () => ({ v: 2 }));",
        &[(
            "src/other.ts",
            "import { d } from './deep';\nexport const o = d;\n",
        )],
    );
    assert_eq!(selected(d.path(), "jest", "src/deep.ts", &[]), [TEST]);
    assert!(selected(d.path(), "jest", "src/m.ts", &[]).is_empty());
    // A helper that loads the real module, or that unmocks something turbotrace
    // cannot name.
    for helper in [
        "export const real = () => jest.requireActual('./m');\n",
        "export const undo = (name: string) => jest.unmock(name);\n",
    ] {
        let d = repo(
            ("jest.config.js", JEST),
            "import '../src/helper';\njest.mock('../src/m', () => ({ v: 2 }));",
            &[("src/helper.ts", helper)],
        );
        assert_eq!(
            selected(d.path(), "jest", "src/deep.ts", &[]),
            [TEST],
            "{helper}"
        );
    }
    // The factory's own imports are code the test runs.
    let d = repo(
        ("jest.config.js", JEST),
        "jest.mock('../src/m', () => require('../src/fake'));",
        &[("src/fake.ts", "export const v = 3;\n")],
    );
    assert_eq!(selected(d.path(), "jest", "src/fake.ts", &[]), [TEST]);
    assert!(selected(d.path(), "jest", "src/deep.ts", &[]).is_empty());
}

/// A test that two projects of one config run (jest runs it once in each) has
/// the inputs of both. A module that one project's run replaces with a mock (in
/// its setup file) and the other's loads for real is loaded: a change behind it
/// selects the test. When every run makes the replacement, it holds.
#[test]
fn a_module_that_one_of_two_projects_mocks_is_still_loaded() {
    let setup = [(
        "test/setup.ts",
        "jest.mock('../src/m', () => ({ v: 2 }));\n",
    )];
    let project = |name: &str, setup: bool| {
        format!(
            "{{ displayName: '{name}', testMatch: ['**/test/**/*.test.ts']{} }}",
            if setup {
                ", setupFiles: ['<rootDir>/test/setup.ts']"
            } else {
                ""
            }
        )
    };
    let config = |b_mocks: bool| {
        format!(
            "module.exports = {{ projects: [{}, {}] }};\n",
            project("a", true),
            project("b", b_mocks)
        )
    };
    let d = repo(("jest.config.js", &config(false)), "", &setup);
    assert_eq!(selected(d.path(), "jest", "src/deep.ts", &[]), [TEST]);
    let d = repo(("jest.config.js", &config(true)), "", &setup);
    assert!(selected(d.path(), "jest", "src/deep.ts", &[]).is_empty());
    // With the inputs of the first project only (as before), the replacement of
    // project `a` was taken for the test's.
    let d = repo(("jest.config.js", &config(false)), "", &setup);
    assert!(
        selected(
            d.path(),
            "jest",
            "src/deep.ts",
            &[("TURBO_TRACE_DEBUG_FW_PROJECTS", "first")]
        )
        .is_empty()
    );
}

#[test]
fn manual_mocks_and_setup_files() {
    // A manual mock next to the module stands in for it.
    let d = repo(
        ("jest.config.js", JEST),
        "jest.mock('../src/m');",
        &[("src/__mocks__/m.ts", "export const v = 5;\n")],
    );
    assert!(selected(d.path(), "jest", "src/deep.ts", &[]).is_empty());
    assert_eq!(
        selected(d.path(), "jest", "src/__mocks__/m.ts", &[]),
        [TEST]
    );
    // A mock registered by a setup file holds for every test of the project ...
    let setup = "module.exports = { testMatch: ['**/test/**/*.test.ts'], setupFiles: \
                 ['<rootDir>/test/setup.ts'] };\n";
    let d = repo(
        ("jest.config.js", setup),
        "",
        &[(
            "test/setup.ts",
            "jest.mock('../src/m', () => ({ v: 2 }));\n",
        )],
    );
    assert!(selected(d.path(), "jest", "src/deep.ts", &[]).is_empty());
    let why = run(
        d.path(),
        &[
            "affected",
            "--tests",
            "jest",
            "--changed",
            "src/deep.ts",
            "--explain",
            TEST,
        ],
        &[],
    );
    assert!(
        why.contains("replaced by the mock at test/setup.ts:1"),
        "{why}"
    );
    // ... unless a setup file loaded the module (or what imports it) first: the
    // real one stays in the registry.
    for import in ["import '../src/m';", "import '../src/user';"] {
        let d = repo(
            ("jest.config.js", setup),
            FACTORY,
            &[("test/setup.ts", &format!("{import}\n"))],
        );
        assert_eq!(
            selected(d.path(), "jest", "src/deep.ts", &[]),
            [TEST],
            "{import}"
        );
    }
}

#[test]
fn a_helpers_manual_mock_counts_for_tests_that_request_its_module() {
    // `src/helper.ts` registers the manual mock of `src/n.ts`, which imports
    // `src/mockdep.ts`. `a` loads the helper only, `b` the helper and `src/n.ts`,
    // `c` only `src/n.ts`.
    let files = |helper: &'static str| -> Vec<(&'static str, &'static str)> {
        vec![
            ("src/helper.ts", helper),
            ("src/n.ts", "export const n = 1;\n"),
            ("src/o.ts", "export const o = 1;\n"),
            (
                "src/__mocks__/n.ts",
                "import { md } from '../mockdep';\nexport const n = md;\n",
            ),
            ("src/mockdep.ts", "export const md = 2;\n"),
            (
                "test/b.test.ts",
                "import '../src/helper';\nimport { n } from '../src/n';\ntest('b', () => { n; \
                 });\n",
            ),
            (
                "test/c.test.ts",
                "import { n } from '../src/n';\ntest('c', () => { n; });\n",
            ),
        ]
    };
    const B: &str = "test/b.test.ts";
    const C: &str = "test/c.test.ts";
    let d = repo(
        ("jest.config.js", JEST),
        "import '../src/helper';",
        &files("jest.mock('./n');\nexport const h = 1;\n"),
    );
    let r = d.path();
    for changed in ["src/mockdep.ts", "src/__mocks__/n.ts"] {
        // `a` never requests the module: the mock is not loaded. `c` requests it
        // without the helper: it gets the real module.
        assert_eq!(selected(r, "jest", changed, &[]), [B], "{changed}");
        // The switch: the mock counts for whatever requests the module.
        assert_eq!(
            selected(r, "jest", changed, &[("TURBO_TRACE_DEBUG_MOCKS", "0")]),
            [B, C],
            "{changed}"
        );
    }
    assert_eq!(selected(r, "jest", "src/helper.ts", &[]), [TEST, B]);
    assert_eq!(selected(r, "jest", "src/n.ts", &[]), [B, C]);
    let why = run(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--changed",
            "src/mockdep.ts",
            "--explain",
            C,
        ],
        &[],
    );
    assert!(
        why.contains(
            "not selected through src/__mocks__/n.ts: the mock of src/n.ts for tests that load \
             src/helper.ts, which the test does not"
        ),
        "{why}"
    );
    let json = run(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--changed",
            "src/mockdep.ts",
            "--format",
            "json",
        ],
        &[],
    );
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let u = &v["pruning"]["mock_unregistered"][0];
    assert_eq!(u["entry"], C, "{json}");
    assert_eq!(u["mock"], "src/__mocks__/n.ts");
    assert_eq!(u["module"], "src/n.ts");
    assert_eq!(u["registered_by"], "src/helper.ts");
    // The helper requests the module itself: every test that loads it loads the
    // mock.
    let d = repo(
        ("jest.config.js", JEST),
        "import '../src/helper';",
        &files("jest.mock('./n');\nimport { n } from './n';\nexport const h = n;\n"),
    );
    assert_eq!(selected(d.path(), "jest", "src/mockdep.ts", &[]), [TEST, B]);
    // An automock is built from the real module when it is requested: the helper
    // adds no dependency. `requireActual` loads it with the helper.
    let d = repo(
        ("jest.config.js", JEST),
        "import '../src/helper';",
        &files("jest.mock('./o');\nexport const h = 1;\n"),
    );
    for env in [&[][..], &[("TURBO_TRACE_DEBUG_MOCKS", "0")][..]] {
        assert!(selected(d.path(), "jest", "src/o.ts", env).is_empty());
    }
    let d = repo(
        ("jest.config.js", JEST),
        "import '../src/helper';",
        &files("export const real = () => jest.requireActual('./o');\n"),
    );
    assert_eq!(selected(d.path(), "jest", "src/o.ts", &[]), [TEST, B]);
}

/// Names (`turborepo_trace_graph::names`) follow edges without conditions. The
/// manual mock that a helper registers is an edge of the mocked module, not of
/// the helper, so a read with a computed path in the mock and the strings next
/// to it count for the tests that request the module (with or without the
/// helper: more than the tests that get the mock), and not for a test that only
/// loads the helper.
#[test]
fn names_reach_a_helpers_manual_mock_through_its_module() {
    const B: &str = "test/b.test.ts";
    const C: &str = "test/c.test.ts";
    let d = repo(
        ("jest.config.js", JEST),
        "import '../src/helper';",
        &[
            ("src/helper.ts", "jest.mock('./n');\nexport const h = 1;\n"),
            ("src/n.ts", "export const n = () => '';\n"),
            (
                "src/__mocks__/n.ts",
                "import * as fs from 'fs';\nexport const n = (base: string) => \
                 fs.readFileSync(base + '/seeds/' + 'a.json', 'utf8');\n",
            ),
            ("seeds/a.json", "{}\n"),
            (
                B,
                "import '../src/helper';\nimport { n } from '../src/n';\ntest('b', () => { n; \
                 });\n",
            ),
            (
                C,
                "import { n } from '../src/n';\ntest('c', () => { n; });\n",
            ),
        ],
    );
    let r = d.path();
    // The string `/seeds/` of the mock names the changed file for its read.
    assert_eq!(selected(r, "jest", "seeds/a.json", &[]), [B, C]);
    // Without the bound the read is open: every test that reaches it, the same
    // tests.
    assert_eq!(
        selected(
            r,
            "jest",
            "seeds/a.json",
            &[("TURBO_TRACE_DEBUG_NAMES", "0")]
        ),
        [B, C]
    );
    // A change of the mock itself is narrowed to the tests that get it.
    assert_eq!(selected(r, "jest", "src/__mocks__/n.ts", &[]), [B]);
}

#[test]
fn project_conditions() {
    let with = |extra: &str| {
        format!("module.exports = {{ testMatch: ['**/test/**/*.test.ts'], {extra} }};\n")
    };
    // A transform that is not known to hoist mock calls, automocking (also when the
    // value is not known), a custom runner, ES modules, a custom resolver: the
    // replacement does not count.
    for extra in [
        "transform: { '\\\\.tsx?$': '<rootDir>/transform.js' }",
        "transform: { '\\\\.tsx?$': 'esbuild-jest' }",
        "automock: true",
        "automock: process.env.AUTOMOCK === '1'",
        "runner: 'jest-runner-other'",
        "extensionsToTreatAsEsm: ['.ts']",
        "resolver: '<rootDir>/resolver.js'",
    ] {
        let d = repo(
            ("jest.config.js", &with(extra)),
            FACTORY,
            &[
                (
                    "transform.js",
                    "module.exports = { process: (src) => ({ code: src }) };\n",
                ),
                (
                    "resolver.js",
                    "module.exports = (request, options) => options.basedir + '/' + request;\n",
                ),
            ],
        );
        assert_eq!(
            selected(d.path(), "jest", "src/deep.ts", &[]),
            [TEST],
            "{extra}"
        );
    }
    // Transformers that hoist.
    for extra in [
        "transform: { '\\\\.tsx?$': '@swc/jest' }",
        "transform: { '^.+\\\\.(t|j)sx?$': ['babel-jest', { presets: [] }] }",
        "transform: { '\\\\.tsx?$': ['ts-jest', { isolatedModules: true }] }",
        // jest's own runner, named.
        "runner: 'jest-runner'",
    ] {
        let d = repo(("jest.config.js", &with(extra)), FACTORY, &[]);
        assert!(
            selected(d.path(), "jest", "src/deep.ts", &[]).is_empty(),
            "{extra}"
        );
    }
    // ts-jest with type checks reads the module's types.
    let d = repo(
        (
            "jest.config.js",
            &with("transform: { '\\\\.tsx?$': 'ts-jest' }"),
        ),
        FACTORY,
        &[],
    );
    assert_eq!(selected(d.path(), "jest", "src/deep.ts", &[]), [TEST]);
}

#[test]
fn vitest() {
    let mock = "import { vi } from 'vitest';\nvi.mock('../src/m', () => ({ v: 2 }));";
    let d = repo(("vitest.config.ts", VITEST), mock, &[]);
    assert!(selected(d.path(), "vitest", "src/deep.ts", &[]).is_empty());
    // The factory takes the original module; a spy wraps it.
    for partial in [
        "import { vi } from 'vitest';\nvi.mock('../src/m', async (importOriginal) => ({ ...(await \
         importOriginal()) }));",
        "import { vi } from 'vitest';\nvi.mock('../src/m', { spy: true });",
        "import { vi } from 'vitest';\nvi.mock('../src/m', () => ({ v: 2 }));\nconst real = await \
         vi.importActual('../src/m');",
    ] {
        let d = repo(("vitest.config.ts", VITEST), partial, &[]);
        assert_eq!(
            selected(d.path(), "vitest", "src/deep.ts", &[]),
            [TEST],
            "{partial}"
        );
    }
    // Test files that share a module registry.
    let shared = "export default { test: { include: ['test/**/*.test.ts'], isolate: false } };\n";
    let d = repo(("vitest.config.ts", shared), mock, &[]);
    assert_eq!(selected(d.path(), "vitest", "src/deep.ts", &[]), [TEST]);
    // Node's own `require` is not intercepted.
    let d = repo(
        ("vitest.config.ts", VITEST),
        "import { vi } from 'vitest';\nvi.mock('../src/n.cjs', () => ({ v: 2 }));",
        &[
            ("src/n.cjs", "module.exports = { v: 1 };\n"),
            (
                "src/user.ts",
                "import { createRequire } from 'node:module';\nconst require = \
                 createRequire(import.meta.url);\nconst n = require('./n.cjs');\nexport const use \
                 = () => n.v;\n",
            ),
        ],
    );
    assert_eq!(selected(d.path(), "vitest", "src/n.cjs", &[]), [TEST]);
}

/// Mock calls edited between two queries on the same graph directory: the
/// refreshed graph answers like a graph built from scratch. The composed graph
/// of the previous snapshot is reused across an edit that leaves the marker
/// edges as they are and composed again otherwise, and the derived mock layer
/// (the edges from modules to the manual mocks that helpers register, the tags
/// of the files with markers) is the new manifest's.
#[test]
fn mock_calls_edited_between_queries() {
    // The selection in the repository's own graph directory, which the queries
    // before it left behind, checked against a graph built from scratch for
    // this query.
    let both = |r: &Path, changed: &str| -> Vec<String> {
        let kept = selected(r, "jest", changed, &[]);
        let fresh = tempfile::tempdir().unwrap();
        let scratch = run(
            r,
            &[
                "affected",
                "--tests",
                "jest",
                "--changed",
                changed,
                "--graph",
                fresh.path().to_str().unwrap(),
            ],
            &[],
        );
        let scratch: Vec<String> = scratch.lines().map(str::to_owned).collect();
        assert_eq!(kept, scratch, "{changed}: refreshed against from scratch");
        kept
    };
    let why = |r: &Path, changed: &str, entry: &str| {
        run(
            r,
            &[
                "affected",
                "--tests",
                "jest",
                "--changed",
                changed,
                "--explain",
                entry,
            ],
            &[],
        )
    };
    let test = |mock: &str, tail: &str| {
        format!(
            "import {{ use }} from '../src/user';\n{mock}\ntest('a', () => {{ use(); }});\n{tail}"
        )
    };

    // A test's own mock.
    let d = repo(("jest.config.js", JEST), "", &[]);
    let r = d.path();
    assert_eq!(both(r, "src/deep.ts"), [TEST]);
    // Added.
    write(r, TEST, &test(FACTORY, ""));
    assert!(both(r, "src/deep.ts").is_empty());
    assert!(
        why(r, "src/deep.ts", TEST).contains("replaced by the mock at test/a.test.ts:2"),
        "added"
    );
    // Moved down two lines.
    let moved = format!("// one\n// two\n{FACTORY}");
    write(r, TEST, &test(&moved, ""));
    assert!(both(r, "src/deep.ts").is_empty());
    assert!(
        why(r, "src/deep.ts", TEST).contains("replaced by the mock at test/a.test.ts:4"),
        "moved"
    );
    // An edit below it (the composed graph is reused).
    write(r, TEST, &test(&moved, "test('b', () => {});\n"));
    assert!(both(r, "src/deep.ts").is_empty());
    // Undone by a real load, then removed.
    write(
        r,
        TEST,
        &test(
            &format!("{moved}\nconst real = jest.requireActual('../src/m');"),
            "",
        ),
    );
    assert_eq!(both(r, "src/deep.ts"), [TEST]);
    write(r, TEST, &test("", ""));
    assert_eq!(both(r, "src/deep.ts"), [TEST]);

    // A manual mock that a helper registers: `b` loads the helper and the module,
    // `c` the module only.
    const B: &str = "test/b.test.ts";
    const C: &str = "test/c.test.ts";
    const C_TEXT: &str = "import { n } from '../src/n';\ntest('c', () => { n; });\n";
    let d = repo(
        ("jest.config.js", JEST),
        "import '../src/helper';",
        &[
            ("src/helper.ts", "export const h = 1;\n"),
            ("src/n.ts", "export const n = 1;\n"),
            (
                "src/__mocks__/n.ts",
                "import { md } from '../mockdep';\nexport const n = md;\n",
            ),
            ("src/mockdep.ts", "export const md = 2;\n"),
            (
                B,
                "import '../src/helper';\nimport { n } from '../src/n';\ntest('b', () => { n; \
                 });\n",
            ),
            (C, C_TEXT),
        ],
    );
    let r = d.path();
    // Nobody registers the mock.
    assert!(both(r, "src/mockdep.ts").is_empty());
    // Registered.
    write(
        r,
        "src/helper.ts",
        "jest.mock('./n');\nexport const h = 1;\n",
    );
    assert_eq!(both(r, "src/mockdep.ts"), [B]);
    // Moved in the helper (the composed graph is reused: this marker names no
    // line).
    write(
        r,
        "src/helper.ts",
        "export const h = 1;\n\njest.mock('./n');\n",
    );
    assert_eq!(both(r, "src/mockdep.ts"), [B]);
    assert_eq!(both(r, "src/helper.ts"), [TEST, B]);
    assert!(
        why(r, "src/mockdep.ts", C).contains(
            "not selected through src/__mocks__/n.ts: the mock of src/n.ts for tests that load \
             src/helper.ts, which the test does not"
        ),
        "moved in the helper"
    );
    // The other test registers it for itself, then the helper stops.
    write(r, C, &format!("jest.mock('../src/n');\n{C_TEXT}"));
    assert_eq!(both(r, "src/mockdep.ts"), [B, C]);
    write(r, "src/helper.ts", "export const h = 1;\n");
    assert_eq!(both(r, "src/mockdep.ts"), [C]);
    write(r, C, C_TEXT);
    assert!(both(r, "src/mockdep.ts").is_empty());
}

/// Review finding: a module mapper
/// names the file that the runner loads for `./deep`. As the mapper of a config
/// file it is an edge of the importer, and a change of the mapped file selects
/// the test; as the mapper of the `jest` key of the manifest it was applied to
/// no import (`Fw::js_config_index` holds config files only), and the change
/// selected nothing. (jest 29.7 and 30.5 apply the manifest's mapper to the
/// imports of every module, measured.)
#[test]
fn the_module_mapper_of_a_manifest_config_selects() {
    let fake = [("src/fake.ts", "export const d = 2;\n")];
    let file = r#"module.exports = { testMatch: ['**/test/**/*.test.ts'], moduleNameMapper: { '^\\./deep$': '<rootDir>/src/fake.ts' } };"#;
    let d = repo(("jest.config.js", file), "", &fake);
    assert_eq!(
        selected(d.path(), "jest", "src/fake.ts", &[]),
        [TEST],
        "a config file"
    );
    let manifest = r#"{"name":"r","version":"1.0.0","jest":{"testMatch":["**/test/**/*.test.ts"],"moduleNameMapper":{"^\\./deep$":"<rootDir>/src/fake.ts"}}}"#;
    let d = repo(("package.json", manifest), "", &fake);
    // The manifest's config governs the test: it is a test of the runner.
    assert_eq!(
        selected(d.path(), "jest", "src/user.ts", &[]),
        [TEST],
        "the test is known"
    );
    assert_eq!(
        selected(d.path(), "jest", "src/fake.ts", &[]),
        [TEST],
        "the jest key of the manifest"
    );
}

/// The other things a config in a manifest gives to files that are no tests,
/// like a config file does: a manual mock of a package in the haste map
/// (`<rootDir>/__mocks__/<name>`, loaded for every import of the name), and a
/// mapper of a manifest below the root (a config of the directory it is in, for
/// the files below it).
#[test]
fn a_manifest_config_gives_what_a_config_file_gives() {
    // A haste mock: `src/m.ts` imports the package `pkg`.
    let haste = [
        (
            "src/m.ts",
            "import p from 'pkg';\nimport { d } from './deep';\nexport const v = [p, d];\n",
        ),
        ("__mocks__/pkg.js", "module.exports = 1;\n"),
    ];
    let file = "module.exports = { testMatch: ['**/test/**/*.test.ts'] };";
    let d = repo(("jest.config.js", file), "", &haste);
    assert_eq!(
        selected(d.path(), "jest", "__mocks__/pkg.js", &[]),
        [TEST],
        "a config file's haste mock"
    );
    let manifest =
        r#"{"name":"r","version":"1.0.0","jest":{"testMatch":["**/test/**/*.test.ts"]}}"#;
    let d = repo(("package.json", manifest), "", &haste);
    assert_eq!(
        selected(d.path(), "jest", "__mocks__/pkg.js", &[]),
        [TEST],
        "the haste mock of a manifest's config"
    );
    // A manifest below the root with a mapper for the imports of the files below
    // it: the test (at the root, under the root's config file) loads
    // `src/m.ts`.
    let below = [
        (
            "src/package.json",
            r#"{"jest":{"moduleNameMapper":{"^\\./deep$":"<rootDir>/fake.ts"}}}"#,
        ),
        ("src/fake.ts", "export const d = 2;\n"),
    ];
    let d = repo(("jest.config.js", file), "", &below);
    assert_eq!(
        selected(d.path(), "jest", "src/fake.ts", &[]),
        [TEST],
        "the mapper of a manifest below the root"
    );
    // A manifest without the key maps nothing.
    let plain = [
        ("src/package.json", r#"{"private":true}"#),
        ("src/fake.ts", "export const d = 2;\n"),
    ];
    let d = repo(("jest.config.js", file), "", &plain);
    assert!(
        selected(d.path(), "jest", "src/fake.ts", &[]).is_empty(),
        "no key, no mapper"
    );
}

/// Review probe: a file that a config
/// names as a setup file or an environment is a symbolic link to a file. The
/// runner loads what the link leads to, so a change of that file changes the
/// run; the graph records the link's path only, and the change selects nothing.
/// (With git: a setup file that is a link to nothing whose target appears
/// selects nothing either, `--strict` included; refreshes see it.)
#[test]
#[ignore = "probe: a setup file behind a link is not followed to its target"]
fn a_setup_file_that_is_a_link_selects_through_what_it_leads_to() {
    let link = |r: &Path, at: &str| {
        let p = r.join(at);
        let _ = std::fs::remove_file(&p);
        turborepo_trace_core::os::symlink("lib/real.js", p).unwrap();
    };
    let real = [("lib/real.js", "globalThis.s = 1;\n")];
    let mut failures = Vec::new();
    for (runner, config) in [
        (
            "jest",
            (
                "jest.config.js",
                "module.exports = { testMatch: ['**/test/**/*.test.ts'], setupFiles: \
                 ['<rootDir>/setup.js'] };\n",
            ),
        ),
        (
            "jest",
            (
                "jest.config.js",
                "module.exports = { testMatch: ['**/test/**/*.test.ts'], setupFilesAfterEnv: \
                 ['<rootDir>/setup.js'] };\n",
            ),
        ),
        (
            "jest",
            (
                "jest.config.js",
                "module.exports = { testMatch: ['**/test/**/*.test.ts'], testEnvironment: \
                 '<rootDir>/setup.js' };\n",
            ),
        ),
        (
            "vitest",
            (
                "vitest.config.ts",
                "export default { test: { include: ['test/**/*.test.ts'], setupFiles: \
                 ['./setup.js'] } };\n",
            ),
        ),
    ] {
        let d = repo(config, "", &real);
        link(d.path(), "setup.js");
        if selected(d.path(), runner, "setup.js", &[]) != [TEST] {
            failures.push(format!("{runner} {}: the link itself", config.1.trim()));
        }
        if selected(d.path(), runner, "lib/real.js", &[]) != [TEST] {
            failures.push(format!("{runner} {}: what it leads to", config.1.trim()));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
