//! `affected --since`: a `tsconfig.json` whose project references
//! changed (`crates/turborepo-trace/src/tsconfigdiff.rs`).
//!
//! `references` orders builds and guides editors. A test runner that only
//! transpiles does not read it, with two exceptions the rule excludes: Vite
//! parses every referenced config and uses one for a file the referencing
//! config does not include, and a Vite plugin may read the list. So a reference
//! to a project elsewhere in the repository that keeps to its own directory is
//! no input change for jest tests of a transpile-only transform and for
//! vitest tests, unless a project that uses `vite-tsconfig-paths` can load a
//! file below the config; type-check tasks see it as before.
//!
//! The shape is a real monorepo's: a package config that extends a shared
//! config package (linked in `node_modules`), has `compilerOptions.types` and a
//! list of references to sibling packages, and gains one more reference. Argued
//! from the tools' sources, not from a runtime oracle.
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
    c.args(args).current_dir(root);
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().expect("run the harness")
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

const GATE_TSCONFIG: &str = "packages/gate/tsconfig.json";

/// The config of `packages/gate` with the given references and compiler
/// options.
fn gate_tsconfig(references: &[&str], options: &str) -> String {
    let refs: Vec<String> = references
        .iter()
        .map(|r| format!("    {{ \"path\": \"{r}\" }}"))
        .collect();
    format!(
        "{{\n  \"extends\": [\"@acme/typescript-config/base.json\"],\n  \"compilerOptions\": {{ \
         {options} }},\n  \"references\": [\n{}\n  ]\n}}\n",
        refs.join(",\n")
    )
}

/// Commits the working tree as the new base, with the config of `packages/gate`
/// as it was.
fn commit(r: &Path, message: &str) {
    write(r, GATE_TSCONFIG, &gate_tsconfig(&BASE_REFS, TYPES));
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "--allow-empty", "-m", message]);
}

const BASE_REFS: [&str; 2] = ["../headers", "../iam"];
const TYPES: &str = "\"types\": [\"vitest/globals\"]";

fn package(r: &Path, dir: &str, name: &str, scripts: &str) {
    write(
        r,
        &format!("{dir}/package.json"),
        &format!(
            "{{\"name\":\"{name}\",\"version\":\"1.0.0\",\"scripts\":{{{scripts}}},\"\
             devDependencies\":{{\"@acme/typescript-config\":\"workspace:*\"}}}}"
        ),
    );
    // The workspace link of the shared config package, as an install leaves it.
    let link = r.join(dir).join("node_modules/@acme/typescript-config");
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    let up = "../".repeat(dir.split('/').count() + 2);
    turborepo_trace_core::os::symlink(format!("{up}tooling/typescript-config"), link).unwrap();
}

const V_A: &str = "packages/gate/src/a.test.ts";
const V_B: &str = "packages/gate/src/b.test.ts";
const J_C: &str = "packages/gate/jest/c.test.js";
const P_T: &str = "packages/plug/src/p.test.ts";

fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"root","private":true}"#);
    write(
        r,
        "pnpm-workspace.yaml",
        "packages:\n  - 'packages/*'\n  - 'tooling/*'\n",
    );
    write(r, ".gitignore", ".turbo/trace/\nnode_modules/\n");
    write(
        r,
        "tooling/typescript-config/package.json",
        r#"{"name":"@acme/typescript-config","version":"1.0.0"}"#,
    );
    write(
        r,
        "tooling/typescript-config/base.json",
        "{\n  // shared options\n  \"compilerOptions\": { \"strict\": true, \"module\": \
         \"esnext\", \"moduleResolution\": \"bundler\" }\n}\n",
    );
    // The package whose config changes: vitest tests with a plain config, one jest
    // test.
    package(
        r,
        "packages/gate",
        "@acme/gate",
        "\"test\":\"vitest run\",\"typecheck\":\"tsc --noEmit\"",
    );
    write(r, GATE_TSCONFIG, &gate_tsconfig(&BASE_REFS, TYPES));
    write(
        r,
        "packages/gate/vitest.config.ts",
        "import { defineConfig } from 'vitest/config';\n\nexport default defineConfig({ test: { \
         include: ['src/**/*.test.ts'] } });\n",
    );
    write(
        r,
        "packages/gate/jest.config.js",
        "module.exports = { testMatch: ['<rootDir>/jest/**/*.test.js'] };\n",
    );
    for stem in ["a", "b"] {
        write(
            r,
            &format!("packages/gate/src/{stem}.ts"),
            &format!("export const {stem} = 1;\n"),
        );
        write(
            r,
            &format!("packages/gate/src/{stem}.test.ts"),
            &format!(
                "import {{ expect, test }} from 'vitest';\nimport {{ {stem} }} from \
                 './{stem}';\n\ntest('{stem}', () => {{\n  expect({stem}).toBe(1);\n}});\n"
            ),
        );
    }
    write(r, "packages/gate/jest/c.js", "module.exports = 1;\n");
    write(
        r,
        "packages/gate/jest/c.test.js",
        "const c = require('./c');\n\ntest('c', () => {\n  expect(c).toBe(1);\n});\n",
    );
    // The projects it references.
    for name in ["headers", "iam", "otel"] {
        let dir = format!("packages/{name}");
        package(r, &dir, &format!("@acme/{name}"), "");
        write(r, &format!("{dir}/src/index.ts"), "export const x = 1;\n");
    }
    write(
        r,
        "packages/headers/tsconfig.json",
        "{ \"extends\": [\"@acme/typescript-config/base.json\"] }\n",
    );
    write(
        r,
        "packages/iam/tsconfig.json",
        "{ \"extends\": [\"@acme/typescript-config/base.json\"], \"include\": [\"**/*.ts\", \
         \"**/*.tsx\"] }\n",
    );
    write(
        r,
        "packages/otel/tsconfig.json",
        "{\n  \"extends\": [\"@acme/typescript-config/base.json\"],\n  \"include\": [\"src\", \
         \"build.ts\"],\n  \"exclude\": [\"node_modules\", \"dist\"],\n  \"references\": [{ \
         \"path\": \"../headers\" }]\n}\n",
    );
    // A vitest project whose config loads a plugin that reads tsconfig files.
    package(r, "packages/plug", "@acme/plug", "\"test\":\"vitest run\"");
    write(
        r,
        "packages/plug/tsconfig.json",
        &gate_tsconfig(&BASE_REFS, TYPES),
    );
    write(
        r,
        "packages/plug/vitest.config.ts",
        "import tsconfigPaths from 'vite-tsconfig-paths';\nimport { defineConfig } from \
         'vitest/config';\n\nexport default defineConfig({ plugins: [tsconfigPaths()] });\n",
    );
    write(r, "packages/plug/src/p.ts", "export const p = 1;\n");
    write(
        r,
        "packages/plug/src/p.test.ts",
        "import { expect, test } from 'vitest';\nimport { p } from './p';\n\ntest('p', () => {\n  \
         expect(p).toBe(1);\n});\n",
    );
    git(r, &["init", "-q"]);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "base"]);
    d
}

fn lines(o: Output, args: &[&str]) -> Vec<String> {
    assert!(
        o.status.success(),
        "harness {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

const TESTS: [&str; 9] = [
    "affected",
    "--since",
    "HEAD",
    "--on-uncertain",
    "ignore",
    "--tests",
    "jest",
    "--tests",
    "vitest",
];

fn tests_env(r: &Path, env: &[(&str, &str)]) -> Vec<String> {
    lines(run_env(r, &TESTS, env), &TESTS)
}

fn tests(r: &Path) -> Vec<String> {
    tests_env(r, &[])
}

fn tasks(r: &Path) -> Vec<String> {
    let args = [
        "affected",
        "--since",
        "HEAD",
        "--on-uncertain",
        "ignore",
        "--tasks",
        "**",
    ];
    lines(run_env(r, &args, &[]), &args)
}

fn stats(r: &Path) -> String {
    let mut args = TESTS.to_vec();
    args.push("--stats");
    String::from_utf8_lossy(&run_env(r, &args, &[]).stderr).into_owned()
}

/// The rule off.
const OFF: [(&str, &str); 1] = [("TURBO_TRACE_DEBUG_TSCONFIG_FIELDS", "0")];

const WITH_OTEL: [&str; 3] = ["../otel", "../headers", "../iam"];

#[test]
fn a_reference_to_a_project_elsewhere_is_not_an_input_of_transpile_only_tests() {
    let d = repo();
    let r = d.path();
    // The real change: one more referenced project, a sibling package with its own
    // `include`.
    write(r, GATE_TSCONFIG, &gate_tsconfig(&WITH_OTEL, TYPES));
    // Without the rule every test below the config is selected, through the config
    // only.
    assert_eq!(tests_env(r, &OFF), [J_C, V_A, V_B]);
    assert!(tests(r).is_empty());
    let err = stats(r);
    assert!(
        err.contains(
            "tsconfig packages/gate/tsconfig.json: only `references` changed (../otel), and no \
             referenced project that came or went can include a file of packages/gate: not an \
             input change for tests that only transpile"
        ),
        "{err}"
    );
    // `--explain` says why a test below the config is not selected.
    let args = [&TESTS[..], &["--explain", V_A]].concat();
    let explained = String::from_utf8(run_env(r, &args, &[]).stdout).unwrap();
    assert!(
        explained.contains(&format!("unaffected {V_A}"))
            && explained
                .contains("note: tsconfig packages/gate/tsconfig.json: only `references` changed"),
        "{explained}"
    );
    // The type check reads the whole file.
    assert!(
        tasks(r).contains(&"task:packages/gate#typecheck".to_owned()),
        "{:?}",
        tasks(r)
    );
    // A reference that goes away, and comments.
    write(r, GATE_TSCONFIG, &gate_tsconfig(&["../iam"], TYPES));
    assert_eq!(tests_env(r, &OFF), [J_C, V_A, V_B]);
    assert!(tests(r).is_empty());
    write(
        r,
        GATE_TSCONFIG,
        &format!("// the gate\n{}", gate_tsconfig(&BASE_REFS, TYPES)),
    );
    assert_eq!(tests_env(r, &OFF), [J_C, V_A, V_B]);
    assert!(tests(r).is_empty());
    assert!(stats(r).contains("only comments or formatting changed"));
}

#[test]
fn a_project_with_a_plugin_that_reads_references_keeps_the_whole_file() {
    let d = repo();
    let r = d.path();
    // `vite-tsconfig-paths` builds its projects from the references.
    write(
        r,
        "packages/plug/tsconfig.json",
        &gate_tsconfig(&WITH_OTEL, TYPES),
    );
    assert_eq!(tests(r), [P_T]);
    let err = stats(r);
    assert!(
        err.contains(
            "tsconfig packages/plug/tsconfig.json: a whole-file change: \
             packages/plug/vitest.config.ts loads vite-tsconfig-paths, a Vite plugin that reads \
             project references"
        ),
        "{err}"
    );
}

#[test]
fn everything_else_is_a_whole_file_change() {
    let d = repo();
    let r = d.path();
    let all = [J_C, V_A, V_B];
    let whole = |r: &Path, why: &str| {
        assert_eq!(tests(r), all, "{why}");
        let err = stats(r);
        assert!(
            err.contains("tsconfig packages/gate/tsconfig.json: a whole-file change:")
                && err.contains(why),
            "{why}: {err}"
        );
    };
    // A compiler option, alone or with a reference.
    write(
        r,
        GATE_TSCONFIG,
        &gate_tsconfig(
            &WITH_OTEL,
            "\"types\": [\"vitest/globals\"], \"jsx\": \"react-jsx\"",
        ),
    );
    whole(r, "`compilerOptions` changed");
    // The same projects in another order: the first one that includes a file wins.
    write(
        r,
        GATE_TSCONFIG,
        &gate_tsconfig(&["../iam", "../headers"], TYPES),
    );
    whole(r, "the order of the referenced projects changed");
    // A project below the config's directory can include its files.
    write(
        r,
        "packages/gate/sub/tsconfig.lib.json",
        "{ \"include\": [\"../src\"] }\n",
    );
    write(
        r,
        GATE_TSCONFIG,
        &gate_tsconfig(&["./sub/tsconfig.lib.json", "../headers", "../iam"], TYPES),
    );
    whole(r, "is in, above or below the config's directory");
    // A project elsewhere whose `include` climbs out of its directory.
    std::fs::remove_dir_all(r.join("packages/gate/sub")).unwrap();
    write(
        r,
        "packages/otel/tsconfig.json",
        "{ \"include\": [\"src\", \"../gate/src\"] }\n",
    );
    commit(r, "otel includes gate");
    write(r, GATE_TSCONFIG, &gate_tsconfig(&WITH_OTEL, TYPES));
    whole(
        r,
        "packages/otel includes files from packages/gate/src, which holds or is held by the \
         directory of the config",
    );
    // A referenced project that does not exist: Vite fails on it.
    write(
        r,
        GATE_TSCONFIG,
        &gate_tsconfig(&["../missing", "../headers", "../iam"], TYPES),
    );
    whole(
        r,
        "the referenced project packages/missing does not load: Tsconfig not found \
         packages/missing",
    );
    // A reference to a project that did not exist at the base goes away: Vite
    // failed on it before.
    commit(r, "base again");
    write(
        r,
        GATE_TSCONFIG,
        &gate_tsconfig(&["../missing", "../headers", "../iam"], TYPES),
    );
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "a dangling reference"]);
    write(r, GATE_TSCONFIG, &gate_tsconfig(&BASE_REFS, TYPES));
    whole(r, "the referenced project packages/missing does not load");
    commit(r, "base again");
    // A referenced project that changes in the same diff.
    write(
        r,
        "packages/otel/tsconfig.json",
        "{ \"include\": [\"src\"] }\n",
    );
    write(r, GATE_TSCONFIG, &gate_tsconfig(&WITH_OTEL, TYPES));
    whole(r, "packages/otel/tsconfig.json changes in the same diff");
    // A list TypeScript reports.
    write(
        r,
        GATE_TSCONFIG,
        &gate_tsconfig(&BASE_REFS, TYPES).replace("{ \"path\": \"../iam\" }", "\"../iam\""),
    );
    whole(r, "`references` is not a list of projects at one revision");
}

#[test]
fn a_project_that_inherits_what_it_includes() {
    let d = repo();
    let r = d.path();
    // `headers` has no `include` of its own and extends a config that has none:
    // `**/*` below its own directory.
    write(r, GATE_TSCONFIG, &gate_tsconfig(&["../iam"], TYPES));
    assert!(tests(r).is_empty());
    // An extended config that defines `include` relative to the repository root
    // reaches every package.
    write(
        r,
        "tsconfig.base.json",
        "{ \"include\": [\"packages/*/src\"] }\n",
    );
    write(
        r,
        "packages/headers/tsconfig.json",
        "{ \"extends\": \"../../tsconfig.base.json\" }\n",
    );
    commit(r, "headers inherits include");
    write(r, GATE_TSCONFIG, &gate_tsconfig(&["../iam"], TYPES));
    assert_eq!(tests(r), [J_C, V_A, V_B]);
    assert!(stats(r).contains("includes files from the repository root"));
    // An `extends` whose package is not installed: the tracer's loader goes on
    // without that base, and what the base would add is not known.
    write(
        r,
        "packages/headers/tsconfig.json",
        "{ \"extends\": \"@acme/not-installed/base.json\" }\n",
    );
    commit(r, "headers extends a missing package");
    write(r, GATE_TSCONFIG, &gate_tsconfig(&["../iam"], TYPES));
    assert_eq!(tests(r), [J_C, V_A, V_B]);
    assert!(stats(r).contains("`extends` names a package that is not installed here"));
}

/// The tests on the uncertainty floor, on the graph in `graph`.
fn floor(r: &Path, graph: &Path, env: &[(&str, &str)]) -> u64 {
    let graph = graph.to_str().unwrap();
    let args = [
        "floor", "--graph", graph, "--tests", "jest", "--tests", "vitest", "--json",
    ];
    let o = run_env(r, &args, env);
    assert!(
        o.status.success(),
        "harness {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).expect("json");
    v["floor"].as_u64().unwrap_or_else(|| panic!("{v}"))
}

/// A config that names a config that is not there (the target of an `extends`,
/// a project reference) is covered by the places its resolution looked at, not
/// open (`TURBO_TRACE_DEBUG_TSCONFIG_PROBED`). Next to the rule for
/// `references`:
///
/// - the rule changes what is open, never what a change reaches: every diff
///   here selects the same tests with the rule and without it (`--on-uncertain
///   ignore` leaves the floor out);
/// - a referenced project whose `extends` is missing: the files below the
///   referencing configs are not on the floor; an edit of the project that
///   names the missing config selects their tests; a reference to that project
///   that goes is a whole-file change (the project does not parse with its
///   chain);
/// - a referenced project that is not there: the files below the referencing
///   config are not on the floor, and the reference going is a whole-file
///   change;
/// - a config whose own `extends` is missing and whose `references` change: the
///   missing config appearing in the same diff selects.
#[test]
fn a_config_that_is_not_there_next_to_changed_references() {
    let all = [J_C, V_A, V_B];
    let scratch = tempfile::tempdir().unwrap();
    let off = [("TURBO_TRACE_DEBUG_TSCONFIG_PROBED", "0")];
    // What the diff selects: with the rule, and without it on a graph of its own
    // (the switch works while a file is traced).
    let both = |r: &Path, graph: &str, what: &str| -> Vec<String> {
        let with = tests(r);
        let graph = scratch.path().join(graph);
        let mut args = TESTS.to_vec();
        args.extend(["--graph", graph.to_str().unwrap()]);
        let without = lines(run_env(r, &args, &off), &args);
        assert_eq!(with, without, "{what}: with the rule and without it");
        with
    };

    // `headers`, which the configs of `gate` and `plug` reference, extends a
    // generated config that is not there.
    let d = repo();
    let r = d.path();
    let headers = "packages/headers/tsconfig.json";
    let generated = "packages/headers/.generated/tsconfig.json";
    let names_generated = "{ \"extends\": \"./.generated/tsconfig.json\" }\n";
    let before = floor(r, &scratch.path().join("base"), &[]);
    write(r, headers, names_generated);
    commit(r, "headers extends a generated config");
    // Covered, not open: without the rule the tests below the two referencing
    // configs are on the floor.
    let referencing = [J_C, V_A, V_B, P_T];
    assert_eq!(floor(r, &scratch.path().join("on"), &[]), before);
    assert_eq!(floor(r, &scratch.path().join("off"), &off), before + 4);
    assert!(both(r, "t1", "nothing changed").is_empty());
    // The project that names the missing config changes.
    write(
        r,
        headers,
        "{ \"extends\": \"./.generated/tsconfig.json\", \"compilerOptions\": {} }\n",
    );
    assert_eq!(both(r, "t1", "the naming project changes"), referencing);
    write(r, headers, names_generated);
    // The reference to it goes: the project does not parse with its chain, so
    // nothing says that Vite could not have used it.
    write(r, GATE_TSCONFIG, &gate_tsconfig(&["../iam"], TYPES));
    assert_eq!(both(r, "t1", "the reference goes"), all);
    let err = stats(r);
    assert!(
        err.contains("tsconfig packages/gate/tsconfig.json: a whole-file change:")
            && err.contains(
                "the referenced project packages/headers does not load: Tsconfig not found \
                 packages/headers/.generated/tsconfig.json"
            ),
        "{err}"
    );
    write(r, GATE_TSCONFIG, &gate_tsconfig(&BASE_REFS, TYPES));
    // The generated config appears: the project loads and includes no file below
    // the referencing configs, as before `headers` named it. Whatever that
    // selects, the rule is no part of it. (A project that loads and comes to be
    // broken: `a_referenced_project_that_comes_to_be_broken`.)
    write(r, generated, "{}\n");
    both(r, "t1", "the generated config appears");

    // A referenced project that is not there, at both revisions.
    let d = repo();
    let r = d.path();
    let dangling = gate_tsconfig(&["../missing", "../headers", "../iam"], TYPES);
    write(r, GATE_TSCONFIG, &dangling);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "a dangling reference"]);
    assert_eq!(floor(r, &scratch.path().join("dangling"), &[]), before);
    assert!(both(r, "t2", "nothing changed").is_empty());
    // It appears, and includes no file below `gate`: as for the generated config
    // above.
    write(r, "packages/missing/tsconfig.json", "{}\n");
    both(r, "t2", "the referenced project appears");
    // The dangling reference goes (the project is not there): Vite failed on it
    // before.
    std::fs::remove_dir_all(r.join("packages/missing")).unwrap();
    write(r, GATE_TSCONFIG, &gate_tsconfig(&BASE_REFS, TYPES));
    assert_eq!(both(r, "t2", "the dangling reference goes"), all);

    // The config of `gate` itself extends a generated config, and its references
    // change.
    let d = repo();
    let r = d.path();
    let broken = |refs: &[&str]| {
        gate_tsconfig(refs, TYPES).replace(
            "@acme/typescript-config/base.json",
            "./.generated/tsconfig.json",
        )
    };
    write(r, GATE_TSCONFIG, &broken(&BASE_REFS));
    git(r, &["add", "-A"]);
    git(
        r,
        &["commit", "-q", "-m", "gate extends a generated config"],
    );
    assert_eq!(floor(r, &scratch.path().join("own"), &[]), before);
    assert!(both(r, "t3", "nothing changed").is_empty());
    // Only the references: no transform reads them, and the config loads no better.
    write(r, GATE_TSCONFIG, &broken(&WITH_OTEL));
    let only_references = both(r, "t3", "only the references change");
    assert!(
        only_references.is_empty(),
        "{only_references:?}\n{}",
        stats(r)
    );
    // With the generated config appearing in the same diff the config loads:
    // selected, whatever the references are.
    write(r, "packages/gate/.generated/tsconfig.json", "{}\n");
    assert_eq!(both(r, "t3", "references and the generated config"), all);
    write(r, GATE_TSCONFIG, &broken(&BASE_REFS));
    assert_eq!(both(r, "t3", "the generated config appears"), all);
}

/// A referenced project that loads and does not include the files below the
/// referencing config is no input of them, and still decides their records: a
/// project that does not load is reported on every file that consults it, and
/// one that includes a file governs it. So what the project's load read is a
/// key observation of those files (`turborepo_trace_core::record::key`): when
/// the project comes to be broken, or to include them, a refresh traces them
/// again and the graph is the one a cold build writes. The same diff selects
/// the same tests on the refreshed graph of the repository and on a new
/// graph, and the record that an open policy counts is there after a refresh.
#[test]
fn a_referenced_project_that_comes_to_be_broken() {
    let scratch = tempfile::tempdir().unwrap();
    let off = [("TURBO_TRACE_DEBUG_TSCONFIG_PROBED", "0")];
    let referencing = [J_C, V_A, V_B, P_T];
    let cold = std::cell::Cell::new(0);
    // The tests the diff selects on the repository's graph (built at the base,
    // refreshed now) and on a graph built now.
    let check = |r: &Path, what: &str| -> Vec<String> {
        let refreshed = tests(r);
        cold.set(cold.get() + 1);
        let graph = scratch.path().join(format!("cold-{}", cold.get()));
        let mut args = TESTS.to_vec();
        args.extend(["--graph", graph.to_str().unwrap()]);
        let fresh = lines(run_env(r, &args, &[]), &args);
        assert_eq!(refreshed, fresh, "{what}: refreshed and cold");
        refreshed
    };

    // `headers`, which the configs of `gate` and `plug` reference, extends a
    // generated config that is there.
    let d = repo();
    let r = d.path();
    let headers = "packages/headers/tsconfig.json";
    let generated = "packages/headers/.generated/tsconfig.json";
    write(
        r,
        headers,
        "{ \"extends\": \"./.generated/tsconfig.json\" }\n",
    );
    write(r, generated, "{}\n");
    commit(r, "headers extends a generated config");
    // A graph traced with a missing config as an open record, for the floor.
    let open = scratch.path().join("open");
    let before = floor(r, &open, &off);
    assert!(check(r, "nothing changed").is_empty());
    // The generated config goes: the project does not load. The files below the two
    // referencing configs looked for it now, and their tests are selected.
    std::fs::remove_file(r.join(generated)).unwrap();
    assert_eq!(check(r, "the generated config is deleted"), referencing);
    assert_eq!(
        floor(r, &open, &off),
        before + 4,
        "the refreshed graph has the record"
    );
    assert_eq!(
        floor(r, &scratch.path().join("open-cold"), &off),
        before + 4
    );
    // It comes back: the project loads again and is no input, as at the base.
    write(r, generated, "{}\n");
    assert!(check(r, "the generated config is back").is_empty());
    assert_eq!(floor(r, &open, &off), before);

    // The project's own config goes.
    let d = repo();
    let r = d.path();
    let open = scratch.path().join("open-2");
    let before = floor(r, &open, &off);
    assert!(check(r, "nothing changed").is_empty());
    std::fs::remove_file(r.join(headers)).unwrap();
    assert_eq!(check(r, "the referenced config is deleted"), referencing);
    assert_eq!(floor(r, &open, &off), before + 4);
    assert_eq!(
        floor(r, &scratch.path().join("open-2-cold"), &off),
        before + 4
    );

    // The project comes to be a config that does not parse.
    let d = repo();
    let r = d.path();
    assert!(check(r, "nothing changed").is_empty());
    write(r, headers, "{ \"extends\": [\n");
    assert_eq!(
        check(r, "the referenced config does not parse"),
        referencing
    );

    // The project comes to include the sources of `gate`: it governs them now, and
    // its config is their input.
    let d = repo();
    let r = d.path();
    assert!(check(r, "nothing changed").is_empty());
    write(
        r,
        headers,
        "{ \"extends\": [\"@acme/typescript-config/base.json\"], \"include\": \
         [\"../gate/src/**/*.ts\"] }\n",
    );
    assert_eq!(
        check(r, "the referenced project includes files of gate"),
        [V_A, V_B]
    );

    // An edit of the project that changes neither: nothing below the referencing
    // configs is selected, on either graph.
    let d = repo();
    let r = d.path();
    assert!(check(r, "nothing changed").is_empty());
    write(
        r,
        headers,
        "{ \"extends\": [\"@acme/typescript-config/base.json\"], \"compilerOptions\": { \
         \"strict\": false } }\n",
    );
    assert!(check(r, "an option of the referenced project").is_empty());
}

// ---------------------------------------------------------------------------------------
// A referenced project that stops governing a file
// (`crates/turborepo-trace/src/tsgovern.rs`).
// ---------------------------------------------------------------------------------------

const G_A: &str = "packages/app/tests/a.test.ts";
const G_B: &str = "packages/app/tests/b.test.ts";
const G_APP: &str = "packages/app/tsconfig.json";
const G_LIB: &str = "packages/app/tsconfig.lib.json";

/// The project config of the fixture: `@lib/*` names `v2`, with the given other
/// fields.
fn lib_config(fields: &str) -> String {
    format!(
        "{{ \"compilerOptions\": {{ \"paths\": {{ \"@lib/*\": [\"./v2/*\"] }} }}, {fields} }}\n"
    )
}

/// `packages/app`: a config whose `paths` send `@lib/*` to `v1` and that
/// references the project `tsconfig.lib.json`, whose `paths` send it to `v2`
/// and which includes `src`. `src/helper.ts` imports `@lib/dep`. The tests are
/// in `tests`, which the project never includes: `a` imports the helper, `b` a
/// file of `other`.
fn governed() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"root","private":true}"#);
    write(
        r,
        "pnpm-workspace.yaml",
        "packages:\n  - 'packages/*'\n  - 'tooling/*'\n",
    );
    write(r, ".gitignore", ".turbo/trace/\nnode_modules/\n");
    write(
        r,
        "packages/app/package.json",
        r#"{"name":"@acme/app","version":"1.0.0","scripts":{"test":"vitest run"}}"#,
    );
    write(
        r,
        "packages/app/vitest.config.ts",
        "import { defineConfig } from 'vitest/config';\n\nexport default defineConfig({ test: { \
         include: ['tests/**/*.test.ts'] } });\n",
    );
    write(
        r,
        G_APP,
        "{\n  \"compilerOptions\": { \"paths\": { \"@lib/*\": [\"./v1/*\"] } },\n  \
         \"references\": [{ \"path\": \"./tsconfig.lib.json\" }]\n}\n",
    );
    write(r, G_LIB, &lib_config("\"include\": [\"src\"]"));
    for v in ["v1", "v2"] {
        write(
            r,
            &format!("packages/app/{v}/dep.ts"),
            &format!("export const dep = '{v}';\n"),
        );
    }
    write(
        r,
        "packages/app/src/helper.ts",
        "import { dep } from '@lib/dep';\n\nexport const helper = dep;\n",
    );
    write(r, "packages/app/other/o.ts", "export const o = 1;\n");
    // The runner, as an install leaves it: nothing of the fixture is open, also
    // under `--strict`.
    write(
        r,
        "node_modules/vitest/package.json",
        r#"{"name":"vitest","version":"1.0.0","exports":{".":"./index.js","./config":"./config.js"}}"#,
    );
    write(
        r,
        "node_modules/vitest/index.js",
        "exports.test = () => {};\nexports.expect = () => {};\n",
    );
    write(
        r,
        "node_modules/vitest/config.js",
        "exports.defineConfig = (c) => c;\n",
    );
    write(
        r,
        G_A,
        "import { expect, test } from 'vitest';\nimport { helper } from \
         '../src/helper';\n\ntest('a', () => {\n  expect(helper).toBe('v2');\n});\n",
    );
    write(
        r,
        G_B,
        "import { expect, test } from 'vitest';\nimport { o } from '../other/o';\n\ntest('b', () \
         => {\n  expect(o).toBe(1);\n});\n",
    );
    git(r, &["init", "-q"]);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "base"]);
    d
}

/// Commits the working tree as the new base.
fn rebase(r: &Path, message: &str) {
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "--allow-empty", "-m", message]);
}

/// What the diff against `HEAD` selects under `--on-uncertain ignore`, the
/// default policy and `--strict`, each on the repository's graph (built at an
/// earlier state, refreshed now) and on a new graph, which must agree.
fn selections(r: &Path, scratch: &Path, what: &str) -> Vec<(String, Vec<String>)> {
    static COLD: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let mut out = Vec::new();
    for (policy, flags) in [
        ("ignore", &["--on-uncertain", "ignore"][..]),
        ("default", &[][..]),
        ("strict", &["--strict"][..]),
    ] {
        let base = [
            &["affected", "--since", "HEAD"][..],
            flags,
            &["--tests", "jest", "--tests", "vitest"][..],
        ]
        .concat();
        let refreshed = lines(run_env(r, &base, &[]), &base);
        let n = COLD.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let graph = scratch.join(format!("g-cold-{n}"));
        let mut args = base.clone();
        args.extend(["--graph", graph.to_str().unwrap()]);
        let cold = lines(run_env(r, &args, &[]), &args);
        assert_eq!(refreshed, cold, "{what}: refreshed and cold ({policy})");
        out.push((policy.to_owned(), refreshed));
    }
    out
}

/// The rule of `crates/turborepo-trace/src/tsgovern.rs` off: the selection
/// before it.
const GOVERNED_OFF: [(&str, &str); 1] = [("TURBO_TRACE_DEBUG_TSCONFIG_GOVERNED", "0")];

/// The graph of the working tree: per file its edges and uncertainty records as
/// text, and the edges as pairs of paths.
#[derive(Default)]
struct Records {
    by_file: std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
    edges: Vec<(String, String)>,
}

fn records(r: &Path) -> Records {
    records_with(r, &[])
}

/// As [`records`], with more arguments (`--graph <dir>`: a graph built from
/// nothing).
fn records_with(r: &Path, extra: &[&str]) -> Records {
    let args = [&["graph", "export", "--format", "ndjson"][..], extra].concat();
    let rows: Vec<serde_json::Value> = lines(run_env(r, &args, &[]), &args)
        .iter()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let paths: std::collections::BTreeMap<u64, String> = rows
        .iter()
        .filter(|v| v["type"] == "file")
        .map(|v| {
            (
                v["id"].as_u64().unwrap(),
                v["path"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let mut out = Records::default();
    for v in &rows {
        match v["type"].as_str() {
            Some("edge") => {
                let from = paths[&v["from"].as_u64().unwrap()].clone();
                let to = v["to"].as_u64().map(|t| paths[&t].clone());
                out.by_file.entry(from.clone()).or_default().insert(format!(
                    "{} {} -> {}",
                    v["kind"],
                    v["specifier"],
                    to.as_deref().unwrap_or("?")
                ));
                if let Some(to) = to {
                    out.edges.push((from, to));
                }
            }
            Some("uncertainty") => {
                let file = paths[&v["file"].as_u64().unwrap()].clone();
                out.by_file
                    .entry(file)
                    .or_default()
                    .insert(format!("uncertain {} {}", v["kind"], v["detail"]));
            }
            _ => {}
        }
    }
    out
}

/// The tests that load, in one of the two graphs, a file whose edges or
/// uncertainty records differ between them. The two graphs are builds of the
/// base and of the working tree: a selection that lacks one of these tests is
/// unsound, whatever rule made it.
fn must_select(before: &Records, after: &Records, tests: &[&str]) -> Vec<String> {
    let differ: std::collections::BTreeSet<&str> = before
        .by_file
        .keys()
        .chain(after.by_file.keys())
        .filter(|f| before.by_file.get(*f) != after.by_file.get(*f))
        .map(String::as_str)
        .collect();
    let reaches = |test: &str| {
        let mut seen = vec![test.to_owned()];
        let mut i = 0;
        while i < seen.len() {
            if differ.contains(seen[i].as_str()) {
                return true;
            }
            for (from, to) in before.edges.iter().chain(&after.edges) {
                if *from == seen[i] && !seen.contains(to) {
                    seen.push(to.clone());
                }
            }
            i += 1;
        }
        false
    };
    tests
        .iter()
        .filter(|t| reaches(t))
        .map(|t| (*t).to_owned())
        .collect()
}

/// One diff of the fixture. `before`: the graph at the base. With the rule the
/// diff selects `want` under every policy, on the refreshed graph and on a new
/// one, and `want` has every test that a build at the base and a build now say
/// it must have. Without the rule it selects `without`.
///
/// "A build now" is a graph made from nothing, and the repository's graph,
/// refreshed, must hold the same records: a refresh that keeps a stale
/// resolution would otherwise hide the very difference the oracle looks for.
fn governed_diff(
    r: &Path,
    scratch: &Path,
    what: &str,
    before: &Records,
    want: &[&str],
    without: &[&str],
) {
    static ORACLE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = ORACLE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let cold = scratch.join(format!("g-oracle-{n}"));
    let after = records_with(r, &["--graph", cold.to_str().unwrap()]);
    assert!(
        records(r).by_file == after.by_file,
        "{what}: the refreshed graph and a new one hold other records"
    );
    let must = must_select(before, &after, &[G_A, G_B]);
    for t in &must {
        assert!(
            want.contains(&t.as_str()),
            "{what}: {t} loads a file whose records differ between the base and now"
        );
    }
    for (policy, got) in selections(r, scratch, what) {
        assert_eq!(got, want, "{what} ({policy})");
    }
    assert_eq!(
        tests_env(r, &GOVERNED_OFF),
        without,
        "{what}: without the rule"
    );
}

fn app_config(rest: &str) -> String {
    format!(
        "{{\n  \"compilerOptions\": {{ \"paths\": {{ \"@lib/*\": [\"./v1/*\"] }} }}{rest}\n}}\n"
    )
}

const LIB_REFERENCE: &str = ",\n  \"references\": [{ \"path\": \"./tsconfig.lib.json\" }]";

/// The defect: at the base the project includes `src`, so `src/helper.ts`
/// resolves `@lib/dep` through the project's `paths`; then the project stops
/// including it and the config above governs the file. The project is no input
/// of the file in the graph of the working tree, and nothing was selected. The
/// test that loads the helper is selected now, and only that one; `--explain`
/// says why.
#[test]
fn a_referenced_project_that_stops_including_a_file() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    let before = records(r);
    governed_diff(r, s, "nothing changed", &before, &[], &[]);
    // `include` names another directory.
    write(r, G_LIB, &lib_config("\"include\": [\"lib\"]"));
    governed_diff(r, s, "include goes elsewhere", &before, &[G_A], &[]);
    let args = [&TESTS[..], &["--explain", G_A, "--explain", G_B]].concat();
    let explained = String::from_utf8(run_env(r, &args, &[]).stdout).unwrap();
    assert!(
        explained.contains(&format!(
            "affected {G_A}: modified {G_LIB} (read or probed) at packages/app/src/helper.ts"
        )) && explained.contains(&format!("unaffected {G_B}"))
            && explained.contains(&format!(
                "note: tsconfig {G_LIB}: a project that a config references, or a config such a \
                 project extends, changed in what it includes or whether it loads: 1 of the "
            ))
            && explained.contains(&format!(
                "are governed differently than at the base and are seeded as changed \
                 (packages/app/src/helper.ts: governed by {G_LIB} at the base and by {G_APP} now)"
            )),
        "{explained}"
    );
    // The base is the merge base of the revision and HEAD, or the revision itself.
    git(r, &["branch", "other"]);
    git(r, &["stash", "-q"]);
    git(r, &["checkout", "-q", "other"]);
    write(r, "packages/app/other/o.ts", "export const o = 2;\n");
    rebase(r, "other moves on");
    git(r, &["checkout", "-q", "-"]);
    git(r, &["stash", "pop", "-q"]);
    for (exact, want) in [(false, &[G_A][..]), (true, &[G_A, G_B][..])] {
        let mut args = vec!["affected", "--since", "other", "--tests", "vitest"];
        if exact {
            args.push("--exact-base");
        }
        assert_eq!(lines(run_env(r, &args, &[]), &args), want, "exact {exact}");
    }
    // The mirror: the project comes to include the file. It governs it now, an
    // input.
    rebase(r, "the project includes lib");
    let before = records(r);
    write(r, G_LIB, &lib_config("\"include\": [\"src\"]"));
    governed_diff(r, s, "include comes back", &before, &[G_A], &[G_A]);

    // `exclude` gains the file; `files` loses it.
    let d = governed();
    let r = d.path();
    let before = records(r);
    write(
        r,
        G_LIB,
        &lib_config("\"include\": [\"src\"], \"exclude\": [\"src/helper.ts\"]"),
    );
    governed_diff(r, s, "exclude gains the file", &before, &[G_A], &[]);
    write(r, G_LIB, &lib_config("\"files\": [\"src/helper.ts\"]"));
    rebase(r, "files");
    let before = records(r);
    write(r, G_LIB, &lib_config("\"files\": [\"src/none.ts\"]"));
    governed_diff(r, s, "files loses the file", &before, &[G_A], &[]);

    // `allowJs` goes: the project no longer includes a JavaScript file.
    let d = governed();
    let r = d.path();
    let with_js = |allow: &str| {
        format!(
            "{{ \"compilerOptions\": {{ {allow}\"paths\": {{ \"@lib/*\": [\"./v2/*\"] }} }}, \
             \"include\": [\"src\"] }}\n"
        )
    };
    write(r, G_LIB, &with_js("\"allowJs\": true, "));
    write(
        r,
        "packages/app/src/helper.js",
        "import { dep } from '@lib/dep';\n\nexport const helper = dep;\n",
    );
    std::fs::remove_file(r.join("packages/app/src/helper.ts")).unwrap();
    rebase(r, "a JavaScript helper");
    let before = records(r);
    write(r, G_LIB, &with_js(""));
    governed_diff(r, s, "allowJs goes", &before, &[G_A], &[]);
}

/// What an edit of a referenced project's config does not select: the project's
/// `paths` and other options only matter to the files it governs (their input),
/// and an edit that leaves its `extends`, `files`, `include`, `exclude` and
/// `allowJs` alone moves no file.
#[test]
fn an_option_of_a_referenced_project_moves_no_file() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    // The project governs the helper: its `paths` are the helper's input.
    let d = governed();
    let r = d.path();
    let before = records(r);
    write(
        r,
        G_LIB,
        "{ \"compilerOptions\": { \"paths\": { \"@lib/*\": [\"./v1/*\"] } }, \"include\": \
         [\"src\"] }\n",
    );
    governed_diff(
        r,
        s,
        "paths of the governing project",
        &before,
        &[G_A],
        &[G_A],
    );
    // The project governs nothing below the config: nothing is selected, and the
    // note says why.
    write(r, G_LIB, &lib_config("\"include\": [\"lib\"]"));
    rebase(r, "the project governs nothing below");
    let before = records(r);
    write(
        r,
        G_LIB,
        "{ \"compilerOptions\": { \"paths\": { \"@lib/*\": [\"./v1/*\"] }, \"strict\": true }, \
         \"include\": [\"lib\"] }\n",
    );
    governed_diff(
        r,
        s,
        "options of a project that governs nothing",
        &before,
        &[],
        &[],
    );
    let err = stats(r);
    assert!(
        err.contains(&format!("tsconfig {G_LIB}: "))
            && err.contains(
                "its `extends`, `files`, `include`, `exclude` and `allowJs` are as at the base, \
                 so it includes the same files and loads as before: nothing is selected through \
                 them"
            ),
        "{err}"
    );
    // An `include` edit that moves no file of the graph: compared, and nothing
    // selected.
    write(r, G_LIB, &lib_config("\"include\": [\"lib\", \"gen\"]"));
    governed_diff(
        r,
        s,
        "include gains a directory without files",
        &before,
        &[],
        &[],
    );
    assert!(
        stats(r).contains("is governed differently than at the base"),
        "{}",
        stats(r)
    );
}

/// The project inherits what it includes: from a config next to it, and from a
/// config of a workspace package linked in `node_modules` (`${configDir}` is
/// the project's directory).
#[test]
fn a_referenced_project_whose_extended_config_changes_what_it_includes() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    let base = "packages/app/lib.base.json";
    write(r, base, "{ \"include\": [\"src\"] }\n");
    write(r, G_LIB, &lib_config("\"extends\": \"./lib.base.json\""));
    rebase(r, "the project inherits include");
    let before = records(r);
    write(r, base, "{ \"include\": [\"lib\"] }\n");
    governed_diff(
        r,
        s,
        "the extended config stops including",
        &before,
        &[G_A],
        &[],
    );
    rebase(r, "the extended config includes lib");
    let before = records(r);
    write(r, base, "{ \"include\": [\"src\"] }\n");
    governed_diff(
        r,
        s,
        "the extended config starts including",
        &before,
        &[G_A],
        &[G_A],
    );
    // An option of the extended config: nothing.
    rebase(r, "the extended config includes src");
    let before = records(r);
    write(
        r,
        base,
        "{ \"compilerOptions\": { \"strict\": true }, \"include\": [\"src\"] }\n",
    );
    governed_diff(
        r,
        s,
        "an option of the extended config",
        &before,
        &[G_A],
        &[G_A],
    );
    // The extended config goes: the project does not load.
    std::fs::remove_file(r.join(base)).unwrap();
    governed_diff(
        r,
        s,
        "the extended config goes",
        &before,
        &[G_A, G_B],
        &[G_A, G_B],
    );

    // Through a workspace package.
    let d = governed();
    let r = d.path();
    let shared = "tooling/tsconfig/lib.json";
    write(
        r,
        "tooling/tsconfig/package.json",
        r#"{"name":"@acme/tsconfig","version":"1.0.0"}"#,
    );
    write(r, shared, "{ \"include\": [\"${configDir}/src\"] }\n");
    let link = r.join("packages/app/node_modules/@acme/tsconfig");
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    turborepo_trace_core::os::symlink("../../../../tooling/tsconfig", link).unwrap();
    write(
        r,
        G_LIB,
        &lib_config("\"extends\": \"@acme/tsconfig/lib.json\""),
    );
    rebase(r, "the project extends a workspace package");
    let before = records(r);
    write(r, shared, "{ \"include\": [\"${configDir}/lib\"] }\n");
    governed_diff(
        r,
        s,
        "the package's config stops including",
        &before,
        &[G_A],
        &[],
    );
    write(
        r,
        shared,
        "{ \"compilerOptions\": { \"strict\": true }, \"include\": [\"${configDir}/src\"] }\n",
    );
    governed_diff(
        r,
        s,
        "an option of the package's config",
        &before,
        &[G_A],
        &[G_A],
    );
}

/// A referenced project that loads at one revision only. A project that does
/// not load is reported on every file that consults it (a transpiler that
/// parses every referenced config fails on it), so its coming to load changes
/// those files' records also when it includes none of them. The other direction
/// was selected before: a project that does not load is an input of the files
/// that consult it.
#[test]
fn a_referenced_project_that_loads_at_one_revision_only() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let all = [G_A, G_B];
    let d = governed();
    let r = d.path();
    let before = records(r);
    write(r, G_LIB, "{ \"include\": [\n");
    governed_diff(r, s, "comes not to parse", &before, &all, &all);
    rebase(r, "the project does not parse");
    let before = records(r);
    write(r, G_LIB, &lib_config("\"include\": [\"src\"]"));
    governed_diff(r, s, "parses again and includes src", &before, &all, &[G_A]);
    write(r, G_LIB, &lib_config("\"include\": [\"lib\"]"));
    governed_diff(
        r,
        s,
        "parses again and includes nothing below",
        &before,
        &all,
        &[],
    );
    let err = stats(r);
    assert!(
        err.contains(&format!(
            "{G_APP} loads with other failures than at the base"
        )),
        "{err}"
    );

    // The project's config goes, and appears.
    let d = governed();
    let r = d.path();
    let before = records(r);
    std::fs::remove_file(r.join(G_LIB)).unwrap();
    governed_diff(r, s, "the project's config goes", &before, &all, &all);
    rebase(r, "the project is not there");
    let before = records(r);
    write(r, G_LIB, &lib_config("\"include\": [\"lib\"]"));
    governed_diff(r, s, "the project's config appears", &before, &all, &[]);
}

/// What the graph of the working tree answered already: the config that lists
/// the project and the nearest config are inputs of every file that consults
/// them.
#[test]
fn the_listing_config_is_an_input_of_the_files_below_it() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let all = [G_A, G_B];
    let d = governed();
    let r = d.path();
    let before = records(r);
    write(r, G_APP, &app_config(",\n  \"references\": []"));
    governed_diff(r, s, "the reference goes", &before, &all, &all);
    rebase(r, "no reference");
    let before = records(r);
    write(r, G_APP, &app_config(LIB_REFERENCE));
    governed_diff(r, s, "the reference comes", &before, &all, &all);
    rebase(r, "the reference is back");
    let before = records(r);
    write(
        r,
        G_APP,
        &app_config(&format!(",\n  \"include\": [\"tests\"]{LIB_REFERENCE}")),
    );
    governed_diff(
        r,
        s,
        "the nearest config changes include",
        &before,
        &all,
        &all,
    );
}

/// Where the base revision cannot be read, every file that consulted the
/// changed project is seeded: without `--since` there is no base, and what a
/// third-party package held before a lockfile bumped it is not in the tree.
#[test]
fn without_the_base_contents_every_file_that_consulted_the_project_is_seeded() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let all = [G_A, G_B];
    // No base revision: the change is named.
    let d = governed();
    let r = d.path();
    selections(r, s, "nothing changed");
    write(r, G_LIB, &lib_config("\"include\": [\"lib\"]"));
    let args = ["affected", "--changed", G_LIB, "--tests", "vitest"];
    assert_eq!(lines(run_env(r, &args, &[]), &args), all);
    assert_eq!(lines(run_env(r, &args, &GOVERNED_OFF), &args), [""; 0]);
    let err = String::from_utf8_lossy(&run_env(r, &[&args[..], &["--stats"]].concat(), &[]).stderr)
        .into_owned();
    assert!(
        err.contains("there is no single base revision to compare with"),
        "{err}"
    );

    // A third-party package the project extends, bumped in the lockfile: its config
    // includes another directory now.
    let d = governed();
    let r = d.path();
    let lock = |v: &str| {
        format!(
            "lockfileVersion: '9.0'\n\nsettings:\n  autoInstallPeers: false\n  \
             excludeLinksFromLockfile: false\n\nimporters:\n\n  .: {{}}\n\n  packages/app:\n    \
             devDependencies:\n      '@vendor/tsconfig':\n        specifier: {v}\n        \
             version: {v}\n\npackages:\n\n  '@vendor/tsconfig@{v}':\n    resolution: {{integrity: \
             sha512-vendor-{v}==}}\n\nsnapshots:\n\n  '@vendor/tsconfig@{v}': {{}}\n"
        )
    };
    let install = |v: &str, include: &str| {
        write(
            r,
            "node_modules/@vendor/tsconfig/package.json",
            &format!("{{\"name\":\"@vendor/tsconfig\",\"version\":\"{v}\"}}"),
        );
        write(
            r,
            "node_modules/@vendor/tsconfig/lib.json",
            &format!("{{ \"include\": [\"${{configDir}}/{include}\"] }}\n"),
        );
        write(r, "pnpm-lock.yaml", &lock(v));
        write(
            r,
            "packages/app/package.json",
            &format!(
                "{{\"name\":\"@acme/app\",\"version\":\"1.0.0\",\"scripts\":{{\"test\":\"vitest \
                 run\"}},\"devDependencies\":{{\"@vendor/tsconfig\":\"{v}\"}}}}"
            ),
        );
    };
    install("1.0.0", "src");
    write(
        r,
        G_LIB,
        &lib_config("\"extends\": \"@vendor/tsconfig/lib.json\""),
    );
    rebase(r, "the project extends a third-party package");
    let before = records(r);
    governed_diff(r, s, "nothing changed", &before, &[], &[]);
    install("1.1.0", "lib");
    // (Without the rule the files are selected all the same: the places the
    // project's load looked for the package at are probes of the files that
    // consult the project, and the lockfile says that the name leads elsewhere
    // now.)
    governed_diff(r, s, "the package is bumped", &before, &all, &all);
    assert!(
        stats(r).contains(
            "tsconfig: the package @vendor/tsconfig changed, and a referenced tsconfig project \
             extends a config of it"
        ),
        "{}",
        stats(r)
    );
}

// ---------------------------------------------------------------------------------------
// Review probes: diffs of the `governed` fixture checked against the
// two-build oracle (`must_select`), whose second build is a graph made from
// nothing. Each prints what must be selected and what is.
//
// Before their fix eight of them failed (a test that must be selected is not):
// `s1`, `s1c`, `s1d` (a config behind a tracked symlink), `s13`, `s13b` (an
// `extends` that a package's `exports` map to another file), `s14` (`extends:
// "#import"`), `s15`, `s15b` (`extends: "./name.base"` without `.json`). The
// `b` and `d` cases involve no project reference: the config that an `extends`
// names is not what `record_tsconfig`
// (`crates/turborepo-trace-lang-js/src/resolve/mod.rs`) records, so it is
// neither an input nor a key observation. `s4`, `s4r`, `s5`, `s7`, `s16` pass.
// ---------------------------------------------------------------------------------------

const G_HELPER: &str = "packages/app/src/helper.ts";

/// Fails when a test that a build at the base and a build now say must be
/// selected is not.
fn review_probe(r: &Path, s: &Path, what: &str, before: &Records) {
    // The oracle's second build is a graph made from nothing, not the refreshed
    // one.
    let cold_dir = s.join(format!("g-oracle-{}", what.replace(' ', "-")));
    let after = records_with(r, &["--graph", cold_dir.to_str().unwrap()]);
    let refreshed = records(r);
    let must = must_select(before, &after, &[G_A, G_B]);
    let got = tests(r);
    let off = tests_env(r, &GOVERNED_OFF);
    let st = stats(r);
    let notes: Vec<&str> = st.lines().filter(|l| l.contains("tsconfig")).collect();
    eprintln!(
        "REVIEW {what}: must={must:?} got={got:?} off={off:?} refreshed_equals_cold={}\n  helper \
         before={:?}\n  helper after(cold)={:?}\n  helper after(refreshed)={:?}\n  \
         notes={notes:#?}",
        refreshed.by_file == after.by_file,
        before.by_file.get(G_HELPER),
        after.by_file.get(G_HELPER),
        refreshed.by_file.get(G_HELPER)
    );
    let graph = s.join(format!("g-review-{}", what.replace(' ', "-")));
    let mut args = TESTS.to_vec();
    args.extend(["--graph", graph.to_str().unwrap()]);
    let cold = lines(run_env(r, &args, &[]), &args);
    assert_eq!(got, cold, "{what}: refreshed and cold");
    assert!(
        refreshed.by_file == after.by_file,
        "{what}: the refreshed graph and a new one hold other records"
    );
    for t in &must {
        assert!(
            got.contains(t),
            "{what}: {t} must be selected (records differ between the base and now); selected: \
             {got:?}"
        );
    }
}

/// The referenced project extends a config through a tracked symlink; the
/// link's target stops including the file.
#[test]
fn review_s1_extended_config_behind_a_tracked_symlink() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    write(
        r,
        "shared/lib.json",
        "{ \"include\": [\"${configDir}/src\"] }\n",
    );
    turborepo_trace_core::os::symlink(
        "../../shared/lib.json",
        r.join("packages/app/lib.base.json"),
    )
    .unwrap();
    write(r, G_LIB, &lib_config("\"extends\": \"./lib.base.json\""));
    rebase(r, "the project extends a symlinked config");
    let before = records(r);
    write(
        r,
        "shared/lib.json",
        "{ \"include\": [\"${configDir}/lib\"] }\n",
    );
    review_probe(r, s, "s1 symlinked extended config", &before);
}

/// The referenced project's config file is itself a tracked symlink.
#[test]
fn review_s1c_project_config_is_a_tracked_symlink() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    let shared = |include: &str| {
        format!(
            "{{ \"compilerOptions\": {{ \"paths\": {{ \"@lib/*\": [\"${{configDir}}/v2/*\"] }} \
             }}, \"include\": [\"${{configDir}}/{include}\"] }}\n"
        )
    };
    write(r, "shared/tsconfig.lib.json", &shared("src"));
    std::fs::remove_file(r.join(G_LIB)).unwrap();
    turborepo_trace_core::os::symlink("../../shared/tsconfig.lib.json", r.join(G_LIB)).unwrap();
    rebase(r, "the project config is a symlink");
    let before = records(r);
    write(r, "shared/tsconfig.lib.json", &shared("lib"));
    review_probe(r, s, "s1c symlinked project config", &before);
}

/// No reference involved: the governing config extends a config through a
/// tracked symlink, and the target's `paths` change.
#[test]
fn review_s1d_governing_config_extends_behind_a_tracked_symlink() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    let paths = |v: &str| {
        format!(
            "{{ \"compilerOptions\": {{ \"paths\": {{ \"@lib/*\": [\"${{configDir}}/{v}/*\"] }} \
             }} }}\n"
        )
    };
    write(r, "shared/app.json", &paths("v1"));
    turborepo_trace_core::os::symlink(
        "../../shared/app.json",
        r.join("packages/app/app.base.json"),
    )
    .unwrap();
    write(r, G_APP, "{ \"extends\": \"./app.base.json\" }\n");
    rebase(r, "the app config extends a symlinked config");
    let before = records(r);
    write(r, "shared/app.json", &paths("v2"));
    review_probe(r, s, "s1d governing extends a symlinked config", &before);
}

/// The project stops including the file and a nearer config appears in the same
/// diff.
#[test]
fn review_s4_a_nearer_config_appears_too() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    let before = records(r);
    write(r, G_LIB, &lib_config("\"include\": [\"lib\"]"));
    write(
        r,
        "packages/app/src/tsconfig.json",
        "{ \"compilerOptions\": { \"paths\": { \"@lib/*\": [\"../v1/*\"] } } }\n",
    );
    review_probe(r, s, "s4 nearer config appears", &before);
}

/// A nearer config governed the file at the base and is deleted: the project
/// governs now.
#[test]
fn review_s4r_a_nearer_config_goes() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    write(
        r,
        "packages/app/src/tsconfig.json",
        "{ \"compilerOptions\": { \"paths\": { \"@lib/*\": [\"../v1/*\"] } } }\n",
    );
    rebase(r, "a nearer config");
    let before = records(r);
    std::fs::remove_file(r.join("packages/app/src/tsconfig.json")).unwrap();
    review_probe(r, s, "s4r nearer config goes", &before);
}

/// The project's own edit passes the field test (`paths`), and the config it
/// extends stops including the file in the same diff.
#[test]
fn review_s5_an_inert_edit_with_an_extended_config_that_moves() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    let base = "packages/app/lib.base.json";
    write(r, base, "{ \"include\": [\"src\"] }\n");
    write(r, G_LIB, &lib_config("\"extends\": \"./lib.base.json\""));
    rebase(r, "the project inherits include");
    let before = records(r);
    write(r, base, "{ \"include\": [\"lib\"] }\n");
    write(
        r,
        G_LIB,
        "{ \"compilerOptions\": { \"paths\": { \"@lib/*\": [\"./v3/*\"] } }, \"extends\": \
         \"./lib.base.json\" }\n",
    );
    review_probe(r, s, "s5 inert edit and extended config moves", &before);
}

/// The graph root is a subdirectory of the git repository (a path prefix at the
/// base).
#[test]
fn review_s7_root_below_the_git_toplevel() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let outer = tempfile::tempdir().unwrap();
    std::fs::remove_dir_all(d.path().join(".git")).unwrap();
    let repo = outer.path().join("repo");
    let o = Command::new("cp")
        .arg("-R")
        .arg(d.path())
        .arg(&repo)
        .output()
        .unwrap();
    assert!(o.status.success());
    git(outer.path(), &["init", "-q"]);
    git(outer.path(), &["add", "-A"]);
    git(outer.path(), &["commit", "-q", "-m", "base"]);
    let r = repo.as_path();
    let before = records(r);
    write(r, G_LIB, &lib_config("\"include\": [\"lib\"]"));
    review_probe(r, s, "s7 root below toplevel", &before);
}

fn link_tooling(r: &Path, manifest: &str) {
    write(r, "tooling/tsconfig/package.json", manifest);
    let link = r.join("packages/app/node_modules/@acme/tsconfig");
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    turborepo_trace_core::os::symlink("../../../../tooling/tsconfig", link).unwrap();
}

/// The referenced project extends a config of a workspace package that the
/// package's `exports` map to another file; that file stops including the file.
#[test]
fn review_s13_extended_config_behind_an_exports_map() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    link_tooling(
        r,
        r#"{"name":"@acme/tsconfig","version":"1.0.0","exports":{"./lib":"./configs/lib.json"}}"#,
    );
    let shared = "tooling/tsconfig/configs/lib.json";
    write(r, shared, "{ \"include\": [\"${configDir}/src\"] }\n");
    write(r, G_LIB, &lib_config("\"extends\": \"@acme/tsconfig/lib\""));
    rebase(r, "the project extends through exports");
    let before = records(r);
    write(r, shared, "{ \"include\": [\"${configDir}/lib\"] }\n");
    review_probe(r, s, "s13 extends through exports", &before);
}

/// The same for the governing config itself (no reference involved): its
/// `paths` come from a config behind an `exports` map.
#[test]
fn review_s13b_governing_config_extends_behind_an_exports_map() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    link_tooling(
        r,
        r#"{"name":"@acme/tsconfig","version":"1.0.0","exports":{"./app":"./configs/app.json"}}"#,
    );
    let shared = "tooling/tsconfig/configs/app.json";
    let paths = |v: &str| {
        format!(
            "{{ \"compilerOptions\": {{ \"paths\": {{ \"@lib/*\": [\"${{configDir}}/{v}/*\"] }} \
             }} }}\n"
        )
    };
    write(r, shared, &paths("v1"));
    write(r, G_APP, "{ \"extends\": \"@acme/tsconfig/app\" }\n");
    rebase(r, "the app config extends through exports");
    let before = records(r);
    write(r, shared, &paths("v2"));
    review_probe(r, s, "s13b governing extends through exports", &before);
}

/// The referenced project extends `./lib.base` (no `.json`): the resolver loads
/// `lib.base.json`.
#[test]
fn review_s15_extends_without_extension_of_a_dotted_name() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    let base = "packages/app/lib.base.json";
    write(r, base, "{ \"include\": [\"src\"] }\n");
    write(r, G_LIB, &lib_config("\"extends\": \"./lib.base\""));
    rebase(r, "the project inherits include, no extension");
    let before = records(r);
    write(r, base, "{ \"include\": [\"lib\"] }\n");
    review_probe(r, s, "s15 extends without extension", &before);
}

/// The same for the governing config itself: `extends: "./app.base"`, whose
/// `paths` change.
#[test]
fn review_s15b_governing_config_extends_without_extension() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    let base = "packages/app/app.base.json";
    let paths = |v: &str| {
        format!("{{ \"compilerOptions\": {{ \"paths\": {{ \"@lib/*\": [\"./{v}/*\"] }} }} }}\n")
    };
    write(r, base, &paths("v1"));
    write(r, G_APP, "{ \"extends\": \"./app.base\" }\n");
    rebase(r, "the app config extends without extension");
    let before = records(r);
    write(r, base, &paths("v2"));
    review_probe(r, s, "s15b governing extends without extension", &before);
}

/// The referenced project extends a config of a linked workspace package; the
/// config is deleted (the project stops loading), and comes back (it loads
/// again).
#[test]
fn review_s16_extended_config_of_a_workspace_package_goes_and_comes() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    link_tooling(r, r#"{"name":"@acme/tsconfig","version":"1.0.0"}"#);
    let shared = "tooling/tsconfig/lib.json";
    write(r, shared, "{ \"include\": [\"${configDir}/lib\"] }\n");
    write(
        r,
        G_LIB,
        &lib_config("\"extends\": \"@acme/tsconfig/lib.json\""),
    );
    rebase(r, "the project extends a workspace package");
    let before = records(r);
    std::fs::remove_file(r.join(shared)).unwrap();
    review_probe(r, s, "s16 package config goes", &before);
    rebase(r, "the package config is gone");
    let before = records(r);
    write(r, shared, "{ \"include\": [\"${configDir}/lib\"] }\n");
    review_probe(r, s, "s16r package config comes", &before);
}

/// The referenced project extends `#lib-config`, a subpath import of the
/// package.
#[test]
fn review_s14_extends_a_subpath_import() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    write(
        r,
        "packages/app/package.json",
        r##"{"name":"@acme/app","version":"1.0.0","scripts":{"test":"vitest run"},"imports":{"#lib-config":"./lib.base.json"}}"##,
    );
    let base = "packages/app/lib.base.json";
    write(r, base, "{ \"include\": [\"src\"] }\n");
    write(r, G_LIB, &lib_config("\"extends\": \"#lib-config\""));
    rebase(r, "the project extends a subpath import");
    let before = records(r);
    write(r, base, "{ \"include\": [\"lib\"] }\n");
    review_probe(r, s, "s14 extends a subpath import", &before);
}

// ---------------------------------------------------------------------------------------
// What a config's load read is what the file records (m-tsrec): the cases the
// review did not probe, against the same oracle. The record is what the loader
// asked of the file system
// (`crates/turborepo-trace-lang-js/src/resolve/tsconfig.rs`), so none
// of these has a rule of its own.
// ---------------------------------------------------------------------------------------

/// A config whose only content is a `paths` entry sending `@lib/*` to `target`.
/// As the app's own config (without the reference of the fixture) it governs
/// every file of the package, the tests included: an edit of what it loads
/// selects both tests, and the oracle says which of them must be.
fn paths_to(target: &str) -> String {
    format!("{{ \"compilerOptions\": {{ \"paths\": {{ \"@lib/*\": [\"{target}/*\"] }} }} }}\n")
}

/// The governing config extends a config of its own package by the package's
/// name: the package's `exports` name the file.
#[test]
fn loaded_extends_by_the_packages_own_name() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    write(
        r,
        "packages/app/package.json",
        r#"{"name":"@acme/app","version":"1.0.0","scripts":{"test":"vitest run"},"exports":{"./base":"./configs/base.json"}}"#,
    );
    let base = "packages/app/configs/base.json";
    write(r, base, &paths_to("../v1"));
    write(r, G_APP, "{ \"extends\": \"@acme/app/base\" }\n");
    rebase(r, "the app config extends its own package");
    let before = records(r);
    assert!(
        before.by_file[G_HELPER]
            .iter()
            .any(|e| e.ends_with("packages/app/v1/dep.ts")),
        "the config loads through the package's own name: {:?}",
        before.by_file[G_HELPER]
    );
    write(r, base, &paths_to("../v2"));
    review_probe(r, s, "own name", &before);
    assert_eq!(tests(r), [G_A, G_B]);
}

/// `"extends": "../../tsconfig.base"` names `tsconfig.base.json`. The
/// `tsconfig.json` next to it (what the name with its "extension" replaced
/// would be) is nothing the load reads: an edit of it selects nothing, an edit
/// of the base does.
#[test]
fn loaded_extends_without_extension_next_to_a_tsconfig_json() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    write(r, "tsconfig.base.json", &paths_to("./packages/app/v1"));
    write(r, "tsconfig.json", &paths_to("./packages/app/v1"));
    write(r, G_APP, "{ \"extends\": \"../../tsconfig.base\" }\n");
    rebase(r, "the app config extends ../../tsconfig.base");
    let before = records(r);
    write(r, "tsconfig.json", &paths_to("./packages/app/v2"));
    review_probe(r, s, "the neighbour of the base", &before);
    assert_eq!(tests(r), Vec::<String>::new(), "the neighbour is not read");
    rebase(r, "the neighbour changed");
    let before = records(r);
    write(r, "tsconfig.base.json", &paths_to("./packages/app/v2"));
    review_probe(r, s, "the base without extension", &before);
    assert_eq!(tests(r), [G_A, G_B]);
}

/// An `extends` array: every entry is loaded, a relative one without extension
/// and one that a package's `exports` map.
#[test]
fn loaded_extends_array() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    link_tooling(
        r,
        r#"{"name":"@acme/tsconfig","version":"1.0.0","exports":{"./app":"./configs/app.json"}}"#,
    );
    let first = "packages/app/one.base.json";
    let second = "tooling/tsconfig/configs/app.json";
    write(r, first, &paths_to("./v1"));
    write(r, second, "{ \"compilerOptions\": { \"strict\": true } }\n");
    write(
        r,
        G_APP,
        "{ \"extends\": [\"./one.base\", \"@acme/tsconfig/app\"] }\n",
    );
    rebase(r, "the app config extends two configs");
    let before = records(r);
    write(r, first, &paths_to("./v2"));
    review_probe(r, s, "first of an extends array", &before);
    assert_eq!(tests(r), [G_A, G_B]);
    rebase(r, "the first changed");
    let before = records(r);
    // The later entry wins.
    write(r, second, &paths_to("${configDir}/v1"));
    review_probe(r, s, "second of an extends array", &before);
    assert_eq!(tests(r), [G_A, G_B]);
}

/// The extended config names its paths by `${configDir}`.
#[test]
fn loaded_extends_with_config_dir() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    write(r, "shared/app.json", &paths_to("${configDir}/v1"));
    write(r, G_APP, "{ \"extends\": \"../../shared/app.json\" }\n");
    rebase(r, "the app config extends a config with ${configDir}");
    let before = records(r);
    write(r, "shared/app.json", &paths_to("${configDir}/v2"));
    review_probe(r, s, "configDir", &before);
    assert_eq!(tests(r), [G_A, G_B]);
}

/// A reference `./lib` that names `lib.json` at the base and the directory
/// `lib` (its `tsconfig.json`) now: the project that included the file is no
/// longer the one loaded.
#[test]
fn loaded_reference_that_becomes_a_directory() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    std::fs::remove_file(r.join(G_LIB)).unwrap();
    write(
        r,
        "packages/app/lib.json",
        &lib_config("\"include\": [\"src\"]"),
    );
    write(
        r,
        G_APP,
        &app_config(",\n  \"references\": [{ \"path\": \"./lib\" }]"),
    );
    rebase(r, "the reference names lib.json");
    let before = records(r);
    assert!(
        before.by_file[G_HELPER]
            .iter()
            .any(|e| e.ends_with("packages/app/v2/dep.ts")),
        "the project governs the helper: {:?}",
        before.by_file[G_HELPER]
    );
    write(
        r,
        "packages/app/lib/tsconfig.json",
        "{ \"include\": [\"nothing\"] }\n",
    );
    review_probe(r, s, "reference becomes a directory", &before);
    assert_eq!(tests(r), [G_A, G_B]);
}

/// The extended config is behind a tracked link that the diff makes lead to
/// another file with the same contents: the config's own paths are relative to
/// where it really lies.
#[test]
fn loaded_link_that_leads_elsewhere() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    for at in ["x", "y"] {
        write(r, &format!("shared/{at}/app.json"), &paths_to("./lib"));
        write(
            r,
            &format!("shared/{at}/lib/dep.ts"),
            &format!("export const dep = '{at}';\n"),
        );
    }
    let link = r.join("packages/app/app.base.json");
    turborepo_trace_core::os::symlink("../../shared/x/app.json", &link).unwrap();
    write(r, G_APP, "{ \"extends\": \"./app.base.json\" }\n");
    rebase(r, "the app config extends a link");
    let before = records(r);
    assert!(
        before.by_file[G_HELPER]
            .iter()
            .any(|e| e.ends_with("shared/x/lib/dep.ts")),
        "the config loads through the link: {:?}",
        before.by_file[G_HELPER]
    );
    std::fs::remove_file(&link).unwrap();
    turborepo_trace_core::os::symlink("../../shared/y/app.json", &link).unwrap();
    review_probe(r, s, "link leads elsewhere", &before);
    assert_eq!(tests(r), [G_A, G_B]);
}

/// The extended config lies in a directory that is a tracked link.
#[test]
fn loaded_extends_through_a_linked_directory() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    write(r, "shared/configs/app.json", &paths_to("${configDir}/v1"));
    turborepo_trace_core::os::symlink("../../shared/configs", r.join("packages/app/configs"))
        .unwrap();
    write(r, G_APP, "{ \"extends\": \"./configs/app.json\" }\n");
    rebase(r, "the app config extends through a linked directory");
    let before = records(r);
    write(r, "shared/configs/app.json", &paths_to("${configDir}/v2"));
    review_probe(r, s, "linked directory", &before);
    assert_eq!(tests(r), [G_A, G_B]);
}

/// How the engine treats a tracked link that an import goes through (the rule
/// the config records follow: the real file, and the link): the link comes to
/// lead elsewhere, as a linked directory and as a linked file.
#[test]
fn an_import_through_a_link_that_leads_elsewhere() {
    for file_link in [false, true] {
        let scratch = tempfile::tempdir().unwrap();
        let s = scratch.path();
        let d = governed();
        let r = d.path();
        write(r, "packages/app/other2/o.ts", "export const o = 2;\n");
        let (link, to, then, import) = if file_link {
            (
                "packages/app/linked.ts",
                "other/o.ts",
                "other2/o.ts",
                "../linked",
            )
        } else {
            ("packages/app/linked", "other", "other2", "../linked/o")
        };
        turborepo_trace_core::os::symlink(to, r.join(link)).unwrap();
        write(
            r,
            G_B,
            &format!(
                "import {{ expect, test }} from 'vitest';\nimport {{ o }} from \
                 '{import}';\n\ntest('b', () => {{\n  expect(o).toBe(1);\n}});\n"
            ),
        );
        rebase(r, "a test imports through a link");
        let before = records(r);
        std::fs::remove_file(r.join(link)).unwrap();
        turborepo_trace_core::os::symlink(then, r.join(link)).unwrap();
        review_probe(
            r,
            s,
            if file_link {
                "an import through a linked file that leads elsewhere"
            } else {
                "an import through a linked directory that leads elsewhere"
            },
            &before,
        );
    }
}

/// The cost of a diff that edits many referenced projects (not a check: it
/// prints the time of the query's config rules, `TURBO_TRACE_DEBUG_TIMING`).
/// The base's contents of the changed configs are read with one `git` process,
/// not one per config.
#[test]
#[ignore = "a measurement: cargo test -p turborepo-trace --test affected_tsconfig cost_ -- \
            --ignored --nocapture"]
fn cost_of_a_diff_that_edits_many_referenced_projects() {
    const N: usize = 150;
    let d = governed();
    let r = d.path();
    let project = |i: usize, include: &str| {
        write(
            r,
            &format!("packages/app/tsconfig.p{i}.json"),
            &format!("{{ \"include\": [\"{include}{i}\"] }}\n"),
        );
    };
    let references: Vec<String> = (0..N)
        .map(|i| format!("{{ \"path\": \"./tsconfig.p{i}.json\" }}"))
        .collect();
    write(
        r,
        G_APP,
        &app_config(&format!(",\n  \"references\": [{}]", references.join(", "))),
    );
    for i in 0..N {
        project(i, "p");
    }
    rebase(r, "many projects");
    tests(r);
    for i in 0..N {
        project(i, "q");
    }
    let mut times = Vec::new();
    for _ in 0..5 {
        let start = std::time::Instant::now();
        let o = run_env(r, &TESTS, &[("TURBO_TRACE_DEBUG_TIMING", "1")]);
        let all = start.elapsed().as_secs_f64() * 1e3;
        let err = String::from_utf8_lossy(&o.stderr).into_owned();
        // `timing <since start> ms (+<since the mark before>) <label>`
        let since = |label: &str| -> f64 {
            err.lines()
                .find(|l| l.ends_with(label))
                .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
                .unwrap_or(f64::NAN)
        };
        times.push((
            since("affected: config diffs") - since("affected: workflows"),
            all,
        ));
    }
    times.sort_by(|a, b| a.0.total_cmp(&b.0));
    eprintln!(
        "COST {N} modified referenced projects: config rules median {:.1} ms (min {:.1}, max \
         {:.1}); whole query median {:.1} ms",
        times[2].0, times[0].0, times[4].0, times[2].1
    );
}

// ---------------------------------------------------------------------------------------
// Review probes: each is one diff
// against the oracle of `review_probe` (a build at the base and a build now say
// what must be selected; the refreshed graph holds the records of a new one).
// ---------------------------------------------------------------------------------------

/// The fixture with the project's `include` in a shared config that the project
/// reaches as `spec`: at `x` the project does not include `src` (the app's
/// config governs the helper: `v1`), at `y` it does (`v2`).
fn shared_includes(r: &Path, spec: &str) {
    write(
        r,
        "shared/x/lib.json",
        "{ \"include\": [\"${configDir}/lib\"] }\n",
    );
    write(
        r,
        "shared/y/lib.json",
        "{ \"include\": [\"${configDir}/src\"] }\n",
    );
    write(r, G_LIB, &lib_config(&format!("\"extends\": \"{spec}\"")));
}

fn helper_resolves_to(rec: &Records, v: &str) -> bool {
    rec.by_file[G_HELPER]
        .iter()
        .any(|e| e.ends_with(&format!("packages/app/{v}/dep.ts")))
}

/// T1: the referenced project (which does not govern the helper: a key
/// observation) extends a config behind a tracked link to a file; the diff
/// makes the link lead to a config that includes the helper.
#[test]
fn t1_project_extends_a_linked_file_that_leads_elsewhere() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    shared_includes(r, "./lib.base.json");
    let link = r.join("packages/app/lib.base.json");
    turborepo_trace_core::os::symlink("../../shared/x/lib.json", &link).unwrap();
    rebase(r, "the project extends a linked config");
    let before = records(r);
    assert!(
        helper_resolves_to(&before, "v1"),
        "{:?}",
        before.by_file[G_HELPER]
    );
    std::fs::remove_file(&link).unwrap();
    turborepo_trace_core::os::symlink("../../shared/y/lib.json", &link).unwrap();
    review_probe(r, s, "t1 linked file leads elsewhere", &before);
}

/// T2: the same through a tracked link to a directory.
#[test]
fn t2_project_extends_through_a_linked_directory_that_leads_elsewhere() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    shared_includes(r, "./configs/lib.json");
    let link = r.join("packages/app/configs");
    turborepo_trace_core::os::symlink("../../shared/x", &link).unwrap();
    rebase(r, "the project extends through a linked directory");
    let before = records(r);
    assert!(
        helper_resolves_to(&before, "v1"),
        "{:?}",
        before.by_file[G_HELPER]
    );
    std::fs::remove_file(&link).unwrap();
    turborepo_trace_core::os::symlink("../../shared/y", &link).unwrap();
    review_probe(r, s, "t2 linked directory leads elsewhere", &before);
}

/// T2r: the other direction (the project governs the helper at the base: an
/// input).
#[test]
fn t2r_project_extends_through_a_linked_directory_that_leads_back() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    shared_includes(r, "./configs/lib.json");
    let link = r.join("packages/app/configs");
    turborepo_trace_core::os::symlink("../../shared/y", &link).unwrap();
    rebase(r, "the project extends through a linked directory");
    let before = records(r);
    assert!(
        helper_resolves_to(&before, "v2"),
        "{:?}",
        before.by_file[G_HELPER]
    );
    std::fs::remove_file(&link).unwrap();
    turborepo_trace_core::os::symlink("../../shared/x", &link).unwrap();
    review_probe(r, s, "t2r linked directory leads back", &before);
}

/// T2a: the linked directory is not there at the base (the project does not
/// load, and every file below the referencing config carries that); the diff
/// adds the link, and the project loads without including the helper.
#[test]
fn t2a_the_linked_directory_appears() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    shared_includes(r, "./configs/lib.json");
    rebase(
        r,
        "the project extends through a directory that is not there",
    );
    let before = records(r);
    turborepo_trace_core::os::symlink("../../shared/x", r.join("packages/app/configs")).unwrap();
    // Staged, as a commit would name it (`A packages/app/configs`).
    git(r, &["add", "-A"]);
    review_probe(r, s, "t2a linked directory appears", &before);
}

/// T2u: T2a with the link left untracked (a working tree before `git add`).
#[test]
fn t2u_the_linked_directory_appears_untracked() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    shared_includes(r, "./configs/lib.json");
    rebase(
        r,
        "the project extends through a directory that is not there",
    );
    let before = records(r);
    turborepo_trace_core::os::symlink("../../shared/x", r.join("packages/app/configs")).unwrap();
    review_probe(r, s, "t2u linked directory appears untracked", &before);
}

/// T2d: the project governs the helper through the linked directory at the
/// base; the diff deletes the link (the project does not load now).
#[test]
fn t2d_the_linked_directory_goes() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    shared_includes(r, "./configs/lib.json");
    let link = r.join("packages/app/configs");
    turborepo_trace_core::os::symlink("../../shared/y", &link).unwrap();
    rebase(r, "the project extends through a linked directory");
    let before = records(r);
    assert!(
        helper_resolves_to(&before, "v2"),
        "{:?}",
        before.by_file[G_HELPER]
    );
    std::fs::remove_file(&link).unwrap();
    review_probe(r, s, "t2d linked directory goes", &before);
}

/// T2f: T2a and T2d with a link to a file: it appears, and it goes.
#[test]
fn t2f_the_linked_file_appears_and_goes() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    shared_includes(r, "./lib.base.json");
    rebase(r, "the project extends a file that is not there");
    let before = records(r);
    let link = r.join("packages/app/lib.base.json");
    turborepo_trace_core::os::symlink("../../shared/x/lib.json", &link).unwrap();
    git(r, &["add", "-A"]);
    review_probe(r, s, "t2f linked file appears", &before);
    rebase(r, "the link is there");
    let before = records(r);
    std::fs::remove_file(&link).unwrap();
    review_probe(r, s, "t2f linked file goes", &before);
}

/// T3: `extends: "./base"` where `base` is a directory without a
/// `tsconfig.json` (the config does not load) next to `base.json`; the
/// directory goes, and `base.json` loads.
#[test]
fn t3_extends_a_directory_that_goes() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    write(r, "packages/app/base/readme.md", "nothing\n");
    write(r, "packages/app/base.json", &paths_to("./v2"));
    write(r, G_APP, "{ \"extends\": \"./base\" }\n");
    rebase(r, "the app config extends a directory without a config");
    let before = records(r);
    std::fs::remove_dir_all(r.join("packages/app/base")).unwrap();
    review_probe(r, s, "t3 directory goes", &before);
    assert!(helper_resolves_to(&records(r), "v2"));
}

/// T4: the governing config extends through a tracked link into a workspace
/// package's `node_modules` link (a link behind a link); the package's config
/// changes.
#[test]
fn t4_extends_a_link_into_a_package_link() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    link_tooling(r, r#"{"name":"@acme/tsconfig","version":"1.0.0"}"#);
    write(r, "tooling/tsconfig/app.json", &paths_to("${configDir}/v1"));
    turborepo_trace_core::os::symlink("node_modules/@acme/tsconfig", r.join("packages/app/cfg"))
        .unwrap();
    write(r, G_APP, "{ \"extends\": \"./cfg/app.json\" }\n");
    rebase(r, "the app config extends a link into a package link");
    let before = records(r);
    assert!(
        helper_resolves_to(&before, "v1"),
        "{:?}",
        before.by_file[G_HELPER]
    );
    write(r, "tooling/tsconfig/app.json", &paths_to("${configDir}/v2"));
    review_probe(r, s, "t4 link into a package link", &before);
    assert_eq!(tests(r), [G_A, G_B]);
}

/// T3b: a reference to a directory without a config (the project does not
/// load); the directory goes (first diff), then `<path>.json` appears and
/// includes the helper.
#[test]
fn t3b_reference_to_a_directory_that_goes_then_a_json_appears() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    std::fs::remove_file(r.join(G_LIB)).unwrap();
    write(r, "packages/app/proj/readme.md", "nothing\n");
    write(
        r,
        G_APP,
        &app_config(",\n  \"references\": [{ \"path\": \"./proj\" }]"),
    );
    rebase(r, "the reference names a directory without a config");
    let before = records(r);
    std::fs::remove_dir_all(r.join("packages/app/proj")).unwrap();
    // Not asserted here (step 1 alone:
    // `t3c_reference_to_a_directory_that_goes`): the repository's graph is
    // refreshed.
    let _ = (records(r), &before);
    rebase(r, "the directory is gone");
    let before = records(r);
    write(
        r,
        "packages/app/proj.json",
        &lib_config("\"include\": [\"src\"]"),
    );
    review_probe(r, s, "t3b json appears", &before);
    assert!(helper_resolves_to(&records(r), "v2"));
}

/// T3c: step 1 of T3b alone: the directory a reference names goes. The records
/// of a new graph and of the refreshed one are the same, their selections must
/// be too.
#[test]
fn t3c_reference_to_a_directory_that_goes() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    std::fs::remove_file(r.join(G_LIB)).unwrap();
    write(r, "packages/app/proj/readme.md", "nothing\n");
    write(
        r,
        G_APP,
        &app_config(",\n  \"references\": [{ \"path\": \"./proj\" }]"),
    );
    rebase(r, "the reference names a directory without a config");
    let before = records(r);
    std::fs::remove_dir_all(r.join("packages/app/proj")).unwrap();
    review_probe(r, s, "t3c directory goes", &before);
}

/// T8: the governing config extends `@acme/tsconfig/app.json`, which a
/// `node_modules` link answers; then the package's own manifest takes that name
/// and maps the request to a local config (a self-reference wins over
/// `node_modules`).
#[test]
fn t8_a_self_reference_appears() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    link_tooling(r, r#"{"name":"@acme/tsconfig","version":"1.0.0"}"#);
    write(r, "tooling/tsconfig/app.json", &paths_to("${configDir}/v1"));
    write(r, "packages/app/local.json", &paths_to("./v2"));
    write(r, G_APP, "{ \"extends\": \"@acme/tsconfig/app.json\" }\n");
    rebase(r, "the app config extends a package");
    let before = records(r);
    assert!(
        helper_resolves_to(&before, "v1"),
        "{:?}",
        before.by_file[G_HELPER]
    );
    write(
        r,
        "packages/app/package.json",
        r#"{"name":"@acme/tsconfig","version":"1.0.0","scripts":{"test":"vitest run"},"exports":{"./app.json":"./local.json"}}"#,
    );
    review_probe(r, s, "t8 self reference appears", &before);
    eprintln!(
        "REV51 T8 helper now: {:?}",
        records(r).by_file.get(G_HELPER)
    );
}

/// T9: an ordinary import through a tracked link to a directory; a file that
/// takes precedence appears in the directory the link leads to (the diff names
/// its real path).
#[test]
fn t9_a_file_appears_behind_a_link_an_import_goes_through() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    write(r, "packages/app/real/o.js", "export const o = 1;\n");
    turborepo_trace_core::os::symlink("real", r.join("packages/app/linked")).unwrap();
    write(
        r,
        G_B,
        "import { expect, test } from 'vitest';\nimport { o } from '../linked/o';\n\ntest('b', () \
         => {\n  expect(o).toBe(1);\n});\n",
    );
    rebase(r, "a test imports through a link");
    let before = records(r);
    assert!(
        before.by_file[G_B]
            .iter()
            .any(|e| e.ends_with("packages/app/real/o.js")),
        "{:?}",
        before.by_file[G_B]
    );
    write(r, "packages/app/real/o.ts", "export const o = 2;\n");
    review_probe(r, s, "t9 a file appears behind a link", &before);
}

/// T12: a `references`-only edit adds a project whose `extends` is `./base`,
/// where both `base.json` (its `include` stays inside the project) and the
/// directory `base` with a `tsconfig.json` (its `include` reaches the app's
/// `src`) exist. The loader takes the directory; the project includes the
/// helper and governs it now.
#[test]
fn t12_a_reference_whose_extends_names_a_directory_next_to_a_json() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    std::fs::remove_file(r.join(G_LIB)).unwrap();
    write(r, G_APP, &app_config(""));
    write(
        r,
        "packages/libx/tsconfig.json",
        "{ \"extends\": \"./base\", \"compilerOptions\": { \"paths\": { \"@lib/*\": \
         [\"../app/v2/*\"] } } }\n",
    );
    write(r, "packages/libx/base.json", "{ \"include\": [\"src\"] }\n");
    write(
        r,
        "packages/libx/base/tsconfig.json",
        // (The loader reads an inherited `include` relative to the project.)
        "{ \"include\": [\"../app/src\"] }\n",
    );
    rebase(r, "a project that nothing references yet");
    let before = records(r);
    assert!(
        helper_resolves_to(&before, "v1"),
        "{:?}",
        before.by_file[G_HELPER]
    );
    write(
        r,
        G_APP,
        &app_config(",\n  \"references\": [{ \"path\": \"../libx\" }]"),
    );
    review_probe(r, s, "t12 reference added", &before);
}

/// T13: an ordinary import through a tracked link to a directory; the link is
/// deleted (the import no longer resolves), and in a second diff it comes back.
#[test]
fn t13_a_link_an_import_goes_through_goes_and_comes() {
    for file_link in [false, true] {
        let scratch = tempfile::tempdir().unwrap();
        let s = scratch.path();
        let d = governed();
        let r = d.path();
        let (link, to, import) = if file_link {
            ("packages/app/linked.ts", "other/o.ts", "../linked")
        } else {
            ("packages/app/linked", "other", "../linked/o")
        };
        turborepo_trace_core::os::symlink(to, r.join(link)).unwrap();
        write(
            r,
            G_B,
            &format!(
                "import {{ expect, test }} from 'vitest';\nimport {{ o }} from \
                 '{import}';\n\ntest('b', () => {{\n  expect(o).toBe(1);\n}});\n"
            ),
        );
        rebase(r, "a test imports through a link");
        let before = records(r);
        std::fs::remove_file(r.join(link)).unwrap();
        review_probe(
            r,
            s,
            &format!("t13 link goes (file link: {file_link})"),
            &before,
        );
        rebase(r, "the link is gone");
        let before = records(r);
        turborepo_trace_core::os::symlink(to, r.join(link)).unwrap();
        git(r, &["add", "-A"]);
        review_probe(
            r,
            s,
            &format!("t13 link comes (file link: {file_link})"),
            &before,
        );
    }
}

/// T7: an import that goes through a workspace package's `node_modules` link
/// and then through a tracked link inside the package; the tracked link comes
/// to lead elsewhere.
#[test]
fn t7_an_import_through_a_package_link_and_a_tracked_link() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    write(
        r,
        "packages/ui/package.json",
        r#"{"name":"@acme/ui","version":"1.0.0"}"#,
    );
    write(r, "packages/ui/other/o.ts", "export const o = 1;\n");
    write(r, "packages/ui/other2/o.ts", "export const o = 2;\n");
    turborepo_trace_core::os::symlink("other", r.join("packages/ui/linked")).unwrap();
    let link = r.join("packages/app/node_modules/@acme/ui");
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    turborepo_trace_core::os::symlink("../../../ui", link).unwrap();
    write(
        r,
        G_B,
        "import { expect, test } from 'vitest';\nimport { o } from \
         '@acme/ui/linked/o';\n\ntest('b', () => {\n  expect(o).toBe(1);\n});\n",
    );
    rebase(
        r,
        "a test imports through a package link and a tracked link",
    );
    let before = records(r);
    assert!(
        before.by_file[G_B]
            .iter()
            .any(|e| e.ends_with("packages/ui/other/o.ts")),
        "{:?}",
        before.by_file[G_B]
    );
    std::fs::remove_file(r.join("packages/ui/linked")).unwrap();
    turborepo_trace_core::os::symlink("other2", r.join("packages/ui/linked")).unwrap();
    review_probe(r, s, "t7 package link then tracked link", &before);
}

// ---------------------------------------------------------------------------------------
// The other direction of the review's directory probes: a directory
// that the diff makes at a place a config load looked at. No file of the diff
// is at a path that was looked for or above one: the directory is what changed,
// and the loader looks inside it instead of next to it.
// ---------------------------------------------------------------------------------------

/// `extends: "./base"` loads `base.json`; the diff adds a file to a new
/// directory `base`, which holds no config: the loader asks for
/// `base/tsconfig.json` and the config no longer loads.
#[test]
fn fix51_extends_a_directory_that_appears() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    write(r, "packages/app/base.json", &paths_to("./v2"));
    write(r, G_APP, "{ \"extends\": \"./base\" }\n");
    rebase(r, "the app config extends a name without .json");
    let before = records(r);
    assert!(
        helper_resolves_to(&before, "v2"),
        "{:?}",
        before.by_file[G_HELPER]
    );
    write(r, "packages/app/base/readme.md", "nothing\n");
    review_probe(r, s, "fix51 directory appears", &before);
    assert_eq!(tests(r), [G_A, G_B]);
    // A second file in the directory, which was there at the base now: nothing
    // looked for it, and the directory is no change.
    rebase(r, "the directory is there");
    write(r, "packages/app/base/more.md", "nothing\n");
    assert_eq!(tests(r), [] as [&str; 0]);
}

/// The same for a project reference: `./proj` names `proj.json` (which includes
/// the helper); the diff makes a directory `proj` without a config, and the
/// project does not load.
#[test]
fn fix51_reference_to_a_directory_that_appears() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    std::fs::remove_file(r.join(G_LIB)).unwrap();
    write(
        r,
        "packages/app/proj.json",
        &lib_config("\"include\": [\"src\"]"),
    );
    write(
        r,
        G_APP,
        &app_config(",\n  \"references\": [{ \"path\": \"./proj\" }]"),
    );
    rebase(r, "the reference names a project without .json");
    let before = records(r);
    assert!(
        helper_resolves_to(&before, "v2"),
        "{:?}",
        before.by_file[G_HELPER]
    );
    write(r, "packages/app/proj/readme.md", "nothing\n");
    review_probe(r, s, "fix51 referenced directory appears", &before);
    assert_eq!(tests(r), [G_A, G_B]);
}

/// The directory is made by an untracked file, and the change is given as paths
/// (`--changed`, no base revision to ask): a directory in which everything is a
/// changed path is taken to be new.
#[test]
fn fix51_extends_a_directory_that_appears_without_a_base() {
    let d = governed();
    let r = d.path();
    write(r, "packages/app/base.json", &paths_to("./v2"));
    write(r, G_APP, "{ \"extends\": \"./base\" }\n");
    rebase(r, "the app config extends a name without .json");
    let _ = records(r);
    write(r, "packages/app/base/readme.md", "nothing\n");
    let args = [
        "affected",
        "--changed",
        "packages/app/base/readme.md",
        "--on-uncertain",
        "ignore",
        "--tests",
        "vitest",
    ];
    assert_eq!(lines(run_env(r, &args, &[]), &args), [G_A, G_B]);
}

// ---------------------------------------------------------------------------------------
// Review probes (directories and links at
// looked-at places). Each fails when a test that a build at the base and a
// build now say must be selected is not, or when the refreshed graph is not the
// cold one.
// ---------------------------------------------------------------------------------------

/// The governing config extends a config of a workspace package through the
/// package's `node_modules` link; that config extends `./base` (`base.json`).
fn package_chain(r: &Path) {
    link_tooling(r, r#"{"name":"@acme/tsconfig","version":"1.0.0"}"#);
    write(
        r,
        "tooling/tsconfig/chain.json",
        "{ \"extends\": \"./base\" }\n",
    );
    write(
        r,
        "tooling/tsconfig/base.json",
        &paths_to("../../packages/app/v2"),
    );
    write(r, G_APP, "{ \"extends\": \"@acme/tsconfig/chain.json\" }\n");
}

/// N1: a directory `base` (without a config) appears next to `base.json` in the
/// workspace package: the loader asks for `base/tsconfig.json`, and the app's
/// config no longer loads. The loader asked about the directory by its
/// `node_modules` path.
#[test]
fn n1_a_directory_appears_next_to_the_base_of_a_package_config() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    package_chain(r);
    rebase(
        r,
        "the app config extends a package config that extends ./base",
    );
    let before = records(r);
    assert!(
        helper_resolves_to(&before, "v2"),
        "{:?}",
        before.by_file[G_HELPER]
    );
    write(r, "tooling/tsconfig/base/readme.md", "nothing\n");
    review_probe(r, s, "n1 package directory appears", &before);
}

/// N1r: the same directory goes.
#[test]
fn n1r_a_directory_goes_next_to_the_base_of_a_package_config() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    package_chain(r);
    write(r, "tooling/tsconfig/base/readme.md", "nothing\n");
    rebase(r, "a directory next to the base of the package config");
    let before = records(r);
    std::fs::remove_dir_all(r.join("tooling/tsconfig/base")).unwrap();
    review_probe(r, s, "n1r package directory goes", &before);
    assert!(helper_resolves_to(&records(r), "v2"));
}

/// N2: the directory an `extends` names comes with a first file three levels
/// down, and goes with it (git names the file, not the three directories).
#[test]
fn n2_a_directory_comes_and_goes_with_a_file_three_levels_down() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    write(r, "packages/app/base.json", &paths_to("./v2"));
    write(r, G_APP, "{ \"extends\": \"./base\" }\n");
    rebase(r, "the app config extends a name without .json");
    let before = records(r);
    assert!(helper_resolves_to(&before, "v2"));
    write(r, "packages/app/base/x/y/readme.md", "nothing\n");
    review_probe(r, s, "n2 deep directory appears", &before);
    rebase(r, "the directory is there");
    let before = records(r);
    std::fs::remove_dir_all(r.join("packages/app/base")).unwrap();
    review_probe(r, s, "n2 deep directory goes", &before);
    assert!(helper_resolves_to(&records(r), "v2"));
}

/// N6: the governing config extends `./configs/base` through a tracked link to
/// a directory (`base.json` behind it); a directory `base` appears at the real
/// place.
#[test]
fn n6_a_directory_appears_behind_a_linked_directory() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    write(
        r,
        "shared/configs/base.json",
        &paths_to("../../packages/app/v2"),
    );
    turborepo_trace_core::os::symlink("../../shared/configs", r.join("packages/app/configs"))
        .unwrap();
    write(r, G_APP, "{ \"extends\": \"./configs/base\" }\n");
    rebase(r, "the app config extends through a linked directory");
    let before = records(r);
    assert!(
        helper_resolves_to(&before, "v2"),
        "{:?}",
        before.by_file[G_HELPER]
    );
    write(r, "shared/configs/base/readme.md", "nothing\n");
    review_probe(r, s, "n6 directory behind a link appears", &before);
    rebase(r, "the directory is there");
    let before = records(r);
    std::fs::remove_dir_all(r.join("shared/configs/base")).unwrap();
    review_probe(r, s, "n6 directory behind a link goes", &before);
}

/// N8: the referenced project extends through a tracked link to a directory;
/// the diff puts a real directory in the link's place, whose config includes
/// the helper (git: the link deleted, a file added below its path).
#[test]
fn n8_a_real_directory_takes_the_place_of_a_linked_one() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    shared_includes(r, "./configs/lib.json");
    let link = r.join("packages/app/configs");
    turborepo_trace_core::os::symlink("../../shared/x", &link).unwrap();
    rebase(r, "the project extends through a linked directory");
    let before = records(r);
    assert!(
        helper_resolves_to(&before, "v1"),
        "{:?}",
        before.by_file[G_HELPER]
    );
    std::fs::remove_file(&link).unwrap();
    write(
        r,
        "packages/app/configs/lib.json",
        "{ \"include\": [\"${configDir}/src\"] }\n",
    );
    review_probe(r, s, "n8 real directory for a link", &before);
    // And back: the link takes the directory's place.
    rebase(r, "a real directory");
    let before = records(r);
    std::fs::remove_dir_all(&link).unwrap();
    turborepo_trace_core::os::symlink("../../shared/x", &link).unwrap();
    review_probe(r, s, "n8 link for a real directory", &before);
}

const G_C: &str = "packages/zed/tests/c.test.ts";

/// A test of another package (no tsconfig above it) that imports the helper: it
/// consults none of the configs of `packages/app` itself.
fn outside_test(r: &Path) {
    write(
        r,
        "packages/zed/package.json",
        r#"{"name":"@acme/zed","version":"1.0.0","scripts":{"test":"vitest run"}}"#,
    );
    write(
        r,
        "packages/zed/vitest.config.ts",
        "import { defineConfig } from 'vitest/config';\n\nexport default defineConfig({ test: { \
         include: ['tests/**/*.test.ts'] } });\n",
    );
    write(
        r,
        G_C,
        "import { expect, test } from 'vitest';\nimport { helper } from \
         '../../app/src/helper';\n\ntest('c', () => {\n  expect(helper).toBe('v2');\n});\n",
    );
}

/// As [`review_probe`], for the tests `of`.
fn review_probe_of(r: &Path, s: &Path, what: &str, before: &Records, of: &[&str]) {
    let cold_dir = s.join(format!("g-oracle-{}", what.replace(' ', "-")));
    let after = records_with(r, &["--graph", cold_dir.to_str().unwrap()]);
    let refreshed = records(r);
    let must = must_select(before, &after, of);
    let got = tests(r);
    eprintln!(
        "REVIEW {what}: must={must:?} got={got:?} refreshed_equals_cold={}\n  helper \
         before={:?}\n  helper after(cold)={:?}",
        refreshed.by_file == after.by_file,
        before.by_file.get(G_HELPER),
        after.by_file.get(G_HELPER),
    );
    assert!(
        refreshed.by_file == after.by_file,
        "{what}: the refreshed graph and a new one hold other records"
    );
    for t in &must {
        assert!(
            got.contains(t),
            "{what}: {t} must be selected (records differ between the base and now); selected: \
             {got:?}"
        );
    }
}

/// N9: the reference after the governing one (`./late`: `late.json`) comes to
/// be broken by a directory `late` that appears: the helper, which the first
/// reference governs, reports the reference that does not load. A test outside
/// the package imports the helper.
#[test]
fn n9_a_later_reference_is_broken_by_a_directory_that_appears() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    outside_test(r);
    write(
        r,
        "packages/app/late.json",
        "{ \"include\": [\"nothing\"] }\n",
    );
    write(
        r,
        G_APP,
        &app_config(
            ",\n  \"references\": [{ \"path\": \"./tsconfig.lib.json\" }, { \"path\": \"./late\" \
             }]",
        ),
    );
    rebase(r, "a second reference");
    let before = records(r);
    assert!(
        helper_resolves_to(&before, "v2"),
        "{:?}",
        before.by_file[G_HELPER]
    );
    write(r, "packages/app/late/readme.md", "nothing\n");
    review_probe_of(
        r,
        s,
        "n9 later reference broken by a directory",
        &before,
        &[G_A, G_B, G_C],
    );
}

/// N9b: the same reference comes to be broken because the config it extends
/// goes.
#[test]
fn n9b_a_later_reference_is_broken_by_a_base_that_goes() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    outside_test(r);
    write(
        r,
        "shared/latebase.json",
        "{ \"include\": [\"nothing\"] }\n",
    );
    write(
        r,
        "packages/app/late.json",
        "{ \"extends\": \"../../shared/latebase.json\" }\n",
    );
    write(
        r,
        G_APP,
        &app_config(
            ",\n  \"references\": [{ \"path\": \"./tsconfig.lib.json\" }, { \"path\": \"./late\" \
             }]",
        ),
    );
    rebase(r, "a second reference that extends a shared config");
    let before = records(r);
    assert!(
        helper_resolves_to(&before, "v2"),
        "{:?}",
        before.by_file[G_HELPER]
    );
    std::fs::remove_file(r.join("shared/latebase.json")).unwrap();
    review_probe_of(
        r,
        s,
        "n9b later reference broken by a base that goes",
        &before,
        &[G_A, G_B, G_C],
    );
}

/// N5: the change is given as paths (`--changed`), and the file that makes the
/// directory is a source file (a node of the refreshed graph: `--changed` takes
/// it for modified).
#[test]
fn n5_a_directory_made_by_a_source_file_given_as_a_path() {
    let d = governed();
    let r = d.path();
    write(r, "packages/app/base.json", &paths_to("./v2"));
    write(r, G_APP, "{ \"extends\": \"./base\" }\n");
    rebase(r, "the app config extends a name without .json");
    let _ = records(r);
    write(r, "packages/app/base/x.ts", "export const x = 1;\n");
    let args = [
        "affected",
        "--changed",
        "packages/app/base/x.ts",
        "--on-uncertain",
        "ignore",
        "--tests",
        "vitest",
    ];
    assert_eq!(lines(run_env(r, &args, &[]), &args), [G_A, G_B]);
}

// ---------------------------------------------------------------------------------------
// What the review probes above left open.
// ---------------------------------------------------------------------------------------

/// The memberships that a graph of `r` built from nothing holds, as the key
/// dump names them (`member <directory> :: <pattern>`, the directory relative
/// to its package).
fn fix55_memberships(r: &Path, s: &Path, what: &str) -> Vec<String> {
    let dump = s.join(format!("dump-{what}"));
    let graph = s.join(format!("g-dump-{what}"));
    let args = [
        "graph",
        "export",
        "--format",
        "ndjson",
        "--graph",
        graph.to_str().unwrap(),
    ];
    let out = run_env(
        r,
        &args,
        &[
            ("TURBO_TRACE_DEBUG_KEY_DUMP", dump.to_str().unwrap()),
            ("TURBO_TRACE_DEBUG_KEY_DUMP_FILES", "1"),
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = std::fs::read_to_string(&dump).unwrap();
    let mut found: Vec<String> = text
        .lines()
        .flat_map(|l| l.split('\t'))
        .filter_map(|f| f.strip_prefix("member "))
        .map(str::to_owned)
        .collect();
    found.sort();
    found.dedup();
    found
}

/// F1, both sides of the rule. A config of a workspace package that was reached
/// through the package's `node_modules` link asks about a directory next to its
/// base by the `node_modules` path: the answer is recorded at the real
/// directory, a path of the repository. The same question inside a package that
/// is really installed (a directory in `node_modules`) is recorded nowhere: the
/// package's identity covers what is in it.
#[test]
fn fix55_a_directory_answer_inside_a_package_is_recorded_where_the_package_is() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    // A workspace package behind a link.
    let d = governed();
    let r = d.path();
    package_chain(r);
    write(r, "tooling/tsconfig/base/readme.md", "nothing\n");
    rebase(r, "a directory next to the base of the package config");
    let linked = fix55_memberships(r, s, "linked");
    assert!(
        linked
            .iter()
            .any(|m| m.ends_with("tooling/tsconfig :: base")),
        "{linked:#?}"
    );
    assert!(
        !linked.iter().any(|m| m.contains("node_modules")),
        "{linked:#?}"
    );
    // The same package, installed: a real directory.
    let d = governed();
    let r = d.path();
    let pkg = "packages/app/node_modules/@acme/tsconfig";
    write(
        r,
        &format!("{pkg}/package.json"),
        r#"{"name":"@acme/tsconfig","version":"1.0.0"}"#,
    );
    write(
        r,
        &format!("{pkg}/chain.json"),
        "{ \"extends\": \"./base\" }\n",
    );
    write(r, &format!("{pkg}/base.json"), "{}\n");
    write(r, &format!("{pkg}/base/readme.md"), "nothing\n");
    write(r, G_APP, "{ \"extends\": \"@acme/tsconfig/chain.json\" }\n");
    rebase(r, "the app config extends an installed package's config");
    let installed = fix55_memberships(r, s, "installed");
    assert!(
        !installed
            .iter()
            .any(|m| m.contains("node_modules") || m.ends_with(":: base")),
        "{installed:#?}"
    );
}

/// The app config extends `./base` (`base.json`), committed; nothing else.
fn fix55_extends_base(r: &Path) {
    write(r, "packages/app/base.json", &paths_to("./v2"));
    write(r, G_APP, "{ \"extends\": \"./base\" }\n");
    rebase(r, "the app config extends a name without .json");
    let _ = records(r);
}

/// `affected` with the change given by `change` (no `--since` unless it is
/// among them).
fn fix55_given(r: &Path, change: &[&str]) -> Vec<String> {
    let args = [
        &["affected"][..],
        change,
        &["--on-uncertain", "ignore", "--tests", "vitest"][..],
    ]
    .concat();
    lines(run_env(r, &args, &[]), &args)
}

/// F2, paths without a status and no base revision. What such a path was before
/// is not known, whether it has a node in the graph or not: its directory may
/// be new, unless something in the directory is no path of the change (the
/// change is what was given: the rest was there).
#[test]
fn fix55_a_path_without_a_status_may_have_made_its_directory() {
    let scratch = tempfile::tempdir().unwrap();
    let d = governed();
    let r = d.path();
    fix55_extends_base(r);
    let none: [&str; 0] = [];
    let x = "packages/app/base/x.ts";
    write(r, x, "export const x = 1;\n");
    // A plain line of `--changed-from` is a path without a status too.
    let list = scratch.path().join("changed.txt");
    std::fs::write(&list, format!("{x}\n")).unwrap();
    let from = ["--changed-from", list.to_str().unwrap()];
    assert_eq!(fix55_given(r, &from), [G_A, G_B]);
    // A line with a status states it: modified, so the directory was there.
    std::fs::write(&list, format!("M\t{x}\n")).unwrap();
    assert_eq!(fix55_given(r, &from), none);
    std::fs::write(&list, format!("A\t{x}\n")).unwrap();
    assert_eq!(fix55_given(r, &from), [G_A, G_B]);
    // A second file in the directory that is no path of the change: it was there,
    // and so was the directory.
    write(r, "packages/app/base/old.ts", "export const old = 1;\n");
    assert_eq!(fix55_given(r, &["--changed", x]), none);
    // Both given: everything in the directory is a path of the change.
    assert_eq!(
        fix55_given(
            r,
            &["--changed", x, "--changed", "packages/app/base/old.ts"]
        ),
        [G_A, G_B]
    );
    // With a base revision next to the paths, the base's tree answers: the
    // directory is not in it,
    assert_eq!(
        fix55_given(r, &["--changed", x, "--since", "HEAD"]),
        [G_A, G_B]
    );
    // and after a commit it is (the config does not load at either revision).
    rebase(r, "the directory is there");
    write(r, x, "export const x = 2;\n");
    assert_eq!(fix55_given(r, &["--changed", x, "--since", "HEAD"]), none);
}

/// F2, several merge bases: the changes are the union of the diffs against each
/// base, so a file that one base has and another lacks is both modified and
/// added. Its directory was made since the base that lacks it. A directory that
/// every base has is not made.
#[test]
fn fix55_a_directory_that_one_of_two_merge_bases_lacks_is_made() {
    for (everywhere, expected) in [(false, &[G_A, G_B][..]), (true, &[][..])] {
        let d = governed();
        let r = d.path();
        if everywhere {
            // The directory is in the first commit, and so in both bases.
            write(r, "packages/app/base/first.md", "nothing\n");
        }
        fix55_extends_base(r);
        git(r, &["branch", "-M", "main"]);
        // Two lines that each merge the other: `main` and `feature` have two merge
        // bases.
        git(r, &["checkout", "-q", "-b", "feature"]);
        write(r, "packages/app/base/x.md", "one\n");
        rebase(r, "feature: a file in the directory");
        git(r, &["tag", "f1"]);
        git(r, &["checkout", "-q", "main"]);
        write(r, "notes.md", "main\n");
        rebase(r, "main: notes");
        git(r, &["tag", "m1"]);
        git(
            r,
            &["merge", "-q", "--no-ff", "-m", "main merges feature", "f1"],
        );
        git(r, &["checkout", "-q", "feature"]);
        git(
            r,
            &["merge", "-q", "--no-ff", "-m", "feature merges main", "m1"],
        );
        // Against `f1` the file is modified, against `m1` it is added.
        write(r, "packages/app/base/x.md", "two\n");
        rebase(r, "feature: the file again");
        let args = [
            "affected",
            "--since",
            "main",
            "--on-uncertain",
            "ignore",
            "--tests",
            "vitest",
        ];
        let out = run_env(r, &args, &[]);
        let err = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(err.contains("2 merge bases"), "{err}");
        assert_eq!(lines(out, &args), expected, "in every base: {everywhere}");
    }
}

const FIX55_LIB: &str = "packages/zed/lib/thing.ts";

/// A package without a config of its own (`packages/zed`), whose test imports
/// `@zed/thing`: the config at the root governs it. `root_config` is that
/// config.
fn fix55_below_the_root_config(r: &Path, root_config: &str) {
    outside_test(r);
    write(r, FIX55_LIB, "export const thing = 1;\n");
    write(
        r,
        "packages/zed/other/thing.ts",
        "export const thing = 2;\n",
    );
    write(
        r,
        G_C,
        "import { expect, test } from 'vitest';\nimport { thing } from '@zed/thing';\n\ntest('c', \
         () => {\n  expect(thing).toBe(1);\n});\n",
    );
    write(r, "tsconfig.json", root_config);
}

/// Item 5. A config at the root that extends a package has the root manifest as
/// its nearest one: the loader asks it whether the package is its own. A field
/// of the root manifest that no config load can read (`packageManager`,
/// `pnpm.overrides`, `engines`: everything but `name`, `exports`, `imports`,
/// `main`, `tsconfig`, `typesVersions`) is no change for the files that
/// have the manifest only because that load read it.
#[test]
fn fix55_a_manifest_field_that_no_config_load_reads() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    fix55_below_the_root_config(
        r,
        "{ \"extends\": \"@tsconfig/node20/tsconfig.json\", \"compilerOptions\": { \"paths\": { \
         \"@zed/*\": [\"./packages/zed/lib/*\"] } } }\n",
    );
    rebase(r, "a root config that extends a package");
    let before = records(r);
    assert!(
        before.by_file[G_C].iter().any(|e| e.ends_with(FIX55_LIB)),
        "{:?}",
        before.by_file[G_C]
    );
    let none: [&str; 0] = [];
    for manifest in [
        r#"{"name":"root","private":true,"packageManager":"pnpm@9.1.0"}"#,
        r#"{"name":"root","private":true,"packageManager":"pnpm@9.1.0","pnpm":{"overrides":{"left-pad":"1.3.0"}},"engines":{"node":">=20"}}"#,
    ] {
        write(r, "package.json", manifest);
        review_probe_of(
            r,
            s,
            "fix55 a field no load reads",
            &before,
            &[G_A, G_B, G_C],
        );
        assert_eq!(tests(r), none, "{manifest}");
        // Without the rule every reader of the manifest is selected: the test that the
        // root config governs.
        assert_eq!(
            tests_env(r, &[("TURBO_TRACE_DEBUG_LOADER_VIEW", "0")]),
            [G_C],
            "{manifest}"
        );
    }
    // The name is a field of the loader's view (the package would be the root's
    // own): the same readers are selected for it.
    write(r, "package.json", r#"{"name":"other","private":true}"#);
    assert_eq!(tests(r), [G_C]);
}

/// Item 5, the other side: a field of the loader's view that changes is a
/// change for every file below a config whose load read the manifest, also
/// where the fields say "entry by entry, for the importers" (`exports`). The
/// root config extends the root package itself by name, and the manifest's
/// `exports` name the config.
#[test]
fn fix55_the_exports_that_a_config_load_went_through_change() {
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let d = governed();
    let r = d.path();
    fix55_below_the_root_config(r, "{ \"extends\": \"@acme/root/config\" }\n");
    let manifest = |config: &str| {
        format!(
            r#"{{"name":"@acme/root","private":true,"exports":{{"./config":"./configs/{config}.json"}}}}"#
        )
    };
    for (name, to) in [("one", "lib"), ("two", "other")] {
        write(
            r,
            &format!("configs/{name}.json"),
            &format!(
                "{{ \"compilerOptions\": {{ \"paths\": {{ \"@zed/*\": \
                 [\"../packages/zed/{to}/*\"] }} }} }}\n"
            ),
        );
    }
    write(r, "package.json", &manifest("one"));
    rebase(r, "a root config that extends the root package's config");
    let before = records(r);
    assert!(
        before.by_file[G_C].iter().any(|e| e.ends_with(FIX55_LIB)),
        "{:?}",
        before.by_file[G_C]
    );
    write(r, "package.json", &manifest("two"));
    review_probe_of(
        r,
        s,
        "fix55 exports of the extended package",
        &before,
        &[G_A, G_B, G_C],
    );
    assert_eq!(tests(r), [G_C]);
    // Without the rule the entry of `exports` is applied for the importers of the
    // package, which the test is not: nothing was selected (before the
    // fix).
    let none: [&str; 0] = [];
    assert_eq!(
        tests_env(r, &[("TURBO_TRACE_DEBUG_LOADER_VIEW", "0")]),
        none
    );
    // Here the loader itself read the manifest (a self-reference), through its
    // view: a field outside the view is no change for the test.
    rebase(r, "the exports name the other config");
    let before = records(r);
    write(
        r,
        "package.json",
        &manifest("two").replace(
            "\"private\":true",
            "\"private\":true,\"packageManager\":\"pnpm@9.1.0\"",
        ),
    );
    review_probe_of(
        r,
        s,
        "fix55 a field no load reads, self-reference",
        &before,
        &[G_A, G_B, G_C],
    );
    assert_eq!(tests(r), none);
    assert_eq!(
        tests_env(r, &[("TURBO_TRACE_DEBUG_LOADER_VIEW", "0")]),
        [G_C]
    );
}

/// Item 3: a directory whose name reads as a pattern. `[id]` is a dynamic route
/// segment of Next.js, `[[...all]]` an optional catch-all; the others are names
/// a file system takes. The governing config extends `./routes/<name>`
/// (`<name>.json`); the diff adds a first file to a directory of that name (the
/// loader now asks for `<name>/tsconfig.json`, and the config does not load),
/// and a second diff removes it.
#[test]
fn fix55_a_directory_named_like_a_pattern_comes_and_goes() {
    for name in ["[id]", "[[...all]]", "{slug}", "a*b", "what?", "a]b}!"] {
        let scratch = tempfile::tempdir().unwrap();
        let s = scratch.path();
        let d = governed();
        let r = d.path();
        write(
            r,
            &format!("packages/app/routes/{name}.json"),
            &paths_to("../v2"),
        );
        write(
            r,
            G_APP,
            &format!("{{ \"extends\": \"./routes/{name}\" }}\n"),
        );
        rebase(r, "the app config extends a name that reads as a pattern");
        let before = records(r);
        assert!(
            helper_resolves_to(&before, "v2"),
            "{name}: {:?}",
            before.by_file[G_HELPER]
        );
        write(
            r,
            &format!("packages/app/routes/{name}/page.tsx"),
            "export default function Page() { return null; }\n",
        );
        review_probe(r, s, &format!("fix55 directory {name} appears"), &before);
        assert_eq!(tests(r), [G_A, G_B], "{name}");
        // The directory is there now. A file that comes next to it is no answer to
        // "is `<name>` a directory?": the record names the directory, not every entry
        // of `routes` (and `i` is not what the pattern `[id]` would match).
        rebase(r, "the directory is there");
        let before = records(r);
        write(r, "packages/app/routes/other.md", "nothing\n");
        write(r, "packages/app/routes/i", "nothing\n");
        assert_eq!(tests(r), [] as [&str; 0], "{name}");
        std::fs::remove_file(r.join("packages/app/routes/other.md")).unwrap();
        std::fs::remove_file(r.join("packages/app/routes/i")).unwrap();
        std::fs::remove_dir_all(r.join(format!("packages/app/routes/{name}"))).unwrap();
        review_probe(r, s, &format!("fix55 directory {name} goes"), &before);
        assert_eq!(tests(r), [G_A, G_B], "{name}");
    }
}

// ---------------------------------------------------------------------------------------
// Review probes of those fixes. Each prints
// what the branch selects with the rule under test and without it.
// ---------------------------------------------------------------------------------------

/// P1. TypeScript resolves an `extends` that names a package through the
/// package's `tsconfig` field (never `main`); so does get-tsconfig. The field
/// is in the loader's view, so a change of it is a change for the files that
/// have the manifest through the load.
#[test]
fn p1_the_tsconfig_field_of_a_package_that_a_config_extends() {
    let d = governed();
    let r = d.path();
    let manifest = |to: &str| {
        format!(
            r#"{{"name":"@acme/tsconfig","version":"1.0.0","main":"./strict.json","tsconfig":"./{to}.json"}}"#
        )
    };
    link_tooling(r, &manifest("strict"));
    write(
        r,
        "tooling/tsconfig/strict.json",
        &paths_to("../../packages/app/v2"),
    );
    write(
        r,
        "tooling/tsconfig/loose.json",
        &paths_to("../../packages/app/v1"),
    );
    write(r, G_APP, "{ \"extends\": \"@acme/tsconfig\" }\n");
    rebase(r, "the app config extends a package by its name");
    let before = records(r);
    assert!(
        helper_resolves_to(&before, "v2"),
        "{:?}",
        before.by_file[G_HELPER]
    );
    // tsc now loads `loose.json` for the app config.
    write(r, "tooling/tsconfig/package.json", &manifest("loose"));
    let off = tests_env(r, &[("TURBO_TRACE_DEBUG_LOADER_VIEW", "0")]);
    let got = tests(r);
    eprintln!("p1 tsconfig field: with the rule {got:?}, without {off:?}");
    assert!(
        got.contains(&G_A.to_owned()),
        "the tsconfig field of the extended package changed: with the rule {got:?}, without \
         {off:?}"
    );
}

/// P2. A package whose own config extends a package: the loader reads the
/// package's own manifest (is the extended package its own?). A test without a
/// bare import reads the manifest no other way. Fields of that manifest that
/// decide how the test's files load (`type`) or run (`jest`, `sideEffects`)
/// must still select it.
#[test]
fn p2_a_field_of_the_own_manifest_that_the_load_does_not_read() {
    let d = governed();
    let r = d.path();
    package_chain(r);
    outside_test(r);
    let link = r.join("packages/zed/node_modules/@acme/tsconfig");
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    turborepo_trace_core::os::symlink("../../../../tooling/tsconfig", link).unwrap();
    write(
        r,
        "packages/zed/tsconfig.json",
        "{ \"extends\": \"@acme/tsconfig/chain.json\" }\n",
    );
    let plain = "packages/zed/tests/d.test.ts";
    write(r, "packages/zed/lib/thing.ts", "export const thing = 1;\n");
    write(
        r,
        plain,
        "import { thing } from '../lib/thing';\n\ntest('d', () => {\n  \
         expect(thing).toBe(1);\n});\n",
    );
    rebase(r, "a package whose config extends a package");
    let _ = records(r);
    let base = r#"{"name":"@acme/zed","version":"1.0.0","scripts":{"test":"vitest run"}"#;
    let mut failed = Vec::new();
    for field in [
        r#""type":"module""#,
        r#""sideEffects":false"#,
        r#""jest":{"testEnvironment":"jsdom"}"#,
        r#""engines":{"node":">=20"}"#,
    ] {
        write(r, "packages/zed/package.json", &format!("{base},{field}}}"));
        let off = tests_env(r, &[("TURBO_TRACE_DEBUG_LOADER_VIEW", "0")]);
        let got = tests(r);
        eprintln!("p2 {field}: with the rule {got:?}, without {off:?}");
        for t in &off {
            if !got.contains(t) {
                failed.push(format!("{field}: {t}"));
            }
        }
    }
    assert!(
        failed.is_empty(),
        "selected without the rule only: {failed:#?}"
    );
}

/// P3. A jest configuration in the root manifest (`jest` key) for a test of a
/// package below a root config that extends a package: the config load is one
/// reader of the root manifest, jest is another.
#[test]
fn p3_a_jest_key_in_a_root_manifest_that_a_config_load_reads() {
    let d = governed();
    let r = d.path();
    let manifest = |env: &str| {
        format!(
            r#"{{"name":"root","private":true,"scripts":{{"test":"jest"}},"jest":{{"testEnvironment":"{env}","testMatch":["**/jt/**/*.test.js"]}}}}"#
        )
    };
    write(r, "package.json", &manifest("node"));
    write(
        r,
        "node_modules/jest/package.json",
        r#"{"name":"jest","version":"29.0.0","main":"./index.js","bin":"./bin.js"}"#,
    );
    write(r, "node_modules/jest/index.js", "module.exports = {};\n");
    write(r, "node_modules/jest/bin.js", "#!/usr/bin/env node\n");
    write(
        r,
        "packages/zed/package.json",
        r#"{"name":"@acme/zed","version":"1.0.0"}"#,
    );
    write(r, "packages/zed/lib/thing.js", "exports.thing = 1;\n");
    let e = "packages/zed/jt/e.test.js";
    write(
        r,
        e,
        "const { thing } = require('../lib/thing');\n\ntest('e', () => {\n  \
         expect(thing).toBe(1);\n});\n",
    );
    write(
        r,
        "tsconfig.json",
        "{ \"extends\": \"@tsconfig/node20/tsconfig.json\", \"compilerOptions\": { \"allowJs\": \
         true } }\n",
    );
    rebase(r, "a jest config in the root manifest");
    let _ = records(r);
    // Nothing changed: is the test an entry at all?
    write(
        r,
        e,
        "const { thing } = require('../lib/thing');\n\ntest('e', () => {\n  \
         expect(thing).toBe(2);\n});\n",
    );
    let entry = tests(r);
    eprintln!("p3 the test itself edited: {entry:?}");
    git(r, &["checkout", "-q", "--", e]);
    write(r, "package.json", &manifest("jsdom"));
    let off = tests_env(r, &[("TURBO_TRACE_DEBUG_LOADER_VIEW", "0")]);
    let got = tests(r);
    eprintln!("p3 jest key: with the rule {got:?}, without {off:?}");
    assert!(entry.contains(&e.to_owned()), "not an entry: {entry:?}");
    for t in &off {
        assert!(
            got.contains(t),
            "jest key of the root manifest: {t} is selected without the rule only ({got:?})"
        );
    }
    assert!(
        got.contains(&e.to_owned()),
        "the jest configuration changed: {got:?} (without the rule {off:?})"
    );
}

/// P4. The files of a directory without a manifest of its own have the root
/// manifest as their nearest one: a test there reads it through the root
/// config's load and through its own resolution. A field outside the loader's
/// view (`type`) must select it.
#[test]
fn p4_a_root_manifest_that_a_file_reads_through_a_load_and_by_itself() {
    let d = governed();
    let r = d.path();
    write(
        r,
        "packages/zed/vitest.config.ts",
        "import { defineConfig } from 'vitest/config';\n\nexport default defineConfig({ test: { \
         include: ['tests/**/*.test.ts'] } });\n",
    );
    write(r, "packages/zed/lib/thing.ts", "export const thing = 1;\n");
    let test = |n: u32| {
        format!(
            "import {{ expect, test }} from 'vitest';\nimport {{ thing }} from \
             '../lib/thing';\n\ntest('c', () => {{\n  expect(thing).toBe({n});\n}});\n"
        )
    };
    write(r, G_C, &test(1));
    write(
        r,
        "tsconfig.json",
        "{ \"extends\": \"@tsconfig/node20/tsconfig.json\" }\n",
    );
    rebase(
        r,
        "a directory without a manifest below a root config that extends a package",
    );
    let _ = records(r);
    write(r, G_C, &test(2));
    let entry = tests(r);
    git(r, &["checkout", "-q", "--", G_C]);
    assert!(entry.contains(&G_C.to_owned()), "not an entry: {entry:?}");
    write(
        r,
        "package.json",
        r#"{"name":"root","private":true,"type":"module"}"#,
    );
    let off = tests_env(r, &[("TURBO_TRACE_DEBUG_LOADER_VIEW", "0")]);
    let got = tests(r);
    eprintln!("p4 type of the nearest manifest: with the rule {got:?}, without {off:?}");
    assert!(got.contains(&G_C.to_owned()), "{got:?} (without {off:?})");
}

/// P5. A test that reads the root manifest as data, below a root config whose
/// load reads it too: a field outside the loader's view is a change for the
/// test.
#[test]
fn p5_a_root_manifest_that_a_test_imports() {
    let d = governed();
    let r = d.path();
    fix55_below_the_root_config(
        r,
        "{ \"extends\": \"@tsconfig/node20/tsconfig.json\", \"compilerOptions\": { \
         \"resolveJsonModule\": true, \"paths\": { \"@zed/*\": [\"./packages/zed/lib/*\"] } } }\n",
    );
    let v = "packages/zed/tests/v.test.ts";
    write(
        r,
        v,
        "import { expect, test } from 'vitest';\nimport pkg from \
         '../../../package.json';\n\ntest('v', () => {\n  expect(pkg.name).toBe('root');\n});\n",
    );
    rebase(r, "a test that imports the root manifest");
    let _ = records(r);
    write(
        r,
        "package.json",
        r#"{"name":"root","private":true,"packageManager":"pnpm@9.1.0"}"#,
    );
    let off = tests_env(r, &[("TURBO_TRACE_DEBUG_LOADER_VIEW", "0")]);
    let got = tests(r);
    eprintln!(
        "p5 a field outside the view, manifest imported: with the rule {got:?}, without {off:?}"
    );
    assert!(got.contains(&v.to_owned()), "{got:?} (without {off:?})");
    assert!(!got.contains(&G_C.to_owned()), "{got:?}");
}

/// C2b. `--since` with one base: the change set also holds the sections of a
/// changed turbo config as "modified" pseudo-paths (`<dir>/turbo.json#global`),
/// also for a config that the diff adds. Such a path below a directory is taken
/// as evidence that the directory was there.
#[test]
#[ignore = "review finding (fails): a section of an added turbo config counts as a modification \
            below its directory"]
fn c2b_a_new_directory_that_holds_a_turbo_config() {
    let d = governed();
    let r = d.path();
    fix55_extends_base(r);
    write(r, "packages/app/base/x.ts", "export const x = 1;\n");
    let without = tests(r);
    write(
        r,
        "packages/app/base/turbo.json",
        "{ \"extends\": [\"//\"], \"tasks\": { \"build\": {} } }\n",
    );
    let with = tests(r);
    eprintln!("c2b: new directory {without:?}; with a turbo.json in it {with:?}");
    assert_eq!(without, [G_A, G_B]);
    assert_eq!(with, [G_A, G_B], "a new directory with a turbo config");
}

/// C2a. Paths without a status and no base: a directory that the change made
/// and that also holds a file that is no path of the change (build output that
/// git ignores, a `.DS_Store`) is taken to have been there.
#[test]
#[ignore = "review finding (fails): without a base, a file that is no path of the change makes a \
            new directory old"]
fn c2a_a_new_directory_that_holds_an_ignored_file() {
    let scratch = tempfile::tempdir().unwrap();
    let d = governed();
    let r = d.path();
    write(
        r,
        ".gitignore",
        ".turbo/trace/\nnode_modules/\n*.tsbuildinfo\n",
    );
    fix55_extends_base(r);
    let x = "packages/app/base/x.ts";
    write(r, x, "export const x = 1;\n");
    // The directory is new and so is everything in it: selected.
    assert_eq!(fix55_given(r, &["--changed", x]), [G_A, G_B]);
    // The same change after a build left an ignored file in the new directory.
    write(r, "packages/app/base/x.tsbuildinfo", "{}\n");
    let bare = fix55_given(r, &["--changed", x]);
    let list = scratch.path().join("changed.txt");
    std::fs::write(&list, format!("A\t{x}\n")).unwrap();
    let stated = fix55_given(r, &["--changed-from", list.to_str().unwrap()]);
    let since = fix55_given(r, &["--since", "HEAD"]);
    eprintln!("c2a: --changed {bare:?}; --changed-from with A {stated:?}; --since HEAD {since:?}");
    assert_eq!(since, [G_A, G_B]);
    assert_eq!(stated, [G_A, G_B], "--changed-from with a stated addition");
    assert_eq!(bare, [G_A, G_B], "--changed");
}
