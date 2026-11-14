#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]
//! `turbo trace`: hidden, gated by `TURBO_EXPERIMENTAL_TRACE`, and the tests
//! it selects on two small monorepos (`trace_jest`, `trace_vitest`).

mod common;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    process::Output,
};

use common::{git, run_turbo_with_env, setup};

const GATE: (&str, &str) = ("TURBO_EXPERIMENTAL_TRACE", "1");
const GATE_MESSAGE: &str = "`turbo trace` is experimental: set TURBO_EXPERIMENTAL_TRACE=1";

/// A fixture copied into a temporary directory and committed on `main`.
fn fixture(name: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    setup::copy_fixture(name, dir.path()).unwrap();
    setup::setup_git(dir.path()).unwrap();
    dir
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The user's cache directory of the turbo runs of a fixture (`root`): in its
/// `.git`, removed with the fixture and never part of its tree.
fn cache_home(root: &Path) -> std::path::PathBuf {
    root.join(".git").join("trace-cache-home")
}

/// Where `turbo trace` keeps the per-machine caches of the graph of `root`'s
/// runs: `turbo/trace` in the user's cache directory (`$XDG_CACHE_HOME` on
/// Linux, `~/Library/Caches` on macOS).
fn machine_cache(root: &Path) -> std::path::PathBuf {
    let base = if cfg!(target_os = "macos") {
        cache_home(root).join("Library/Caches")
    } else {
        cache_home(root)
    };
    base.join("turbo/trace")
}

/// Runs `turbo <args>` in `cwd` with the gate on, plus `env`, as it ships: no
/// debug variable is set (`turbo_command` removes every `TURBO_*` of the test's
/// own environment), and the user's cache directory is the fixture's
/// ([`cache_home`]).
fn turbo_at(root: &Path, cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let home = cache_home(root);
    let home = home.to_str().unwrap();
    let mut all = vec![GATE, ("XDG_CACHE_HOME", home)];
    if cfg!(target_os = "macos") {
        all.push(("HOME", home));
    }
    all.extend_from_slice(env);
    run_turbo_with_env(cwd, args, &all)
}

fn trace(dir: &Path, args: &[&str]) -> Output {
    trace_with_env(dir, args, &[])
}

fn trace_with_env(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut words = vec!["trace"];
    words.extend_from_slice(args);
    turbo_at(dir, dir, &words, env)
}

/// `turbo trace affected <args>` in text mode: the selected test paths.
#[cfg(unix)]
fn affected(dir: &Path, args: &[&str]) -> BTreeSet<String> {
    let mut words = vec!["affected"];
    words.extend_from_slice(args);
    let output = trace(dir, &words);
    assert!(
        output.status.success(),
        "turbo trace {words:?} failed\nstdout:\n{}\nstderr:\n{}",
        stdout(&output),
        stderr(&output)
    );
    stdout(&output).lines().map(str::to_owned).collect()
}

#[track_caller]
fn assert_selects(selected: &BTreeSet<String>, expected: &[&str], not_expected: &[&str]) {
    for test in expected {
        assert!(
            selected.contains(*test),
            "{test} not selected: {selected:?}"
        );
    }
    for test in not_expected {
        assert!(!selected.contains(*test), "{test} selected: {selected:?}");
    }
}

// ---------------------------------------------------------------------------
// Gate off: hidden, refuses, touches nothing.

#[test]
fn trace_is_hidden_from_help() {
    let dir = fixture("trace_jest");
    for env in [&[][..], &[GATE][..]] {
        let output = run_turbo_with_env(dir.path(), &["--help"], env);
        assert!(output.status.success(), "{}", stderr(&output));
        let help = stdout(&output);
        assert!(help.contains("Usage: turbo"), "{help}");
        assert!(
            !help
                .lines()
                .any(|line| line.trim_start().starts_with("trace")),
            "`trace` is listed in `turbo --help`:\n{help}"
        );
    }
}

#[test]
fn without_the_gate_trace_is_a_task_name() {
    // `turbo trace ...` is `turbo run trace ...`, as before the command
    // existed: the same exit code and errors, and no graph is built.
    let dir = fixture("trace_jest");
    let commands: [&[&str]; 4] = [
        &["trace", "affected"],
        &["trace", "affected", "--json", "--exit-code"],
        &["trace", "lint"],
        &["trace"],
    ];
    let envs: [&[(&str, &str)]; 2] = [&[], &[("TURBO_EXPERIMENTAL_TRACE", "0")]];
    for args in commands {
        for env in envs {
            let output = run_turbo_with_env(dir.path(), args, env);
            let mut run = vec!["run"];
            run.extend_from_slice(args);
            let expected = run_turbo_with_env(dir.path(), &run, env);
            assert_eq!(
                output.status.code(),
                expected.status.code(),
                "turbo {args:?} with {env:?}\nstderr:\n{}\nturbo {run:?}:\n{}",
                stderr(&output),
                stderr(&expected)
            );
            assert!(
                !stderr(&output).contains(GATE_MESSAGE),
                "turbo {args:?} with {env:?}: {}",
                stderr(&output)
            );
            assert!(!dir.path().join(".turbo/trace").exists());
        }
    }
    assert!(!dir.path().join(".turbotrace").exists());
}

#[cfg(windows)]
#[test]
fn trace_is_not_supported_on_windows_yet() {
    let dir = fixture("trace_jest");
    let output = trace(dir.path(), &["affected"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("not supported on Windows yet"),
        "{}",
        stderr(&output)
    );
}

// ---------------------------------------------------------------------------
// Gate on: selection on the jest fixture.
//
// `math` exports `add` and `mul` (CommonJS, `src/index.js` requires both);
// `app/src/total.js` requires `math`; `app/src/greet.js` stands alone.

#[cfg(unix)]
const JEST_ADD: &str = "packages/math/test/add.test.js";
#[cfg(unix)]
const JEST_MUL: &str = "packages/math/test/mul.test.js";
#[cfg(unix)]
const JEST_TOTAL: &str = "packages/app/test/total.test.js";
#[cfg(unix)]
const JEST_GREET: &str = "packages/app/test/greet.test.js";

#[cfg(unix)]
#[test]
fn jest_affected_by_explicit_changes() {
    let dir = fixture("trace_jest");
    let selected = affected(dir.path(), &["--changed", "packages/math/src/add.js"]);
    assert_selects(&selected, &[JEST_ADD, JEST_TOTAL], &[JEST_MUL, JEST_GREET]);

    let selected = affected(dir.path(), &["--changed", "packages/app/src/greet.js"]);
    assert_selects(&selected, &[JEST_GREET], &[JEST_ADD, JEST_MUL, JEST_TOTAL]);

    // A test file that changed selects itself.
    let selected = affected(dir.path(), &["--changed", JEST_MUL]);
    assert_selects(&selected, &[JEST_MUL], &[JEST_ADD, JEST_TOTAL, JEST_GREET]);
}

#[cfg(unix)]
#[test]
fn jest_affected_by_a_working_tree_edit() {
    let dir = fixture("trace_jest");
    assert_eq!(affected(dir.path(), &["--base", "HEAD"]), BTreeSet::new());

    fs::write(
        dir.path().join("packages/math/src/mul.js"),
        "function mul(a, b) {\n  return b * a;\n}\n\nmodule.exports = { mul };\n",
    )
    .unwrap();
    let selected = affected(dir.path(), &["--base", "HEAD"]);
    assert_selects(&selected, &[JEST_MUL, JEST_TOTAL], &[JEST_ADD, JEST_GREET]);

    // TURBO_SCM_BASE stands in for --base, as in `turbo query affected`.
    let output = trace_with_env(dir.path(), &["affected"], &[("TURBO_SCM_BASE", "HEAD")]);
    assert!(output.status.success(), "{}", stderr(&output));
    let from_env: BTreeSet<String> = stdout(&output).lines().map(str::to_owned).collect();
    assert_eq!(from_env, selected);
}

#[cfg(unix)]
#[test]
fn jest_affected_relative_to_the_invocation_directory() {
    let dir = fixture("trace_jest");
    let app = dir.path().join("packages/app");
    let output = turbo_at(
        dir.path(),
        &app,
        &["trace", "affected", "--changed", "src/greet.js"],
        &[],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let selected: BTreeSet<String> = stdout(&output).lines().map(str::to_owned).collect();
    // Output paths stay relative to the repository root.
    assert_selects(&selected, &[JEST_GREET], &[JEST_TOTAL]);
}

#[cfg(unix)]
#[test]
fn jest_filter_narrows_the_output_only() {
    let dir = fixture("trace_jest");
    let selected = affected(
        dir.path(),
        &["--changed", "packages/math/src/add.js", "--filter", "app"],
    );
    // `total.test.js` is selected through `math`, which the filter excludes.
    assert_selects(&selected, &[JEST_TOTAL], &[JEST_ADD, JEST_MUL, JEST_GREET]);
}

#[cfg(unix)]
#[test]
fn jest_json_is_byte_stable_and_has_the_query_shape() {
    let dir = fixture("trace_jest");
    fs::write(
        dir.path().join("packages/math/src/add.js"),
        "function add(a, b) {\n  return b + a;\n}\n\nmodule.exports = { add };\n",
    )
    .unwrap();
    let run = || {
        let output = trace(dir.path(), &["affected", "--base", "HEAD", "--json"]);
        assert!(output.status.success(), "{}", stderr(&output));
        output.stdout
    };
    let first = run();
    let second = run();
    assert_eq!(
        String::from_utf8_lossy(&first),
        String::from_utf8_lossy(&second)
    );

    let json: serde_json::Value = serde_json::from_slice(&first).unwrap();
    let object = json.as_object().unwrap();
    assert_eq!(
        object.keys().collect::<Vec<_>>(),
        ["affectedTests", "uncoveredTasks"]
    );
    assert_eq!(json["uncoveredTasks"]["length"], 0);
    let items = json["affectedTests"]["items"].as_array().unwrap();
    assert_eq!(json["affectedTests"]["length"], items.len());
    let paths: BTreeSet<&str> = items
        .iter()
        .map(|item| item["path"].as_str().unwrap())
        .collect();
    assert!(
        paths.contains(JEST_ADD) && paths.contains(JEST_TOTAL),
        "{paths:?}"
    );
    for item in items {
        for key in ["path", "package", "task", "reason"] {
            assert!(item.get(key).is_some(), "{key} missing in {item}");
        }
        assert!(item["reason"]["__typename"].is_string(), "{item}");
    }
    let add = items.iter().find(|item| item["path"] == JEST_ADD).unwrap();
    assert_eq!(add["package"], "math");
    let total = items
        .iter()
        .find(|item| item["path"] == JEST_TOTAL)
        .unwrap();
    assert_eq!(total["package"], "app");
}

#[cfg(unix)]
#[test]
fn jest_exit_code_follows_the_selection() {
    let dir = fixture("trace_jest");
    let output = trace(dir.path(), &["affected", "--base", "HEAD", "--exit-code"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));

    fs::write(
        dir.path().join("packages/app/src/greet.js"),
        "function greet(name) {\n  return `Hi, ${name}!`;\n}\n\nmodule.exports = { greet };\n",
    )
    .unwrap();
    let output = trace(dir.path(), &["affected", "--base", "HEAD", "--exit-code"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(stdout(&output).lines().any(|line| line == JEST_GREET));

    // Errors exit with 2 under --exit-code.
    let output = trace(
        dir.path(),
        &[
            "affected",
            "--base",
            "HEAD",
            "--exit-code",
            "--tests",
            "no-such-framework",
        ],
    );
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
}

#[cfg(unix)]
#[test]
fn jest_why_shows_the_chain() {
    let dir = fixture("trace_jest");
    // `why` traces as Node resolves: within the package (the fixture has no
    // install, so a workspace import ends at the package name).
    let output = trace(
        dir.path(),
        &["why", JEST_TOTAL, "packages/app/src/total.js"],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("total.test.js"), "{text}");
    assert!(text.contains("src/total.js"), "{text}");
    // A file outside the trace is an error.
    let output = trace(
        dir.path(),
        &["why", JEST_TOTAL, "packages/app/src/greet.js"],
    );
    assert!(!output.status.success(), "{}", stdout(&output));
    assert!(
        stderr(&output).contains("is not in the trace"),
        "{}",
        stderr(&output)
    );
}

#[cfg(unix)]
#[test]
fn jest_explain_says_why() {
    let dir = fixture("trace_jest");
    let output = trace(
        dir.path(),
        &[
            "affected",
            "--changed",
            "packages/app/src/greet.js",
            "--explain",
            JEST_GREET,
            JEST_ADD,
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("greet.test.js"), "{text}");
    assert!(text.contains("add.test.js"), "{text}");
}

#[cfg(unix)]
#[test]
fn lint_is_clean_on_the_fixtures() {
    for name in ["trace_jest", "trace_vitest"] {
        let dir = fixture(name);
        let output = trace(dir.path(), &["lint"]);
        assert_eq!(output.status.code(), Some(0), "{name}: {}", stderr(&output));
        let output = trace(dir.path(), &["lint", "--json"]);
        assert_eq!(output.status.code(), Some(0), "{name}: {}", stderr(&output));
    }
}

/// The trace reads the root `turbo.json` only: with another one
/// (`--root-turbo-json`), `affected` selects every test and says why, and
/// `why` and `lint` fail.
#[cfg(unix)]
#[test]
fn another_root_turbo_json_selects_every_test() {
    let dir = fixture("trace_jest");
    fs::copy(
        dir.path().join("turbo.json"),
        dir.path().join("custom-turbo.json"),
    )
    .unwrap();
    let custom = ["--root-turbo-json", "custom-turbo.json"];
    let output = trace(
        dir.path(),
        &[
            &[
                "affected",
                "--changed",
                "packages/math/src/mul.js",
                "--json",
            ][..],
            &custom[..],
        ]
        .concat(),
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    let items = json["affectedTests"]["items"].as_array().unwrap();
    let paths: BTreeSet<&str> = items.iter().map(|i| i["path"].as_str().unwrap()).collect();
    assert_eq!(
        paths,
        BTreeSet::from([JEST_ADD, JEST_MUL, JEST_TOTAL, JEST_GREET]),
        "{json}"
    );
    for item in items {
        assert_eq!(item["reason"]["__typename"], "AllTestsChanged", "{json}");
        let why = item["reason"]["description"].as_str().unwrap();
        assert!(why.contains("--root-turbo-json"), "{why}");
    }
    // The default file, named: the selection as without the flag.
    let named = trace(
        dir.path(),
        &[
            "affected",
            "--changed",
            "packages/math/src/mul.js",
            "--root-turbo-json",
            "turbo.json",
        ],
    );
    assert!(named.status.success(), "{}", stderr(&named));
    let lines: BTreeSet<String> = stdout(&named).lines().map(str::to_owned).collect();
    assert_selects(&lines, &[JEST_MUL, JEST_TOTAL], &[JEST_ADD, JEST_GREET]);
    for args in [
        &["lint"][..],
        &["why", JEST_MUL, "packages/math/src/mul.js"][..],
    ] {
        let output = trace(dir.path(), &[args, &custom[..]].concat());
        assert!(!output.status.success(), "{args:?}: {}", stdout(&output));
        // (miette wraps the message.)
        let err: String = stderr(&output)
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '|')
            .collect();
        assert!(err.contains("--root-turbo-json"), "{}", stderr(&output));
    }
}

/// `turbo trace lint` takes its options from flags only: a baseline file in
/// the repository root is not read (one that does not parse, or that names a
/// finding the tree no longer has, would fail the check).
#[cfg(unix)]
#[test]
fn lint_reads_no_baseline_file() {
    let dir = fixture("trace_jest");
    for text in [
        "this is not json\n",
        r#"{"version":1,"entries":[{"rule":"unconstrained-load","file":"packages/app/src/total.js","key":"gone","count":1}]}"#,
    ] {
        fs::write(dir.path().join("lint-baseline.json"), text).unwrap();
        for args in [&["lint"][..], &["lint", "--json"][..]] {
            let output = trace(dir.path(), args);
            assert_eq!(
                output.status.code(),
                Some(0),
                "{args:?}: {}",
                stderr(&output)
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Gate on: selection on the vitest fixture.
//
// `core` re-exports `slugify` and `clamp` (ESM, TypeScript); `web/src/title.ts`
// imports `slugify` from `core`; `web/src/format.ts` stands alone.

#[cfg(unix)]
const VITEST_SLUG: &str = "packages/core/test/slug.test.ts";
#[cfg(unix)]
const VITEST_CLAMP: &str = "packages/core/test/clamp.test.ts";
#[cfg(unix)]
const VITEST_TITLE: &str = "packages/web/test/title.test.ts";
#[cfg(unix)]
const VITEST_FORMAT: &str = "packages/web/test/format.test.ts";

#[cfg(unix)]
#[test]
fn vitest_affected_by_explicit_changes() {
    let dir = fixture("trace_vitest");
    let selected = affected(dir.path(), &["--changed", "packages/core/src/slug.ts"]);
    assert_selects(
        &selected,
        &[VITEST_SLUG, VITEST_TITLE],
        &[VITEST_CLAMP, VITEST_FORMAT],
    );

    let selected = affected(dir.path(), &["--changed", "packages/web/src/format.ts"]);
    assert_selects(
        &selected,
        &[VITEST_FORMAT],
        &[VITEST_SLUG, VITEST_CLAMP, VITEST_TITLE],
    );

    let selected = affected(
        dir.path(),
        &[
            "--changed",
            "packages/core/src/clamp.ts",
            "--tests",
            "vitest",
        ],
    );
    assert_selects(&selected, &[VITEST_CLAMP], &[VITEST_SLUG, VITEST_FORMAT]);

    // Only jest tests: none in this repository.
    let selected = affected(
        dir.path(),
        &["--changed", "packages/core/src/slug.ts", "--tests", "jest"],
    );
    assert_eq!(selected, BTreeSet::new());
}

#[cfg(unix)]
#[test]
fn vitest_affected_by_a_working_tree_edit() {
    let dir = fixture("trace_vitest");
    assert_eq!(affected(dir.path(), &["--base", "HEAD"]), BTreeSet::new());

    fs::write(
        dir.path().join("packages/core/src/slug.ts"),
        "export function slugify(text: string): string {\n  return \
         text.trim().toLowerCase().replace(/\\s+/g, \"-\");\n}\n",
    )
    .unwrap();
    let selected = affected(dir.path(), &["--base", "HEAD"]);
    assert_selects(
        &selected,
        &[VITEST_SLUG, VITEST_TITLE],
        &[VITEST_CLAMP, VITEST_FORMAT],
    );

    // A committed change is found between two refs.
    git(dir.path(), &["commit", "-am", "edit slug", "--quiet"]);
    let between = affected(dir.path(), &["--base", "HEAD~1", "--head", "HEAD"]);
    assert_eq!(between, selected);
    assert_eq!(affected(dir.path(), &["--base", "HEAD"]), BTreeSet::new());
}

#[cfg(unix)]
#[test]
fn vitest_new_test_file_selects_itself() {
    let dir = fixture("trace_vitest");
    let new_test = "packages/web/test/extra.test.ts";
    fs::write(
        dir.path().join(new_test),
        "import { expect, test } from \"vitest\";\nimport { formatCount } from \
         \"../src/format\";\n\ntest(\"one\", () => {\n  expect(formatCount(1)).toBe(\"1 \
         item\");\n});\n",
    )
    .unwrap();
    let selected = affected(dir.path(), &["--base", "HEAD"]);
    assert_selects(&selected, &[new_test], &[VITEST_FORMAT, VITEST_SLUG]);
}

#[cfg(unix)]
#[test]
fn vitest_json_is_byte_stable() {
    let dir = fixture("trace_vitest");
    let args = [
        "affected",
        "--changed",
        "packages/core/src/slug.ts",
        "--json",
    ];
    let first = trace(dir.path(), &args);
    let second = trace(dir.path(), &args);
    assert!(first.status.success(), "{}", stderr(&first));
    assert_eq!(stdout(&first), stdout(&second));
    let json: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert!(json["affectedTests"]["items"].is_array(), "{json}");
}

// ---------------------------------------------------------------------------
// Selection: the base, unknown tests, the gate, file names.

/// The branch edits `add.js`; `main` later gets the same edit (a cherry-pick,
/// a squash-merged part of a stack). The changes are those since the merge
/// base, and the base's files are read there too, not at `main`'s tip: `add.js`
/// changed, and its tests are selected.
#[cfg(unix)]
#[test]
fn a_base_that_moved_on_is_compared_at_the_merge_base() {
    let dir = fixture("trace_jest");
    let add = dir.path().join("packages/math/src/add.js");
    let edited = "function add(a, b) {\n  return b + a;\n}\n\nmodule.exports = { add };\n";
    git(dir.path(), &["checkout", "--quiet", "-b", "work"]);
    fs::write(&add, edited).unwrap();
    git(dir.path(), &["commit", "--quiet", "-am", "work"]);
    git(dir.path(), &["checkout", "--quiet", "main"]);
    fs::write(&add, edited).unwrap();
    git(
        dir.path(),
        &["commit", "--quiet", "-am", "the same edit on main"],
    );
    git(dir.path(), &["checkout", "--quiet", "work"]);

    let selected = affected(dir.path(), &["--base", "main"]);
    assert_selects(&selected, &[JEST_ADD, JEST_TOTAL], &[JEST_GREET]);
}

/// The configuration `turbo trace` ships in, in a pull request's CI run: no
/// debug variable, the base from GitHub Actions (`GITHUB_BASE_REF`) resolved by
/// turbo's SCM layer and compared at the merge base, the caches in the user's
/// cache directory, and the graph put into turbo's cache (CI only).
#[cfg(unix)]
#[test]
fn a_pull_request_run_in_the_shipped_configuration() {
    let dir = fixture("trace_jest");
    git(dir.path(), &["checkout", "--quiet", "-b", "work"]);
    edit_mul(dir.path());
    git(dir.path(), &["commit", "--quiet", "-am", "mul"]);
    git(dir.path(), &["checkout", "--quiet", "main"]);
    fs::write(
        dir.path().join("packages/math/src/add.js"),
        "function add(a, b) {\n  return b + a;\n}\n\nmodule.exports = { add };\n",
    )
    .unwrap();
    git(dir.path(), &["commit", "--quiet", "-am", "main moves on"]);
    git(dir.path(), &["checkout", "--quiet", "work"]);
    let cache = dir.path().join(".git/turbo-cache");
    let output = trace_with_env(
        dir.path(),
        &["affected", "--cache-dir", cache.to_str().unwrap()],
        &[
            ("CI", "1"),
            ("GITHUB_ACTIONS", "true"),
            ("GITHUB_BASE_REF", "main"),
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let selected: BTreeSet<String> = stdout(&output).lines().map(str::to_owned).collect();
    assert_selects(&selected, &[JEST_MUL, JEST_TOTAL], &[JEST_ADD, JEST_GREET]);
    let err = stderr(&output);
    assert!(
        err.contains("Resolved base ref from GitHub Actions event: main"),
        "{err}"
    );
    assert!(err.contains("put the graph of"), "{err}");
    assert_eq!(cached_artifacts(&cache).len(), 1);
    assert!(machine_cache(dir.path()).is_dir());
    assert!(!dir.path().join(".turbo/trace/cache").exists());
}

/// A jest package without a config file (jest's defaults): the selection does
/// not know which files are its tests, so it lists the files that look like
/// tests as `UnknownTest`, which always run. They are in the output, and
/// `--exit-code` says 1 when a test file itself changed.
#[cfg(unix)]
#[test]
fn tests_the_selection_does_not_know_are_reported() {
    let dir = fixture("trace_jest");
    fs::remove_file(dir.path().join("packages/math/jest.config.js")).unwrap();
    git(dir.path(), &["commit", "--quiet", "-am", "jest defaults"]);

    fs::write(
        dir.path().join("packages/math/src/add.js"),
        "function add(a, b) {\n  return b + a;\n}\n\nmodule.exports = { add };\n",
    )
    .unwrap();
    let selected = affected(dir.path(), &["--base", "HEAD"]);
    assert_selects(&selected, &[JEST_ADD, JEST_TOTAL], &[]);

    git(dir.path(), &["checkout", "--quiet", "--", "."]);
    fs::write(
        dir.path().join("packages/math/test/add.test.js"),
        "const { add } = require(\"../src/add\");\n\ntest(\"adds\", () => {\n  expect(add(2, \
         2)).toBe(4);\n});\n",
    )
    .unwrap();
    let output = trace(dir.path(), &["affected", "--base", "HEAD", "--exit-code"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "a changed test file is not selected\nstdout:\n{}\nstderr:\n{}",
        stdout(&output),
        stderr(&output)
    );
}

/// jest without a config runs its default `testMatch`: every file below
/// `__tests__`, a bare `test.js` and `spec.js`, and `*.test.js` / `*.spec.js`.
/// The selection does not know these as tests, so they are listed as
/// `UnknownTest` (they always run), and `--exit-code` does not say that nothing
/// needs to run.
#[cfg(unix)]
#[test]
fn jest_default_test_files_without_a_config_are_listed() {
    let dir = fixture("trace_jest");
    let math = dir.path().join("packages/math");
    fs::remove_file(math.join("jest.config.js")).unwrap();
    fs::remove_dir_all(math.join("test")).unwrap();
    let shapes = [
        "packages/math/__tests__/sum.js",
        "packages/math/test.js",
        "packages/math/spec.js",
    ];
    for shape in shapes {
        let path = dir.path().join(shape);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            "const { add } = require(\"./src/add\");\n\ntest(\"adds\", () => {\n  expect(add(1, \
             2)).toBe(3);\n});\n",
        )
        .unwrap();
    }
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "--quiet", "-m", "jest defaults"]);

    let output = trace(dir.path(), &["affected", "--base", "HEAD", "--json"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = json["affectedTests"]["items"].as_array().unwrap();
    for shape in shapes {
        let item = items.iter().find(|i| i["path"] == shape);
        assert!(
            item.is_some_and(|i| i["reason"]["__typename"] == "UnknownTest"),
            "{shape}: {json}"
        );
    }
    let output = trace(dir.path(), &["affected", "--base", "HEAD", "--exit-code"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "jest's default test files are not run\nstdout:\n{}\nstderr:\n{}",
        stdout(&output),
        stderr(&output)
    );
    let listed: BTreeSet<String> = stdout(&output).lines().map(str::to_owned).collect();
    assert_selects(&listed, &shapes, &[]);
}

/// With the gate off, `turbo trace` is `turbo run trace`, as before the command
/// existed: a repository's task named `trace` runs.
#[test]
fn a_task_named_trace_still_runs_with_the_gate_off() {
    let dir = fixture("trace_jest");
    fs::write(
        dir.path().join("turbo.json"),
        "{\n  \"tasks\": {\n    \"test\": {},\n    \"trace\": {}\n  }\n}\n",
    )
    .unwrap();
    git(dir.path(), &["commit", "--quiet", "-am", "a trace task"]);
    let output = run_turbo_with_env(dir.path(), &["trace", "--dry=json"], &[]);
    assert!(
        output.status.success(),
        "`turbo trace --dry=json` (the task) failed\nstderr:\n{}",
        stderr(&output)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        json["tasks"]
            .as_array()
            .is_some_and(|tasks| tasks.iter().any(|t| t["taskId"] == "math#trace")),
        "{json}"
    );
}

/// With the gate off, run flags before the word (`turbo --filter math trace`,
/// `turbo --no-daemon trace`) make it `turbo run ... trace` too: the task.
#[test]
fn a_task_named_trace_still_runs_after_a_leading_run_flag_with_the_gate_off() {
    let dir = fixture("trace_jest");
    fs::write(
        dir.path().join("turbo.json"),
        "{\n  \"tasks\": {\n    \"test\": {},\n    \"trace\": {}\n  }\n}\n",
    )
    .unwrap();
    git(dir.path(), &["commit", "--quiet", "-am", "a trace task"]);
    for args in [
        &["--filter", "math", "trace", "--dry=json"][..],
        &["-F", "math", "trace", "--dry=json"][..],
        &["--no-daemon", "trace", "--dry=json"][..],
    ] {
        let output = run_turbo_with_env(dir.path(), args, &[]);
        assert!(
            output.status.success(),
            "`turbo {args:?}` (the task) failed\nstderr:\n{}",
            stderr(&output)
        );
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(
            json["tasks"]
                .as_array()
                .is_some_and(|tasks| tasks.iter().any(|t| t["taskId"] == "math#trace")),
            "{json}"
        );
    }
}

/// `affected` lists a test it does not know as `UnknownTest` (it always runs),
/// and `--explain` says the same of that file.
#[cfg(unix)]
#[test]
fn explain_agrees_with_the_selection_on_a_test_it_does_not_know() {
    let dir = fixture("trace_jest");
    fs::remove_file(dir.path().join("packages/math/jest.config.js")).unwrap();
    git(dir.path(), &["commit", "--quiet", "-am", "jest defaults"]);

    let selected = affected(dir.path(), &["--base", "HEAD"]);
    assert_selects(&selected, &[JEST_ADD], &[]);
    let output = trace(
        dir.path(),
        &["affected", "--base", "HEAD", "--explain", JEST_ADD],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(
        !text.contains(&format!("unaffected {JEST_ADD}")),
        "`affected` lists {JEST_ADD}, --explain says:\n{text}"
    );
}

/// A source file whose name starts with a character outside ASCII (`é.js`) is
/// traced like any other.
#[cfg(unix)]
#[test]
fn a_file_name_that_starts_outside_ascii_does_not_crash() {
    let dir = fixture("trace_jest");
    fs::write(
        dir.path().join("packages/app/src/é.js"),
        "module.exports = { v: 1 };\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("packages/app/test/e.test.js"),
        "const p = require(\"../src/é\");\n\ntest(\"p\", () => {\n  expect(p.v).toBe(1);\n});\n",
    )
    .unwrap();
    git(dir.path(), &["add", "-A"]);
    git(
        dir.path(),
        &["commit", "--quiet", "-m", "a name outside ASCII"],
    );
    fs::write(
        dir.path().join("packages/app/src/é.js"),
        "module.exports = { v: 2 };\n",
    )
    .unwrap();
    let selected = affected(dir.path(), &["--base", "HEAD"]);
    assert_selects(&selected, &["packages/app/test/e.test.js"], &[]);
}

/// `require("../src/q?")` loads `q?.js` in Node's CommonJS loader. The
/// resolver reads `?` as the start of a query and looks for `../src/q`, so the
/// test has no reference to the file and an edit of it selects nothing
/// (deleting it does).
#[cfg(unix)]
#[test]
#[ignore = "known gap: a `?` in a CommonJS specifier is read as a query, so an edit of `q?.js` \
            does not select the test that requires it"]
fn a_file_name_with_a_question_mark_is_followed() {
    let dir = fixture("trace_jest");
    fs::write(
        dir.path().join("packages/app/src/q?.js"),
        "module.exports = { v: 1 };\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("packages/app/test/q.test.js"),
        "const p = require(\"../src/q?\");\n\ntest(\"p\", () => {\n  expect(p.v).toBe(1);\n});\n",
    )
    .unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "--quiet", "-m", "a question mark"]);
    fs::write(
        dir.path().join("packages/app/src/q?.js"),
        "module.exports = { v: 2 };\n",
    )
    .unwrap();
    let selected = affected(dir.path(), &["--base", "HEAD"]);
    assert_selects(&selected, &["packages/app/test/q.test.js"], &[]);
}

/// The graph is stored under `.turbo`, which a repository ignores: after a run
/// the work tree is clean, so turbo's own `--affected` sees no change.
#[cfg(unix)]
#[test]
fn a_run_leaves_the_work_tree_clean() {
    let dir = fixture("trace_jest");
    affected(dir.path(), &["--base", "HEAD"]);
    let status = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&status.stdout), "");
}

/// There is no configuration file: a `turbotrace.toml` that does not parse is
/// not read, and the command succeeds.
#[cfg(unix)]
#[test]
fn a_configuration_file_is_not_read() {
    let dir = fixture("trace_jest");
    fs::write(dir.path().join("turbotrace.toml"), "this is [not toml\n").unwrap();
    let output = trace(dir.path(), &["affected", "--base", "HEAD"]);
    assert!(output.status.success(), "{}", stderr(&output));
}

#[test]
fn trace_without_a_subcommand_is_an_error_with_the_gate() {
    let dir = fixture("trace_jest");
    let output = trace(dir.path(), &[]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let err = stderr(&output);
    assert!(
        err.contains("turbo trace affected") || err.contains("not supported on Windows"),
        "{err}"
    );
}

#[test]
fn run_is_unchanged_by_the_gate() {
    // The gate is not a task-hash input: a dry run hashes the same with it.
    let dir = fixture("trace_jest");
    let hashes = |env: &[(&str, &str)]| {
        let output = run_turbo_with_env(dir.path(), &["run", "test", "--dry=json"], env);
        assert!(output.status.success(), "{}", stderr(&output));
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        json["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|task| {
                (
                    task["taskId"].as_str().unwrap().to_owned(),
                    task["hash"].as_str().unwrap().to_owned(),
                )
            })
            .collect::<BTreeMap<_, _>>()
    };
    let off = hashes(&[]);
    assert!(!off.is_empty());
    assert_eq!(off, hashes(&[GATE]));
}

// ---------------------------------------------------------------------------
// Storage: the graph in `.turbo/trace`, its caches per machine; the graph
// through turbo's cache.

/// A clone of the committed fixture `src` at `dst` (a second checkout of the
/// same commits, as CI makes).
#[cfg(unix)]
fn clone(src: &Path, dst: &Path) {
    let out = std::process::Command::new("git")
        .args(["clone", "--quiet", "--no-hardlinks"])
        .arg(src)
        .arg(dst)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    git(dst, &["config", "user.email", "turbo-test@example.com"]);
    git(dst, &["config", "user.name", "Turbo Test"]);
}

#[cfg(unix)]
fn copy_dir(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let to = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &to);
        } else {
            fs::copy(entry.path(), &to).unwrap();
        }
    }
}

/// The same edit in any checkout: `mul` changes, with the file's size kept
/// (a stamp cache would only be fooled by a change it cannot see by size).
#[cfg(unix)]
fn edit_mul(root: &Path) {
    let path = root.join("packages/math/src/mul.js");
    let before = fs::read_to_string(&path).unwrap();
    let after = before.replacen("a * b", "b * a", 1);
    assert_ne!(before, after, "the fixture's mul.js changed");
    assert_eq!(before.len(), after.len());
    fs::write(&path, after).unwrap();
}

#[cfg(unix)]
#[test]
fn the_graph_is_in_dot_turbo_and_its_caches_are_not() {
    let dir = fixture("trace_jest");
    affected(dir.path(), &["--changed", "packages/math/src/add.js"]);
    let graph = dir.path().join(".turbo/trace");
    assert!(graph.join("graph.json").is_file());
    assert!(graph.join("store").is_dir());
    let mut names: Vec<String> = fs::read_dir(&graph)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    // Nothing keyed by this machine's file stamps travels with `.turbo`.
    assert_eq!(names, ["graph.json", "store"], "{names:?}");
    assert!(!dir.path().join(".turbotrace").exists());
    // The caches are in the per-machine directory, one per graph directory.
    let caches: Vec<_> = fs::read_dir(machine_cache(dir.path())).unwrap().collect();
    assert_eq!(caches.len(), 1);
    let cache = caches[0].as_ref().unwrap().path();
    // (The walk cache is not written for a tree edited within two seconds.)
    assert!(cache.join("hashes").is_file(), "no hashes in {cache:?}");
    let named = fs::read_to_string(cache.join("graph-dir")).unwrap();
    assert!(named.trim_end().ends_with(".turbo/trace"), "{named}");
}

#[cfg(unix)]
#[test]
fn dot_turbo_restored_into_a_second_checkout_selects_as_a_cold_build() {
    let first = fixture("trace_jest");
    // A graph built in the first checkout, with its working tree edited.
    edit_mul(first.path());
    let in_first = affected(first.path(), &["--base", "HEAD"]);
    assert_selects(&in_first, &[JEST_MUL, JEST_TOTAL], &[JEST_ADD, JEST_GREET]);

    // A second checkout of the same commit, with the first one's `.turbo`
    // (as `actions/cache` restores it) and another edit.
    let second = tempfile::tempdir().unwrap();
    let second = second.path().join("repo");
    clone(first.path(), &second);
    copy_dir(&first.path().join(".turbo"), &second.join(".turbo"));
    fs::write(
        second.join("packages/math/src/add.js"),
        fs::read_to_string(second.join("packages/math/src/add.js"))
            .unwrap()
            .replacen("a + b", "b + a", 1),
    )
    .unwrap();
    let restored = affected(&second, &["--base", "HEAD"]);

    // A third checkout with the same edit and no `.turbo`: the cold build.
    let third = tempfile::tempdir().unwrap();
    let third = third.path().join("repo");
    clone(first.path(), &third);
    fs::copy(
        second.join("packages/math/src/add.js"),
        third.join("packages/math/src/add.js"),
    )
    .unwrap();
    let cold = affected(&third, &["--base", "HEAD"]);
    assert_eq!(restored, cold);
    assert_selects(&cold, &[JEST_ADD, JEST_TOTAL], &[JEST_MUL, JEST_GREET]);
}

/// The local cache's files of the graph keys (`<hash>.tar.zst`).
#[cfg(unix)]
fn cached_artifacts(cache: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(cache)
        .map(|entries| {
            entries
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|n| n.ends_with(".tar.zst"))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[cfg(unix)]
#[test]
fn the_graph_travels_through_the_turbo_cache() {
    let scratch = tempfile::tempdir().unwrap();
    let cache = scratch.path().join("turbo-cache");
    let cache_arg = cache.to_str().unwrap();

    // CI on `main`: builds the graph and puts it into the cache.
    let main = fixture("trace_jest");
    let output = trace_with_env(
        main.path(),
        &["affected", "--base", "HEAD", "--cache-dir", cache_arg],
        &[("CI", "1")],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("put the graph of"),
        "{}",
        stderr(&output)
    );
    assert_eq!(cached_artifacts(&cache).len(), 1);
    // Once per commit.
    let output = trace_with_env(
        main.path(),
        &["affected", "--base", "HEAD", "--cache-dir", cache_arg],
        &[("CI", "1")],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        !stderr(&output).contains("put the graph"),
        "{}",
        stderr(&output)
    );
    // Not outside CI.
    let other = scratch.path().join("other-cache");
    let output = trace(
        main.path(),
        &[
            "affected",
            "--base",
            "HEAD",
            "--cache-dir",
            other.to_str().unwrap(),
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(cached_artifacts(&other).is_empty());

    // A fresh clone with a commit of its own on a branch: the graph of the
    // merge base with `main` comes from the cache.
    let fresh = scratch.path().join("fresh");
    clone(main.path(), &fresh);
    git(&fresh, &["checkout", "--quiet", "-b", "feature"]);
    edit_mul(&fresh);
    git(&fresh, &["commit", "--quiet", "-am", "mul"]);
    let output = trace(
        &fresh,
        &["affected", "--base", "main", "--cache-dir", cache_arg],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("restored the graph of"),
        "{}",
        stderr(&output)
    );
    let restored: BTreeSet<String> = stdout(&output).lines().map(str::to_owned).collect();

    // The same commit, cold: another clone, and a cache that holds nothing.
    let cold = scratch.path().join("cold");
    clone(&fresh, &cold);
    git(&cold, &["branch", "--quiet", "main", "origin/main"]);
    let empty = scratch.path().join("empty-cache");
    let output = trace(
        &cold,
        &[
            "affected",
            "--base",
            "main",
            "--cache-dir",
            empty.to_str().unwrap(),
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(!stderr(&output).contains("restored"), "{}", stderr(&output));
    let cold: BTreeSet<String> = stdout(&output).lines().map(str::to_owned).collect();
    assert_eq!(restored, cold);
    assert_selects(&cold, &[JEST_MUL, JEST_TOTAL], &[JEST_ADD, JEST_GREET]);

    // `--cache=local:w` reads nothing: no restore.
    let unread = scratch.path().join("unread");
    clone(&fresh, &unread);
    git(&unread, &["branch", "--quiet", "main", "origin/main"]);
    let output = trace(
        &unread,
        &[
            "affected",
            "--base",
            "main",
            "--cache-dir",
            cache_arg,
            "--cache=local:w",
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(!stderr(&output).contains("restored"), "{}", stderr(&output));
}

/// A package whose runner the trace does not cover (mocha): its test task runs
/// whole, and says so.
#[cfg(unix)]
#[test]
fn a_task_of_a_runner_the_trace_does_not_cover_runs_whole() {
    let dir = fixture("trace_jest");
    let math = dir.path().join("packages/math/package.json");
    let manifest = fs::read_to_string(&math).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&manifest).unwrap();
    json["scripts"]["test"] = "mocha".into();
    fs::write(&math, serde_json::to_string_pretty(&json).unwrap()).unwrap();
    fs::remove_file(dir.path().join("packages/math/jest.config.js")).unwrap();
    git(dir.path(), &["commit", "--quiet", "-am", "mocha"]);
    fs::write(
        dir.path().join("packages/app/src/greet.js"),
        fs::read_to_string(dir.path().join("packages/app/src/greet.js")).unwrap() + "\n",
    )
    .unwrap();
    let output = trace(dir.path(), &["affected", "--base", "HEAD", "--json"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let tasks: Vec<&str> = json["uncoveredTasks"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["task"].as_str().unwrap())
        .collect();
    assert_eq!(tasks, ["math#test"], "{json}");
    let output = trace(dir.path(), &["affected", "--base", "HEAD", "--exit-code"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("math#test runs whole"),
        "{}",
        stderr(&output)
    );
}

/// A file that looks like a test but that the package's jest config leaves
/// out of `testMatch` is not known to be no test: it is listed.
#[cfg(unix)]
#[test]
fn a_test_file_outside_the_configured_pattern_is_listed() {
    let dir = fixture("trace_jest");
    let config = dir.path().join("packages/math/jest.config.js");
    let text = fs::read_to_string(&config).unwrap();
    let text = text.replacen("test/**/*.test.js", "test/add.test.js", 1);
    fs::write(&config, text).unwrap();
    git(
        dir.path(),
        &["commit", "--quiet", "-am", "narrow testMatch"],
    );
    fs::write(
        dir.path().join(JEST_MUL),
        fs::read_to_string(dir.path().join(JEST_MUL)).unwrap() + "\n",
    )
    .unwrap();
    let output = trace(dir.path(), &["affected", "--base", "HEAD", "--json"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let mul = json["affectedTests"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["path"] == JEST_MUL)
        .cloned();
    assert!(mul.is_some(), "{json}");
}

/// `--tests` takes the frameworks `turbo trace` lists tests of; any other is an
/// error (exit code 2 under `--exit-code`), never an empty selection.
#[cfg(unix)]
#[test]
fn tests_takes_only_the_frameworks_it_supports() {
    let dir = fixture("trace_jest");
    for framework in ["playwright", "pytest", "mocha"] {
        let output = trace(
            dir.path(),
            &["affected", "--changed", JEST_MUL, "--tests", framework],
        );
        assert!(!output.status.success(), "--tests {framework} succeeded");
        assert!(
            stderr(&output).contains("unknown test framework"),
            "{}",
            stderr(&output)
        );
        let output = trace(
            dir.path(),
            &[
                "affected",
                "--changed",
                JEST_MUL,
                "--tests",
                framework,
                "--exit-code",
            ],
        );
        assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    }
    let selected = affected(dir.path(), &["--changed", JEST_MUL, "--tests", "jest"]);
    assert_selects(&selected, &[JEST_MUL], &[JEST_ADD]);
}
