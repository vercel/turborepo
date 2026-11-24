#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]
//! `turbo trace`: hidden, gated by `TURBO_EXPERIMENTAL_TRACE`, and the tests
//! it selects on small monorepos (`trace_jest`, `trace_vitest`, and
//! `trace_polyglot` with TypeScript, Go and Rust packages).

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
    for name in ["trace_jest", "trace_vitest", "trace_polyglot"] {
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
// Gate on: selection across languages (`trace_polyglot`).
//
// `apps/web` is TypeScript with vitest: `price.test.ts` imports `src/price.ts`,
// `greeting.test.ts` imports `src/greeting.ts`, and `rates.test.ts` reads
// `shared/rates.json` with `fs.readFileSync(path.join(__dirname, ...))`.
// `services/ledger` is a Go module (`go test ./...`): package `book` imports
// package `money`, and `book_test.go` reads `shared/rates.json` with
// `os.ReadFile`. `crates/rates` is a Rust crate (`cargo test`) whose `lib.rs`
// has `include_str!("../../../shared/rates.json")`; its unit tests are listed
// by the crate root (`src/lib.rs`, which also stands for those in
// `src/convert.rs`), its integration test is `tests/convert.rs`.

#[cfg(unix)]
const POLY_PRICE: &str = "apps/web/test/price.test.ts";
#[cfg(unix)]
const POLY_GREETING: &str = "apps/web/test/greeting.test.ts";
#[cfg(unix)]
const POLY_JS_RATES: &str = "apps/web/test/rates.test.ts";
#[cfg(unix)]
const POLY_MONEY: &str = "services/ledger/money/money_test.go";
#[cfg(unix)]
const POLY_BOOK: &str = "services/ledger/book/book_test.go";
#[cfg(unix)]
const POLY_RUST_UNIT: &str = "crates/rates/src/lib.rs";
#[cfg(unix)]
const POLY_RUST_INTEGRATION: &str = "crates/rates/tests/convert.rs";

/// What `turbo trace affected --json <args>` reports: each selected test with
/// its task, and the tasks that run whole.
#[cfg(unix)]
struct JsonSelection {
    tests: BTreeMap<String, String>,
    uncovered: Vec<String>,
}

#[cfg(unix)]
fn affected_json(dir: &Path, args: &[&str]) -> JsonSelection {
    let mut words = vec!["affected", "--json"];
    words.extend_from_slice(args);
    let output = trace(dir, &words);
    assert!(
        output.status.success(),
        "turbo trace {words:?} failed\nstdout:\n{}\nstderr:\n{}",
        stdout(&output),
        stderr(&output)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let tests = json["affectedTests"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["path"].as_str().unwrap().to_owned(),
                item["task"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    let uncovered = json["uncoveredTasks"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["task"].as_str().unwrap().to_owned())
        .collect();
    JsonSelection { tests, uncovered }
}

#[cfg(unix)]
fn test_paths(selection: &JsonSelection) -> BTreeSet<&str> {
    selection.tests.keys().map(String::as_str).collect()
}

/// A Go source file selects the tests of its package and of the packages that
/// import it, and no test of another language.
#[cfg(unix)]
#[test]
fn polyglot_go_change_selects_its_package_and_importers() {
    let dir = fixture("trace_polyglot");
    let selection = affected_json(dir.path(), &["--changed", "services/ledger/money/money.go"]);
    assert_eq!(
        test_paths(&selection),
        BTreeSet::from([POLY_MONEY, POLY_BOOK])
    );
    assert!(
        selection.tests.values().all(|task| task == "ledger#test"),
        "{:?}",
        selection.tests
    );
    assert_eq!(selection.uncovered, Vec::<String>::new());

    // The importing package does not select the imported one's tests.
    let selected = affected(dir.path(), &["--changed", "services/ledger/book/book.go"]);
    assert_eq!(selected, BTreeSet::from([POLY_BOOK.to_owned()]));

    // `--explain` gives the chain through the importing package's source.
    let output = trace(
        dir.path(),
        &[
            "affected",
            "--changed",
            "services/ledger/money/money.go",
            "--explain",
            POLY_BOOK,
            POLY_PRICE,
            POLY_RUST_UNIT,
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.contains(&format!(
            "affected {POLY_BOOK}: modified services/ledger/money/money.go → \
             services/ledger/book/book.go → {POLY_BOOK}"
        )),
        "{text}"
    );
    assert!(text.contains(&format!("unaffected {POLY_PRICE}")), "{text}");
    assert!(
        text.contains(&format!("unaffected {POLY_RUST_UNIT}")),
        "{text}"
    );

    // Only Go tests asked for: the same tests, and the other languages' test
    // tasks are not covered by the selection, so they run whole.
    let selection = affected_json(
        dir.path(),
        &[
            "--changed",
            "services/ledger/money/money.go",
            "--tests",
            "gotest",
        ],
    );
    assert_eq!(
        test_paths(&selection),
        BTreeSet::from([POLY_MONEY, POLY_BOOK])
    );
    assert_eq!(selection.uncovered, ["rates#test", "web#test"]);
}

/// A Rust source file selects the crate's unit and integration tests, and
/// nothing else.
#[cfg(unix)]
#[test]
fn polyglot_rust_change_selects_the_crate_tests() {
    let dir = fixture("trace_polyglot");
    let selection = affected_json(dir.path(), &["--changed", "crates/rates/src/convert.rs"]);
    assert_eq!(
        test_paths(&selection),
        BTreeSet::from([POLY_RUST_UNIT, POLY_RUST_INTEGRATION])
    );
    assert!(
        selection.tests.values().all(|task| task == "rates#test"),
        "{:?}",
        selection.tests
    );
    assert_eq!(selection.uncovered, Vec::<String>::new());

    let selected = affected(
        dir.path(),
        &[
            "--changed",
            "crates/rates/src/convert.rs",
            "--tests",
            "cargo-test",
        ],
    );
    assert_eq!(
        selected,
        BTreeSet::from([POLY_RUST_UNIT.to_owned(), POLY_RUST_INTEGRATION.to_owned()])
    );

    // An integration test that changed selects itself only.
    let selected = affected(dir.path(), &["--changed", POLY_RUST_INTEGRATION]);
    assert_eq!(selected, BTreeSet::from([POLY_RUST_INTEGRATION.to_owned()]));
}

/// A TypeScript source file selects the tests that import it, and no Go or
/// Rust test.
#[cfg(unix)]
#[test]
fn polyglot_ts_change_selects_the_tests_that_import_it() {
    let dir = fixture("trace_polyglot");
    let selection = affected_json(dir.path(), &["--changed", "apps/web/src/price.ts"]);
    assert_eq!(test_paths(&selection), BTreeSet::from([POLY_PRICE]));
    assert_eq!(selection.tests[POLY_PRICE], "web#test");
    assert_eq!(selection.uncovered, Vec::<String>::new());

    let selected = affected(dir.path(), &["--changed", "apps/web/src/greeting.ts"]);
    assert_eq!(selected, BTreeSet::from([POLY_GREETING.to_owned()]));
}

/// A data file read by a JavaScript test (`fs.readFileSync`), a Go test
/// (`os.ReadFile`) and a Rust crate (`include_str!`) selects the tests of all
/// three, and only those.
#[cfg(unix)]
#[test]
fn polyglot_shared_file_selects_every_test_that_reads_it() {
    let dir = fixture("trace_polyglot");
    let readers = BTreeSet::from([
        POLY_JS_RATES,
        POLY_BOOK,
        POLY_RUST_UNIT,
        POLY_RUST_INTEGRATION,
    ]);
    let selection = affected_json(dir.path(), &["--changed", "shared/rates.json"]);
    assert_eq!(test_paths(&selection), readers);
    assert_eq!(selection.tests[POLY_JS_RATES], "web#test");
    assert_eq!(selection.tests[POLY_BOOK], "ledger#test");
    assert_eq!(selection.tests[POLY_RUST_UNIT], "rates#test");
    assert_eq!(selection.tests[POLY_RUST_INTEGRATION], "rates#test");
    assert_eq!(selection.uncovered, Vec::<String>::new());

    let output = trace(
        dir.path(),
        &[
            "affected",
            "--changed",
            "shared/rates.json",
            "--explain",
            POLY_BOOK,
            POLY_RUST_INTEGRATION,
            POLY_MONEY,
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.contains(&format!(
            "affected {POLY_BOOK}: modified shared/rates.json → {POLY_BOOK}"
        )),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "affected {POLY_RUST_INTEGRATION}: modified shared/rates.json → {POLY_RUST_UNIT} → \
             {POLY_RUST_INTEGRATION}"
        )),
        "{text}"
    );
    assert!(text.contains(&format!("unaffected {POLY_MONEY}")), "{text}");

    // The same from a working tree edit.
    fs::write(
        dir.path().join("shared/rates.json"),
        "{\n  \"EUR\": 109,\n  \"GBP\": 127\n}\n",
    )
    .unwrap();
    let selected = affected(dir.path(), &["--base", "HEAD"]);
    let selected: BTreeSet<&str> = selected.iter().map(String::as_str).collect();
    assert_eq!(selected, readers);
}

/// A file no test reads selects nothing, and no task runs whole.
#[cfg(unix)]
#[test]
fn polyglot_unrelated_change_selects_nothing() {
    let dir = fixture("trace_polyglot");
    let selection = affected_json(dir.path(), &["--changed", "README.md"]);
    assert_eq!(selection.tests, BTreeMap::new());
    assert_eq!(selection.uncovered, Vec::<String>::new());
    let output = trace(
        dir.path(),
        &["affected", "--changed", "README.md", "--exit-code"],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
}

// ---------------------------------------------------------------------------
// Gate on: Go strings that climb out of the test's directory
// (`trace_go_climb`).
//
// `services/billing` is a Go module. `config.Load(path)` reads a path it is
// given, so a test that calls it is bounded by the path-like strings of the
// code it loads. `go test` runs a package's tests in the package directory, so
// a relative string that climbs names its rest at the directories it climbs
// to, not its shape anywhere: `config_test.go` (in `services/billing/config`)
// reads `../../../package.json` and `../../../shared/defaults.json` (and has
// the string `shared/defaults.json` without parent steps too), `store_test.go`
// joins `../../../../../apps/admin/settings.json` onto a base below its
// package directory, and `harness_test.go` reads below the repository root,
// which it finds from its own location (`runtime.Caller`).

#[cfg(unix)]
const CLIMB_CONFIG: &str = "services/billing/config/config_test.go";
#[cfg(unix)]
const CLIMB_STORE: &str = "services/billing/store/store_test.go";
#[cfg(unix)]
const CLIMB_HARNESS: &str = "services/billing/harness/harness_test.go";

/// A climbing string names the file it climbs to, and the same name elsewhere
/// no longer selects the test.
#[cfg(unix)]
#[test]
fn go_climbing_string_names_only_where_it_climbs_to() {
    let dir = fixture("trace_go_climb");
    // Another package's manifest of the same name as the root one.
    let selected = affected(dir.path(), &["--changed", "apps/admin/package.json"]);
    assert_selects(&selected, &[CLIMB_HARNESS], &[CLIMB_CONFIG, CLIMB_STORE]);
    // The root manifest it climbs to.
    let selected = affected(dir.path(), &["--changed", "package.json"]);
    assert_selects(&selected, &[CLIMB_CONFIG], &[]);
    // The data file it climbs to, and the reason.
    let selected = affected(dir.path(), &["--changed", "shared/defaults.json"]);
    assert_selects(&selected, &[CLIMB_CONFIG, CLIMB_HARNESS], &[CLIMB_STORE]);
    let output = trace(
        dir.path(),
        &[
            "affected",
            "--changed",
            "shared/defaults.json",
            "--explain",
            CLIMB_CONFIG,
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.contains(
            "the string 'shared/defaults.json' with parent steps, 3 up from the working directory \
             (services/billing/config/)"
        ),
        "{text}"
    );
    // A file of that shape in another directory is not where it climbs to.
    let selected = affected(
        dir.path(),
        &["--changed", "services/legacy/shared/defaults.json"],
    );
    assert_selects(&selected, &[CLIMB_HARNESS], &[CLIMB_CONFIG, CLIMB_STORE]);
}

/// The same name without parent steps still names it inside the package; a
/// base below the package directory and the file's own location are still
/// covered.
#[cfg(unix)]
#[test]
fn go_climbing_string_keeps_its_package_name_deeper_bases_and_locations() {
    let dir = fixture("trace_go_climb");
    let selected = affected(
        dir.path(),
        &[
            "--changed",
            "services/billing/config/testdata/shared/defaults.json",
        ],
    );
    // (`store_test.go` too: a test file with parent steps names everything in its
    // package from its working directory.)
    assert_selects(&selected, &[CLIMB_CONFIG, CLIMB_HARNESS, CLIMB_STORE], &[]);
    // `filepath.Join("testdata/golden", "../../../../../apps/admin/settings.json")`
    // from `services/billing/store` is `apps/admin/settings.json`.
    let selected = affected(dir.path(), &["--changed", "apps/admin/settings.json"]);
    assert_selects(&selected, &[CLIMB_STORE, CLIMB_HARNESS], &[CLIMB_CONFIG]);
    let selected = affected(dir.path(), &["--changed", "apps/web/settings.json"]);
    assert_selects(&selected, &[CLIMB_HARNESS], &[CLIMB_CONFIG, CLIMB_STORE]);
    let output = trace(
        dir.path(),
        &[
            "affected",
            "--changed",
            "apps/admin/settings.json",
            "--explain",
            CLIMB_STORE,
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.contains("assumed: a Go string with parent steps"),
        "{text}"
    );
}

// ---------------------------------------------------------------------------
// More Go tests that read a file through a climbing string (each passes under
// `go test`, and each is selected under `--strict`): the default selects them
// when the file changes, except where the stated assumption says it does not.

/// `trace_go_climb` with more files, committed on `main`.
#[cfg(unix)]
fn climb_fixture_with(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    setup::copy_fixture("trace_go_climb", dir.path()).unwrap();
    for (path, text) in files {
        let path = dir.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    setup::setup_git(dir.path()).unwrap();
    dir
}

/// The test `test` reads `changed`: `--strict` selects it (the fixture is
/// what it says), and so must the default.
#[cfg(unix)]
#[track_caller]
fn assert_climb_selects(files: &[(&str, &str)], changed: &str, test: &str) {
    let dir = climb_fixture_with(files);
    let strict = affected(dir.path(), &["--changed", changed, "--strict"]);
    assert_selects(&strict, &[test], &[]);
    let selected = affected(dir.path(), &["--changed", changed]);
    assert_selects(&selected, &[test], &[]);
}

/// As [`assert_climb_selects`], for a case the stated assumption excludes (a
/// base or a working directory that another package sets): `--strict`
/// selects the test, the default does not, and `--explain` states the
/// assumption.
#[cfg(unix)]
#[track_caller]
fn assert_climb_assumed(files: &[(&str, &str)], changed: &str, test: &str) {
    let dir = climb_fixture_with(files);
    let strict = affected(dir.path(), &["--changed", changed, "--strict"]);
    assert_selects(&strict, &[test], &[]);
    let selected = affected(dir.path(), &["--changed", changed]);
    assert_selects(&selected, &[], &[test]);
    let output = trace(
        dir.path(),
        &["affected", "--changed", changed, "--explain", test],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.contains("a directory another package finds from its own location")
            && text.contains("also in the init of an imported package"),
        "{text}"
    );
}

/// `base + ".json"`: the climbing string is the path without its extension.
/// A shape names a file by its stem too (`Keys`), the place it climbs to does
/// not (`below`).
#[cfg(unix)]
#[test]
fn go_climbing_string_completed_with_an_extension() {
    assert_climb_selects(
        &[
            ("shared/golden.json", "{}\n"),
            (
                "services/billing/golden/golden_test.go",
                r#"package golden

import (
	"testing"

	"example.com/billing/config"
)

func TestGolden(t *testing.T) {
	base := "../../../shared/golden"
	for _, ext := range []string{".json"} {
		if _, err := config.Load(base + ext); err != nil {
			t.Fatal(err)
		}
	}
}
"#,
            ),
        ],
        "shared/golden.json",
        "services/billing/golden/golden_test.go",
    );
}

/// A base below the working directory that lies in another package (a Go
/// module at the repository root, a test there over a workspace package): the
/// path stays below the working directory, which the stated assumption allows,
/// but the shape is only looked for in the test's own package, the root one.
#[cfg(unix)]
#[test]
fn go_climbing_string_on_a_base_in_a_nested_package() {
    assert_climb_selects(
        &[
            ("go.mod", "module example.com/rootmod\n\ngo 1.22\n"),
            (
                "read/read.go",
                "package read\n\nimport \"os\"\n\n// Load reads the file at path.\nfunc Load(path \
                 string) ([]byte, error) {\n\treturn os.ReadFile(path)\n}\n",
            ),
            (
                "apps_test.go",
                r#"package rootmod

import (
	"path/filepath"
	"testing"

	"example.com/rootmod/read"
)

func TestApps(t *testing.T) {
	for _, src := range []string{"apps/web/src"} {
		if _, err := read.Load(filepath.Join(src, "../settings.json")); err != nil {
			t.Fatal(err)
		}
	}
}
"#,
            ),
        ],
        "apps/web/settings.json",
        "apps_test.go",
    );
}

/// A second climb in the same file: the base is a variable of the test.
#[cfg(unix)]
#[test]
fn go_climbing_string_on_a_climbing_base_of_the_same_file() {
    assert_climb_selects(
        &[(
            "services/billing/twice/twice_test.go",
            r#"package twice

import (
	"path/filepath"
	"testing"

	"example.com/billing/config"
)

func TestShared(t *testing.T) {
	root := filepath.Join("..", "..")
	if _, err := config.Load(filepath.Join(root, "../shared/defaults.json")); err != nil {
		t.Fatal(err)
	}
}
"#,
        )],
        "shared/defaults.json",
        "services/billing/twice/twice_test.go",
    );
}

/// The same inside one `Join` call, with a variable between the literals.
#[cfg(unix)]
#[test]
fn go_climbing_literals_around_a_variable_in_one_join() {
    assert_climb_selects(
        &[(
            "services/billing/split/split_test.go",
            r#"package split

import (
	"path/filepath"
	"testing"

	"example.com/billing/config"
)

func TestShared(t *testing.T) {
	for _, svc := range []string{"billing"} {
		path := filepath.Join("..", "..", svc, "..", "..", "shared", "defaults.json")
		if _, err := config.Load(path); err != nil {
			t.Fatal(err)
		}
	}
}
"#,
        )],
        "shared/defaults.json",
        "services/billing/split/split_test.go",
    );
}

/// `filepath.Dir` in a loop counts once.
#[cfg(unix)]
#[test]
fn go_climbing_string_after_a_loop_of_dir_calls() {
    assert_climb_selects(
        &[(
            "services/billing/looped/looped_test.go",
            r#"package looped

import (
	"os"
	"path/filepath"
	"testing"

	"example.com/billing/config"
)

func TestShared(t *testing.T) {
	dir, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	for i := 0; i < 2; i++ {
		dir = filepath.Dir(dir)
	}
	if _, err := config.Load(filepath.Join(dir, "../shared/defaults.json")); err != nil {
		t.Fatal(err)
	}
}
"#,
        )],
        "shared/defaults.json",
        "services/billing/looped/looped_test.go",
    );
}

/// The climbing base is a constant in another file of the same package.
#[cfg(unix)]
#[test]
fn go_climbing_string_on_a_constant_of_a_sibling_file() {
    assert_climb_selects(
        &[
            (
                "services/billing/sibling/paths.go",
                "package sibling\n\n// Root is the services directory, from this package.\nconst \
                 Root = \"../..\"\n",
            ),
            (
                "services/billing/sibling/sibling_test.go",
                r#"package sibling

import (
	"path/filepath"
	"testing"

	"example.com/billing/config"
)

func TestShared(t *testing.T) {
	if _, err := config.Load(filepath.Join(Root, "../shared/defaults.json")); err != nil {
		t.Fatal(err)
	}
}
"#,
            ),
        ],
        "shared/defaults.json",
        "services/billing/sibling/sibling_test.go",
    );
}

/// `TestMain` in another file of the package changes the working directory.
#[cfg(unix)]
#[test]
fn go_chdir_in_the_test_main_of_a_sibling_file() {
    assert_climb_selects(
        &[
            (
                "services/billing/moved/main_test.go",
                r#"package moved

import (
	"os"
	"testing"
)

func TestMain(m *testing.M) {
	if err := os.Chdir("../.."); err != nil {
		panic(err)
	}
	os.Exit(m.Run())
}
"#,
            ),
            (
                "services/billing/moved/moved_test.go",
                r#"package moved

import (
	"testing"

	"example.com/billing/config"
)

func TestShared(t *testing.T) {
	name := "../shared/defaults.json"
	if _, err := config.Load(name); err != nil {
		t.Fatal(err)
	}
}
"#,
            ),
        ],
        "shared/defaults.json",
        "services/billing/moved/moved_test.go",
    );
}

/// The base is the location of a helper package (`runtime.Caller` there): the
/// stated assumption excludes it, so the default does not select the test.
#[cfg(unix)]
#[test]
fn go_climbing_string_on_the_location_of_a_helper_package() {
    assert_climb_assumed(
        &[
            (
                "services/billing/testutil/testutil.go",
                r#"// Package testutil has helpers for tests.
package testutil

import (
	"path/filepath"
	"runtime"
)

// Root is the module directory, from this file's location.
func Root() string {
	_, file, _, _ := runtime.Caller(0)
	return filepath.Join(filepath.Dir(file), "..")
}
"#,
            ),
            (
                "services/billing/based/deep/deep_test.go",
                r#"package deep

import (
	"path/filepath"
	"testing"

	"example.com/billing/config"
	"example.com/billing/testutil"
)

func TestShared(t *testing.T) {
	path := filepath.Join(testutil.Root(), "../../shared/defaults.json")
	if _, err := config.Load(path); err != nil {
		t.Fatal(err)
	}
}
"#,
            ),
        ],
        "shared/defaults.json",
        "services/billing/based/deep/deep_test.go",
    );
}

/// A helper package imported for its `init`, which moves to the module
/// directory: the stated assumption excludes it, so the default does not
/// select the test.
#[cfg(unix)]
#[test]
fn go_chdir_in_the_init_of_an_imported_package() {
    assert_climb_assumed(
        &[
            (
                "services/billing/testinit/init.go",
                r#"// Package testinit moves the tests that import it to the module directory.
package testinit

import (
	"os"
	"path/filepath"
	"runtime"
)

func init() {
	_, file, _, _ := runtime.Caller(0)
	if err := os.Chdir(filepath.Join(filepath.Dir(file), "..")); err != nil {
		panic(err)
	}
}
"#,
            ),
            (
                "services/billing/inited/deep/deep_test.go",
                r#"package deep

import (
	"testing"

	"example.com/billing/config"
	_ "example.com/billing/testinit"
)

func TestShared(t *testing.T) {
	name := "../../shared/defaults.json"
	if _, err := config.Load(name); err != nil {
		t.Fatal(err)
	}
}
"#,
            ),
        ],
        "shared/defaults.json",
        "services/billing/inited/deep/deep_test.go",
    );
}

/// `--strict` opens a read with a computed path: every test that has one is
/// selected, whatever its strings say.
#[cfg(unix)]
#[test]
fn go_climbing_strings_assume_nothing_under_strict() {
    let dir = fixture("trace_go_climb");
    for path in [
        "apps/admin/package.json",
        "services/legacy/shared/defaults.json",
    ] {
        let selected = affected(dir.path(), &["--changed", path, "--strict"]);
        assert_selects(&selected, &[CLIMB_CONFIG, CLIMB_STORE, CLIMB_HARNESS], &[]);
    }
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

/// A remote cache that holds the artifacts of the local cache directory
/// `cache`, read-only: `HEAD` and `GET /v8/artifacts/<key>` answer for each of
/// them, and everything else is a 404 (also the batched existence query, as on
/// a server without that endpoint).
#[cfg(unix)]
fn remote_cache_of(cache: &Path) -> httpmock::MockServer {
    let server = httpmock::MockServer::start();
    server.mock(|when, then| {
        when.path("/v8/artifacts/status");
        then.status(200)
            .json_body(serde_json::json!({"status": "enabled"}));
    });
    for name in cached_artifacts(cache) {
        let key = name.strip_suffix(".tar.zst").unwrap().to_owned();
        let body = fs::read(cache.join(&name)).unwrap();
        server.mock(|when, then| {
            when.path(format!("/v8/artifacts/{key}"));
            then.status(200).body(body);
        });
    }
    server
}

/// Every `transport.graph` below `root` (the packed graph's name while it is
/// fetched or put), outside `.git`.
#[cfg(unix)]
fn transport_files(root: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                if entry.file_name() != ".git" {
                    walk(root, &path, out);
                }
            } else if entry.file_name() == "transport.graph" {
                out.push(path.strip_prefix(root).unwrap().display().to_string());
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out
}

/// A test that reads every entry of the repository root: any change selects
/// it, also a file added at the root or below `.turbo`.
#[cfg(unix)]
const JEST_ROOT: &str = "packages/app/test/root.test.js";

/// The `trace_jest` fixture as a repository that does not ignore `.turbo`,
/// with [`JEST_ROOT`], committed on `main`.
#[cfg(unix)]
fn fixture_without_dot_turbo_ignored() -> tempfile::TempDir {
    let dir = fixture("trace_jest");
    fs::write(dir.path().join(".gitignore"), "node_modules/\n.npmrc\n").unwrap();
    fs::write(
        dir.path().join(JEST_ROOT),
        "const { readdirSync, readFileSync } = require('fs');\nconst { join } = \
         require('path');\nconst root = join(__dirname, '../../..');\ntest('root', () => {\n  for \
         (const name of readdirSync(root)) {\n    readFileSync(join(root, name));\n  }\n});\n",
    )
    .unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "--quiet", "-m", "root listing"]);
    dir
}

/// The selection of `turbo trace <args>` with `env`, which must succeed, and
/// its stderr.
#[cfg(unix)]
#[track_caller]
fn selected(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> (Vec<String>, String) {
    let output = trace_with_env(dir, args, env);
    assert!(output.status.success(), "{}", stderr(&output));
    (
        stdout(&output).lines().map(str::to_owned).collect(),
        stderr(&output),
    )
}

/// The graph of the merge base comes from the remote cache into the graph
/// directory, is installed, and is gone afterwards: the packed graph is in no
/// place of the working tree, and is no change of the repository, which here
/// does not ignore `.turbo`.
#[cfg(unix)]
#[test]
fn the_graph_from_the_remote_cache_is_installed_and_is_not_a_change() {
    let scratch = tempfile::tempdir().unwrap();
    let cache = scratch.path().join("turbo-cache");
    let none = Vec::<String>::new();

    // CI on `main` builds the graph and puts it into the cache.
    let main = fixture_without_dot_turbo_ignored();
    let (tests, err) = selected(
        main.path(),
        &[
            "affected",
            "--base",
            "HEAD",
            "--cache-dir",
            cache.to_str().unwrap(),
        ],
        &[("CI", "1")],
    );
    assert!(err.contains("put the graph of"), "{err}");
    assert_eq!(tests, none);
    assert_eq!(transport_files(main.path()), none);

    // The same artifacts in a remote cache, and a fresh clone that reads only
    // that cache, on a branch with a commit that changes no file.
    let server = remote_cache_of(&cache);
    let api = server.base_url();
    let remote = [
        ("TURBO_API", api.as_str()),
        ("TURBO_TOKEN", "token"),
        ("TURBO_TEAM", "team"),
    ];
    let fresh = scratch.path().join("fresh");
    clone(main.path(), &fresh);
    git(&fresh, &["checkout", "--quiet", "-b", "feature"]);
    git(
        &fresh,
        &["commit", "--quiet", "--allow-empty", "-m", "nothing"],
    );
    let args = ["affected", "--base", "main", "--cache=remote:r"];
    let (tests, err) = selected(&fresh, &args, &remote);
    assert_eq!(tests, none, "{err}");
    assert_eq!(transport_files(&fresh), none);
    assert!(err.contains("restored the graph of"), "{err}");
    // A second run in the same checkout finds the graph.
    let (tests, err) = selected(&fresh, &args, &remote);
    assert_eq!(tests, none, "{err}");
    assert!(!err.contains("restored"), "{err}");

    // A packed graph that an interrupted run left in the graph directory is
    // no change either.
    fs::write(fresh.join(".turbo/trace/transport.graph"), "left behind\n").unwrap();
    let (tests, err) = selected(&fresh, &args, &remote);
    assert_eq!(tests, none, "{err}");
}

/// What the graph's transport writes into turbo's local cache directory is no
/// change of a repository that does not ignore `.turbo`: the entry CI puts
/// there, and the entry a fetch from the remote cache leaves there.
#[cfg(unix)]
#[test]
fn the_cache_entries_of_the_graph_are_not_a_change() {
    let none = Vec::<String>::new();
    let main = fixture_without_dot_turbo_ignored();
    let args = ["affected", "--base", "HEAD"];
    let (tests, err) = selected(main.path(), &args, &[("CI", "1")]);
    assert!(err.contains("put the graph of"), "{err}");
    assert_eq!(tests, none);
    let cache = main.path().join(".turbo/cache");
    assert_eq!(cached_artifacts(&cache).len(), 1);
    // The cache entry is there now, untracked.
    let (tests, err) = selected(main.path(), &args, &[("CI", "1")]);
    assert_eq!(tests, none, "{err}");

    // A fresh clone fetches from the remote cache, and the local cache takes
    // the entry before the changes are read.
    let server = remote_cache_of(&cache);
    let api = server.base_url();
    let remote = [
        ("TURBO_API", api.as_str()),
        ("TURBO_TOKEN", "token"),
        ("TURBO_TEAM", "team"),
    ];
    let scratch = tempfile::tempdir().unwrap();
    let fresh = scratch.path().join("fresh");
    clone(main.path(), &fresh);
    git(&fresh, &["checkout", "--quiet", "-b", "feature"]);
    git(
        &fresh,
        &["commit", "--quiet", "--allow-empty", "-m", "nothing"],
    );
    let args = ["affected", "--base", "main", "--cache=local:rw,remote:r"];
    let (tests, err) = selected(&fresh, &args, &remote);
    assert!(err.contains("restored the graph of"), "{err}");
    assert_eq!(cached_artifacts(&fresh.join(".turbo/cache")).len(), 1);
    assert_eq!(tests, none, "{err}");

    // Another file in the cache directory is a change.
    fs::write(fresh.join(".turbo/cache/notes.txt"), "n\n").unwrap();
    let (tests, _) = selected(&fresh, &args, &remote);
    assert_eq!(tests, [JEST_ROOT]);
}

/// A file of the repository that is named like a cache entry, in turbo's cache
/// directory, is a file like any other: only the entries git does not track
/// are left out of the changes.
#[cfg(unix)]
#[test]
fn a_tracked_file_named_like_a_cache_entry_is_a_change() {
    let none = Vec::<String>::new();
    let main = fixture_without_dot_turbo_ignored();
    let entry = ".turbo/cache/0123456789abcdef-meta.json";
    let args = ["affected", "--base", "main"];
    git(main.path(), &["checkout", "--quiet", "-b", "feature"]);
    let (tests, err) = selected(main.path(), &args, &[]);
    assert_eq!(tests, none, "{err}");

    // Untracked, it is an entry the cache wrote.
    fs::create_dir_all(main.path().join(".turbo/cache")).unwrap();
    fs::write(main.path().join(entry), "{}\n").unwrap();
    let (tests, err) = selected(main.path(), &args, &[]);
    assert_eq!(tests, none, "{err}");

    // Added to the index, and committed, it is a file of the repository.
    git(main.path(), &["add", "-f", entry]);
    let (tests, err) = selected(main.path(), &args, &[]);
    assert_eq!(tests, [JEST_ROOT], "{err}");
    git(main.path(), &["commit", "--quiet", "-m", "a fixture"]);
    let (tests, err) = selected(main.path(), &args, &[]);
    assert_eq!(tests, [JEST_ROOT], "{err}");

    // Tracked at the base only (the branch removed it), and written again: the
    // file of the base changed.
    git(main.path(), &["checkout", "--quiet", "main"]);
    git(main.path(), &["merge", "--quiet", "--ff-only", "feature"]);
    git(main.path(), &["checkout", "--quiet", "feature"]);
    git(main.path(), &["rm", "--quiet", entry]);
    git(main.path(), &["commit", "--quiet", "-m", "no fixture"]);
    fs::create_dir_all(main.path().join(".turbo/cache")).unwrap();
    fs::write(main.path().join(entry), "{\"changed\": true}\n").unwrap();
    let (tests, err) = selected(main.path(), &args, &[]);
    assert_eq!(tests, [JEST_ROOT], "{err}");
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

// ---------------------------------------------------------------------------
// Gate on: package scripts that start `go test` (`trace_go_scripts`).
//
// Each service's `test` script starts `go test` another way: `cd app && go
// test ./...` (`billing`), `go -C engine test -race ./...` (`search`),
// `gotestsum … -- -tags=integration ./...` (`queue`, with a module of its own
// in `tools`), `go test ./one/... ./two/...` over a `go.work` (`batch`), a
// shell script file (`mail`) and one without an extension (`ops`). The root's
// `test` script is `turbo run test`, and `tools/lint` is a module in the root
// package.

/// One selected test of `--json`: its package, its task (empty: none) and the
/// type of its reason.
#[cfg(unix)]
#[derive(Debug, PartialEq, Eq)]
struct Row {
    package: String,
    task: String,
    reason: String,
}

#[cfg(unix)]
fn row(package: &str, task: &str, reason: &str) -> Row {
    Row {
        package: package.to_owned(),
        task: task.to_owned(),
        reason: reason.to_owned(),
    }
}

/// `turbo trace affected --json <args>`: the selected tests by path.
#[cfg(unix)]
fn affected_rows(dir: &Path, args: &[&str]) -> BTreeMap<String, Row> {
    let mut words = vec!["affected", "--json"];
    words.extend_from_slice(args);
    let output = trace(dir, &words);
    assert!(
        output.status.success(),
        "turbo trace {words:?} failed\nstdout:\n{}\nstderr:\n{}",
        stdout(&output),
        stderr(&output)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["uncoveredTasks"]["length"], 0, "{json}");
    json["affectedTests"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["path"].as_str().unwrap().to_owned(),
                Row {
                    package: item["package"].as_str().unwrap().to_owned(),
                    task: item["task"].as_str().unwrap_or_default().to_owned(),
                    reason: item["reason"]["__typename"].as_str().unwrap().to_owned(),
                },
            )
        })
        .collect()
}

#[cfg(unix)]
fn rows(expected: &[(&str, Row)]) -> BTreeMap<String, Row> {
    expected
        .iter()
        .map(|(path, r)| ((*path).to_owned(), row(&r.package, &r.task, &r.reason)))
        .collect()
}

/// Replaces `from` with `to` in the file at `rel`, which has it once.
#[cfg(unix)]
fn edit(root: &Path, rel: &str, from: &str, to: &str) {
    let path = root.join(rel);
    let text = fs::read_to_string(&path).unwrap();
    assert_eq!(text.matches(from).count(), 1, "{rel}: {from}");
    fs::write(&path, text.replace(from, to)).unwrap();
}

/// A Go test belongs to turbo's package around it, and its task is the script
/// of that package that starts it: also where the script runs `go test` in a
/// directory below the package, which is the Go module.
#[cfg(unix)]
#[test]
fn a_go_test_has_the_task_of_the_script_that_starts_it() {
    let dir = fixture("trace_go_scripts");
    for (changed, test, package) in [
        (
            "services/billing/app/invoice/invoice.go",
            "services/billing/app/invoice/invoice_test.go",
            "billing",
        ),
        (
            "services/search/engine/index/index.go",
            "services/search/engine/index/index_test.go",
            "search",
        ),
        (
            "services/queue/worker/worker.go",
            "services/queue/worker/worker_test.go",
            "queue",
        ),
        (
            "services/batch/two/two.go",
            "services/batch/two/two_test.go",
            "batch",
        ),
        (
            "services/mail/smtp/smtp.go",
            "services/mail/smtp/smtp_test.go",
            "mail",
        ),
        // A script file that is not read command by command: its text names
        // `go test`.
        ("services/ops/ops.go", "services/ops/ops_test.go", "ops"),
    ] {
        assert_eq!(
            affected_rows(dir.path(), &["--changed", changed]),
            rows(&[(
                test,
                row(package, &format!("{package}#test"), "FileChanged")
            )]),
            "{changed}"
        );
        // The filter keeps the tests of the package it names.
        assert_eq!(
            affected(dir.path(), &["--changed", changed, "--filter", package]),
            BTreeSet::from([test.to_owned()]),
            "{changed}"
        );
    }
    // `./...` stays in its module: no script starts the tests of the module
    // below the package's.
    assert_eq!(
        affected_rows(dir.path(), &["--changed", "services/queue/tools/tools.go"]),
        rows(&[(
            "services/queue/tools/tools_test.go",
            row("queue", "", "FileChanged")
        )])
    );
    // The root's `turbo run test` starts the scripts of the other packages:
    // their tests are not the root script's, and no script tests the module in
    // the root package.
    assert_eq!(
        affected_rows(dir.path(), &["--changed", "tools/lint/lint.go"]),
        rows(&[("tools/lint/lint_test.go", row("//", "", "FileChanged"))])
    );
}

/// A script whose text changed can run its tests another way: every test it
/// starts is selected, and no test of another script.
#[cfg(unix)]
#[test]
fn a_changed_script_selects_the_tests_it_starts() {
    let dir = fixture("trace_go_scripts");
    let changed = |package: &str| row(package, &format!("{package}#test"), "TaskChanged");
    for (package, from, to, tests) in [
        (
            "billing",
            "go test ./...",
            "go test -race ./...",
            [
                "services/billing/app/billing_test.go",
                "services/billing/app/invoice/invoice_test.go",
            ],
        ),
        (
            "search",
            "-race",
            "-race -count=1",
            [
                "services/search/engine/index/index_test.go",
                "services/search/engine/search_test.go",
            ],
        ),
        (
            "queue",
            "-tags=integration",
            "-tags=integration,slow",
            [
                "services/queue/queue_test.go",
                "services/queue/worker/worker_test.go",
            ],
        ),
        (
            "batch",
            "go test",
            "go test -short",
            [
                "services/batch/one/one_test.go",
                "services/batch/two/two_test.go",
            ],
        ),
        (
            "mail",
            "./scripts/test.sh",
            "./scripts/test.sh --verbose",
            [
                "services/mail/mail_test.go",
                "services/mail/smtp/smtp_test.go",
            ],
        ),
    ] {
        let manifest = format!("services/{package}/package.json");
        edit(dir.path(), &manifest, from, to);
        let expected = rows(&[(tests[0], changed(package)), (tests[1], changed(package))]);
        assert_eq!(
            affected_rows(dir.path(), &["--base", "HEAD"]),
            expected,
            "{package}"
        );
        // Without a base revision to compare with, the scripts of a changed
        // manifest count as changed (a test next to the manifest is selected
        // as a reader of its directory already).
        git(dir.path(), &["checkout", "--quiet", "--", &manifest]);
        let selected = affected_rows(dir.path(), &["--changed", &manifest]);
        assert_eq!(
            selected.keys().collect::<Vec<_>>(),
            expected.keys().collect::<Vec<_>>(),
            "{package}"
        );
        assert!(
            selected
                .values()
                .all(|r| r.task == format!("{package}#test")),
            "{package}: {selected:?}"
        );
    }

    // Another field of the manifest starts no test differently.
    edit(
        dir.path(),
        "services/billing/package.json",
        "\"version\": \"1.0.0\"",
        "\"version\": \"1.0.1\"",
    );
    assert_eq!(
        affected_rows(dir.path(), &["--base", "HEAD"]),
        BTreeMap::new()
    );
    git(dir.path(), &["checkout", "--quiet", "--", "."]);

    // The shell script file a script runs is the script's command too.
    edit(
        dir.path(),
        "services/mail/scripts/test.sh",
        "go test ./...",
        "go test -race ./...",
    );
    assert_eq!(
        affected_rows(dir.path(), &["--base", "HEAD"]),
        rows(&[
            ("services/mail/mail_test.go", changed("mail")),
            ("services/mail/smtp/smtp_test.go", changed("mail")),
        ])
    );
}

/// The same for a JavaScript runner: `jest` that becomes `jest --ci`.
#[cfg(unix)]
#[test]
fn a_changed_jest_script_selects_the_tests_it_starts() {
    let dir = fixture("trace_jest");
    edit(
        dir.path(),
        "packages/math/package.json",
        "\"test\": \"jest\"",
        "\"test\": \"jest --ci\"",
    );
    let changed = row("math", "math#test", "TaskChanged");
    assert_eq!(
        affected_rows(dir.path(), &["--base", "HEAD"]),
        rows(&[
            (JEST_ADD, row("math", "math#test", "TaskChanged")),
            (JEST_MUL, changed),
        ])
    );
}
