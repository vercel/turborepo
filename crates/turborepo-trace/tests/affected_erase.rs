//! `affected`: changes a test cannot observe when its runner only
//! transpiles.
//!
//! - Erased-equal changes (`--since`): a modified file whose runtime JavaScript
//!   is the same at the base revision and in the working tree (only types,
//!   comments or formatting changed) does not select tests that load it as a
//!   module.
//! - Type-only edges per entry: such tests don't follow `import type` edges;
//!   build roots, tasks and type-checking runners do.
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

const JEST_CONFIG: &str = "module.exports = { testMatch: ['**/test/**/*.test.ts'] };\n";

const TYPES: &str = "export interface User {\n  id: string;\n}\nexport enum Role {\n  Admin = \
                     'admin',\n}\nexport const DEFAULT_ROLE = Role.Admin;\n";

/// A jest repo, committed:
/// - `src/types.ts`: an interface, an enum and a constant;
/// - `src/model.ts` imports `{ DEFAULT_ROLE, User }` from it (a value import);
/// - `src/view.ts` has only `import type { User }` from it;
/// - `test/model.test.ts` loads `model.ts`, `test/view.test.ts` loads
///   `view.ts`, `test/other.test.ts` loads neither.
fn repo(extra: &[(&str, &str)]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(r, ".gitignore", ".turbo/trace/\n");
    write(r, "jest.config.js", JEST_CONFIG);
    write(r, "src/types.ts", TYPES);
    write(
        r,
        "src/model.ts",
        "import { DEFAULT_ROLE, User } from './types';\nexport function role(u: User) {\n  return \
         u.id ? DEFAULT_ROLE : undefined;\n}\n",
    );
    write(
        r,
        "src/view.ts",
        "import type { User } from './types';\nexport function label(u: User): string {\n  return \
         u.id;\n}\n",
    );
    write(r, "src/other.ts", "export const other = 1;\n");
    write(
        r,
        "test/model.test.ts",
        "import { role } from '../src/model';\ntest('m', () => expect(role({ id: 'a' \
         })).toBe('admin'));\n",
    );
    write(
        r,
        "test/view.test.ts",
        "import { label } from '../src/view';\ntest('v', () => expect(label({ id: 'a' \
         })).toBe('a'));\n",
    );
    write(
        r,
        "test/other.test.ts",
        "import { other } from '../src/other';\ntest('o', () => expect(other).toBe(1));\n",
    );
    for (p, c) in extra {
        write(r, p, c);
    }
    git(r, &["init", "-q"]);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "base"]);
    d
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

fn json(root: &Path, more: &[&str]) -> serde_json::Value {
    let mut args = vec![
        "affected", "--tests", "jest", "--since", "HEAD", "--format", "json",
    ];
    args.extend_from_slice(more);
    serde_json::from_str(&ok_env(root, &args, &[])).unwrap()
}

const NONE: [&str; 0] = [];

#[test]
fn type_only_change_selects_no_transpiled_test() {
    let d = repo(&[]);
    let r = d.path();
    // A new field, a new interface, a new alias and a comment: no runtime change.
    write(
        r,
        "src/types.ts",
        &TYPES.replace(
            "  id: string;\n}\n",
            "  id: string;\n  /** Display name. */\n  name?: string;\n}\nexport interface Team \
             {\n  users: User[];\n}\nexport type Id = User['id'];\n",
        ),
    );
    assert_eq!(affected(r), NONE);
    let e = explain(r, "test/model.test.ts");
    assert!(
        e.contains("unaffected test/model.test.ts")
            && e.contains(
                "src/types.ts skipped: only types, comments or formatting changed (same \
                 JavaScript as at the base revision)"
            ),
        "{e}"
    );
    let v = json(r, &[]);
    assert_eq!(v["affected"], 0);
    assert_eq!(v["pruning"]["erased"], serde_json::json!(["src/types.ts"]));
    assert_eq!(v["pruning"]["erased_skipped"][0]["path"], "src/types.ts");
    assert_eq!(v["pruning"]["erased_skipped"][0]["entries_skipped"], 1);
    // The off switches. Without the erased-equal rule the change is still found to
    // be one of type exports only, which `model.ts` reads as types
    // (`turborepo_trace_lang_js::symbols`); without that as well, it is a
    // change of everything `types.ts` exports.
    assert_eq!(affected_env(r, &[("TURBO_TRACE_DEBUG_ERASE", "0")]), NONE);
    assert_eq!(
        affected_env(
            r,
            &[
                ("TURBO_TRACE_DEBUG_ERASE", "0"),
                ("TURBO_TRACE_DEBUG_SYMBOLS", "0")
            ]
        ),
        ["test/model.test.ts"]
    );
    // An entry that is not a test (a build root) sees every change.
    let out = ok_env(
        r,
        &["affected", "--since", "HEAD", "src/model.ts", "src/view.ts"],
        &[],
    );
    assert_eq!(
        out.lines().collect::<Vec<_>>(),
        ["src/model.ts", "src/view.ts"]
    );
}

#[test]
fn formatting_and_comment_changes_select_nothing() {
    let d = repo(&[]);
    let r = d.path();
    write(
        r,
        "src/model.ts",
        "// The role of a user.\nimport { DEFAULT_ROLE, User } from './types';\n\nexport function \
         role(u: User) {\n    return u.id\n        ? DEFAULT_ROLE\n        : undefined;\n}\n",
    );
    assert_eq!(affected(r), NONE);
}

#[test]
fn runtime_changes_select() {
    // A changed value.
    let d = repo(&[]);
    let r = d.path();
    write(r, "src/types.ts", &TYPES.replace("'admin'", "'root'"));
    assert_eq!(affected(r), ["test/model.test.ts"]);
    let e = explain(r, "test/model.test.ts");
    assert!(
        e.contains("modified src/types.ts → src/model.ts → test/model.test.ts"),
        "{e}"
    );
    let v = json(r, &[]);
    assert_eq!(v["pruning"]["erased"], serde_json::json!([]));
    assert_eq!(v["pruning"]["not_erased"][0]["path"], "src/types.ts");
    assert_eq!(
        v["pruning"]["not_erased"][0]["reason"],
        "runtime code differs"
    );

    // A new enum member is runtime code.
    let d = repo(&[]);
    let r = d.path();
    write(
        r,
        "src/types.ts",
        &TYPES.replace(
            "  Admin = 'admin',\n",
            "  Admin = 'admin',\n  User = 'user',\n",
        ),
    );
    assert_eq!(affected(r), ["test/model.test.ts"]);

    // A new import is a new module load, even if only a type uses it.
    let d = repo(&[]);
    let r = d.path();
    write(
        r,
        "src/types.ts",
        &format!("import {{ other }} from './other';\nexport type O = typeof other;\n{TYPES}"),
    );
    assert_eq!(affected(r), ["test/model.test.ts"]);

    // A syntax error breaks the tests that load the file.
    let d = repo(&[]);
    let r = d.path();
    write(r, "src/types.ts", &format!("{TYPES}export type = ;\n"));
    assert_eq!(affected(r), ["test/model.test.ts"]);
}

#[test]
fn types_next_to_decorators_select() {
    let service = "import { Injectable } from './di';\nimport { User } from \
                   './types';\n@Injectable()\nexport class Service {\n  constructor(private user: \
                   User) {}\n}\n";
    let d = repo(&[
        (
            "src/di.ts",
            "export function Injectable() {\n  return (t: unknown) => t;\n}\n",
        ),
        ("src/service.ts", service),
        (
            "test/service.test.ts",
            "import { Service } from '../src/service';\ntest('s', () => \
             expect(Service).toBeDefined());\n",
        ),
    ]);
    let r = d.path();
    // emitDecoratorMetadata would emit the parameter's type.
    write(
        r,
        "src/service.ts",
        &service.replace("private user: User", "private user: object"),
    );
    assert_eq!(affected(r), ["test/service.test.ts"]);
    // A comment is still not a change.
    write(r, "src/service.ts", &format!("// The service.\n{service}"));
    assert_eq!(affected(r), NONE);
}

#[test]
fn a_modified_test_file_always_runs() {
    let d = repo(&[]);
    let r = d.path();
    write(
        r,
        "test/other.test.ts",
        "// A comment.\nimport { other } from '../src/other';\ntest('o', () => \
         expect(other).toBe(1));\n",
    );
    assert_eq!(affected(r), ["test/other.test.ts"]);
}

#[test]
fn tests_that_read_the_file_as_text_select() {
    let d = repo(&[(
        "test/read.test.ts",
        "import fs from 'fs';\nimport path from 'path';\ntest('r', () => {\n  \
         expect(fs.readFileSync(path.join(__dirname, '../src/types.ts'), \
         'utf8')).toContain('User');\n});\n",
    )]);
    let r = d.path();
    write(r, "src/types.ts", &format!("// A comment.\n{TYPES}"));
    assert_eq!(affected(r), ["test/read.test.ts"]);
}

#[test]
fn type_checking_runners_select() {
    let d = repo(&[
        (
            "node_modules/ts-jest/package.json",
            r#"{"name":"ts-jest","version":"29.0.0","main":"index.js"}"#,
        ),
        ("node_modules/ts-jest/index.js", "module.exports = {};\n"),
        (
            "jest.config.js",
            "module.exports = {\n  testMatch: ['**/test/**/*.test.ts'],\n  transform: { \
             '^.+\\\\.ts$': 'ts-jest' },\n};\n",
        ),
    ]);
    let r = d.path();
    // A type-only change: ts-jest type-checks, and follows `import type`.
    write(
        r,
        "src/types.ts",
        &TYPES.replace("  id: string;\n", "  id: string;\n  name?: string;\n"),
    );
    assert_eq!(affected(r), ["test/model.test.ts", "test/view.test.ts"]);
}

#[test]
fn import_type_edges_are_not_followed_for_transpiled_tests() {
    let d = repo(&[]);
    let r = d.path();
    write(r, "src/types.ts", &TYPES.replace("'admin'", "'root'"));
    // test/view.test.ts reaches types.ts only through `import type`.
    assert_eq!(affected(r), ["test/model.test.ts"]);
    let e = explain(r, "test/view.test.ts");
    assert!(
        e.contains("unaffected test/view.test.ts")
            && e.contains(
                "src/types.ts not followed: src/view.ts imports src/types.ts only for types, \
                 which the test runner does not load"
            ),
        "{e}"
    );
    // The off switch, and the mode in JSON.
    assert_eq!(
        affected_env(r, &[("TURBO_TRACE_DEBUG_TYPE_EDGES", "0")]),
        ["test/model.test.ts", "test/view.test.ts"]
    );
    let v = json(r, &["src/view.ts"]);
    assert_eq!(v["pruning"]["runtime_only_entries"], 3);
    assert_eq!(
        v["pruning"]["type_only_skipped"],
        serde_json::json!(["test/view.test.ts"])
    );
    let edges: Vec<(String, String)> = v["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| {
            (
                x["entry"].as_str().unwrap().to_owned(),
                x["edges"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    // The build root follows the type-only edge; the test does not.
    assert_eq!(
        edges,
        [
            ("src/view.ts".to_owned(), "all".to_owned()),
            ("test/model.test.ts".to_owned(), "runtime".to_owned()),
        ]
    );
    // Without a base revision the rule applies as well.
    let out = ok_env(
        r,
        &["affected", "--tests", "jest", "--changed", "src/types.ts"],
        &[],
    );
    assert_eq!(out.lines().collect::<Vec<_>>(), ["test/model.test.ts"]);
}
