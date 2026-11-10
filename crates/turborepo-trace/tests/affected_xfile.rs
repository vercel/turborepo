//! `turbotrace affected` end to end for values that cross files: a module path
//! or the key of a lookup table that a caller passes to a helper in another
//! file (`turborepo_trace_lang_js::xfile`, applied in the derived layer of the
//! graph).
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{
    path::Path,
    process::{Command, Output},
};

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn run_env(root: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut c = Command::new(support::harness());
    // These tests are about binding, whatever the default is.
    c.args(args)
        .current_dir(root)
        .env("TURBO_TRACE_DEBUG_BIND", "1");
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().expect("run turbotrace")
}

fn lines_env(root: &Path, args: &[&str], env: &[(&str, &str)]) -> Vec<String> {
    let o = run_env(root, args, env);
    assert!(
        o.status.success(),
        "turbotrace {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn affected(root: &Path, changed: &str) -> Vec<String> {
    lines_env(
        root,
        &["affected", "--tests", "jest", "--changed", changed],
        &[],
    )
}

fn affected_unbound(root: &Path, changed: &str) -> Vec<String> {
    lines_env(
        root,
        &["affected", "--tests", "jest", "--changed", changed],
        &[("TURBO_TRACE_DEBUG_BIND", "0")],
    )
}

/// The shape of an endpoint-test helper: a table from service names to route
/// modules, read with the names the caller passes; the helper also loads
/// `<cwd>/src/routes` when the test runs inside a service.
const HELPER: &str = r#"import { existsSync } from 'fs';
import { join } from 'path';

const ROUTES_BY_SERVICE = {
  'svc-a': 'services/svc-a/src/routes',
  'svc-b': 'services/svc-b/src/routes.ts',
};

export type ServiceName = keyof typeof ROUTES_BY_SERVICE;

function localRoutes(): string | undefined {
  const custom = join(process.cwd(), 'src', 'routes.ts');
  if (existsSync(custom)) {
    return process.cwd() + '/src/routes';
  }
}

export function loadRoutes(extra: ServiceName[] = []) {
  const paths = new Set<string>();
  const local = localRoutes();
  if (local) {
    paths.add(local);
  }
  for (const name of extra) {
    if (!ROUTES_BY_SERVICE[name]) {
      throw new Error(`Invalid value: ${name}`);
    }
    paths.add(ROUTES_BY_SERVICE[name]);
  }
  let merged: unknown[] = [];
  for (const routePath of paths) {
    const loaded = require(routePath);
    merged = [...merged, ...loaded.routes];
  }
  return { routes: merged };
}

interface Opts {
  extraRoutes?: ServiceName[];
  seed?: boolean;
}

export function setupEndpoint(opts: Opts = {}) {
  const { extraRoutes = [] } = opts;
  const definition = loadRoutes(extraRoutes);
  return { definition, seed: opts.seed ?? true };
}
"#;

/// ```text
/// packages/helpers/{index.ts (barrel), setup-endpoint.ts, jest.config.js (maps services/*)}
/// services/svc-a/src/{routes.ts → handler.ts}, services/svc-b/src/routes.ts
/// packages/x/test/a.test.ts      setupEndpoint({ extraRoutes: ['svc-a'] })
/// packages/x/test/b.test.ts      setupEndpoint({ extraRoutes: ['svc-b'] })
/// packages/x/test/c.test.ts      setupEndpoint()
/// packages/x/test/plain.test.ts  does not use the helper
/// ```
fn helper_repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(
        r,
        "packages/helpers/package.json",
        r#"{"name":"helpers","version":"1.0.0"}"#,
    );
    write(
        r,
        "packages/helpers/jest.config.js",
        "module.exports = {\n  moduleNameMapper: { '^services/(.*)$': \
         '<rootDir>/../../services/$1' },\n};\n",
    );
    write(
        r,
        "packages/helpers/index.ts",
        "export * from './setup-endpoint';\n",
    );
    write(r, "packages/helpers/setup-endpoint.ts", HELPER);
    for s in ["svc-a", "svc-b"] {
        write(
            r,
            &format!("services/{s}/package.json"),
            &format!(r#"{{"name":"{s}","version":"1.0.0"}}"#),
        );
    }
    write(
        r,
        "services/svc-a/src/routes.ts",
        "import { handler } from './handler';\nexport const routes = [handler];\n",
    );
    write(
        r,
        "services/svc-a/src/handler.ts",
        "export const handler = () => 1;\n",
    );
    write(
        r,
        "services/svc-b/src/routes.ts",
        "export const routes = [];\n",
    );
    write(
        r,
        "packages/x/package.json",
        r#"{"name":"x","version":"1.0.0"}"#,
    );
    write(r, "packages/x/jest.config.js", "module.exports = {};\n");
    write(
        r,
        "packages/x/test/a.test.ts",
        "import { setupEndpoint } from '../../helpers';\nconst { definition } = setupEndpoint({ \
         extraRoutes: ['svc-a'] });\ntest('a', () => { expect(definition).toBeDefined(); });\n",
    );
    write(
        r,
        "packages/x/test/b.test.ts",
        "import { setupEndpoint } from '../../helpers';\nconst { definition } = setupEndpoint({ \
         extraRoutes: ['svc-b'], seed: false });\ntest('b', () => { \
         expect(definition).toBeDefined(); });\n",
    );
    write(
        r,
        "packages/x/test/c.test.ts",
        "import { setupEndpoint } from '../../helpers';\nconst { definition } = \
         setupEndpoint();\ntest('c', () => { expect(definition).toBeDefined(); });\n",
    );
    write(
        r,
        "packages/x/test/plain.test.ts",
        "test('plain', () => {});\n",
    );
    write(r, "README.md", "x\n");
    d
}

const A: &str = "packages/x/test/a.test.ts";
const B: &str = "packages/x/test/b.test.ts";
const C: &str = "packages/x/test/c.test.ts";

#[test]
fn a_key_selects_the_route_module_at_the_caller() {
    let d = helper_repo();
    let r = d.path();
    // Without binding, the helper depends on every table value: each user of the
    // helper is selected when any route module changes.
    assert_eq!(
        affected_unbound(r, "services/svc-a/src/routes.ts"),
        [A, B, C]
    );
    assert_eq!(
        affected_unbound(r, "services/svc-b/src/routes.ts"),
        [A, B, C]
    );
    // With binding, a route module belongs to the tests that pass its key.
    assert_eq!(affected(r, "services/svc-a/src/routes.ts"), [A]);
    assert_eq!(affected(r, "services/svc-a/src/handler.ts"), [A]);
    assert_eq!(affected(r, "services/svc-b/src/routes.ts"), [B]);
    // The helper itself still selects all its users; an unrelated file none.
    assert_eq!(affected(r, "packages/helpers/setup-endpoint.ts"), [A, B, C]);
    assert!(affected(r, "README.md").is_empty());
}

#[test]
fn a_step_budget_that_runs_out_closes_nothing() {
    // The binder stops evaluating when its step budget is spent
    // (`TURBO_TRACE_DEBUG_BIND_STEPS` sets the share of one site file).
    // Wherever it stops, the site it was binding stays as it was: the helper
    // keeps its dependency on every table value. So with any budget
    // the selection lies between the bound answer and the answer without binding,
    // and never below the bound one.
    let d = helper_repo();
    let r = d.path();
    let changes = [
        "services/svc-a/src/routes.ts",
        "services/svc-a/src/handler.ts",
        "services/svc-b/src/routes.ts",
        "packages/helpers/setup-endpoint.ts",
        "README.md",
    ];
    let run = |changed: &str, steps: &str| -> (Vec<String>, bool) {
        let o = run_env(
            r,
            &[
                "affected",
                "--tests",
                "jest",
                "--changed",
                changed,
                "--stats",
            ],
            &[("TURBO_TRACE_DEBUG_BIND_STEPS", steps)],
        );
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let exhausted = String::from_utf8_lossy(&o.stderr).contains("budget exhausted");
        let out = String::from_utf8(o.stdout).unwrap();
        (out.lines().map(str::to_owned).collect(), exhausted)
    };
    let answers: Vec<(Vec<String>, Vec<String>)> = changes
        .iter()
        .map(|c| (affected(r, c), affected_unbound(r, c)))
        .collect();
    let mut budgets: Vec<u32> = (1..=24).collect();
    let mut n = 24u32;
    while n < 100_000 {
        n += n / 2;
        budgets.push(n);
    }
    let (mut spent, mut whole) = (0, 0);
    for steps in budgets {
        let steps = steps.to_string();
        for (changed, (bound, unbound)) in changes.iter().zip(&answers) {
            let (got, exhausted) = run(changed, &steps);
            for t in bound {
                assert!(
                    got.contains(t),
                    "{steps} step(s), {changed}: {t} is not selected ({got:?})"
                );
            }
            for t in &got {
                assert!(
                    unbound.contains(t),
                    "{steps} step(s), {changed}: {t} is selected ({got:?})"
                );
            }
            if exhausted {
                spent += 1;
            } else {
                whole += 1;
                assert_eq!(&got, bound, "{steps} step(s), {changed}");
            }
        }
    }
    // Both happen in the range: budgets that run out, and budgets that do not.
    assert!(spent > 0 && whole > 0, "{spent} spent, {whole} whole");
    // One step is not enough for anything: the answer without binding.
    let (got, exhausted) = run("services/svc-b/src/routes.ts", "1");
    assert!(exhausted);
    assert_eq!(got, [A, B, C]);
}

#[test]
fn new_callers_are_bound_on_refresh() {
    let d = helper_repo();
    let r = d.path();
    assert_eq!(affected(r, "services/svc-b/src/routes.ts"), [B]);
    // A new caller with a key that is not known: it may load every table value.
    write(
        r,
        "packages/x/test/d.test.ts",
        "import { loadRoutes } from '../../helpers';\nconst definition = \
         loadRoutes([process.argv[2] as any]);\ntest('d', () => { \
         expect(definition).toBeDefined(); });\n",
    );
    const D: &str = "packages/x/test/d.test.ts";
    assert_eq!(affected(r, "services/svc-b/src/routes.ts"), [B, D]);
    assert_eq!(affected(r, "services/svc-a/src/routes.ts"), [A, D]);
    // A caller that lets the helper escape: whoever gets it may pass anything.
    write(
        r,
        "packages/x/test/e.test.ts",
        "import { setupEndpoint } from '../../helpers';\nconst variants = \
         [setupEndpoint];\ntest('e', () => { variants.forEach((s) => s()); });\n",
    );
    const E: &str = "packages/x/test/e.test.ts";
    assert_eq!(affected(r, "services/svc-b/src/routes.ts"), [B, D, E]);
    // A caller that stops passing the key loses the dependency.
    write(
        r,
        "packages/x/test/b.test.ts",
        "import { setupEndpoint } from '../../helpers';\nconst { definition } = setupEndpoint({ \
         seed: false });\ntest('b', () => { expect(definition).toBeDefined(); });\n",
    );
    assert_eq!(affected(r, "services/svc-b/src/routes.ts"), [D, E]);
}

#[test]
fn an_escape_in_the_helper_keeps_the_table_on_the_helper() {
    let d = helper_repo();
    let r = d.path();
    assert_eq!(affected(r, "services/svc-a/src/routes.ts"), [A]);
    // The helper hands `loadRoutes` to code the analysis does not follow: its
    // callers are no longer all known, so every table value stays a dependency
    // of the helper.
    write(
        r,
        "packages/helpers/setup-endpoint.ts",
        &HELPER.replace(
            "interface Opts {",
            "export const registry = [loadRoutes];\n\ninterface Opts {",
        ),
    );
    assert_eq!(affected(r, "services/svc-a/src/routes.ts"), [A, B, C]);
    // And back.
    write(r, "packages/helpers/setup-endpoint.ts", HELPER);
    assert_eq!(affected(r, "services/svc-a/src/routes.ts"), [A]);
}

/// The shape of a service runner: it loads the module a caller names.
const RUNNER: &str = r#"interface Options {
  configPath: string;
  tags: string[];
}

export async function generateEntrypoint(options: Options) {
  const content = await generateSource(options, 'out');
  return { content };
}

export async function generateSource(options: Options, moduleDirectory: string) {
  const config = require(options.configPath);
  return JSON.stringify({ config, moduleDirectory, tags: options.tags });
}
"#;

/// ```text
/// packages/runner/{generate.ts, jest.config.js}
/// packages/runner/test/known.test.ts    generateEntrypoint({ configPath: join(__dirname, 'fixtures/routes.ts') })
/// packages/runner/test/fixtures/routes.ts → ./endpoint.ts
/// packages/runner/test/other.test.ts    imports generate.ts, calls nothing
/// ```
fn runner_repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(
        r,
        "packages/runner/package.json",
        r#"{"name":"runner","version":"1.0.0"}"#,
    );
    write(
        r,
        "packages/runner/jest.config.js",
        "module.exports = {};\n",
    );
    write(r, "packages/runner/generate.ts", RUNNER);
    write(
        r,
        "packages/runner/test/known.test.ts",
        "import { join } from 'path';\nimport { generateEntrypoint } from \
         '../generate';\ntest('known', async () => {\n  await generateEntrypoint({ configPath: \
         join(__dirname, 'fixtures/routes.ts'), tags: [] });\n});\n",
    );
    write(
        r,
        "packages/runner/test/fixtures/routes.ts",
        "import { endpoint } from './endpoint';\nexport const routes = [endpoint];\n",
    );
    write(
        r,
        "packages/runner/test/fixtures/endpoint.ts",
        "export const endpoint = () => 1;\n",
    );
    write(
        r,
        "packages/runner/test/other.test.ts",
        "import * as generate from '../generate';\ntest('other', () => { expect(typeof \
         generate).toBe('object'); });\n",
    );
    write(r, "README.md", "x\n");
    d
}

const KNOWN: &str = "packages/runner/test/known.test.ts";
const OTHER: &str = "packages/runner/test/other.test.ts";

#[test]
fn a_module_path_parameter_belongs_to_the_caller() {
    let d = runner_repo();
    let r = d.path();
    // Without binding, the runner's `require(options.configPath)` is an open
    // record: every test that reaches it is selected on every change.
    assert_eq!(affected_unbound(r, "README.md"), [KNOWN, OTHER]);
    // `other.test.ts` uses the namespace object as a value: it may call anything
    // with anything, so the unknown module path is its own open record.
    assert_eq!(affected(r, "README.md"), [OTHER]);
    // The caller that passes a path depends on that module and what it imports.
    assert_eq!(
        affected(r, "packages/runner/test/fixtures/endpoint.ts"),
        [KNOWN, OTHER]
    );
    // Once nothing lets the function escape, the record is gone for everyone.
    write(
        r,
        "packages/runner/test/other.test.ts",
        "import { generateSource } from '../generate';\ntest('other', () => { expect(typeof \
         generateSource).toBe('function'); });\n",
    );
    assert_eq!(affected(r, "README.md"), [OTHER]);
    write(
        r,
        "packages/runner/test/other.test.ts",
        "import '../generate';\ntest('other', () => {});\n",
    );
    assert!(affected(r, "README.md").is_empty());
    assert_eq!(
        affected(r, "packages/runner/test/fixtures/endpoint.ts"),
        [KNOWN]
    );
    // A new caller with a path from the command line: its own open record.
    write(
        r,
        "packages/runner/test/argv.test.ts",
        "import { generateEntrypoint } from '../generate';\ntest('argv', async () => {\n  await \
         generateEntrypoint({ configPath: process.argv[2], tags: [] });\n});\n",
    );
    assert_eq!(
        affected(r, "README.md"),
        ["packages/runner/test/argv.test.ts"]
    );
}

#[test]
fn the_floor_report_shows_bound_records_on_the_supplier() {
    let d = runner_repo();
    let r = d.path();
    let out = lines_env(r, &["uncertainty", "--tests", "jest"], &[]).join("\n");
    // The record moved from the runner to the test that lets it escape.
    assert!(out.contains("uncertainty floor: 1 of 2 entries"), "{out}");
    assert!(out.contains("packages/runner/test/other.test.ts"), "{out}");
    assert!(!out.contains("packages/runner/generate.ts\n"), "{out}");
    let off = lines_env(
        r,
        &["uncertainty", "--tests", "jest"],
        &[("TURBO_TRACE_DEBUG_BIND", "0")],
    )
    .join("\n");
    assert!(off.contains("uncertainty floor: 2 of 2 entries"), "{off}");
}

/// ```text
/// lib/paths.ts            configPath(name) → <dir>/configs/<name>.js, DEFAULT → <dir>/configs/default.js
/// lib/configs/{prod,default,other}.js
/// app/load.ts             require(configPath('prod')), require(DEFAULT)
/// app/test/load.test.ts   imports app/load.ts
/// ```
fn summaries_repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(r, "jest.config.js", "module.exports = {};\n");
    write(
        r,
        "lib/paths.ts",
        "import { join } from 'path';\nexport function configPath(name: string) {\n  return \
         join(__dirname, 'configs', `${name}.js`);\n}\nexport const DEFAULT = join(__dirname, \
         'configs', 'default.js');\n",
    );
    for c in ["prod", "default", "other"] {
        write(r, &format!("lib/configs/{c}.js"), "module.exports = {};\n");
    }
    write(
        r,
        "app/load.ts",
        "import { configPath, DEFAULT } from '../lib/paths';\nexport const prod = \
         require(configPath('prod'));\nexport const fallback = require(DEFAULT);\n",
    );
    write(
        r,
        "app/test/load.test.ts",
        "import { prod } from '../load';\ntest('load', () => { expect(prod).toBeDefined(); });\n",
    );
    write(r, "README.md", "x\n");
    d
}

#[test]
fn results_of_calls_and_constants_of_other_files() {
    let d = summaries_repo();
    let r = d.path();
    const T: &str = "app/test/load.test.ts";
    // Without binding, the two loads are open records (a call, an imported
    // binding).
    assert_eq!(affected_unbound(r, "README.md"), [T]);
    // With binding they are the modules the helper's return value and constant
    // name.
    assert!(affected(r, "README.md").is_empty());
    assert_eq!(affected(r, "lib/configs/prod.js"), [T]);
    assert_eq!(affected(r, "lib/configs/default.js"), [T]);
    // (`other.js` is also selected: the helper's own anchored pattern
    // `configs/*.js` is a reference of the helper.)
    // The helper changes what it returns: the binding follows on refresh.
    write(
        r,
        "lib/paths.ts",
        "import { join } from 'path';\nexport function configPath(name: string) {\n  return \
         join(__dirname, 'configs', 'other.js');\n}\nexport const DEFAULT = join(__dirname, \
         'configs', 'default.js');\n",
    );
    assert_eq!(affected(r, "lib/configs/other.js"), [T]);
    assert!(affected(r, "lib/configs/prod.js").is_empty());
}

#[test]
fn the_bound_patch_is_kept_with_the_graph() {
    let d = helper_repo();
    let r = d.path();
    let run = |changed: &str| {
        let o = run_env(
            r,
            &[
                "affected",
                "--tests",
                "jest",
                "--changed",
                changed,
                "--stats",
            ],
            &[],
        );
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let err = String::from_utf8_lossy(&o.stderr).into_owned();
        let bound = err
            .lines()
            .find(|l| l.starts_with("bound:"))
            .unwrap_or_else(|| panic!("no bound line in {err}"))
            .to_owned();
        (String::from_utf8(o.stdout).unwrap(), bound)
    };
    // The first query binds, the second loads the patch: same selection.
    let (first, line) = run("services/svc-a/src/routes.ts");
    assert!(!line.contains("(cached)"), "{line}");
    assert!(line.contains("site(s) closed"), "{line}");
    let (second, line) = run("services/svc-a/src/routes.ts");
    assert!(line.contains("(cached)"), "{line}");
    assert_eq!(first, second);
    assert_eq!(first.lines().collect::<Vec<_>>(), [A]);
    // Another change set on the same graph: still the stored patch.
    let (other, line) = run("services/svc-b/src/routes.ts");
    assert!(line.contains("(cached)"), "{line}");
    assert_eq!(other.lines().collect::<Vec<_>>(), [B]);
    // A caller starts passing a key: the graph changed, so binding runs again.
    write(
        r,
        C,
        "import { setupEndpoint } from '../../helpers';\nconst { definition } = setupEndpoint({ \
         extraRoutes: ['svc-b'] });\ntest('c', () => { expect(definition).toBeDefined(); });\n",
    );
    let (after, line) = run("services/svc-b/src/routes.ts");
    assert!(!line.contains("(cached)"), "{line}");
    assert_eq!(after.lines().collect::<Vec<_>>(), [B, C]);
    // A damaged cache file is computed again, not misread.
    let cache = std::fs::read_dir(r)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .chain([r.join(".turbo/trace")])
        .find_map(|p| {
            let f = p.join("cache/derived-bound");
            f.exists().then_some(f)
        });
    if let Some(f) = cache {
        let mut b = std::fs::read(&f).unwrap();
        let n = b.len();
        b.truncate(n - 3);
        std::fs::write(&f, &b).unwrap();
        let (again, line) = run("services/svc-b/src/routes.ts");
        assert!(!line.contains("(cached)"), "{line}");
        assert_eq!(again, after);
    }
}

/// Copies the directory `from` to `to`, without the entries named `skip` at its
/// top.
fn copy_dir(from: &Path, to: &Path, skip: &str) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        if e.file_name() == skip {
            continue;
        }
        let (p, t) = (e.path(), to.join(e.file_name()));
        if p.is_dir() {
            copy_dir(&p, &t, "");
        } else {
            std::fs::copy(&p, &t).unwrap();
        }
    }
}

/// A copy of the graph directory of `root` without its `cache/` directory: the
/// same shards, and nothing that was composed or derived from them before.
fn store_only(root: &Path) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    copy_dir(&root.join(".turbo/trace"), d.path(), "cache");
    d
}

/// One step of a sequence of edits: queries the graph of `r` the default way
/// (what is kept is used) and checks that it is the graph derived from nothing,
/// twice: with the derived caches off (`TURBO_TRACE_DEBUG_DERIVED_CACHE=0`),
/// and on a copy of the graph directory without its caches, where the same
/// shards are composed from nothing too. The whole graph and two selections are
/// compared.
///
/// Returns how the default path got the bound layer (`cached`; `kept`: the
/// layer of the graph before the edit, taken as it is; `replayed`; `bound`) and
/// whether its composed graph was the previous snapshot's
/// (`turborepo_trace_store`, `compose/reuse.rs`).
fn kept_is_derived_from_nothing(r: &Path, what: &str) -> (&'static str, bool) {
    let o = run_env(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--changed",
            "README.md",
            "--stats",
        ],
        &[],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let err = String::from_utf8_lossy(&o.stderr).into_owned();
    let line = err
        .lines()
        .find(|l| l.starts_with("bound:"))
        .unwrap_or_else(|| panic!("{what}: no bound line in {err}"));
    let how = if line.contains("(cached)") {
        "cached"
    } else if line.contains("(kept across the edit)") {
        "kept"
    } else if line.contains("replayed") {
        "replayed"
    } else {
        "bound"
    };
    let reused = err
        .lines()
        .any(|l| l.starts_with("load:") && l.contains("composed parts reused"));
    let scratch = [("TURBO_TRACE_DEBUG_DERIVED_CACHE", "0")];
    let export = ["graph", "export", "--format", "ndjson"];
    let kept = lines_env(r, &export, &[]);
    assert!(kept.iter().any(|l| l.contains("bound: ")), "{what}");
    assert_eq!(kept, lines_env(r, &export, &scratch), "{what}: the graph");
    // The same shards with nothing kept: composed from them, bound from nothing.
    let fresh = store_only(r);
    let dir = fresh.path().to_str().unwrap();
    let fresh_export = [
        "graph",
        "export",
        "--format",
        "ndjson",
        "--graph",
        dir,
        "--no-refresh",
    ];
    assert_eq!(
        kept,
        lines_env(r, &fresh_export, &scratch),
        "{what}: the graph, against the same shards composed from nothing"
    );
    for changed in [
        "services/svc-a/src/routes.ts",
        "services/svc-b/src/routes.ts",
    ] {
        let args = ["affected", "--tests", "jest", "--changed", changed];
        let selected = lines_env(r, &args, &[]);
        assert_eq!(selected, lines_env(r, &args, &scratch), "{what}: {changed}");
        let fresh_args = [
            "affected",
            "--tests",
            "jest",
            "--changed",
            changed,
            "--graph",
            dir,
            "--no-refresh",
        ];
        assert_eq!(
            selected,
            lines_env(r, &fresh_args, &scratch),
            "{what}: {changed}, against the same shards composed from nothing"
        );
    }
    (how, reused)
}

/// After every edit of a sequence, the graph with the kept results (the cached
/// patch, or the recorded run replayed against the changed graph) is byte for
/// byte the graph bound from nothing (`TURBO_TRACE_DEBUG_DERIVED_CACHE=0`). A
/// recorded run is replayed when the edit changes no fact the binder looked at,
/// and only then.
///
/// The composed graph is kept across an edit too, when the edit changes nothing
/// composition reads (`turborepo_trace_store`, `compose/reuse.rs`): the two
/// meet in every step. So the kept graph is also compared with the same shards
/// composed and bound with nothing kept, and the steps where the composed graph
/// was the previous snapshot's are checked to be the ones meant: a reused
/// composition with a replayed run, and a reused composition under a
/// run that has to be bound again.
#[test]
fn a_replayed_run_is_the_run_bound_from_nothing() {
    let d = helper_repo();
    let r = d.path();
    // Whether the composed graph of the last step was the previous snapshot's.
    let reused = std::cell::Cell::new(false);
    // How the default path got the bound layer after the last edit.
    let step = |what: &str| -> &'static str {
        let (how, kept) = kept_is_derived_from_nothing(r, what);
        reused.set(kept);
        how
    };
    let caller = |arg: &str| {
        format!(
            "import {{ setupEndpoint }} from '../../helpers';\nconst {{ definition }} = \
             setupEndpoint({arg});\ntest('t', () => {{ expect(definition).toBeDefined(); }});\n"
        )
    };
    assert_eq!(step("the first query"), "bound");
    assert!(!reused.get(), "the first open composes");
    assert_eq!(step("nothing changed"), "cached");
    assert!(!reused.get(), "an unchanged graph reads its own caches");
    // Edits that change no fact the binder looked at.
    write(
        r,
        A,
        &format!("{}// a comment\n", caller("{ extraRoutes: ['svc-a'] }")),
    );
    assert_eq!(step("a comment in a caller"), "replayed");
    assert!(
        reused.get(),
        "a comment keeps every reference: the composed graph is kept, and the recorded run is \
         replayed against it"
    );
    write(r, "README.md", "y\n");
    // (No file of the graph: the graph and its cached layers stay as they are.)
    assert_ne!(step("a file that is no code"), "bound");
    write(
        r,
        "packages/x/test/plain.test.ts",
        "test('plain', () => { expect(1).toBe(1); });\n",
    );
    // (The store kept the composed graph, and the file is none the run consulted:
    // the layer of the graph before the edit is taken as it is.)
    assert_eq!(step("a test that does not use the helper"), "kept");
    assert!(reused.get());
    write(
        r,
        "packages/x/test/plain.test.ts",
        "// a line above\ntest('plain', () => { expect(1).toBe(1); });\n",
    );
    assert_eq!(step("a line above, in that test"), "kept");
    write(r, "packages/x/src/unrelated.ts", "export const z = 1;\n");
    assert_eq!(step("a new file nothing imports"), "replayed");
    write(
        r,
        "services/svc-a/src/handler.ts",
        "export const handler = () => 2;\n",
    );
    step("a module a bound site loads");
    // Edits that do.
    write(r, C, &caller("{ extraRoutes: ['svc-b'] }"));
    assert_eq!(step("a caller passes another key"), "bound");
    write(r, C, &format!("\n{}", caller("{ extraRoutes: ['svc-b'] }")));
    assert_eq!(step("a call moves to another line"), "bound");
    assert!(
        reused.get(),
        "a line above the references changes nothing composition reads: the composed graph is \
         kept, while the facts of the caller changed and the run is bound again"
    );
    write(
        r,
        "packages/helpers/setup-endpoint.ts",
        &HELPER.replace(
            "'svc-b': 'services/svc-b/src/routes.ts'",
            "'svc-b': 'services/svc-a/src/routes'",
        ),
    );
    assert_eq!(step("the callee's table"), "bound");
    write(r, "packages/helpers/setup-endpoint.ts", HELPER);
    assert_eq!(step("the callee's table, back"), "bound");
    write(
        r,
        "packages/x/test/d.test.ts",
        "import { loadRoutes } from '../../helpers';\nconst definition = \
         loadRoutes([process.argv[2] as any]);\ntest('d', () => { \
         expect(definition).toBeDefined(); });\n",
    );
    assert_eq!(step("a new caller"), "bound");
    assert!(!reused.get(), "a new file: composed again");
    std::fs::remove_file(r.join("packages/x/test/d.test.ts")).unwrap();
    assert_eq!(step("the caller is deleted"), "bound");
    std::fs::remove_file(r.join(B)).unwrap();
    assert_eq!(step("another caller is deleted"), "bound");
    write(r, B, &caller("{ extraRoutes: ['svc-b'], seed: false }"));
    assert_eq!(step("and comes back"), "bound");
    // Hints: one that adds a module load to the helper, changed, and taken away.
    write(
        r,
        "turbotrace.toml",
        "[[hint]]\nfiles = \"packages/helpers/setup-endpoint.ts\"\ndepends-on = \
         [\"services/svc-b/src/routes.ts\"]\n",
    );
    step("a hint");
    write(
        r,
        "turbotrace.toml",
        "[[hint]]\nfiles = \"packages/x/test/c.test.ts\"\ndepends-on = \
         [\"services/svc-a/src/**\"]\n",
    );
    step("another hint");
    std::fs::remove_file(r.join("turbotrace.toml")).unwrap();
    step("no hint");
    assert_eq!(step("nothing changed again"), "cached");
    // A damaged recorded run is not used.
    let kept = r.join(".turbo/trace/cache/xfile-trace");
    let mut b = std::fs::read(&kept).expect("the recorded run is kept with the graph");
    let n = b.len();
    b.truncate(n - 2);
    std::fs::write(&kept, &b).unwrap();
    write(
        r,
        A,
        &format!(
            "{}// another comment\n",
            caller("{ extraRoutes: ['svc-a'] }")
        ),
    );
    assert_eq!(step("a comment, with a damaged recorded run"), "bound");
    write(
        r,
        A,
        &format!(
            "{}// a third comment\n",
            caller("{ extraRoutes: ['svc-a'] }")
        ),
    );
    assert_eq!(step("a comment again"), "replayed");
}

/// The same check over generated edit sequences (seeded: the same sequences on
/// every run): lines above and below the references of callers, of the callee
/// and of files binding never looks at, a caller that passes another key, a
/// caller that comes and goes. Lines above and below keep the composed graph
/// (`compose/reuse.rs`) and may or may not change a fact binding looked at;
/// another key keeps the composed graph and changes the bound layer; a new file
/// composes again. After every step the kept graph (composed graph kept or not;
/// bound layer cached, kept across the edit, replayed or bound) is the graph
/// the same shards give with nothing kept.
#[test]
fn edit_sequences_keep_the_graph_the_shards_give() {
    const FILES: [&str; 8] = [
        A,
        B,
        C,
        "packages/helpers/setup-endpoint.ts",
        "packages/helpers/index.ts",
        "services/svc-a/src/routes.ts",
        "services/svc-a/src/handler.ts",
        "packages/x/test/plain.test.ts",
    ];
    const ARGS: [&str; 4] = [
        "",
        "{ extraRoutes: ['svc-a'] }",
        "{ extraRoutes: ['svc-b'] }",
        "{ extraRoutes: ['svc-a', 'svc-b'] }",
    ];
    const EXTRA: &str = "packages/x/test/e.test.ts";
    let caller = |arg: &str| {
        format!(
            "import {{ setupEndpoint }} from '../../helpers';\nconst {{ definition }} = \
             setupEndpoint({arg});\ntest('t', () => {{ expect(definition).toBeDefined(); }});\n"
        )
    };
    let mut seen = std::collections::BTreeSet::new();
    for seed in 1..=3u64 {
        let d = helper_repo();
        let r = d.path();
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let mut next = |n: usize| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            ((x >> 11) % n as u64) as usize
        };
        kept_is_derived_from_nothing(r, &format!("seed {seed}: the first query"));
        for i in 0..8 {
            let kind = next(8);
            let file = FILES[next(FILES.len())];
            let text = std::fs::read_to_string(r.join(file)).unwrap();
            let what = match kind {
                0..=2 => {
                    write(r, file, &format!("// above\n{text}"));
                    format!("a line above everything in {file}")
                }
                3..=5 => {
                    write(r, file, &format!("{text}\nexport const added{i} = {i};\n"));
                    format!("a line below everything in {file}")
                }
                6 => {
                    // (Not A: one caller always passes a key, so something stays bound.)
                    let (c, arg) = ([B, C][next(2)], ARGS[next(ARGS.len())]);
                    write(r, c, &caller(arg));
                    format!("{c} passes {arg:?}")
                }
                _ => {
                    if r.join(EXTRA).exists() {
                        std::fs::remove_file(r.join(EXTRA)).unwrap();
                        "a caller is deleted".to_owned()
                    } else {
                        write(r, EXTRA, &caller(ARGS[next(ARGS.len())]));
                        "a new caller".to_owned()
                    }
                }
            };
            let what = format!("seed {seed}, step {i}: {what}");
            seen.insert(kept_is_derived_from_nothing(r, &what));
        }
    }
    // The sequences cover what they are for.
    for want in [
        ("kept", true),
        ("replayed", true),
        ("bound", true),
        ("bound", false),
    ] {
        assert!(
            seen.contains(&want),
            "no step with the bound layer {} and the composed graph {}: {seen:?}",
            want.0,
            if want.1 { "kept" } else { "composed again" }
        );
    }
}

#[test]
fn a_closed_table_lends_no_names() {
    // A test with a read whose path is not known is selected by every changed path
    // that a file of its closure names (the string scan). The strings of a
    // table that binding closed are not such names: they only reach the module
    // loads the table is read for.
    let d = helper_repo();
    let r = d.path();
    write(
        r,
        "packages/helpers/load.ts",
        "const MODS = {\n  a: 'services/svc-a/src/routes',\n  b: \
         'services/svc-b/src/routes.ts',\n};\nexport function load(key: keyof typeof MODS) {\n  \
         return require(MODS[key]);\n}\n",
    );
    write(
        r,
        "packages/x/test/d.test.ts",
        "import fs from 'fs';\nimport { load } from '../../helpers/load';\nconst routes = \
         load('a');\nconst extra = fs.readFileSync((globalThis as any).somePath, \
         'utf8');\ntest('d', () => { expect(routes).toBeDefined(); expect(extra).toBeDefined(); \
         });\n",
    );
    const D: &str = "packages/x/test/d.test.ts";
    // The key it passes selects it; the other value of the table does not.
    assert!(affected(r, "services/svc-a/src/routes.ts").contains(&D.to_owned()));
    let why = || {
        let o = run_env(
            r,
            &[
                "affected",
                "--tests",
                "jest",
                "--changed",
                "services/svc-b/src/routes.ts",
                "--explain",
                "packages/x/test/d.test.ts",
                "--stats",
            ],
            &[("TURBO_TRACE_DEBUG_BIND_DEBUG", "helpers")],
        );
        format!(
            "{}\n{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        )
    };
    assert!(
        !affected(r, "services/svc-b/src/routes.ts").contains(&D.to_owned()),
        "{}",
        why()
    );
    // The scan alone would select it: the helper is in its closure and holds the
    // string.
    assert!(
        lines_env(
            r,
            &[
                "affected",
                "--tests",
                "jest",
                "--changed",
                "services/svc-b/src/routes.ts"
            ],
            &[("TURBO_TRACE_DEBUG_BIND_NAMES", "0")],
        )
        .contains(&D.to_owned())
    );
    // Without binding the table's strings are references of the helper.
    assert!(affected_unbound(r, "services/svc-b/src/routes.ts").contains(&D.to_owned()));
    // The same string outside the table is a name again.
    write(
        r,
        "packages/helpers/load.ts",
        "const MODS = {\n  a: 'services/svc-a/src/routes',\n  b: \
         'services/svc-b/src/routes.ts',\n};\nexport const DEFAULT = \
         'services/svc-b/src/routes.ts';\nexport function load(key: keyof typeof MODS) {\n  \
         return require(MODS[key]);\n}\n",
    );
    assert!(affected(r, "services/svc-b/src/routes.ts").contains(&D.to_owned()));
}
