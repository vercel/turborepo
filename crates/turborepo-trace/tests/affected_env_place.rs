//! A module load whose request comes from the environment, asked of a `require`
//! function: the place the function searches from against the form of the
//! assigned value.
//!
//! The environment layer resolves an assigned value like a request. An absolute
//! value (anchored at a file's directory, the working directory, a package
//! script's directory) names the same module for every `require` function. A
//! relative value (`./a.js`) is searched from the place of the function that is
//! asked: the file's directory for the file's own `require` and for
//! `createRequire` of the file or of a file next to it, the other directory for
//! a function made for a known place elsewhere. Where the place is not known a
//! relative value proves nothing and the record stays open. A bare specifier is
//! never resolved by the environment layer.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{path::Path, process::Command};

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn affected(root: &Path, changed: &str, extra: &[&str]) -> Vec<String> {
    let o = Command::new(support::harness())
        .args(["affected", "--tests", "jest", "--changed", changed])
        .args(extra)
        .current_dir(root)
        .env_remove("TURBO_TRACE_DEBUG_ENV")
        .env_remove("TURBO_TRACE_DEBUG_LOADS_PACKAGE")
        .env_remove("TURBO_TRACE_DEBUG_BIND")
        .output()
        .expect("run turbotrace");
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

/// Where the `require` function that is asked searches from.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Place {
    /// The file's own `require`.
    Plain,
    /// `const require = createRequire(import.meta.url)`: the ES module idiom
    /// (the recognizer takes the name as the file's own `require`).
    Idiom,
    /// `createRequire(import.meta.url)(x)` written in place.
    IdiomInline,
    /// `const req = createRequire(__filename)`.
    Filename,
    /// `let req = createRequire(import.meta.url)`: a name that can be bound
    /// again, so the call is asked of its callee, which the engine
    /// evaluates to the file's own.
    IdiomLet,
    /// `createRequire(path.join(__dirname, 'noop.js'))`: a computed place in
    /// the file's directory.
    Beside,
    /// `createRequire(path.join(__dirname, '..', 'plugins', 'noop.js'))`: a
    /// known place in another directory.
    Elsewhere,
    /// `createRequire(path.join(process.cwd(), 'noop.js'))`: a place that is
    /// not known.
    Unknown,
}

const PLACES: [Place; 8] = [
    Place::Plain,
    Place::Idiom,
    Place::IdiomInline,
    Place::Filename,
    Place::IdiomLet,
    Place::Beside,
    Place::Elsewhere,
    Place::Unknown,
];

impl Place {
    /// The loader's file name below `src/` and its text; `request` is the
    /// expression that is loaded.
    fn loader(self, request: &str) -> (&'static str, String) {
        const CJS: &str = "const { createRequire } = require('node:module');\nconst path = \
                           require('node:path');\n";
        const ESM: &str =
            "import { createRequire } from 'node:module';\nimport path from 'node:path';\n";
        match self {
            Place::Plain => (
                "plugins.js",
                format!("{CJS}module.exports = () => require({request});\n"),
            ),
            Place::Idiom => (
                "plugins.mjs",
                format!(
                    "{ESM}const require = createRequire(import.meta.url);\nexport default () => \
                     require({request});\n"
                ),
            ),
            Place::IdiomInline => (
                "plugins.mjs",
                format!("{ESM}export default () => createRequire(import.meta.url)({request});\n"),
            ),
            Place::Filename => (
                "plugins.js",
                format!(
                    "{CJS}const req = createRequire(__filename);\nmodule.exports = () => \
                     req({request});\n"
                ),
            ),
            Place::IdiomLet => (
                "plugins.mjs",
                format!(
                    "{ESM}let req = createRequire(import.meta.url);\nexport default () => \
                     req({request});\n"
                ),
            ),
            Place::Beside => (
                "plugins.js",
                format!(
                    "{CJS}const req = createRequire(path.join(__dirname, \
                     'noop.js'));\nmodule.exports = () => req({request});\n"
                ),
            ),
            Place::Elsewhere => (
                "plugins.js",
                format!(
                    "{CJS}const req = createRequire(path.join(__dirname, '..', 'plugins', \
                     'noop.js'));\nmodule.exports = () => req({request});\n"
                ),
            ),
            Place::Unknown => (
                "plugins.js",
                format!(
                    "{CJS}const req = createRequire(path.join(process.cwd(), \
                     'noop.js'));\nmodule.exports = () => req({request});\n"
                ),
            ),
        }
    }
}

/// What the test assigns to the variable, and what the loader asks for.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Value {
    /// `path.join(__dirname, '..', 'plugins', 'a.js')`: an absolute path.
    Absolute,
    /// `./a.js`: searched from the place of the `require` function.
    Relative,
    /// `widget-plugin`: a package, which the environment layer does not
    /// resolve.
    Bare,
    /// `plugins/a.js` assigned, and the loader asks for
    /// `path.resolve(process.cwd(), process.env.X)`: absolute where it is
    /// asked.
    Resolved,
}

const VALUES: [Value; 4] = [
    Value::Absolute,
    Value::Relative,
    Value::Bare,
    Value::Resolved,
];

impl Value {
    fn assignment(self) -> &'static str {
        match self {
            Value::Absolute => {
                "process.env.WIDGET_PLUGIN = path.join(__dirname, '..', 'plugins', 'a.js');"
            }
            Value::Relative => "process.env.WIDGET_PLUGIN = './a.js';",
            Value::Bare => "process.env.WIDGET_PLUGIN = 'widget-plugin';",
            Value::Resolved => "process.env.WIDGET_PLUGIN = 'plugins/a.js';",
        }
    }

    fn request(self) -> &'static str {
        match self {
            Value::Resolved => "path.resolve(process.cwd(), process.env.WIDGET_PLUGIN)",
            _ => "process.env.WIDGET_PLUGIN",
        }
    }
}

const TEST: &str = "src/plugins.test.js";

/// One package (no workspace): `src/<loader>` loads what the variable names,
/// and `src/plugins.test.js` assigns it. A module `a.js` is in the file's
/// directory (`src`) and in the other place (`plugins`).
fn single(place: Place, value: Value) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    let (name, text) = place.loader(value.request());
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(r, "jest.config.js", "module.exports = {};\n");
    write(r, "unrelated.txt", "x\n");
    write(r, &format!("src/{name}"), &text);
    write(
        r,
        TEST,
        &format!(
            "const path = require('node:path');\n{}\nrequire('./{name}');\ntest('p', () => \
             {{}});\n",
            value.assignment()
        ),
    );
    write(r, "src/a.js", "module.exports = 0;\n");
    write(r, "plugins/a.js", "module.exports = 1;\n");
    write(r, "plugins/b.js", "module.exports = 2;\n");
    d
}

/// What the load came to, by which changes select the test: `plugins/a.js` (the
/// module in the other directory), `src/a.js` (the module in the file's
/// directory), `open` (an unrelated change selects the test: the record is
/// open), `nothing`, or the list of the changes that select.
fn outcome(place: Place, value: Value, extra: &[&str]) -> String {
    let d = single(place, value);
    let hit = |changed: &str| !affected(d.path(), changed, extra).is_empty();
    let hits = [
        hit("plugins/a.js"),
        hit("src/a.js"),
        hit("plugins/b.js"),
        hit("unrelated.txt"),
    ];
    match hits {
        [true, false, false, false] => "plugins/a.js".to_owned(),
        [false, true, false, false] => "src/a.js".to_owned(),
        [true, true, true, true] => "open".to_owned(),
        [false, false, false, false] => "nothing".to_owned(),
        other => format!("{other:?}"),
    }
}

/// The whole table, for the record (`--ignored --nocapture`): place, value, the
/// outcome under the default policy and under `--strict`.
#[test]
#[ignore = "prints the table of cases"]
fn table() {
    for place in PLACES {
        for value in VALUES {
            println!(
                "TABLE {place:?} x {value:?}: default {} | strict {}",
                outcome(place, value, &[]),
                outcome(place, value, &["--strict"])
            );
        }
    }
}

/// The places that are the file's own directory: every form of them resolves
/// like the file's own `require`.
const OWN: [Place; 6] = [
    Place::Plain,
    Place::Idiom,
    Place::IdiomInline,
    Place::Filename,
    Place::IdiomLet,
    Place::Beside,
];

/// An absolute value names the same module for every `require` function: the
/// module is a dependency and nothing else is, also where the place is
/// elsewhere or not known.
#[test]
fn an_absolute_value_is_resolved_from_any_place() {
    for place in PLACES {
        assert_eq!(
            outcome(place, Value::Absolute, &[]),
            "plugins/a.js",
            "{place:?}"
        );
    }
}

/// A relative value asked of the file's own `require`, in each of its forms, is
/// searched from the file's directory.
#[test]
fn a_relative_value_is_searched_from_the_files_directory() {
    for place in OWN {
        assert_eq!(
            outcome(place, Value::Relative, &[]),
            "src/a.js",
            "{place:?}"
        );
    }
}

/// A relative value asked of a function made for a known place in another
/// directory is searched from that directory: the edge goes there, not to the
/// file's directory.
#[test]
fn a_relative_value_is_searched_from_a_known_place_elsewhere() {
    assert_eq!(
        outcome(Place::Elsewhere, Value::Relative, &[]),
        "plugins/a.js"
    );
}

/// A relative value asked of a function whose place is not known: no edge is
/// guessed and the record stays open, under the default policy and under
/// `--strict`.
#[test]
fn a_relative_value_from_an_unknown_place_stays_open() {
    assert_eq!(outcome(Place::Unknown, Value::Relative, &[]), "open");
    assert_eq!(
        outcome(Place::Unknown, Value::Relative, &["--strict"]),
        "open"
    );
}

/// A bare specifier from the environment stays open for every place, as for the
/// file's own `require`.
#[test]
fn a_bare_value_stays_open() {
    for place in PLACES {
        assert_eq!(outcome(place, Value::Bare, &[]), "open", "{place:?}");
    }
}

/// A path resolved against the working directory where it is asked is absolute:
/// the module is found (in every directory that holds it) whatever the place.
#[test]
fn a_value_resolved_where_it_is_asked_is_resolved_from_any_place() {
    for place in PLACES {
        assert_eq!(
            outcome(place, Value::Resolved, &[]),
            "plugins/a.js",
            "{place:?}"
        );
    }
}

/// A pnpm workspace: `services/a` loads what the variable names through a
/// function whose place is not known, and the test assigns a relative value.
/// The open record selects the test for a change in its own package under the
/// default policy (the declared-packages bound), not for a change in a package
/// it does not declare; `--strict` assumes nothing.
#[test]
fn an_open_load_is_bounded_by_the_declared_packages() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    let (name, text) = Place::Unknown.loader(Value::Relative.request());
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(
        r,
        "pnpm-workspace.yaml",
        "packages:\n  - 'services/*'\n  - 'packages/*'\n",
    );
    write(
        r,
        "packages/plug/package.json",
        r#"{"name":"@x/plug","version":"1.0.0","dependencies":{}}"#,
    );
    write(r, "packages/plug/a.js", "module.exports = 1;\n");
    write(
        r,
        "services/a/package.json",
        r#"{"name":"svc-a","version":"1.0.0","dependencies":{}}"#,
    );
    write(r, "services/a/jest.config.js", "module.exports = {};\n");
    write(r, &format!("services/a/src/{name}"), &text);
    write(r, "services/a/src/a.js", "module.exports = 0;\n");
    write(r, "services/a/other.js", "module.exports = 3;\n");
    write(
        r,
        "services/a/test/a.test.js",
        &format!(
            "{}\nrequire('../src/{name}');\ntest('p', () => {{}});\n",
            Value::Relative.assignment()
        ),
    );
    const T: &str = "services/a/test/a.test.js";
    // No edge is guessed from the file's directory, but the module there is a file
    // of the test's own package, which the open record covers.
    assert_eq!(affected(r, "services/a/other.js", &[]), [T]);
    assert_eq!(affected(r, "services/a/src/a.js", &[]), [T]);
    assert!(affected(r, "packages/plug/a.js", &[]).is_empty());
    assert_eq!(affected(r, "packages/plug/a.js", &["--strict"]), [T]);
}

/// A known place in another package: the relative value is found there,
/// although the loading package does not declare that package (an edge, not the
/// open record's bound).
#[test]
fn a_known_place_in_another_package() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(r, "README.md", "x\n");
    write(
        r,
        "pnpm-workspace.yaml",
        "packages:\n  - 'services/*'\n  - 'packages/*'\n",
    );
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
    write(
        r,
        "services/a/src/plugins.js",
        "const { createRequire } = require('node:module');\nconst path = \
         require('node:path');\nconst req = createRequire(path.join(__dirname, '..', '..', '..', \
         'packages', 'plug', 'noop.js'));\nmodule.exports = () => \
         req(process.env.WIDGET_PLUGIN);\n",
    );
    write(r, "services/a/src/a.js", "module.exports = 0;\n");
    write(
        r,
        "services/a/test/a.test.js",
        "process.env.WIDGET_PLUGIN = './a.js';\nrequire('../src/plugins');\ntest('p', () => {});\n",
    );
    const T: &str = "services/a/test/a.test.js";
    assert_eq!(affected(r, "packages/plug/a.js", &[]), [T]);
    assert!(affected(r, "packages/plug/b.js", &[]).is_empty());
    assert!(affected(r, "services/a/src/a.js", &[]).is_empty());
    assert!(affected(r, "README.md", &[]).is_empty());
}

const HELPER: &str = "export function getEnv(name: string, fallback: string): string {\n  const \
                      value = process.env[name];\n  if (!value) {\n    return fallback;\n  }\n  \
                      return value;\n}\n";

/// One package: `src/plugins.ts` loads `load` (an expression over `getEnv`, an
/// environment helper of `lib/env.ts`), and the test assigns the variable an
/// absolute path.
fn helper(prelude: &str, load: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(r, "jest.config.js", "module.exports = {};\n");
    write(r, "unrelated.txt", "x\n");
    write(r, "lib/env.ts", HELPER);
    write(
        r,
        "src/plugins.ts",
        &format!(
            "import {{ createRequire }} from 'node:module';\nimport path from \
             'node:path';\nimport {{ getEnv }} from '../lib/env';\n{prelude}const FALLBACK = \
             path.join(__dirname, 'default.js');\nexport const load = (other: string, c: boolean) \
             => {load};\n"
        ),
    );
    write(
        r,
        "src/plugins.test.ts",
        "import path from 'node:path';\nimport { load } from \
         './plugins';\nprocess.env.WIDGET_PLUGIN = path.join(__dirname, '..', 'plugins', \
         'a.js');\ntest('p', () => { load(process.argv[2], true); });\n",
    );
    write(r, "src/default.js", "module.exports = 0;\n");
    write(r, "plugins/a.js", "module.exports = 1;\n");
    write(r, "plugins/b.js", "module.exports = 2;\n");
    d
}

const HELPER_TEST: &str = "src/plugins.test.ts";

/// The request is what an environment helper returns, asked of a `require`
/// function made for a computed place in the file's directory: the template
/// finds its record (whose origin is `unknown` for such a load), the modules
/// the variable and the fallback name are dependencies, and nothing else is.
#[test]
fn a_helper_call_asked_of_a_require_function_is_resolved() {
    for (what, prelude, load) in [
        (
            "the file's own require",
            "",
            "require(getEnv('WIDGET_PLUGIN', FALLBACK))",
        ),
        (
            "a function made for a place beside the file",
            "const req = createRequire(path.join(__dirname, 'noop.js'));\n",
            "req(getEnv('WIDGET_PLUGIN', FALLBACK))",
        ),
    ] {
        let d = helper(prelude, load);
        let r = d.path();
        assert_eq!(affected(r, "plugins/a.js", &[]), [HELPER_TEST], "{what}");
        assert_eq!(affected(r, "src/default.js", &[]), [HELPER_TEST], "{what}");
        assert!(affected(r, "plugins/b.js", &[]).is_empty(), "{what}");
        assert!(affected(r, "unrelated.txt", &[]).is_empty(), "{what}");
    }
}

/// Two alternatives of one load: what an environment helper returns, and a
/// parameter that nothing binds. The environment layer resolves the first; the
/// record of the second must stay, so an unrelated change still selects the
/// test. (The two records have the same kind, line and span. For the file's own
/// `require` binding puts the parameter's record on the caller; for a load
/// asked of its callee nothing does, and the helper's template says that its
/// record is shared, so the environment layer leaves both records alone.)
#[test]
fn a_resolved_helper_call_keeps_the_record_of_another_alternative() {
    for (what, prelude, load) in [
        (
            "the file's own require",
            "",
            "require(c ? getEnv('WIDGET_PLUGIN', FALLBACK) : other)",
        ),
        (
            "a function made for a place beside the file",
            "const req = createRequire(path.join(__dirname, 'noop.js'));\n",
            "req(c ? getEnv('WIDGET_PLUGIN', FALLBACK) : other)",
        ),
    ] {
        let d = helper(prelude, load);
        let r = d.path();
        assert_eq!(affected(r, "plugins/a.js", &[]), [HELPER_TEST], "{what}");
        assert_eq!(affected(r, "unrelated.txt", &[]), [HELPER_TEST], "{what}");
    }
}
