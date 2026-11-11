//! A review case of the change "a load asked of its callee names no origin a
//! binder could answer": a load asked of a `require` function made for a place
//! the engine evaluates (`createRequire(path.join(__dirname, 'noop.js'))`, a
//! load site of the recognizer) whose request comes from the environment.
//! Before that change the request was a template that the environment layer
//! resolved against the repository's assignments, like the request of a plain
//! `require(process.env.X)`; the change drops the template of every load site,
//! also of one whose place is known (and the value assigned here is an absolute
//! path, which is the same for every `require` function).
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
    let o = Command::new(support::harness())
        .args(["affected", "--tests", "jest", "--changed", changed])
        .current_dir(root)
        .env_remove("TURBO_TRACE_DEBUG_ENV")
        .env_remove("TURBO_TRACE_DEBUG_LOADS_PACKAGE")
        .output()
        .expect("run the harness");
    assert!(
        o.status.success(),
        "turbotrace failed: {}",
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

const PLAIN: &str = "module.exports = () => require(process.env.WIDGET_PLUGIN);\n";
// (`createRequire(__filename)(x)` written in place is the file's own `require`
// for the recognizer; a function made for a computed place is a load site, also
// when the place is the file's directory.)
const ASKED: &str = "const { createRequire } = require('node:module');\nconst path = \
                     require('node:path');\nconst req = createRequire(path.join(__dirname, \
                     'noop.js'));\nmodule.exports = () => req(process.env.WIDGET_PLUGIN);\n";

/// One package (no workspace): the fixture of `affected_env.rs`
/// `module_load_from_the_environment`, with the loader given.
fn single(loader: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(r, "jest.config.js", "module.exports = {};\n");
    write(r, "unrelated.txt", "x\n");
    write(r, "src/plugins.js", loader);
    write(
        r,
        "src/plugins.test.js",
        "const path = require('node:path');\nprocess.env.WIDGET_PLUGIN = path.join(__dirname, \
         '..', 'plugins', 'a.js');\nrequire('./plugins');\ntest('p', () => {});\n",
    );
    write(r, "plugins/a.js", "module.exports = 1;\n");
    write(r, "plugins/b.js", "module.exports = 2;\n");
    d
}

/// A pnpm workspace: the plugin the variable names is a file of a package that
/// the loading package does not declare.
fn workspace(loader: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(
        r,
        "pnpm-workspace.yaml",
        "packages:\n  - 'services/*'\n  - 'packages/*'\n",
    );
    write(r, "README.md", "x\n");
    write(
        r,
        "packages/plug/package.json",
        r#"{"name":"@x/plug","version":"1.0.0","dependencies":{}}"#,
    );
    write(r, "packages/plug/a.js", "module.exports = 1;\n");
    write(r, "packages/plug/b.js", "module.exports = 2;\n");
    write(
        r,
        "services/a/package.json",
        r#"{"name":"svc-a","version":"1.0.0","dependencies":{}}"#,
    );
    write(r, "services/a/jest.config.js", "module.exports = {};\n");
    write(r, "services/a/src/plugins.js", loader);
    write(
        r,
        "services/a/test/a.test.js",
        "const path = require('node:path');\nprocess.env.WIDGET_PLUGIN = path.join(__dirname, \
         '..', '..', '..', 'packages', 'plug', 'a.js');\nrequire('../src/plugins');\ntest('p', () \
         => {});\n",
    );
    d
}

const T: &str = "services/a/test/a.test.js";

/// The control: the file's own `require`.
#[test]
fn plain_require_from_the_environment() {
    let d = single(PLAIN);
    assert_eq!(affected(d.path(), "plugins/a.js"), ["src/plugins.test.js"]);
    assert!(affected(d.path(), "unrelated.txt").is_empty());
    assert!(affected(d.path(), "plugins/b.js").is_empty());
    let d = workspace(PLAIN);
    assert_eq!(affected(d.path(), "packages/plug/a.js"), [T]);
    assert!(affected(d.path(), "README.md").is_empty());
}

/// The module the variable is assigned stays a dependency of the test (recall).
#[test]
fn asked_load_from_the_environment_keeps_its_module_in_a_workspace() {
    let d = workspace(ASKED);
    assert_eq!(affected(d.path(), "packages/plug/a.js"), [T]);
}

/// In the workspace a change the load cannot reach selects nothing (precision):
/// a file of the root package, and a module of the other package that nothing
/// is assigned.
#[test]
fn asked_load_from_the_environment_is_resolved_in_a_workspace() {
    let d = workspace(ASKED);
    assert!(
        affected(d.path(), "README.md").is_empty(),
        "a file of the root package selects the test"
    );
    assert!(
        affected(d.path(), "packages/plug/b.js").is_empty(),
        "a module nothing names selects the test"
    );
}

/// The fixture of `affected_env.rs`
/// `assignment_on_a_package_script_command_line`: a package script of
/// `apps/conf` sets the variable to a module of `packages/events`, and
/// `packages/seeder` (which declares neither) loads it; `load`: the expression
/// that loads `configPath`.
fn script(load: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(r, "jest.config.js", "module.exports = {};\n");
    write(r, "unrelated.txt", "x\n");
    write(
        r,
        "pnpm-workspace.yaml",
        "packages:\n  - 'apps/*'\n  - 'packages/*'\n",
    );
    write(
        r,
        "apps/conf/package.json",
        r#"{"name":"conf","version":"1.0.0","scripts":{"seed":"SEED_CONFIG_PATH=../../packages/events/seed.config.ts tsx ../../packages/seeder/src/seed.ts"}}"#,
    );
    write(
        r,
        "packages/seeder/package.json",
        r#"{"name":"seeder","version":"1.0.0"}"#,
    );
    write(
        r,
        "packages/events/package.json",
        r#"{"name":"events","version":"1.0.0"}"#,
    );
    write(
        r,
        "packages/seeder/src/seed.ts",
        &format!(
            "import path from 'node:path';\nimport {{ createRequire }} from 'node:module';\nconst \
             req = createRequire(path.join(__dirname, 'noop.js'));\nconst configPath = \
             process.env.SEED_CONFIG_PATH\n  ? path.resolve(process.cwd(), \
             process.env.SEED_CONFIG_PATH)\n  : './seed.config';\nexport const config = {load};\n"
        ),
    );
    write(
        r,
        "packages/seeder/src/seed.config.ts",
        "export default {};\n",
    );
    write(
        r,
        "packages/seeder/src/seed.test.ts",
        "import './seed';\ntest('s', () => {});\n",
    );
    write(
        r,
        "packages/events/seed.config.ts",
        "import { collections } from './collections';\nexport default { collections };\n",
    );
    write(
        r,
        "packages/events/collections.ts",
        "export const collections = [];\n",
    );
    d
}

const SEED_TEST: &str = "packages/seeder/src/seed.test.ts";

/// The control: the file's own `require` of the path.
#[test]
fn plain_require_of_a_module_a_package_script_names() {
    let d = script("require(configPath)");
    assert_eq!(
        affected(d.path(), "packages/events/seed.config.ts"),
        [SEED_TEST]
    );
    assert_eq!(
        affected(d.path(), "packages/events/collections.ts"),
        [SEED_TEST]
    );
    assert!(affected(d.path(), "unrelated.txt").is_empty());
}

/// The module a package script names stays a dependency of the test that loads
/// it through a `require` function made for a computed place (recall): the
/// value is an absolute path, the same for every `require` function, and the
/// loading package does not declare the package the module is in.
#[test]
fn asked_load_of_a_module_a_package_script_names() {
    let d = script("req(configPath)");
    assert_eq!(
        affected(d.path(), "packages/events/seed.config.ts"),
        [SEED_TEST]
    );
    assert_eq!(
        affected(d.path(), "packages/events/collections.ts"),
        [SEED_TEST]
    );
}

/// The same in one package (recall), and an unrelated change selects nothing
/// (precision).
#[test]
fn asked_load_from_the_environment_in_one_package() {
    let d = single(ASKED);
    assert_eq!(affected(d.path(), "plugins/a.js"), ["src/plugins.test.js"]);
    assert!(
        affected(d.path(), "unrelated.txt").is_empty(),
        "an unrelated change selects the test"
    );
}
