//! `affected` end to end for the working directories of a test file that two
//! frameworks take for a test: a jest project in a package, and a vitest
//! project at the repository root (a `vite.config.js` there) whose default
//! patterns match the same file. A file that hands the working directory to
//! code that is not followed names everything below the directory the runner
//! is started in, and that directory is the run's: the jest run of the package
//! is started in the package, the vitest run of the root project in the root.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{path::Path, process::Command};

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// The tests of `frameworks` that a change of `changed` selects.
fn affected(root: &Path, frameworks: &[&str], changed: &str) -> Vec<String> {
    let mut c = Command::new(support::harness());
    c.arg("affected");
    for f in frameworks {
        c.args(["--tests", f]);
    }
    c.args(["--changed", changed]).current_dir(root);
    let o = c.output().expect("run the harness");
    assert!(
        o.status.success(),
        "turbotrace affected failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let mut out: Vec<String> = String::from_utf8(o.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    out.sort();
    out
}

/// ```text
/// vite.config.js                  a vitest project at the root: every test file below is one of its tests
/// packages/tools/index.js         locate(): find(process.cwd()), find is not followed; load(p): fs.readFileSync(p)
/// packages/math/src/add.js        a file nothing imports
/// services/web/jest.config.js     a jest project
/// services/web/test/web.test.js   loads packages/tools: a jest test of services/web, a vitest test of the root
/// services/web/data/a.json
/// tests/root.test.js              loads packages/tools: a vitest test of the root only
/// ```
fn repo(root_runner: bool) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(
        r,
        "pnpm-workspace.yaml",
        "packages:\n  - 'services/*'\n  - 'packages/*'\n",
    );
    if root_runner {
        write(r, "vite.config.js", "module.exports = {};\n");
    }
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
        "services/web/package.json",
        r#"{"name":"web","version":"1.0.0"}"#,
    );
    write(r, "services/web/jest.config.js", "module.exports = {};\n");
    write(r, "services/web/data/a.json", "{}\n");
    let test = |tools: &str| {
        format!(
            "const {{ locate, load }} = require('{tools}');\ntest('t', () => {{ \
             load(locate(globalThis.find) + globalThis.name); }});\n"
        )
    };
    write(
        r,
        "services/web/test/web.test.js",
        &test("../../../packages/tools/index.js"),
    );
    write(r, "tests/root.test.js", &test("../packages/tools/index.js"));
    d
}

const WEB: &str = "services/web/test/web.test.js";
const ROOT: &str = "tests/root.test.js";
const ELSEWHERE: &str = "packages/math/src/add.js";

#[test]
fn a_jest_run_does_not_take_the_working_directory_of_another_frameworks_project() {
    let d = repo(true);
    let r = d.path();
    // The jest run of the package's test is started in the package: a file of
    // another package is not below its working directory.
    assert_eq!(affected(r, &["jest"], ELSEWHERE), Vec::<String>::new());
    // What is below the package still is.
    assert_eq!(affected(r, &["jest"], "services/web/data/a.json"), [WEB]);
}

#[test]
fn the_run_of_the_root_project_names_everything_below_the_root() {
    let d = repo(true);
    let r = d.path();
    // The vitest project at the root runs both files in the root: the escaping
    // working directory names every file of the repository.
    assert_eq!(affected(r, &["vitest"], ELSEWHERE), [WEB, ROOT]);
    // With every framework asked for, a file is selected when one of its runs is.
    assert_eq!(affected(r, &["all"], ELSEWHERE), [WEB, ROOT]);
    assert_eq!(affected(r, &["jest", "vitest"], ELSEWHERE), [WEB, ROOT]);
}

#[test]
fn without_a_project_at_the_root_the_package_is_the_working_directory() {
    let d = repo(false);
    let r = d.path();
    assert_eq!(affected(r, &["all"], ELSEWHERE), Vec::<String>::new());
    assert_eq!(affected(r, &["all"], "services/web/data/a.json"), [WEB]);
}
