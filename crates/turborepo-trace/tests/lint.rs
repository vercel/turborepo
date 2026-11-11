//! `turbotrace lint` end to end on a small jest repo: the rule
//! `unconstrained-load`, the baseline, suppression comments, the configuration
//! and the output formats.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{path::Path, process::Command};

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// Exit code, stdout and stderr of `turbotrace <args>` with extra environment
/// variables.
fn run_env(root: &Path, args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
    let mut c = Command::new(support::harness());
    c.args(args).current_dir(root);
    for (k, v) in env {
        c.env(k, v);
    }
    let o = c.output().expect("run turbotrace");
    (
        o.status.code().expect("exit code"),
        String::from_utf8(o.stdout).unwrap(),
        String::from_utf8(o.stderr).unwrap(),
    )
}

fn run(root: &Path, args: &[&str]) -> (i32, String, String) {
    run_env(root, args, &[])
}

/// `turbotrace lint --tests jest <extra>`: exit code and stdout.
fn lint(root: &Path, extra: &[&str]) -> (i32, String) {
    let mut args = vec!["lint", "--tests", "jest"];
    args.extend_from_slice(extra);
    let (code, out, err) = run(root, &args);
    assert!(code == 0 || code == 1, "turbotrace {args:?}: {err}");
    (code, out)
}

fn lint_json(root: &Path, extra: &[&str]) -> (i32, serde_json::Value) {
    let mut args = vec!["--format", "json"];
    args.extend_from_slice(extra);
    let (code, out) = lint(root, &args);
    (code, serde_json::from_str(&out).expect("json"))
}

const LOAD: &str = "exports.load = function (name) {\n  return require(name);\n};\n";
const MESSAGE: &str =
    "the specifier is the parameter `name` of `exports.load`: any module can be loaded here";
const KEY: &str = "computed_specifier/param: name; parameter exports.load#0";
const FIX: &str = "give the specifier a literal prefix: `import(`./handlers/${name}`)` or \
                   `require(path.join(__dirname, 'handlers', name))`";
const BASELINE: &str = "turbotrace-lint-baseline.json";

/// Three jest tests. `src/load.js` requires a module named by a parameter
/// (unconstrained). `src/prefix.js` loads with a literal prefix, a path
/// anchored to the file, and a literal table (all bounded).
///
/// ```text
/// test/a.test.js → src/load.js, src/prefix.js
/// test/b.test.js → src/load.js
/// test/c.test.js → src/plain.js
/// ```
fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(r, "jest.config.js", "module.exports = {};\n");
    write(r, "src/load.js", LOAD);
    write(
        r,
        "src/prefix.js",
        "const path = require('path');\nexports.a = (name) => \
         import(`./handlers/${name}`);\nexports.b = (name) => require(path.join(__dirname, \
         'handlers', name));\nconst HANDLERS = { a: () => import('./handlers/a') };\nexports.c = \
         (name) => HANDLERS[name]();\n",
    );
    write(r, "src/handlers/a.js", "module.exports = 1;\n");
    write(r, "src/plain.js", "module.exports = 1;\n");
    for (t, deps) in [
        ("a", &["load", "prefix"][..]),
        ("b", &["load"]),
        ("c", &["plain"]),
    ] {
        let mut src = String::new();
        for dep in deps {
            src.push_str(&format!("require('../src/{dep}');\n"));
        }
        src.push_str(&format!("test('{t}', () => {{}});\n"));
        write(r, &format!("test/{t}.test.js"), &src);
    }
    write(r, "docs/readme.txt", "x");
    d
}

/// A second unconstrained load, reached by test c.
fn add_other(r: &Path) {
    write(
        r,
        "src/other.js",
        "exports.other = function (mod) {\n  return require(mod);\n};\n",
    );
    write(
        r,
        "test/c.test.js",
        "require('../src/plain');\nrequire('../src/other');\ntest('c', () => {});\n",
    );
}

#[test]
fn an_unconstrained_load_is_a_new_finding() {
    let d = repo();
    let r = d.path();
    let (code, out) = lint(r, &[]);
    assert_eq!(code, 1, "{out}");
    let lines: Vec<&str> = out.lines().collect();
    let (head, fix) = (
        format!("src/load.js:2  unconstrained-load  {MESSAGE}"),
        format!("  fix: {FIX}"),
    );
    assert_eq!(
        lines,
        [
            head.as_str(),
            "  holds 2 tests on the floor (2 only this site)",
            fix.as_str(),
            "       or check it before the load: `if (!name.startsWith('./handlers/')) throw …` \
             (turbo trace then bounds the load to that directory)",
            "",
            "1 new, 0 baselined, 0 suppressed, 0 stale; 2 of 3 tests on the floor",
        ],
        "{out}"
    );
    // The literal prefix, the anchored path and the literal table are not findings.
    assert!(!out.contains("src/prefix.js"), "{out}");

    // Without --tests: every test framework.
    let (code, all, _) = run(r, &["lint"]);
    assert_eq!((code, all.as_str()), (1, out.as_str()));
    // The graph as stored, with timings.
    let (code, _, err) = run(r, &["lint", "--no-refresh", "--stats"]);
    assert_eq!(code, 1);
    assert!(
        err.lines()
            .any(|l| l.starts_with("lint: 1 finding(s) from 2 rule(s), 3 entr(ies);")),
        "{err}"
    );
}

#[test]
fn baseline_round_trip() {
    let d = repo();
    let r = d.path();
    let (code, out) = lint(r, &["--update-baseline"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        out,
        "turbotrace-lint-baseline.json: 1 finding(s) written as 1 entr(ies)\n"
    );
    assert_eq!(
        std::fs::read_to_string(r.join(BASELINE)).unwrap(),
        format!(
            "{{\n  \"version\": 1,\n  \"entries\": [\n    {{ \"rule\": \"unconstrained-load\", \
             \"file\": \"src/load.js\", \"key\": \"{KEY}\", \"count\": 1 }}\n  ]\n}}\n"
        )
    );
    // Accepted: listed with its cost, exit 0.
    let (code, out) = lint(r, &[]);
    assert_eq!(code, 0, "{out}");
    let row = format!("       2  src/load.js:2  unconstrained-load  {MESSAGE}");
    assert_eq!(
        out.lines().collect::<Vec<_>>(),
        [
            "baselined: 1 finding(s) accepted by lint-baseline.json, by the tests they hold on \
             the floor",
            row.as_str(),
            "",
            "0 new, 1 baselined, 0 suppressed, 0 stale; 2 of 3 tests on the floor",
        ],
        "{out}"
    );
    // --no-baseline: new again.
    assert_eq!(lint(r, &["--no-baseline"]).0, 1);

    // The load moves to another line: still baselined.
    write(r, "src/load.js", &format!("// loads plugins\n\n{LOAD}"));
    let (code, out) = lint(r, &[]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("  src/load.js:4  unconstrained-load  "),
        "{out}"
    );

    // A second unconstrained load: only it is new.
    add_other(r);
    let (code, v) = lint_json(r, &[]);
    assert_eq!(code, 1, "{v}");
    let status: Vec<(&str, &str)> = v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["file"].as_str().unwrap(), f["status"].as_str().unwrap()))
        .collect();
    assert_eq!(
        status,
        [("src/load.js", "baselined"), ("src/other.js", "new")]
    );
    assert_eq!(v["summary"]["new"], 1);
    assert_eq!(v["summary"]["baselined"], 1);
    let (_, out) = lint(r, &[]);
    assert!(
        out.starts_with("src/other.js:2  unconstrained-load  "),
        "{out}"
    );
    assert!(
        out.contains("1 new, 1 baselined, 0 suppressed, 0 stale; 3 of 3 tests"),
        "{out}"
    );

    // A copy of a baselined load in the same function has the same key: one is
    // accepted, one is new.
    write(
        r,
        "src/load.js",
        "exports.load = function (name) {\n  require(name);\n  return require(name);\n};\n",
    );
    let (_, v) = lint_json(r, &[]);
    let status: Vec<(u64, &str)> = v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["file"] == "src/load.js")
        .map(|f| (f["line"].as_u64().unwrap(), f["status"].as_str().unwrap()))
        .collect();
    assert_eq!(status, [(2, "baselined"), (3, "new")], "{v}");
    write(r, "src/load.js", LOAD);

    // Accept both; then the first load is fixed: its entry is stale, and that
    // fails.
    assert_eq!(lint(r, &["--update-baseline"]).0, 0);
    assert_eq!(lint(r, &[]).0, 0);
    write(
        r,
        "src/load.js",
        "exports.load = function () {\n  return require('./plain');\n};\n",
    );
    let (code, out) = lint(r, &[]);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains(
            "stale baseline entries: lint-baseline.json lists findings that are gone. Update the \
             baseline and commit the file."
        ),
        "{out}"
    );
    assert!(
        out.contains(&format!("  src/load.js  unconstrained-load  {KEY}\n")),
        "{out}"
    );
    assert!(
        out.contains("0 new, 1 baselined, 0 suppressed, 1 stale; 1 of 3 tests"),
        "{out}"
    );
    let (_, v) = lint_json(r, &[]);
    assert_eq!(
        v["stale"],
        serde_json::json!([{
            "rule": "unconstrained-load", "file": "src/load.js", "key": KEY, "count": 1,
            "severity": "error",
        }])
    );
    assert_eq!(v["summary"]["ok"], false);
    // The remedy.
    assert_eq!(lint(r, &["--update-baseline"]).0, 0);
    let (code, out) = lint(r, &[]);
    assert_eq!(code, 0, "{out}");
    assert!(
        !std::fs::read_to_string(r.join(BASELINE))
            .unwrap()
            .contains("src/load.js")
    );

    // Another file, relative to the current directory.
    assert_eq!(lint(r, &["--baseline", "elsewhere.json"]).0, 1);
    assert_eq!(
        lint(r, &["--baseline", "elsewhere.json", "--update-baseline"]).0,
        0
    );
    assert_eq!(lint(r, &["--baseline", "elsewhere.json"]).0, 0);
    // A baseline that is not one is an error, not an empty baseline.
    write(r, "broken.json", "{\"version\": 9, \"entries\": []}\n");
    let (code, _, err) = run(r, &["lint", "--baseline", "broken.json"]);
    assert_eq!(code, 1);
    assert!(
        err.contains("broken.json") && err.contains("format version 9"),
        "{err}"
    );
}

#[test]
fn suppression_comments() {
    let d = repo();
    let r = d.path();
    // With a reason, on the line above.
    write(
        r,
        "src/load.js",
        "exports.load = function (name) {\n  // turbo-trace-ignore unconstrained-load: plugins \
         are installed by the user\n  return require(name);\n};\n",
    );
    let (code, out) = lint(r, &[]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        out,
        "0 new, 0 baselined, 1 suppressed, 0 stale; 2 of 3 tests on the floor\n"
    );
    let (_, v) = lint_json(r, &[]);
    let f = &v["findings"][0];
    assert_eq!(f["status"], "suppressed");
    assert_eq!(f["reason"], "plugins are installed by the user");
    assert_eq!(f["line"], 3);
    assert_eq!(v["summary"]["suppressed"], 1);
    assert_eq!(v["summary"]["ok"], true);
    // A suppressed finding is not written to the baseline.
    let (_, out) = lint(r, &["--update-baseline"]);
    assert_eq!(
        out,
        "lint-baseline.json: 0 finding(s) written as 0 entr(ies); 1 suppressed finding(s) are not \
         in the baseline\n"
    );
    std::fs::remove_file(r.join(BASELINE)).unwrap();
    // The selection is not changed by the comment: the tests stay on the floor.
    let (code, selected, _) = run(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--changed",
            "docs/readme.txt",
        ],
    );
    assert_eq!(code, 0);
    assert_eq!(
        selected.lines().collect::<Vec<_>>(),
        ["test/a.test.js", "test/b.test.js"]
    );

    // On the same line, as a block comment.
    write(
        r,
        "src/load.js",
        "exports.load = function (name) {\n  return require(name); /* turbo-trace-ignore \
         unconstrained-load: see above */\n};\n",
    );
    assert_eq!(lint(r, &[]).0, 0);

    // Without a reason: an error, and the load is not suppressed.
    write(
        r,
        "src/load.js",
        "exports.load = function (name) {\n  // turbo-trace-ignore unconstrained-load\n  return \
         require(name);\n};\n",
    );
    let (code, v) = lint_json(r, &[]);
    assert_eq!(code, 1, "{v}");
    let got: Vec<(&str, u64, &str, &str)> = v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["rule"].as_str().unwrap(),
                f["line"].as_u64().unwrap(),
                f["severity"].as_str().unwrap(),
                f["status"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            ("invalid-suppression", 2, "error", "new"),
            ("unconstrained-load", 3, "error", "new"),
        ]
    );
    assert!(
        v["findings"][0]["message"]
            .as_str()
            .unwrap()
            .contains("needs a reason"),
        "{v}"
    );
    // It cannot be baselined or switched off.
    assert_eq!(lint(r, &["--update-baseline"]).0, 0);
    let (code, out) = lint(r, &[]);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.starts_with("src/load.js:2  invalid-suppression  "),
        "{out}"
    );
    assert!(
        out.contains("1 new, 1 baselined, 0 suppressed, 0 stale"),
        "{out}"
    );
    let (code, _, err) = run(r, &["lint", "--rule", "invalid-suppression=off"]);
    assert_eq!(code, 1);
    assert!(
        err.contains("`invalid-suppression` is not a rule that can be configured"),
        "{err}"
    );
    std::fs::remove_file(r.join(BASELINE)).unwrap();

    // An unknown rule name.
    write(
        r,
        "src/load.js",
        "exports.load = function (name) {\n  // turbo-trace-ignore unconstrained-loads: typo\n  \
         return require(name);\n};\n",
    );
    let (code, out) = lint(r, &[]);
    assert_eq!(code, 1);
    assert!(
        out.contains(
            "src/load.js:2  invalid-suppression  a suppression names the unknown rule \
             `unconstrained-loads`"
        ),
        "{out}"
    );

    // A comment that suppresses nothing: a warning, exit 0.
    write(
        r,
        "src/load.js",
        "// turbo-trace-ignore unconstrained-load: nothing here\nexports.load = function (name) \
         {\n  // turbo-trace-ignore unconstrained-load: plugins\n  return require(name);\n};\n",
    );
    let (code, out) = lint(r, &[]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        out.lines().collect::<Vec<_>>(),
        [
            "warnings (they do not fail the check):",
            "src/load.js:1  unused-suppression  this suppression matches no `unconstrained-load` \
             finding on its line or the next",
            "  fix: remove the comment",
            "",
            "1 new, 0 baselined, 1 suppressed, 0 stale; 2 of 3 tests on the floor",
        ],
        "{out}"
    );
}

#[test]
fn rules_are_switched_by_flag_and_by_config() {
    let d = repo();
    let r = d.path();
    // Off: nothing to report, and the floor is not computed.
    let (code, out) = lint(r, &["--rule", "unconstrained-load=off"]);
    assert_eq!(
        (code, out.as_str()),
        (0, "0 new, 0 baselined, 0 suppressed, 0 stale\n")
    );
    // Warn: reported, exit 0.
    let (code, out) = lint(r, &["--rule", "unconstrained-load=warn"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.starts_with(
            "warnings (they do not fail the check):\nsrc/load.js:2  unconstrained-load  "
        ),
        "{out}"
    );
    // Unknown names and values are errors that say what is known.
    let (code, _, err) = run(r, &["lint", "--rule", "no-such-rule=error"]);
    assert_eq!(code, 1);
    assert!(
        err.contains(
            "unknown rule `no-such-rule`: the rules are unconstrained-load, unconstrained-read, \
             unresolved-reference"
        ),
        "{err}"
    );
    let (code, _, err) = run(r, &["lint", "--rule", "unconstrained-load=fatal"]);
    assert_eq!(code, 1);
    assert!(err.contains("expected off, warn or error"), "{err}");
    let (code, _, err) = run(r, &["lint", "--rule", "unconstrained-load"]);
    assert_eq!(code, 1);
    assert!(err.contains("expected <rule>=off|warn|error"), "{err}");

    // The configuration: a warning, another baseline file, tests, excludes.
    write(
        r,
        "turbotrace.toml",
        "[lint]\nbaseline = \"ci/lint-baseline.json\"\ntests = [\"jest\"]\nexclude = \
         [\"src/legacy/**\"]\n\n[lint.rules]\nunconstrained-load = \"warn\"\nunconstrained-read = \
         \"off\"\n",
    );
    let (code, out, err) = run(r, &["lint"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(
        out.starts_with("warnings (they do not fail the check):\nsrc/load.js:2  "),
        "{out}"
    );
    let (code, rules, _) = run(r, &["lint", "--list-rules"]);
    assert_eq!(code, 0);
    let rules: Vec<Vec<&str>> = rules
        .lines()
        .map(|l| l.split_whitespace().take(2).collect())
        .collect();
    assert_eq!(
        rules,
        [
            ["unconstrained-load", "warn"],
            ["unconstrained-read", "off"],
            ["unresolved-reference", "off"],
            ["undeclared-dependency", "off"],
            ["test-pattern-above-root", "warn"],
            ["cwd-path-explained", "off"]
        ]
    );
    // The command line wins over the configuration.
    let error = ["--rule", "unconstrained-load=error"];
    assert_eq!(lint(r, &error).0, 1);
    // The configured baseline, relative to the repository root.
    let (_, out) = lint(r, &["--update-baseline"]);
    assert!(
        out.starts_with("ci/lint-baseline.json: 1 finding(s) written"),
        "{out}"
    );
    assert!(r.join("ci/lint-baseline.json").is_file() && !r.join(BASELINE).exists());
    assert_eq!(lint(r, &error).0, 0);
    // A stale entry of a rule that only warns does not fail; of a rule that is off,
    // it is not judged.
    write(
        r,
        "src/load.js",
        "exports.load = () => require('./plain');\n",
    );
    let (code, out) = lint(r, &[]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains(&format!(
            "  src/load.js  unconstrained-load  {KEY}  (warning)\n"
        )),
        "{out}"
    );
    assert_eq!(lint(r, &error).0, 1);
    let (code, out) = lint(r, &["--rule", "unconstrained-load=off"]);
    assert_eq!(
        (code, out.as_str()),
        (0, "0 new, 0 baselined, 0 suppressed, 0 stale\n")
    );
    write(r, "src/load.js", LOAD);

    // Excluded files have no findings: from the configuration and from the flag.
    write(r, "src/legacy/old.js", "exports.old = (m) => require(m);\n");
    write(
        r,
        "test/c.test.js",
        "require('../src/plain');\nrequire('../src/legacy/old');\ntest('c', () => {});\n",
    );
    let (code, out) = lint(r, &error);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("src/legacy/old.js"), "{out}");
    assert!(
        out.contains("0 new, 1 baselined, 0 suppressed, 0 stale; 3 of 3 tests"),
        "{out}"
    );
    std::fs::remove_file(r.join("ci/lint-baseline.json")).unwrap();
    let (code, out) = lint(r, &[error[0], error[1], "--exclude", "src/*.js"]);
    assert_eq!(
        (code, out.as_str()),
        (
            0,
            "0 new, 0 baselined, 0 suppressed, 0 stale; 3 of 3 tests on the floor\n"
        )
    );

    // The other commands accept a file with a `[lint]` table.
    let (code, selected, err) = run(
        r,
        &["affected", "--tests", "jest", "--changed", "src/plain.js"],
    );
    assert_eq!(code, 0, "{err}");
    assert!(
        selected.lines().any(|l| l == "test/c.test.js"),
        "{selected}"
    );
    let (code, floor, err) = run(r, &["floor", "--tests", "jest"]);
    assert_eq!(code, 0, "{err}");
    assert!(
        floor.starts_with("uncertainty floor: 3 of 3 entries"),
        "{floor}"
    );

    // Mistakes in the table are errors that name the file.
    write(
        r,
        "turbotrace.toml",
        "[lint.rules]\nno-such-rule = \"error\"\n",
    );
    let (code, _, err) = run(r, &["lint"]);
    assert_eq!(code, 1);
    assert!(
        err.contains("turbotrace.toml: unknown rule `no-such-rule`"),
        "{err}"
    );
    write(r, "turbotrace.toml", "[lint]\nbaselin = \"x.json\"\n");
    let (code, _, err) = run(r, &["lint"]);
    assert_eq!(code, 1);
    assert!(
        err.contains("turbotrace.toml") && err.contains("baselin"),
        "{err}"
    );
}

#[test]
fn reads_are_a_separate_rule_that_is_off_by_default() {
    let d = repo();
    let r = d.path();
    write(
        r,
        "src/plain.js",
        "const fs = require('fs');\nexports.read = (p) => fs.readFileSync(p, 'utf8');\n",
    );
    // In JavaScript a read with a computed path is bounded by names and is not
    // open. With that layer off it is an open record, which only
    // `unconstrained-read` reports.
    let names_off = [("TURBO_TRACE_DEBUG_NAMES", "0")];
    let json = |extra: &[&str], env: &[(&str, &str)]| -> serde_json::Value {
        let mut args = vec!["lint", "--tests", "jest", "--format", "json"];
        args.extend_from_slice(extra);
        let (_, out, err) = run_env(r, &args, env);
        serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}: {out}{err}"))
    };
    let rules = |v: &serde_json::Value| -> Vec<(String, String)> {
        v["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                (
                    f["rule"].as_str().unwrap().to_owned(),
                    f["file"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    };
    let load = ("unconstrained-load".to_owned(), "src/load.js".to_owned());
    let read = ("unconstrained-read".to_owned(), "src/plain.js".to_owned());
    assert_eq!(rules(&json(&[], &names_off)), std::slice::from_ref(&load));
    let on = ["--rule", "unconstrained-read=error"];
    let v = json(&on, &names_off);
    assert_eq!(rules(&v), [load.clone(), read]);
    let f = &v["findings"][1];
    assert_eq!(f["line"], 2);
    assert_eq!(
        f["key"],
        "computed_specifier/param: p; parameter exports.read#0"
    );
    assert_eq!(
        f["message"],
        "the path is the parameter `p` of `exports.read`: any file can be read here"
    );
    assert_eq!(f["cost"], serde_json::json!({"reached": 1, "exclusive": 1}));
    assert!(
        f["fix"]
            .as_str()
            .unwrap()
            .starts_with("give the path a literal directory")
    );
    // With the default layers the read is bounded: no finding even with the rule
    // on.
    assert_eq!(rules(&json(&on, &[])), [load]);
}

/// A literal path that resolves to nothing is not a computed target: it belongs
/// to the rule `unresolved-reference` (off by default), not to
/// `unconstrained-load`.
#[test]
fn unresolved_references_are_a_rule_of_their_own() {
    let d = repo();
    let r = d.path();
    // A script a test runs, which sources a file that is not there.
    write(
        r,
        "scripts/run.sh",
        "#!/bin/sh\n. ./scripts/missing-env.sh\n",
    );
    write(
        r,
        "test/c.test.js",
        "const { execFileSync } = require('child_process');\nconst path = \
         require('path');\ntest('c', () => { execFileSync(path.join(__dirname, \
         '../scripts/run.sh')); });\n",
    );
    let (_, v) = lint_json(r, &[]);
    let of = |v: &serde_json::Value, rule: &str| -> Vec<String> {
        v["findings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["rule"] == rule)
            .map(|f| f["file"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(of(&v, "unconstrained-load"), ["src/load.js"], "{v}");
    assert!(of(&v, "unresolved-reference").is_empty(), "{v}");
    let (code, v) = lint_json(r, &["--rule", "unresolved-reference=error"]);
    assert_eq!(code, 1);
    assert_eq!(of(&v, "unconstrained-load"), ["src/load.js"], "{v}");
    // The rule reports whatever unresolved references are open; none of them is a
    // load of `src/load.js`, and every one says what it is.
    for f in v["findings"].as_array().unwrap() {
        if f["rule"] == "unresolved-reference" {
            assert!(
                f["message"]
                    .as_str()
                    .unwrap()
                    .contains("names nothing turbo trace can find"),
                "{f}"
            );
            assert!(
                f["key"]
                    .as_str()
                    .unwrap()
                    .starts_with("unresolved_specifier/")
            );
        }
    }
}

#[test]
fn json_and_github_formats() {
    let d = repo();
    let r = d.path();
    add_other(r);
    let (code, v) = lint_json(r, &[]);
    assert_eq!(code, 1);
    assert_eq!(v["version"], 1);
    assert_eq!(
        v["summary"],
        serde_json::json!({
            "ok": false, "new": 2, "errors": 2, "warnings": 0, "baselined": 0, "suppressed": 0,
            "stale": 0, "tests": 3, "floor": 3,
        })
    );
    assert_eq!(v["stale"], serde_json::json!([]));
    assert_eq!(
        v["findings"][0],
        serde_json::json!({
            "rule": "unconstrained-load",
            "severity": "error",
            "status": "new",
            "file": "src/load.js",
            "line": 2,
            "key": KEY,
            "message": MESSAGE,
            "cost": {"reached": 2, "exclusive": 2},
            "fix": format!("{FIX}\nor check it before the load: `if (!name.startsWith('./handlers/')) throw …` (turbo trace then bounds the load to that directory)"),
        })
    );
    assert_eq!(v["findings"][1]["file"], "src/other.js");
    assert_eq!(
        v["findings"][1]["cost"],
        serde_json::json!({"reached": 1, "exclusive": 1})
    );
    assert_eq!(v["findings"].as_array().unwrap().len(), 2);

    // GitHub: one annotation per new finding, then the summary.
    let (code, out) = lint(r, &["--format", "github"]);
    assert_eq!(code, 1);
    let (first, second) = (
        format!(
            "::error file=src/load.js,line=2,title=turbo trace unconstrained-load::{MESSAGE}. \
             Holds 2 tests. Fix: {FIX}"
        ),
        format!(
            "::error file=src/other.js,line=2,title=turbo trace unconstrained-load::the specifier \
             is the parameter `mod` of `exports.other`: any module can be loaded here. Holds 1 \
             test. Fix: {FIX}"
        ),
    );
    assert_eq!(
        out.lines().collect::<Vec<_>>(),
        [
            first.as_str(),
            second.as_str(),
            "2 new, 0 baselined, 0 suppressed, 0 stale; 3 of 3 tests on the floor",
        ],
        "{out}"
    );
    // Baselined findings are not annotated; a stale entry and a warning are.
    assert_eq!(lint(r, &["--update-baseline"]).0, 0);
    write(
        r,
        "src/other.js",
        "// turbo-trace-ignore unconstrained-load: nothing here\nexports.other = () => \
         require('./plain');\nexports.more = (m) => require(m);\n",
    );
    let (code, out) = lint(r, &["--format", "github"]);
    assert_eq!(code, 1);
    let more = format!(
        "::error file=src/other.js,line=3,title=turbo trace unconstrained-load::the specifier is \
         the parameter `m` of `exports.more`: any module can be loaded here. Holds 1 test. Fix: \
         {FIX}"
    );
    assert_eq!(
        out.lines().collect::<Vec<_>>(),
        [
            "::warning file=src/other.js,line=1,title=turbo trace unused-suppression::this \
             suppression matches no `unconstrained-load` finding on its line or the next. Fix: \
             remove the comment",
            more.as_str(),
            "::error file=lint-baseline.json,title=turbo trace stale baseline entry::The baseline \
             lists a finding that is gone: src/other.js  unconstrained-load  \
             computed_specifier/param: mod; parameter exports.other#0. Update the baseline and \
             commit the file.",
            "2 new, 1 baselined, 0 suppressed, 1 stale; 3 of 3 tests on the floor",
        ],
        "{out}"
    );
}

/// The fix the findings recommend, checked: a test in front of the load bounds
/// it (no finding); a test whose prefix names more modules than a pattern is
/// expanded to stays a finding that says so.
#[test]
fn a_check_before_the_load_is_no_finding_unless_its_prefix_names_too_much() {
    let d = repo();
    let r = d.path();
    // The same loader as `src/load.js`, with the check.
    write(
        r,
        "src/load.js",
        "exports.load = function (name) {\n  if (typeof name !== 'string' || \
         !name.startsWith('./handlers/')) throw new Error('outside');\n  return \
         require(name);\n};\n",
    );
    let (code, out) = lint(r, &[]);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("src/load.js"), "{out}");

    // A table of module names indexed by a key, and a value checked against a list:
    // the load is one of modules that all exist, so nothing about it is
    // unknown.
    write(
        r,
        "src/load.js",
        "const NAMES = { a: './handlers/a', one: './plain' };\nconst ALLOWED = ['./handlers/a', \
         './plain'];\nexports.load = function (name) {\n  if (ALLOWED.includes(name)) return \
         require(name);\n  return require(NAMES[name]);\n};\n",
    );
    let (code, out) = lint(r, &[]);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("src/load.js"), "{out}");
    // A member that names no module (yet) is no finding either: the paths its
    // resolution probed stand for it, so the file appearing selects the tests.
    write(
        r,
        "src/load.js",
        "const NAMES = { a: './handlers/a', gone: './handlers/gone' };\nexports.load = function \
         (name) {\n  return require(NAMES[name]);\n};\n",
    );
    let (code, out) = lint(r, &[]);
    assert_eq!(code, 0, "{out}");
    write(
        r,
        "src/load.js",
        "exports.load = function (name) {\n  if (typeof name !== 'string' || \
         !name.startsWith('./handlers/')) throw new Error('outside');\n  return \
         require(name);\n};\n",
    );

    // A prefix that names three modules, with a cap of two.
    write(r, "src/handlers/b.js", "module.exports = 2;\n");
    write(r, "src/handlers/c.js", "module.exports = 3;\n");
    let (code, _, err) = run(r, &["graph", "build", "--max-glob", "2"]);
    assert_eq!(code, 0, "{err}");
    let (code, v) = lint_json(r, &["--no-refresh"]);
    assert_eq!(code, 1, "{v}");
    let f = v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["file"] == "src/load.js")
        .unwrap_or_else(|| panic!("{v}"));
    assert_eq!(f["rule"], "unconstrained-load");
    let message = f["message"].as_str().unwrap();
    assert!(
        message.starts_with("the load is held to a pattern that is not expanded"),
        "{message}"
    );
    assert!(
        f["fix"]
            .as_str()
            .unwrap()
            .starts_with("name one directory in the prefix or the check"),
        "{f}"
    );
}

/// A pnpm workspace (a package resolves only what it declares) with a shared
/// loader:
///
/// ```text
/// packages/h/src/load.js      load(name): require(name)   declares nothing
/// packages/h/src/only.js      the same, used by services/c only
/// packages/dep/index.js                                    services/d declares it
/// packages/far/index.js                                    nobody declares it
/// services/c/test/c.test.js   → load.js, only.js           declares h
/// services/d/test/d.test.js   → load.js                    declares h, dep
/// ```
#[cfg(unix)]
fn workspace() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(
        r,
        "pnpm-workspace.yaml",
        "packages:\n  - 'services/*'\n  - 'packages/*'\n",
    );
    let packages: [(&str, &str, &[&str]); 5] = [
        ("packages/h", "@x/h", &[]),
        ("packages/dep", "@x/dep", &[]),
        ("packages/far", "@x/far", &[]),
        ("services/c", "svc-c", &["@x/h"]),
        ("services/d", "svc-d", &["@x/h", "@x/dep"]),
    ];
    for (dir, name, deps) in packages {
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
        for dep in deps {
            // One link per declared dependency, as pnpm installs them.
            let target = packages.iter().find(|p| p.1 == *dep).unwrap().0;
            let link = r.join(dir).join("node_modules").join(dep);
            std::fs::create_dir_all(link.parent().unwrap()).unwrap();
            turborepo_trace_core::os::symlink(r.join(target), link).unwrap();
        }
    }
    write(r, "packages/h/src/load.js", LOAD);
    write(
        r,
        "packages/h/src/only.js",
        "exports.only = function (mod) {\n  return require(mod);\n};\n",
    );
    write(r, "packages/dep/index.js", "module.exports = 1;\n");
    write(r, "packages/far/index.js", "module.exports = 2;\n");
    for (s, more) in [("c", "require('@x/h/src/only.js');\n"), ("d", "")] {
        write(
            r,
            &format!("services/{s}/jest.config.js"),
            "module.exports = {};\n",
        );
        write(
            r,
            &format!("services/{s}/test/{s}.test.js"),
            &format!("require('@x/h/src/load.js');\n{more}test('{s}', () => {{}});\n"),
        );
    }
    d
}

/// `lint` and `uncertainty` report on what `affected` evaluates. Under every
/// switch that changes what a record is taken to be, the three agree: the
/// lint's floor is the floor `uncertainty` reports, and both are the tests a
/// change of nothing selects (plus, for a switch that makes a file name every
/// path, the tests that name then selects).
///
/// The switches of rules that work while a file is traced or the graph is built
/// (`TURBO_TRACE_DEBUG_ANCHORS_OFF`, `TURBO_TRACE_DEBUG_FW_LOADS`,
/// `TURBO_TRACE_DEBUG_LOCATED_PACKAGE`, `TURBO_TRACE_DEBUG_TSCONFIG_PROBED`)
/// are not options of the evaluation: they change the graph the three commands
/// read, so each setting has a graph of its own here.
#[cfg(unix)]
#[test]
fn lint_uncertainty_and_affected_agree_under_every_switch() {
    let d = workspace();
    let r = d.path();
    // A read with a computed path (bounded by names) next to the loads.
    write(
        r,
        "packages/h/src/read.js",
        "const fs = require('fs');\nexports.read = function (p) {\n  return fs.readFileSync(p, \
         'utf8');\n};\n",
    );
    write(
        r,
        "services/d/test/d.test.js",
        "require('@x/h/src/load.js');\nconst { read } = require('@x/h/src/read.js');\ntest('d', \
         () => { read(process.env.DATA); });\n",
    );
    // One test for each rule that a switch of its own turns off.
    let service = |s: &str, manifest: &str, config: &str, test: &str| {
        write(r, &format!("services/{s}/package.json"), manifest);
        write(r, &format!("services/{s}/jest.config.js"), config);
        write(r, &format!("services/{s}/test/{s}.test.js"), test);
    };
    let plain = |s: &str| format!(r#"{{"name":"svc-{s}","version":"1.0.0"}}"#);
    // What the runner may load in place of a module: a resolver of its own.
    service(
        "e",
        &plain("e"),
        "module.exports = { resolver: './resolver.js' };\n",
        "test('e', () => {});\n",
    );
    write(
        r,
        "services/e/resolver.js",
        "module.exports = (request, options) => options.defaultResolver(request, options);\n",
    );
    // A config that extends a config that is not there.
    service(
        "f",
        &plain("f"),
        "module.exports = {};\n",
        "require('../app/main');\ntest('f', () => {});\n",
    );
    write(
        r,
        "services/f/app/jsconfig.json",
        r#"{"extends": "./.generated/tsconfig.json"}"#,
    );
    write(r, "services/f/app/main.js", "module.exports = 1;\n");
    // A load next to a registry package that is not installed.
    service(
        "g",
        r#"{"name":"svc-g","version":"1.0.0","dependencies":{"left-pad":"^1"}}"#,
        "module.exports = {};\n",
        "const { createRequire } = require('module');\nconst inner = \
         createRequire(require.resolve('left-pad'))('./inner.js');\ntest('g', () => { inner; });\n",
    );
    write(
        r,
        "pnpm-lock.yaml",
        "lockfileVersion: '9.0'\nimporters:\n  .: {}\n  services/g:\n    dependencies:\n      left-pad:\n        specifier: ^1\n        version: 1.3.0\npackages:\n  left-pad@1.3.0:\n    resolution: {integrity: sha512-pad}\nsnapshots:\n  left-pad@1.3.0: {}\n",
    );
    // A located root that is only the working directory of a child process, in a
    // file that hands a path below its own directory to a reader (a read with a
    // computed path, bounded by names).
    service(
        "k",
        &plain("k"),
        "module.exports = {};\n",
        "require('../tools');\ntest('k', () => {});\n",
    );
    write(
        r,
        "services/k/tools.js",
        "const path = require('path');\nconst { execSync } = require('child_process');\nconst { \
         read } = require('./reader');\nconst root = path.resolve(__dirname, '..', \
         '..');\nexports.status = () => execSync('git status --porcelain', { cwd: root \
         });\nexports.own = (name) => read(path.join(__dirname, 'data', name));\n",
    );
    write(
        r,
        "services/k/reader.js",
        "exports.read = (p) => require('fs').readFileSync(p, 'utf8');\n",
    );
    write(r, "services/k/data/x.txt", "x\n");
    // An import of a third-party Python module below a dependency declaration.
    let mut frameworks = vec!["--tests", "jest"];
    if false {
        frameworks.extend(["--tests", "pytest"]);
        write(r, "pytest.ini", "[pytest]\n");
        write(
            r,
            "app/pyproject.toml",
            "[project]\nname = \"app\"\nversion = \"1.0\"\ndependencies = [\"psycopg>=3\"]\n",
        );
        write(r, "app/__init__.py", "");
        write(
            r,
            "app/db.py",
            "import psycopg\n\n\ndef connect():\n    return psycopg.connect()\n",
        );
        write(
            r,
            "tests/test_db.py",
            "from app.db import connect\n\n\ndef test_db():\n    assert connect\n",
        );
    }
    let count = |text: &str, before: &str| -> (usize, usize) {
        let (head, _) = text.rsplit_once(before).unwrap_or_else(|| panic!("{text}"));
        let mut words = head.split_whitespace().rev();
        let total = words.next().unwrap().parse().unwrap();
        assert_eq!(words.next(), Some("of"), "{text}");
        (words.next().unwrap().parse().unwrap(), total)
    };
    let python = usize::from(false);
    // Each switch with the value that turns its rule off, the floor that leaves,
    // and the tests a change of nothing selects beyond the floor (through a
    // name of every path).
    let switches: [(&str, &str, usize, usize); 11] = [
        ("", "", 0, 0),
        // The computed reads of `read.js` and of the reader of `tools.js` are open.
        ("TURBO_TRACE_DEBUG_NAMES", "0", 2, 0),
        ("TURBO_TRACE_DEBUG_NAMES_PACKAGE", "0", 0, 0),
        ("TURBO_TRACE_DEBUG_NAMES_TASK_RUNNER", "0", 0, 0),
        // The loads of `load.js` and `only.js`, and what the resolver of `e` may load.
        ("TURBO_TRACE_DEBUG_LOADS_PACKAGE", "0", 3, 0),
        ("TURBO_TRACE_DEBUG_LOADS_STEP", "0", 0, 0),
        // The root of `tools.js` escapes: the file names every path, which is no floor.
        // (A rule switch, not the switch of an assumption: what the child reads is outside
        // the model with the rule on and off, `affected_anchors.rs`.)
        ("TURBO_TRACE_DEBUG_ANCHORS_OFF", "child", 0, 1),
        // The resolver's record is a file read again.
        ("TURBO_TRACE_DEBUG_FW_LOADS", "0", 1, 0),
        // The load next to `left-pad` has an unknown target: the declared packages bound
        // it here, so it is not on the floor either.
        ("TURBO_TRACE_DEBUG_LOCATED_PACKAGE", "0", 0, 0),
        ("TURBO_TRACE_DEBUG_TSCONFIG_PROBED", "0", 1, 0),
        ("TURBO_TRACE_DEBUG_PY_DECLARED", "0", python, 0),
    ];
    let mut seen = Vec::new();
    let mut declared = Vec::new();
    for (i, (switch, value, ..)) in switches.iter().enumerate() {
        let env: &[(&str, &str)] = if switch.is_empty() {
            &[]
        } else {
            &[(switch, value)]
        };
        let graph = format!(".tt-{i}");
        let run = |command: &str, more: &[&str]| {
            let mut args = vec![command, "--graph", graph.as_str()];
            args.extend(&frameworks);
            args.extend(more);
            run_env(r, &args, env)
        };
        let (code, out, err) = run("lint", &[]);
        assert!(code == 0 || code == 1, "{switch}: {err}");
        let lint = count(&out, " tests on the floor");
        let (code, out, err) = run("floor", &[]);
        assert_eq!(code, 0, "{switch}: {err}");
        assert!(out.starts_with("uncertainty floor: "), "{out}");
        let floor = count(out.lines().next().unwrap(), " entries");
        let (code, out, err) = run("floor", &["--json"]);
        assert_eq!(code, 0, "{switch}: {err}");
        let v: serde_json::Value = serde_json::from_str(&out).expect("json");
        declared.push(v["records"]["declared"].as_u64().unwrap_or(0));
        let (code, out, err) = run("affected", &["--changed", "no/such/file"]);
        assert_eq!(code, 0, "{switch}: {err}");
        assert_eq!(lint, floor, "{switch}: lint and uncertainty");
        assert!(out.lines().count() >= lint.0, "{switch}: {out}");
        seen.push((lint.0, out.lines().count() - lint.0));
    }
    // The switches do move the floor here: each rule's record is open without the
    // rule.
    let expected: Vec<(usize, usize)> = switches.iter().map(|s| (s.2, s.3)).collect();
    assert_eq!(
        seen, expected,
        "(floor, selected by a change of nothing beyond it) per switch"
    );
    // The load next to the package that is not installed: one more record that the
    // declared packages bound.
    assert_eq!(declared[8], declared[0] + 1, "{declared:?}");
    // Who runs a process decides what a changed path selects, not what is open: the
    // findings are the same with the switch off.
    let json = |env: &[(&str, &str)]| {
        let args = ["lint", "--tests", "jest", "--format", "json", "--rule"];
        let mut args = args.to_vec();
        args.extend([
            "unconstrained-read=warn",
            "--rule",
            "unresolved-reference=warn",
        ]);
        let (code, out, err) = run_env(r, &args, env);
        assert!(code == 0 || code == 1, "{err}");
        let v: serde_json::Value = serde_json::from_str(&out).expect("json");
        v["findings"].clone()
    };
    let findings = json(&[]);
    assert!(!findings.as_array().unwrap().is_empty());
    assert_eq!(
        findings,
        json(&[("TURBO_TRACE_DEBUG_NAMES_TASK_RUNNER", "0")])
    );
}

/// Where the declared packages bound a load with an unknown target, the load is
/// not on the floor, and it is still an unconstrained load: the site is what an
/// edit would tighten. The finding says what bounds it, its cost is the tests
/// that reach it, and its key is the one it has without the bound.
#[cfg(unix)]
#[test]
fn a_load_bounded_by_the_declared_packages_is_still_an_unconstrained_load() {
    let d = workspace();
    let r = d.path();
    let (code, out) = lint(r, &[]);
    assert_eq!(code, 1, "{out}");
    let lines: Vec<&str> = out.lines().collect();
    let load = format!(
        "packages/h/src/load.js:2  unconstrained-load  {MESSAGE}; bounded to packages/h/ and the \
         packages it declares (1 of 6), and to the package of each test and what that declares"
    );
    let fix = format!("  fix: {FIX}");
    assert_eq!(
        lines[..4],
        [
            load.as_str(),
            "  reached by 2 tests, each selected when the change affects a package the load can \
             reach (1 reach no other unconstrained site)",
            fix.as_str(),
            "       or check it before the load: `if (!name.startsWith('./handlers/')) throw …` \
             (turbo trace then bounds the load to that directory)",
        ],
        "{out}"
    );
    assert!(
        out.contains(
            "packages/h/src/only.js:2  unconstrained-load  the specifier is the parameter `mod` \
             of `exports.only`: any module can be loaded here; bounded to packages/h/ and the \
             packages it declares (1 of 6), and to the package of each test and what that \
             declares\n  reached by 1 test, each selected when the change affects a package the \
             load can reach (0 reach no other unconstrained site)\n"
        ),
        "{out}"
    );
    // With the declared closure (TURBO_TRACE_DEBUG_LOADS_STEP=0) the bound is what
    // the packages of the tests depend on; the loader that the tests of one
    // package reach names that package.
    let (code, closure, err) = run_env(
        r,
        &["lint", "--tests", "jest"],
        &[("TURBO_TRACE_DEBUG_LOADS_STEP", "0")],
    );
    assert_eq!(code, 1, "{closure}{err}");
    assert!(
        closure.contains(
            "; bounded to the packages each test's package depends on (tests of 2 packages reach \
             it, which depend on 4 of 6 packages)\n"
        ),
        "{closure}"
    );
    assert!(
        closure.contains("; bounded to the packages services/c/ depends on (2 of 6)\n"),
        "{closure}"
    );
    // Neither is on the floor.
    assert_eq!(
        lines.last().copied(),
        Some("2 new, 0 baselined, 0 suppressed, 0 stale; 0 of 2 tests on the floor"),
        "{out}"
    );
    let (_, v) = lint_json(r, &[]);
    assert_eq!(
        v["findings"][0]["cost"],
        serde_json::json!({"reached": 2, "exclusive": 1, "bounded": true})
    );
    assert_eq!(v["findings"][0]["key"], KEY);
    assert_eq!(v["summary"]["floor"], 0);

    // Without the bound the same two findings are open records with the same keys:
    // a baseline written either way accepts both.
    const OPEN: &[(&str, &str)] = &[("TURBO_TRACE_DEBUG_LOADS_PACKAGE", "0")];
    let (code, open, err) = run_env(r, &["lint", "--tests", "jest"], OPEN);
    assert_eq!(code, 1, "{open}{err}");
    assert!(
        open.starts_with(&format!(
            "packages/h/src/load.js:2  unconstrained-load  {MESSAGE}\n  holds 2 tests on the \
             floor (1 only this site)\n"
        )),
        "{open}"
    );
    assert!(
        open.ends_with("2 new, 0 baselined, 0 suppressed, 0 stale; 2 of 2 tests on the floor\n"),
        "{open}"
    );
    let (code, _, err) = run_env(r, &["lint", "--tests", "jest", "--update-baseline"], OPEN);
    assert_eq!(code, 0, "{err}");
    let (code, out) = lint(r, &[]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.ends_with("0 new, 2 baselined, 0 suppressed, 0 stale; 0 of 2 tests on the floor\n"),
        "{out}"
    );
    // A suppression comment applies to it like to an open one.
    std::fs::remove_file(r.join(BASELINE)).unwrap();
    write(
        r,
        "packages/h/src/only.js",
        "exports.only = function (mod) {\n  // turbo-trace-ignore unconstrained-load: plugins are \
         named by the caller\n  return require(mod);\n};\n",
    );
    let (code, out) = lint(r, &[]);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.ends_with("1 new, 0 baselined, 1 suppressed, 0 stale; 0 of 2 tests on the floor\n"),
        "{out}"
    );
}

/// `undeclared-dependency`: a file that loads, reads or imports the types of a
/// file of a package its own package does not declare. Off by default; its
/// findings group in the baseline by file, target package and kind of
/// reference.
#[cfg(unix)]
#[test]
fn undeclared_dependencies_are_a_rule_that_is_off_by_default() {
    let d = workspace();
    let r = d.path();
    // No unconstrained load in this test.
    for f in ["load", "only"] {
        write(
            r,
            &format!("packages/h/src/{f}.js"),
            "module.exports = 1;\n",
        );
    }
    write(r, "packages/far/data.json", "{}\n");
    write(r, "packages/far/types.ts", "export type T = number;\n");
    // services/c: two loads and a read of a package nothing it depends on declares.
    write(
        r,
        "services/c/src/reach.js",
        "const far = require('../../../packages/far/index.js');\nconst again = \
         require('../../../packages/far');\nconst fs = require('fs');\nconst path = \
         require('path');\nmodule.exports = [far, again, fs.readFileSync(path.join(__dirname, \
         '../../../packages/far/data.json'), 'utf8')];\n",
    );
    write(
        r,
        "services/c/src/types.ts",
        "import type { T } from '../../../packages/far/types';\nexport type U = T;\n",
    );
    // packages/h declares nothing; services/d declares h and dep: fine. services/c
    // reaches dep, which only services/d declares: nothing services/c depends
    // on does.
    write(
        r,
        "services/d/src/ok.js",
        "module.exports = [require('@x/dep'), require('../../../packages/dep/index.js')];\n",
    );
    write(
        r,
        "services/c/test/c.test.js",
        "require('@x/h/src/load.js');\nrequire('../src/reach');\ntest('c', () => {});\n",
    );

    // Off by default.
    let (code, out) = lint(r, &[]);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("undeclared-dependency"), "{out}");

    const ON: &[&str] = &["--rule", "undeclared-dependency=error"];
    let (code, out) = lint(r, ON);
    assert_eq!(code, 1, "{out}");
    let load = "services/c/src/reach.js:1  undeclared-dependency  loads packages/far/index.js of \
                packages/far/ (`../../../packages/far/index.js`, a relative path): services/c/ \
                does not declare `@x/far` (nor does a package it depends on); test selection \
                assumes that a file reaches only its own package and the packages it declares\n  \
                fix: declare the dependency: add `@x/far` to `dependencies` (or `devDependencies` \
                for a test or a tool) in services/c/package.json\n       or do not reach into \
                another package: import what it exports by its name once it is declared, or move \
                the shared code into a package both declare\n";
    assert!(out.contains(load), "{out}");
    assert!(
        out.contains(
            "services/c/src/reach.js:2  undeclared-dependency  loads packages/far/index.js of \
             packages/far/ (`../../../packages/far`, a relative path)"
        ),
        "{out}"
    );
    assert!(
        out.contains(
            "services/c/src/reach.js:5  undeclared-dependency  reads packages/far/data.json of \
             packages/far/ ("
        ),
        "{out}"
    );
    assert!(
        out.contains(
            "       or do not reach into another package: have that package export the data"
        ),
        "{out}"
    );
    assert!(
        out.contains(
            "services/c/src/types.ts:1  undeclared-dependency  imports the types of \
             packages/far/types.ts of packages/far/ (`../../../packages/far/types`, a relative \
             path)"
        ),
        "{out}"
    );
    assert!(
        out.contains(
            "add `@x/far` to `devDependencies` in services/c/package.json (types are erased, so a \
             dev dependency is enough)"
        ),
        "{out}"
    );
    // A declared package is no finding, however it is written.
    assert!(!out.contains("services/d/"), "{out}");
    assert!(
        out.contains("\n4 new, 0 baselined, 0 suppressed, 0 stale; 0 of 2 tests on the floor\n"),
        "{out}"
    );

    // The baseline: one entry per file, target package and kind, with a count.
    let (code, out) = lint(r, &[ON[0], ON[1], "--update-baseline"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("4 finding(s) written as 3 entr(ies)"), "{out}");
    let baseline = std::fs::read_to_string(r.join(BASELINE)).unwrap();
    assert!(
        baseline.contains(r#"{ "rule": "undeclared-dependency", "file": "services/c/src/reach.js", "key": "loads packages/far/ (relative path)", "count": 2 }"#),
        "{baseline}"
    );
    assert!(
        baseline.contains(r#""key": "types packages/far/ (relative path)""#),
        "{baseline}"
    );
    assert_eq!(lint(r, ON).0, 0);
    // A third load of the same package from the same file is one new finding; a
    // line that moves is none.
    write(
        r,
        "services/c/src/reach.js",
        "\n\nconst far = require('../../../packages/far/index.js');\nconst again = \
         require('../../../packages/far');\nconst third = \
         require('../../../packages/far/index');\nconst fs = require('fs');\nconst path = \
         require('path');\nmodule.exports = [far, again, third, \
         fs.readFileSync(path.join(__dirname, '../../../packages/far/data.json'), 'utf8')];\n",
    );
    let (code, out) = lint(r, ON);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("\n1 new, 4 baselined, 0 suppressed, 0 stale; 0 of 2 tests on the floor\n"),
        "{out}"
    );
    // Declaring the package: nothing is left, and the baseline is stale until
    // updated.
    write(
        r,
        "services/c/package.json",
        r#"{"name":"svc-c","version":"1.0.0","dependencies":{"@x/h":"workspace:*","@x/far":"workspace:*"}}"#,
    );
    let (code, out) = lint(r, ON);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("\n0 new, 0 baselined, 0 suppressed, 3 stale; 0 of 2 tests on the floor\n"),
        "{out}"
    );
    // As a warning it never fails the check, and a rule that is off is not judged.
    let (code, _) = lint(r, &["--rule", "undeclared-dependency=warn"]);
    assert_eq!(code, 0);
    assert_eq!(lint(r, &[]).0, 0);
}

/// jest matches its test patterns against the absolute path of a file. A
/// pattern that matches through the directories the checkout lies below gives
/// other tests at another place: the rule `test-pattern-above-root` reports it
/// (a warning by default), with the directories it matches through and how many
/// files are tests here and none at a neutral place (or the other way round),
/// whether or not the pattern also finds tests the ordinary way. A pattern that
/// cannot see above the root is not reported, and neither are jest's defaults
/// at an ordinary place.
#[test]
fn a_test_pattern_that_matches_through_a_directory_above_the_root_is_reported() {
    let d = tempfile::tempdir().unwrap();
    // The checkout lies below directories called `src` and `elsewhere`.
    let r = &d.path().canonicalize().unwrap().join("elsewhere/src/repo");
    write(
        r,
        "package.json",
        r#"{"name":"root","private":true,"version":"1.0.0","workspaces":["*"]}"#,
    );
    for (dir, config) in [
        // Every file of a checkout below `elsewhere`.
        ("only", "{ testMatch: ['**/elsewhere/**'] }"),
        ("regex", "{ testRegex: 'elsewhere.*[.]js$' }"),
        // Finds `src/a.js` the ordinary way, and `checks/one.js` through `src` above.
        ("inside", "{ testMatch: ['**/src/**/*.js'] }"),
        // Leaves every file out through `src` above.
        ("ignored", "{ testPathIgnorePatterns: ['/src/'] }"),
        // Cannot see above the root.
        ("rooted", "{ testMatch: ['<rootDir>/src/**/*.js'] }"),
        ("plain", "{}"),
    ] {
        write(
            r,
            &format!("{dir}/package.json"),
            &format!(r#"{{"name":"{dir}","version":"1.0.0"}}"#),
        );
        write(
            r,
            &format!("{dir}/jest.config.js"),
            &format!("module.exports = {config};\n"),
        );
        write(r, &format!("{dir}/lib.js"), "module.exports = 1;\n");
        for t in ["src/a.js", "checks/one.js", "checks/b.test.js"] {
            write(
                r,
                &format!("{dir}/{t}"),
                "require('../lib');\ntest('t', () => {});\n",
            );
        }
    }
    let (code, v) = lint_json(r, &[]);
    assert_eq!(code, 0, "{v}");
    let found: Vec<(&str, &str, &str)> = v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["rule"] == "test-pattern-above-root")
        .map(|f| {
            (
                f["file"].as_str().unwrap(),
                f["key"].as_str().unwrap(),
                f["severity"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            ("ignored/jest.config.js", "/src/", "warn"),
            ("inside/jest.config.js", "**/src/**/*.js", "warn"),
            ("only/jest.config.js", "**/elsewhere/**", "warn"),
            ("regex/jest.config.js", "elsewhere.*[.]js$", "warn"),
        ],
        "{v}"
    );
    let message = |file: &str| -> String {
        v["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["file"] == file)
            .unwrap()["message"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    // The pattern finds `inside/src/a.js` wherever the checkout is: reported all
    // the same.
    assert_eq!(
        message("inside/jest.config.js"),
        "the test pattern `**/src/**/*.js` matches through `src` above the checkout: which files \
         are tests depends on where the repository is checked out (4 file(s) are tests here and \
         none in a checkout elsewhere, 0 are tests elsewhere and none here; for example \
         inside/checks/b.test.js)"
    );
    assert_eq!(
        message("ignored/jest.config.js"),
        "the pattern `/src/`, which leaves files out, matches through `src` above the checkout: \
         which files are tests depends on where the repository is checked out (0 file(s) are \
         tests here and none in a checkout elsewhere, 1 are tests elsewhere and none here; for \
         example ignored/checks/b.test.js)"
    );
    assert!(
        message("only/jest.config.js").starts_with(
            "the test pattern `**/elsewhere/**` matches through `elsewhere` above the checkout: "
        ),
        "{}",
        message("only/jest.config.js")
    );
    // The rule can be switched off like the others.
    let (_, v) = lint_json(r, &["--rule", "test-pattern-above-root=off"]);
    assert!(
        !v["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["rule"] == "test-pattern-above-root"),
        "{v}"
    );
    // The files are tests here, as for jest, and `--explain` says what the place
    // decides.
    let (code, out, err) = run(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--changed",
            "inside/lib.js",
            "--explain",
            "inside/checks/one.js",
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains(
            "assumed: inside/checks/one.js is a jest test here and none in a checkout elsewhere: \
             the test pattern `**/src/**/*.js` matches through `src` above the checkout \
             (inside/jest.config.js); which files are tests depends on where the repository is \
             checked out"
        ),
        "{out}"
    );
    assert!(out.contains("affected inside/checks/one.js"), "{out}");
    // And for a file that is no test here and one elsewhere.
    let (code, out, err) = run(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--changed",
            "ignored/lib.js",
            "--explain",
            "ignored/checks/b.test.js",
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains(
            "assumed: ignored/checks/b.test.js is no jest test here and one in a checkout \
             elsewhere: the pattern `/src/`, which leaves files out, matches through `src` above \
             the checkout (ignored/jest.config.js)"
        ),
        "{out}"
    );
    // The selection lists it as an unknown test, and `--explain` says so.
    assert!(
        out.contains("affected ignored/checks/b.test.js: an unknown test"),
        "{out}"
    );
}

/// A pattern written from the project root (`<rootDir>/src/`) that still
/// answers otherwise here (the checkout's path has characters the pattern reads
/// as syntax: `c++`, `[x]`): if `test-pattern-above-root` reports it, the
/// advice is not to write it from the root.
#[test]
fn a_pattern_written_from_the_root_is_not_told_to_be_written_from_the_root() {
    let d = tempfile::tempdir().unwrap();
    let mut reported = Vec::new();
    for place in ["c++/repo", "a[x]/repo", "p(q)/repo", "x{y}/repo"] {
        let r = &d.path().canonicalize().unwrap().join(place);
        write(r, "package.json", r#"{"name":"p","version":"1.0.0"}"#);
        write(
            r,
            "jest.config.js",
            "module.exports = { testMatch: ['<rootDir>/src/**/*.test.js'], \
             testPathIgnorePatterns: ['<rootDir>/src/dist/'] };\n",
        );
        write(r, "lib.js", "module.exports = 1;\n");
        for t in ["src/a.test.js", "src/dist/b.test.js"] {
            write(r, t, "require('../lib');\ntest('t', () => {});\n");
        }
        let (code, v) = lint_json(r, &[]);
        assert_eq!(code, 0, "{v}");
        for f in v["findings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["rule"] == "test-pattern-above-root")
        {
            let fix = f["fix"].as_str().unwrap();
            assert!(
                f["key"].as_str().unwrap().starts_with("<rootDir>"),
                "{place}: {f}"
            );
            assert!(
                !fix.contains("Write the pattern from the project root"),
                "{place}: {f}"
            );
            assert!(fix.contains("pattern syntax"), "{place}: {f}");
            reported.push(place);
        }
    }
    eprintln!("reported at: {reported:?}");
}
