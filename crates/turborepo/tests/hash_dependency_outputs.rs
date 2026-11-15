//! `futureFlags.hashDependencyOutputs`: dependents are hashed by the contents
//! of their dependencies' outputs, so a dependency that reruns and produces
//! identical outputs leaves its dependents cached.

#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{collections::BTreeMap, fs, path::Path};

use common::{git, run_turbo, setup};
use serde_json::Value;

const DEFERRED_MESSAGE: &str =
    "Deferred because hashDependencyOutputs hashes dependency outputs after they are built.";

/// `my-app` depends on `util`. `util#build` copies `src/index.js` to
/// `dist/index.js`; `my-app#build` copies util's built file into its own
/// `dist`. `typecheck` reaches dependencies through an output-less `transit`
/// task.
fn setup_fixture(dir: &Path, flag: Option<bool>) {
    setup::setup_integration_test(dir, "basic_monorepo", "npm@10.5.0", false).unwrap();

    let mut turbo_json = serde_json::json!({
        "$schema": "https://turborepo.dev/schema.json",
        "tasks": {
            "build": { "dependsOn": ["^build"], "outputs": ["dist/**"] },
            "transit": { "dependsOn": ["^transit"] },
            "typecheck": { "dependsOn": ["^transit"] }
        }
    });
    if let Some(flag) = flag {
        turbo_json["futureFlags"] = serde_json::json!({ "hashDependencyOutputs": flag });
    }
    fs::write(
        dir.join("turbo.json"),
        serde_json::to_string_pretty(&turbo_json).unwrap(),
    )
    .unwrap();

    let util = dir.join("packages/util");
    fs::create_dir_all(util.join("src")).unwrap();
    fs::write(util.join("src/index.js"), "module.exports = 1;\n").unwrap();
    fs::write(util.join("README.md"), "# util\n").unwrap();
    fs::write(
        util.join("package.json"),
        r#"{
  "name": "util",
  "scripts": {
    "build": "node -e \"const fs = require('fs'); fs.mkdirSync('dist', { recursive: true }); fs.copyFileSync('src/index.js', 'dist/index.js')\"",
    "typecheck": "echo checked"
  }
}
"#,
    )
    .unwrap();

    fs::write(
        dir.join("apps/my-app/package.json"),
        r#"{
  "name": "my-app",
  "scripts": {
    "build": "node -e \"const fs = require('fs'); fs.mkdirSync('dist', { recursive: true }); fs.copyFileSync('../../packages/util/dist/index.js', 'dist/bundle.js')\"",
    "typecheck": "echo checked"
  },
  "dependencies": {
    "util": "*"
  }
}
"#,
    )
    .unwrap();

    let gitignore = fs::read_to_string(dir.join(".gitignore")).unwrap();
    fs::write(dir.join(".gitignore"), format!("{gitignore}dist/\n")).unwrap();

    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "set up fixture", "--quiet"]);
}

fn run(dir: &Path, args: &[&str]) -> String {
    let output = run_turbo(dir, args);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    assert!(
        output.status.success(),
        "turbo {args:?} failed\nstdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout
}

/// Whether each task was a cache hit, from `--output-logs=hash-only` output.
fn cache_hits(stdout: &str) -> BTreeMap<String, bool> {
    stdout
        .lines()
        .filter_map(|line| {
            let (task, rest) = line.split_once(": ")?;
            let hit = if rest.starts_with("cache hit") {
                true
            } else if rest.starts_with("cache miss") || rest.starts_with("cache bypass") {
                false
            } else {
                return None;
            };
            Some((task.trim().to_string(), hit))
        })
        .collect()
}

fn run_task(dir: &Path, task: &str) -> BTreeMap<String, bool> {
    cache_hits(&run(dir, &["run", task, "--output-logs=hash-only"]))
}

fn expect(entries: &[(&str, bool)]) -> BTreeMap<String, bool> {
    entries
        .iter()
        .map(|(task, hit)| (task.to_string(), *hit))
        .collect()
}

fn dry_json(dir: &Path, task: &str) -> Value {
    serde_json::from_str(&run(dir, &["run", task, "--dry=json"])).unwrap()
}

fn task_field<'a>(json: &'a Value, task_id: &str, field: &str) -> &'a Value {
    &json["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["taskId"] == task_id)
        .unwrap_or_else(|| panic!("missing {task_id}"))[field]
}

fn is_task_hash(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|hash| hash.len() == 16 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
}

#[test]
fn non_output_change_reruns_dependency_but_keeps_dependent_cached() {
    let tempdir = tempfile::tempdir().unwrap();
    let dir = tempdir.path();
    setup_fixture(dir, Some(true));

    assert_eq!(
        run_task(dir, "build"),
        expect(&[("my-app:build", false), ("util:build", false)])
    );
    assert_eq!(
        run_task(dir, "build"),
        expect(&[("my-app:build", true), ("util:build", true)])
    );

    // The README is one of util#build's inputs but not part of its outputs.
    fs::write(dir.join("packages/util/README.md"), "# util, documented\n").unwrap();
    assert_eq!(
        run_task(dir, "build"),
        expect(&[("my-app:build", true), ("util:build", false)]),
        "util reruns and produces identical outputs, so my-app stays cached"
    );
    assert_eq!(
        fs::read_to_string(dir.join("apps/my-app/dist/bundle.js")).unwrap(),
        "module.exports = 1;\n"
    );
}

#[test]
fn without_the_flag_a_non_output_change_reruns_dependents() {
    for flag in [None, Some(false)] {
        let tempdir = tempfile::tempdir().unwrap();
        let dir = tempdir.path();
        setup_fixture(dir, flag);

        run_task(dir, "build");
        fs::write(dir.join("packages/util/README.md"), "# util, documented\n").unwrap();
        assert_eq!(
            run_task(dir, "build"),
            expect(&[("my-app:build", false), ("util:build", false)]),
            "flag={flag:?}"
        );
    }
}

#[test]
fn output_change_reruns_dependent() {
    let tempdir = tempfile::tempdir().unwrap();
    let dir = tempdir.path();
    setup_fixture(dir, Some(true));

    run_task(dir, "build");
    fs::write(
        dir.join("packages/util/src/index.js"),
        "module.exports = 2;\n",
    )
    .unwrap();
    assert_eq!(
        run_task(dir, "build"),
        expect(&[("my-app:build", false), ("util:build", false)])
    );
    assert_eq!(
        fs::read_to_string(dir.join("apps/my-app/dist/bundle.js")).unwrap(),
        "module.exports = 2;\n"
    );

    // Reverting the output restores the earlier hashes from the cache.
    fs::write(
        dir.join("packages/util/src/index.js"),
        "module.exports = 1;\n",
    )
    .unwrap();
    assert_eq!(
        run_task(dir, "build"),
        expect(&[("my-app:build", true), ("util:build", true)])
    );
}

#[test]
fn restored_dependency_outputs_keep_dependent_cached() {
    let tempdir = tempfile::tempdir().unwrap();
    let dir = tempdir.path();
    setup_fixture(dir, Some(true));

    run_task(dir, "build");
    // Outputs read after a cache restore fingerprint the same as after a run.
    fs::remove_dir_all(dir.join("packages/util/dist")).unwrap();
    fs::remove_dir_all(dir.join("apps/my-app/dist")).unwrap();
    assert_eq!(
        run_task(dir, "build"),
        expect(&[("my-app:build", true), ("util:build", true)])
    );
    assert!(dir.join("apps/my-app/dist/bundle.js").exists());
}

#[test]
fn transit_dependency_without_outputs_keeps_task_hash() {
    let tempdir = tempfile::tempdir().unwrap();
    let dir = tempdir.path();
    setup_fixture(dir, Some(true));

    assert_eq!(
        run_task(dir, "typecheck"),
        expect(&[("my-app:typecheck", false), ("util:typecheck", false)])
    );
    fs::write(dir.join("packages/util/README.md"), "# util, documented\n").unwrap();
    assert_eq!(
        run_task(dir, "typecheck"),
        expect(&[("my-app:typecheck", false), ("util:typecheck", false)]),
        "my-app#typecheck reaches util only through util#transit, which has no outputs"
    );
}

#[test]
fn dependency_without_a_command_keeps_task_hash() {
    let tempdir = tempfile::tempdir().unwrap();
    let dir = tempdir.path();
    setup_fixture(dir, Some(true));

    // util becomes an internal package compiled by its consumers: util#build
    // has no command, so its declared outputs say nothing about what my-app
    // reads.
    fs::write(
        dir.join("packages/util/package.json"),
        r#"{ "name": "util" }
"#,
    )
    .unwrap();
    fs::write(
        dir.join("apps/my-app/package.json"),
        r#"{
  "name": "my-app",
  "scripts": {
    "build": "node -e \"const fs = require('fs'); fs.mkdirSync('dist', { recursive: true }); fs.copyFileSync('../../packages/util/src/index.js', 'dist/bundle.js')\""
  },
  "dependencies": {
    "util": "*"
  }
}
"#,
    )
    .unwrap();
    git(dir, &["add", "."]);
    git(
        dir,
        &["commit", "-m", "compile util from source", "--quiet"],
    );

    run_task(dir, "build");
    fs::write(
        dir.join("packages/util/src/index.js"),
        "module.exports = 2;\n",
    )
    .unwrap();
    assert_eq!(run_task(dir, "build"), expect(&[("my-app:build", false)]));
    assert_eq!(
        fs::read_to_string(dir.join("apps/my-app/dist/bundle.js")).unwrap(),
        "module.exports = 2;\n"
    );
}

#[test]
fn dry_json_reports_deferred_hashes() {
    let tempdir = tempfile::tempdir().unwrap();
    let dir = tempdir.path();
    setup_fixture(dir, None);
    let flag_off = dry_json(dir, "build");

    fs::write(
        dir.join("turbo.json"),
        fs::read_to_string(dir.join("turbo.json"))
            .unwrap()
            .replacen(
                '{',
                r#"{ "futureFlags": { "hashDependencyOutputs": true },"#,
                1,
            ),
    )
    .unwrap();
    let flag_on = dry_json(dir, "build");

    assert!(is_task_hash(task_field(&flag_off, "my-app#build", "hash")));
    assert_eq!(
        task_field(&flag_on, "my-app#build", "hash"),
        &Value::Null,
        "my-app#build is hashed once util#build has produced its outputs"
    );
    assert_eq!(
        task_field(&flag_on, "my-app#build", "hashReason"),
        DEFERRED_MESSAGE
    );
    assert_eq!(
        task_field(&flag_on, "util#build", "hash"),
        task_field(&flag_off, "util#build", "hash"),
        "tasks without dependencies that have outputs keep their hash"
    );
    assert!(is_task_hash(task_field(&flag_on, "util#build", "hash")));

    let typecheck = dry_json(dir, "typecheck");
    for task in ["util#typecheck", "my-app#typecheck"] {
        assert!(
            is_task_hash(task_field(&typecheck, task, "hash")),
            "{task} depends only on output-less tasks and is hashed up front"
        );
    }
}

#[test]
fn query_reports_the_same_deferral_as_dry_run() {
    let tempdir = tempfile::tempdir().unwrap();
    let dir = tempdir.path();
    setup_fixture(dir, Some(true));

    let dry = dry_json(dir, "build");
    let query: Value = serde_json::from_str(&run(
        dir,
        &[
            "query",
            "query { packages { items { tasks { items { fullName hash hashReason } } } } }",
        ],
    ))
    .unwrap();
    let tasks = query["data"]["packages"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|package| package["tasks"]["items"].as_array().unwrap().clone())
        .map(|task| (task["fullName"].as_str().unwrap().to_string(), task))
        .collect::<BTreeMap<_, _>>();

    assert_eq!(tasks["my-app#build"]["hash"], Value::Null);
    assert_eq!(tasks["my-app#build"]["hashReason"], DEFERRED_MESSAGE);
    assert_eq!(
        &tasks["util#build"]["hash"],
        task_field(&dry, "util#build", "hash")
    );
}

#[test]
fn run_summary_records_hashes_computed_after_dependencies() {
    let tempdir = tempfile::tempdir().unwrap();
    let dir = tempdir.path();
    setup_fixture(dir, Some(true));

    let stdout = run(
        dir,
        &["run", "build", "--summarize", "--output-logs=hash-only"],
    );
    let runs = fs::read_dir(dir.join(".turbo/runs"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect::<Vec<_>>();
    assert_eq!(runs.len(), 1);
    let summary: Value = serde_json::from_str(&fs::read_to_string(&runs[0]).unwrap()).unwrap();
    let hash = task_field(&summary, "my-app#build", "hash");
    assert!(is_task_hash(hash), "{hash}");
    assert!(
        stdout.contains(&format!(
            "my-app:build: cache miss, executing {}",
            hash.as_str().unwrap()
        )),
        "{stdout}"
    );
}

#[test]
fn transitive_output_change_reaches_dependents_through_unchanged_outputs() {
    let tempdir = tempfile::tempdir().unwrap();
    let dir = tempdir.path();
    setup_fixture(dir, Some(true));

    // my-app -> util -> another. util's outputs only refer to another, so
    // they stay byte-identical when another's outputs change.
    let another = dir.join("packages/another");
    fs::create_dir_all(another.join("src")).unwrap();
    fs::write(another.join("src/index.js"), "module.exports = 'a';\n").unwrap();
    fs::write(another.join("README.md"), "# another\n").unwrap();
    fs::write(
        another.join("package.json"),
        r#"{
  "name": "another",
  "scripts": {
    "build": "node -e \"const fs = require('fs'); fs.mkdirSync('dist', { recursive: true }); fs.copyFileSync('src/index.js', 'dist/index.js')\""
  }
}
"#,
    )
    .unwrap();
    let util_manifest = dir.join("packages/util/package.json");
    let mut util: Value =
        serde_json::from_str(&fs::read_to_string(&util_manifest).unwrap()).unwrap();
    util["dependencies"] = serde_json::json!({ "another": "*" });
    fs::write(&util_manifest, serde_json::to_string_pretty(&util).unwrap()).unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "add another", "--quiet"]);

    assert_eq!(
        run_task(dir, "build"),
        expect(&[
            ("another:build", false),
            ("my-app:build", false),
            ("util:build", false)
        ])
    );

    fs::write(another.join("README.md"), "# another, documented\n").unwrap();
    assert_eq!(
        run_task(dir, "build"),
        expect(&[
            ("another:build", false),
            ("my-app:build", true),
            ("util:build", true)
        ]),
        "identical outputs two levels down cut off every dependent"
    );

    fs::write(another.join("src/index.js"), "module.exports = 'b';\n").unwrap();
    assert_eq!(
        run_task(dir, "build"),
        expect(&[
            ("another:build", false),
            ("my-app:build", false),
            ("util:build", false)
        ]),
        "util's outputs are unchanged, but my-app can load another through them"
    );
}
