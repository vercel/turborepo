//! `turbotrace affected`: repo steering files (`.github/**`, `CODEOWNERS`,
//! `.vercel.approvers`, ...) can't change what an application does. A changed
//! one affects only the entries that reference it explicitly; it does not flow
//! through whole-directory task inputs, task nodes or the uncertainty floor of
//! application code.
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

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(support::harness())
        .args(args)
        .current_dir(root)
        .output()
        .expect("run turbotrace")
}

fn ok(root: &Path, args: &[&str]) -> String {
    let o = run(root, args);
    assert!(
        o.status.success(),
        "turbotrace {args:?} failed: {}",
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

const APP_TEST: &str = "packages/a/test/a.test.js";
const DYN_TEST: &str = "packages/a/test/dyn.test.js";
const X_TEST: &str = ".github/scripts/x.test.mjs";
const READ_TEST: &str = ".github/scripts/read.test.js";
const TOOL_DYN_TEST: &str = ".github/scripts/dyn.test.js";
const ENTRIES: [&str; 5] = [APP_TEST, DYN_TEST, X_TEST, READ_TEST, TOOL_DYN_TEST];

/// A workspace with one package whose `build` script is a command turbotrace
/// doesn't know (so the task's input is the whole package directory, approvers
/// file included) and whose output `dist/**` a test loads; an application test
/// behind an open uncertainty; and CI tooling below `.github/` with its own
/// tests. Everything is committed.
fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"root","private":true}"#);
    write(r, "pnpm-workspace.yaml", "packages:\n  - 'packages/*'\n");
    write(r, ".gitignore", ".turbo/trace/\ndist/\n");
    write(
        r,
        "turbo.json",
        r#"{"tasks":{"build":{"outputs":["dist/**"]}}}"#,
    );
    write(r, "CODEOWNERS", "* @acme/core\n");
    write(
        r,
        "packages/a/package.json",
        r#"{"name":"@acme/a","version":"1.0.0","scripts":{"build":"acme-bundle --out dist"}}"#,
    );
    write(r, "packages/a/.vercel.approvers", "@acme/core\n");
    write(r, "packages/a/OWNERS", "core\n");
    write(r, "packages/a/src/index.js", "module.exports = { a: 1 };\n");
    write(
        r,
        "packages/a/src/dyn.js",
        "module.exports = (name) => require(name);\n",
    );
    write(
        r,
        APP_TEST,
        "const a = require('../dist/index.js');\ntest('a', () => expect(a.a).toBe(1));\n",
    );
    write(
        r,
        DYN_TEST,
        "const load = require('../src/dyn');\ntest('d', () => expect(load('fs')).toBeTruthy());\n",
    );
    write(
        r,
        ".github/workflows/ci.yml",
        "name: ci\non: [push]\njobs:\n  test:\n    runs-on: ubuntu-latest\n    steps:\n      - \
         run: node .github/scripts/x.mjs\n",
    );
    write(r, ".github/scripts/x.mjs", "export const x = 1;\n");
    write(
        r,
        X_TEST,
        "import { x } from './x.mjs';\nimport test from 'node:test';\ntest('x', () => { if (x !== \
         1) throw new Error(); });\n",
    );
    write(
        r,
        READ_TEST,
        "const fs = require('fs');\nconst path = require('path');\nconst wf = \
         fs.readFileSync(path.join(__dirname, '../workflows/ci.yml'), \
         'utf8');\nrequire('node:test')('wf', () => { if (!wf.includes('jobs')) throw new \
         Error(); });\n",
    );
    write(
        r,
        ".github/scripts/load.js",
        "module.exports = (name) => require(name);\n",
    );
    write(
        r,
        TOOL_DYN_TEST,
        "const load = require('./load');\nrequire('node:test')('l', () => load('fs'));\n",
    );
    git(r, &["init", "-q"]);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "base"]);
    d
}

/// The affected entries among [`ENTRIES`] for the working tree against `HEAD`.
fn since(r: &Path, extra: &[&str]) -> Vec<String> {
    let mut args = vec!["affected", "--since", "HEAD"];
    args.extend(ENTRIES);
    args.extend(extra);
    ok(r, &args).lines().map(str::to_owned).collect()
}

fn reset(r: &Path) {
    git(r, &["checkout", "-q", "--", "."]);
    git(r, &["clean", "-q", "-fd", "-e", ".turbo"]);
}

#[test]
fn approvers_edit_selects_nothing() {
    let d = repo();
    let r = d.path();
    // The fixture works as described: a source edit reaches the test through the
    // task.
    write(r, "packages/a/src/index.js", "module.exports = { a: 2 };\n");
    assert_eq!(since(r, &["--on-uncertain", "ignore"]), [APP_TEST]);
    reset(r);

    write(r, "packages/a/.vercel.approvers", "@acme/core\n@acme/web\n");
    // Without the treatment, the approvers file is a member of the task's
    // whole-package input, and the open uncertainty selects more.
    assert_eq!(
        since(r, &["--no-steering", "--on-uncertain", "ignore"]),
        [APP_TEST]
    );
    assert_eq!(
        since(r, &["--no-steering"]),
        [TOOL_DYN_TEST, APP_TEST, DYN_TEST]
    );
    // With it: nothing, under either policy (only a steering file changed, so the
    // uncertainty of application code does not count; the tooling test's does).
    assert!(since(r, &["--on-uncertain", "ignore"]).is_empty());
    assert_eq!(since(r, &[]), [TOOL_DYN_TEST]);
    // The same without a base revision.
    let mut args = vec![
        "affected",
        "--changed",
        "packages/a/.vercel.approvers",
        "--on-uncertain",
        "ignore",
    ];
    args.extend(ENTRIES);
    assert_eq!(ok(r, &args), "");
    // The task itself is not affected either.
    let tasks = ok(
        r,
        &[
            "affected",
            "--since",
            "HEAD",
            "--tasks",
            "**",
            "--on-uncertain",
            "ignore",
        ],
    );
    assert_eq!(tasks, "", "{tasks}");
    let tasks = ok(
        r,
        &[
            "affected",
            "--since",
            "HEAD",
            "--tasks",
            "**",
            "--no-steering",
            "--on-uncertain",
            "ignore",
        ],
    );
    assert!(tasks.contains("task:packages/a#build"), "{tasks}");
}

#[test]
fn added_and_deleted_approvers_select_nothing() {
    let d = repo();
    let r = d.path();
    write(r, "packages/a/src/.vercel.approvers", "@acme/web\n");
    assert_eq!(
        since(r, &["--no-steering", "--on-uncertain", "ignore"]),
        [APP_TEST]
    );
    assert!(since(r, &["--on-uncertain", "ignore"]).is_empty());
    reset(r);
    std::fs::remove_file(r.join("packages/a/.vercel.approvers")).unwrap();
    assert_eq!(
        since(r, &["--no-steering", "--on-uncertain", "ignore"]),
        [APP_TEST]
    );
    assert!(since(r, &["--on-uncertain", "ignore"]).is_empty());
    // A mixed change: the other file counts as always, and so does the uncertainty
    // floor.
    write(r, "packages/a/src/index.js", "module.exports = { a: 3 };\n");
    assert_eq!(since(r, &["--on-uncertain", "ignore"]), [APP_TEST]);
    assert_eq!(since(r, &[]), [TOOL_DYN_TEST, APP_TEST, DYN_TEST]);
}

#[test]
fn workflow_and_tooling_edits_select_their_explicit_readers() {
    let d = repo();
    let r = d.path();
    // A workflow edit: only the test that reads the file by a resolved path.
    write(
        r,
        ".github/workflows/ci.yml",
        "name: ci\non: [push, pull_request]\njobs:\n  test:\n    runs-on: ubuntu-latest\n    \
         steps:\n      - run: node .github/scripts/x.mjs\n",
    );
    assert_eq!(since(r, &["--on-uncertain", "ignore"]), [READ_TEST]);
    // Only the trigger changed: no job's definition did, so no task reports
    // (crates/turborepo-trace/src/workflows.rs; tests/affected_workflows.rs covers
    // the job side).
    let tasks = ok(
        r,
        &[
            "affected",
            "--since",
            "HEAD",
            "--tasks",
            "**",
            "--on-uncertain",
            "ignore",
        ],
    );
    assert_eq!(tasks, "", "a trigger edit changes no job");
    let explain = ok(
        r,
        &[
            "affected",
            "--since",
            "HEAD",
            "--on-uncertain",
            "ignore",
            "--explain",
            READ_TEST,
            "--explain",
            APP_TEST,
        ],
    );
    assert!(
        explain.contains(&format!(
            "affected {READ_TEST}: modified .github/workflows/ci.yml → {READ_TEST}"
        )),
        "{explain}"
    );
    assert!(
        explain.contains(&format!("unaffected {APP_TEST}")),
        "{explain}"
    );
    assert!(
        explain.contains("1 changed repo steering file(s) (.github/workflows/ci.yml)"),
        "{explain}"
    );
    reset(r);

    // A tooling script edit: its test, through the import.
    write(r, ".github/scripts/x.mjs", "export const x = 2;\n");
    assert_eq!(since(r, &["--on-uncertain", "ignore"]), [X_TEST]);
    // A tooling test is an entry like any other.
    write(r, READ_TEST, "require('node:test')('wf', () => {});\n");
    assert_eq!(since(r, &["--on-uncertain", "ignore"]), [READ_TEST, X_TEST]);
    reset(r);

    // A deleted tooling script: the importer that now fails to resolve it.
    std::fs::remove_file(r.join(".github/scripts/x.mjs")).unwrap();
    assert_eq!(since(r, &["--on-uncertain", "ignore"]), [X_TEST]);
}

#[test]
fn steering_globs_and_reporting() {
    let d = repo();
    let r = d.path();
    // Not a steering file by default: a member of the package like any other.
    write(r, "packages/a/OWNERS", "core\nweb\n");
    assert_eq!(since(r, &["--on-uncertain", "ignore"]), [APP_TEST]);
    assert!(since(r, &["--on-uncertain", "ignore", "--steering", "**/OWNERS"]).is_empty());
    reset(r);

    write(r, "CODEOWNERS", "* @acme/web\n");
    write(r, "packages/a/src/index.js", "module.exports = { a: 4 };\n");
    let mut args = vec![
        "affected",
        "--since",
        "HEAD",
        "--format",
        "json",
        "--on-uncertain",
        "ignore",
    ];
    args.extend(ENTRIES);
    let v: serde_json::Value = serde_json::from_str(&ok(r, &args)).unwrap();
    assert_eq!(v["steering"]["changes"][0]["path"], "CODEOWNERS");
    assert_eq!(v["steering"]["only"], false);
    assert_eq!(v["affected"], 1);
    let paths: Vec<&str> = v["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["packages/a/src/index.js"]);
    args.push("--stats");
    let o = run(r, &args);
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.contains("steering: 1 of 2 change(s) are repo steering files"),
        "{err}"
    );
    assert!(
        run(
            r,
            &[
                "affected",
                "--since",
                "HEAD",
                APP_TEST,
                "--steering",
                "a/{b"
            ]
        )
        .status
        .code()
            != Some(0)
    );
}
