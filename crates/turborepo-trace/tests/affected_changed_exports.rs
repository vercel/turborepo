//! `affected --since`: export-level tracking of a modified file.
//! Only the exports whose definition changed are followed, an importer passes
//! the change on through the exports that read it, and a file whose top level
//! runs code is still tracked this way when what it does at load time is the
//! same at both revisions.
//!
//! The fixtures keep the shape of two real sites (names changed): a utility
//! module of a tracking package from which an unused export was removed,
//! reached by hundreds of tests through a chain of non-pure modules; and a
//! one-file error predicate with a load-time `Symbol.for` call, imported by a
//! fetch wrapper that most tests load.
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
    Command::new(support::harness())
        .args(args)
        .envs(env.iter().copied())
        .current_dir(root)
        .output()
        .expect("run the harness")
}

fn ok_env(root: &Path, args: &[&str], env: &[(&str, &str)]) -> String {
    let o = run_env(root, args, env);
    assert!(
        o.status.success(),
        "harness {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout).unwrap()
}

fn git(root: &Path, args: &[&str]) {
    let o = Command::new("git")
        .args(["-c", "user.email=t@t", "-c", "user.name=t"])
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
}

fn affected_env(root: &Path, env: &[(&str, &str)]) -> Vec<String> {
    ok_env(
        root,
        &["affected", "--tests", "jest", "--since", "HEAD"],
        env,
    )
    .lines()
    .map(str::to_owned)
    .collect()
}

fn affected(root: &Path) -> Vec<String> {
    affected_env(root, &[])
}

fn explain(root: &Path, entry: &str) -> String {
    ok_env(
        root,
        &[
            "affected",
            "--tests",
            "jest",
            "--since",
            "HEAD",
            "--explain",
            entry,
        ],
        &[],
    )
}

fn json(root: &Path) -> serde_json::Value {
    let o = ok_env(
        root,
        &[
            "affected", "--tests", "jest", "--since", "HEAD", "--format", "json",
        ],
        &[],
    );
    serde_json::from_str(&o).unwrap()
}

fn commit(files: &[(&str, &str)]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(r, ".gitignore", ".turbo/trace/\n");
    write(
        r,
        "jest.config.js",
        "module.exports = { testMatch: ['**/test/**/*.test.ts'] };\n",
    );
    for (p, c) in files {
        write(r, p, c);
    }
    git(r, &["init", "-q"]);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "base"]);
    d
}

const UTILS: &str = "import { COOKIES, PARAMS } from './constants';\nimport type { CookieKey, \
                     ParamKey, UtmKey } from './types';\n\nexport const isParam = (p: string): p \
                     is ParamKey => {\n  return p in PARAMS;\n};\n\nexport const isUtm = (p: \
                     string): p is UtmKey => {\n  return p.startsWith('utm_');\n};\n\n/** Whether \
                     a key is a tracked cookie key. */\nexport const isCookie = (k: string): k is \
                     CookieKey => {\n  return k in COOKIES;\n};\n\nexport function \
                     hasSignal(touch: Record<string, string>): boolean {\n  return \
                     Object.keys(touch).some((k) => isUtm(k) || isParam(k));\n}\n";

/// The utility module without `isCookie` and what only it used.
const UTILS_WITHOUT_COOKIE: &str =
    "import { PARAMS } from './constants';\nimport type { ParamKey, UtmKey } from \
     './types';\n\nexport const isParam = (p: string): p is ParamKey => {\n  return p in \
     PARAMS;\n};\n\nexport const isUtm = (p: string): p is UtmKey => {\n  return \
     p.startsWith('utm_');\n};\n\nexport function hasSignal(touch: Record<string, string>): \
     boolean {\n  return Object.keys(touch).some((k) => isUtm(k) || isParam(k));\n}\n";

/// The tracking package: `utils.ts` is read by `migrate.ts` (module state, not
/// pure), which `resolve.ts`, `build.ts` and `service.ts` (a class, a load-time
/// `new`) pass on. `test/service.test.ts` uses the service,
/// `test/build.test.ts` only a constant of `build.ts`, `test/utils.test.ts` one
/// predicate.
fn tracking() -> tempfile::TempDir {
    commit(&[
        (
            "src/attribution/constants.ts",
            "export const PARAMS = { gclid: 1, fbclid: 1 };\nexport const COOKIES = { _ga: 1 };\n",
        ),
        (
            "src/attribution/types.ts",
            "export type ParamKey = 'gclid' | 'fbclid';\nexport type CookieKey = '_ga';\nexport \
             type UtmKey = `utm_${string}`;\n",
        ),
        ("src/attribution/utils.ts", UTILS),
        (
            "src/attribution/migrate.ts",
            "import { hasSignal } from './utils';\n\nlet ran = false;\n\nexport function \
             migrate(data: Record<string, string>): boolean {\n  if (ran) return false;\n  ran = \
             true;\n  return hasSignal(data);\n}\n",
        ),
        (
            "src/attribution/resolve.ts",
            "import { migrate } from './migrate';\n\nexport function resolveAttribution(data: \
             Record<string, string>) {\n  return { migrated: migrate(data) };\n}\n",
        ),
        (
            "src/build.ts",
            "import { resolveAttribution } from './attribution/resolve';\n\nexport const VERSION \
             = 2;\n\nexport function build(data: Record<string, string>) {\n  return { version: \
             VERSION, attribution: resolveAttribution(data) };\n}\n",
        ),
        (
            "src/service.ts",
            "import { build } from './build';\n\nexport class Service {\n  track(data: \
             Record<string, string>) {\n    return build(data);\n  }\n}\n\nexport const service = \
             new Service();\n",
        ),
        (
            "test/service.test.ts",
            "import { service } from '../src/service';\ntest('track', () => \
             expect(service.track({}).version).toBe(2));\n",
        ),
        (
            "test/build.test.ts",
            "import { VERSION } from '../src/build';\ntest('version', () => \
             expect(VERSION).toBe(2));\n",
        ),
        (
            "test/utils.test.ts",
            "import { isUtm } from '../src/attribution/utils';\ntest('utm', () => \
             expect(isUtm('utm_x')).toBe(true));\n",
        ),
    ])
}

#[test]
fn a_removed_unused_export_selects_nothing() {
    let d = tracking();
    let r = d.path();
    write(r, "src/attribution/utils.ts", UTILS_WITHOUT_COOKIE);
    assert_eq!(affected(r), Vec::<String>::new());
    // Without the export-level comparison (as before): every test that loads the
    // file.
    assert_eq!(
        affected_env(
            r,
            &[
                ("TURBO_TRACE_DEBUG_EXPORT_DIFF", "0"),
                ("TURBO_TRACE_DEBUG_SYMBOLS", "0")
            ]
        ),
        [
            "test/build.test.ts",
            "test/service.test.ts",
            "test/utils.test.ts"
        ]
    );
    let e = explain(r, "test/service.test.ts");
    assert!(
        e.contains("unaffected test/service.test.ts")
            && e.contains(
                "src/attribution/utils.ts skipped: only isCookie changed in it, and nothing this \
                 entry loads uses it"
            ),
        "{e}"
    );
    let v = json(r);
    assert_eq!(
        v["pruning"]["changed_exports"][0]["path"],
        "src/attribution/utils.ts"
    );
    assert_eq!(
        v["pruning"]["changed_exports"][0]["names"],
        serde_json::json!(["isCookie"])
    );
    assert_eq!(v["pruning"]["changed_exports"][0]["entries_skipped"], 3);
}

#[test]
fn a_changed_export_selects_the_tests_that_use_it_through_importers() {
    // The body of `hasSignal`: the service test runs it (service -> build ->
    // resolve -> migrate), the test of a constant in `build.ts` and the test of
    // another predicate don't.
    let d = tracking();
    let r = d.path();
    write(
        r,
        "src/attribution/utils.ts",
        &UTILS.replace("isUtm(k) || isParam(k)", "isParam(k) || isUtm(k)"),
    );
    assert_eq!(affected(r), ["test/service.test.ts"]);
    let e = explain(r, "test/service.test.ts");
    assert!(
        e.contains("src/attribution/utils.ts")
            && e.contains("src/attribution/migrate.ts")
            && e.contains("src/service.ts"),
        "{e}"
    );
    // Which export changed, and the file whose load-time code reads it.
    assert!(
        e.contains("  src/attribution/utils.ts: only hasSignal changed\n")
            && e.contains(
                "  src/service.ts reads build of src/build.ts when it is loaded (a `new \
                 Service`): every entry that loads it is affected\n"
            ),
        "{e}"
    );
    // And why a test that loads the file is not selected.
    let e = explain(r, "test/build.test.ts");
    assert!(
        e.contains("unaffected test/build.test.ts")
            && e.contains(
                "  src/attribution/utils.ts skipped: only hasSignal changed in it, and nothing \
                 this entry loads uses it\n"
            ),
        "{e}"
    );
    // Without definition-level facts of importers, a side-effect-free importer
    // passes the change on through all of its exports: the test of a constant
    // in `build.ts` is selected too.
    assert_eq!(
        affected_env(r, &[("TURBO_TRACE_DEBUG_SYMBOLS", "0")]),
        ["test/build.test.ts", "test/service.test.ts"]
    );
    // A predicate that both `hasSignal` and a test use.
    write(
        r,
        "src/attribution/utils.ts",
        &UTILS.replace("startsWith('utm_')", "startsWith('utm-')"),
    );
    assert_eq!(affected(r), ["test/service.test.ts", "test/utils.test.ts"]);
    // A comment and spacing: no export changed.
    write(
        r,
        "src/attribution/utils.ts",
        &UTILS.replace(
            "/** Whether a key is a tracked cookie key. */",
            "// cookie keys",
        ),
    );
    assert_eq!(affected(r), Vec::<String>::new());
}

#[test]
fn an_importer_that_reads_the_change_at_load_time_selects_its_dependents() {
    // `registry.ts` calls `hasSignal` while it loads: whatever a test uses of it,
    // the change can show.
    let d = tracking();
    let r = d.path();
    write(
        r,
        "src/registry.ts",
        "import { hasSignal } from './attribution/utils';\nexport const initial = \
         hasSignal({});\nexport const NAME = 'registry';\n",
    );
    write(
        r,
        "test/registry.test.ts",
        "import { NAME } from '../src/registry';\ntest('name', () => \
         expect(NAME).toBe('registry'));\n",
    );
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "registry"]);
    write(
        r,
        "src/attribution/utils.ts",
        &UTILS.replace("isUtm(k) || isParam(k)", "isParam(k) || isUtm(k)"),
    );
    assert_eq!(
        affected(r),
        ["test/registry.test.ts", "test/service.test.ts"]
    );
    let e = explain(r, "test/registry.test.ts");
    assert!(
        e.contains(
            "  src/registry.ts reads hasSignal of src/attribution/utils.ts when it is loaded (a \
             call of hasSignal): every entry that loads it is affected\n"
        ),
        "{e}"
    );
}

const PREDICATE: &str =
    "// The symbol is not exported by the library, so it is defined here.\nconst POSTPONE: symbol \
     = Symbol.for('lib.postpone');\n\nfunction isPostpone(error: unknown): boolean {\n  return \
     typeof error === 'object' && error !== null && '$$typeof' in error && error.$$typeof === \
     POSTPONE;\n}\n\nexport function isInternalError(error: unknown): boolean {\n  if \
     (isPostpone(error)) return true;\n  if (typeof error !== 'object' || error === null || \
     !('digest' in error)) return false;\n  const code = String(error.digest).split(';')[0];\n  \
     return code === 'REDIRECT' || code === 'NOT_FOUND';\n}\n\nexport function describe(error: \
     unknown): string {\n  return String(error);\n}\n";

/// The error predicate (a load-time `Symbol.for`) behind a fetch wrapper:
/// `fetchApi` calls it, `toFetchError` does not; a hook uses `fetchApi`.
fn fetching() -> tempfile::TempDir {
    commit(&[
        ("src/is-internal-error.ts", PREDICATE),
        (
            "src/fetch-api.ts",
            "import { isInternalError } from './is-internal-error';\n\nconst NETWORK_ERROR = \
             'network_error';\n\nexport async function fetchApi(url: string): Promise<unknown> \
             {\n  try {\n    return await (await fetch(url)).json();\n  } catch (e) {\n    if \
             (isInternalError(e)) throw e;\n    throw toFetchError(e);\n  }\n}\n\nexport function \
             toFetchError(e: unknown): Error {\n  return new Error(NETWORK_ERROR + ': ' + \
             String(e));\n}\n",
        ),
        (
            "src/use-team.ts",
            "import { fetchApi } from './fetch-api';\n\nexport const TEAM_KEY = 'team';\n\nexport \
             function loadTeam(id: string) {\n  return fetchApi('/teams/' + id);\n}\n",
        ),
        (
            "test/team.test.ts",
            "import { loadTeam } from '../src/use-team';\ntest('team', () => expect(typeof \
             loadTeam).toBe('function'));\n",
        ),
        (
            "test/key.test.ts",
            "import { TEAM_KEY } from '../src/use-team';\ntest('key', () => \
             expect(TEAM_KEY).toBe('team'));\n",
        ),
        (
            "test/error.test.ts",
            "import { toFetchError } from '../src/fetch-api';\ntest('error', () => \
             expect(toFetchError(1).message).toContain('network'));\n",
        ),
    ])
}

#[test]
fn load_time_code_that_did_not_change_does_not_hide_which_export_changed() {
    let d = fetching();
    let r = d.path();
    // One more code in the predicate, and a reworded comment.
    write(
        r,
        "src/is-internal-error.ts",
        &PREDICATE
            .replace(
                "code === 'REDIRECT' ||",
                "code === 'REDIRECT' || code === 'HANGING' ||",
            )
            .replace("so it is defined here", "so we define it here"),
    );
    assert_eq!(affected(r), ["test/team.test.ts"]);
    // As before: a file with a load-time call is a whole-file change.
    assert_eq!(
        affected_env(
            r,
            &[
                ("TURBO_TRACE_DEBUG_EXPORT_DIFF", "0"),
                ("TURBO_TRACE_DEBUG_SYMBOLS", "0")
            ]
        ),
        [
            "test/error.test.ts",
            "test/key.test.ts",
            "test/team.test.ts"
        ]
    );
    let e = explain(r, "test/error.test.ts");
    assert!(
        e.contains(
            "src/is-internal-error.ts skipped: only isInternalError changed in it, and nothing \
             this entry loads uses it"
        ),
        "{e}"
    );
    // An export nobody uses.
    write(
        r,
        "src/is-internal-error.ts",
        &PREDICATE.replace("return String(error);", "return `${error}`;"),
    );
    assert_eq!(affected(r), Vec::<String>::new());
}

#[test]
fn changed_load_time_code_is_a_whole_file_change() {
    let d = fetching();
    let r = d.path();
    // `Symbol.for` only makes a value: changing its argument changes the export
    // that reads the symbol, nothing else.
    write(
        r,
        "src/is-internal-error.ts",
        &PREDICATE.replace("'lib.postpone'", "'lib.postpone.v2'"),
    );
    assert_eq!(affected(r), ["test/team.test.ts"]);
    // A lookup that runs code at load time (a member access can be a getter): the
    // statement itself changes.
    let lookup = PREDICATE.replace("Symbol.for('lib.postpone')", "globalThis.symbols.postpone");
    write(r, "src/is-internal-error.ts", &lookup);
    git(r, &["commit", "-q", "-am", "lookup"]);
    write(
        r,
        "src/is-internal-error.ts",
        &lookup.replace("symbols.postpone", "symbols.postponed"),
    );
    assert_eq!(
        affected(r),
        [
            "test/error.test.ts",
            "test/key.test.ts",
            "test/team.test.ts"
        ]
    );
    let v = json(r);
    assert_eq!(
        v["barrels"]["not_skippable"][0]["reason"],
        "head: top-level side effects"
    );
    assert_eq!(
        v["pruning"]["load_time_changes"][0]["what"],
        "a member access is new or changed"
    );
    let e = explain(r, "test/error.test.ts");
    assert!(
        e.contains(
            "  src/is-internal-error.ts counts as changed as a whole: what it does when it is \
             loaded differs between the base and the head (a member access is new or changed)\n"
        ),
        "{e}"
    );
    // A new load-time statement.
    write(
        r,
        "src/is-internal-error.ts",
        &format!("{PREDICATE}console.log('loaded');\n"),
    );
    assert_eq!(
        affected(r),
        [
            "test/error.test.ts",
            "test/key.test.ts",
            "test/team.test.ts"
        ]
    );
}

#[test]
fn a_load_time_difference_the_comparison_finds_is_a_whole_file_change() {
    // `m.ts` stops using its import of `log.ts` as a value: a transpiler drops the
    // import, and `log.ts`, which logs when it is loaded, no longer runs. Both
    // versions of `m.ts` are free of side effects and import the same modules
    // as written, so barrel pruning alone would track `m.ts` by its exports and
    // skip a test that loads it without naming one. The comparison knows that
    // the module's loads differ.
    let d = commit(&[
        (
            "src/log.ts",
            "console.log('loaded');\nexport const tag = 1;\n",
        ),
        (
            "src/m.ts",
            "import { tag } from './log';\nexport const a = () => tag;\nexport const b = 2;\n",
        ),
        (
            "test/side.test.ts",
            "import '../src/m';\ntest('loads', () => expect(1).toBe(1));\n",
        ),
    ]);
    let r = d.path();
    write(
        r,
        "src/m.ts",
        "import { tag } from './log';\nexport const a = () => 1;\nexport const b = 2;\n",
    );
    assert_eq!(affected(r), ["test/side.test.ts"]);
    let v = json(r);
    assert_eq!(
        v["barrels"]["not_skippable"][0]["reason"],
        "what the file does when it is loaded differs between base and head"
    );
    assert_eq!(
        v["pruning"]["load_time_changes"][0]["what"],
        "an import is used as a value at one revision only (a transpiler drops an unused import)"
    );
    // A change of `a` that keeps the import in use is a change of `a` only.
    write(
        r,
        "src/m.ts",
        "import { tag } from './log';\nexport const a = () => tag + 1;\nexport const b = 2;\n",
    );
    assert_eq!(affected(r), Vec::<String>::new());
}

#[test]
fn a_mock_of_a_builder_selects_the_tests_that_load_a_file_relying_on_it() {
    // `declareFlag` only builds a value, so a changed declaration changes `flagX`
    // and nothing else. A test that mocks the module `declareFlag` comes from
    // sees the call and its argument whatever it reads.
    let d = commit(&[
        (
            "src/declare.ts",
            "export const declareFlag = (o: { key: string }) => ({ o });\n",
        ),
        (
            "src/flags.ts",
            "import { declareFlag } from './declare';\nexport const flagX = declareFlag({ key: \
             'x' });\nexport const other = 1;\n",
        ),
        (
            "test/other.test.ts",
            "import { other } from '../src/flags';\ntest('other', () => expect(other).toBe(1));\n",
        ),
        (
            "test/mocked.test.ts",
            "import { other } from '../src/flags';\nimport { declareFlag } from \
             '../src/declare';\njest.mock('../src/declare', () => ({ declareFlag: jest.fn((o) => \
             o) }));\ntest('calls', () => expect([other, (declareFlag as \
             jest.Mock).mock.calls.length]).toEqual([1, 1]));\n",
        ),
        (
            "test/unrelated.test.ts",
            "import { declareFlag } from \
             '../src/declare';\njest.mock('../src/declare');\ntest('mock', () => \
             expect(declareFlag).toBeDefined());\n",
        ),
    ]);
    let r = d.path();
    write(
        r,
        "src/flags.ts",
        "import { declareFlag } from './declare';\nexport const flagX = declareFlag({ key: 'y' \
         });\nexport const other = 1;\n",
    );
    // Not the test that reads `other` without a mock, and not the one that mocks
    // the builder without loading the file that calls it.
    assert_eq!(affected(r), ["test/mocked.test.ts"]);
    let e = explain(r, "test/mocked.test.ts");
    assert!(
        e.contains("  src/flags.ts: only flagX changed\n")
            && e.contains(
                "  test/mocked.test.ts mocks a module whose function src/flags.ts calls while it \
                 is loaded (../src/declare for src/declare.ts): the call was taken to only build \
                 a value, and the mock sees it\n"
            ),
        "{e}"
    );
    let v = json(r);
    assert_eq!(
        v["pruning"]["builders"]["relied_on"][0]["builder"],
        "src/declare.ts"
    );
}

#[test]
fn a_function_that_reads_this_reads_the_namespace_of_a_barrel() {
    // `api.viaThis()` hands the namespace of the barrel to `viaThis`, which reads
    // `hidden` from it: a change of `hidden` reaches the test although it names
    // only `viaThis`.
    let d = commit(&[
        (
            "src/a.ts",
            "export function viaThis(this: any) {\n  return this ? this.hidden() : 0;\n}\n",
        ),
        ("src/c.ts", "export function hidden() {\n  return 1;\n}\n"),
        (
            "src/index.ts",
            "export { viaThis } from './a';\nexport { hidden } from './c';\n",
        ),
        (
            "test/ns.test.ts",
            "import * as api from '../src/index';\ntest('ns', () => \
             expect(api.viaThis()).toBe(1));\n",
        ),
        (
            "test/read.test.ts",
            "import * as api from '../src/index';\ntest('read', () => expect(typeof \
             api.viaThis).toBe('function'));\n",
        ),
        (
            "test/named.test.ts",
            "import { viaThis } from '../src/index';\ntest('named', () => \
             expect(viaThis()).toBe(0));\n",
        ),
    ]);
    let r = d.path();
    write(
        r,
        "src/c.ts",
        "export function hidden() {\n  return 2;\n}\n",
    );
    // A named import calls the function without a receiver, and a namespace that is
    // only read member by member is handed to nobody.
    assert_eq!(affected(r), ["test/ns.test.ts"]);
}

#[test]
fn a_modified_test_always_runs() {
    // Only a comment and an unused helper change in a test file.
    let d = fetching();
    let r = d.path();
    write(
        r,
        "test/key.test.ts",
        "// the key\nimport { TEAM_KEY } from '../src/use-team';\nfunction unused() \
         {}\ntest('key', () => expect(TEAM_KEY).toBe('team'));\n",
    );
    assert_eq!(affected(r), ["test/key.test.ts"]);
}

#[test]
fn whole_module_uses_of_an_importer_select() {
    // A test that mocks the fetch wrapper, and one that takes its namespace apart
    // at run time: both can observe any export of it.
    let d = fetching();
    let r = d.path();
    write(
        r,
        "test/mocked.test.ts",
        "import { TEAM_KEY } from '../src/use-team';\njest.mock('../src/fetch-api');\ntest('m', \
         () => expect(TEAM_KEY).toBe('team'));\n",
    );
    write(
        r,
        "test/namespace.test.ts",
        "import * as api from '../src/fetch-api';\ntest('n', () => \
         expect(Object.keys(api).length).toBe(2));\n",
    );
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "tests"]);
    write(
        r,
        "src/is-internal-error.ts",
        &PREDICATE.replace(
            "code === 'REDIRECT' ||",
            "code === 'REDIRECT' || code === 'HANGING' ||",
        ),
    );
    assert_eq!(
        affected(r),
        [
            "test/mocked.test.ts",
            "test/namespace.test.ts",
            "test/team.test.ts"
        ]
    );
}

/// [`fetching`] plus the load-time wrappers of the real site: a route handler
/// wrapped by a first-party tracing wrapper that returns a closure, a data
/// loader wrapped by React's `cache`, and a registry whose `register` keeps its
/// argument.
fn wrappers() -> tempfile::TempDir {
    let d = fetching();
    let r = d.path();
    write(
        r,
        "src/trace.ts",
        "function run<T>(fn: () => T, _options: { name: string }): T {\n  return \
         fn();\n}\n\nexport function withTrace(\n  handler: (req: Request) => Promise<unknown>,\n  \
         options: { name: string },\n) {\n  return (req: Request): Promise<unknown> => {\n    \
         return run(() => handler(req), options);\n  };\n}\n\nconst handlers: unknown[] = \
         [];\n\nexport function register<T>(handler: T): T {\n  handlers.push(handler);\n  return \
         handler;\n}\n",
    );
    write(
        r,
        "src/route.ts",
        "import { withTrace } from './trace';\nimport { fetchApi } from './fetch-api';\n\nexport \
         const GET = withTrace(async (req) => fetchApi(req.url), { name: 'get' });\n\nexport \
         const dynamic = 'force-dynamic';\n",
    );
    write(
        r,
        "src/data.ts",
        "import { cache } from 'react';\nimport { fetchApi } from './fetch-api';\n\nexport const \
         getProduct = cache(async (slug: string) => fetchApi('/products/' + slug));\n\nexport \
         const PRODUCT_TAG = 'products';\n",
    );
    write(
        r,
        "src/registered.ts",
        "import { register } from './trace';\nimport { fetchApi } from './fetch-api';\n\nexport \
         const POST = register(async (req: Request) => fetchApi(req.url));\n\nexport const \
         runtime = 'nodejs';\n",
    );
    for (name, import, expect) in [
        ("route", "{ GET } from '../src/route'", "typeof GET"),
        ("dynamic", "{ dynamic } from '../src/route'", "dynamic"),
        (
            "product",
            "{ getProduct } from '../src/data'",
            "typeof getProduct",
        ),
        ("tag", "{ PRODUCT_TAG } from '../src/data'", "PRODUCT_TAG"),
        ("runtime", "{ runtime } from '../src/registered'", "runtime"),
    ] {
        write(
            r,
            &format!("test/{name}.test.ts"),
            &format!("import {import};\ntest('{name}', () => expect({expect}).toBeTruthy());\n"),
        );
    }
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "wrappers"]);
    d
}

#[test]
fn a_load_time_call_that_only_builds_a_value_passes_the_change_on_by_name() {
    let d = wrappers();
    let r = d.path();
    write(
        r,
        "src/is-internal-error.ts",
        &PREDICATE.replace(
            "code === 'REDIRECT' ||",
            "code === 'REDIRECT' || code === 'HANGING' ||",
        ),
    );
    // `GET = withTrace(...)` wraps a function that reaches the change: its own test
    // runs, the test of the constant next to it doesn't. `register(...)` keeps
    // its argument in module state: everything that loads that file.
    // `getProduct = cache(...)` too: React's `cache` is not in every React
    // build (18 stable has none), so a call of it is load-time code.
    assert_eq!(
        affected(r),
        [
            "test/product.test.ts",
            "test/route.test.ts",
            "test/runtime.test.ts",
            "test/tag.test.ts",
            "test/team.test.ts"
        ]
    );
    let v = json(r);
    assert_eq!(v["pruning"]["importers"]["pure_load_time_calls"], 1);
    let reasons: Vec<&str> = v["pruning"]["importers"]["read_at_load_time"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["reason"].as_str().unwrap())
        .collect();
    assert_eq!(reasons, ["a call of cache", "a call of register"]);
    // Without the rule every load-time call ends the walk.
    assert_eq!(
        affected_env(r, &[("TURBO_TRACE_DEBUG_PURE_CALLS", "0")]),
        [
            "test/dynamic.test.ts",
            "test/product.test.ts",
            "test/route.test.ts",
            "test/runtime.test.ts",
            "test/tag.test.ts",
            "test/team.test.ts"
        ]
    );
}

#[test]
fn a_changed_wrapper_is_load_time_code_of_its_callers() {
    // The wrapper itself changes (it now calls the handler once while wrapping):
    // what its callers do at load time changed, so every test that loads a
    // caller runs.
    let d = wrappers();
    let r = d.path();
    let trace = std::fs::read_to_string(r.join("src/trace.ts")).unwrap();
    write(
        r,
        "src/trace.ts",
        &trace.replace(
            "  return (req: Request): Promise<unknown> => {",
            "  void handler;\n  return (req: Request): Promise<unknown> => {",
        ),
    );
    assert_eq!(affected(r), ["test/dynamic.test.ts", "test/route.test.ts"]);
}

/// The shape of a flag registry and what stands around it on the real site: a
/// declarator in its own file that hands its argument and an adapter to the
/// flags SDK, the adapter made by the SDK's factory, a registry of one
/// declaration per export, a schema file built with zod, and a route table
/// whose labels use an imported constant.
fn registry() -> tempfile::TempDir {
    commit(&[
        (
            "src/flags/client.ts",
            "export const client = { project: 'site' };\n",
        ),
        (
            "src/flags/adapter.ts",
            "import { createVercelAdapter } from '@flags-sdk/vercel';\nimport { client } from \
             './client';\n\nexport const adapter = createVercelAdapter(client);\n",
        ),
        (
            "src/flags/identify.ts",
            "export async function identify() {\n  return { user: 'u' };\n}\n",
        ),
        (
            "src/flags/declare.ts",
            "import { flag } from 'flags/next';\nimport { adapter } from './adapter';\nimport { \
             identify } from './identify';\n\nexport function declareFlag<T>(declaration: { key: \
             string; defaultValue: T; description?: string }) {\n  return flag({ ...declaration, \
             adapter: adapter(), identify });\n}\n",
        ),
        (
            "src/flags/server.ts",
            "import { declareFlag } from './declare';\n\nexport const enableComments = \
             declareFlag<boolean>({\n  key: 'comments',\n  defaultValue: false,\n});\n\nexport \
             const enableUsageCache = declareFlag<boolean>({\n  key: 'usage-cache',\n  \
             defaultValue: true,\n});\n\nexport const enableJsonLogs = declareFlag<boolean>({\n  \
             key: 'json-logs',\n  defaultValue: false,\n  description:\n    'Render JSON in \
             logs.',\n});\n",
        ),
        (
            "src/comments.ts",
            "import { enableComments } from './flags/server';\n\nexport async function \
             commentsTab() {\n  return (await enableComments()) ? 'comments' : null;\n}\n",
        ),
        (
            "src/logs.ts",
            "import { enableJsonLogs } from './flags/server';\n\nexport async function renderer() \
             {\n  return (await enableJsonLogs()) ? 'json' : 'text';\n}\n",
        ),
        (
            "src/schemas.ts",
            "import { z } from 'zod';\n\nconst environment = z.enum(['production', \
             'preview']);\nconst level = z.enum(['error', 'info']);\n\nexport const query = \
             z.object({\n  env: environment.optional(),\n  limit: \
             z.coerce.number().positive().max(100),\n});\n\nexport const row = z.object({ level, \
             message: z.string() });\n",
        ),
        ("src/constants.ts", "export const AGENT_NAME = 'Agent';\n"),
        (
            "src/routes.ts",
            "import { AGENT_NAME } from './constants';\n\nconst definitions = {\n  '/[team]': { \
             label: 'Overview' },\n  '/[team]/agent': { label: `${AGENT_NAME} Credits`, visible: \
             () => true },\n};\n\nexport const Routes = definitions;\n\nexport function \
             isKnown(path: string): boolean {\n  return path.startsWith('/');\n}\n",
        ),
        (
            "test/comments.test.ts",
            "import { commentsTab } from '../src/comments';\ntest('c', () => expect(typeof \
             commentsTab).toBe('function'));\n",
        ),
        (
            "test/logs.test.ts",
            "import { renderer } from '../src/logs';\ntest('l', () => expect(typeof \
             renderer).toBe('function'));\n",
        ),
        (
            "test/query.test.ts",
            "import { query } from '../src/schemas';\ntest('q', () => \
             expect(query).toBeTruthy());\n",
        ),
        (
            "test/row.test.ts",
            "import { row } from '../src/schemas';\ntest('r', () => expect(row).toBeTruthy());\n",
        ),
        (
            "test/routes.test.ts",
            "import { Routes } from '../src/routes';\ntest('t', () => \
             expect(Routes['/[team]'].label).toBe('Overview'));\n",
        ),
        (
            "test/known.test.ts",
            "import { isKnown } from '../src/routes';\ntest('k', () => \
             expect(isKnown('/x')).toBe(true));\n",
        ),
    ])
}

const BEFORE: [(&str, &str); 2] = [
    ("TURBO_TRACE_DEBUG_PURE_CALLS", "0"),
    ("TURBO_TRACE_DEBUG_SYMBOLS", "0"),
];

#[test]
fn a_flag_added_to_a_registry_selects_nothing_else() {
    let d = registry();
    let r = d.path();
    let server = std::fs::read_to_string(r.join("src/flags/server.ts")).unwrap();
    // One flag goes away with its only reader gone already, another is new.
    let without = server.replace(
        "export const enableUsageCache = declareFlag<boolean>({\n  key: 'usage-cache',\n  \
         defaultValue: true,\n});\n\n",
        "",
    );
    assert_ne!(without, server);
    write(
        r,
        "src/flags/server.ts",
        &format!(
            "{without}\nexport const enableRequestDetails = declareFlag<boolean>({{\n  key: \
             'request-details',\n  defaultValue: false,\n}});\n"
        ),
    );
    assert_eq!(affected(r), Vec::<String>::new());
    // Before: declaring a flag is a load-time call, the registry a whole-file
    // change.
    assert_eq!(
        affected_env(r, &BEFORE),
        ["test/comments.test.ts", "test/logs.test.ts"]
    );
    let v = json(r);
    assert_eq!(
        v["pruning"]["changed_exports"][0]["names"],
        serde_json::json!(["enableRequestDetails", "enableUsageCache"])
    );
    // A changed default selects the readers of that flag.
    write(
        r,
        "src/flags/server.ts",
        &server.replace(
            "key: 'json-logs',\n  defaultValue: false",
            "key: 'json-logs',\n  defaultValue: true",
        ),
    );
    assert_eq!(affected(r), ["test/logs.test.ts"]);
    let e = explain(r, "test/comments.test.ts");
    assert!(
        e.contains("src/flags/server.ts skipped: only enableJsonLogs changed in it"),
        "{e}"
    );
}

#[test]
fn a_registry_entry_that_runs_code_is_load_time_code() {
    let d = registry();
    let r = d.path();
    let server = std::fs::read_to_string(r.join("src/flags/server.ts")).unwrap();
    // The new declaration computes its key by a call nothing is known about.
    write(
        r,
        "src/flags/server.ts",
        &format!(
            "{server}\nexport const enableX = declareFlag<boolean>({{\n  key: lookupKey('x'),\n  \
             defaultValue: false,\n}});\n"
        ),
    );
    assert_eq!(affected(r), ["test/comments.test.ts", "test/logs.test.ts"]);
    assert_eq!(
        json(r)["pruning"]["load_time_changes"][0]["what"],
        "a call of lookupKey is new or changed"
    );
    // The declarator itself changes (it now reports every declaration): its
    // callers' load-time behaviour changed.
    let d = registry();
    let r = d.path();
    let declare = std::fs::read_to_string(r.join("src/flags/declare.ts")).unwrap();
    write(
        r,
        "src/flags/declare.ts",
        &declare.replace(
            "  return flag({",
            "  console.log(declaration.key);\n  return flag({",
        ),
    );
    assert_eq!(affected(r), ["test/comments.test.ts", "test/logs.test.ts"]);
    // The adapter is made differently: the declarator is no longer taken on trust
    // at the base revision, so a flag added in the same change is a whole-file
    // change again.
    let d = registry();
    let r = d.path();
    write(
        r,
        "src/flags/adapter.ts",
        "import { createVercelAdapter } from '@flags-sdk/vercel';\nimport { client } from \
         './client';\n\nexport const adapter = createVercelAdapter({ ...client });\n",
    );
    write(
        r,
        "src/flags/server.ts",
        &format!(
            "{server}\nexport const enableY = declareFlag<boolean>({{\n  key: 'y',\n  \
             defaultValue: false,\n}});\n"
        ),
    );
    assert_eq!(affected(r), ["test/comments.test.ts", "test/logs.test.ts"]);
}

#[test]
fn schema_builders_and_tables_over_constants_are_compared_by_export() {
    let d = registry();
    let r = d.path();
    // A schema nobody else uses is removed from the schema file, another one
    // tightened.
    let schemas = std::fs::read_to_string(r.join("src/schemas.ts")).unwrap();
    write(
        r,
        "src/schemas.ts",
        &schemas.replace(
            "z.enum(['error', 'info'])",
            "z.enum(['error', 'info', 'warn'])",
        ),
    );
    assert_eq!(affected(r), ["test/row.test.ts"]);
    assert_eq!(
        affected_env(r, &BEFORE),
        ["test/query.test.ts", "test/row.test.ts"]
    );
    // That a zod builder only builds a value is an assumption about a package: it
    // is stated for the test that is not selected, and `--strict` does not make
    // it.
    let e = explain(r, "test/query.test.ts");
    assert!(
        e.contains("unaffected test/query.test.ts")
            && e.contains(
                "  assumed: both revisions of a changed file transpile (syntax and early errors \
                 are checked), and that the builders of zod only build a value from their \
                 arguments when a module calls them while it loads (--strict does not assume \
                 that)\n"
            ),
        "{e}"
    );
    // (`--strict` also opens the imports of packages that are not installed here,
    // so the list has more than these two; what the rule did is in the JSON.)
    let strict: serde_json::Value = serde_json::from_str(&ok_env(
        r,
        &[
            "affected", "--tests", "jest", "--since", "HEAD", "--strict", "--format", "json",
        ],
        &[],
    ))
    .unwrap();
    // The schema file's own builder calls are load-time code now, and one of them
    // changed: the file counts as a whole.
    let changes = &strict["pruning"]["load_time_changes"];
    assert_eq!(changes[0]["path"], "src/schemas.ts", "{changes}");
    assert!(
        changes[0]["what"]
            .as_str()
            .unwrap()
            .starts_with("a call of z."),
        "{changes}"
    );
    let selected: Vec<&str> = strict["results"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["verdict"] == "affected")
        .filter_map(|e| e["entry"].as_str())
        .collect();
    assert!(selected.contains(&"test/query.test.ts"), "{selected:?}");
    // The one switch of the rule.
    assert_eq!(
        affected_env(r, &[("TURBO_TRACE_DEBUG_EXPORTS", "0")]),
        ["test/query.test.ts", "test/row.test.ts"]
    );
    // A schema that is parsed while the file loads is load-time code.
    write(
        r,
        "src/schemas.ts",
        &format!("{schemas}\nexport const fallback = level.parse('info');\n"),
    );
    assert_eq!(affected(r), ["test/query.test.ts", "test/row.test.ts"]);
    // An entry added to the route table: the table's readers, not the readers of a
    // function next to it.
    let d = registry();
    let r = d.path();
    let routes = std::fs::read_to_string(r.join("src/routes.ts")).unwrap();
    write(
        r,
        "src/routes.ts",
        &routes.replace(
            "  '/[team]': { label: 'Overview' },\n",
            "  '/[team]': { label: 'Overview' },\n  '/[team]/requests': { label: 'My Requests' \
             },\n",
        ),
    );
    assert_eq!(affected(r), ["test/routes.test.ts"]);
    assert_eq!(
        affected_env(r, &BEFORE),
        ["test/known.test.ts", "test/routes.test.ts"]
    );
    // The constant in a label changes too: it is what the template is made from.
    write(
        r,
        "src/constants.ts",
        "export const AGENT_NAME = 'Assistant';\n",
    );
    assert_eq!(affected(r), ["test/known.test.ts", "test/routes.test.ts"]);
}

/// The shape of a permissions enum in a shared types file: most code reads one
/// member.
fn permissions() -> tempfile::TempDir {
    commit(&[
        (
            "src/permissions.ts",
            "export enum Action {\n  Read = 'read',\n  Write = 'write',\n}\n\nexport enum \
             Resource {\n  Team = 'team',\n  Project = 'project',\n}\n\nexport type Permissions = \
             Partial<Record<Resource, Action[]>>;\n",
        ),
        (
            "src/use-permissions.ts",
            "import { Action, type Permissions, Resource } from './permissions';\n\nexport \
             function canEditTeam(p: Permissions): boolean {\n  return \
             Boolean(p[Resource.Team]?.includes(Action.Write));\n}\n\nexport function \
             canSeeProject(p: Permissions): boolean {\n  return Boolean(p[Resource.Project]);\n}\n",
        ),
        (
            "src/all.ts",
            "import { Resource } from './permissions';\n\nexport function allResources(): \
             string[] {\n  return Object.values(Resource);\n}\n",
        ),
        (
            "test/team.test.ts",
            "import { canEditTeam } from '../src/use-permissions';\ntest('team', () => \
             expect(canEditTeam({})).toBe(false));\n",
        ),
        (
            "test/project.test.ts",
            "import { canSeeProject } from '../src/use-permissions';\ntest('project', () => \
             expect(canSeeProject({})).toBe(false));\n",
        ),
        (
            "test/all.test.ts",
            "import { allResources } from '../src/all';\ntest('all', () => \
             expect(allResources()).toContain('team'));\n",
        ),
        (
            "test/direct.test.ts",
            "import { Resource } from '../src/permissions';\ntest('direct', () => \
             expect(Resource.Team).toBe('team'));\n",
        ),
    ])
}

#[test]
fn a_member_added_to_an_enum_selects_the_code_that_enumerates_it() {
    let d = permissions();
    let r = d.path();
    let src = std::fs::read_to_string(r.join("src/permissions.ts")).unwrap();
    write(
        r,
        "src/permissions.ts",
        &src.replace(
            "  Project = 'project',\n",
            "  Project = 'project',\n  Rules = 'rules',\n",
        ),
    );
    // Only `allResources` takes the enum as a whole; the others read members that
    // did not change.
    assert_eq!(affected(r), ["test/all.test.ts"]);
    assert_eq!(
        affected_env(r, &BEFORE),
        [
            "test/all.test.ts",
            "test/direct.test.ts",
            "test/project.test.ts",
            "test/team.test.ts"
        ]
    );
    let e = explain(r, "test/team.test.ts");
    assert!(
        e.contains("src/permissions.ts skipped: only") && e.contains("Resource.Rules"),
        "{e}"
    );
    // A member's value: its readers, and the code that enumerates.
    write(
        r,
        "src/permissions.ts",
        &src.replace("Team = 'team'", "Team = 'teams'"),
    );
    assert_eq!(
        affected(r),
        [
            "test/all.test.ts",
            "test/direct.test.ts",
            "test/team.test.ts"
        ]
    );
    // A member of the other enum, read by one function.
    write(
        r,
        "src/permissions.ts",
        &src.replace("Write = 'write'", "Write = 'w'"),
    );
    assert_eq!(affected(r), ["test/team.test.ts"]);
    // An enum whose members get their values from their position is one value.
    let d = permissions();
    let r = d.path();
    let numbered = src.replace(
        "  Team = 'team',\n  Project = 'project',\n",
        "  Team,\n  Project,\n",
    );
    write(r, "src/permissions.ts", &numbered);
    git(r, &["commit", "-q", "-am", "numbered"]);
    write(
        r,
        "src/permissions.ts",
        &numbered.replace("  Team,\n", "  Org,\n  Team,\n"),
    );
    assert_eq!(
        affected(r),
        [
            "test/all.test.ts",
            "test/direct.test.ts",
            "test/project.test.ts",
            "test/team.test.ts"
        ]
    );
}

/// Without definition-level facts (here: switched off) the names a barrel
/// forwards are not compared, so a changed re-export list keeps the file a
/// whole-file change. Found by the exports oracle with the rule off: the
/// importer of a dropped name was not selected.
#[test]
fn a_dropped_reexport_selects_its_importers_without_facts() {
    let d = commit(&[
        ("src/m1.ts", "export const a = 1;\nexport const b = 2;\n"),
        ("src/index.ts", "export { a as renamed, b } from './m1';\n"),
        (
            "src/user.ts",
            "import { renamed } from './index';\nexport const use = () => renamed;\n",
        ),
        (
            "test/user.test.ts",
            "import { use } from '../src/user';\ntest('u', () => expect(use()).toBe(1));\n",
        ),
    ]);
    let r = d.path();
    write(r, "src/index.ts", "export { b } from './m1';\n");
    assert_eq!(affected(r), ["test/user.test.ts"]);
    assert_eq!(
        affected_env(
            r,
            &[
                ("TURBO_TRACE_DEBUG_EXPORT_DIFF", "0"),
                ("TURBO_TRACE_DEBUG_SYMBOLS", "0")
            ]
        ),
        ["test/user.test.ts"]
    );
}
