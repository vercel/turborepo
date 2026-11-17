//! `affected` end to end for module loads whose specifier is known
//! in part or as a set of values: a pattern that is not expanded keeps an open
//! record, and a module mapper of the test runner applies to every value of a
//! reference. Both cases come from the multi-file runtime oracle (shapes `bare`
//! and `alias`).
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{path::Path, process::Command};

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn affected(root: &Path, changed: &str) -> Vec<String> {
    affected_env(root, changed, &[])
}

/// Without cross-file binding: every value of a reference counts for whoever
/// loads the file of the reference.
fn affected_unbound(root: &Path, changed: &str) -> Vec<String> {
    affected_env(root, changed, &[("TURBO_TRACE_DEBUG_BIND", "0")])
}

fn affected_env(root: &Path, changed: &str, env: &[(&str, &str)]) -> Vec<String> {
    let o = Command::new(support::harness())
        .args(["affected", "--tests", "jest", "--changed", changed])
        .envs(env.iter().copied())
        .current_dir(root)
        .output()
        .expect("run the harness");
    assert!(
        o.status.success(),
        "turbotrace affected --changed {changed} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// A workspace of two packages, linked into `node_modules` the way npm does it.
fn workspace(files: &[(&str, &str)]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(
        r,
        "package.json",
        r#"{"name":"root","version":"1.0.0","private":true,"workspaces":["pkgs/*"]}"#,
    );
    write(r, "README.md", "x\n");
    for p in ["p1", "p2"] {
        write(
            r,
            &format!("pkgs/{p}/package.json"),
            &format!(r#"{{"name":"{p}","version":"1.0.0"}}"#),
        );
        write(
            r,
            &format!("pkgs/{p}/jest.config.js"),
            "module.exports = {};\n",
        );
    }
    std::fs::create_dir_all(r.join("node_modules")).unwrap();
    for p in ["p1", "p2"] {
        turborepo_trace_core::os::symlink(format!("../pkgs/{p}"), r.join("node_modules").join(p))
            .unwrap();
    }
    for (rel, content) in files {
        write(r, rel, content);
    }
    d
}

/// `require('p' + name)`: the start of a package name is in the helper, the
/// rest comes from the caller, and the module is a workspace package. The
/// pattern is not expanded (it would span `node_modules`), so the site keeps an
/// open record: every test that reaches the helper is selected, and a test that
/// does not is not.
#[test]
fn a_package_name_known_in_part_keeps_an_open_record() {
    let d = workspace(&[
        (
            "pkgs/p1/lib/helper.js",
            "export function load(name) {\n  return require('p' + name);\n}\n",
        ),
        (
            "pkgs/p1/test/a.test.js",
            "import { load } from '../lib/helper';\ntest('t', () => {\n  load('2');\n});\n",
        ),
        (
            "pkgs/p1/test/b.test.js",
            "import { load } from '../lib/helper';\ntest('t', () => {\n  load('2/lib/x');\n});\n",
        ),
        (
            "pkgs/p1/test/plain.test.js",
            "test('t', () => {\n  expect(1).toBe(1);\n});\n",
        ),
        ("pkgs/p2/index.js", "module.exports = { name: 'index' };\n"),
        ("pkgs/p2/lib/x.js", "module.exports = { name: 'x' };\n"),
    ]);
    let r = d.path();
    const USERS: [&str; 2] = ["pkgs/p1/test/a.test.js", "pkgs/p1/test/b.test.js"];
    assert_eq!(affected(r, "pkgs/p2/index.js"), USERS);
    assert_eq!(affected(r, "pkgs/p2/lib/x.js"), USERS);
    // The record is open: an unrelated edit selects the tests that reach the helper
    // too, and nothing else.
    assert_eq!(affected(r, "README.md"), USERS);
}

/// The same inside `try`: an optional load of a pattern is no less open.
#[test]
fn an_optional_load_of_a_partial_package_name_is_open_too() {
    let d = workspace(&[
        (
            "pkgs/p1/lib/helper.js",
            "const load = (name) => {\n  try {\n    return require('p' + name);\n  } catch {\n    \
             return null;\n  }\n};\nmodule.exports.load = load;\n",
        ),
        (
            "pkgs/p1/test/a.test.js",
            "const { load } = require('../lib/helper');\ntest('t', () => {\n  load('2');\n});\n",
        ),
        (
            "pkgs/p1/test/plain.test.js",
            "test('t', () => {\n  expect(1).toBe(1);\n});\n",
        ),
        ("pkgs/p2/index.js", "module.exports = { name: 'index' };\n"),
    ]);
    let r = d.path();
    assert_eq!(affected(r, "pkgs/p2/index.js"), ["pkgs/p1/test/a.test.js"]);
}

const MAPPER: &str = "module.exports = {\n  moduleNameMapper: { '^@shared/(.*)$': \
                      '<rootDir>/../../shared/$1' },\n};\n";
const TABLE: &str = "const T = {\n  a: '@shared/routes/a',\n  b: '@shared/routes/b',\n};\n";

/// A table whose values are aliases of the jest configuration
/// (`moduleNameMapper`). The value-flow engine turns each load into one
/// reference with both values; the mapper applies to each of them, whether the
/// values leave the table by key, through `Object.entries`, `Object.values` or
/// a `for…in` loop.
#[test]
fn a_module_mapper_applies_to_every_value_of_a_reference() {
    let by_key = format!("{TABLE}export function load(key) {{\n  return require(T[key]);\n}}\n");
    let entries = format!(
        "{TABLE}export function except(skip) {{\n  return Object.entries(T)\n    .filter(([k]) => \
         k !== skip)\n    .map(([, f]) => require(f));\n}}\n"
    );
    let values = format!(
        "{TABLE}export function all() {{\n  return Object.values(T).map((f) => require(f));\n}}\n"
    );
    let keys = format!(
        "{TABLE}export function all() {{\n  const out = [];\n  for (const k in T) \
         out.push(require(T[k]));\n  return out;\n}}\n"
    );
    let d = workspace(&[
        ("pkgs/p1/jest.config.js", MAPPER),
        ("pkgs/p1/lib/by-key.js", &by_key),
        ("pkgs/p1/lib/entries.js", &entries),
        ("pkgs/p1/lib/values.js", &values),
        ("pkgs/p1/lib/keys.js", &keys),
        (
            "pkgs/p1/test/a.test.js",
            "import { load } from '../lib/by-key';\ntest('t', () => {\n  load('a');\n});\n",
        ),
        (
            "pkgs/p1/test/b.test.js",
            "import { except } from '../lib/entries';\ntest('t', () => {\n  except('b');\n});\n",
        ),
        (
            "pkgs/p1/test/c.test.js",
            "import { all } from '../lib/values';\ntest('t', () => {\n  all();\n});\n",
        ),
        (
            "pkgs/p1/test/d.test.js",
            "import { all } from '../lib/keys';\ntest('t', () => {\n  all();\n});\n",
        ),
        (
            "pkgs/p1/test/plain.test.js",
            "test('t', () => {\n  expect(1).toBe(1);\n});\n",
        ),
        ("shared/routes/a.js", "module.exports = { name: 'a' };\n"),
        ("shared/routes/b.js", "module.exports = { name: 'b' };\n"),
        ("shared/routes/c.js", "module.exports = { name: 'c' };\n"),
    ]);
    let r = d.path();
    const USERS: [&str; 4] = [
        "pkgs/p1/test/a.test.js",
        "pkgs/p1/test/b.test.js",
        "pkgs/p1/test/c.test.js",
        "pkgs/p1/test/d.test.js",
    ];
    assert_eq!(affected_unbound(r, "shared/routes/a.js"), USERS);
    assert_eq!(affected_unbound(r, "shared/routes/b.js"), USERS);
    // With cross-file binding (the default) the test that passes the key `a` loads
    // that value alone; the mapper still applies to it and to every value of
    // the others.
    assert_eq!(affected(r, "shared/routes/a.js"), USERS);
    assert_eq!(affected(r, "shared/routes/b.js"), USERS[1..]);
    // Every value is mapped, so nothing is open: a file the table does not name and
    // an unrelated edit select nothing.
    assert_eq!(affected(r, "shared/routes/c.js"), [""; 0]);
    assert_eq!(affected(r, "README.md"), [""; 0]);
}

const PLAIN: (&str, &str) = (
    "pkgs/p1/test/plain.test.js",
    "test('t', () => {\n  expect(1).toBe(1);\n});\n",
);
const USER: (&str, &str) = (
    "pkgs/p1/test/a.test.js",
    "import { load } from '../lib/helper';\ntest('t', () => {\n  load('a');\n});\n",
);

/// A pattern whose literal start is an alias of the governing tsconfig
/// (`paths`): it is expanded below the mapped directory, like a relative
/// pattern.
#[test]
fn a_pattern_with_a_tsconfig_paths_prefix_is_expanded() {
    let d = workspace(&[
        (
            "pkgs/p1/tsconfig.json",
            r#"{"compilerOptions":{"paths":{"@/*":["./src/*"]}}}"#,
        ),
        (
            "pkgs/p1/lib/helper.js",
            "export function load(name) {\n  return import(`@/guides/${name}.js`);\n}\n",
        ),
        USER,
        PLAIN,
        ("pkgs/p1/src/guides/a.js", "export const name = 'a';\n"),
        ("pkgs/p1/src/guides/b.js", "export const name = 'b';\n"),
        ("pkgs/p1/src/other/c.js", "export const name = 'c';\n"),
    ]);
    let r = d.path();
    assert_eq!(affected(r, "pkgs/p1/src/guides/a.js"), [USER.0]);
    assert_eq!(affected(r, "pkgs/p1/src/guides/b.js"), [USER.0]);
    assert_eq!(affected(r, "pkgs/p1/src/other/c.js"), [""; 0]);
    assert_eq!(affected(r, "README.md"), [""; 0]);
}

/// A more specific `paths` key that applies only to some of the names the
/// pattern stands for: the pattern is not expanded, and its record is open.
#[test]
fn a_pattern_that_a_longer_paths_key_may_match_is_open() {
    let d = workspace(&[
        (
            "pkgs/p1/tsconfig.json",
            r#"{"compilerOptions":{"paths":{"@/*":["./src/*"],"@/guides/special/*":["./special/*"]}}}"#,
        ),
        (
            "pkgs/p1/lib/helper.js",
            "export function load(name) {\n  return import(`@/guides/${name}.js`);\n}\n",
        ),
        USER,
        PLAIN,
        ("pkgs/p1/src/guides/a.js", "export const name = 'a';\n"),
        ("pkgs/p1/special/s.js", "export const name = 's';\n"),
    ]);
    let r = d.path();
    assert_eq!(affected(r, "pkgs/p1/special/s.js"), [USER.0]);
    assert_eq!(affected(r, "README.md"), [USER.0]);
}

/// A pattern whose literal start is an alias of the jest configuration. Mappers
/// are not applied to patterns, so nothing is expanded and the load is open:
/// the test that reaches it is selected by the mapped files and by everything
/// else.
#[test]
fn a_pattern_in_a_project_with_module_mappers_is_open() {
    let d = workspace(&[
        ("pkgs/p1/jest.config.js", MAPPER),
        (
            "pkgs/p1/lib/helper.js",
            "export function load(name) {\n  return require('@shared/routes/' + name);\n}\n",
        ),
        USER,
        PLAIN,
        ("shared/routes/a.js", "module.exports = { name: 'a' };\n"),
        ("shared/routes/b.js", "module.exports = { name: 'b' };\n"),
        ("shared/other/c.js", "module.exports = { name: 'c' };\n"),
    ]);
    let r = d.path();
    assert_eq!(affected(r, "shared/routes/a.js"), [USER.0]);
    assert_eq!(affected(r, "shared/routes/b.js"), [USER.0]);
    assert_eq!(affected(r, "README.md"), [USER.0]);
}

/// A literal specifier with a `*` in it (a glob string that a plugin of the
/// runner expands) is not a pattern: it stays what it was, an unresolved
/// specifier.
#[test]
fn a_literal_with_a_star_is_not_a_pattern() {
    let d = workspace(&[
        ("pkgs/p1/jest.config.js", MAPPER),
        (
            "pkgs/p1/lib/helper.js",
            "import all from 'src/things/**/*.js';\nexport function load() {\n  return all;\n}\n",
        ),
        USER,
        PLAIN,
    ]);
    assert_eq!(affected(d.path(), "README.md"), [""; 0]);
}

/// A mapper that does not match the pattern's text but may match what the
/// pattern stands for: nothing is expanded, so the load is open.
#[test]
fn a_pattern_that_a_module_mapper_may_match_is_open() {
    let d = workspace(&[
        (
            "pkgs/p1/jest.config.js",
            "module.exports = {\n  moduleNameMapper: { '^@shared/([a-z]+)$': \
             '<rootDir>/../../shared/$1' },\n};\n",
        ),
        (
            "pkgs/p1/lib/helper.js",
            "export function load(name) {\n  return require('@shared/' + name);\n}\n",
        ),
        USER,
        PLAIN,
        ("shared/a.js", "module.exports = { name: 'a' };\n"),
    ]);
    let r = d.path();
    assert_eq!(affected(r, "shared/a.js"), [USER.0]);
    assert_eq!(affected(r, "README.md"), [USER.0]);
}
