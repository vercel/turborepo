#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{fs, path::Path};

use common::{git, run_turbo, run_turbo_with_env, setup, turbo_output_filters};

fn setup_affected(dir: &std::path::Path) {
    setup::setup_integration_test(dir, "basic_monorepo", "npm@10.5.0", false).unwrap();
    git(dir, &["checkout", "-b", "my-branch"]);
}

fn setup_berry_catalog_affected(dir: &Path) {
    fs::create_dir_all(dir.join("packages/pkg-a")).unwrap();
    fs::create_dir_all(dir.join("packages/pkg-b")).unwrap();

    fs::write(
        dir.join("package.json"),
        r#"{
  "name": "root",
  "private": true,
  "packageManager": "yarn@4.12.0",
  "workspaces": ["packages/*"],
  "dependencies": {
    "lodash": "catalog:"
  }
}
"#,
    )
    .unwrap();
    fs::write(
        dir.join("turbo.json"),
        r#"{"tasks":{}}
"#,
    )
    .unwrap();
    fs::write(
        dir.join(".yarnrc.yml"),
        "nodeLinker: node-modules\ncatalog:\n  chalk: ^5.3.0\n  lodash: ^4.17.21\n",
    )
    .unwrap();
    fs::write(
        dir.join("packages/pkg-a/package.json"),
        r#"{
  "name": "pkg-a",
  "version": "1.0.0"
}
"#,
    )
    .unwrap();
    fs::write(
        dir.join("packages/pkg-b/package.json"),
        r#"{
  "name": "pkg-b",
  "version": "1.0.0",
  "dependencies": {
    "chalk": "catalog:"
  }
}
"#,
    )
    .unwrap();
    fs::write(dir.join("yarn.lock"), berry_catalog_lockfile(false)).unwrap();

    git(dir, &["init", "--quiet", "--initial-branch=main"]);
    git(dir, &["config", "user.email", "turbo-test@example.com"]);
    git(dir, &["config", "user.name", "Turbo Test"]);
    git(dir, &["add", "."]);
    git(
        dir,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "Initial",
            "--quiet",
        ],
    );
    git(dir, &["checkout", "-b", "my-branch"]);
}

fn berry_catalog_lockfile(include_pkg_a_dependency: bool) -> String {
    let pkg_a_dependencies = if include_pkg_a_dependency {
        "  dependencies:\n    is-odd: 3.0.1\n"
    } else {
        ""
    };
    let is_odd_entries = if include_pkg_a_dependency {
        r#"
"is-number@npm:^6.0.0":
  version: 6.0.0
  resolution: "is-number@npm:6.0.0"
  checksum: abc123
  languageName: node
  linkType: hard

"is-odd@npm:3.0.1":
  version: 3.0.1
  resolution: "is-odd@npm:3.0.1"
  dependencies:
    is-number: ^6.0.0
  checksum: abc123
  languageName: node
  linkType: hard
"#
    } else {
        ""
    };

    format!(
        r#"__metadata:
  version: 8
  cacheKey: 10

"root@workspace:.":
  version: 0.0.0-use.local
  resolution: "root@workspace:."
  dependencies:
    lodash: "catalog:"
  languageName: unknown
  linkType: soft

"pkg-a@workspace:packages/pkg-a":
  version: 0.0.0-use.local
  resolution: "pkg-a@workspace:packages/pkg-a"
{pkg_a_dependencies}  languageName: unknown
  linkType: soft

"pkg-b@workspace:packages/pkg-b":
  version: 0.0.0-use.local
  resolution: "pkg-b@workspace:packages/pkg-b"
  dependencies:
    chalk: "catalog:"
  languageName: unknown
  linkType: soft

"chalk@npm:^5.3.0":
  version: 5.3.0
  resolution: "chalk@npm:5.3.0"
  checksum: abc123
  languageName: node
  linkType: hard

"lodash@npm:^4.17.21":
  version: 4.17.21
  resolution: "lodash@npm:4.17.21"
  checksum: abc123
  languageName: node
  linkType: hard
{is_odd_entries}"#,
    )
}

#[test]
fn test_nothing_affected_on_new_branch() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    let output = run_turbo(tempdir.path(), &["ls", "--affected"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("0 no packages"),
        "nothing should be affected: {stdout}"
    );
}

#[test]
fn test_affected_run_with_file_change() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    fs::write(tempdir.path().join("apps/my-app/new.js"), "foo").unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--affected", "--log-order", "grouped"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Packages in scope: my-app"));
    assert!(stdout.contains("1 successful, 1 total"));
}

#[test]
fn test_affected_ls_with_file_change() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    fs::write(tempdir.path().join("apps/my-app/new.js"), "foo").unwrap();

    let output = run_turbo(tempdir.path(), &["ls", "--affected"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("1 package"));
    assert!(stdout.contains("my-app"));
}

#[test]
fn test_affected_query_file_changed() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    fs::write(tempdir.path().join("apps/my-app/new.js"), "foo").unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedPackages { items { name reason { __typename } } } }",
        ],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = json["data"]["affectedPackages"]["items"]
        .as_array()
        .unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "my-app");
    assert_eq!(items[0]["reason"]["__typename"], "FileChanged");
}

#[test]
fn test_affected_dependency_changed() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    fs::write(tempdir.path().join("packages/util/new.js"), "hello world").unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedPackages { items { name reason { __typename } } } }",
        ],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = json["data"]["affectedPackages"]["items"]
        .as_array()
        .unwrap();
    let names: Vec<&str> = items.iter().map(|i| i["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"util"));
    assert!(names.contains(&"my-app"));

    let util_item = items.iter().find(|i| i["name"] == "util").unwrap();
    assert_eq!(util_item["reason"]["__typename"], "FileChanged");
    let app_item = items.iter().find(|i| i["name"] == "my-app").unwrap();
    assert_eq!(app_item["reason"]["__typename"], "DependencyChanged");
}

#[test]
fn test_affected_berry_catalog_lockfile_change_does_not_affect_unchanged_workspace() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_berry_catalog_affected(tempdir.path());

    fs::write(
        tempdir.path().join("packages/pkg-a/package.json"),
        r#"{
  "name": "pkg-a",
  "version": "1.0.0",
  "dependencies": {
    "is-odd": "3.0.1"
  }
}
"#,
    )
    .unwrap();
    fs::write(
        tempdir.path().join("yarn.lock"),
        berry_catalog_lockfile(true),
    )
    .unwrap();
    git(tempdir.path(), &["add", "."]);
    git(
        tempdir.path(),
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "add pkg-a dependency",
            "--quiet",
        ],
    );

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            r#"query {
  affectedPackages(base: "HEAD~1", head: "HEAD") {
    items {
      name
      reason {
        __typename
        ... on LockfileChanged {
          added { items { name } }
          removed { items { name } }
        }
      }
    }
  }
}"#,
        ],
    );

    assert!(
        output.status.success(),
        "query should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = json["data"]["affectedPackages"]["items"]
        .as_array()
        .unwrap();
    let names: Vec<&str> = items.iter().map(|i| i["name"].as_str().unwrap()).collect();
    let workspace_names: Vec<&str> = names.iter().copied().filter(|name| *name != "//").collect();

    assert_eq!(
        workspace_names,
        vec!["pkg-a"],
        "unexpected affected packages: {items:?}"
    );
    let pkg_a = items.iter().find(|i| i["name"] == "pkg-a").unwrap();
    assert_eq!(pkg_a["reason"]["__typename"], "FileChanged");
}

#[test]
fn test_affected_committed_change() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    let pkg_path = tempdir.path().join("apps/my-app/package.json");
    let contents = fs::read_to_string(&pkg_path).unwrap();
    let mut pkg: serde_json::Value = serde_json::from_str(&contents).unwrap();
    pkg["description"] = serde_json::Value::String("foo".to_string());
    fs::write(&pkg_path, serde_json::to_string_pretty(&pkg).unwrap()).unwrap();

    git(tempdir.path(), &["add", "."]);
    git(tempdir.path(), &["commit", "-m", "add foo", "--quiet"]);

    let output = run_turbo(tempdir.path(), &["ls", "--affected"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("my-app"));
}

#[test]
fn test_affected_scm_base_override() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    fs::write(tempdir.path().join("apps/my-app/new.js"), "foo").unwrap();
    git(tempdir.path(), &["add", "."]);
    git(tempdir.path(), &["commit", "-m", "add foo", "--quiet"]);

    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--affected", "--log-order", "grouped"],
        &[("TURBO_SCM_BASE", "HEAD")],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("0 successful, 0 total"),
        "SCM_BASE=HEAD should show no changes: {stdout}"
    );
}

#[test]
fn test_affected_scm_head_override() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    fs::write(tempdir.path().join("apps/my-app/new.js"), "foo").unwrap();
    git(tempdir.path(), &["add", "."]);
    git(tempdir.path(), &["commit", "-m", "add foo", "--quiet"]);

    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--affected", "--log-order", "grouped"],
        &[("TURBO_SCM_HEAD", "main")],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("0 successful, 0 total"),
        "SCM_HEAD=main should show no changes: {stdout}"
    );
}

#[test]
fn test_affected_merge_base_diverged() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    let pkg_path = tempdir.path().join("apps/my-app/package.json");
    let contents = fs::read_to_string(&pkg_path).unwrap();
    let mut pkg: serde_json::Value = serde_json::from_str(&contents).unwrap();
    pkg["description"] = serde_json::Value::String("foo".to_string());
    fs::write(&pkg_path, serde_json::to_string_pretty(&pkg).unwrap()).unwrap();
    git(tempdir.path(), &["add", "."]);
    git(tempdir.path(), &["commit", "-m", "add foo", "--quiet"]);

    git(tempdir.path(), &["checkout", "main", "--quiet"]);
    let index_path = tempdir.path().join("packages/util/index.js");
    let mut idx = fs::read_to_string(&index_path).unwrap_or_default();
    idx.push_str("\nfoo");
    fs::write(&index_path, idx).unwrap();
    git(tempdir.path(), &["add", "."]);
    git(tempdir.path(), &["commit", "-m", "add foo", "--quiet"]);
    git(tempdir.path(), &["checkout", "my-branch", "--quiet"]);

    let output = run_turbo(tempdir.path(), &["ls", "--affected"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("my-app"));
    assert!(
        !stdout.contains("util"),
        "util changed on main, not branch: {stdout}"
    );
}

#[test]
fn test_filter_merge_base_range_with_filter_using_tasks() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let turbo_json_path = tempdir.path().join("turbo.json");
    let turbo_json = fs::read_to_string(&turbo_json_path).unwrap();
    let turbo_json = turbo_json.replacen(
        "{\n",
        "{\n  \"futureFlags\": { \"filterUsingTasks\": true },\n",
        1,
    );
    fs::write(&turbo_json_path, turbo_json).unwrap();
    git(tempdir.path(), &["add", "turbo.json"]);
    git(
        tempdir.path(),
        &["commit", "-m", "configure future flags", "--quiet"],
    );
    git(tempdir.path(), &["checkout", "-b", "my-branch", "--quiet"]);

    let pkg_path = tempdir.path().join("apps/my-app/package.json");
    let contents = fs::read_to_string(&pkg_path).unwrap();
    let mut pkg: serde_json::Value = serde_json::from_str(&contents).unwrap();
    pkg["description"] = serde_json::Value::String("foo".to_string());
    fs::write(&pkg_path, serde_json::to_string_pretty(&pkg).unwrap()).unwrap();
    git(tempdir.path(), &["add", "."]);
    git(
        tempdir.path(),
        &["commit", "-m", "change my-app", "--quiet"],
    );

    git(tempdir.path(), &["checkout", "main", "--quiet"]);
    let index_path = tempdir.path().join("packages/util/index.js");
    let mut idx = fs::read_to_string(&index_path).unwrap_or_default();
    idx.push_str("\nfoo");
    fs::write(&index_path, idx).unwrap();
    git(tempdir.path(), &["add", "."]);
    git(
        tempdir.path(),
        &["commit", "-m", "change util on main", "--quiet"],
    );
    git(tempdir.path(), &["checkout", "my-branch", "--quiet"]);

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=[main...HEAD]", "--dry=json"],
    );
    assert!(
        output.status.success(),
        "run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let tasks: std::collections::HashSet<String> = json["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["taskId"].as_str().unwrap().to_string())
        .collect();

    assert!(tasks.contains("my-app#build"), "{tasks:?}");
    assert!(
        !tasks.contains("util#build"),
        "util changed on main after the branch point, a merge-base range must not select it: \
         {tasks:?}"
    );
}

// ── affectedTasks tests ──

#[test]
fn test_affected_tasks_nothing_on_new_branch() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedTasks { length items { name fullName reason { __typename } } } }",
        ],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["data"]["affectedTasks"]["length"], 0);
}

#[test]
fn test_affected_tasks_file_change() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    fs::write(tempdir.path().join("apps/my-app/new.js"), "foo").unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedTasks { items { name fullName package { name } reason { __typename } \
             } } }",
        ],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = json["data"]["affectedTasks"]["items"].as_array().unwrap();

    // my-app has build and maybefails scripts — both should be affected
    let my_app_tasks: Vec<&str> = items
        .iter()
        .filter(|i| i["package"]["name"] == "my-app")
        .map(|i| i["name"].as_str().unwrap())
        .collect();
    assert!(
        my_app_tasks.contains(&"build"),
        "my-app#build should be affected: {items:?}"
    );
    assert!(
        my_app_tasks.contains(&"maybefails"),
        "my-app#maybefails should be affected: {items:?}"
    );

    // All my-app tasks should have TaskFileChanged reason
    for item in items.iter().filter(|i| i["package"]["name"] == "my-app") {
        assert_eq!(
            item["reason"]["__typename"], "TaskFileChanged",
            "expected TaskFileChanged for my-app task: {item:?}"
        );
    }
}

#[test]
fn test_affected_tasks_global_dep_change() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    // Change foo.txt which is a globalDependency
    fs::write(tempdir.path().join("foo.txt"), "changed").unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedTasks { length items { name package { name } reason { __typename } } \
             } }",
        ],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let length = json["data"]["affectedTasks"]["length"].as_i64().unwrap();
    // All tasks should be affected when a globalDependency changes
    assert!(
        length > 0,
        "all tasks should be affected when globalDependency changes"
    );

    let items = json["data"]["affectedTasks"]["items"].as_array().unwrap();
    // All should have GlobalDepsChanged reason
    for item in items {
        assert_eq!(
            item["reason"]["__typename"], "TaskGlobalDepsChanged",
            "expected TaskGlobalDepsChanged reason: {item:?}"
        );
    }
}

fn setup_affected_tasks_fixture_with_flags(dir: &std::path::Path, future_flags: serde_json::Value) {
    setup::setup_integration_test(dir, "affected_tasks_inputs", "npm@10.5.0", false).unwrap();
    if future_flags != serde_json::json!({}) {
        let turbo_json_path = dir.join("turbo.json");
        let mut turbo_json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&turbo_json_path).unwrap()).unwrap();
        turbo_json["futureFlags"] = future_flags;
        fs::write(
            turbo_json_path,
            serde_json::to_string_pretty(&turbo_json).unwrap(),
        )
        .unwrap();
        git(dir, &["add", "turbo.json"]);
        git(dir, &["commit", "-m", "configure future flags", "--quiet"]);
    }
    git(dir, &["checkout", "-b", "my-branch"]);
}

fn setup_affected_tasks_fixture(dir: &std::path::Path) {
    setup_affected_tasks_fixture_with_flags(dir, serde_json::json!({}));
}

#[test]
fn test_affected_tasks_filter_by_task_name() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected_tasks_fixture(tempdir.path());

    // Change a source file — all tasks in lib-a should be affected
    fs::write(
        tempdir.path().join("packages/lib-a/index.ts"),
        "export const changed = true;",
    )
    .unwrap();

    // Without filter — should include build, test, and typecheck
    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedTasks { length items { name package { name } } } }",
        ],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let all_count = json["data"]["affectedTasks"]["length"].as_i64().unwrap();
    assert!(
        all_count >= 3,
        "should have at least build + test + typecheck for lib-a: got {all_count}"
    );

    // With filter — only test tasks
    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedTasks(tasks: [\"test\"]) { length items { name package { name } } } }",
        ],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = json["data"]["affectedTasks"]["items"].as_array().unwrap();
    let task_ids: std::collections::HashSet<_> = items
        .iter()
        .map(|item| {
            format!(
                "{}#{}",
                item["package"]["name"].as_str().unwrap(),
                item["name"].as_str().unwrap()
            )
        })
        .collect();
    assert!(task_ids.contains("lib-a#test"));
    assert!(
        task_ids.contains("lib-a#build"),
        "task filter should retain execution dependencies: {items:?}"
    );
}

#[test]
fn test_affected_tasks_excludes_packages_without_script() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected_tasks_fixture(tempdir.path());

    // Change a file in lib-no-test (which has build + typecheck but NOT test)
    fs::write(
        tempdir.path().join("packages/lib-no-test/index.ts"),
        "export const changed = true;",
    )
    .unwrap();

    // affectedTasks(tasks: ["test"]) should NOT include lib-no-test
    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedTasks(tasks: [\"test\"]) { items { name package { name } } } }",
        ],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = json["data"]["affectedTasks"]["items"].as_array().unwrap();
    for item in items {
        assert_ne!(
            item["package"]["name"], "lib-no-test",
            "lib-no-test has no test script and should not appear in affectedTasks(tasks: \
             [\"test\"])"
        );
    }

    // affectedTasks (no filter) should also not include phantom test task for
    // lib-no-test
    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedTasks { items { name package { name } } } }",
        ],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = json["data"]["affectedTasks"]["items"].as_array().unwrap();
    let phantom_test = items
        .iter()
        .any(|item| item["package"]["name"] == "lib-no-test" && item["name"] == "test");
    assert!(
        !phantom_test,
        "lib-no-test#test should not appear — no test script in package.json"
    );

    // But lib-no-test's real tasks (build, typecheck) SHOULD still appear
    let has_build = items
        .iter()
        .any(|item| item["package"]["name"] == "lib-no-test" && item["name"] == "build");
    let has_typecheck = items
        .iter()
        .any(|item| item["package"]["name"] == "lib-no-test" && item["name"] == "typecheck");
    assert!(has_build, "lib-no-test#build should be affected");
    assert!(has_typecheck, "lib-no-test#typecheck should be affected");
}

fn affected_query_task_ids(dir: &Path, tasks: &[&str]) -> std::collections::HashSet<String> {
    let task_names = tasks
        .iter()
        .map(|task| format!("\"{task}\""))
        .collect::<Vec<_>>()
        .join(",");
    let query =
        format!("query {{ affectedTasks(tasks: [{task_names}]) {{ items {{ fullName }} }} }}");
    let output = run_turbo(dir, &["query", &query]);
    assert!(
        output.status.success(),
        "affected query failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    json["data"]["affectedTasks"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["fullName"].as_str().unwrap().to_string())
        .collect()
}

fn affected_run_task_ids(dir: &Path, tasks: &[&str]) -> std::collections::HashSet<String> {
    let mut args = vec!["run"];
    args.extend_from_slice(tasks);
    args.extend_from_slice(&["--affected", "--dry=json"]);
    let output = run_turbo(dir, &args);
    assert!(
        output.status.success(),
        "affected run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    json["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["taskId"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn test_affected_tasks_matches_run_for_virtual_task() {
    for future_flags in [
        serde_json::json!({}),
        serde_json::json!({ "affectedUsingTaskInputs": false }),
        serde_json::json!({ "affectedUsingTaskInputs": true }),
        serde_json::json!({ "filterUsingTasks": true }),
        serde_json::json!({ "strictTaskEntrypointSelection": true }),
    ] {
        let tempdir = tempfile::tempdir().unwrap();
        setup_affected_tasks_fixture_with_flags(tempdir.path(), future_flags.clone());

        fs::write(
            tempdir.path().join("packages/lib-a/index.ts"),
            "export const changed = true;",
        )
        .unwrap();

        let query_tasks = affected_query_task_ids(tempdir.path(), &["aggregate"]);
        let run_tasks = affected_run_task_ids(tempdir.path(), &["aggregate"]);
        assert_eq!(
            query_tasks, run_tasks,
            "affected query and run differ with {future_flags}"
        );
        assert!(query_tasks.contains("lib-a#aggregate"));
        assert!(query_tasks.contains("lib-a#build"));
    }
}

#[test]
fn test_affected_tasks_matches_run_with_command_overrides() {
    for command in [
        serde_json::json!(null),
        serde_json::json!(["echo", "aggregate"]),
    ] {
        let tempdir = tempfile::tempdir().unwrap();
        setup::setup_integration_test(tempdir.path(), "affected_tasks_inputs", "npm@10.5.0", false)
            .unwrap();

        let turbo_json_path = tempdir.path().join("turbo.json");
        let mut turbo_json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&turbo_json_path).unwrap()).unwrap();
        turbo_json["futureFlags"] = serde_json::json!({
            "affectedUsingTaskInputs": true,
            "experimentalTaskCommand": true
        });
        turbo_json["tasks"]["lib-a#aggregate"] = serde_json::json!({
            "dependsOn": ["build"],
            "command": command
        });
        fs::write(
            &turbo_json_path,
            serde_json::to_string_pretty(&turbo_json).unwrap(),
        )
        .unwrap();
        git(tempdir.path(), &["add", "turbo.json"]);
        git(
            tempdir.path(),
            &["commit", "-m", "configure command override", "--quiet"],
        );
        git(tempdir.path(), &["checkout", "-b", "my-branch"]);

        fs::write(
            tempdir.path().join("packages/lib-a/index.ts"),
            "export const changed = true;",
        )
        .unwrap();

        let query_tasks = affected_query_task_ids(tempdir.path(), &["aggregate"]);
        let run_tasks = affected_run_task_ids(tempdir.path(), &["aggregate"]);
        assert_eq!(
            query_tasks, run_tasks,
            "affected query and run differ with command override {command}"
        );
    }
}

#[test]
fn test_affected_tasks_matches_run_task_input_flag_behavior() {
    for (future_flags, expected_lib_tasks) in [
        (
            serde_json::json!({}),
            [
                "lib-a#aggregate",
                "lib-a#build",
                "lib-a#test",
                "lib-a#typecheck",
            ]
            .as_slice(),
        ),
        (
            serde_json::json!({ "affectedUsingTaskInputs": false }),
            [
                "lib-a#aggregate",
                "lib-a#build",
                "lib-a#test",
                "lib-a#typecheck",
            ]
            .as_slice(),
        ),
        (
            serde_json::json!({ "affectedUsingTaskInputs": true }),
            ["lib-a#aggregate", "lib-a#build"].as_slice(),
        ),
        (
            serde_json::json!({ "filterUsingTasks": true }),
            ["lib-a#aggregate", "lib-a#build"].as_slice(),
        ),
    ] {
        let tempdir = tempfile::tempdir().unwrap();
        setup_affected_tasks_fixture_with_flags(tempdir.path(), future_flags.clone());
        fs::write(tempdir.path().join("packages/lib-a/README.md"), "changed").unwrap();

        let tasks = ["aggregate", "build", "test", "typecheck"];
        let query_tasks = affected_query_task_ids(tempdir.path(), &tasks);
        let run_tasks = affected_run_task_ids(tempdir.path(), &tasks);
        assert_eq!(
            query_tasks, run_tasks,
            "affected query and run differ with {future_flags}"
        );
        for task in expected_lib_tasks {
            assert!(
                query_tasks.contains(*task),
                "missing {task} with {future_flags}"
            );
        }
    }
}

#[test]
fn test_root_package_json_change_does_not_globally_affect_tasks() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected_tasks_fixture(tempdir.path());

    // root package.json is not in the global hash (when a lockfile exists),
    // so changing it should not mark all tasks as affected.
    let root_pkg = tempdir.path().join("package.json");
    let contents = fs::read_to_string(&root_pkg).unwrap();
    let mut pkg: serde_json::Value = serde_json::from_str(&contents).unwrap();
    pkg["description"] = serde_json::Value::String("changed".to_string());
    fs::write(&root_pkg, serde_json::to_string_pretty(&pkg).unwrap()).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["query", "query { affectedTasks { length } }"],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let length = json["data"]["affectedTasks"]["length"].as_i64().unwrap();
    assert_eq!(
        length, 0,
        "root package.json change should not globally affect tasks"
    );
}

#[test]
fn test_affected_tasks_with_explicit_base() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected_tasks_fixture(tempdir.path());

    // Commit a change, then query with base=HEAD to see no tasks affected
    fs::write(
        tempdir.path().join("packages/lib-a/index.ts"),
        "export const changed = true;",
    )
    .unwrap();
    git(tempdir.path(), &["add", "."]);
    git(tempdir.path(), &["commit", "-m", "change lib-a", "--quiet"]);

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedTasks(base: \"HEAD\") { length } }",
        ],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["data"]["affectedTasks"]["length"], 0,
        "base=HEAD with no uncommitted changes should show 0 affected tasks"
    );
}

#[test]
fn test_affected_with_nonexistent_task_errors() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    let output = run_turbo(
        tempdir.path(),
        &["run", "foobarbaz", "--affected", "--log-order", "grouped"],
    );
    assert!(
        !output.status.success(),
        "expected failure for non-existent task with --affected"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    insta::with_settings!({ filters => turbo_output_filters() }, {
        insta::assert_snapshot!("affected_nonexistent_task", stderr.to_string());
    });
}

// Query construction and success exit-code behavior are tested in-process.
// Keep this binary smoke for the query-error exit code.

#[test]
fn test_query_affected_exit_code_error_returns_2() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    // An invalid --base ref should produce a query error, which exits 2
    // (distinct from exit 1 meaning "affected results found").
    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "affected",
            "--base",
            "nonexistent-ref-00000",
            "--exit-code",
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(2),
        "--exit-code should exit 2 on query errors, not 1"
    );
}

#[test]
fn test_affected_with_filter_ls() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    // Change util → affected = {util, my-app}
    fs::write(tempdir.path().join("packages/util/new.js"), "hello").unwrap();

    // turbo ls --affected --filter=my-app should list only my-app
    let output = run_turbo(tempdir.path(), &["ls", "--affected", "--filter=my-app"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("1 package"), "expected 1 package: {stdout}");
    assert!(
        stdout.contains("my-app"),
        "my-app should be listed: {stdout}"
    );
}
