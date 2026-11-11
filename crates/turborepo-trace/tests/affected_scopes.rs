//! Covered scopes of truncated globs (`--max-glob`), end to end: which changes
//! select a test whose reads were cut off at the limit.
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
        .expect("run the harness");
    assert!(
        o.status.success(),
        "harness {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout).unwrap()
}

fn affected(root: &Path, changed: &[&str]) -> Vec<String> {
    let mut args = vec!["affected", "--tests", "jest", "--no-refresh"];
    for c in changed {
        args.push("--changed");
        args.push(c);
    }
    ok(root, &args).lines().map(str::to_owned).collect()
}

fn explain(root: &Path, changed: &str, entry: &str) -> String {
    ok(
        root,
        &[
            "affected",
            "--no-refresh",
            "--changed",
            changed,
            "--explain",
            entry,
        ],
    )
}

/// `load(dir, names)` reads `<dir>/<name><suffix>` for names it cannot know,
/// once for a small fixture and once for the repository root: the shape of a
/// test that runs a detector over a fixture and over the repository itself.
fn detector_test(suffix: &str) -> String {
    format!(
        "const fs = require('fs');\nconst path = require('path');\nconst {{ listed }} = \
         require('./listed');\nconst root = path.join(__dirname, '..', '..');\nconst fixture = \
         path.join(__dirname, 'fixtures', 'mono');\nfunction load(dir, names) {{\n\x20 return \
         names.map((n) => fs.readFileSync(path.join(dir, n{suffix}), \
         'utf8'));\n}}\ntest('detect', () => {{\n\x20 load(fixture, listed(fixture));\n\x20 \
         load(root, listed(root));\n}});\n"
    )
}

fn detector_repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(r, "jest.config.js", "module.exports = {};\n");
    write(r, "pkg/test/fixtures/mono/a.json", "{}\n");
    write(r, "pkg/test/fixtures/mono/app/package.json", "{}\n");
    for p in ["aa", "bb", "cc", "zz"] {
        write(r, &format!("{p}/package.json"), "{}\n");
    }
    write(r, "zz/late.txt", "late\n");
    write(
        r,
        "pkg/test/listed.js",
        "exports.listed = (dir) => JSON.parse(process.env['NAMES_' + dir.length] || '[]');\n",
    );
    // `<dir>/<name>`: for the root this is `<ancestor>/*`, which is not expanded.
    write(r, "pkg/test/detect.test.js", &detector_test(""));
    // `<dir>/<name>/package.json`: expanded, and for the root truncated at the
    // limit.
    write(
        r,
        "pkg/test/manifests.test.js",
        &detector_test(", 'package.json'"),
    );
    write(r, "pkg/src/plain.js", "module.exports = 1;\n");
    write(
        r,
        "pkg/test/plain.test.js",
        "require('../src/plain');\ntest('plain', () => {});\n",
    );
    d
}

/// A reference with alternatives keeps the record of the branch that says the
/// most. In both tests the first branch (the fixture) expands completely. In
/// `manifests.test.js` the second branch is truncated: its scope must survive,
/// or the files beyond the limit are neither edges nor covered. In
/// `detect.test.js` the second branch is not expanded at all: the reference is
/// open.
#[test]
fn a_later_alternative_keeps_its_record() {
    let d = detector_repo();
    let r = d.path();
    ok(r, &["graph", "build", "--max-glob", "2"]);
    let open = "pkg/test/detect.test.js";
    let covered = "pkg/test/manifests.test.js";

    // The fixture branch: ordinary edges.
    assert_eq!(
        affected(r, &["pkg/test/fixtures/mono/app/package.json"]),
        [open, covered]
    );
    // A manifest the root branch matches beyond the limit: no edge, only the scope.
    assert_eq!(affected(r, &["zz/package.json"]), [open, covered]);
    let ex = explain(r, "zz/package.json", covered);
    assert!(
        ex.contains(
            "zz/package.json (inside the scope {*/package.json,*/**/*/package.json} of a covered \
             uncertainty) at pkg/test/manifests.test.js"
        ),
        "{ex}"
    );
    // The scope is the pattern, not the directory: another file below the root is
    // outside.
    assert_eq!(affected(r, &["zz/late.txt"]), [open]);
    assert!(
        ex.contains("covered pkg/test/manifests.test.js:7 pattern_too_broad"),
        "{ex}"
    );
    // The branch that was not expanded keeps its record: the test is selected for a
    // file anywhere below the root (as uncertain, or through the names that
    // bound the read).
    let ex = explain(r, "zz/late.txt", open);
    assert!(
        ex.starts_with("affected pkg/test/detect.test.js: ") && ex.contains("pattern_too_broad"),
        "{ex}"
    );
    // A test without such reads is selected by its import only.
    assert!(
        !affected(r, &["zz/late.txt"]).contains(&"pkg/test/plain.test.js".to_owned()),
        "plain"
    );
    assert!(affected(r, &["pkg/src/plain.js"]).contains(&"pkg/test/plain.test.js".to_owned()));
    let json: serde_json::Value = serde_json::from_str(&ok(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--no-refresh",
            "--changed",
            "zz/late.txt",
            "--format",
            "json",
        ],
    ))
    .unwrap();
    let count = |k: &str| json["uncertainty"][k].as_u64().unwrap();
    assert_eq!(count("open") + count("covered"), 2, "{json}");
    assert!(count("covered") >= 1, "{json}");
}

/// `affected` for one `git diff --name-status` line (`--changed` cannot say
/// "modified" for a file the graph has no node for).
fn affected_status(root: &Path, status: &str, path: &str) -> Vec<String> {
    let list = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(list.path(), format!("{status}\t{path}\n")).unwrap();
    let out = ok(
        root,
        &[
            "affected",
            "--tests",
            "jest",
            "--no-refresh",
            "--changed-from",
            list.path().to_str().unwrap(),
        ],
    );
    out.lines().map(str::to_owned).collect()
}

/// A test body over every file `P` of `data/`. The path is joined inside each
/// call: a path bound to a variable is also an asset reference, which stands
/// for a read of the contents.
fn data_test(name: &str, body: &str) -> String {
    let body = body.replace('P', "path.join(__dirname, '..', 'data', f)");
    format!(
        "const fs = require('fs');\nconst path = require('path');\ntest('{name}', () => {{\n\x20 \
         for (const f of fs.readdirSync(path.join(__dirname, '..', 'data'))) {{\n{body}\x20 \
         }}\n}});\n"
    )
}

/// A listing, an existence check and a `stat` asked only for the kind of the
/// entry observe names, not contents: past `--max-glob` they need no scope,
/// because no edit can change what they see and the directory membership covers
/// additions and deletions. A `stat` read for its size and a read of the
/// contents keep their scope.
#[test]
fn names_only_references_ignore_edits_past_the_limit() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(r, "jest.config.js", "module.exports = {};\n");
    for f in ["a", "b", "c", "d"] {
        write(r, &format!("data/{f}.txt"), f);
    }
    write(r, "other/x.txt", "x");
    write(
        r,
        "test/kinds.test.js",
        &data_test(
            "kinds",
            "    const st = fs.statSync(P);\n    if (st.isDirectory()) continue;\n    \
             expect(fs.existsSync(P)).toBe(true);\n",
        ),
    );
    write(
        r,
        "test/sizes.test.js",
        &data_test(
            "sizes",
            "    const st = fs.statSync(P);\n    expect(st.size).toBeLessThan(100);\n",
        ),
    );
    write(
        r,
        "test/contents.test.js",
        &data_test(
            "contents",
            "    expect(fs.readFileSync(P, 'utf8')).toBeTruthy();\n",
        ),
    );
    ok(r, &["graph", "build", "--max-glob", "2"]);
    let all = [
        "test/contents.test.js",
        "test/kinds.test.js",
        "test/sizes.test.js",
    ];
    let reads = ["test/contents.test.js", "test/sizes.test.js"];

    // An edit past the limit: the size and the contents can change, the names
    // can't.
    assert_eq!(affected_status(r, "M", "data/d.txt"), reads, "edit");
    // A new or a removed file changes the listing for everyone.
    write(r, "data/e.txt", "e");
    assert_eq!(affected_status(r, "A", "data/e.txt"), all, "add");
    assert_eq!(affected(r, &["data/e.txt"]), all, "--changed of a new file");
    assert_eq!(affected_status(r, "D", "data/d.txt"), all, "delete");
    // Elsewhere: nothing.
    assert!(affected_status(r, "M", "other/x.txt").is_empty());
    assert!(affected_status(r, "A", "other/y.txt").is_empty());

    let ex = explain(r, "other/x.txt", "test/kinds.test.js");
    assert!(ex.starts_with("unaffected test/kinds.test.js"), "{ex}");
    assert!(
        !ex.contains("pattern_too_broad"),
        "no scope is needed: {ex}"
    );
    let ex = explain(r, "other/x.txt", "test/sizes.test.js");
    assert!(
        ex.contains("covered test/sizes.test.js:5 pattern_too_broad (scope data/**)"),
        "{ex}"
    );
    let json: serde_json::Value = serde_json::from_str(&ok(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--no-refresh",
            "--changed",
            "other/x.txt",
            "--format",
            "json",
        ],
    ))
    .unwrap();
    assert_eq!(json["uncertainty"]["open"], 0, "{json}");
    assert_eq!(json["uncertainty"]["covered"], 2, "{json}");
}
