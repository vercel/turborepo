//! `turbotrace affected` on files whose reads the value-flow engine resolves
//! (`crates/turborepo-trace-lang-js/src/flow`): the target edit selects the
//! test, an unrelated edit selects nothing, and what stays unknown keeps the
//! test on the floor.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{path::Path, process::Command};

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// The jest tests affected by `changed`, sorted.
fn affected(root: &Path, changed: &[&str]) -> Vec<String> {
    affected_env(root, changed, &[])
}

fn affected_env(root: &Path, changed: &[&str], env: &[(&str, &str)]) -> Vec<String> {
    let mut args = vec!["affected", "--tests", "jest"];
    for c in changed {
        args.push("--changed");
        args.push(c);
    }
    let o = Command::new(support::harness())
        .args(&args)
        .envs(env.iter().copied())
        .current_dir(root)
        .output()
        .expect("run turbotrace");
    assert!(
        o.status.success(),
        "turbotrace {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let mut v: Vec<String> = String::from_utf8(o.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    v.sort();
    v
}

fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(r, "jest.config.js", "module.exports = {};\n");
    write(r, "unrelated.txt", "x\n");
    d
}

/// A helper function builds the fixture path from a parameter; every call is in
/// the file (through a `describe.each` table).
#[test]
fn fixture_helper_called_with_a_table() {
    let d = repo();
    let r = d.path();
    write(r, "__fixtures__/one.json", "{}\n");
    write(r, "__fixtures__/two.json", "{}\n");
    write(r, "__fixtures__/other.json", "{}\n");
    write(
        r,
        "src/parse.test.js",
        "const { join } = require('node:path');\nconst { readFileSync } = \
         require('node:fs');\nconst FIXTURE_DIR = join(__dirname, '..', '__fixtures__');\nconst \
         FIXTURES = ['one.json', 'two.json'];\nfunction parseFixture(fixtureName) {\n  const \
         fixturePath = join(FIXTURE_DIR, fixtureName);\n  return \
         JSON.parse(readFileSync(fixturePath, 'utf8'));\n}\ndescribe.each(FIXTURES)('parse(%s)', \
         (fixtureName) => {\n  it('parses', () => { parseFixture(fixtureName); });\n});\n",
    );
    assert_eq!(affected(r, &["unrelated.txt"]), Vec::<String>::new());
    assert_eq!(
        affected(r, &["__fixtures__/one.json"]),
        ["src/parse.test.js"]
    );
    assert_eq!(
        affected(r, &["__fixtures__/two.json"]),
        ["src/parse.test.js"]
    );
}

/// A local helper reads a directory given as a parameter and every file in it;
/// it is only called with directories next to the file.
#[test]
fn local_reader_of_known_directories() {
    let d = repo();
    let r = d.path();
    write(r, "defs/queries/a.gql", "query A { a }\n");
    write(r, "defs/fragments/f.gql", "fragment F on T { f }\n");
    write(r, "defs/other/x.gql", "x\n");
    write(
        r,
        "src/collect.js",
        "const fs = require('node:fs');\nconst path = require('node:path');\nconst DEFS_DIR = \
         path.join(__dirname, '../defs');\nconst QUERIES_DIR = path.join(DEFS_DIR, \
         'queries');\nconst FRAGMENTS_DIR = path.join(DEFS_DIR, 'fragments');\nfunction \
         readGqlFiles(dir) {\n  return fs.readdirSync(dir).filter((f) => f.endsWith('.gql'))\n    \
         .map((fileName) => ({ fileName, doc: fs.readFileSync(path.join(dir, fileName), 'utf-8') \
         }));\n}\nmodule.exports = { queries: readGqlFiles(QUERIES_DIR), fragments: \
         readGqlFiles(FRAGMENTS_DIR) };\n",
    );
    write(
        r,
        "src/collect.test.js",
        "require('./collect');\ntest('c', () => {});\n",
    );
    assert_eq!(affected(r, &["unrelated.txt"]), Vec::<String>::new());
    assert_eq!(
        affected(r, &["defs/queries/a.gql"]),
        ["src/collect.test.js"]
    );
    // A new file in a listed directory changes what the helper reads.
    write(r, "defs/fragments/new.gql", "fragment N on T { n }\n");
    assert_eq!(
        affected(r, &["defs/fragments/new.gql"]),
        ["src/collect.test.js"]
    );
}

/// A path taken from the environment at module level: an input from outside the
/// repository, ignored by default; `--strict` counts it.
#[test]
fn environment_path_is_not_open() {
    let d = repo();
    let r = d.path();
    write(
        r,
        "src/client.js",
        "const { readFileSync } = require('node:fs');\nconst testDatafilePath = \
         process.env.TEST_DATAFILE;\nconst testDatafile = testDatafilePath\n  ? \
         JSON.parse(readFileSync(testDatafilePath, 'utf8'))\n  : undefined;\nmodule.exports = { \
         testDatafile };\n",
    );
    write(
        r,
        "src/client.test.js",
        "require('./client');\ntest('c', () => {});\n",
    );
    assert_eq!(affected(r, &["unrelated.txt"]), Vec::<String>::new());
    assert_eq!(affected(r, &["src/client.js"]), ["src/client.test.js"]);
}

/// Scratch files in the temporary directory are not dependencies.
#[test]
fn temporary_files_are_no_dependencies() {
    let d = repo();
    let r = d.path();
    write(
        r,
        "src/out.test.js",
        "const fs = require('node:fs');\nconst os = require('node:os');\nconst path = \
         require('node:path');\ntest('writes', () => {\n  const dir = \
         fs.mkdtempSync(path.join(os.tmpdir(), 'out-'));\n  const file = path.join(dir, \
         'result.json');\n  fs.writeFileSync(file, '{}');\n  \
         expect(JSON.parse(fs.readFileSync(file, 'utf8'))).toEqual({});\n});\n",
    );
    assert_eq!(affected(r, &["unrelated.txt"]), Vec::<String>::new());
}

/// A value the engine cannot evaluate keeps its record: a path from parsed
/// data, and a parameter of an exported function nobody in the file calls. The
/// reads are then bounded by names (tests/affected_names.rs); without that
/// layer they select their tests on every change.
#[test]
fn unknown_values_stay_open() {
    let d = repo();
    let r = d.path();
    write(r, "cfg.json", "{\"file\": \"x\"}\n");
    write(
        r,
        "src/data.test.js",
        "const { readFileSync } = require('node:fs');\nconst { join } = \
         require('node:path');\nconst cfg = JSON.parse(readFileSync(join(__dirname, '..', \
         'cfg.json'), 'utf8'));\ntest('d', () => { readFileSync(cfg.file); });\n",
    );
    write(
        r,
        "src/reader.js",
        "const { readFile } = require('node:fs/promises');\nexports.newReader = function (path) { \
         return () => readFile(path, 'ascii'); };\n",
    );
    write(
        r,
        "src/reader.test.js",
        "require('./reader');\ntest('r', () => {});\n",
    );
    assert_eq!(
        affected_env(r, &["unrelated.txt"], &[("TURBO_TRACE_DEBUG_NAMES", "0")]),
        ["src/data.test.js", "src/reader.test.js"]
    );
    assert_eq!(affected(r, &["unrelated.txt"]), Vec::<String>::new());
}
