//! `turbotrace affected --audit` end to end on a small synthetic workspace.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{path::Path, process::Command};

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn ok(root: &Path, args: &[&str]) -> String {
    let o = Command::new(support::harness())
        .args(args)
        .current_dir(root)
        .output()
        .expect("run turbotrace");
    assert!(
        o.status.success(),
        "turbotrace {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout).unwrap()
}

/// A workspace of three packages under one jest config:
///
/// - `@t/b` declares `@t/a`, `@t/c` doesn't; both import
///   `packages/a/src/index.js`;
/// - `packages/a` has a `build` task with an unknown command (so it depends on
///   the whole package directory) and a declared output `dist/**`, which
///   `@t/b`'s other test reads;
/// - `packages/c/test/dyn.test.js` has an open uncertainty.
fn workspace() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(
        r,
        "package.json",
        r#"{"name":"root","private":true,"workspaces":["packages/*"]}"#,
    );
    write(r, "jest.config.js", "module.exports = {};\n");
    write(
        r,
        "turbo.json",
        r#"{"tasks":{"build":{"outputs":["dist/**"]}}}"#,
    );
    write(
        r,
        "packages/a/package.json",
        r#"{"name":"@t/a","version":"1.0.0","scripts":{"build":"frobnicate --all"}}"#,
    );
    write(r, "packages/a/src/index.js", "module.exports = 1;\n");
    write(r, "packages/a/.vercel.approvers", "team-a\n");
    write(
        r,
        "packages/b/package.json",
        r#"{"name":"@t/b","version":"1.0.0","dependencies":{"@t/a":"workspace:*"}}"#,
    );
    write(
        r,
        "packages/b/test/b.test.js",
        "require('../../a/src/index.js');\ntest('b', () => {});\n",
    );
    write(
        r,
        "packages/b/test/built.test.js",
        "require('../../a/dist/out.js');\ntest('built', () => {});\n",
    );
    write(
        r,
        "packages/c/package.json",
        r#"{"name":"@t/c","version":"1.0.0"}"#,
    );
    write(
        r,
        "packages/c/test/c.test.js",
        "require('../../a/src/index.js');\ntest('c', () => {});\n",
    );
    write(
        r,
        "packages/c/test/dyn.test.js",
        "const name = globalThis.pick();\nrequire(name);\ntest('dyn', () => {});\n",
    );
    d
}

fn audit(root: &Path, changed: &[&str]) -> serde_json::Value {
    audit_with(root, &[], changed)
}

fn audit_with(root: &Path, extra: &[&str], changed: &[&str]) -> serde_json::Value {
    let mut args = vec!["affected", "--tests", "jest", "--audit", "--format", "json"];
    args.extend(extra);
    args.push("--changed");
    args.extend(changed);
    serde_json::from_str(&ok(root, &args)).unwrap()
}

fn entry<'a>(v: &'a serde_json::Value, path: &str) -> &'a serde_json::Value {
    v["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["entry"] == path)
        .unwrap_or_else(|| panic!("no entry {path} in {v:#}"))
}

fn signals(v: &serde_json::Value, path: &str) -> Vec<String> {
    entry(v, path)["signals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_owned())
        .collect()
}

fn source<'a>(v: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    v["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == id)
        .unwrap_or_else(|| panic!("no source {id} in {:#}", v["sources"]))
}

#[test]
fn undeclared_imports_the_floor_and_the_json_shape() {
    let d = workspace();
    let r = d.path();
    let v = audit(r, &["packages/a/src/index.js"]);
    assert_eq!(v["version"], 1);
    assert_eq!(v["entries_total"], 4, "{v:#}");
    assert_eq!(v["selected"], 4, "{v:#}");
    // @t/b declares @t/a: an ordinary, declared import.
    let b = entry(&v, "packages/b/test/b.test.js");
    assert_eq!(
        (&b["selected"], &b["plausible"]),
        (&true.into(), &true.into())
    );
    assert!(signals(&v, "packages/b/test/b.test.js").is_empty());
    // @t/c doesn't.
    assert_eq!(
        signals(&v, "packages/c/test/c.test.js"),
        ["non_import", "no_package_dep"]
    );
    let c = entry(&v, "packages/c/test/c.test.js");
    assert_eq!(c["bridge"]["kind"], "undeclared");
    assert_eq!(c["bridge"]["source"], "undeclared:packages/c -> packages/a");
    assert_eq!(c["a_priori"]["no_package_dep"], true);
    assert_eq!(c["ecosystem"], "js");
    let s = source(&v, "undeclared:packages/c -> packages/a");
    assert_eq!((&s["exclusive"], &s["inclusive"]), (&1.into(), &1.into()));
    // The test of the build output goes through the task; the test with a computed
    // require is selected by the floor alone.
    assert_eq!(signals(&v, "packages/b/test/built.test.js"), ["non_import"]);
    assert_eq!(source(&v, "task:packages/a#build")["exclusive"], 1);
    assert_eq!(
        signals(&v, "packages/c/test/dyn.test.js"),
        ["non_import", "floor", "no_package_dep"]
    );
    assert_eq!(
        (&v["floor_only"], &v["change_only"], &v["floor_and_change"]),
        (&1.into(), &3.into(), &0.into())
    );
    assert_eq!(v["implausible"], 3);

    // The readable form ranks the same sources.
    let text = ok(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--audit",
            "--no-refresh",
            "--changed",
            "packages/a/src/index.js",
        ],
    );
    assert!(text.starts_with("audit: 4 of 4 entries selected"), "{text}");
    assert!(
        text.contains("undeclared:packages/c -> packages/a"),
        "{text}"
    );
    assert!(text.contains("task:packages/a#build"), "{text}");
}

#[test]
fn a_steering_file_in_a_package_hub() {
    let d = workspace();
    let r = d.path();
    // Treated like any other file (`--no-steering`): the approvers file is part of
    // the whole-package membership of the build task, so the test of the build
    // output is selected; nothing refers to the file itself.
    let v = audit_with(r, &["--no-steering"], &["packages/a/.vercel.approvers"]);
    assert_eq!(
        signals(&v, "packages/b/test/built.test.js"),
        ["non_import", "steering"]
    );
    assert_eq!(v["steering"]["changes"], 1);
    assert_eq!(v["steering"]["ignored"], 0);
    assert_eq!(v["steering"]["classes"][".vercel.approvers"], 1);
    assert_eq!(v["steering"]["exclusive"], 1);
    let s = source(&v, "steering:.vercel.approvers");
    assert_eq!((&s["exclusive"], &s["inclusive"]), (&1.into(), &1.into()));
    // The other selected test is the floor's.
    assert_eq!(v["selected"], 2, "{v:#}");
    assert_eq!(v["floor_only"], 1);
    let b = entry(&v, "packages/b/test/b.test.js");
    assert_eq!(b["selected"], false);
    assert_eq!(b["a_priori"]["steering_only"], true);

    // By default a steering file counts only through explicit references: it
    // selects nothing here, and the audit reports it as ignored.
    let v = audit(r, &["packages/a/.vercel.approvers"]);
    assert_eq!(v["selected"], 0, "{v:#}");
    assert_eq!(v["steering"]["changes"], 1);
    assert_eq!(v["steering"]["ignored"], 1);
    assert_eq!(v["steering"]["inclusive"], 0);
    let b = entry(&v, "packages/b/test/built.test.js");
    assert_eq!(b["selected"], false);
    assert_eq!(b["a_priori"]["steering_only"], true);
}
