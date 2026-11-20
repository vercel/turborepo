//! `affected` end to end for where vitest runs start.
//!
//! vitest's root is the directory a run is started in (or `--root`): the base
//! of its patterns and the working directory of its tests. The config is the
//! one in that directory, else (vitest up to 4) the one of the nearest
//! directory above, else none. A config directory is a project whatever starts
//! where; the runs that start add working directories and tests.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{path::Path, process::Command};

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// The output lines of `harness affected <args>` in `root`, with `env` set.
fn run(root: &Path, args: &[&str], env: &[(&str, &str)]) -> Vec<String> {
    let mut c = Command::new(support::harness());
    c.args(["affected", "--tests", "all"])
        .args(args)
        .current_dir(root);
    for (k, v) in env {
        c.env(k, v);
    }
    let o = c.output().expect("run the harness");
    assert!(
        o.status.success(),
        "turbotrace affected {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// The tests a change of `changed` selects.
fn affected(root: &Path, changed: &str) -> Vec<String> {
    let mut out = run(root, &["--changed", changed], &[]);
    out.sort();
    out
}

/// A test that reads `fixtures/data.json` below its working directory.
const READS_CWD: &str =
    "const fs = require('node:fs');\nconst path = require('node:path');\ntest('data', () => {\n  \
     fs.readFileSync(path.join(process.cwd(), 'fixtures/data.json'), 'utf8');\n});\n";

/// Whether the selection document for a change of `changed` lists `file` as a
/// test that runs: selected, or an unknown test (which always runs).
fn runs_it(root: &Path, changed: &str, file: &str, env: &[(&str, &str)]) -> bool {
    let doc = run(root, &["--format", "selection", "--changed", changed], env).join("\n");
    let quoted = format!("\"{file}\"");
    ["\"selected\": [", "\"unknown\": ["].iter().any(|key| {
        doc.match_indices(key).any(|(i, _)| {
            // The list, to the bracket that closes it.
            let rest = &doc[i + key.len()..];
            let mut depth = 1;
            let end = rest
                .char_indices()
                .find(|(_, c)| {
                    match c {
                        '[' => depth += 1,
                        ']' => depth -= 1,
                        _ => {}
                    }
                    depth == 0
                })
                .map_or(rest.len(), |(at, _)| at);
            rest[..end].contains(&quoted)
        })
    })
}

const UNKNOWN: &str = "an unknown test, which always runs";

/// The lines of `--explain` for `file` under a change of `changed`.
fn explain(root: &Path, changed: &str, file: &str) -> String {
    run(root, &["--changed", changed, "--explain", file], &[]).join("\n")
}

// ---------------------------------------------------------------------------
// A config directory is a project.

/// ```text
/// package.json                 the root scripts under test
/// vite.config.ts               formatting settings only: vitest would read it as a config
/// packages/tools/index.js      locate(): find(process.cwd()), find is not followed; load(p): fs.readFileSync(p)
/// packages/math/src/add.js     a file nothing imports
/// apps/web/jest.config.js      a jest project, started in apps/web by its `test` script
/// apps/web/test/web.test.js    loads packages/tools: reads what the working directory names
/// apps/web/data/a.json
/// ```
///
/// The working directory escapes to code that is not followed, so the test
/// reads anything below the directories its runs are started in: a file of
/// another package selects it exactly when the repository root is one of them.
fn root_config(root_scripts: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", &manifest(root_scripts));
    write(
        r,
        "pnpm-workspace.yaml",
        "packages:\n  - 'apps/*'\n  - 'packages/*'\n",
    );
    write(
        r,
        "vite.config.ts",
        "export default { fmt: { semi: false } };\n",
    );
    write(
        r,
        "packages/tools/package.json",
        r#"{"name":"tools","version":"1.0.0"}"#,
    );
    write(
        r,
        "packages/tools/index.js",
        "const fs = require('fs');\nexports.locate = function (find) {\n  return \
         find(process.cwd());\n};\nexports.load = function (p) {\n  return fs.readFileSync(p, \
         'utf8');\n};\n",
    );
    write(
        r,
        "packages/math/package.json",
        r#"{"name":"math","version":"1.0.0"}"#,
    );
    write(
        r,
        "packages/math/src/add.js",
        "exports.add = (a, b) => a + b;\n",
    );
    write(
        r,
        "apps/web/package.json",
        r#"{"name":"web","version":"1.0.0","scripts":{"test":"jest"}}"#,
    );
    write(r, "apps/web/jest.config.js", "module.exports = {};\n");
    write(r, "apps/web/data/a.json", "{}\n");
    write(
        r,
        "apps/web/test/web.test.js",
        "const { locate, load } = require('../../../packages/tools/index.js');\ntest('t', () => { \
         load(locate(globalThis.find) + globalThis.name); });\n",
    );
    d
}

fn manifest(scripts: &str) -> String {
    format!(r#"{{"name":"r","version":"1.0.0","private":true,"scripts":{{{scripts}}}}}"#)
}

const WEB: &str = "apps/web/test/web.test.js";
/// A file outside the package of the test: below the repository root only.
const ELSEWHERE: &str = "packages/math/src/add.js";

/// A config claims the files below it and gives them its directory as a
/// working directory, whether or not anything is seen to start vitest there.
#[test]
fn a_config_directory_is_a_project_whatever_starts_there() {
    for scripts in ["", r#""test":"vitest run""#, r#""lint:rules":"sg test""#] {
        let d = root_config(scripts);
        let r = d.path();
        assert_eq!(affected(r, "apps/web/data/a.json"), [WEB], "{scripts}");
        assert_eq!(affected(r, ELSEWHERE), [WEB], "{scripts}");
    }
}

// ---------------------------------------------------------------------------
// A run started below its config.

/// ```text
/// package.json                       the root scripts under test
/// vite.config.ts                     the config under test
/// fixtures/data.json
/// packages/math/package.json         the scripts and the vitest version under test
/// packages/math/src/add.test.ts      reads fixtures/data.json below the working directory
/// packages/math/checks/shape.test.ts the same
/// packages/math/fixtures/data.json
/// ```
fn below(config: &str, root_scripts: &str, math_scripts: &str, vitest: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", &manifest(root_scripts));
    write(r, "pnpm-workspace.yaml", "packages:\n  - 'packages/*'\n");
    write(r, "vite.config.ts", config);
    write(r, "fixtures/data.json", "{}\n");
    let dev = if vitest.is_empty() {
        String::new()
    } else {
        format!(r#","devDependencies":{{"vitest":"{vitest}"}}"#)
    };
    write(
        r,
        "packages/math/package.json",
        &format!(r#"{{"name":"math","version":"1.0.0","scripts":{{{math_scripts}}}{dev}}}"#),
    );
    write(r, "packages/math/fixtures/data.json", "{}\n");
    let test = "import fs from 'node:fs';\nimport path from 'node:path';\nimport { test } from \
                'vitest';\ntest('data', () => {\n  fs.readFileSync(path.join(process.cwd(), \
                'fixtures/data.json'), 'utf8');\n});\n";
    write(r, "packages/math/src/add.test.ts", test);
    write(r, "packages/math/checks/shape.test.ts", test);
    d
}

const ADD: &str = "packages/math/src/add.test.ts";
const SHAPE: &str = "packages/math/checks/shape.test.ts";
const CHECK: &str = "packages/math/src/shape.check.ts";
const MATH_DATA: &str = "packages/math/fixtures/data.json";
const PLAIN: &str = "export default {};\n";
const CHECKS_ONLY: &str = "export default { test: { include: ['src/**/*.check.ts'] } };\n";
const RUN: &str = r#""test":"vitest run""#;

/// The tests of a config above run in the package the run is started in: a
/// file they read below the working directory is the package's. Whatever the
/// version: up to 4 the config above is loaded with the package as root, from
/// 5 on the run has the defaults.
#[test]
fn a_run_below_a_config_has_its_directory_as_working_directory() {
    for version in ["3.2.4", "^4.0.18", "5.1.0", "", "^3.2.0 || ^4.0.0"] {
        let d = below(PLAIN, "", RUN, version);
        assert_eq!(
            affected(d.path(), MATH_DATA),
            [SHAPE, ADD],
            "vitest {version:?}"
        );
    }
    // Without a run in the package, the config's directory is the only one.
    let d = below(PLAIN, "", "", "^4.0.18");
    assert_eq!(affected(d.path(), MATH_DATA), Vec::<String>::new());
}

#[test]
fn a_root_option_names_the_directory_the_run_is_rooted_at() {
    let d = below(
        PLAIN,
        r#""test:math":"vitest run --root packages/math""#,
        "",
        "^4.0.18",
    );
    assert_eq!(affected(d.path(), MATH_DATA), [SHAPE, ADD]);
}

/// The patterns of a config above are matched from the directory the run is
/// started in (vitest up to 4): a file they take from there is a test of the
/// run, also when they do not take it from the config's own directory.
#[test]
fn a_file_the_config_above_takes_from_the_run_is_its_test() {
    for version in ["3.2.4", "^4.0.18", "", "^3.2.0 || ^4.0.0", ">=3"] {
        let d = below(CHECKS_ONLY, "", RUN, version);
        let r = d.path();
        write(r, CHECK, READS_CWD);
        assert!(
            runs_it(r, CHECK, CHECK, &[]),
            "vitest {version:?}: the file"
        );
        assert!(
            runs_it(r, MATH_DATA, CHECK, &[]),
            "vitest {version:?}: what it reads"
        );
    }
    // vitest 5 loads no config of a directory above: the file is no test.
    let d = below(CHECKS_ONLY, "", RUN, "5.1.0");
    let r = d.path();
    write(r, CHECK, READS_CWD);
    assert!(!runs_it(r, CHECK, CHECK, &[]));
    // Nor is it one without a run in the package.
    let d = below(CHECKS_ONLY, "", "", "^4.0.18");
    let r = d.path();
    write(r, CHECK, READS_CWD);
    assert!(!runs_it(r, CHECK, CHECK, &[]));
}

/// A file that looks like a test and that the config leaves out from its own
/// directory always runs: it stays so when a run below takes it.
#[test]
fn a_test_file_the_config_leaves_out_always_runs() {
    for version in ["^4.0.18", "5.1.0", ""] {
        let d = below(
            "export default { test: { include: ['src/**/*.test.ts'] } };\n",
            "",
            RUN,
            version,
        );
        let r = d.path();
        for file in [ADD, SHAPE] {
            assert!(
                explain(r, MATH_DATA, file).contains(UNKNOWN),
                "{version:?} {file}"
            );
            assert!(runs_it(r, "package.json", file, &[]), "{version:?} {file}");
        }
    }
}

/// `vp test` executes the vitest vite-plus depends on, whatever the package
/// declares for `vitest`.
#[test]
fn a_vite_plus_run_has_the_vitest_of_vite_plus() {
    let vp = r#""test":"vp test run""#;
    // Not installed, nothing says which it is: the config above may be loaded.
    let d = below(CHECKS_ONLY, "", vp, "5.1.0");
    let r = d.path();
    write(r, CHECK, READS_CWD);
    assert!(runs_it(r, MATH_DATA, CHECK, &[]));
    // Installed, it says so: vitest 5 loads no config of a directory above.
    write(
        r,
        "node_modules/vite-plus/package.json",
        r#"{"name":"vite-plus","version":"1.7.0","dependencies":{"vitest":"5.1.0"}}"#,
    );
    assert!(!runs_it(r, MATH_DATA, CHECK, &[]));
}

#[test]
fn a_catalog_says_the_version() {
    for (range, is_test) in [("^4.0.18", true), ("^5.1.0", false)] {
        let d = below(CHECKS_ONLY, "", RUN, "catalog:");
        let r = d.path();
        write(
            r,
            "pnpm-workspace.yaml",
            &format!("packages:\n  - 'packages/*'\ncatalog:\n  vitest: {range}\n"),
        );
        write(r, CHECK, READS_CWD);
        assert_eq!(runs_it(r, MATH_DATA, CHECK, &[]), is_test, "{range}");
    }
}

/// `-c` is resolved against `--root` (vitest 2 to 5), and the files the named
/// config takes from the root are tests of the run: only the task knows the
/// config, so they always run.
#[test]
fn a_named_config_is_resolved_against_the_root_option() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(
        r,
        "package.json",
        &manifest(r#""test":"vitest run --root packages/math -c alt/vitest.config.mjs""#),
    );
    write(r, "pnpm-workspace.yaml", "packages:\n  - 'packages/*'\n");
    write(
        r,
        "packages/math/package.json",
        r#"{"name":"math","version":"1.0.0","devDependencies":{"vitest":"^4.0.18"}}"#,
    );
    write(
        r,
        "packages/math/alt/vitest.config.mjs",
        "export default { test: { include: ['checks/**/*.check.js'] } };\n",
    );
    let check = "packages/math/checks/shape.check.js";
    write(r, check, READS_CWD);
    assert!(runs_it(r, check, check, &[]));
    assert!(runs_it(r, "package.json", check, &[]));
}

/// A command in a package below a config that is not recognized and names
/// vitest: its directory is a working directory of the tests below it.
#[test]
fn a_command_that_names_vitest_below_a_config_adds_its_directory() {
    let d = below(PLAIN, "", r#""test":"node ./run-tests.mjs""#, "^4.0.18");
    let r = d.path();
    write(
        r,
        "packages/math/run-tests.mjs",
        "import { execSync } from 'node:child_process';\nexecSync('vitest run', { stdio: \
         'inherit' });\n",
    );
    assert_eq!(affected(r, MATH_DATA), [SHAPE, ADD]);
    // The runner by the path of its program, and through wireit.
    for math in [
        r#""test":"node ../../node_modules/vitest/vitest.mjs run""#,
        r#""test":"node node_modules/.bin/vitest run""#,
        r#""test":"wireit"},"wireit":{"test":{"command":"vitest run"}"#,
    ] {
        let d = below(PLAIN, "", math, "^4.0.18");
        assert_eq!(affected(d.path(), MATH_DATA), [SHAPE, ADD], "{math}");
    }
}

/// A run started below the config looks for the mock of a package in its own
/// root.
#[test]
fn a_run_below_a_config_takes_package_mocks_from_its_directory() {
    let d = below(PLAIN, "", RUN, "^4.0.18");
    let r = d.path();
    let test = "packages/math/src/pad.test.ts";
    write(
        r,
        test,
        "import pad from 'left-pad';\nimport { expect, test, vi } from \
         'vitest';\nvi.mock('left-pad');\ntest('pad', () => {\n  expect(pad('a', \
         2)).toBe('mock');\n});\n",
    );
    write(
        r,
        "node_modules/left-pad/package.json",
        r#"{"name":"left-pad","version":"1.0.0","main":"index.js"}"#,
    );
    write(
        r,
        "node_modules/left-pad/index.js",
        "module.exports = () => 1;\n",
    );
    write(
        r,
        "packages/math/__mocks__/left-pad.js",
        "export default () => 'mock';\n",
    );
    assert_eq!(affected(r, "packages/math/__mocks__/left-pad.js"), [test]);
}

/// A script that adds or removes a start changes the working directories of a
/// graph that is refreshed, as it does those of a new graph.
#[test]
fn a_refreshed_graph_follows_the_starts() {
    let d = below(PLAIN, "", "", "^4.0.18");
    let r = d.path();
    let scratch = tempfile::tempdir().unwrap();
    let cold = std::cell::Cell::new(0);
    let both = |what: &str| -> Vec<String> {
        let refreshed = affected(r, MATH_DATA);
        cold.set(cold.get() + 1);
        let graph = scratch.path().join(format!("cold-{}", cold.get()));
        let mut fresh = run(
            r,
            &["--changed", MATH_DATA, "--graph", graph.to_str().unwrap()],
            &[],
        );
        fresh.sort();
        assert_eq!(refreshed, fresh, "{what}: refreshed and cold");
        refreshed
    };
    let math = |scripts: &str| {
        write(
            r,
            "packages/math/package.json",
            &format!(
                r#"{{"name":"math","version":"1.0.0","scripts":{{{scripts}}},"devDependencies":{{"vitest":"^4.0.18"}}}}"#
            ),
        );
    };
    assert_eq!(both("nothing starts in the package"), Vec::<String>::new());
    math(RUN);
    assert_eq!(both("a script in the package"), [SHAPE, ADD]);
    math("");
    assert_eq!(both("the script removed"), Vec::<String>::new());
    // A CI step started in the package.
    write(
        r,
        ".github/workflows/ci.yml",
        "name: ci\non: push\njobs:\n  test:\n    runs-on: ubuntu-latest\n    steps:\n      - \
         uses: actions/checkout@v4\n      - run: npx vitest run\n        working-directory: \
         packages/math\n",
    );
    assert_eq!(both("a CI step"), [SHAPE, ADD]);
    std::fs::remove_file(r.join(".github/workflows/ci.yml")).unwrap();
    assert_eq!(both("the CI step removed"), Vec::<String>::new());
}

// ---------------------------------------------------------------------------
// The default exclude list by version.

/// A package with a config of its own that takes `*.check.js`, and files below
/// `dist/`.
fn dist(vitest: &str) -> tempfile::TempDir {
    let d = below(PLAIN, "", RUN, vitest);
    let r = d.path();
    std::fs::remove_file(r.join("vite.config.ts")).unwrap();
    write(
        r,
        "packages/math/vitest.config.ts",
        "export default { test: { include: ['**/*.check.js', '**/*.test.{js,ts}'] } };\n",
    );
    write(r, BUILT_CHECK, READS_CWD);
    write(r, BUILT_TEST, READS_CWD);
    d
}

const BUILT_CHECK: &str = "packages/math/dist/built.check.js";
const BUILT_TEST: &str = "packages/math/dist/built.test.js";

/// vitest up to 3 leaves out what is below `dist/`; 4 and later run it, and so
/// does a version nothing says.
#[test]
fn the_default_exclude_list_is_the_one_of_the_version() {
    let d = dist("3.2.4");
    assert!(!runs_it(d.path(), MATH_DATA, BUILT_CHECK, &[]));
    for version in ["^4.0.18", "5.1.0", ""] {
        let d = dist(version);
        assert!(
            runs_it(d.path(), MATH_DATA, BUILT_CHECK, &[]),
            "{version:?}: {}",
            explain(d.path(), MATH_DATA, BUILT_CHECK)
        );
    }
}

/// A file below `dist/` that looks like a test always runs, whatever the
/// version.
#[test]
fn a_test_file_below_dist_always_runs() {
    for version in ["3.2.4", "^4.0.18", ""] {
        let d = dist(version);
        let r = d.path();
        assert!(
            explain(r, MATH_DATA, BUILT_TEST).contains(UNKNOWN),
            "{version:?}"
        );
        assert!(runs_it(r, "package.json", BUILT_TEST, &[]), "{version:?}");
    }
}

// ---------------------------------------------------------------------------
// A run without a config.

/// ```text
/// package.json              `test`: vitest run, no config anywhere
/// lib/other.ts
/// lib/__mocks__/other.ts    what `vi.mock("../lib/other")` loads
/// e2e/b.test.ts             mocks ../lib/other, has a snapshot
/// e2e/__snapshots__/b.test.ts.snap
/// ```
fn config_less(scripts: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(
        r,
        "package.json",
        &format!(
            r#"{{"name":"r","version":"1.0.0","private":true,"scripts":{{{scripts}}},"devDependencies":{{"vitest":"^4.0.18"}}}}"#
        ),
    );
    write(r, "lib/other.ts", "export const o = 1;\n");
    write(r, "lib/__mocks__/other.ts", "export const o = 2;\n");
    write(r, "lib/index.ts", "export const v = 1;\n");
    write(
        r,
        "e2e/b.test.ts",
        "import { expect, test, vi } from 'vitest';\nimport { o } from '../lib/other';\nimport { \
         v } from '@lib';\nvi.mock('../lib/other');\ntest('t', () => {\n  expect([o, \
         v]).toMatchSnapshot();\n});\n",
    );
    write(
        r,
        "e2e/__snapshots__/b.test.ts.snap",
        "// Vitest Snapshot v1\n\nexports[`t 1`] = `2`;\n",
    );
    d
}

const E2E: &str = "e2e/b.test.ts";

/// A test of a run without a config always runs: also when its snapshot file
/// or the `__mocks__` file it mocks with changes.
#[test]
fn a_test_of_a_run_without_a_config_always_runs() {
    let d = config_less(RUN);
    let r = d.path();
    for changed in [
        "e2e/__snapshots__/b.test.ts.snap",
        "lib/__mocks__/other.ts",
        "package.json",
    ] {
        assert!(runs_it(r, changed, E2E, &[]), "{changed}");
    }
}

/// A file that a run without a config takes and that a run with a config
/// named on the command line takes too always runs: also when the setup file,
/// the alias target or the config of the second run changes.
#[test]
fn a_test_of_a_run_with_a_named_config_always_runs() {
    let d = config_less(r#""test":"vitest run","test:e2e":"vitest run -c vitest.e2e.config.ts""#);
    let r = d.path();
    write(
        r,
        "vitest.e2e.config.ts",
        "import path from 'node:path';\nexport default {\n  resolve: { alias: { '@lib': \
         path.resolve(__dirname, 'lib/index.ts') } },\n  test: { include: ['e2e/**/*.test.ts'], \
         setupFiles: ['./e2e/setup.ts'] },\n};\n",
    );
    write(r, "e2e/setup.ts", "globalThis.ready = true;\n");
    for changed in ["e2e/setup.ts", "lib/index.ts", "vitest.e2e.config.ts"] {
        assert!(runs_it(r, changed, E2E, &[]), "{changed}");
    }
}

// ---------------------------------------------------------------------------
// A package with a config whose vitest is started from elsewhere.

/// ```text
/// package.json                   `test:quick`: vitest run (no config at the root: the defaults) and the scripts under test
/// packages/math/package.json     the scripts under test
/// packages/math/vitest.config.ts
/// packages/math/test/data.test.ts  reads fixtures/data.json below the working directory
/// packages/math/fixtures/data.json
/// ```
///
/// The root run takes the test with the root as its working directory; the
/// run under test starts vitest in `packages/math`.
fn two_runs(root_scripts: &str, math_scripts: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    let more = if root_scripts.is_empty() {
        String::new()
    } else {
        format!(",{root_scripts}")
    };
    write(
        r,
        "package.json",
        &format!(
            r#"{{"name":"r","version":"1.0.0","private":true,"scripts":{{"test:quick":"vitest run"{more}}},"devDependencies":{{"vitest":"5.1.0"}}}}"#
        ),
    );
    write(r, "pnpm-workspace.yaml", "packages:\n  - 'packages/*'\n");
    write(
        r,
        "packages/math/package.json",
        &format!(r#"{{"name":"math","version":"1.0.0","scripts":{{{math_scripts}}}}}"#),
    );
    write(r, "packages/math/vitest.config.ts", "export default {};\n");
    write(
        r,
        "packages/math/test/data.test.ts",
        "import fs from 'node:fs';\nimport path from 'node:path';\nimport { test } from \
         'vitest';\ntest('data', () => {\n  fs.readFileSync(path.join(process.cwd(), \
         'fixtures/data.json'), 'utf8');\n});\n",
    );
    write(r, "packages/math/fixtures/data.json", "{}\n");
    d
}

const DATA_TEST: &str = "packages/math/test/data.test.ts";

/// The directory of a config is a working directory of its tests however
/// vitest is started there: by a script in the package, by a command at the
/// root that changes directory, through a wrapper that is not followed, by the
/// path of the runner's program, by a tool that infers its targets, or by a CI
/// system that is not read.
#[test]
fn the_directory_of_a_config_stays_a_working_directory() {
    let d = two_runs("", RUN);
    assert_eq!(affected(d.path(), MATH_DATA), [DATA_TEST]);
    for root in [
        "",
        r#""t":"cd packages/math && vitest run""#,
        r#""t":"pnpm --dir packages/math exec vitest run""#,
        r#""t":"pnpm -r exec vitest run""#,
        r#""t":"vp -C packages/math test run""#,
        r#""t":"lerna exec -- vitest run""#,
        r#""t":"yarn workspace math vitest run""#,
        r#""t":"yarn workspaces foreach -A exec vitest run""#,
        r#""t":"docker compose run --rm -w /app/packages/math app vitest run""#,
        r#""t":"nx run-many -t test""#,
        r#""t":"pnpm nx run-many -t test""#,
        r#""t":"node scripts/test-all.mjs""#,
    ] {
        let d = two_runs(root, "");
        assert_eq!(affected(d.path(), MATH_DATA), [DATA_TEST], "{root}");
    }
    for math in [
        r#""test":"node ../../node_modules/vitest/vitest.mjs run""#,
        r#""test":"node node_modules/.bin/vitest run""#,
        r#""test":"wireit"},"wireit":{"test":{"command":"vitest run"}"#,
    ] {
        let d = two_runs("", math);
        assert_eq!(affected(d.path(), MATH_DATA), [DATA_TEST], "{math}");
    }
    let d = two_runs("", "");
    write(
        d.path(),
        ".gitlab-ci.yml",
        "test:\n  script:\n    - cd packages/math\n    - npx vitest run\n",
    );
    assert_eq!(affected(d.path(), MATH_DATA), [DATA_TEST], "gitlab");
}
