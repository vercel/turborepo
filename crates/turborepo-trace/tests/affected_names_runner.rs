//! `affected` end to end for who runs a process
//! (`turborepo_trace_graph::names`): a build script that a test reaches only
//! through the output of a build task is named by the task (its working
//! directory, its package), not by the test.
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
    let mut c = Command::new(support::harness());
    c.args(["affected", "--tests", "jest"])
        .args(args)
        .current_dir(root);
    for (k, v) in env {
        c.env(k, v);
    }
    let o = c.output().expect("run the harness");
    assert!(
        o.status.success(),
        "turbotrace affected failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout).unwrap()
}

/// The tests a change of `changed` selects.
fn affected(root: &Path, changed: &str, env: &[(&str, &str)]) -> Vec<String> {
    run(root, &["--changed", changed], env)
        .lines()
        .map(str::to_owned)
        .collect()
}

const OFF: &[(&str, &str)] = &[("TURBO_TRACE_DEBUG_NAMES_TASK_RUNNER", "0")];

/// ```text
/// packages/reader/index.js      readText(p): fs.readFileSync(p)             p is a parameter
/// packages/built/build.js       readText('fixtures/' + name), relative to the working
///                               directory; writes dist/index.js. Run by the package's
///                               `build` script: `node build.js`
/// packages/moved/build.js       the same; its script is `cd scripts && node ../build.js`
/// apps/web/test/web.test.js     loads packages/built/dist/index.js      (the task's output)
/// apps/web/test/moved.test.js   loads packages/moved/dist/index.js
/// apps/web/test/script.test.js  loads packages/built/build.js           (the script itself)
/// ```
///
/// Every package has a `fixtures/a.json`.
fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(
        r,
        "pnpm-workspace.yaml",
        "packages:\n  - 'apps/*'\n  - 'packages/*'\n",
    );
    write(
        r,
        "turbo.json",
        r#"{"tasks":{"build":{"outputs":["dist/**"]}}}"#,
    );
    write(
        r,
        "packages/reader/package.json",
        r#"{"name":"reader","version":"1.0.0"}"#,
    );
    write(
        r,
        "packages/reader/index.js",
        "const fs = require('fs');\nexports.readText = function (p) {\n  return \
         fs.readFileSync(p, 'utf8');\n};\n",
    );
    let build = "const fs = require('fs');\nconst path = require('path');\nconst { readText } = \
                 require('../reader/index.js');\nconst name = process.argv[2] || \
                 'a.json';\nreadText('fixtures/' + name);\nfs.mkdirSync(path.join(__dirname, \
                 'dist'), { recursive: true });\nfs.writeFileSync(path.join(__dirname, 'dist', \
                 'index.js'), 'module.exports = 1;\\n');\n";
    for (pkg, script) in [
        ("built", "node build.js"),
        ("moved", "cd scripts && node ../build.js"),
    ] {
        write(
            r,
            &format!("packages/{pkg}/package.json"),
            &format!(
                r#"{{"name":"{pkg}","version":"1.0.0","main":"dist/index.js","scripts":{{"build":"{script}"}}}}"#
            ),
        );
        write(r, &format!("packages/{pkg}/build.js"), build);
        write(r, &format!("packages/{pkg}/fixtures/a.json"), "{}\n");
        write(
            r,
            &format!("packages/{pkg}/scripts/fixtures/a.json"),
            "{}\n",
        );
    }
    write(
        r,
        "apps/web/package.json",
        r#"{"name":"web","version":"1.0.0"}"#,
    );
    write(r, "apps/web/jest.config.js", "module.exports = {};\n");
    write(r, "apps/web/fixtures/a.json", "{}\n");
    write(
        r,
        "apps/web/test/web.test.js",
        "const built = require('../../../packages/built/dist/index.js');\ntest('t', () => { \
         expect(built).toBe(1); });\n",
    );
    write(
        r,
        "apps/web/test/moved.test.js",
        "const built = require('../../../packages/moved/dist/index.js');\ntest('t', () => { \
         expect(built).toBe(1); });\n",
    );
    write(
        r,
        "apps/web/test/script.test.js",
        "test('t', () => { require('../../../packages/built/build.js'); });\n",
    );
    d
}

const WEB: &str = "apps/web/test/web.test.js";
const MOVED: &str = "apps/web/test/moved.test.js";
const SCRIPT: &str = "apps/web/test/script.test.js";

#[test]
fn a_build_script_reads_where_its_task_runs() {
    let d = repo();
    let r = d.path();
    let got = (
        affected(r, "packages/built/fixtures/a.json", &[]),
        affected(r, "packages/built/fixtures/a.json", OFF),
        affected(r, "apps/web/fixtures/a.json", &[]),
        affected(r, "apps/web/fixtures/a.json", OFF),
    );
    let v = |x: &[&str]| x.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    assert_eq!(
        got,
        (
            // A file the script reads when the task runs it, in the task's package: the
            // tests above the task (the one that loads the script is above it too, through
            // the package's manifest).
            v(&[SCRIPT, WEB]),
            // Named as if the test ran the script, nobody was selected: the string counted
            // for the test's package and for the reader's.
            v(&[]),
            // The same path in the test's package: the script does not run there. The
            // test that loads the script itself runs it in its own directory, and the
            // test above a task whose command changes directory keeps the answer from
            // before.
            v(&[MOVED, SCRIPT]),
            v(&[MOVED, SCRIPT, WEB]),
        )
    );
}

#[test]
fn the_reason_names_the_task() {
    let d = repo();
    let r = d.path();
    let out = run(
        r,
        &[
            "--changed",
            "packages/built/fixtures/b.json",
            "--explain",
            WEB,
        ],
        &[],
    );
    assert!(
        out.contains("packages/built/fixtures/b.json") && out.contains("task:packages/built#build"),
        "{out}"
    );
}
