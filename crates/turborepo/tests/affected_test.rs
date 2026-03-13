mod common;

use std::{fs, path::Path};

use common::{git, run_turbo, run_turbo_with_env, setup, turbo_output_filters};

fn setup_affected(dir: &std::path::Path) {
    setup::setup_integration_test(dir, "basic_monorepo", "npm@10.5.0", true).unwrap();
    git(dir, &["checkout", "-b", "my-branch"]);
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
fn test_affected_tasks_dependency_propagation() {
    let tempdir = tempfile::tempdir().unwrap();
    // Use the task_dependencies/query fixture which has ^build0 dependencies
    setup::setup_integration_test(
        tempdir.path(),
        "task_dependencies/query",
        "npm@10.5.0",
        true,
    )
    .unwrap();
    git(tempdir.path(), &["checkout", "-b", "my-branch"]);

    // lib-a has no dependents with ^build0 in this fixture, but app-a depends
    // on lib-a and has build0 dependsOn: ['^build0']. Changing a file in lib-a
    // should propagate to app-a's build0 task through the task graph.
    fs::write(tempdir.path().join("lib-a/new.js"), "hello world").unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedTasks { items { name package { name } reason { __typename } } } }",
        ],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = json["data"]["affectedTasks"]["items"].as_array().unwrap();

    // lib-a's tasks should be directly affected
    let lib_a_tasks: Vec<&str> = items
        .iter()
        .filter(|i| i["package"]["name"] == "lib-a")
        .map(|i| i["name"].as_str().unwrap())
        .collect();
    assert!(
        !lib_a_tasks.is_empty(),
        "lib-a should have affected tasks: {items:?}"
    );

    // app-a depends on lib-a and has:
    //   test dependsOn: ['^build0', 'prepare']
    // So app-a#test should be affected via task dependency propagation from
    // lib-a#build0
    let app_a_tasks: Vec<&str> = items
        .iter()
        .filter(|i| i["package"]["name"] == "app-a")
        .map(|i| i["name"].as_str().unwrap())
        .collect();
    assert!(
        app_a_tasks.contains(&"test"),
        "app-a#test should be affected through ^build0 dependency on lib-a: {items:?}"
    );

    // Tasks propagated through the task graph should have TaskDependencyTaskChanged
    // reason
    let app_a_test = items
        .iter()
        .find(|i| i["package"]["name"] == "app-a" && i["name"] == "test")
        .unwrap();
    assert_eq!(
        app_a_test["reason"]["__typename"], "TaskDependencyTaskChanged",
        "app-a#test reason should be TaskDependencyTaskChanged: {app_a_test:?}"
    );
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
fn test_affected_tasks_input_exclusion() {
    let tempdir = tempfile::tempdir().unwrap();
    // Use a special fixture that has task input exclusions
    setup::setup_integration_test(tempdir.path(), "affected_tasks_inputs", "npm@10.5.0", true)
        .unwrap();
    git(tempdir.path(), &["checkout", "-b", "my-branch"]);

    // Change a .md file which is excluded from the test task's inputs
    fs::write(
        tempdir.path().join("packages/lib-a/README.md"),
        "updated docs",
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedTasks { items { name package { name } reason { __typename } } } }",
        ],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = json["data"]["affectedTasks"]["items"].as_array().unwrap();

    // The "test" task excludes *.md files, so it should NOT be affected
    let test_tasks: Vec<_> = items
        .iter()
        .filter(|i| i["package"]["name"] == "lib-a" && i["name"] == "test")
        .collect();
    assert!(
        test_tasks.is_empty(),
        "lib-a#test should NOT be affected by .md changes (excluded by inputs): {items:?}"
    );

    // The "build" task uses $TURBO_DEFAULT$ without exclusions, so it SHOULD be
    // affected
    let build_tasks: Vec<_> = items
        .iter()
        .filter(|i| i["package"]["name"] == "lib-a" && i["name"] == "build")
        .collect();
    assert!(
        !build_tasks.is_empty(),
        "lib-a#build SHOULD be affected by .md changes (default inputs): {items:?}"
    );
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

fn setup_affected_tasks_fixture(dir: &std::path::Path) {
    setup::setup_integration_test(dir, "affected_tasks_inputs", "npm@10.5.0", true).unwrap();
    git(dir, &["checkout", "-b", "my-branch"]);
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
    for item in items {
        assert_eq!(
            item["name"], "test",
            "filter should only return test tasks: {item:?}"
        );
    }
}

#[test]
fn test_affected_tasks_test_file_excluded_from_typecheck() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected_tasks_fixture(tempdir.path());

    // Change only a .test.ts file
    fs::write(
        tempdir.path().join("packages/lib-a/index.test.ts"),
        "// updated test",
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedTasks { items { name package { name } } } }",
        ],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = json["data"]["affectedTasks"]["items"].as_array().unwrap();

    let lib_a_tasks: Vec<&str> = items
        .iter()
        .filter(|i| i["package"]["name"] == "lib-a")
        .map(|i| i["name"].as_str().unwrap())
        .collect();

    // test task includes $TURBO_DEFAULT$ minus *.md — .test.ts matches
    assert!(
        lib_a_tasks.contains(&"test"),
        "lib-a#test SHOULD be affected by .test.ts changes: {items:?}"
    );

    // build task has no input exclusions — .test.ts matches default inputs
    assert!(
        lib_a_tasks.contains(&"build"),
        "lib-a#build SHOULD be affected by .test.ts changes: {items:?}"
    );

    // typecheck excludes both *.md and *.test.ts — should NOT be affected
    assert!(
        !lib_a_tasks.contains(&"typecheck"),
        "lib-a#typecheck should NOT be affected by .test.ts changes: {items:?}"
    );
}

#[test]
fn test_affected_tasks_propagation_through_task_deps() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected_tasks_fixture(tempdir.path());

    // Change a source file in lib-a. app-a depends on lib-a and has
    // build dependsOn: ['^build'], test dependsOn: ['^build'], typecheck dependsOn:
    // ['^build']
    fs::write(
        tempdir.path().join("packages/lib-a/index.ts"),
        "export const changed = true;",
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedTasks { items { name package { name } reason { __typename } } } }",
        ],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = json["data"]["affectedTasks"]["items"].as_array().unwrap();

    // lib-a tasks should be directly affected (FileChanged)
    let lib_a_build = items
        .iter()
        .find(|i| i["package"]["name"] == "lib-a" && i["name"] == "build");
    assert!(
        lib_a_build.is_some(),
        "lib-a#build should be directly affected: {items:?}"
    );
    assert_eq!(
        lib_a_build.unwrap()["reason"]["__typename"],
        "TaskFileChanged"
    );

    // app-a tasks should be affected through task dependency propagation
    // because they depend on ^build which includes lib-a#build
    let app_a_tasks: Vec<(&str, &str)> = items
        .iter()
        .filter(|i| i["package"]["name"] == "app-a")
        .map(|i| {
            (
                i["name"].as_str().unwrap(),
                i["reason"]["__typename"].as_str().unwrap(),
            )
        })
        .collect();
    assert!(
        !app_a_tasks.is_empty(),
        "app-a should have affected tasks through dependency propagation: {items:?}"
    );

    // Propagated tasks should have TaskDependencyTaskChanged reason
    for (task_name, reason) in &app_a_tasks {
        assert_eq!(
            *reason, "TaskDependencyTaskChanged",
            "app-a#{task_name} should be affected via dependency: {items:?}"
        );
    }
}

#[test]
fn test_affected_tasks_md_change_no_propagation() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected_tasks_fixture(tempdir.path());

    // Change only a .md file in lib-a. Since test and typecheck exclude *.md,
    // only build should be directly affected. The question is whether app-a's
    // tasks get propagated — they should, because app-a's tasks depend on
    // ^build and lib-a#build IS affected.
    fs::write(tempdir.path().join("packages/lib-a/README.md"), "# updated").unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedTasks { items { name package { name } reason { __typename } } } }",
        ],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = json["data"]["affectedTasks"]["items"].as_array().unwrap();

    // lib-a#build should be directly affected (no exclusions)
    assert!(
        items
            .iter()
            .any(|i| i["package"]["name"] == "lib-a" && i["name"] == "build"),
        "lib-a#build should be affected: {items:?}"
    );

    // lib-a#test and lib-a#typecheck should NOT be directly affected (*.md
    // excluded)
    assert!(
        !items
            .iter()
            .any(|i| i["package"]["name"] == "lib-a" && i["name"] == "test"),
        "lib-a#test should NOT be affected by .md change: {items:?}"
    );
    assert!(
        !items
            .iter()
            .any(|i| i["package"]["name"] == "lib-a" && i["name"] == "typecheck"),
        "lib-a#typecheck should NOT be affected by .md change: {items:?}"
    );

    // app-a's tasks should still be affected through ^build propagation
    // from lib-a#build
    assert!(
        items
            .iter()
            .any(|i| i["package"]["name"] == "app-a" && i["name"] == "build"),
        "app-a#build should be affected via ^build from lib-a: {items:?}"
    );
}

#[test]
fn test_affected_tasks_default_global_file_change() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected_tasks_fixture(tempdir.path());

    // package.json is a default global file — changing it marks everything affected
    let root_pkg = tempdir.path().join("package.json");
    let contents = fs::read_to_string(&root_pkg).unwrap();
    let mut pkg: serde_json::Value = serde_json::from_str(&contents).unwrap();
    pkg["description"] = serde_json::Value::String("changed".to_string());
    fs::write(&root_pkg, serde_json::to_string_pretty(&pkg).unwrap()).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedTasks { length items { reason { __typename } } } }",
        ],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let length = json["data"]["affectedTasks"]["length"].as_i64().unwrap();
    assert!(length > 0, "all tasks should be affected");

    let items = json["data"]["affectedTasks"]["items"].as_array().unwrap();
    for item in items {
        assert_eq!(
            item["reason"]["__typename"], "TaskGlobalFileChanged",
            "default global file change should produce TaskGlobalFileChanged: {item:?}"
        );
    }
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

// ── turbo run --affected + affectedUsingTaskInputs future flag ──

const TURBO_JSON_WITH_TASK_INPUTS_FLAG: &str = r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "tasks": {
    "build": {
      "dependsOn": ["^build"],
      "outputs": []
    },
    "test": {
      "dependsOn": ["^build"],
      "inputs": ["$TURBO_DEFAULT$", "!**/*.md"]
    },
    "typecheck": {
      "dependsOn": ["^build"],
      "inputs": ["$TURBO_DEFAULT$", "!**/*.md", "!**/*.test.ts"]
    }
  },
  "futureFlags": {
    "affectedUsingTaskInputs": true
  }
}
"#;

/// Sets up the `affected_tasks_inputs` fixture with the
/// `affectedUsingTaskInputs` future flag enabled.
fn setup_task_level_affected(dir: &Path) {
    setup::setup_integration_test(dir, "affected_tasks_inputs", "npm@10.5.0", true).unwrap();
    // Enable the future flag before branching so it's on main.
    fs::write(dir.join("turbo.json"), TURBO_JSON_WITH_TASK_INPUTS_FLAG).unwrap();
    git(dir, &["add", "."]);
    git(
        dir,
        &["commit", "-m", "enable task-level affected", "--quiet"],
    );
    git(dir, &["checkout", "-b", "my-branch"]);
}

#[test]
fn test_task_level_affected_no_changes() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_task_level_affected(tempdir.path());

    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "build",
            "test",
            "typecheck",
            "--affected",
            "--log-order",
            "grouped",
        ],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("0 successful, 0 total"),
        "no changes → no tasks should run: {stdout}"
    );
}

#[test]
fn test_task_level_affected_source_file_change() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_task_level_affected(tempdir.path());

    // Change a .ts source file in lib-a. All three tasks (build, test,
    // typecheck) should be affected since .ts matches default inputs and
    // isn't excluded by any of them.
    fs::write(
        tempdir.path().join("packages/lib-a/index.ts"),
        "export const changed = true;",
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "build",
            "test",
            "typecheck",
            "--affected",
            "--dry=json",
        ],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("failed to parse dry run JSON: {e}\nstdout: {stdout}"));

    let tasks = json["tasks"].as_array().expect("tasks array");
    let task_ids: Vec<&str> = tasks
        .iter()
        .map(|t| t["taskId"].as_str().unwrap())
        .collect();

    // lib-a's build, test, and typecheck should all be affected
    assert!(
        task_ids.contains(&"lib-a#build"),
        "lib-a#build should be affected: {task_ids:?}"
    );
    assert!(
        task_ids.contains(&"lib-a#test"),
        "lib-a#test should be affected: {task_ids:?}"
    );
    assert!(
        task_ids.contains(&"lib-a#typecheck"),
        "lib-a#typecheck should be affected: {task_ids:?}"
    );

    // app-a depends on ^build so its tasks should also be affected
    assert!(
        task_ids.contains(&"app-a#build"),
        "app-a#build should be affected via ^build: {task_ids:?}"
    );
}

#[test]
fn test_task_level_affected_md_excludes_test_and_typecheck() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_task_level_affected(tempdir.path());

    // Change only a .md file in lib-a. The test task excludes *.md and the
    // typecheck task excludes *.md. Only build (default inputs) should be
    // directly affected in lib-a.
    fs::write(
        tempdir.path().join("packages/lib-a/README.md"),
        "# updated docs",
    )
    .unwrap();

    // Use --dry=json to inspect exactly which tasks are planned
    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "build",
            "test",
            "typecheck",
            "--affected",
            "--dry=json",
        ],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("failed to parse dry run JSON: {e}\nstdout: {stdout}"));

    let tasks = json["tasks"]
        .as_array()
        .expect("tasks array in dry run JSON");
    let task_ids: Vec<&str> = tasks
        .iter()
        .map(|t| t["taskId"].as_str().unwrap())
        .collect();

    // lib-a#build should be affected (default inputs match .md)
    assert!(
        task_ids.contains(&"lib-a#build"),
        "lib-a#build should be affected by .md change: {task_ids:?}"
    );

    // lib-a#test should NOT be affected (inputs exclude *.md)
    assert!(
        !task_ids.contains(&"lib-a#test"),
        "lib-a#test should NOT be affected by .md change (excluded by inputs): {task_ids:?}"
    );

    // lib-a#typecheck should NOT be affected (inputs exclude *.md)
    assert!(
        !task_ids.contains(&"lib-a#typecheck"),
        "lib-a#typecheck should NOT be affected by .md change (excluded by inputs): {task_ids:?}"
    );

    // app-a tasks should be affected via dependency propagation from lib-a#build
    assert!(
        task_ids.contains(&"app-a#build"),
        "app-a#build should be affected via ^build from lib-a: {task_ids:?}"
    );
}

#[test]
fn test_task_level_affected_test_file_excludes_typecheck() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_task_level_affected(tempdir.path());

    // Change only a .test.ts file. typecheck excludes both *.md and *.test.ts.
    fs::write(
        tempdir.path().join("packages/lib-a/index.test.ts"),
        "// updated test",
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "build",
            "test",
            "typecheck",
            "--affected",
            "--dry=json",
        ],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("failed to parse dry run JSON: {e}\nstdout: {stdout}"));

    let tasks = json["tasks"].as_array().expect("tasks array");
    let task_ids: Vec<&str> = tasks
        .iter()
        .map(|t| t["taskId"].as_str().unwrap())
        .collect();

    // build and test should be affected (default inputs / test includes .test.ts)
    assert!(
        task_ids.contains(&"lib-a#build"),
        "lib-a#build should be affected by .test.ts change: {task_ids:?}"
    );
    assert!(
        task_ids.contains(&"lib-a#test"),
        "lib-a#test should be affected by .test.ts change: {task_ids:?}"
    );

    // typecheck should NOT be affected (excludes *.test.ts)
    assert!(
        !task_ids.contains(&"lib-a#typecheck"),
        "lib-a#typecheck should NOT be affected by .test.ts change (excluded): {task_ids:?}"
    );
}

#[test]
fn test_task_level_affected_global_file_runs_everything() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_task_level_affected(tempdir.path());

    // Change root package.json — a default global dependency.
    let root_pkg = tempdir.path().join("package.json");
    let contents = fs::read_to_string(&root_pkg).unwrap();
    let mut pkg: serde_json::Value = serde_json::from_str(&contents).unwrap();
    pkg["description"] = serde_json::Value::String("changed".to_string());
    fs::write(&root_pkg, serde_json::to_string_pretty(&pkg).unwrap()).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "build",
            "test",
            "typecheck",
            "--affected",
            "--dry=json",
        ],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("failed to parse dry run JSON: {e}\nstdout: {stdout}"));

    let tasks = json["tasks"].as_array().expect("tasks array");
    let task_ids: Vec<&str> = tasks
        .iter()
        .map(|t| t["taskId"].as_str().unwrap())
        .collect();

    // All tasks in both packages should be affected when a global file changes.
    assert!(
        task_ids.contains(&"lib-a#build"),
        "global change should affect all tasks: {task_ids:?}"
    );
    assert!(
        task_ids.contains(&"lib-a#test"),
        "global change should affect all tasks: {task_ids:?}"
    );
    assert!(
        task_ids.contains(&"app-a#build"),
        "global change should affect all tasks: {task_ids:?}"
    );
    assert!(
        task_ids.len() >= 4,
        "global change should affect many tasks, got {}: {task_ids:?}",
        task_ids.len()
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

// -- turbo query affected shorthand tests --

#[test]
fn test_query_affected_shorthand_no_args() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    fs::write(tempdir.path().join("apps/my-app/new.js"), "foo").unwrap();

    // Default (no flags) returns affected tasks
    let output = run_turbo(tempdir.path(), &["query", "affected"]);
    assert!(output.status.success(), "query affected should succeed");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = json["data"]["affectedTasks"]["items"].as_array().unwrap();
    assert!(!items.is_empty(), "should have affected tasks");
    let names: Vec<&str> = items
        .iter()
        .map(|i| i["fullName"].as_str().unwrap())
        .collect();
    assert!(
        names.iter().any(|n| n.contains("my-app")),
        "my-app tasks should be affected: {names:?}"
    );
}

#[test]
fn test_query_affected_shorthand_bare_packages() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    fs::write(tempdir.path().join("apps/my-app/new.js"), "foo").unwrap();

    // --packages with no value returns all affected packages
    let output = run_turbo(tempdir.path(), &["query", "affected", "--packages"]);
    assert!(
        output.status.success(),
        "query affected --packages should succeed"
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = json["data"]["affectedPackages"]["items"]
        .as_array()
        .unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "my-app");
    assert_eq!(items[0]["reason"]["__typename"], "FileChanged");
    assert!(items[0]["path"].is_string(), "should include path field");
}

#[test]
fn test_query_affected_shorthand_with_packages() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    fs::write(tempdir.path().join("packages/util/new.js"), "foo").unwrap();

    // Filter to only "util" — should exclude "my-app" (which is a dependent)
    let output = run_turbo(tempdir.path(), &["query", "affected", "--packages", "util"]);
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = json["data"]["affectedPackages"]["items"]
        .as_array()
        .unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "util");
}

#[test]
fn test_query_affected_shorthand_nothing_affected() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    // No file changes — nothing should be affected
    let output = run_turbo(tempdir.path(), &["query", "affected"]);
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let length = json["data"]["affectedTasks"]["length"].as_i64().unwrap();
    assert_eq!(length, 0, "nothing should be affected on a clean branch");
}

#[test]
fn test_query_affected_shorthand_with_tasks() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    fs::write(tempdir.path().join("apps/my-app/new.js"), "foo").unwrap();

    let output = run_turbo(tempdir.path(), &["query", "affected", "--tasks", "build"]);
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = json["data"]["affectedTasks"]["items"].as_array().unwrap();
    assert!(!items.is_empty(), "build task should be affected");
    let names: Vec<&str> = items
        .iter()
        .map(|i| i["fullName"].as_str().unwrap())
        .collect();
    assert!(
        names.iter().any(|n| n.contains("my-app")),
        "my-app#build should be in affected tasks: {names:?}"
    );
}

#[test]
fn test_query_affected_shorthand_with_base_head() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    fs::write(tempdir.path().join("apps/my-app/new.js"), "foo").unwrap();
    git(tempdir.path(), &["add", "."]);
    git(tempdir.path(), &["commit", "-m", "add file", "--quiet"]);

    // With --base=HEAD, no uncommitted changes → nothing affected
    let output = run_turbo(tempdir.path(), &["query", "affected", "--base", "HEAD"]);
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let length = json["data"]["affectedTasks"]["length"].as_i64().unwrap();
    assert_eq!(length, 0, "base=HEAD should show no changes: {}", json);
}

#[test]
fn test_query_affected_shorthand_combined_packages_and_tasks() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    fs::write(tempdir.path().join("apps/my-app/new.js"), "foo").unwrap();

    // --tasks build --packages my-app → intersection: tasks named "build" AND in
    // package "my-app"
    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "affected",
            "--tasks",
            "build",
            "--packages",
            "my-app",
        ],
    );
    assert!(output.status.success(), "combined query should succeed");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = json["data"]["affectedTasks"]["items"].as_array().unwrap();
    assert!(!items.is_empty(), "should have affected tasks: {json}");
    let full_names: Vec<&str> = items
        .iter()
        .map(|i| i["fullName"].as_str().unwrap())
        .collect();
    assert!(
        full_names.contains(&"my-app#build"),
        "my-app#build should be in results: {full_names:?}"
    );
}
