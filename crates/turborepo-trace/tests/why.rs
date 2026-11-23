//! `turbo trace why` end to end (the harness's `affected --why`,
//! `--why-summary`): every test a change selects is explained with the reason
//! the selection gives, link by link, with the other witnesses that select it
//! too.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{path::Path, process::Command};

use serde_json::Value;

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// The harness's exit code and output.
fn run(root: &Path, args: &[&str]) -> (i32, String) {
    let o = Command::new(support::harness())
        .args(args)
        .current_dir(root)
        .output()
        .expect("run the harness");
    let code = o.status.code().unwrap();
    assert!(
        code == 0 || code == 1,
        "harness {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    (code, String::from_utf8(o.stdout).unwrap())
}

fn args<'a>(first: &[&'a str], change: &[&'a str], last: &[&'a str]) -> Vec<&'a str> {
    [first, &["--tests", "jest"], change, last].concat()
}

/// What the selection lists for `change`: each test with the `__typename` of
/// its reason in `turbo trace affected --json`.
fn selected(root: &Path, change: &[&str]) -> Vec<(String, &'static str)> {
    let (_, out) = run(
        root,
        &args(&["affected", "--format", "selection"], change, &[]),
    );
    let doc: Value = serde_json::from_str(&out).unwrap();
    let mut items = Vec::new();
    for p in doc["packages"].as_array().unwrap() {
        for s in p["selected"].as_array().unwrap() {
            let kind = match s["reason"].as_str().unwrap() {
                "test-changed" => "TestChanged",
                "change" => "FileChanged",
                "dependency" => "PackageDependencyChanged",
                "name" => "NameReferenced",
                "task" => "TaskChanged",
                "not-in-graph" => "NotInGraph",
                _ => "Uncertain",
            };
            items.push((s["path"].as_str().unwrap().to_owned(), kind));
        }
        for u in p["unknown"].as_array().unwrap() {
            items.push((u.as_str().unwrap().to_owned(), "UnknownTest"));
        }
    }
    items.sort();
    items
}

/// `why <test>` for `change`, as JSON.
fn why(root: &Path, change: &[&str], test: &str) -> Value {
    let (code, out) = run(
        root,
        &args(&["affected"], change, &["--why", test, "--why-json"]),
    );
    let v: Value = serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}: {out}"));
    assert_eq!(code == 0, v["selected"].as_bool().unwrap(), "{out}");
    v
}

fn why_text(root: &Path, change: &[&str], test: &str) -> String {
    run(root, &args(&["affected"], change, &["--why", test])).1
}

fn links(v: &Value) -> Vec<&str> {
    v["witnesses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["link"].as_str().unwrap())
        .collect()
}

/// A pnpm workspace with the constructs that select broadly.
///
/// ```text
/// packages/framework/normalize.js   import(spec), spec a parameter       declares util
/// packages/harness/setup.js         boots the framework                  declares framework
/// packages/util/other.js            nothing imports it
/// packages/util/list.js             nothing imports it; it lists docs/
/// packages/reader/index.js          readText(p): fs.readFileSync(p)      p is a parameter
/// services/a/test/a.test.js         boots the framework; imports ../src/a.js
/// services/b/test/b.test.js         boots the framework; readText(join(ROOT, name))  
/// services/n/test/token.test.js     readText(base + '/secrets/' + name)  a name
/// services/r/test/root.test.js      readText(join(ROOT, name))           ROOT: three levels up
/// services/p/test/plain.test.js     imports ../src/plain.js
/// ```
fn workspace() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(
        r,
        "pnpm-workspace.yaml",
        "packages:\n  - 'services/*'\n  - 'packages/*'\n",
    );
    write(r, "docs/guide.md", "guide\n");
    let pkg = |dir: &str, deps: &[&str]| {
        let name = dir.rsplit('/').next().unwrap();
        let list: Vec<String> = deps
            .iter()
            .map(|d| format!(r#""@x/{d}":"workspace:*""#))
            .collect();
        write(
            r,
            &format!("{dir}/package.json"),
            &format!(
                r#"{{"name":"@x/{name}","version":"1.0.0","dependencies":{{{}}}}}"#,
                list.join(",")
            ),
        );
        for dep in deps {
            // One link per declared dependency, as pnpm installs them.
            let link = r.join(dir).join("node_modules/@x").join(dep);
            std::fs::create_dir_all(link.parent().unwrap()).unwrap();
            turborepo_trace_core::os::symlink(r.join("packages").join(dep), link).unwrap();
        }
        if dir.starts_with("services/") {
            write(
                r,
                &format!("{dir}/jest.config.js"),
                "module.exports = {};\n",
            );
        }
    };
    pkg("packages/framework", &["util"]);
    pkg("packages/util", &[]);
    pkg("packages/harness", &["framework"]);
    pkg("packages/reader", &[]);
    pkg("services/a", &["harness"]);
    pkg("services/b", &["harness", "reader"]);
    pkg("services/n", &["reader"]);
    pkg("services/r", &["reader"]);
    pkg("services/p", &[]);
    write(
        r,
        "packages/framework/normalize.js",
        "const { isString } = require('@x/util');\nexports.normalize = function normalize(spec) \
         {\n\x20 if (isString(spec)) {\n\x20   return [{ handler: () => import(spec), modulePath: \
         spec }];\n\x20 }\n\x20 return [];\n};\n",
    );
    write(
        r,
        "packages/util/index.js",
        "exports.isString = (x) => typeof x === 'string';\n",
    );
    write(r, "packages/util/other.js", "module.exports = 1;\n");
    write(
        r,
        "packages/util/list.js",
        "const fs = require('fs');\nconst path = require('path');\nexports.docs = () => \
         fs.readdirSync(path.join(__dirname, '..', '..', 'docs'));\n",
    );
    write(
        r,
        "packages/harness/setup.js",
        "const { normalize } = require('@x/framework/normalize.js');\nexports.boot = (routes) => \
         routes.flatMap((r) => normalize(r));\n",
    );
    write(
        r,
        "packages/reader/index.js",
        "const fs = require('fs');\nexports.readText = function (p) {\n  return \
         fs.readFileSync(p, 'utf8');\n};\n",
    );
    write(r, "services/a/src/a.js", "module.exports = 1;\n");
    write(
        r,
        "services/a/test/a.test.js",
        "const { boot } = require('@x/harness/setup.js');\nrequire('../src/a.js');\ntest('t', () \
         => { boot(globalThis.routes); });\n",
    );
    write(
        r,
        "services/b/test/b.test.js",
        "const path = require('path');\nconst { boot } = require('@x/harness/setup.js');\nconst { \
         readText } = require('@x/reader');\nconst ROOT = path.join(__dirname, '..', '..', \
         '..');\nconst top = (name) => readText(path.join(ROOT, name));\ntest('t', () => {\n  \
         boot(globalThis.routes);\n  top(globalThis.name);\n});\n",
    );
    write(
        r,
        "services/n/test/paths.js",
        "const base = process.env.SERVICE_ROOT;\nexports.tokenPath = function (name) {\n  return \
         base + '/secrets/' + name + '.token';\n};\n",
    );
    write(
        r,
        "services/n/test/token.test.js",
        "const { readText } = require('@x/reader');\nconst { tokenPath } = \
         require('./paths');\ntest('token', () => { readText(tokenPath('dev')); });\n",
    );
    write(r, "services/n/secrets/dev.token", "t\n");
    write(
        r,
        "services/r/test/root.test.js",
        "const path = require('path');\nconst { readText } = require('@x/reader');\nconst ROOT = \
         path.join(__dirname, '..', '..', '..');\nconst top = (name) => readText(path.join(ROOT, \
         name));\ntest('root', () => { top(globalThis.name); });\n",
    );
    write(r, "services/p/src/plain.js", "module.exports = 1;\n");
    write(
        r,
        "services/p/test/plain.test.js",
        "require('../src/plain.js');\ntest('plain', () => {});\n",
    );
    d
}

const A: &str = "services/a/test/a.test.js";
const B: &str = "services/b/test/b.test.js";
const TOKEN: &str = "services/n/test/token.test.js";
const ROOT: &str = "services/r/test/root.test.js";
const PLAIN: &str = "services/p/test/plain.test.js";

/// Workspaces without a per-package `node_modules` (a load is not bounded by
/// what a package declares), a build task whose source has a computed load,
/// and a task with a command the trace does not know.
///
/// ```text
/// lib/src/index.ts     require(name)            built by lib#build into lib/dist
/// app/lib.test.js      requires ../lib/dist/index.js
/// app/plain.test.js    requires ./plain.js
/// app/odd.spec.js      looks like a test; the jest config's pattern leaves it out
/// ```
fn tasks() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(
        r,
        "package.json",
        r#"{"name":"root","private":true,"workspaces":["lib","app"]}"#,
    );
    write(
        r,
        "turbo.json",
        r#"{"tasks":{"build":{"outputs":["dist/**"]},"test":{"dependsOn":["^build"]}}}"#,
    );
    write(
        r,
        "lib/package.json",
        r#"{"name":"lib","version":"1.0.0","main":"dist/index.js","scripts":{"build":"tsc -p tsconfig.json"}}"#,
    );
    write(
        r,
        "lib/tsconfig.json",
        r#"{"compilerOptions":{"outDir":"dist","rootDir":"src"},"include":["src"]}"#,
    );
    write(
        r,
        "lib/src/index.ts",
        "export const load = (name: string) => require(name);\n",
    );
    write(r, "lib/src/other.ts", "export const one = 1;\n");
    write(
        r,
        "app/package.json",
        r#"{"name":"app","version":"1.0.0","scripts":{"test":"jest"}}"#,
    );
    write(
        r,
        "app/jest.config.js",
        "module.exports = { testMatch: ['**/*.test.js'] };\n",
    );
    write(
        r,
        "app/lib.test.js",
        "const { load } = require('../lib/dist/index.js');\ntest('l', () => \
         expect(load).toBeDefined());\n",
    );
    write(r, "app/plain.js", "module.exports = 1;\n");
    write(
        r,
        "app/plain.test.js",
        "require('./plain.js');\ntest('p', () => {});\n",
    );
    write(r, "app/odd.spec.js", "test('odd', () => {});\n");
    write(r, "README.md", "x\n");
    d
}

const LIB: &str = "app/lib.test.js";
const APP_PLAIN: &str = "app/plain.test.js";

/// Every test that `affected` selects is explained by `why`, with the same
/// kind of reason, a first witness and (unless the test is listed without a
/// chain) the steps to the test.
#[test]
fn every_selected_test_is_explained_with_the_reason_of_the_selection() {
    let w = workspace();
    let t = tasks();
    let cases: &[(&Path, &[&[&str]])] = &[
        (
            w.path(),
            &[
                &["--changed", "docs/guide.md"],
                &["--changed", "packages/util/other.js"],
                &["--changed", "services/n/secrets/dev.token"],
                &["--changed", "services/a/src/a.js"],
                &["--changed", "services/a/test/a.test.js"],
                &["--changed", "services/a/package.json"],
                &["--changed", "packages/reader/index.js", "docs/guide.md"],
                &["--changed", "docs/guide.md", "--strict"],
            ],
        ),
        (
            t.path(),
            &[
                &["--changed", "README.md"],
                &["--changed", "lib/src/other.ts"],
                &["--changed", "lib/package.json"],
                &["--changed", "app/package.json"],
                &["--changed", "turbo.json"],
                &["--changed", "app/odd.spec.js"],
            ],
        ),
    ];
    let mut kinds = std::collections::BTreeSet::new();
    for (root, changes) in cases {
        for change in *changes {
            for (test, kind) in selected(root, change) {
                let v = why(root, change, &test);
                assert_eq!(v["selected"], true, "{change:?} {test}: {v}");
                assert_eq!(v["reason"]["__typename"], kind, "{change:?} {test}: {v}");
                let first = &v["witnesses"][0];
                if !matches!(kind, "UnknownTest" | "NotInGraph") {
                    assert_eq!(first["reason"], v["reason"], "{change:?} {test}: {v}");
                    let chain = first["chain"].as_array().unwrap();
                    assert_eq!(chain.last().unwrap()["file"], test.as_str(), "{v}");
                    assert!(!first["link"].as_str().unwrap().is_empty(), "{v}");
                }
                kinds.insert(kind);
            }
        }
    }
    // The fixtures show the kinds a broad selection is made of.
    for kind in [
        "FileChanged",
        "NameReferenced",
        "TestChanged",
        "Uncertain",
        "UnknownTest",
    ] {
        assert!(kinds.contains(kind), "no {kind} in {kinds:?}");
    }
}

#[test]
fn a_name_reference_is_explained_with_the_name_and_the_read() {
    let d = workspace();
    let r = d.path();
    let change = ["--changed", "services/n/secrets/dev.token"];
    let v = why(r, &change, TOKEN);
    assert_eq!(v["reason"]["__typename"], "NameReferenced", "{v}");
    assert_eq!(v["reason"]["filePath"], "services/n/secrets/dev.token");
    let w = &v["witnesses"][0];
    assert_eq!(w["link"], "name reference", "{v}");
    assert_eq!(w["changed"], "services/n/secrets/dev.token");
    // The name and the read it is loaded with, each with its line of source.
    assert_eq!(w["code"][0]["file"], "services/n/test/paths.js", "{v}");
    assert_eq!(w["code"][0]["line"], 3);
    assert!(
        w["code"][0]["text"]
            .as_str()
            .unwrap()
            .contains("'/secrets/'"),
        "{v}"
    );
    assert_eq!(w["code"][1]["file"], "packages/reader/index.js", "{v}");
    assert!(
        w["code"][1]["text"]
            .as_str()
            .unwrap()
            .contains("readFileSync(p"),
        "{v}"
    );
    // From the file with the name to the test, with the reference of each step.
    assert_eq!(w["chain"][0]["file"], "services/n/test/paths.js");
    assert_eq!(w["chain"][1]["file"], TOKEN);
    assert!(
        w["chain"][1]["via"]
            .as_str()
            .unwrap()
            .contains("\"./paths\""),
        "{v}"
    );
    let text = why_text(r, &change, TOKEN);
    assert!(
        text.starts_with(&format!(
            "{TOKEN}: selected by the change to services/n/secrets/dev.token (added) \
             (NameReferenced services/n/secrets/dev.token)\n"
        )),
        "{text}"
    );
    assert!(text.contains("  1. name reference: changed"), "{text}");
    assert!(text.contains("no other kind of link selects it"), "{text}");
}

#[test]
fn a_name_of_the_repository_root_says_that_it_is_broad() {
    let d = workspace();
    let r = d.path();
    let v = why(r, &["--changed", "docs/guide.md"], ROOT);
    assert_eq!(v["reason"]["__typename"], "NameReferenced", "{v}");
    let w = &v["witnesses"][0];
    assert_eq!(
        w["class"], "it names the repository root: every path of the repository is below it",
        "{v}"
    );
    assert_eq!(w["code"][0]["file"], ROOT, "{v}");
    assert!(
        w["code"][0]["text"]
            .as_str()
            .unwrap()
            .contains("path.join(ROOT, name)"),
        "{v}"
    );
}

#[test]
fn a_bounded_load_is_explained_with_its_bound() {
    let d = workspace();
    let r = d.path();
    let v = why(r, &["--changed", "packages/util/other.js"], A);
    assert_eq!(v["reason"]["__typename"], "Uncertain", "{v}");
    let w = &v["witnesses"][0];
    assert_eq!(w["link"], "bounded load", "{v}");
    assert_eq!(w["noChange"], false, "{v}");
    assert!(
        w["description"]
            .as_str()
            .unwrap()
            .contains("can reach packages/util/ (packages/util/other.js changed)"),
        "{v}"
    );
    assert!(
        w["class"]
            .as_str()
            .unwrap()
            .contains("bounded only by the packages"),
        "{v}"
    );
    // The load, with its line, and the way from it to the test.
    assert_eq!(w["code"][0]["file"], "packages/framework/normalize.js");
    assert!(
        w["code"][0]["text"]
            .as_str()
            .unwrap()
            .contains("import(spec)"),
        "{v}"
    );
    assert_eq!(w["chain"][0]["file"], "packages/framework/normalize.js");
    assert_eq!(w["chain"].as_array().unwrap().last().unwrap()["file"], A);
}

/// A load is bounded by the packages it can reach, and it selects when a file
/// of one of them depends on the change: `why` names that file, which is the
/// next thing to ask about.
#[test]
fn a_bound_names_the_file_to_ask_about_next() {
    let d = workspace();
    let r = d.path();
    let change = ["--changed", "docs/guide.md"];
    let v = why(r, &change, A);
    let w = &v["witnesses"][0];
    assert_eq!(w["link"], "bounded load", "{v}");
    assert_eq!(w["next"], "packages/util/list.js", "{v}");
    let text = why_text(r, &change, A);
    assert!(
        text.contains(
            "     next: turbo trace why packages/util/list.js  (why that file depends on the \
             change)\n"
        ),
        "{text}"
    );
    // That file is explained like a test: it lists the directory of the change.
    let v = why(r, &change, "packages/util/list.js");
    assert_eq!(v["selected"], true, "{v}");
    assert_eq!(v["reason"]["__typename"], "FileChanged", "{v}");
    let w = &v["witnesses"][0];
    assert_eq!(w["chain"][0]["file"], "docs/guide.md", "{v}");
    assert!(
        w["chain"][1]["via"]
            .as_str()
            .unwrap()
            .contains("at packages/util/list.js:3"),
        "{v}"
    );
}

#[test]
fn two_witnesses_are_both_listed() {
    let d = workspace();
    let r = d.path();
    // The change is in a package the framework's load can reach, and below the
    // repository root that the test's own read names.
    let change = ["--changed", "packages/util/other.js"];
    let v = why(r, &change, B);
    assert_eq!(v["witnesses"].as_array().unwrap().len(), 2, "{v}");
    let mut found = links(&v);
    found.sort_unstable();
    assert_eq!(found, ["bounded load", "name reference"], "{v}");
    // The first is the selection's own.
    let listed = selected(r, &change);
    let kind = listed.iter().find(|s| s.0 == B).unwrap().1;
    assert_eq!(v["witnesses"][0]["reason"]["__typename"], kind, "{v}");
    let text = why_text(r, &change, B);
    assert!(
        text.contains("selected through 2 independent witnesses"),
        "{text}"
    );
    assert!(text.contains("\n  2. "), "{text}");
    // The other test of the load has it alone.
    assert_eq!(links(&why(r, &change, A)), ["bounded load"]);
}

#[test]
fn an_import_and_the_test_itself() {
    let d = workspace();
    let r = d.path();
    let v = why(r, &["--changed", "services/a/src/a.js"], A);
    assert_eq!(v["reason"]["__typename"], "FileChanged", "{v}");
    let w = &v["witnesses"][0];
    assert_eq!(w["link"], "import", "{v}");
    assert_eq!(w["chain"][0]["file"], "services/a/src/a.js");
    assert!(
        w["chain"][1]["via"]
            .as_str()
            .unwrap()
            .starts_with("require \"../src/a.js\" at services/a/test/a.test.js:2"),
        "{v}"
    );
    let v = why(r, &["--changed", A], A);
    assert_eq!(v["reason"]["__typename"], "TestChanged", "{v}");
    assert_eq!(v["witnesses"][0]["link"], "the test itself", "{v}");
}

#[test]
fn an_unrelated_pair_is_not_selected() {
    let d = workspace();
    let r = d.path();
    let change = ["--changed", "services/a/src/a.js"];
    let v = why(r, &change, PLAIN);
    assert_eq!(v["selected"], false, "{v}");
    assert!(v["witnesses"].as_array().unwrap().is_empty(), "{v}");
    let (code, text) = run(r, &args(&["affected"], &change, &["--why", PLAIN]));
    assert_eq!(code, 1);
    assert_eq!(
        text,
        format!(
            "{PLAIN}: not selected by this change (the change to services/a/src/a.js (modified))\n"
        )
    );
}

#[test]
fn a_test_behind_a_task_is_selected_with_no_change_at_all() {
    let d = tasks();
    let r = d.path();
    // An unrelated change, and the file of the task's package.
    for change in [
        &["--changed", "README.md"][..],
        &["--changed", "lib/src/other.ts"][..],
    ] {
        let v = why(r, change, LIB);
        assert_eq!(v["selected"], true, "{v}");
        assert_eq!(v["noChange"], true, "{change:?}: {v}");
        let w = v["witnesses"]
            .as_array()
            .unwrap()
            .iter()
            .find(|w| w["noChange"] == true)
            .unwrap();
        assert_eq!(w["reason"]["__typename"], "Uncertain", "{v}");
        // Through the task that builds what the test loads.
        let chain: Vec<&str> = w["chain"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h["file"].as_str().unwrap())
            .collect();
        assert!(chain.contains(&"task:lib#build"), "{v}");
        assert_eq!(*chain.last().unwrap(), LIB, "{v}");
    }
    let text = why_text(r, &["--changed", "README.md"], LIB);
    assert!(
        text.contains("; it is selected with no change at all too\n"),
        "{text}"
    );
    assert!(text.contains("(needs no change)"), "{text}");
    // A changed input of the task is one more witness.
    let v = why(r, &["--changed", "lib/src/other.ts"], LIB);
    assert_eq!(v["reason"]["__typename"], "FileChanged", "{v}");
    assert_eq!(v["witnesses"][0]["link"], "task input", "{v}");
    assert_eq!(v["witnesses"].as_array().unwrap().len(), 2, "{v}");
    // A test that loads nothing of it is not selected.
    assert_eq!(
        why(r, &["--changed", "README.md"], APP_PLAIN)["selected"],
        false
    );
}

#[test]
fn an_unknown_test_always_runs() {
    let d = tasks();
    let r = d.path();
    let v = why(r, &["--changed", "README.md"], "app/odd.spec.js");
    assert_eq!(v["selected"], true, "{v}");
    assert_eq!(v["reason"]["__typename"], "UnknownTest", "{v}");
    assert_eq!(v["noChange"], true, "{v}");
}

#[test]
fn the_summary_ranks_the_constructs_by_the_tests_they_select() {
    let d = workspace();
    let r = d.path();
    let change = ["--changed", "packages/util/other.js"];
    let (_, out) = run(
        r,
        &args(&["affected"], &change, &["--why-summary", "--why-json"]),
    );
    let v: Value = serde_json::from_str(&out).unwrap();
    let listed = selected(r, &change);
    assert_eq!(v["selected"], listed.len(), "{v}");
    let rows = v["constructs"].as_array().unwrap();
    assert_eq!(
        rows.iter()
            .map(|r| r["tests"].as_u64().unwrap())
            .sum::<u64>(),
        listed.len() as u64,
        "{v}"
    );
    // Sorted by the tests each selects first.
    let counts: Vec<u64> = rows.iter().map(|r| r["tests"].as_u64().unwrap()).collect();
    assert!(counts.windows(2).all(|w| w[0] >= w[1]), "{v}");
    // The framework's load selects two tests. One of them is selected first by
    // its own read, which names the repository root, as a third test is: the
    // load has one test first, and the name's row says its test stays selected.
    let sum = |link: &str, field: &str| -> u64 {
        rows.iter()
            .filter(|r| r["link"] == link)
            .map(|r| r[field].as_u64().unwrap())
            .sum()
    };
    assert_eq!(sum("bounded load", "tests"), 1, "{v}");
    assert_eq!(sum("name reference", "tests"), 2, "{v}");
    assert_eq!(sum("bounded load", "alsoSelected"), 0, "{v}");
    assert_eq!(sum("name reference", "alsoSelected"), 1, "{v}");
    assert_eq!(rows[0]["tests"], 1, "{v}");
    let load = rows.iter().find(|r| r["link"] == "bounded load").unwrap();
    assert_eq!(
        load["construct"],
        "a bounded load can reach packages/util/ (packages/util/other.js changed)",
        "{v}"
    );
    assert_eq!(load["example"], A, "{v}");
    // The lint finding at the load, with the cost `lint` reports for it: two
    // tests reach it.
    assert_eq!(
        load["code"]["file"], "packages/framework/normalize.js",
        "{v}"
    );
    assert_eq!(load["lint"]["bounded"], true, "{v}");
    assert_eq!(load["lint"]["reached"], 2, "{v}");
    let (_, text) = run(r, &args(&["affected"], &change, &["--why-summary"]));
    assert!(
        text.starts_with(&format!(
            "{} of {} tests selected by the change to packages/util/other.js (modified); 0 of \
             them with no change at all\n",
            listed.len(),
            v["tests"]
        )),
        "{text}"
    );
    assert!(text.contains("  tests   also  construct\n"), "{text}");
}

#[test]
fn the_summary_without_a_change_is_the_floor() {
    let d = tasks();
    let r = d.path();
    // The harness takes a change; a file that nothing reads stands for none.
    let (_, out) = run(
        r,
        &args(
            &["affected"],
            &["--changed", "README.md"],
            &["--why-summary", "--why-json"],
        ),
    );
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["selected"], v["noChange"], "{v}");
    let rows = v["constructs"].as_array().unwrap();
    assert!(
        rows.iter()
            .any(|r| r["reason"] == "UnknownTest" && r["example"] == "app/odd.spec.js"),
        "{v}"
    );
    assert!(
        rows.iter()
            .any(|r| r["reason"] == "Uncertain" && r["example"] == LIB && r["noChange"] == 1),
        "{v}"
    );
}

/// `--explain` with several entries: a note that an entry before it printed is
/// not repeated.
#[test]
fn explain_lists_an_uncertainty_once() {
    let d = workspace();
    let r = d.path();
    let (_, text) = run(
        r,
        &args(
            &["affected"],
            &["--changed", "docs/guide.md"],
            &["--explain", A, "--explain", B],
        ),
    );
    let load = "covered packages/framework/normalize.js:4";
    assert_eq!(text.matches(load).count(), 1, "{text}");
    assert!(
        text.contains("uncertaintie(s) listed above for another entry"),
        "{text}"
    );
    // One entry alone has its notes.
    let (_, text) = run(
        r,
        &args(
            &["affected"],
            &["--changed", "docs/guide.md"],
            &["--explain", B],
        ),
    );
    assert_eq!(text.matches(load).count(), 1, "{text}");
}
