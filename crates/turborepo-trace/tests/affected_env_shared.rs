//! Two alternatives of one reference: a value from the environment, which the
//! environment layer resolves, and a value nothing in the file knows (a
//! parameter, an imported binding, the result of a call, the command line).
//! Resolving the first must not close the second.
//!
//! The property is differential. A *control* repository has the site with the
//! unknown alternative alone (`require(other)`), a *variant* has it next to the
//! environment alternative (`require(c ? getEnv('X', F) : other)`). Whatever
//! the control selects for a change that neither alternative names, the variant
//! must select too: what the environment names is one more set of dependencies,
//! never a reason to know the other alternative. The matrix crosses
//!
//! - the kind of the site: `require`, `import()`, `readFileSync`;
//! - the form of the environment alternative: a helper call (origin `call`, the
//!   same record kind as a parameter's), `process.env.X ?? other`, `c ?
//!   process.env.X : other`, `process.env.X || other` (origin `env`, a record
//!   kind of its own), and the helper call with an assignment nothing can
//!   evaluate (the layer keeps the record as an environment read instead of
//!   resolving it);
//! - where the other alternative comes from, and with it whether cross-file
//!   binding moves its record to another file;
//! - binding on and off (`TURBO_TRACE_DEBUG_BIND=0`), the default policy and
//!   `--strict`;
//! - two changes: a file nothing names (`unrelated.txt`), and one that a string
//!   of the package names (`fixtures/data.txt`: under the default policy a read
//!   with a computed path is bounded by the names, so only such a change shows
//!   whether its record is still there).
//!
//! A patch names a record by node, kind, line and span, so two records of one
//! kind at one site are one key: a layer that resolves one of them and drops
//! the key closes the other. The environment layer leaves a key alone that a
//! record of another origin has
//! (`crates/turborepo-trace-store/src/compose/env.rs`, "shared");
//! and one record of origin `call` that stands for a helper's value and for
//! another function's is not relabelled as an environment read, which the
//! default policy ignores for a file read.
//!
//! Before that, on a plain `require` / `import()` / `readFileSync` (the helper
//! form only): an imported binding, a value of the file itself and
//! `process.argv` next to the helper lost their record with and without
//! binding; a parameter lost it with `TURBO_TRACE_DEBUG_BIND=0` and when its
//! callers cannot be enumerated (binding leaves the record at the site then),
//! and for a file read always.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{fmt::Write as _, path::Path, process::Command};

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// Whether the test is selected for `changed`.
fn selects(root: &Path, changed: &str, bind: bool, extra: &[&str]) -> bool {
    let mut c = Command::new(support::harness());
    c.args(["affected", "--tests", "jest", "--changed", changed])
        .args(extra)
        .current_dir(root)
        .env_remove("TURBO_TRACE_DEBUG_ENV")
        .env_remove("TURBO_TRACE_DEBUG_LOADS_PACKAGE")
        .env_remove("TURBO_TRACE_DEBUG_BIND");
    if !bind {
        c.env("TURBO_TRACE_DEBUG_BIND", "0");
    }
    let o = c.output().expect("run turbotrace");
    assert!(
        o.status.success(),
        "turbotrace failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let out = String::from_utf8(o.stdout).unwrap();
    let v: Vec<&str> = out.lines().collect();
    assert!(v.is_empty() || v == [TEST], "{v:?}");
    !v.is_empty()
}

const TEST: &str = "src/plugins.test.ts";

/// A file that a string of the package names.
const NAMED: &str = "fixtures/data.txt";

/// The changes a cell is asked about: a file nothing names, and one a string
/// names.
const CHANGES: [&str; 2] = ["unrelated.txt", NAMED];

const HELPER: &str = "export function getEnv(name: string, fallback: string): string {\n  const \
                      value = process.env[name];\n  if (!value) {\n    return fallback;\n  }\n  \
                      return value;\n}\n";

/// A value no file can say.
const UNKNOWN: &str = "(globalThis as any).plugin";

const CONF: &str = "export const other: string = (globalThis as any).plugin;\nexport function \
                    pick(): string {\n  return (globalThis as any).plugin;\n}\n";

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Kind {
    Require,
    Import,
    Read,
}

impl Kind {
    fn site(self, expr: &str) -> String {
        match self {
            Kind::Require => format!("require({expr})"),
            Kind::Import => format!("import({expr})"),
            Kind::Read => format!("fs.readFileSync({expr}, 'utf8')"),
        }
    }

    /// The extension of the files the alternatives name.
    fn ext(self) -> &'static str {
        match self {
            Kind::Require | Kind::Import => "js",
            Kind::Read => "txt",
        }
    }
}

/// The environment alternative; `None` is the control.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Env {
    /// `c ? getEnv('WIDGET_PLUGIN', FALLBACK) : other`: a helper of another
    /// file.
    Helper,
    /// `process.env.WIDGET_PLUGIN ?? other`
    Nullish,
    /// `c ? process.env.WIDGET_PLUGIN! : other`
    Cond,
    /// `process.env.WIDGET_PLUGIN || other`
    Or,
    /// The helper call again, with an assignment of the variable that nothing
    /// can evaluate: the environment layer keeps the helper's record as an
    /// environment read (it drops the key and adds a record), and the other
    /// alternative's record with it.
    HelperKept,
}

impl Env {
    fn expr(self, other: &str) -> String {
        match self {
            Env::Helper | Env::HelperKept => {
                format!("c ? getEnv('WIDGET_PLUGIN', FALLBACK) : {other}")
            }
            Env::Nullish => format!("process.env.WIDGET_PLUGIN ?? {other}"),
            Env::Cond => format!("c ? process.env.WIDGET_PLUGIN! : {other}"),
            Env::Or => format!("process.env.WIDGET_PLUGIN || {other}"),
        }
    }
}

/// Where the other alternative comes from.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Other {
    /// A parameter; the test passes a value nothing knows.
    ParamFromTest,
    /// A parameter; the test passes `process.argv[2]`.
    ParamArgv,
    /// A parameter; a source file passes a value nothing knows.
    ParamFromSource,
    /// A parameter of an exported function that nothing in the repository
    /// calls.
    ParamUncalled,
    /// A parameter; the test passes a path it can say (binding closes the
    /// site).
    ParamKnown,
    /// A parameter of a function that leaves the file as a value (a global
    /// hook): its callers cannot be enumerated, and binding leaves the
    /// record where it is.
    ParamEscapes,
    /// A binding imported from another file.
    Imported,
    /// What a function of another file returns (no environment helper).
    CallResult,
    /// A value of the file itself that nothing knows.
    Local,
    /// `process.argv[2]` in the file itself.
    LocalArgv,
}

const ENVS: [Env; 5] = [
    Env::Helper,
    Env::Nullish,
    Env::Cond,
    Env::Or,
    Env::HelperKept,
];
const OTHERS: [Other; 10] = [
    Other::ParamFromTest,
    Other::ParamArgv,
    Other::ParamFromSource,
    Other::ParamUncalled,
    Other::ParamKnown,
    Other::ParamEscapes,
    Other::Imported,
    Other::CallResult,
    Other::Local,
    Other::LocalArgv,
];

/// One package. `src/plugins.ts` has the site; the test assigns the variable
/// the path of `plugins/a.<ext>` (or, for [`Env::HelperKept`], a value nothing
/// knows); `plugins/b.<ext>` is what a caller that knows its value passes.
fn repo(kind: Kind, env: Option<Env>, other: Other) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    let ext = kind.ext();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(r, "jest.config.js", "module.exports = {};\n");
    write(r, "unrelated.txt", "x\n");
    write(r, NAMED, "x\n");
    write(r, "lib/env.ts", HELPER);
    write(r, "lib/conf.ts", CONF);
    let (imports, expr) = match other {
        Other::ParamFromTest
        | Other::ParamArgv
        | Other::ParamFromSource
        | Other::ParamUncalled
        | Other::ParamKnown
        | Other::ParamEscapes => ("", "other"),
        Other::Imported => (
            "import { other as imported } from '../lib/conf';\n",
            "imported",
        ),
        Other::CallResult => ("import { pick } from '../lib/conf';\n", "pick()"),
        Other::Local => ("", UNKNOWN),
        Other::LocalArgv => ("", "process.argv[2]"),
    };
    let expr = match env {
        Some(e) => e.expr(expr),
        None => expr.to_owned(),
    };
    write(
        r,
        "src/plugins.ts",
        &format!(
            "import fs from 'node:fs';\nimport path from 'node:path';\nimport {{ getEnv }} from \
             '../lib/env';\n{imports}const FALLBACK = path.join(__dirname, \
             'default.{ext}');\nexport const helper = () => getEnv('UNUSED', '');\nexport const \
             NAME = '{NAMED}';\nexport const load = (other: string, c: boolean) => {};\n{}",
            kind.site(&expr),
            if other == Other::ParamEscapes {
                "(globalThis as any).hook = load;\n"
            } else {
                ""
            }
        ),
    );
    let assign = if env == Some(Env::HelperKept) {
        format!("process.env.WIDGET_PLUGIN = {UNKNOWN};\n")
    } else {
        format!("process.env.WIDGET_PLUGIN = path.join(__dirname, '..', 'plugins', 'a.{ext}');\n")
    };
    let test = match other {
        Other::ParamFromTest => format!(
            "import path from 'node:path';\nimport {{ load }} from \
             './plugins';\n{assign}test('p', () => {{ load({UNKNOWN}, true); }});\n"
        ),
        Other::ParamArgv => format!(
            "import path from 'node:path';\nimport {{ load }} from \
             './plugins';\n{assign}test('p', () => {{ load(process.argv[2], true); }});\n"
        ),
        Other::ParamFromSource => {
            write(
                r,
                "src/main.ts",
                &format!(
                    "import {{ load }} from './plugins';\nexport const run = () => \
                     load({UNKNOWN}, true);\n"
                ),
            );
            format!(
                "import path from 'node:path';\nimport {{ run }} from \
                 './main';\n{assign}test('p', () => {{ run(); }});\n"
            )
        }
        Other::ParamUncalled => format!(
            "import path from 'node:path';\nimport {{ helper }} from \
             './plugins';\n{assign}test('p', () => {{ helper(); }});\n"
        ),
        Other::ParamKnown => format!(
            "import path from 'node:path';\nimport {{ load }} from \
             './plugins';\n{assign}test('p', () => {{ load(path.join(__dirname, '..', 'plugins', \
             'b.{ext}'), false); }});\n"
        ),
        Other::ParamEscapes => format!(
            "import path from 'node:path';\nimport './plugins';\n{assign}test('p', () => {{ \
             (globalThis as any).hook({UNKNOWN}, true); }});\n"
        ),
        Other::Imported | Other::CallResult | Other::Local | Other::LocalArgv => format!(
            "import path from 'node:path';\nimport {{ load }} from \
             './plugins';\n{assign}test('p', () => {{ load('', true); }});\n"
        ),
    };
    write(r, TEST, &test);
    for f in ["src/default", "plugins/a", "plugins/b"] {
        write(r, &format!("{f}.{ext}"), "module.exports = 0;\n");
    }
    d
}

/// The four columns of a cell: binding on and off, the default policy and
/// `--strict`.
const COLUMNS: [(bool, &[&str], &str); 4] = [
    (true, &[], "bind"),
    (true, &["--strict"], "bind strict"),
    (false, &[], "no-bind"),
    (false, &["--strict"], "no-bind strict"),
];

/// Which of [`CHANGES`] select the test, per column.
type Selected = [[bool; 2]; 4];

fn selected(r: &Path) -> Selected {
    COLUMNS.map(|(bind, extra, _)| CHANGES.map(|c| selects(r, c, bind, extra)))
}

struct Cell {
    selected: Selected,
    /// The file the variable names is a dependency by an edge (binding on and
    /// off): the environment alternative was resolved, the cell is not
    /// vacuous.
    resolved: [bool; 2],
}

fn variant(kind: Kind, env: Env, other: Other) -> Cell {
    let d = repo(kind, Some(env), other);
    let r = d.path();
    let named = format!("plugins/a.{}", kind.ext());
    Cell {
        selected: selected(r),
        resolved: [true, false].map(|bind| selects(r, &named, bind, &["--on-uncertain", "ignore"])),
    }
}

/// One row of the matrix: where the other alternative comes from, what the
/// control selects, and the cell of each form of the environment alternative.
type Row = (Other, Selected, Vec<(Env, Cell)>);

/// Runs the matrix for `kind`; returns the table and the cells that lose a
/// selection.
fn matrix(kind: Kind) -> (String, Vec<String>) {
    let rows: Vec<Row> = std::thread::scope(|s| {
        let hs: Vec<_> = OTHERS
            .iter()
            .map(|&other| {
                s.spawn(move || {
                    (
                        other,
                        selected(repo(kind, None, other).path()),
                        ENVS.iter()
                            .map(|&e| (e, variant(kind, e, other)))
                            .collect::<Vec<_>>(),
                    )
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    // `u`: the change of the file nothing names selects the test; `n`: the named
    // one.
    let mark = |b: [bool; 2]| {
        format!(
            "{}{}",
            if b[0] { 'u' } else { '-' },
            if b[1] { 'n' } else { '-' }
        )
    };
    let yes = |b: bool| if b { "yes" } else { "no" };
    let mut table = String::new();
    let mut holes = Vec::new();
    let mut vacuous = Vec::new();
    writeln!(
        table,
        "{kind:?}: which changes select the test (u: unrelated.txt, n: {NAMED}), control → \
         variant, per column [{}]",
        COLUMNS.map(|c| c.2).join(" | ")
    )
    .unwrap();
    for (other, ctl, cells) in &rows {
        for (env, cell) in cells {
            let cols: Vec<String> = (0..4)
                .map(|i| format!("{}→{}", mark(ctl[i]), mark(cell.selected[i])))
                .collect();
            let lost: Vec<&str> = (0..4)
                .filter(|&i| (0..2).any(|c| ctl[i][c] && !cell.selected[i][c]))
                .map(|i| COLUMNS[i].2)
                .collect();
            let what = format!("{kind:?} / {env:?} / {other:?}");
            writeln!(
                table,
                "  {what:<38} {}  resolved {}{}",
                cols.join(" | "),
                cell.resolved.map(yes).join("/"),
                if lost.is_empty() { "" } else { "   <-- LOST" }
            )
            .unwrap();
            if !lost.is_empty() {
                holes.push(format!("{what}: {}", lost.join(", ")));
            }
            // Every cell is about what it says: the alternative was resolved (the file the
            // variable names is a dependency), or, for the kept record, was not.
            if cell.resolved != [*env != Env::HelperKept; 2] {
                vacuous.push(what);
            }
        }
    }
    assert!(
        vacuous.is_empty(),
        "the environment alternative is not what the cell says in {vacuous:#?}\n{table}"
    );
    (table, holes)
}

fn check(kind: Kind) {
    let (table, holes) = matrix(kind);
    println!("{table}");
    assert!(
        holes.is_empty(),
        "resolving the environment alternative closed the other one in {holes:#?}\n{table}"
    );
}

#[test]
fn a_resolved_environment_alternative_keeps_the_other_open_require() {
    check(Kind::Require);
}

#[test]
fn a_resolved_environment_alternative_keeps_the_other_open_import() {
    check(Kind::Import);
}

#[test]
fn a_resolved_environment_alternative_keeps_the_other_open_read() {
    check(Kind::Read);
}
