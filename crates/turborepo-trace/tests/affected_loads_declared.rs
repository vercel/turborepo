//! `turbotrace affected` and `turbotrace uncertainty` end to end for bounded
//! loads (`turborepo_trace_graph::loads`): in a workspace whose packages
//! resolve only what they declare, a module load with an unknown target selects
//! the tests that reach it only when the change affects a package the load can
//! reach.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{path::Path, process::Command};

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn run(root: &Path, args: &[&str], env: &[(&str, &str)]) -> String {
    let mut c = Command::new(support::harness());
    c.args(args).current_dir(root);
    c.env_remove("TURBO_TRACE_DEBUG_LOADS_PACKAGE");
    for (k, v) in env {
        c.env(k, v);
    }
    let o = c.output().expect("run turbotrace");
    assert!(
        o.status.success(),
        "turbotrace {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout).unwrap()
}

/// The tests a change of `changed` selects.
fn affected(root: &Path, changed: &str, more: &[&str], env: &[(&str, &str)]) -> Vec<String> {
    let mut args = vec!["affected", "--tests", "jest", "--changed", changed];
    args.extend(more);
    run(root, &args, env).lines().map(str::to_owned).collect()
}

const OFF: &[(&str, &str)] = &[("TURBO_TRACE_DEBUG_LOADS_PACKAGE", "0")];
const A: &str = "services/a/test/a.test.js";
const B: &str = "services/b/test/b.test.js";

/// The shape of a service repository whose test harness boots a framework that
/// loads the handler modules of a route table by a string (names changed).
/// `manager`: what makes it a workspace (`pnpm-workspace.yaml`, or npm's
/// `workspaces`).
///
/// ```text
/// packages/framework/normalize.js   import(spec), spec a parameter       declares util
/// packages/util/index.js            imported by normalize.js; other.js: nothing imports it
/// packages/harness/setup.js         boots the framework                  declares framework
/// packages/shared/s.js                                                    services/a declares it
/// packages/leaf/index.js                                                  nobody declares it
/// services/a/test/a.test.js         uses the harness                     declares harness, shared
/// services/b/test/b.test.js         uses the harness                     declares harness
/// services/b/src/handler.js         no edge leads here
/// ```
fn repo(pnpm: bool) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    if pnpm {
        write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
        write(
            r,
            "pnpm-workspace.yaml",
            "packages:\n  - 'services/*'\n  - 'packages/*'\n",
        );
    } else {
        write(
            r,
            "package.json",
            r#"{"name":"r","version":"1.0.0","workspaces":["services/*","packages/*"]}"#,
        );
    }
    write(r, "README.md", "x\n");
    let pkg = |dir: &str, name: &str, deps: &[&str]| {
        let deps: Vec<String> = deps
            .iter()
            .map(|d| format!(r#""{d}":"workspace:*""#))
            .collect();
        write(
            r,
            &format!("{dir}/package.json"),
            &format!(
                r#"{{"name":"{name}","version":"1.0.0","dependencies":{{{}}}}}"#,
                deps.join(",")
            ),
        );
        for d in &deps {
            // One link per declared dependency, as pnpm installs them.
            let dep = d.split('"').nth(1).unwrap();
            let target = match dep {
                "@x/framework" => "packages/framework",
                "@x/util" => "packages/util",
                "@x/harness" => "packages/harness",
                "@x/shared" => "packages/shared",
                _ => unreachable!(),
            };
            let link = r.join(dir).join("node_modules").join(dep);
            std::fs::create_dir_all(link.parent().unwrap()).unwrap();
            turborepo_trace_core::os::symlink(r.join(target), link).unwrap();
        }
    };
    pkg("packages/framework", "@x/framework", &["@x/util"]);
    pkg("packages/util", "@x/util", &[]);
    pkg("packages/harness", "@x/harness", &["@x/framework"]);
    pkg("packages/shared", "@x/shared", &[]);
    pkg("packages/leaf", "@x/leaf", &[]);
    pkg("services/a", "svc-a", &["@x/harness", "@x/shared"]);
    pkg("services/b", "svc-b", &["@x/harness"]);
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
        "packages/harness/setup.js",
        "const { normalize } = require('@x/framework/normalize.js');\nexports.boot = (routes) => \
         routes.flatMap((r) => normalize(r));\n",
    );
    write(r, "packages/shared/s.js", "module.exports = 1;\n");
    write(r, "packages/leaf/index.js", "module.exports = 1;\n");
    for s in ["a", "b"] {
        write(
            r,
            &format!("services/{s}/jest.config.js"),
            "module.exports = {};\n",
        );
        write(
            r,
            &format!("services/{s}/test/{s}.test.js"),
            "const { boot } = require('@x/harness/setup.js');\ntest('t', () => { \
             boot(globalThis.routes); });\n",
        );
    }
    write(r, "services/b/src/handler.js", "module.exports = 1;\n");
    d
}

#[test]
fn an_unrelated_change_selects_nothing_and_the_switches_open_the_load() {
    let d = repo(true);
    let r = d.path();
    // A package nobody declares, and a file of the root package.
    for path in ["packages/leaf/index.js", "README.md"] {
        assert!(affected(r, path, &[], &[]).is_empty(), "{path}");
        assert_eq!(affected(r, path, &[], OFF), [A, B], "{path}");
        assert_eq!(affected(r, path, &["--strict"], &[]), [A, B], "{path}");
    }
}

#[test]
fn a_change_the_load_can_reach_selects_the_tests_that_reach_it() {
    let d = repo(true);
    let r = d.path();
    // A file that nothing imports, of a package the framework declares.
    assert_eq!(affected(r, "packages/util/other.js", &[], &[]), [A, B]);
    // A package that only services/a declares: the entry part.
    assert_eq!(affected(r, "packages/shared/s.js", &[], &[]), [A]);
    // A file of the test's own package that no edge reaches.
    assert_eq!(affected(r, "services/b/src/handler.js", &[], &[]), [B]);
}

#[test]
fn explain_says_what_the_load_can_reach_and_what_is_assumed() {
    let d = repo(true);
    let r = d.path();
    let out = run(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--changed",
            "packages/util/other.js",
            "--explain",
            A,
        ],
        &[],
    );
    assert!(
        out.contains("the load can reach packages/util/ (packages/util/other.js changed)"),
        "{out}"
    );
    assert!(
        out.contains(
            "  assumed: a file requires, imports and reads only files of its own package and of \
             the packages that package declares, so a load with a computed target loads a file of \
             the package of the loading file or of this entry (services/a/), or of a package one \
             of them declares. The module system guarantees it for a specifier; for a computed \
             path or a mapper pattern it is an assumption (--strict does not make it)"
        ),
        "{out}"
    );
    // The declared closure (the A/B switch) says what it holds the load to.
    let out = run(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--changed",
            "packages/util/other.js",
            "--explain",
            A,
        ],
        &[("TURBO_TRACE_DEBUG_LOADS_STEP", "0")],
    );
    assert!(
        out.contains(
            "this entry's package depends on packages/util/ (packages/util/other.js changed)"
        ),
        "{out}"
    );
    assert!(
        out.contains(
            "  assumed: this entry requires, imports and reads only files of the packages its \
             package (services/a/) depends on, whatever declares them;"
        ),
        "{out}"
    );
    // An entry the change does not select says how its load is covered.
    let out = run(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--changed",
            "packages/leaf/index.js",
            "--explain",
            A,
        ],
        &[],
    );
    assert!(
        out.contains("unaffected services/a/test/a.test.js"),
        "{out}"
    );
    assert!(
        out.contains(
            "covered packages/framework/normalize.js:4 computed_specifier (a load bounded by the \
             declared packages: it selects this entry when the change affects a package the load \
             can reach)"
        ),
        "{out}"
    );
}

/// npm links every workspace package into the root `node_modules`: a package
/// resolves what it does not declare, so the declared packages bound nothing
/// and the load is open.
#[test]
fn a_hoisted_workspace_keeps_its_loads_open() {
    let d = repo(false);
    let r = d.path();
    assert_eq!(affected(r, "packages/leaf/index.js", &[], &[]), [A, B]);
    let out = run(r, &["floor", "--tests", "jest"], &[]);
    assert!(out.contains("uncertainty floor: 2 of 2 entries"), "{out}");
}

/// `node-linker=hoisted` and `shamefully-hoist` make a pnpm workspace a hoisted
/// one.
#[test]
fn a_pnpm_workspace_with_a_hoisted_linker_keeps_its_loads_open() {
    for (file, text) in [
        (".npmrc", "node-linker=hoisted\n"),
        (".npmrc", "shamefully-hoist=true\n"),
        (
            "pnpm-workspace.yaml",
            "packages:\n  - 'services/*'\n  - 'packages/*'\nnodeLinker: hoisted\n",
        ),
        (
            "pnpm-workspace.yaml",
            "packages:\n  - 'services/*'\n  - 'packages/*'\npublicHoistPattern:\n  - '*'\n",
        ),
    ] {
        let d = repo(true);
        let r = d.path();
        write(r, file, text);
        assert_eq!(
            affected(r, "packages/leaf/index.js", &[], &[]),
            [A, B],
            "{file}: {text}"
        );
    }
}

/// A workspace package that a public hoist pattern links into the root resolves
/// from every package.
#[test]
fn a_publicly_hoisted_package_is_reachable_from_every_package() {
    let d = repo(true);
    let r = d.path();
    write(
        r,
        "pnpm-workspace.yaml",
        "packages:\n  - 'services/*'\n  - 'packages/*'\npublicHoistPattern:\n  - '@x/le*'\n",
    );
    assert_eq!(affected(r, "packages/leaf/index.js", &[], &[]), [A, B]);
    assert!(affected(r, "README.md", &[], &[]).is_empty());
}

/// A load is held to what one step reaches: `leaf` is declared by `shared`,
/// which services/a declares, and no step from the framework or from services/a
/// reaches it. `TURBO_TRACE_DEBUG_LOADS_STEP=0` holds the load to the declared
/// closure of the test's package instead.
#[test]
fn the_resolution_step_is_a_switch() {
    const CLOSURE: &[(&str, &str)] = &[("TURBO_TRACE_DEBUG_LOADS_STEP", "0")];
    let d = repo(true);
    let r = d.path();
    write(
        r,
        "packages/shared/package.json",
        r#"{"name":"@x/shared","version":"1.0.0","dependencies":{"@x/leaf":"workspace:*"}}"#,
    );
    // No single step from services/a or from the framework reaches leaf.
    assert!(affected(r, "packages/leaf/index.js", &[], &[]).is_empty());
    assert_eq!(affected(r, "packages/util/other.js", &[], &[]), [A, B]);
    // services/a depends on leaf through shared: the declared closure selects its
    // test.
    assert_eq!(affected(r, "packages/leaf/index.js", &[], CLOSURE), [A]);
    assert_eq!(affected(r, "packages/util/other.js", &[], CLOSURE), [A, B]);
}

/// The lint rule's plain report: a relative import into a package nobody
/// declares.
#[test]
fn certain_edges_outside_the_declared_packages_are_reported() {
    let d = repo(true);
    let r = d.path();
    write(
        r,
        "services/b/src/reach.js",
        "module.exports = require('../../../packages/leaf/index.js');\n",
    );
    // `lint` exits with 1 when it has findings.
    let o = Command::new(support::harness())
        .args(["lint", "--tests", "jest", "--no-baseline"])
        .args(["--rule", "undeclared-dependency=warn", "--format", "json"])
        .current_dir(r)
        .env_remove("TURBO_TRACE_DEBUG_LOADS_PACKAGE")
        .output()
        .expect("run turbotrace");
    let lint: serde_json::Value = serde_json::from_slice(&o.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&o.stderr)));
    let found: Vec<(&str, u64, &str)> = lint["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["rule"] == "undeclared-dependency")
        .map(|f| {
            (
                f["file"].as_str().unwrap(),
                f["line"].as_u64().unwrap(),
                f["message"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(found.len(), 1, "{lint}");
    assert_eq!(
        (found[0].0, found[0].1),
        ("services/b/src/reach.js", 1),
        "{lint}"
    );
    assert!(found[0].2.contains("packages/leaf/"), "{lint}");
}

/// A package in whose directory other packages live (a Go module at
/// `packages/`) is no dependency of theirs: it does not chain them together.
#[test]
fn a_package_around_others_does_not_join_them() {
    let d = repo(true);
    let r = d.path();
    write(
        r,
        "packages/go.mod",
        "module example.com/packages\n\ngo 1.22\n",
    );
    assert!(affected(r, "packages/leaf/index.js", &[], &[]).is_empty());
    assert_eq!(affected(r, "packages/util/other.js", &[], &[]), [A, B]);
}

/// A pnpm workspace of `(directory, name, declared packages)` with one
/// `node_modules` link per declared dependency, as pnpm installs them.
fn workspace(r: &Path, packages: &[(&str, &str, &[&str])]) {
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(
        r,
        "pnpm-workspace.yaml",
        "packages:\n  - 'services/*'\n  - 'packages/*'\n",
    );
    write(r, "README.md", "x\n");
    for (dir, name, deps) in packages {
        manifest(r, packages, dir, name, deps);
    }
}

/// Writes the manifest of one package of [`workspace`] and links what it
/// declares.
fn manifest(r: &Path, packages: &[(&str, &str, &[&str])], dir: &str, name: &str, deps: &[&str]) {
    let list: Vec<String> = deps
        .iter()
        .map(|d| format!(r#""{d}":"workspace:*""#))
        .collect();
    write(
        r,
        &format!("{dir}/package.json"),
        &format!(
            r#"{{"name":"{name}","version":"1.0.0","dependencies":{{{}}}}}"#,
            list.join(",")
        ),
    );
    let modules = r.join(dir).join("node_modules");
    let _ = std::fs::remove_dir_all(&modules);
    for dep in deps {
        let target = packages.iter().find(|p| p.1 == *dep).expect("a package").0;
        let link = modules.join(dep);
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        turborepo_trace_core::os::symlink(r.join(target), link).unwrap();
    }
}

const C: &str = "services/c/test/c.test.js";
const D: &str = "services/d/test/d.test.js";

/// A loader in a shared package that tests call with a value: `services/c` with
/// one that nothing evaluates, `services/d` with a path it locates.
///
/// ```text
/// packages/h/index.js           load(spec): require(spec)                    declares dep
/// packages/dep/index.js         other.js: nothing imports either
/// packages/far/index.js                                                       nobody declares it
/// services/c/test/c.test.js     load(globalThis.name)                        declares h
/// services/d/test/d.test.js     load(__dirname + '/../src/handler.js')       declares h
/// services/d/src/handler.js     what that path names; other.js: nothing imports it
/// ```
fn loader_repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    workspace(
        r,
        &[
            ("packages/h", "@x/h", &["@x/dep"]),
            ("packages/dep", "@x/dep", &[]),
            ("packages/far", "@x/far", &[]),
            ("services/c", "svc-c", &["@x/h"]),
            ("services/d", "svc-d", &["@x/h"]),
        ],
    );
    write(
        r,
        "packages/h/index.js",
        "export function load(spec) {\n  return require(spec);\n}\n",
    );
    write(r, "packages/dep/index.js", "module.exports = 1;\n");
    write(r, "packages/dep/other.js", "module.exports = 2;\n");
    write(r, "packages/far/index.js", "module.exports = 3;\n");
    write(r, "services/d/src/handler.js", "module.exports = 4;\n");
    write(r, "services/d/src/other.js", "module.exports = 5;\n");
    for (s, arg) in [
        ("c", "globalThis.name"),
        ("d", "__dirname + '/../src/handler.js'"),
    ] {
        write(
            r,
            &format!("services/{s}/jest.config.js"),
            "module.exports = {};\n",
        );
        write(
            r,
            &format!("services/{s}/test/{s}.test.js"),
            &format!("import {{ load }} from '@x/h';\ntest('t', () => {{ load({arg}); }});\n"),
        );
    }
    d
}

/// Cross-file binding closes the loader for the test that passes a path it
/// locates: that test depends on the module the path names and on nothing else,
/// and the rule does not bring it back. The load that stays unknown (a value
/// nothing evaluates) is the only bounded one; its record sits on the test that
/// passes the value.
#[test]
fn a_load_that_binding_closed_is_not_a_bounded_load() {
    let d = loader_repo();
    let r = d.path();
    const NO_BIND: &[(&str, &str)] = &[("TURBO_TRACE_DEBUG_BIND", "0")];
    // The module the path names: the test that passes it, through its bound
    // dependency (a certain edge). The other test's package does not depend on
    // services/d.
    assert_eq!(affected(r, "services/d/src/handler.js", &[], &[]), [D]);
    // A file of the same package that nothing imports: no test. The loader is
    // closed for the test of `d`, and the rule does not open it again; without
    // binding the loader is a bounded load of that test, and its own package is
    // affected.
    assert!(affected(r, "services/d/src/other.js", &[], &[]).is_empty());
    assert_eq!(affected(r, "services/d/src/other.js", &[], NO_BIND), [D]);
    // A package the loader's package declares: only the load that stays unknown
    // (the package of `c` depends on `dep` through `h`); without binding, both
    // tests.
    assert_eq!(affected(r, "packages/dep/other.js", &[], &[]), [C]);
    assert_eq!(affected(r, "packages/dep/other.js", &[], NO_BIND), [C, D]);
    // A package nobody declares: nothing, with or without binding.
    assert!(affected(r, "packages/far/index.js", &[], &[]).is_empty());
    assert!(affected(r, "packages/far/index.js", &[], NO_BIND).is_empty());
    // One bounded record, on the supplier; none on the floor.
    let floor: serde_json::Value =
        serde_json::from_str(&run(r, &["floor", "--tests", "jest", "--json"], &[])).unwrap();
    assert_eq!(floor["floor"], 0, "{floor}");
    assert_eq!(floor["entries"], 2, "{floor}");
    assert_eq!(floor["records"]["declared"], 1, "{floor}");
    // The test of `d` is explained by the edge, not by a load.
    let why = run(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--changed",
            "services/d/src/handler.js",
            "--explain",
            D,
        ],
        &[],
    );
    assert!(why.contains("modified services/d/src/handler.js"), "{why}");
    assert!(!why.contains("uncertain"), "{why}");
}

/// A load is held to what one step reaches from the file where it happens.
/// Binding moves the record of an unknown value to the test that passes it, but
/// the module system still resolves the value from the loader's file: `h`
/// declares `dep`, the test's package does not.
#[test]
fn the_resolution_step_starts_at_the_file_with_the_load() {
    let d = loader_repo();
    let r = d.path();
    const STEP: &[(&str, &str)] = &[];
    const STEP_NO_BIND: &[(&str, &str)] = &[("TURBO_TRACE_DEBUG_BIND", "0")];
    assert_eq!(affected(r, "packages/dep/other.js", &[], STEP), [C]);
    assert_eq!(
        affected(r, "packages/dep/other.js", &[], STEP_NO_BIND),
        [C, D]
    );
    assert!(affected(r, "packages/far/index.js", &[], STEP).is_empty());
    let why = run(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--changed",
            "packages/dep/other.js",
            "--explain",
            C,
        ],
        STEP,
    );
    assert!(why.contains("the load can reach packages/dep/"), "{why}");
}

/// A declared dependency added and removed between two queries on the same
/// graph: the rule reads the manifests as they are (not the view of them that
/// module resolution gets, which leaves the dependency lists out, so that such
/// an edit re-traces nothing), and every answer equals that of a graph built
/// from nothing.
#[test]
fn a_dependency_added_or_removed_between_queries_is_read_from_the_manifest() {
    let d = repo(true);
    let r = d.path();
    let packages: &[(&str, &str, &[&str])] = &[
        ("packages/leaf", "@x/leaf", &[]),
        ("packages/harness", "@x/harness", &[]),
    ];
    let paths = [
        "packages/leaf/index.js",
        "services/b/package.json",
        "packages/util/other.js",
        "README.md",
    ];
    let answers = |graph: Option<&Path>| -> Vec<Vec<String>> {
        paths
            .iter()
            .map(|p| match graph {
                Some(g) => affected(r, p, &["--graph", g.to_str().unwrap()], &[]),
                None => affected(r, p, &[], &[]),
            })
            .collect()
    };
    let fresh = || {
        let g = tempfile::tempdir().unwrap();
        answers(Some(g.path()))
    };
    let own = |s: &str| vec![s.to_owned()];
    let both = vec![A.to_owned(), B.to_owned()];
    // Nobody declares `leaf`.
    let before = answers(None);
    assert_eq!(before, [vec![], own(B), both.clone(), vec![]]);
    assert_eq!(before, fresh());
    // services/b declares it.
    manifest(
        r,
        packages,
        "services/b",
        "svc-b",
        &["@x/harness", "@x/leaf"],
    );
    let with = answers(None);
    assert_eq!(with, [own(B), own(B), both.clone(), vec![]]);
    assert_eq!(with, fresh());
    // And no longer.
    manifest(r, packages, "services/b", "svc-b", &["@x/harness"]);
    let after = answers(None);
    assert_eq!(after, before);
    assert_eq!(after, fresh());
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

/// The tests a change since the last commit selects.
fn since(root: &Path) -> Vec<String> {
    run(
        root,
        &["affected", "--tests", "jest", "--since", "HEAD"],
        &[],
    )
    .lines()
    .map(str::to_owned)
    .collect()
}

/// The root manifest lends its declarations to every package (resolution walks
/// up), and the install layout decides what is linked for every package. A
/// change of either that changes what packages resolve selects every test that
/// reaches a bounded load; a change of those files that does not (a script, a
/// third-party version) selects none.
#[test]
fn a_change_of_what_every_package_resolves_selects_the_bounded_loads() {
    let d = repo(true);
    let r = d.path();
    write(r, ".gitignore", "node_modules\n.turbo/trace\n");
    git(r, &["init", "-q"]);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "base"]);
    assert!(since(r).is_empty());
    // A script and a third-party package in the root manifest: the same first-party
    // declarations as before.
    write(
        r,
        "package.json",
        r#"{"name":"r","version":"1.0.0","scripts":{"x":"true"},"devDependencies":{"left-pad":"1.0.0"}}"#,
    );
    assert!(since(r).is_empty(), "{:?}", since(r));
    // The root declares a workspace package: every package resolves it now.
    write(
        r,
        "package.json",
        r#"{"name":"r","version":"1.0.0","devDependencies":{"@x/leaf":"workspace:*"}}"#,
    );
    assert_eq!(since(r), [A, B]);
    let why = run(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--since",
            "HEAD",
            "--explain",
            A,
        ],
        &[],
    );
    assert!(
        why.contains("package.json changed what it resolves"),
        "{why}"
    );
    // Without a base to compare with (`--changed`), a changed root manifest counts.
    assert_eq!(affected(r, "package.json", &[], &[]), [A, B]);
    git(r, &["checkout", "-q", "package.json"]);
    // The layout: a setting that changes nothing about the links, then a public
    // hoist pattern.
    write(r, ".npmrc", "strict-peer-dependencies=false\n");
    assert!(since(r).is_empty(), "{:?}", since(r));
    write(r, ".npmrc", "public-hoist-pattern[]=@x/le*\n");
    assert_eq!(since(r), [A, B]);
    // A hoisted layout opens the loads altogether.
    write(r, ".npmrc", "shamefully-hoist=true\n");
    assert_eq!(since(r), [A, B]);
}

/// A package that the change removes is in no manifest and no directory of the
/// tree that is evaluated. Before the change a load could reach it from every
/// package that declared it, or that a package around it or the layout lent it
/// to: their tests are selected, whether the declarations went with the package
/// or were left behind.
#[test]
fn a_removed_package_selects_the_loads_that_could_reach_it() {
    let base = |root_manifest: &str, npmrc: &str| {
        let d = repo(true);
        let r = d.path();
        write(r, ".gitignore", "node_modules\n.turbo/trace\n");
        if !root_manifest.is_empty() {
            write(r, "package.json", root_manifest);
        }
        if !npmrc.is_empty() {
            write(r, ".npmrc", npmrc);
        }
        git(r, &["init", "-q"]);
        git(r, &["add", "-A"]);
        git(r, &["commit", "-q", "-m", "base"]);
        assert!(since(r).is_empty());
        d
    };
    let remove = |r: &Path, dir: &str| std::fs::remove_dir_all(r.join(dir)).unwrap();
    let plain = r#"{"name":"r","version":"1.0.0"}"#;
    let lends = r#"{"name":"r","version":"1.0.0","devDependencies":{"@x/leaf":"workspace:*"}}"#;

    // `services/a` declares `@x/shared`; the declaration goes with the package.
    let d = base("", "");
    let r = d.path();
    remove(r, "packages/shared");
    write(
        r,
        "services/a/package.json",
        r#"{"name":"svc-a","version":"1.0.0","dependencies":{"@x/harness":"workspace:*"}}"#,
    );
    assert_eq!(since(r), [A]);
    // The declaration is left behind (the install is broken: the test cannot pass
    // as before either).
    let d = base("", "");
    let r = d.path();
    remove(r, "packages/shared");
    assert_eq!(since(r), [A]);
    let why = run(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--since",
            "HEAD",
            "--explain",
            A,
        ],
        &[],
    );
    assert!(why.contains("packages/shared/package.json"), "{why}");

    // The root manifest lent `@x/leaf` to every package.
    let d = base(lends, "");
    let r = d.path();
    remove(r, "packages/leaf");
    write(r, "package.json", plain);
    assert_eq!(since(r), [A, B]);
    let d = base(lends, "");
    let r = d.path();
    remove(r, "packages/leaf");
    assert_eq!(since(r), [A, B]);

    // The layout linked it for every package.
    let d = base("", "public-hoist-pattern[]=@x/le*\n");
    let r = d.path();
    remove(r, "packages/leaf");
    assert_eq!(since(r), [A, B]);

    // A package that nothing declared and nothing lent: no load could reach it.
    let d = base("", "");
    let r = d.path();
    remove(r, "packages/leaf");
    assert!(since(r).is_empty(), "{:?}", since(r));

    // Without a base to read the removed manifest from, its name is not known:
    // every bounded load counts.
    let d = base("", "");
    let r = d.path();
    remove(r, "packages/leaf");
    assert_eq!(affected(r, "packages/leaf/package.json", &[], &[]), [A, B]);
}
