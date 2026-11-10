//! `turbotrace affected` on tests that work in a temporary directory. The
//! value-flow engine knows that a path below the temporary directory is outside
//! the repository, so reads there are no references. What the test puts there
//! must then be a dependency by itself: the source of a copy, and a module that
//! is generated there and loaded.
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
    write(r, "fixtures/site/index.html", "<p>x</p>\n");
    write(r, "fixtures/site/nested/data.json", "{}\n");
    write(r, "fixtures/other/index.html", "<p>y</p>\n");
    d
}

const HEAD: &str = "const fs = require('node:fs');\nconst os = require('node:os');\nconst path = \
                    require('node:path');\n";

/// The test copies a fixture directory into a temporary directory and reads the
/// copy. The source is a path relative to the working directory, so only the
/// copy call names it (an expression anchored at `__dirname` is a reference
/// wherever it stands).
#[test]
fn a_fixture_directory_copied_with_cp_sync() {
    let d = repo();
    let r = d.path();
    write(
        r,
        "src/copy.test.js",
        &format!(
            "{HEAD}const fixture = './fixtures/site';\ntest('reads the copy', () => {{\n  const \
             dir = fs.mkdtempSync(path.join(os.tmpdir(), 'copy-'));\n  fs.cpSync(fixture, dir, {{ \
             recursive: true }});\n  expect(fs.readFileSync(path.join(dir, 'nested', \
             'data.json'), 'utf8')).toBe('{{}}\\n');\n}});\n"
        ),
    );
    assert_eq!(affected(r, &["unrelated.txt"]), Vec::<String>::new());
    assert_eq!(
        affected(r, &["fixtures/site/nested/data.json"]),
        ["src/copy.test.js"]
    );
    assert_eq!(
        affected(r, &["fixtures/site/index.html"]),
        ["src/copy.test.js"]
    );
    assert_eq!(
        affected(r, &["fixtures/other/index.html"]),
        Vec::<String>::new()
    );
}

/// The asynchronous forms: `fs.promises.cp`, `copyFile`, and `fs-extra`'s
/// `copy`.
#[test]
fn fixtures_copied_with_the_promise_apis_and_fs_extra() {
    let d = repo();
    let r = d.path();
    write(
        r,
        "src/cp.test.js",
        &format!(
            "{HEAD}const {{ cp, copyFile }} = require('node:fs/promises');\nconst sources = {{ \
             site: 'site' }};\ntest('cp', async () => {{\n  const dir = \
             fs.mkdtempSync(path.join(os.tmpdir(), 'cp-'));\n  for (const name of \
             Object.values(sources)) {{\n    await cp(path.join(__dirname, '../fixtures', name), \
             path.join(dir, name), {{ recursive: true }});\n  }}\n  \
             expect(fs.existsSync(path.join(dir, 'site/index.html'))).toBe(true);\n}});\n"
        ),
    );
    write(
        r,
        "src/file.test.js",
        &format!(
            "{HEAD}const {{ copyFile }} = require('node:fs/promises');\nconst fse = \
             require('fs-extra');\nconst root = path.resolve(__dirname, '..');\ntest('copyFile', \
             async () => {{\n  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'file-'));\n  \
             const [first] = ['fixtures/other/index.html'].map((f) => path.join(root, f));\n  \
             await copyFile(first, path.join(dir, 'index.html'));\n  await fse.copy([root, \
             'fixtures', 'site'].join('/'), path.join(dir, 'site'));\n  \
             expect(fs.readFileSync(path.join(dir, 'index.html'), 'utf8')).toBeTruthy();\n}});\n"
        ),
    );
    assert_eq!(affected(r, &["unrelated.txt"]), Vec::<String>::new());
    assert_eq!(
        affected(r, &["fixtures/site/nested/data.json"]),
        ["src/cp.test.js", "src/file.test.js"]
    );
    assert_eq!(
        affected(r, &["fixtures/other/index.html"]),
        ["src/file.test.js"]
    );
}

/// The source of the copy is not known in the file (a parameter of an exported
/// helper): the copy keeps a record. The read is then bounded by names
/// (tests/affected_names.rs); without that layer the tests that reach the
/// helper stay selected.
#[test]
fn a_copy_from_an_unknown_source_keeps_a_record() {
    let d = repo();
    let r = d.path();
    write(
        r,
        "src/sandbox.js",
        &format!(
            "{HEAD}module.exports.sandbox = function sandbox(from) {{\n  const dir = \
             fs.mkdtempSync(path.join(os.tmpdir(), 'sandbox-'));\n  fs.cpSync(from, dir, {{ \
             recursive: true }});\n  return fs.readFileSync(path.join(dir, 'index.html'), \
             'utf8');\n}};\n"
        ),
    );
    write(
        r,
        "src/sandbox.test.js",
        "const { sandbox } = require('./sandbox');\ntest('s', () => { \
         sandbox(process.env.SITE_DIR); });\n",
    );
    assert_eq!(
        affected_env(r, &["unrelated.txt"], &[("TURBO_TRACE_DEBUG_NAMES", "0")]),
        ["src/sandbox.test.js"]
    );
    assert_eq!(affected(r, &["unrelated.txt"]), Vec::<String>::new());
}

/// A module is generated into the temporary directory by a script and then
/// loaded: the load cannot name what the module was generated from, so the test
/// stays selected.
#[test]
fn a_module_generated_in_the_temp_dir_and_loaded() {
    let d = repo();
    let r = d.path();
    write(
        r,
        "src/build.js",
        "require('node:fs').writeFileSync(process.argv[2], 'module.exports = 1;');\n",
    );
    write(
        r,
        "src/generated.test.js",
        &format!(
            "{HEAD}const {{ execFileSync }} = require('node:child_process');\ntest('loads what \
             the script built', () => {{\n  const dir = fs.mkdtempSync(path.join(os.tmpdir(), \
             'gen-'));\n  const out = path.join(dir, 'out.js');\n  execFileSync(process.execPath, \
             [path.join(__dirname, 'build.js'), out]);\n  expect(require(out)).toBe(1);\n}});\n"
        ),
    );
    // A test that only reads data it wrote itself is not on the floor.
    write(
        r,
        "src/scratch.test.js",
        &format!(
            "{HEAD}test('reads back its own file', () => {{\n  const dir = \
             fs.mkdtempSync(path.join(os.tmpdir(), 'scratch-'));\n  \
             fs.writeFileSync(path.join(dir, 'a.txt'), 'a');\n  \
             expect(fs.readFileSync(path.join(dir, 'a.txt'), 'utf8')).toBe('a');\n}});\n"
        ),
    );
    assert_eq!(affected(r, &["unrelated.txt"]), ["src/generated.test.js"]);
    assert_eq!(affected(r, &["src/build.js"]), ["src/generated.test.js"]);
}

/// A dynamic import with a cache-busting query: the query is not part of the
/// file name, so the pattern must match the files without it.
#[test]
fn a_dynamic_import_with_a_query() {
    let d = repo();
    let r = d.path();
    write(r, "src/plugins/a/hook.js", "module.exports = {};\n");
    write(
        r,
        "src/plugins/b/hook.js",
        "module.exports = require('../../lib/dep');\n",
    );
    write(r, "src/lib/dep.js", "module.exports = {};\n");
    write(r, "src/lib/unused.js", "module.exports = {};\n");
    write(
        r,
        "src/hooks.test.js",
        &format!(
            "{HEAD}const {{ pathToFileURL }} = require('node:url');\nconst root = \
             path.join(__dirname, 'plugins');\ntest('every hook loads', async () => {{\n  for \
             (const [index, entry] of fs.readdirSync(root).entries()) {{\n    const url = \
             `${{pathToFileURL(path.join(root, entry, 'hook.js')).href}}?case=${{index}}`;\n    \
             await import(url);\n  }}\n}});\n"
        ),
    );
    assert_eq!(affected(r, &["unrelated.txt"]), Vec::<String>::new());
    assert_eq!(
        affected(r, &["src/plugins/b/hook.js"]),
        ["src/hooks.test.js"]
    );
    // The hooks are loaded as modules: what they import is a dependency too.
    assert_eq!(affected(r, &["src/lib/dep.js"]), ["src/hooks.test.js"]);
    assert_eq!(affected(r, &["src/lib/unused.js"]), Vec::<String>::new());
}
