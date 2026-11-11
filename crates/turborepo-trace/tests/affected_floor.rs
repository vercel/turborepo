//! `turbotrace affected` and `turbotrace uncertainty` end to end for records
//! that used to keep tests selected on every change although something bounds
//! them:
//!
//! - what a test runner may load in place of the modules the graph has (a Vite
//!   plugin that is not modelled, a module mapper applied to a pattern): a
//!   module load with an unknown target, held to the packages the test's
//!   package declares (`turborepo_trace_graph::loads`);
//! - a tsconfig that extends a config that is not there: a literal reference
//!   that does not resolve, covered by the paths its resolution looked for;
//! - a load next to a module that is not installed: covered when the lockfile
//!   gives the importer a registry package, a load with an unknown target when
//!   it links a package of the repository.
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
    for k in [
        "TURBO_TRACE_DEBUG_LOADS_PACKAGE",
        "TURBO_TRACE_DEBUG_FW_LOADS",
        "TURBO_TRACE_DEBUG_TSCONFIG_PROBED",
    ] {
        c.env_remove(k);
    }
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

/// The tests of `framework` a change of `changed` selects.
fn affected(
    root: &Path,
    framework: &str,
    changed: &str,
    more: &[&str],
    env: &[(&str, &str)],
) -> Vec<String> {
    let mut args = vec!["affected", "--tests", framework, "--changed", changed];
    args.extend(more);
    run(root, &args, env).lines().map(str::to_owned).collect()
}

fn floor(root: &Path, framework: &str, env: &[(&str, &str)]) -> serde_json::Value {
    run(root, &["graph", "build"], env);
    serde_json::from_str(&run(root, &["floor", "--tests", framework, "--json"], env)).unwrap()
}

/// A pnpm workspace with one link per declared dependency.
///
/// ```text
/// apps/web     vitest, `plugins: [custom()]` of a package the adapter does not know; declares @x/lib
/// apps/api     vitest without plugins
/// packages/lib   declared by apps/web; `other.js` is imported by nothing
/// packages/leaf  declared by nobody
/// ```
fn plugin_repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(
        r,
        "pnpm-workspace.yaml",
        "packages:\n  - 'apps/*'\n  - 'packages/*'\n",
    );
    write(r, "README.md", "x\n");
    write(
        r,
        "node_modules/vitest/package.json",
        r#"{"name":"vitest","version":"3.0.0"}"#,
    );
    write(r, "node_modules/vitest/config.js", "module.exports = {};\n");
    write(
        r,
        "apps/web/package.json",
        r#"{"name":"web","version":"1.0.0","dependencies":{"@x/lib":"workspace:*"}}"#,
    );
    let link = r.join("apps/web/node_modules/@x/lib");
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    turborepo_trace_core::os::symlink(r.join("packages/lib"), link).unwrap();
    write(
        r,
        "apps/web/vitest.config.ts",
        "import { defineConfig } from 'vitest/config';\nimport custom from \
         'vite-plugin-custom';\n\nexport default defineConfig({\n  plugins: [custom()],\n});\n",
    );
    write(r, "apps/web/src/a.ts", "export const a = 1;\n");
    write(r, "apps/web/src/unused.ts", "export const u = 1;\n");
    write(
        r,
        "apps/web/src/a.test.ts",
        "import { a } from './a';\ntest('a', () => expect(a).toBe(1));\n",
    );
    write(
        r,
        "apps/web/src/b.test.ts",
        "test('b', () => expect(1).toBe(1));\n",
    );
    write(
        r,
        "apps/api/package.json",
        r#"{"name":"api","version":"1.0.0"}"#,
    );
    write(
        r,
        "apps/api/vitest.config.ts",
        "import { defineConfig } from 'vitest/config';\n\nexport default defineConfig({});\n",
    );
    write(
        r,
        "apps/api/src/c.test.ts",
        "test('c', () => expect(1).toBe(1));\n",
    );
    for p in ["lib", "leaf"] {
        write(
            r,
            &format!("packages/{p}/package.json"),
            &format!(r#"{{"name":"@x/{p}","version":"1.0.0"}}"#),
        );
        write(
            r,
            &format!("packages/{p}/index.js"),
            "module.exports = 1;\n",
        );
        write(
            r,
            &format!("packages/{p}/other.js"),
            "module.exports = 2;\n",
        );
    }
    d
}

const WEB: [&str; 2] = ["apps/web/src/a.test.ts", "apps/web/src/b.test.ts"];

/// A Vite plugin that is not modelled may resolve, load or rewrite the modules
/// the tests of its project import: a module load with an unknown target of
/// each of them. Where a package resolves only what it declares, that load is
/// held to the project's package and the packages it declares.
#[test]
fn an_unknown_vite_plugin_is_bounded_by_the_packages_its_project_declares() {
    let d = plugin_repo();
    let r = d.path();
    // Not on the floor: a load bounded by the declared packages, one record per
    // test.
    let v = floor(r, "vitest", &[]);
    assert_eq!(v["floor"], 0, "{v}");
    assert_eq!(v["records"]["declared"], 2, "{v}");
    // A package nobody declares, a file of the root package, another project.
    for path in ["packages/leaf/other.js", "README.md", "apps/api/src/x.ts"] {
        assert!(
            affected(r, "vitest", path, &[], &[]).is_empty(),
            "{path}: {:?}",
            affected(r, "vitest", path, &[], &[])
        );
    }
    // The project's own package (a file nothing imports) and a package it declares.
    assert_eq!(
        affected(r, "vitest", "apps/web/src/unused.ts", &[], &[]),
        WEB
    );
    assert_eq!(
        affected(r, "vitest", "packages/lib/other.js", &[], &[]),
        WEB
    );
    // Known edges are what they were.
    assert_eq!(
        affected(r, "vitest", "apps/api/src/c.test.ts", &[], &[]),
        ["apps/api/src/c.test.ts"]
    );
    // Without the assumption, and with the bound switched off: every change.
    for path in ["packages/leaf/other.js", "README.md"] {
        assert_eq!(
            affected(r, "vitest", path, &["--strict"], &[]),
            WEB,
            "{path}"
        );
        assert_eq!(
            affected(
                r,
                "vitest",
                path,
                &[],
                &[("TURBO_TRACE_DEBUG_LOADS_PACKAGE", "0")]
            ),
            WEB,
            "{path}"
        );
    }
    // `--explain` says what is assumed.
    let out = run(
        r,
        &[
            "affected",
            "--tests",
            "vitest",
            "--changed",
            "packages/lib/other.js",
            "--explain",
            WEB[1],
        ],
        &[],
    );
    assert!(
        out.contains("the load can reach packages/lib/ (packages/lib/other.js changed)"),
        "{out}"
    );
    assert!(
        out.contains("vite plugins are not modelled"),
        "the record names the plugin: {out}"
    );
    assert!(
        out.contains(
            "  assumed: a file requires, imports and reads only files of its own package and of \
             the packages that package declares"
        ),
        "{out}"
    );
}

/// The record as a file read (the switch for A/B measurements): open, as
/// before.
#[test]
fn the_plugin_record_as_a_read_is_open() {
    let d = plugin_repo();
    let r = d.path();
    let off = [("TURBO_TRACE_DEBUG_FW_LOADS", "0")];
    let v = floor(r, "vitest", &off);
    assert_eq!(v["floor"], 2, "{v}");
    assert_eq!(affected(r, "vitest", "README.md", &[], &off), WEB);
}

/// The record sits on every test of the project; the lint reports it once, at
/// the line of the configuration that names the plugin, with a key that carries
/// no place.
#[test]
fn the_lint_reports_an_unknown_plugin_once_at_its_config_line() {
    let d = plugin_repo();
    let r = d.path();
    let o = Command::new(support::harness())
        .args(["lint", "--tests", "vitest", "--format", "json"])
        .current_dir(r)
        .output()
        .expect("run turbotrace");
    let v: serde_json::Value = serde_json::from_slice(&o.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&o.stderr)));
    let found: Vec<(&str, u64, &str)> = v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["rule"] == "unconstrained-load")
        .map(|f| {
            (
                f["file"].as_str().unwrap(),
                f["line"].as_u64().unwrap(),
                f["key"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(found.len(), 1, "{v}");
    assert_eq!(
        (found[0].0, found[0].1),
        ("apps/web/vitest.config.ts", 5),
        "{v}"
    );
    assert!(
        found[0]
            .2
            .starts_with("conditional_resolution/unknown: plugins: vite plugins are not modelled"),
        "{v}"
    );
    // The finding says what bounds the load, counts the tests of the project once,
    // and its fix is about the configuration (there is no specifier to narrow).
    let f = v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["rule"] == "unconstrained-load")
        .unwrap();
    let message = f["message"].as_str().unwrap();
    assert!(
        message.ends_with(
            "the test runner may load another module than the one turbo trace sees; bounded to \
             apps/web/ and the packages it declares (2 of 5), and to the package of each test and \
             what that declares"
        ),
        "{message}"
    );
    assert_eq!(
        f["cost"],
        serde_json::json!({"reached": 2, "exclusive": 2, "bounded": true})
    );
    assert!(
        f["fix"]
            .as_str()
            .unwrap()
            .starts_with("no specifier to narrow here: declare in the project's package.json"),
        "{f}"
    );
    // The rule for edges that leave the declared packages finds nothing new here.
    let o = Command::new(support::harness())
        .args(["lint", "--tests", "vitest", "--format", "json"])
        .args(["--rule", "undeclared-dependency=error"])
        .current_dir(r)
        .output()
        .expect("run turbotrace");
    let v: serde_json::Value = serde_json::from_slice(&o.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&o.stderr)));
    let rules: Vec<&str> = v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["rule"].as_str().unwrap())
        .collect();
    assert_eq!(rules, ["unconstrained-load"], "{v}");
}

fn git(root: &Path, args: &[&str]) {
    let o = Command::new("git")
        .args(["-c", "user.email=t@t", "-c", "user.name=t"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(root)
        .output()
        .expect("run git");
    assert!(
        o.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
}

/// A package that the change removes is in no manifest of the tree that is
/// evaluated. What a runner may load in place of a module could reach it
/// before, from every project whose package declared it: its tests are selected
/// by the removal, like the tests that reach any other bounded load. A package
/// that nobody declared selects nothing.
#[test]
fn a_removed_package_selects_the_tests_of_a_project_with_an_unknown_plugin() {
    let base = || {
        let d = plugin_repo();
        let r = d.path();
        write(r, ".gitignore", "node_modules\n.turbo/trace\n");
        git(r, &["init", "-q"]);
        git(r, &["add", "-A"]);
        git(r, &["commit", "-q", "-m", "base"]);
        d
    };
    let since = |r: &Path| -> Vec<String> {
        run(
            r,
            &["affected", "--tests", "vitest", "--since", "HEAD"],
            &[],
        )
        .lines()
        .map(str::to_owned)
        .collect()
    };
    let d = base();
    let r = d.path();
    assert!(since(r).is_empty(), "{:?}", since(r));
    // `apps/web` declares `@x/lib`; the declaration is left behind.
    std::fs::remove_dir_all(r.join("packages/lib")).unwrap();
    assert_eq!(since(r), WEB);
    let why = run(
        r,
        &[
            "affected",
            "--tests",
            "vitest",
            "--since",
            "HEAD",
            "--explain",
            WEB[1],
        ],
        &[],
    );
    assert!(why.contains("packages/lib/package.json"), "{why}");
    // The declaration goes with the package.
    write(
        r,
        "apps/web/package.json",
        r#"{"name":"web","version":"1.0.0"}"#,
    );
    assert_eq!(since(r), WEB);
    // Nobody declared `@x/leaf`.
    let d = base();
    let r = d.path();
    std::fs::remove_dir_all(r.join("packages/leaf")).unwrap();
    assert!(since(r).is_empty(), "{:?}", since(r));
}

/// A config that extends a config that is not there (a generated one:
/// `.svelte-kit`, `.nuxt`, `.astro`) does not load. The files it governs
/// resolve without its `paths`, which is all the tree says; the missing config
/// appearing, or an edit of the config that names it, selects.
#[test]
fn a_tsconfig_that_extends_a_missing_config_is_covered_by_its_probes() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(r, "jest.config.js", "module.exports = {};\n");
    write(r, "README.md", "x\n");
    write(
        r,
        "fixtures/app/jsconfig.json",
        r#"{"extends": "./.generated/tsconfig.json"}"#,
    );
    write(r, "fixtures/app/main.js", "module.exports = 1;\n");
    write(r, "src/plain.js", "module.exports = 2;\n");
    write(
        r,
        "test/a.test.js",
        "require('../fixtures/app/main');\ntest('a', () => {});\n",
    );
    write(
        r,
        "test/b.test.js",
        "require('../src/plain');\ntest('b', () => {});\n",
    );
    let a = ["test/a.test.js"];
    let v = floor(r, "jest", &[]);
    assert_eq!(v["floor"], 0, "{v}");
    assert_eq!(v["records"]["probed"], 1, "{v}");
    assert!(affected(r, "jest", "README.md", &[], &[]).is_empty());
    assert_eq!(affected(r, "jest", "README.md", &["--strict"], &[]), a);
    // The config that names the missing one is an input of the file it governs.
    assert_eq!(
        affected(r, "jest", "fixtures/app/jsconfig.json", &[], &[]),
        a
    );
    // The missing config appears.
    write(r, "fixtures/app/.generated/tsconfig.json", "{}\n");
    assert_eq!(
        affected(r, "jest", "fixtures/app/.generated/tsconfig.json", &[], &[]),
        a
    );
}

/// The same tree with the rule switched off, and a config that fails for
/// another reason: nothing says what would make it load, so the record stays
/// open.
#[test]
fn a_tsconfig_that_does_not_load_for_another_reason_stays_open() {
    let repo = |config: &str| {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
        write(r, "jest.config.js", "module.exports = {};\n");
        write(r, "README.md", "x\n");
        write(r, "fixtures/app/jsconfig.json", config);
        write(r, "fixtures/app/main.js", "module.exports = 1;\n");
        write(
            r,
            "test/a.test.js",
            "require('../fixtures/app/main');\ntest('a', () => {});\n",
        );
        d
    };
    let a = ["test/a.test.js"];
    let d = repo(r#"{"extends": "./.generated/tsconfig.json"}"#);
    let off = [("TURBO_TRACE_DEBUG_TSCONFIG_PROBED", "0")];
    assert_eq!(floor(d.path(), "jest", &off)["floor"], 1);
    assert_eq!(affected(d.path(), "jest", "README.md", &[], &off), a);
    // A config that extends itself.
    let d = repo(r#"{"extends": "./jsconfig.json"}"#);
    assert_eq!(floor(d.path(), "jest", &[])["floor"], 1);
    assert_eq!(affected(d.path(), "jest", "README.md", &[], &[]), a);
}

/// A load next to a module that is not installed
/// (`createRequire(require.resolve('pkg'))` in a checkout without
/// `node_modules`). What the module is, is the lockfile's answer for
/// the importer:
///
/// - a registry package: the load is taken to stay in that package's tree. The
///   record is covered (the lockfile entry and the importer's manifest select),
///   `--explain` states the assumption, `--strict` counts it, and
///   `TURBO_TRACE_DEBUG_LOCATED_PACKAGE=0` makes it a load with an unknown
///   target. A package of the repository that merely has the name is not what
///   the importer gets;
/// - a link to a package of the repository: a path next to it is a file of the
///   repository, so the load has an unknown target (bounded by the declared
///   packages here).
#[test]
fn a_load_next_to_a_module_that_is_not_installed_follows_the_lockfile() {
    const G: &str = "services/g/test/g.test.js";
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(
        r,
        "pnpm-workspace.yaml",
        "packages:\n  - 'services/*'\n  - 'packages/*'\n",
    );
    write(r, "README.md", "x\n");
    // A package of the repository that has the name of the registry package.
    write(
        r,
        "packages/lp/package.json",
        r#"{"name":"left-pad","version":"1.0.0"}"#,
    );
    write(r, "packages/lp/index.js", "module.exports = 1;\n");
    write(r, "packages/lp/inner.js", "module.exports = 2;\n");
    write(
        r,
        "services/g/package.json",
        r#"{"name":"svc-g","version":"1.0.0","dependencies":{"left-pad":"^1"}}"#,
    );
    write(r, "services/g/jest.config.js", "module.exports = {};\n");
    write(r, "services/g/notes.txt", "n\n");
    write(
        r,
        G,
        "const { createRequire } = require('module');\nconst inner = \
         createRequire(require.resolve('left-pad'))('./inner.js');\ntest('g', () => { inner; });\n",
    );
    let lock = |version: &str| {
        format!(
            "lockfileVersion: '9.0'\nimporters:\n  .: {{}}\n  packages/lp: {{}}\n  services/g:\n    dependencies:\n      left-pad:\n        specifier: ^1\n        version: {version}\npackages:\n  left-pad@1.3.0:\n    resolution: {{integrity: sha512-pad}}\nsnapshots:\n  left-pad@1.3.0: {{}}\n"
        )
    };
    let none: [&str; 0] = [];

    // The lockfile gives the importer the registry package.
    write(r, "pnpm-lock.yaml", &lock("1.3.0"));
    let v = floor(r, "jest", &[]);
    assert_eq!(v["floor"], 0, "{v}");
    assert_eq!(v["records"]["probed"], 1, "{v}");
    for path in [
        "README.md",
        "services/g/notes.txt",
        "packages/lp/inner.js",
        "packages/lp/package.json",
    ] {
        assert_eq!(affected(r, "jest", path, &[], &[]), none, "{path}");
    }
    assert_eq!(affected(r, "jest", "pnpm-lock.yaml", &[], &[]), [G]);
    assert_eq!(affected(r, "jest", "README.md", &["--strict"], &[]), [G]);
    let out = run(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--changed",
            "README.md",
            "--explain",
            G,
        ],
        &[],
    );
    assert!(
        out.contains("covered services/g/test/g.test.js:2 unresolved_specifier")
            && out.contains(
                "'left-pad' is not installed (searched from services/g/test); the load is taken \
                 to stay in the package tree of the lockfile entry of 'left-pad'"
            ),
        "{out}"
    );
    // Without the assumption: a load with an unknown target, which the declared
    // packages bound (a change in the test's own package selects it).
    let off = [("TURBO_TRACE_DEBUG_LOCATED_PACKAGE", "0")];
    let other = ["--graph", ".tt-off"];
    assert_eq!(
        affected(r, "jest", "services/g/notes.txt", &other, &off),
        [G]
    );
    assert_eq!(affected(r, "jest", "README.md", &other, &off), none);

    // The lockfile links the importer to the package of the repository: the file is
    // traced again, and the load can reach that package.
    write(r, "pnpm-lock.yaml", &lock("link:../../packages/lp"));
    let v = floor(r, "jest", &[]);
    assert_eq!(v["floor"], 0, "{v}");
    assert_eq!(v["records"]["declared"], 1, "{v}");
    assert_eq!(v["records"]["probed"], 0, "{v}");
    assert_eq!(affected(r, "jest", "packages/lp/inner.js", &[], &[]), [G]);
    assert_eq!(affected(r, "jest", "services/g/notes.txt", &[], &[]), [G]);
    assert_eq!(affected(r, "jest", "README.md", &[], &[]), none);
    // And back.
    write(r, "pnpm-lock.yaml", &lock("1.3.0"));
    let v = floor(r, "jest", &[]);
    assert_eq!(v["records"]["probed"], 1, "{v}");
    assert_eq!(affected(r, "jest", "packages/lp/inner.js", &[], &[]), none);
}
